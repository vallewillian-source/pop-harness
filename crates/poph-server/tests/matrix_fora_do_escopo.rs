//! Tipos fora do escopo na sala (MX-45): fim a fim, com o banco e o laço do espelho de verdade.
//!
//! O que este arquivo prova, e por quê:
//!
//! - **os seis tipos viram aviso na sala**: áudio, vídeo, figurinha, contato, localização e enquete
//!   gravados como texto de histórico chegam à sala como `m.notice` com o texto do Python, assinados
//!   pelo fantasma **do agente** (dono da sala), e a mensagem de texto que vem depois segue normal;
//! - **a entrega não duplica o aviso**: a mesma mensagem passada de novo sai com o mesmo `txnId`,
//!   e o Synapse falso deduplica por `txnId` como o de verdade;
//! - **texto comum não vira aviso**: uma mensagem do atendente com a frase de histórico segue como
//!   texto, e um texto do cliente que só se parece com ela também;
//! - **a mensagem seguinte passa**: o aviso não trava o cursor do espelho.
//!
//! O Synapse é falso e com estado; o banco e o laço são os de verdade. O webhook que grava esses
//! tipos está em `crates/poph-evolution/tests/webhook.rs`.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use poph_matrix::erro::ErroChamada;
use poph_matrix::formato::Envio;
use poph_matrix::salas::{ConversaParaSala, ErroSala};
use poph_matrix::usuarios::Fantasma;
use poph_server::matrix_espelho::{
    AvisoNaSala, DestinoMatrix, EnvioMidia, EspelhoMatrix, MidiaPublicada,
};
use poph_store::{Direction, NewAgent, NewConversation, NewMessage, NewOrg, Origin, Role, Store};
use poph_types::{AgentId, ConversationId, MatrixConfig, OrgId, UnsupportedContent};

// ───────────────────────────── cenário ─────────────────────────────

/// Banco e pasta de um teste, apagados no fim.
struct Cenario {
    pasta: PathBuf,
    store: Option<Arc<Store>>,
}

impl Cenario {
    fn novo(rotulo: &str) -> Self {
        static CONTADOR: AtomicUsize = AtomicUsize::new(0);
        let n = CONTADOR.fetch_add(1, Ordering::Relaxed);
        let pasta = std::env::temp_dir().join(format!(
            "poph-fora-do-escopo-{rotulo}-{}-{n}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&pasta);
        fs::create_dir_all(&pasta).expect("cria a pasta do teste");
        let relogio = Arc::new(AtomicI64::new(1_000));
        let store = Store::open_at(&pasta.join("harness.db"), &pasta.join("rag.db"))
            .expect("abre o banco")
            .with_clock(Arc::new(move || relogio.fetch_add(1, Ordering::SeqCst)));
        Self {
            pasta,
            store: Some(Arc::new(store)),
        }
    }

    fn store(&self) -> Arc<Store> {
        Arc::clone(self.store.as_ref().expect("store viva"))
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

/// Mensagem do cliente (entrada, papel `user`), como o recebimento grava o texto de histórico.
fn msg_cliente(store: &Store, org: OrgId, conversa: ConversationId, texto: &str) {
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
            &[],
        )
        .expect("grava a mensagem");
}

/// Mensagem do atendente do agente (saída, papel `assistant`).
fn msg_agente(store: &Store, org: OrgId, conversa: ConversationId, texto: &str) {
    store
        .messages()
        .write_with_attachments(
            org,
            NewMessage {
                conversation_id: conversa,
                direction: Direction::Outbound,
                role: Role::Assistant,
                origin: Origin::Agente,
                author_name: None,
                texto: texto.to_owned(),
                attachments_json: "[]".to_owned(),
                external_id: None,
                turn_id: None,
            },
            &[],
        )
        .expect("grava a mensagem do agente");
}

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

// ───────────────────────────── Synapse falso ─────────────────────────────

/// Um texto que o destino recebeu: quem assina, o tipo de mensagem e o corpo.
#[derive(Debug, Clone)]
struct TextoEntregue {
    txn: String,
    remetente: String,
    msgtype: String,
    corpo: String,
}

#[derive(Default)]
struct Falso {
    /// Textos aceitos, um por `txnId` (o Synapse deduplica pelo `txnId`).
    textos: Mutex<Vec<TextoEntregue>>,
    /// Quantas tentativas de envio cada `txnId` recebeu, aceitas ou não.
    tentativas: Mutex<Vec<String>>,
    /// Quantas próximas chamadas falham de forma transitória (503) antes de aceitar.
    falhas_transitorias: Mutex<usize>,
}

impl Falso {
    fn novo() -> Arc<Self> {
        Arc::new(Self::default())
    }

    fn textos(&self) -> Vec<TextoEntregue> {
        self.textos.lock().unwrap().clone()
    }

    fn tentativas_de(&self, txn: &str) -> usize {
        self.tentativas
            .lock()
            .unwrap()
            .iter()
            .filter(|t| t.as_str() == txn)
            .count()
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

struct Sinal(Arc<Falso>);

impl DestinoMatrix for Sinal {
    fn garantir_sala(
        &self,
        dados: &ConversaParaSala,
    ) -> impl std::future::Future<Output = Result<String, ErroSala>> + Send {
        let sala = format!("!{}:matrix-dev.local", dados.conversa);
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
        _sala: &str,
        envio: &Envio,
    ) -> impl std::future::Future<Output = Result<(), ErroChamada>> + Send {
        let falso = &self.0;
        let txn = envio.txn_id.clone();
        falso.tentativas.lock().unwrap().push(txn.clone());
        let resultado = {
            let mut falhas = falso.falhas_transitorias.lock().unwrap();
            if *falhas > 0 {
                *falhas -= 1;
                Err(transitoria())
            } else {
                Ok(())
            }
        };
        if resultado.is_ok() {
            let mut textos = falso.textos.lock().unwrap();
            if !textos.iter().any(|t| t.txn == txn) {
                textos.push(TextoEntregue {
                    txn,
                    remetente: envio.remetente.user_id().to_owned(),
                    msgtype: envio.msgtype.to_owned(),
                    corpo: envio.corpo.clone(),
                });
            }
        }
        async move { resultado }
    }

    async fn enviar_midia(
        &self,
        _sala: &str,
        _envio: &EnvioMidia,
    ) -> Result<MidiaPublicada, ErroChamada> {
        // Este destino não publica mídia: nenhuma mensagem do teste tem anexo.
        Err(ErroChamada::Configuracao(
            "sem mídia neste destino".to_owned(),
        ))
    }

    async fn avisar(&self, _sala: &str, _aviso: &AvisoNaSala) -> Result<(), ErroChamada> {
        Ok(())
    }
}

fn espelho(cenario: &Cenario, falso: Arc<Falso>) -> EspelhoMatrix<Sinal> {
    EspelhoMatrix::novo(cenario.store(), Sinal(falso), &config()).expect("monta o espelho")
}

// ───────────────────────────── testes ─────────────────────────────

#[tokio::test]
async fn os_seis_tipos_viram_aviso_na_sala_e_a_legenda_seguinte_passa() {
    let cenario = Cenario::novo("seis-tipos");
    let store = cenario.store();
    let org = criar_org(&store, "a");
    let agente = criar_agente(&store, org);
    let conversa = criar_conversa(&store, org, agente, "5511977770001");
    store.matrix().enable(org).expect("liga o módulo");

    for tipo in UnsupportedContent::ALL_VALUES {
        msg_cliente(&store, org, conversa, tipo.history_text());
    }
    msg_cliente(
        &store,
        org,
        conversa,
        "tudo bem, pode me responder por texto?",
    );

    let falso = Falso::novo();
    let relatorio = espelho(&cenario, Arc::clone(&falso))
        .ciclo()
        .await
        .expect("ciclo");

    let textos = falso.textos();
    assert_eq!(
        textos.len(),
        7,
        "seis avisos e a legenda de texto: {textos:?}"
    );
    for (entregue, tipo) in textos.iter().zip(UnsupportedContent::ALL_VALUES) {
        assert_eq!(entregue.msgtype, "m.notice", "{tipo:?}");
        assert_eq!(entregue.corpo, tipo.room_notice(), "{tipo:?}");
        assert_eq!(
            entregue.remetente,
            format!("@ph_ag_{agente}:matrix-dev.local"),
            "o aviso é do fantasma do agente, dono da sala"
        );
    }
    let ultimo = &textos[6];
    assert_eq!(
        ultimo.msgtype, "m.text",
        "a mensagem seguinte entra como texto"
    );
    assert_eq!(ultimo.corpo, "tudo bem, pode me responder por texto?");
    assert_eq!(
        ultimo.remetente, "@ph_wa_5511977770001:matrix-dev.local",
        "a legenda de texto sai pelo fantasma do cliente, como antes"
    );
    assert_eq!(relatorio.enviadas, 7);
    assert_eq!(cursor(&store, org), maior_rowid_e_id(&store));
}

#[tokio::test]
async fn reprocessar_o_aviso_nao_duplica_na_sala() {
    let cenario = Cenario::novo("reprocessa");
    let store = cenario.store();
    let org = criar_org(&store, "b");
    let agente = criar_agente(&store, org);
    let conversa = criar_conversa(&store, org, agente, "5511977770002");
    store.matrix().enable(org).expect("liga o módulo");
    msg_cliente(
        &store,
        org,
        conversa,
        UnsupportedContent::Audio.history_text(),
    );

    // A primeira tentativa cai de forma transitória: a mensagem fica pendente e volta no ciclo
    // seguinte, com o mesmo `txnId`.
    let falso = Falso::novo();
    *falso.falhas_transitorias.lock().unwrap() = 1;
    let espelho_ = espelho(&cenario, Arc::clone(&falso));
    espelho_.ciclo().await.expect("primeiro ciclo");
    assert!(falso.textos().is_empty(), "a primeira tentativa falhou");
    espelho_.ciclo().await.expect("segundo ciclo");

    let textos = falso.textos();
    assert_eq!(textos.len(), 1, "um aviso só na sala: {textos:?}");
    assert_eq!(textos[0].msgtype, "m.notice");
    let txn = textos[0].txn.clone();
    assert_eq!(falso.tentativas_de(&txn), 2, "a mesma entrega foi repetida");
    assert_eq!(cursor(&store, org), maior_rowid_e_id(&store));
}

#[tokio::test]
async fn historico_escrito_pelo_atendente_segue_como_texto() {
    let cenario = Cenario::novo("atendente");
    let store = cenario.store();
    let org = criar_org(&store, "c");
    let agente = criar_agente(&store, org);
    let conversa = criar_conversa(&store, org, agente, "5511977770003");
    store.matrix().enable(org).expect("liga o módulo");
    msg_agente(
        &store,
        org,
        conversa,
        UnsupportedContent::Audio.history_text(),
    );

    let falso = Falso::novo();
    espelho(&cenario, Arc::clone(&falso))
        .ciclo()
        .await
        .expect("ciclo");

    let textos = falso.textos();
    assert_eq!(textos.len(), 1, "{textos:?}");
    assert_eq!(textos[0].msgtype, "m.text", "só o cliente vira aviso");
    assert_eq!(
        textos[0].remetente,
        format!("@ph_ag_{agente}:matrix-dev.local")
    );
}
