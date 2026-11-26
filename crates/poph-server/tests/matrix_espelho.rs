//! Testes do laço de espelhamento (MX-14).
//!
//! Banco de verdade (`Store` numa pasta temporária, relógio controlado) e Synapse **falso** em
//! memória: o duplo deduplica por `txnId` como o Synapse faz (MX-02), então "repetir após queda"
//! é medido de forma honesta — a entrega repetida não cria evento a mais.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use poph_matrix::erro::ErroChamada;
use poph_matrix::formato::{Envio, OrigemMensagem};
use poph_matrix::salas::{ConversaParaSala, ErroSala};
use poph_matrix::usuarios::Fantasma;
use poph_matrix::usuarios::IdsFantasmas;
use poph_server::matrix_espelho::{
    AvisoNaSala, DestinoMatrix, EnvioMidia, EspelhoMatrix, MAX_PAGINAS_POR_CICLO, MidiaPublicada,
    PAGINA, TENTATIVAS_DEFINITIVAS, TETO_ATRASO,
};
use poph_store::{
    Direction, NewAgent, NewConversation, NewDeviceMessage, NewMessage, NewOrg, Origin, Role, Store,
};
use poph_types::{AgentId, ConversationId, IntegrationId, MatrixConfig, MessageId, OrgId};
use rusqlite::Connection;

// ───────────────────────────── cenário ─────────────────────────────

/// Relógio que anda 1 ms a cada leitura: ordem de criação sempre estrita.
struct Cenario {
    pasta: PathBuf,
    store: Option<Arc<Store>>,
}

impl Cenario {
    fn novo(rotulo: &str) -> Self {
        static CONTADOR: AtomicUsize = AtomicUsize::new(0);
        let n = CONTADOR.fetch_add(1, Ordering::Relaxed);
        let pasta =
            std::env::temp_dir().join(format!("poph-espelho-{rotulo}-{}-{n}", std::process::id()));
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

    fn conexao(&self) -> Connection {
        Connection::open(self.store().harness_path()).expect("abre o banco para conferir")
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
        concurrency: 8,
        interval_ms: 500,
        ..MatrixConfig::default()
    }
}

fn criar_org(store: &Store, slug: &str) -> OrgId {
    store
        .orgs()
        .criar(NewOrg {
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
        .criar(
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
        .conversas()
        .criar(
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

/// Mensagem do cliente (entrada, papel `user`).
fn msg_cliente(store: &Store, org: OrgId, conversa: ConversationId, texto: &str) -> String {
    gravar(
        store,
        org,
        conversa,
        Direction::Inbound,
        Role::User,
        Origin::Agente,
        texto,
    )
}

/// Resposta do agente (saída, papel `assistant`).
fn msg_agente(store: &Store, org: OrgId, conversa: ConversationId, texto: &str) -> String {
    gravar(
        store,
        org,
        conversa,
        Direction::Outbound,
        Role::Assistant,
        Origin::Agente,
        texto,
    )
}

fn gravar(
    store: &Store,
    org: OrgId,
    conversa: ConversationId,
    direcao: Direction,
    papel: Role,
    origem: Origin,
    texto: &str,
) -> String {
    store
        .messages()
        .gravar(
            org,
            NewMessage {
                conversation_id: conversa,
                direction: direcao,
                role: papel,
                origin: origem,
                author_name: None,
                texto: texto.to_owned(),
                attachments_json: "[]".to_owned(),
                id_externo: None,
                turn_id: None,
            },
        )
        .expect("grava a mensagem")
        .id
        .to_string()
}

/// Id do `txnId` que o espelho usa para uma mensagem (`poph-msg-` + id).
fn txn_de(id: &str) -> String {
    format!("poph-msg-{id}")
}

// ───────────────────────────── Synapse falso ─────────────────────────────

#[derive(Clone, Copy)]
enum Modo {
    /// Aceita tudo.
    Normal,
    /// Toda chamada falha de forma transitória (Synapse fora do ar).
    Fora,
    /// Corpo que contém o texto é recusado com 403 (definitivo); o resto é aceito.
    Venenoso(&'static str),
    /// A n-ésima chamada de envio falha de forma transitória, uma vez só.
    FalhaNaChamada(usize),
}

/// Um evento que o Synapse falso criou: (sala, txnId, corpo, quem enviou).
#[derive(Debug, Clone, PartialEq, Eq)]
struct Criado {
    sala: String,
    txn: String,
    corpo: String,
    remetente: String,
}

struct Falso {
    modo: Mutex<Modo>,
    criados: Mutex<Vec<Criado>>,
    /// Quantas vezes cada `txnId` foi enviado (inclusive as recusadas e as repetidas).
    tentativas: Mutex<HashMap<String, u32>>,
    chamadas_sala: AtomicUsize,
    chamadas_envio: AtomicUsize,
    /// Quantas vezes o laço pediu para pôr o aparelho na sala (uma por grupo com mensagem dele).
    chamadas_aparelho: AtomicUsize,
    /// Quem está dentro de cada sala: `(sala, user_id)`. O envio de um aparelho que não está aqui é
    /// recusado, como o Synapse faz com quem não é membro.
    membros: Mutex<std::collections::HashSet<(String, String)>>,
    /// Quando `true`, pôr o aparelho na sala falha (Synapse fora do ar nessa etapa).
    falha_aparelho: Mutex<bool>,
}

impl Falso {
    fn novo(modo: Modo) -> Arc<Self> {
        Arc::new(Self {
            modo: Mutex::new(modo),
            criados: Mutex::new(Vec::new()),
            tentativas: Mutex::new(HashMap::new()),
            chamadas_sala: AtomicUsize::new(0),
            chamadas_envio: AtomicUsize::new(0),
            chamadas_aparelho: AtomicUsize::new(0),
            membros: Mutex::new(std::collections::HashSet::new()),
            falha_aparelho: Mutex::new(false),
        })
    }

    fn definir_falha_aparelho(&self, falha: bool) {
        *self.falha_aparelho.lock().unwrap() = falha;
    }

    fn criados_do(&self, remetente: &str) -> Vec<Criado> {
        self.criados()
            .into_iter()
            .filter(|criado| criado.remetente == remetente)
            .collect()
    }

    fn definir_modo(&self, modo: Modo) {
        *self.modo.lock().unwrap() = modo;
    }

    fn criados(&self) -> Vec<Criado> {
        self.criados.lock().unwrap().clone()
    }

    fn txns_da_sala(&self, sala: &str) -> Vec<String> {
        self.criados()
            .into_iter()
            .filter(|criado| criado.sala == sala)
            .map(|criado| criado.txn)
            .collect()
    }

    fn tentativas_de(&self, txn: &str) -> u32 {
        *self.tentativas.lock().unwrap().get(txn).unwrap_or(&0)
    }
}

fn transitorio() -> ErroChamada {
    ErroChamada::Resposta {
        operacao: "send",
        status: 503,
        errcode: Some("M_UNKNOWN".to_owned()),
        mensagem: "indisponível".to_owned(),
        retry_after_ms: None,
    }
}

fn recusado() -> ErroChamada {
    ErroChamada::Resposta {
        operacao: "send",
        status: 403,
        errcode: Some("M_FORBIDDEN".to_owned()),
        mensagem: "recusado".to_owned(),
        retry_after_ms: None,
    }
}

/// O duplo compartilhado com o teste: o espelho fica com uma cópia do `Arc`, o teste com outra.
struct Sinal(Arc<Falso>);

impl DestinoMatrix for Sinal {
    fn garantir_sala(
        &self,
        dados: &ConversaParaSala,
    ) -> impl std::future::Future<Output = Result<String, ErroSala>> + Send {
        let sala = format!("!{}:matrix-dev.local", dados.conversa);
        self.0.chamadas_sala.fetch_add(1, Ordering::SeqCst);
        async move { Ok(sala) }
    }

    fn garantir_aparelho(
        &self,
        _dados: &ConversaParaSala,
        sala: &str,
        aparelho: &Fantasma,
    ) -> impl std::future::Future<Output = Result<(), ErroSala>> + Send {
        let falso = &self.0;
        falso.chamadas_aparelho.fetch_add(1, Ordering::SeqCst);
        let falhar = *falso.falha_aparelho.lock().unwrap();
        if !falhar {
            falso
                .membros
                .lock()
                .unwrap()
                .insert((sala.to_owned(), aparelho.user_id().to_owned()));
        }
        async move {
            if falhar {
                Err(ErroSala::Chamada(transitorio()))
            } else {
                Ok(())
            }
        }
    }

    fn enviar(
        &self,
        sala: &str,
        envio: &Envio,
    ) -> impl std::future::Future<Output = Result<(), ErroChamada>> + Send {
        let falso = &self.0;
        let n = falso.chamadas_envio.fetch_add(1, Ordering::SeqCst) + 1;
        let modo = *falso.modo.lock().unwrap();
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
        // Remetente que não está na sala é recusado, como no Synapse real (MX-42).
        let fora_da_sala = envio.origem == OrigemMensagem::Dispositivo
            && !falso
                .membros
                .lock()
                .unwrap()
                .contains(&(sala.clone(), remetente.clone()));
        let resultado = match modo {
            _ if fora_da_sala => Err(recusado()),
            Modo::Fora => Err(transitorio()),
            Modo::Venenoso(texto) if corpo.contains(texto) => Err(recusado()),
            Modo::FalhaNaChamada(k) if n == k => {
                // Uma vez só: depois da queda, o Synapse volta a aceitar.
                *falso.modo.lock().unwrap() = Modo::Normal;
                Err(transitorio())
            }
            _ => Ok(()),
        };
        let mut criados = falso.criados.lock().unwrap();
        if resultado.is_ok() && !criados.iter().any(|c| c.sala == sala && c.txn == txn) {
            // Como o Synapse: o mesmo `txnId` na mesma sala não cria outro evento.
            criados.push(Criado {
                sala,
                txn,
                corpo,
                remetente,
            });
        }
        drop(criados);
        async move { resultado }
    }

    fn enviar_midia(
        &self,
        sala: &str,
        envio: &EnvioMidia,
    ) -> impl std::future::Future<Output = Result<MidiaPublicada, ErroChamada>> + Send {
        let falso = &self.0;
        let n = falso.chamadas_envio.fetch_add(1, Ordering::SeqCst) + 1;
        let modo = *falso.modo.lock().unwrap();
        let sala = sala.to_owned();
        let txn = envio.midia.txn_id.clone();
        let corpo = envio.midia.nome.clone();
        let remetente = envio.remetente.user_id().to_owned();
        *falso
            .tentativas
            .lock()
            .unwrap()
            .entry(txn.clone())
            .or_insert(0) += 1;
        let resultado = match modo {
            Modo::Fora => Err(transitorio()),
            Modo::Venenoso(texto) if corpo.contains(texto) => Err(recusado()),
            Modo::FalhaNaChamada(k) if n == k => {
                *falso.modo.lock().unwrap() = Modo::Normal;
                Err(transitorio())
            }
            _ => Ok(()),
        };
        let mut criados = falso.criados.lock().unwrap();
        let novo = !criados.iter().any(|c| c.sala == sala && c.txn == txn);
        if resultado.is_ok() && novo {
            criados.push(Criado {
                sala: sala.clone(),
                txn: txn.clone(),
                corpo,
                remetente,
            });
        }
        drop(criados);
        // Como o destino real: o `mxc` sai do `txnId` do anexo e o evento é numerado na sala.
        let publicada = MidiaPublicada {
            evento: format!("$midia-{txn}:matrix-dev.local"),
            mxc: format!("mxc://matrix-dev.local/{txn}"),
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
        let corpo = poph_matrix::midia::corpo_do_aviso(aviso.motivo);
        let remetente = aviso.remetente.user_id().to_owned();
        let mut criados = falso.criados.lock().unwrap();
        if !criados.iter().any(|c| c.sala == sala && c.txn == txn) {
            criados.push(Criado {
                sala,
                txn,
                corpo,
                remetente,
            });
        }
        drop(criados);
        async move { Ok(()) }
    }
}

fn espelho(store: Arc<Store>, destino: Arc<Falso>) -> EspelhoMatrix<Sinal> {
    EspelhoMatrix::novo(store, Sinal(destino), &config()).expect("monta o espelho")
}

fn sala_de(conversa: ConversationId) -> String {
    format!("!{conversa}:matrix-dev.local")
}

// ───────────────────────────── testes ─────────────────────────────

#[tokio::test]
async fn ordem_dentro_da_sala_e_paralelo_entre_salas() {
    let cenario = Cenario::novo("ordem");
    let store = cenario.store();
    let org = criar_org(&store, "a");
    let agente = criar_agente(&store, org);
    let ana = criar_conversa(&store, org, agente, "+5511111");
    let bruno = criar_conversa(&store, org, agente, "+5522222");
    store.matrix().ligar(org).expect("liga");

    let mut esperado_ana = Vec::new();
    let mut esperado_bruno = Vec::new();
    for i in 0..6 {
        if i % 2 == 0 {
            esperado_ana.push(txn_de(&msg_cliente(&store, org, ana, &format!("ana {i}"))));
        } else {
            esperado_bruno.push(txn_de(&msg_cliente(
                &store,
                org,
                bruno,
                &format!("bruno {i}"),
            )));
        }
    }

    let destino = Falso::novo(Modo::Normal);
    let relatorio = espelho(Arc::clone(&store), Arc::clone(&destino))
        .ciclo()
        .await
        .expect("ciclo");

    assert_eq!(relatorio.enviadas, 6);
    assert_eq!(destino.txns_da_sala(&sala_de(ana)), esperado_ana);
    assert_eq!(destino.txns_da_sala(&sala_de(bruno)), esperado_bruno);
}

#[tokio::test]
async fn queda_no_meio_nao_pula_nem_duplica() {
    let cenario = Cenario::novo("queda");
    let store = cenario.store();
    let org = criar_org(&store, "a");
    let agente = criar_agente(&store, org);
    let ana = criar_conversa(&store, org, agente, "+5511111");
    let bruno = criar_conversa(&store, org, agente, "+5522222");
    store.matrix().ligar(org).expect("liga");

    let mut todas = Vec::new();
    for i in 0..6 {
        let conversa = if i % 2 == 0 { ana } else { bruno };
        todas.push(txn_de(&msg_cliente(
            &store,
            org,
            conversa,
            &format!("msg {i}"),
        )));
    }

    let destino = Falso::novo(Modo::FalhaNaChamada(3));
    let espelho = espelho(Arc::clone(&store), Arc::clone(&destino));

    let primeiro = espelho.ciclo().await.expect("ciclo 1");
    assert!(primeiro.pendentes >= 1, "a falha no meio deixa pendência");
    // O cursor não passou da primeira linha não resolvida.
    let cursor = store.matrix().ler_cursores(org).expect("cursor").mensagens;
    assert!(cursor < 6, "cursor não pode andar além da falha: {cursor}");

    espelho.ciclo().await.expect("ciclo 2");
    espelho.ciclo().await.expect("ciclo 3");

    let criados: Vec<String> = destino.criados().into_iter().map(|c| c.txn).collect();
    assert_eq!(criados.len(), 6, "todas entregues uma vez");
    for txn in &todas {
        assert!(criados.contains(txn), "nenhuma linha pulada: {txn}");
    }
    let unicos: std::collections::HashSet<_> = criados.iter().collect();
    assert_eq!(unicos.len(), 6, "nenhum evento duplicado");
    let repetida = todas
        .iter()
        .filter(|txn| destino.tentativas_de(txn) >= 2)
        .count();
    assert!(
        repetida >= 1,
        "a mensagem da queda foi reenviada com o mesmo txnId"
    );
}

#[tokio::test]
async fn mensagem_venenosa_nao_trava_a_fila() {
    let cenario = Cenario::novo("venenosa");
    let store = cenario.store();
    let org = criar_org(&store, "a");
    let agente = criar_agente(&store, org);
    let conversa = criar_conversa(&store, org, agente, "+5511111");
    store.matrix().ligar(org).expect("liga");

    let antes = txn_de(&msg_cliente(&store, org, conversa, "primeira"));
    let veneno = txn_de(&msg_cliente(&store, org, conversa, "VENENO aqui"));
    let depois = txn_de(&msg_cliente(&store, org, conversa, "terceira"));
    let ultima = txn_de(&msg_cliente(&store, org, conversa, "quarta"));

    let destino = Falso::novo(Modo::Venenoso("VENENO"));
    let espelho = espelho(Arc::clone(&store), Arc::clone(&destino));

    // Até o limite de tentativas, a venenosa fica pendente e a fila espera.
    for _ in 0..(TENTATIVAS_DEFINITIVAS - 1) {
        espelho.ciclo().await.expect("ciclo");
    }
    assert!(!destino.txns_da_sala(&sala_de(conversa)).contains(&depois));

    // Na tentativa limite, ela é isolada e a fila anda.
    espelho.ciclo().await.expect("ciclo limite");
    espelho.ciclo().await.expect("ciclo seguinte");

    let enviadas = destino.txns_da_sala(&sala_de(conversa));
    assert!(enviadas.contains(&antes));
    assert!(enviadas.contains(&depois));
    assert!(enviadas.contains(&ultima));
    assert!(!enviadas.contains(&veneno));

    let falhas = store.matrix().listar_falhas(org, 10).expect("falhas");
    assert_eq!(falhas.len(), 1, "a venenosa foi isolada uma vez");
    assert_eq!(falhas[0].motivo, "M_FORBIDDEN");
    assert_eq!(falhas[0].tentativas, TENTATIVAS_DEFINITIVAS);
}

#[tokio::test]
async fn synapse_fora_acumula_sem_avancar_e_depois_drena() {
    let cenario = Cenario::novo("fora");
    let store = cenario.store();
    let org = criar_org(&store, "a");
    let agente = criar_agente(&store, org);
    let conversa = criar_conversa(&store, org, agente, "+5511111");
    store.matrix().ligar(org).expect("liga");

    let mut todas = Vec::new();
    for i in 0..5 {
        todas.push(txn_de(&msg_cliente(
            &store,
            org,
            conversa,
            &format!("m{i}"),
        )));
    }

    let destino = Falso::novo(Modo::Fora);
    let espelho = espelho(Arc::clone(&store), Arc::clone(&destino));
    for _ in 0..3 {
        let relatorio = espelho.ciclo().await.expect("ciclo");
        assert_eq!(relatorio.enviadas, 0);
        assert!(relatorio.pendentes >= 1);
    }
    assert_eq!(
        store.matrix().ler_cursores(org).expect("cursor").mensagens,
        0
    );
    assert!(
        espelho.atraso_apos(3) > espelho.atraso_apos(1),
        "o atraso cresce"
    );

    destino.definir_modo(Modo::Normal);
    espelho.ciclo().await.expect("ciclo de volta");
    let criados: Vec<String> = destino.criados().into_iter().map(|c| c.txn).collect();
    assert_eq!(criados, todas, "tudo entregue, na ordem, uma vez");
}

#[tokio::test]
async fn org_desligada_nao_gera_chamada_alguma() {
    let cenario = Cenario::novo("desligada");
    let store = cenario.store();
    let ligada = criar_org(&store, "ligada");
    let desligada = criar_org(&store, "desligada");
    let agente_l = criar_agente(&store, ligada);
    let agente_d = criar_agente(&store, desligada);
    let conv_l = criar_conversa(&store, ligada, agente_l, "+5511111");
    let conv_d = criar_conversa(&store, desligada, agente_d, "+5522222");
    store.matrix().ligar(ligada).expect("liga");

    msg_cliente(&store, ligada, conv_l, "da ligada");
    msg_cliente(&store, desligada, conv_d, "da desligada");
    msg_cliente(&store, desligada, conv_d, "de novo na desligada");

    let destino = Falso::novo(Modo::Normal);
    let relatorio = espelho(Arc::clone(&store), Arc::clone(&destino))
        .ciclo()
        .await
        .expect("ciclo");

    assert_eq!(relatorio.enviadas, 1);
    assert_eq!(relatorio.de_org_desligada, 2);
    assert_eq!(destino.chamadas_sala.load(Ordering::SeqCst), 1);
    assert!(destino.criados().iter().all(|c| c.sala == sala_de(conv_l)));
}

#[tokio::test]
async fn mensagem_anterior_ao_corte_nao_e_espelhada() {
    let cenario = Cenario::novo("corte");
    let store = cenario.store();
    let org = criar_org(&store, "a");
    let agente = criar_agente(&store, org);
    let conversa = criar_conversa(&store, org, agente, "+5511111");

    msg_cliente(&store, org, conversa, "antes de ligar");
    store.matrix().ligar(org).expect("liga");
    let depois = txn_de(&msg_cliente(&store, org, conversa, "depois de ligar"));

    let destino = Falso::novo(Modo::Normal);
    let relatorio = espelho(Arc::clone(&store), Arc::clone(&destino))
        .ciclo()
        .await
        .expect("ciclo");

    assert_eq!(relatorio.antes_do_corte, 1);
    assert_eq!(destino.txns_da_sala(&sala_de(conversa)), vec![depois]);
}

#[tokio::test]
async fn resposta_humana_e_ferramenta_nao_viram_evento() {
    let cenario = Cenario::novo("nao-espelha");
    let store = cenario.store();
    let org = criar_org(&store, "a");
    let agente = criar_agente(&store, org);
    let conversa = criar_conversa(&store, org, agente, "+5511111");
    store.matrix().ligar(org).expect("liga");

    gravar(
        &store,
        org,
        conversa,
        Direction::Outbound,
        Role::Assistant,
        Origin::Human,
        "vindo do Element",
    );
    gravar(
        &store,
        org,
        conversa,
        Direction::Outbound,
        Role::System,
        Origin::Agente,
        "instrução",
    );
    msg_cliente(&store, org, conversa, "   ");
    let normal = txn_de(&msg_agente(&store, org, conversa, "resposta do agente"));

    let destino = Falso::novo(Modo::Normal);
    let relatorio = espelho(Arc::clone(&store), Arc::clone(&destino))
        .ciclo()
        .await
        .expect("ciclo");

    assert_eq!(relatorio.enviadas, 1);
    assert_eq!(destino.txns_da_sala(&sala_de(conversa)), vec![normal]);
}

#[tokio::test]
async fn rowid_reaproveitado_nao_perde_a_mensagem_nova() {
    let cenario = Cenario::novo("rowid");
    let store = cenario.store();
    let org = criar_org(&store, "a");
    let agente = criar_agente(&store, org);
    let conversa = criar_conversa(&store, org, agente, "+5511111");
    store.matrix().ligar(org).expect("liga");

    msg_cliente(&store, org, conversa, "um");
    msg_cliente(&store, org, conversa, "dois");
    let ultima = msg_cliente(&store, org, conversa, "tres");

    let destino = Falso::novo(Modo::Normal);
    let espelho = espelho(Arc::clone(&store), Arc::clone(&destino));
    espelho.ciclo().await.expect("ciclo 1");
    assert_eq!(destino.criados().len(), 3);

    // Apaga a linha de maior rowid (já espelhada) e grava outra: o SQLite reaproveita o rowid.
    let rowid_antigo: i64 = cenario
        .conexao()
        .query_row(
            "SELECT rowid FROM messages WHERE id = ?1",
            [ultima.as_str()],
            |linha| linha.get(0),
        )
        .expect("rowid");
    cenario
        .conexao()
        .execute("DELETE FROM messages WHERE id = ?1", [ultima.as_str()])
        .expect("apaga");
    let nova = msg_cliente(&store, org, conversa, "quatro, com rowid antigo");
    let rowid_novo: i64 = cenario
        .conexao()
        .query_row(
            "SELECT rowid FROM messages WHERE id = ?1",
            [nova.as_str()],
            |linha| linha.get(0),
        )
        .expect("rowid novo");
    assert_eq!(rowid_novo, rowid_antigo, "o rowid foi reaproveitado");

    espelho.ciclo().await.expect("ciclo 2");
    assert!(
        destino
            .txns_da_sala(&sala_de(conversa))
            .contains(&txn_de(&nova)),
        "a mensagem com rowid reaproveitado sai"
    );
}

#[tokio::test]
async fn cursor_a_frente_do_maior_rowid_volta_ao_ponto_seguro() {
    let cenario = Cenario::novo("recuo");
    let store = cenario.store();
    let org = criar_org(&store, "a");
    let agente = criar_agente(&store, org);
    let conversa = criar_conversa(&store, org, agente, "+5511111");
    store.matrix().ligar(org).expect("liga");
    msg_cliente(&store, org, conversa, "um");
    msg_cliente(&store, org, conversa, "dois");

    // Cursor inventado, à frente de tudo que existe.
    store
        .matrix()
        .avancar_mensagens_com_id(org, 99, "00000000-0000-7000-8000-000000000000")
        .expect("cursor");

    let destino = Falso::novo(Modo::Normal);
    let espelho = espelho(Arc::clone(&store), Arc::clone(&destino));
    let relatorio = espelho.ciclo().await.expect("ciclo");
    assert_eq!(relatorio.recuos, 1);
    assert!(store.matrix().ler_cursores(org).expect("cursor").mensagens <= 2);

    // Mensagem nova depois do recuo é lida normalmente.
    let nova = txn_de(&msg_cliente(&store, org, conversa, "tres"));
    espelho.ciclo().await.expect("ciclo 2");
    assert!(destino.txns_da_sala(&sala_de(conversa)).contains(&nova));
}

#[tokio::test]
async fn memoria_do_ciclo_fica_limitada_a_uma_pagina() {
    let cenario = Cenario::novo("memoria");
    let store = cenario.store();
    let org = criar_org(&store, "a");
    let agente = criar_agente(&store, org);
    let conversa = criar_conversa(&store, org, agente, "+5511111");
    store.matrix().ligar(org).expect("liga");

    let total = PAGINA as usize * 3 + 17;
    for i in 0..total {
        msg_cliente(&store, org, conversa, &format!("m{i}"));
    }

    let destino = Falso::novo(Modo::Normal);
    let relatorio = espelho(Arc::clone(&store), Arc::clone(&destino))
        .ciclo()
        .await
        .expect("ciclo");
    assert!(
        relatorio.maior_pagina <= PAGINA,
        "nunca mais que uma página em memória"
    );
    assert_eq!(
        relatorio.enviadas as usize,
        total.min(PAGINA as usize * MAX_PAGINAS_POR_CICLO as usize)
    );
}

#[tokio::test]
async fn atraso_cresce_com_falhas_e_para_no_teto() {
    let cenario = Cenario::novo("atraso");
    let espelho = espelho(cenario.store(), Falso::novo(Modo::Normal));
    assert_eq!(espelho.atraso_apos(0), Duration::from_millis(500));
    assert_eq!(espelho.atraso_apos(1), Duration::from_millis(1000));
    assert_eq!(espelho.atraso_apos(2), Duration::from_millis(2000));
    assert_eq!(espelho.atraso_apos(40), TETO_ATRASO);
}

#[tokio::test]
async fn parada_encerra_o_laco() {
    let cenario = Cenario::novo("parada");
    let store = cenario.store();
    let espelho = Arc::new(espelho(Arc::clone(&store), Falso::novo(Modo::Normal)));
    let tarefa = tokio::spawn(Arc::clone(&espelho).executar());
    tokio::time::sleep(Duration::from_millis(50)).await;
    espelho.pedir_parada();
    let resultado = tokio::time::timeout(Duration::from_secs(5), tarefa).await;
    assert!(resultado.is_ok(), "o laço sai depois do pedido de parada");
}

/// Cinquenta mil mensagens pendentes com o Synapse fora do ar: o ciclo lê uma página, para na
/// primeira falha e não carrega a fila inteira. Lento (grava 50 mil linhas): roda com `--ignored`.
#[tokio::test]
#[ignore = "grava 50 mil mensagens; rodar com --ignored"]
async fn memoria_com_50_mil_pendentes() {
    let cenario = Cenario::novo("cinquenta-mil");
    let store = cenario.store();
    let org = criar_org(&store, "a");
    let agente = criar_agente(&store, org);
    let conversa = criar_conversa(&store, org, agente, "+5511111");
    store.matrix().ligar(org).expect("liga");
    let inicio = std::time::Instant::now();
    for i in 0..50_000 {
        msg_cliente(&store, org, conversa, &format!("m{i}"));
    }
    eprintln!("gravação de 50 mil: {:?}", inicio.elapsed());

    let destino = Falso::novo(Modo::Fora);
    let inicio = std::time::Instant::now();
    let relatorio = espelho(Arc::clone(&store), Arc::clone(&destino))
        .ciclo()
        .await
        .expect("ciclo");
    eprintln!(
        "ciclo com 50 mil pendentes: {:?}, {relatorio:?}",
        inicio.elapsed()
    );
    assert_eq!(relatorio.enviadas, 0);
    assert!(relatorio.maior_pagina <= PAGINA);
    assert_eq!(relatorio.paginas, 1, "para na primeira página com falha");
    assert_eq!(
        store.matrix().ler_cursores(org).expect("cursor").mensagens,
        0
    );
}

// ───────────────────────────── MX-42: mensagem do aparelho ─────────────────────────────

/// Peer do cliente nos testes do aparelho (o mesmo em toda a conversa).
const PEER_APARELHO: &str = "5511999990000";

/// Org com o módulo ligado, um agente e a integração do WhatsApp (a instância da Evolution).
fn org_com_aparelho(store: &Store) -> (OrgId, AgentId, IntegrationId) {
    let org = criar_org(store, "ap");
    let agente = criar_agente(store, org);
    let integracao: IntegrationId = store
        .create_integration(org, agente, "whatsapp".to_owned(), "active".to_owned())
        .expect("cria a integração")
        .id
        .parse()
        .expect("id da integração");
    store.matrix().ligar(org).expect("liga");
    (org, agente, integracao)
}

/// Mensagem digitada no celular da loja, pelo caminho da MX-41 (`fromMe` gravado como do aparelho).
fn receber_aparelho(
    store: &Store,
    org: OrgId,
    agente: AgentId,
    integracao: IntegrationId,
    id_externo: &str,
    texto: &str,
) -> (ConversationId, MessageId) {
    let recebido = store
        .inbox()
        .receber_do_aparelho(
            org,
            NewDeviceMessage {
                integration_id: integracao,
                agent_id: agente,
                canal: "whatsapp".to_owned(),
                peer: PEER_APARELHO.to_owned(),
                id_externo: id_externo.to_owned(),
                texto: texto.to_owned(),
            },
        )
        .expect("recebe a mensagem do aparelho");
    (recebido.conversa, recebido.mensagem.id)
}

/// Id do usuário do aparelho desta integração, como o espelho monta.
fn fantasma_do_aparelho(integracao: IntegrationId) -> String {
    IdsFantasmas::novo("ph", "matrix-dev.local")
        .expect("ids")
        .aparelho(&integracao.to_string())
        .expect("id do aparelho")
        .user_id()
        .to_owned()
}

fn contar_outbox(cenario: &Cenario) -> i64 {
    cenario
        .conexao()
        .query_row("SELECT COUNT(*) FROM outbox", [], |linha| linha.get(0))
        .expect("conta a fila de saída")
}

#[tokio::test]
async fn mensagem_do_aparelho_entra_na_sala_pelo_usuario_do_aparelho() {
    let cenario = Cenario::novo("aparelho-sala");
    let store = cenario.store();
    let (org, agente, integracao) = org_com_aparelho(&store);
    let (conversa, mensagem) = receber_aparelho(
        &store,
        org,
        agente,
        integracao,
        "AP-1",
        "Já enviei, obrigado",
    );
    // O nome que o WhatsApp deu ao cliente: a mensagem do aparelho não pode trocá-lo.
    store
        .conversas()
        .set_customer_name(org, conversa, "Maria")
        .expect("grava o nome do cliente");
    // Uma resposta do cliente na mesma conversa, para provar que cada um fala com o próprio usuário.
    msg_cliente(&store, org, conversa, "oi, tudo bem?");

    let destino = Falso::novo(Modo::Normal);
    let outbox_antes = contar_outbox(&cenario);
    let relatorio = espelho(Arc::clone(&store), Arc::clone(&destino))
        .ciclo()
        .await
        .expect("ciclo");

    assert_eq!(relatorio.enviadas, 2, "a do cliente e a do aparelho");
    let do_aparelho = destino.criados_do(&fantasma_do_aparelho(integracao));
    assert_eq!(do_aparelho.len(), 1, "o usuário do aparelho fala uma vez");
    assert_eq!(do_aparelho[0].txn, txn_de(&mensagem.to_string()));
    assert_eq!(do_aparelho[0].corpo, "Já enviei, obrigado");
    assert_eq!(do_aparelho[0].sala, sala_de(conversa));
    assert_eq!(
        destino
            .criados_do("@ph_wa_5511999990000:matrix-dev.local")
            .len(),
        1,
        "a fala do cliente continua saindo pelo usuário do cliente"
    );
    assert_eq!(
        destino.chamadas_aparelho.load(Ordering::SeqCst),
        1,
        "o aparelho entrou na sala uma vez"
    );
    assert_eq!(
        contar_outbox(&cenario),
        outbox_antes,
        "o espelho não cria linha na fila de saída: a mensagem já saiu pelo celular"
    );
    let nome = store
        .conversas()
        .get(org, conversa)
        .expect("lê a conversa")
        .customer_name;
    assert_eq!(nome.as_deref(), Some("Maria"), "o nome do cliente não muda");
}

#[tokio::test]
async fn aparelho_entra_uma_vez_por_passagem_e_repeticao_nao_duplica() {
    let cenario = Cenario::novo("aparelho-repeticao");
    let store = cenario.store();
    let (org, agente, integracao) = org_com_aparelho(&store);
    let (conversa, primeira) =
        receber_aparelho(&store, org, agente, integracao, "AP-1", "primeira");
    let (_, segunda) = receber_aparelho(&store, org, agente, integracao, "AP-2", "segunda");
    assert_eq!(
        store.conversas().get(org, conversa).expect("conversa").id,
        conversa,
        "as duas mensagens caem na mesma conversa"
    );

    let destino = Falso::novo(Modo::Normal);
    let espelho = espelho(Arc::clone(&store), Arc::clone(&destino));
    let ciclo = espelho.ciclo().await.expect("ciclo 1");
    assert_eq!(ciclo.enviadas, 2);
    assert_eq!(
        destino.chamadas_aparelho.load(Ordering::SeqCst),
        1,
        "um grupo da sala, um pedido de entrada"
    );

    // Passagem repetida do laço (como depois de uma queda): lê tudo de novo a partir do início.
    store
        .matrix()
        .recuar_cursor_mensagens(org, 0, "")
        .expect("recua o cursor");
    espelho.ciclo().await.expect("ciclo 2");

    let do_aparelho = destino.criados_do(&fantasma_do_aparelho(integracao));
    assert_eq!(do_aparelho.len(), 2, "cada mensagem aparece uma vez");
    for id in [primeira, segunda] {
        assert!(
            destino.tentativas_de(&txn_de(&id.to_string())) >= 2,
            "a mensagem foi reenviada com o mesmo txnId"
        );
    }
}

#[tokio::test]
async fn aparelho_fora_da_sala_adia_a_mensagem_sem_perde_la() {
    let cenario = Cenario::novo("aparelho-adia");
    let store = cenario.store();
    let (org, agente, integracao) = org_com_aparelho(&store);
    let (_, mensagem) = receber_aparelho(&store, org, agente, integracao, "AP-1", "oi do celular");

    let destino = Falso::novo(Modo::Normal);
    destino.definir_falha_aparelho(true);
    let espelho = espelho(Arc::clone(&store), Arc::clone(&destino));
    let primeiro = espelho.ciclo().await.expect("ciclo 1");
    assert_eq!(primeiro.enviadas, 0, "sem entrar na sala, nada é enviado");
    assert!(primeiro.pendentes >= 1, "a mensagem fica pendente");
    assert!(destino.criados().is_empty());

    // A mensagem continua gravada no Harness; quando a sala responder, sai uma vez.
    destino.definir_falha_aparelho(false);
    let segundo = espelho.ciclo().await.expect("ciclo 2");
    assert_eq!(segundo.enviadas, 1);
    let do_aparelho = destino.criados_do(&fantasma_do_aparelho(integracao));
    assert_eq!(do_aparelho.len(), 1);
    assert_eq!(do_aparelho[0].txn, txn_de(&mensagem.to_string()));
}
