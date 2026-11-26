//! Mídia do cliente na sala (MX-43): fim a fim, com o banco e os blobs de verdade.
//!
//! O que este arquivo prova, e por quê:
//!
//! - **imagem e PDF aparecem na sala**: o anexo gravado em `attachments` (com o blob no disco) vira
//!   `m.image`/`m.file` com o bloco `com.<marca>.attachment` e o `mxc://` que a subida devolveu,
//!   publicados pelo fantasma **do cliente**;
//! - **a legenda sai depois da mídia**: o texto do cliente continua sendo o evento de texto, na
//!   ordem em que o atendente precisa ler;
//! - **reprocessar não duplica**: a entrega que falha de forma transitória volta na passagem
//!   seguinte com o mesmo `txnId`, e o Synapse falso deduplica por `txnId` como o de verdade;
//! - **arquivo acima de 16 MB vira aviso e o cursor segue**: a mensagem seguinte é entregue no
//!   mesmo ciclo, e o cursor passa pela mídia recusada;
//! - **anexo sem blob no disco também vira aviso**, sem prender o cursor;
//! - **mídia recusada de vez pelo Synapse deixa o aviso antes de isolar**.
//!
//! O Synapse é falso e com estado (deduplica por `txnId`, recusa o que o teste pedir). O banco, os
//! blobs e o laço são os de verdade.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use poph_matrix::erro::ErroChamada;
use poph_matrix::formato::Envio;
use poph_matrix::midia::{
    self, BLOCO_ANEXO, LIMITE_MIDIA_BYTES, MSGTYPE_ARQUIVO, MSGTYPE_IMAGEM, PREFIXO_AVISO,
};
use poph_matrix::salas::{ConversaParaSala, ErroSala};
use poph_matrix::usuarios::Fantasma;
use poph_server::matrix_espelho::{
    AvisoNaSala, DestinoMatrix, EnvioMidia, EspelhoMatrix, MidiaPublicada,
};
use poph_store::{
    Direction, NewAgent, NewChannelAttachment, NewConversation, NewMessage, NewOrg, Origin, Role,
    Store,
};
use poph_types::{AgentId, ConversationId, MatrixConfig, OrgId};
use serde_json::Value;

// ───────────────────────────── cenário ─────────────────────────────

/// Banco e pastas de um teste, apagados no fim.
struct Cenario {
    pasta: PathBuf,
    blobs: PathBuf,
    store: Option<Arc<Store>>,
}

impl Cenario {
    fn novo(rotulo: &str) -> Self {
        static CONTADOR: AtomicUsize = AtomicUsize::new(0);
        let n = CONTADOR.fetch_add(1, Ordering::Relaxed);
        let pasta =
            std::env::temp_dir().join(format!("poph-midia-{rotulo}-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&pasta);
        fs::create_dir_all(&pasta).expect("cria a pasta do teste");
        let blobs = pasta.join("blobs");
        fs::create_dir_all(&blobs).expect("cria a pasta de blobs");
        let relogio = Arc::new(AtomicI64::new(1_000));
        let store = Store::open_at(&pasta.join("harness.db"), &pasta.join("rag.db"))
            .expect("abre o banco")
            .with_clock(Arc::new(move || relogio.fetch_add(1, Ordering::SeqCst)));
        Self {
            pasta,
            blobs,
            store: Some(Arc::new(store)),
        }
    }

    fn store(&self) -> Arc<Store> {
        Arc::clone(self.store.as_ref().expect("store viva"))
    }

    /// Grava o blob como o recebimento grava e devolve os metadados do anexo.
    fn anexo(&self, org: OrgId, nome: &str, mime: &str, bytes: &[u8]) -> NewChannelAttachment {
        let gravado = poph_files::blob::gravar(&self.blobs, org, bytes).expect("grava o blob");
        NewChannelAttachment {
            name: nome.to_owned(),
            mime: mime.to_owned(),
            size_bytes: bytes.len() as u64,
            sha256: gravado.sha256.clone(),
            blob_path: format!("{org}/{}", gravado.sha256),
            extracted_text: None,
            extraction_status: "skipped",
        }
    }
}

impl Drop for Cenario {
    fn drop(&mut self) {
        let _ = self.store.take();
        let _ = fs::remove_dir_all(&self.pasta);
    }
}

fn config() -> MatrixConfig {
    MatrixConfig {
        enabled: true,
        homeserver_url: "http://127.0.0.1:1".to_owned(),
        server_name: "matrix-dev.local".to_owned(),
        prefix: "ph".to_owned(),
        concurrency: 4,
        interval_ms: 500,
        ..MatrixConfig::default()
    }
}

fn criar_org(store: &Store, slug: &str) -> OrgId {
    store
        .orgs()
        .create(NewOrg {
            name: format!("Org {slug}"),
            slug: slug.to_owned(),
            config_json: String::new(),
        })
        .expect("cria a org")
        .id
}

fn criar_agente(store: &Store, org: OrgId) -> AgentId {
    store
        .agents()
        .create(
            org,
            NewAgent {
                name: "Bia".to_owned(),
                slug: "bia".to_owned(),
                ..NewAgent::default()
            },
        )
        .expect("cria o agente")
        .id
}

fn criar_conversa(store: &Store, org: OrgId, agente: AgentId, peer: &str) -> ConversationId {
    store
        .conversations()
        .create(
            org,
            NewConversation {
                agent_id: agente,
                integration_id: None,
                canal: "whatsapp".to_owned(),
                peer_externo: Some(peer.to_owned()),
                target_department: None,
            },
        )
        .expect("cria a conversa")
        .id
}

/// Mensagem do cliente (entrada, papel `user`), com ou sem anexos.
fn msg_cliente(
    store: &Store,
    org: OrgId,
    conversa: ConversationId,
    texto: &str,
    anexos: &[NewChannelAttachment],
) -> poph_types::MessageId {
    store
        .messages()
        .write_with_attachments(
            org,
            NewMessage {
                conversation_id: conversa,
                direction: Direction::Inbound,
                role: Role::User,
                origin: Origin::Agente,
                author_name: None,
                texto: texto.to_owned(),
                attachments_json: "[]".to_owned(),
                external_id: None,
                turn_id: None,
            },
            anexos,
        )
        .expect("grava a mensagem")
        .id
}

// ───────────────────────────── Synapse falso ─────────────────────────────

/// Um evento de mídia que o destino recebeu: o que a subida devolveu e o conteúdo publicado.
#[derive(Debug, Clone)]
struct MidiaEntregue {
    sala: String,
    txn: String,
    remetente: String,
    /// `mxc://` que a "subida" devolveu (aqui, derivada do `txnId`).
    mxc: String,
    /// Conteúdo do evento, como o destino real monta.
    conteudo: Value,
    /// Bytes que subiram.
    bytes: usize,
}

impl MidiaEntregue {
    /// `msgtype` do evento publicado.
    fn msgtype(&self) -> &str {
        self.conteudo["msgtype"].as_str().unwrap_or_default()
    }
}

/// Um aviso em texto na sala.
#[derive(Debug, Clone)]
struct AvisoEntregue {
    sala: String,
    txn: String,
    remetente: String,
    corpo: String,
}

/// O que o teste pede ao Synapse falso.
#[derive(Debug, Clone, Default)]
enum Recusa {
    /// Aceita tudo.
    #[default]
    Nada,
    /// Recusa a mídia (403) por `vezes` chamadas, depois aceita.
    MidiaDefinitiva(usize),
    /// Recusa a mídia de forma transitória (503) por `vezes` chamadas, depois aceita.
    MidiaTransitoria(usize),
}

#[derive(Default)]
struct Falso {
    /// Eventos de mídia aceitos, na ordem.
    midias: Mutex<Vec<MidiaEntregue>>,
    /// Avisos aceitos, na ordem.
    avisos: Mutex<Vec<AvisoEntregue>>,
    /// Textos aceitos, na ordem: `(sala, txn, corpo, remetente)`.
    textos: Mutex<Vec<(String, String, String, String)>>,
    /// Quantas vezes cada `txnId` foi tentado.
    tentativas: Mutex<HashMap<String, u32>>,
    /// Recusa pedida pelo teste.
    recusa: Mutex<Recusa>,
    /// Salas pedidas.
    salas: AtomicUsize,
    /// Ordem de tudo o que entrou na sala: `midia:<txn>`, `texto:<txn>`, `aviso:<txn>`.
    ordem: Mutex<Vec<String>>,
}

impl Falso {
    fn novo(recusa: Recusa) -> Arc<Self> {
        let falso = Self {
            recusa: Mutex::new(recusa),
            ..Self::default()
        };
        Arc::new(falso)
    }

    fn midias(&self) -> Vec<MidiaEntregue> {
        self.midias.lock().unwrap().clone()
    }

    fn avisos(&self) -> Vec<AvisoEntregue> {
        self.avisos.lock().unwrap().clone()
    }

    fn textos(&self) -> Vec<(String, String, String, String)> {
        self.textos.lock().unwrap().clone()
    }

    fn ordem(&self) -> Vec<String> {
        self.ordem.lock().unwrap().clone()
    }

    fn tentativas_de(&self, txn: &str) -> u32 {
        *self.tentativas.lock().unwrap().get(txn).unwrap_or(&0)
    }

    /// Decide a resposta da próxima chamada de mídia.
    fn decidir_midia(&self) -> Result<(), ErroChamada> {
        let mut recusa = self.recusa.lock().unwrap();
        match &mut *recusa {
            Recusa::Nada => Ok(()),
            Recusa::MidiaDefinitiva(vezes) if *vezes > 0 => {
                *vezes -= 1;
                Err(recusada())
            }
            Recusa::MidiaTransitoria(vezes) if *vezes > 0 => {
                *vezes -= 1;
                Err(transitoria())
            }
            _ => Ok(()),
        }
    }
}

fn transitoria() -> ErroChamada {
    ErroChamada::Resposta {
        operacao: "send",
        status: 503,
        errcode: Some("M_UNKNOWN".to_owned()),
        mensagem: "indisponível".to_owned(),
        retry_after_ms: None,
    }
}

fn recusada() -> ErroChamada {
    ErroChamada::Resposta {
        operacao: "send",
        status: 403,
        errcode: Some("M_FORBIDDEN".to_owned()),
        mensagem: "recusado".to_owned(),
        retry_after_ms: None,
    }
}

struct Sinal(Arc<Falso>);

impl DestinoMatrix for Sinal {
    fn garantir_sala(
        &self,
        dados: &ConversaParaSala,
    ) -> impl std::future::Future<Output = Result<String, ErroSala>> + Send {
        let sala = format!("!{}:matrix-dev.local", dados.conversa);
        self.0.salas.fetch_add(1, Ordering::SeqCst);
        async move { Ok(sala) }
    }

    async fn garantir_aparelho(
        &self,
        _dados: &ConversaParaSala,
        _sala: &str,
        _aparelho: &Fantasma,
    ) -> Result<(), ErroSala> {
        Ok(())
    }

    fn enviar(
        &self,
        sala: &str,
        envio: &Envio,
    ) -> impl std::future::Future<Output = Result<(), ErroChamada>> + Send {
        let falso = &self.0;
        let sala = sala.to_owned();
        let txn = envio.txn_id.clone();
        let corpo = envio.corpo.clone();
        let remetente = envio.remetente.user_id().to_owned();
        *falso
            .tentativas
            .lock()
            .unwrap()
            .entry(txn.clone())
            .or_insert(0) += 1;
        let mut textos = falso.textos.lock().unwrap();
        if !textos.iter().any(|(_, t, _, _)| *t == txn) {
            textos.push((sala, txn.clone(), corpo, remetente));
            falso.ordem.lock().unwrap().push(format!("texto:{txn}"));
        }
        drop(textos);
        async move { Ok(()) }
    }

    fn enviar_midia(
        &self,
        sala: &str,
        envio: &EnvioMidia,
    ) -> impl std::future::Future<Output = Result<MidiaPublicada, ErroChamada>> + Send {
        let falso = &self.0;
        let sala = sala.to_owned();
        let txn = envio.midia.txn_id.clone();
        *falso
            .tentativas
            .lock()
            .unwrap()
            .entry(txn.clone())
            .or_insert(0) += 1;
        let resultado = falso.decidir_midia();
        let remetente = envio.remetente.user_id().to_owned();
        // Como o destino real: sobe o arquivo (aqui, o `mxc` sai do `txnId`) e publica o evento
        // com o conteúdo que a MX-43 monta.
        let mxc = format!("mxc://matrix-dev.local/{}", txn);
        let conteudo = midia::conteudo_do_evento(&mxc, &envio.midia);
        let mut midias = falso.midias.lock().unwrap();
        let novo = !midias.iter().any(|m| m.sala == sala && m.txn == txn);
        if resultado.is_ok() && novo {
            falso.ordem.lock().unwrap().push(format!("midia:{txn}"));
            midias.push(MidiaEntregue {
                sala: sala.clone(),
                txn: txn.clone(),
                remetente,
                mxc: mxc.clone(),
                conteudo,
                bytes: envio.conteudo.len(),
            });
        }
        drop(midias);
        // Como o destino real: o evento publicado e o `mxc` que a subida devolveu.
        let publicada = MidiaPublicada {
            evento: format!("$midia-{txn}:matrix-dev.local"),
            mxc,
        };
        async move { resultado.map(|()| publicada) }
    }

    fn avisar(
        &self,
        sala: &str,
        aviso: &AvisoNaSala,
    ) -> impl std::future::Future<Output = Result<(), ErroChamada>> + Send {
        let falso = &self.0;
        let sala = sala.to_owned();
        let txn = aviso.txn_id.clone();
        let remetente = aviso.remetente.user_id().to_owned();
        let corpo = midia::corpo_do_aviso(aviso.motivo);
        let mut avisos = falso.avisos.lock().unwrap();
        if !avisos.iter().any(|a| a.sala == sala && a.txn == txn) {
            falso.ordem.lock().unwrap().push(format!("aviso:{txn}"));
            avisos.push(AvisoEntregue {
                sala,
                txn,
                remetente,
                corpo,
            });
        }
        drop(avisos);
        async move { Ok(()) }
    }
}

fn espelho(cenario: &Cenario, falso: Arc<Falso>) -> EspelhoMatrix<Sinal> {
    EspelhoMatrix::novo(cenario.store(), Sinal(falso), &config())
        .expect("monta o espelho")
        .com_pasta_de_blobs(cenario.blobs.clone())
}

fn sala_de(conversa: ConversationId) -> String {
    format!("!{conversa}:matrix-dev.local")
}

fn fantasma_do_cliente(peer: &str) -> String {
    format!("@ph_wa_{peer}:matrix-dev.local")
}

/// Um PNG mínimo (assinatura e nada mais) e um PDF mínimo: o conteúdo não importa para o Synapse
/// falso, mas o tipo e o tamanho importam.
const PNG: &[u8] = b"\x89PNG\r\n\x1a\nimagem-de-teste";
const PDF: &[u8] = b"%PDF-1.4\nlaudo de teste\n%%EOF\n";

fn cursor(store: &Store, org: OrgId) -> (u64, String) {
    let cursores = store.matrix().read_cursors(org).expect("lê os cursores");
    (cursores.messages, cursores.message_ids)
}

/// Id da última mensagem: o cursor tem de chegar nela quando não há pendência.
fn maior_rowid_e_id(store: &Store) -> (u64, String) {
    let (rowid, id) = store
        .messages()
        .last_rowid()
        .expect("lê a última rowid")
        .expect("tem mensagem");
    (rowid, id.to_string())
}

/// Id do anexo que o evento publicado carrega no bloco.
fn bloco_id(midia: &MidiaEntregue) -> String {
    midia.conteudo[BLOCO_ANEXO]["id"]
        .as_str()
        .expect("o bloco leva o id")
        .to_owned()
}

// ───────────────────────────── testes ─────────────────────────────

#[tokio::test]
async fn imagem_e_pdf_do_cliente_vao_para_a_sala_depois_da_legenda() {
    let cenario = Cenario::novo("imagem-pdf");
    let store = cenario.store();
    let org = criar_org(&store, "a");
    let agente = criar_agente(&store, org);
    let peer = "5521988887777";
    let conversa = criar_conversa(&store, org, agente, peer);
    store.matrix().enable(org).expect("liga");

    let imagem = cenario.anexo(org, "foto.jpg", "image/jpeg", PNG);
    let pdf = cenario.anexo(org, "laudo.pdf", "application/pdf", PDF);
    let com_imagem = msg_cliente(&store, org, conversa, "olha a foto", &[imagem]);
    let com_pdf = msg_cliente(&store, org, conversa, "segue o laudo", &[pdf]);

    let falso = Falso::novo(Recusa::Nada);
    let relatorio = espelho(&cenario, Arc::clone(&falso))
        .ciclo()
        .await
        .expect("ciclo");

    let midias = falso.midias();
    assert_eq!(midias.len(), 2, "uma imagem e um PDF:");
    assert_eq!(relatorio.midias_enviadas, 2);
    assert_eq!(relatorio.enviadas, 2, "as duas legendas");

    // Imagem: `m.image`, pelo fantasma do cliente, com o bloco e o tamanho do blob.
    assert_eq!(midias[0].msgtype(), MSGTYPE_IMAGEM);
    assert_eq!(midias[0].remetente, fantasma_do_cliente(peer));
    assert_eq!(midias[0].sala, sala_de(conversa));
    assert_eq!(midias[0].conteudo["body"], Value::String("foto.jpg".into()));
    assert_eq!(
        midias[0].conteudo["url"],
        Value::String(midias[0].mxc.clone())
    );
    assert_eq!(
        midias[0].conteudo["info"]["mimetype"],
        Value::String("image/jpeg".into())
    );
    assert_eq!(midias[0].bytes, PNG.len());
    let bloco = &midias[0].conteudo[BLOCO_ANEXO];
    assert_eq!(bloco["source"], Value::String("whatsapp".into()));
    assert!(
        bloco["id"].as_str().is_some_and(|id| !id.is_empty()),
        "o bloco leva o id do anexo: {bloco}"
    );

    // PDF: `m.file`.
    assert_eq!(midias[1].msgtype(), MSGTYPE_ARQUIVO);
    assert_eq!(
        midias[1].conteudo["info"]["mimetype"],
        Value::String("application/pdf".into())
    );
    assert_eq!(midias[1].bytes, PDF.len());

    // A legenda sai depois da mídia, com o texto do cliente e o `txnId` da mensagem.
    let textos = falso.textos();
    assert_eq!(textos.len(), 2);
    assert_eq!(textos[0].2, "olha a foto");
    assert_eq!(textos[0].1, format!("poph-msg-{com_imagem}"));
    assert_eq!(textos[1].2, "segue o laudo");
    assert_eq!(textos[1].1, format!("poph-msg-{com_pdf}"));

    // A ordem dentro da sala é a que o atendente lê: mídia e depois a legenda, mensagem por
    // mensagem (a mídia da segunda não passa na frente da legenda da primeira).
    let ordem = falso.ordem();
    assert_eq!(
        ordem,
        vec![
            format!("midia:{}", midia::txn_da_midia(&bloco_id(&midias[0]))),
            format!("texto:poph-msg-{com_imagem}"),
            format!("midia:{}", midia::txn_da_midia(&bloco_id(&midias[1]))),
            format!("texto:poph-msg-{com_pdf}"),
        ]
    );

    // O cursor chegou ao fim da fila e não há aviso nenhum.
    assert!(falso.avisos().is_empty(), "nada recusado");
    assert_eq!(cursor(&store, org), maior_rowid_e_id(&store));

    // MX-46: cada mídia entregue ficou registrada para expirar, com o evento e o `mxc` que a sala
    // recebeu — é o que a rotina de expiração lê depois do prazo.
    let registradas = store
        .matrix_asset()
        .pending(org, i64::MAX, i64::MAX, 10)
        .expect("lê o registro da mídia");
    assert_eq!(registradas.len(), 2, "as duas mídias registradas");
    for (midia, registrada) in midias.iter().zip(&registradas) {
        assert_eq!(registrada.room, midia.sala);
        assert_eq!(
            registrada.evento,
            format!("$midia-{}:matrix-dev.local", midia.txn)
        );
        assert_eq!(registrada.mxc, midia.mxc);
        assert_eq!(
            registrada.nome,
            midia.conteudo["body"].as_str().unwrap_or_default()
        );
        assert_eq!(registrada.agent, agente.to_string());
        // A categoria segue o que o evento é: imagem ou documento (o frontend só separa os dois).
        assert_eq!(
            registrada.category,
            if midia.msgtype() == MSGTYPE_IMAGEM {
                poph_store::CategoryAsset::Image
            } else {
                poph_store::CategoryAsset::Document
            },
            "categoria da mídia registrada"
        );
        assert_eq!(registrada.size_bytes, midia.bytes as u64);
        assert!(
            registrada.still_to_draft() && registrada.still_to_delete(),
            "recém-publicada: as duas etapas pendentes"
        );
    }
}

#[tokio::test]
async fn reprocessar_a_mesma_midia_nao_duplica_o_evento() {
    let cenario = Cenario::novo("idempotente");
    let store = cenario.store();
    let org = criar_org(&store, "b");
    let agente = criar_agente(&store, org);
    let conversa = criar_conversa(&store, org, agente, "5511911112222");
    store.matrix().enable(org).expect("liga");
    let imagem = cenario.anexo(org, "foto.png", "image/png", PNG);
    msg_cliente(&store, org, conversa, "foto", &[imagem]);

    // Primeira passagem: a mídia falha de forma transitória e fica pendente.
    let falso = Falso::novo(Recusa::MidiaTransitoria(1));
    let espelho_do_caso = espelho(&cenario, Arc::clone(&falso));
    let primeiro = espelho_do_caso.ciclo().await.expect("ciclo 1");
    assert_eq!(primeiro.midias_enviadas, 0);
    assert_eq!(primeiro.pendentes, 1, "a mídia ficou pendente");
    assert!(falso.midias().is_empty());
    let (rowid_pendente, _) = cursor(&store, org);
    assert_eq!(rowid_pendente, 0, "o cursor não passou da mídia pendente");

    // Segunda passagem: entrega. O `txnId` é o mesmo da tentativa anterior.
    let segundo = espelho_do_caso.ciclo().await.expect("ciclo 2");
    assert_eq!(segundo.midias_enviadas, 1);
    let midias = falso.midias();
    assert_eq!(midias.len(), 1, "um evento só, apesar das duas passagens");
    assert_eq!(
        falso.tentativas_de(&midias[0].txn),
        2,
        "as duas passagens usaram o mesmo `txnId` (o Synapse não duplicaria)"
    );

    // Terceira passagem, sem nada novo: nenhum envio.
    let terceiro = espelho_do_caso.ciclo().await.expect("ciclo 3");
    assert_eq!(terceiro.midias_enviadas, 0);
    assert_eq!(falso.midias().len(), 1);
    assert_eq!(cursor(&store, org), maior_rowid_e_id(&store));
}

#[tokio::test]
async fn arquivo_acima_do_limite_vira_aviso_e_o_cursor_segue() {
    let cenario = Cenario::novo("grande");
    let store = cenario.store();
    let org = criar_org(&store, "c");
    let agente = criar_agente(&store, org);
    let conversa = criar_conversa(&store, org, agente, "5511933334444");
    store.matrix().enable(org).expect("liga");

    // A linha diz que o arquivo passa do limite (o blob no disco é pequeno de propósito: nada é
    // lido antes da decisão, e o laço não pode depender do disco para recusar).
    let mut grande = cenario.anexo(org, "video.mp4", "video/mp4", PNG);
    grande.size_bytes = LIMITE_MIDIA_BYTES + 1;
    msg_cliente(&store, org, conversa, "olha o vídeo", &[grande]);
    msg_cliente(&store, org, conversa, "e depois isso", &[]);

    let falso = Falso::novo(Recusa::Nada);
    let relatorio = espelho(&cenario, Arc::clone(&falso))
        .ciclo()
        .await
        .expect("ciclo");

    assert!(falso.midias().is_empty(), "nada de mídia acima do limite");
    let avisos = falso.avisos();
    assert_eq!(avisos.len(), 1, "um aviso na sala");
    assert!(
        avisos[0].corpo.starts_with(PREFIXO_AVISO),
        "aviso: {}",
        avisos[0].corpo
    );
    assert!(
        avisos[0].corpo.contains("16 MB"),
        "aviso: {}",
        avisos[0].corpo
    );
    assert_eq!(relatorio.avisos_de_midia, 1);
    assert_eq!(
        avisos[0].remetente,
        format!("@ph_ag_{agente}:matrix-dev.local"),
        "o aviso é do fantasma do agente, dono da sala"
    );
    // A mensagem seguinte foi entregue no mesmo ciclo e o cursor passou pela mídia recusada.
    assert_eq!(relatorio.enviadas, 2, "as duas legendas passaram");
    assert_eq!(cursor(&store, org), maior_rowid_e_id(&store));
}

#[tokio::test]
async fn anexo_sem_blob_no_disco_vira_aviso_sem_prender_o_cursor() {
    let cenario = Cenario::novo("sem-blob");
    let store = cenario.store();
    let org = criar_org(&store, "d");
    let agente = criar_agente(&store, org);
    let conversa = criar_conversa(&store, org, agente, "5511955556666");
    store.matrix().enable(org).expect("liga");

    let anexo = cenario.anexo(org, "foto.jpg", "image/jpeg", PNG);
    // Apaga o blob do disco, como um volume que sumiu.
    let caminho = cenario.blobs.join(org.to_string()).join(&anexo.sha256);
    fs::remove_file(&caminho).expect("apaga o blob");
    msg_cliente(&store, org, conversa, "olha", &[anexo]);

    let falso = Falso::novo(Recusa::Nada);
    let relatorio = espelho(&cenario, Arc::clone(&falso))
        .ciclo()
        .await
        .expect("ciclo");

    assert!(falso.midias().is_empty());
    let avisos = falso.avisos();
    assert_eq!(avisos.len(), 1);
    assert!(
        avisos[0].corpo.contains("não pôde ser lido"),
        "aviso: {}",
        avisos[0].corpo
    );
    assert_eq!(relatorio.pendentes, 0);
    assert_eq!(cursor(&store, org), maior_rowid_e_id(&store));
}

#[tokio::test]
async fn midia_recusada_de_vez_deixa_o_aviso_antes_de_isolar() {
    let cenario = Cenario::novo("recusada");
    let store = cenario.store();
    let org = criar_org(&store, "e");
    let agente = criar_agente(&store, org);
    let conversa = criar_conversa(&store, org, agente, "5511977778888");
    store.matrix().enable(org).expect("liga");
    let pdf = cenario.anexo(org, "laudo.pdf", "application/pdf", PDF);
    msg_cliente(&store, org, conversa, "laudo", &[pdf]);

    let falso = Falso::novo(Recusa::MidiaDefinitiva(u32::MAX as usize));
    let espelho_do_caso = espelho(&cenario, Arc::clone(&falso));
    let mut ciclos = 0;
    // Até o laço desistir: `TENTATIVAS_DEFINITIVAS` passagens.
    while cursor(&store, org).0 == 0 && ciclos < 10 {
        espelho_do_caso.ciclo().await.expect("ciclo");
        ciclos += 1;
    }

    let avisos = falso.avisos();
    assert_eq!(avisos.len(), 1, "o aviso entrou uma vez só: {avisos:?}");
    assert!(
        avisos[0].corpo.contains("recusou o arquivo"),
        "aviso: {}",
        avisos[0].corpo
    );
    // O cursor passou: a mídia foi isolada em `matrix_falhas` e a fila não fica presa.
    assert_eq!(cursor(&store, org), maior_rowid_e_id(&store));
    // A mensagem ficou isolada em `matrix_falhas`, com o motivo da mídia recusada.
    let falhas = store.matrix().list_failures(org, 10).expect("lê as falhas");
    assert_eq!(falhas.len(), 1, "uma mensagem isolada: {falhas:?}");
    assert_eq!(falhas[0].motivo, "M_FORBIDDEN");
    assert_eq!(ciclos, 3, "as tentativas definitivas do laço");
}
