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

use poph_matrix::error::CallError;
use poph_matrix::format::{MessageOrigin, Outbound};
use poph_matrix::rooms::{ConversationToRoom, RoomError};
use poph_matrix::users::Ghost;
use poph_matrix::users::GhostIds;
use poph_server::matrix_mirror::{
    AssetSend, DEFINITIVE_ATTEMPTS, MAX_DELAY, MAX_PAGES_PER_CYCLE, MatrixMirror, MatrixTarget,
    PAGE, PublishedAsset, RoomNotice,
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
    fn new(rotulo: &str) -> Self {
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

fn criar_conversa(store: &Store, org: OrgId, agent: AgentId, peer: &str) -> ConversationId {
    store
        .conversations()
        .create(
            org,
            NewConversation {
                agent_id: agent,
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
fn msg_cliente(store: &Store, org: OrgId, conversation: ConversationId, text: &str) -> String {
    gravar(
        store,
        org,
        conversation,
        Direction::Inbound,
        Role::User,
        Origin::Agente,
        text,
    )
}

/// Resposta do agente (saída, papel `assistant`).
fn msg_agente(store: &Store, org: OrgId, conversation: ConversationId, text: &str) -> String {
    gravar(
        store,
        org,
        conversation,
        Direction::Outbound,
        Role::Assistant,
        Origin::Agente,
        text,
    )
}

fn gravar(
    store: &Store,
    org: OrgId,
    conversation: ConversationId,
    direcao: Direction,
    role: Role,
    origin: Origin,
    text: &str,
) -> String {
    store
        .messages()
        .write(
            org,
            NewMessage {
                conversation_id: conversation,
                direction: direcao,
                role,
                origin,
                author_name: None,
                texto: text.to_owned(),
                attachments_json: "[]".to_owned(),
                external_id: None,
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
    room: String,
    txn: String,
    body: String,
    sender: String,
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
    members: Mutex<std::collections::HashSet<(String, String)>>,
    /// Quando `true`, pôr o aparelho na sala falha (Synapse fora do ar nessa etapa).
    falha_aparelho: Mutex<bool>,
}

impl Falso {
    fn new(modo: Modo) -> Arc<Self> {
        Arc::new(Self {
            modo: Mutex::new(modo),
            criados: Mutex::new(Vec::new()),
            tentativas: Mutex::new(HashMap::new()),
            chamadas_sala: AtomicUsize::new(0),
            chamadas_envio: AtomicUsize::new(0),
            chamadas_aparelho: AtomicUsize::new(0),
            members: Mutex::new(std::collections::HashSet::new()),
            falha_aparelho: Mutex::new(false),
        })
    }

    fn definir_falha_aparelho(&self, falha: bool) {
        *self.falha_aparelho.lock().unwrap() = falha;
    }

    fn criados_do(&self, sender: &str) -> Vec<Criado> {
        self.criados()
            .into_iter()
            .filter(|criado| criado.sender == sender)
            .collect()
    }

    fn definir_modo(&self, modo: Modo) {
        *self.modo.lock().unwrap() = modo;
    }

    fn criados(&self) -> Vec<Criado> {
        self.criados.lock().unwrap().clone()
    }

    fn txns_da_sala(&self, room: &str) -> Vec<String> {
        self.criados()
            .into_iter()
            .filter(|criado| criado.room == room)
            .map(|criado| criado.txn)
            .collect()
    }

    fn tentativas_de(&self, txn: &str) -> u32 {
        *self.tentativas.lock().unwrap().get(txn).unwrap_or(&0)
    }
}

fn transient() -> CallError {
    CallError::Response {
        operation: "send",
        status: 503,
        errcode: Some("M_UNKNOWN".to_owned()),
        message: "indisponível".to_owned(),
        retry_after_ms: None,
    }
}

fn recusado() -> CallError {
    CallError::Response {
        operation: "send",
        status: 403,
        errcode: Some("M_FORBIDDEN".to_owned()),
        message: "recusado".to_owned(),
        retry_after_ms: None,
    }
}

/// O duplo compartilhado com o teste: o espelho fica com uma cópia do `Arc`, o teste com outra.
struct Sinal(Arc<Falso>);

impl MatrixTarget for Sinal {
    fn ensure_room(
        &self,
        dados: &ConversationToRoom,
    ) -> impl std::future::Future<Output = Result<String, RoomError>> + Send {
        let room = format!("!{}:matrix-dev.local", dados.conversation);
        self.0.chamadas_sala.fetch_add(1, Ordering::SeqCst);
        async move { Ok(room) }
    }

    fn ensure_device(
        &self,
        _dados: &ConversationToRoom,
        room: &str,
        device: &Ghost,
    ) -> impl std::future::Future<Output = Result<(), RoomError>> + Send {
        let falso = &self.0;
        falso.chamadas_aparelho.fetch_add(1, Ordering::SeqCst);
        let falhar = *falso.falha_aparelho.lock().unwrap();
        if !falhar {
            falso
                .members
                .lock()
                .unwrap()
                .insert((room.to_owned(), device.user_id().to_owned()));
        }
        async move {
            if falhar {
                Err(RoomError::Call(transient()))
            } else {
                Ok(())
            }
        }
    }

    fn send(
        &self,
        room: &str,
        send: &Outbound,
    ) -> impl std::future::Future<Output = Result<(), CallError>> + Send {
        let falso = &self.0;
        let n = falso.chamadas_envio.fetch_add(1, Ordering::SeqCst) + 1;
        let modo = *falso.modo.lock().unwrap();
        let room = room.to_owned();
        let txn = send.txn_id.clone();
        let body = send.body.clone();
        let sender = send.sender.user_id().to_owned();
        *falso
            .tentativas
            .lock()
            .unwrap()
            .entry(txn.clone())
            .or_insert(0) += 1;
        // Remetente que não está na sala é recusado, como no Synapse real (MX-42).
        let fora_da_sala = send.origin == MessageOrigin::Device
            && !falso
                .members
                .lock()
                .unwrap()
                .contains(&(room.clone(), sender.clone()));
        let resultado = match modo {
            _ if fora_da_sala => Err(recusado()),
            Modo::Fora => Err(transient()),
            Modo::Venenoso(text) if body.contains(text) => Err(recusado()),
            Modo::FalhaNaChamada(k) if n == k => {
                // Uma vez só: depois da queda, o Synapse volta a aceitar.
                *falso.modo.lock().unwrap() = Modo::Normal;
                Err(transient())
            }
            _ => Ok(()),
        };
        let mut criados = falso.criados.lock().unwrap();
        if resultado.is_ok() && !criados.iter().any(|c| c.room == room && c.txn == txn) {
            // Como o Synapse: o mesmo `txnId` na mesma sala não cria outro evento.
            criados.push(Criado {
                room,
                txn,
                body,
                sender,
            });
        }
        drop(criados);
        async move { resultado }
    }

    fn send_asset(
        &self,
        room: &str,
        send: &AssetSend,
    ) -> impl std::future::Future<Output = Result<PublishedAsset, CallError>> + Send {
        let falso = &self.0;
        let n = falso.chamadas_envio.fetch_add(1, Ordering::SeqCst) + 1;
        let modo = *falso.modo.lock().unwrap();
        let room = room.to_owned();
        let txn = send.asset.txn_id.clone();
        let body = send.asset.name.clone();
        let sender = send.sender.user_id().to_owned();
        *falso
            .tentativas
            .lock()
            .unwrap()
            .entry(txn.clone())
            .or_insert(0) += 1;
        let resultado = match modo {
            Modo::Fora => Err(transient()),
            Modo::Venenoso(text) if body.contains(text) => Err(recusado()),
            Modo::FalhaNaChamada(k) if n == k => {
                *falso.modo.lock().unwrap() = Modo::Normal;
                Err(transient())
            }
            _ => Ok(()),
        };
        let mut criados = falso.criados.lock().unwrap();
        let new = !criados.iter().any(|c| c.room == room && c.txn == txn);
        if resultado.is_ok() && new {
            criados.push(Criado {
                room: room.clone(),
                txn: txn.clone(),
                body,
                sender,
            });
        }
        drop(criados);
        // Como o destino real: o `mxc` sai do `txnId` do anexo e o evento é numerado na sala.
        let publicada = PublishedAsset {
            event: format!("$midia-{txn}:matrix-dev.local"),
            mxc: format!("mxc://matrix-dev.local/{txn}"),
        };
        async move { resultado.map(|()| publicada) }
    }

    fn send_notice(
        &self,
        room: &str,
        aviso: &RoomNotice,
    ) -> impl std::future::Future<Output = Result<(), CallError>> + Send {
        let falso = &self.0;
        let room = room.to_owned();
        let txn = aviso.txn_id.clone();
        let body = poph_matrix::asset::notice_body(aviso.reason);
        let sender = aviso.sender.user_id().to_owned();
        let mut criados = falso.criados.lock().unwrap();
        if !criados.iter().any(|c| c.room == room && c.txn == txn) {
            criados.push(Criado {
                room,
                txn,
                body,
                sender,
            });
        }
        drop(criados);
        async move { Ok(()) }
    }
}

fn espelho(store: Arc<Store>, destino: Arc<Falso>) -> MatrixMirror<Sinal> {
    MatrixMirror::new(store, Sinal(destino), &config()).expect("monta o espelho")
}

fn sala_de(conversation: ConversationId) -> String {
    format!("!{conversation}:matrix-dev.local")
}

// ───────────────────────────── testes ─────────────────────────────

#[tokio::test]
async fn ordem_dentro_da_sala_e_paralelo_entre_salas() {
    let cenario = Cenario::new("ordem");
    let store = cenario.store();
    let org = criar_org(&store, "a");
    let agent = criar_agente(&store, org);
    let ana = criar_conversa(&store, org, agent, "+5511111");
    let bruno = criar_conversa(&store, org, agent, "+5522222");
    store.matrix().enable(org).expect("liga");

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

    let destino = Falso::new(Modo::Normal);
    let relatorio = espelho(Arc::clone(&store), Arc::clone(&destino))
        .cycle()
        .await
        .expect("ciclo");

    assert_eq!(relatorio.sent, 6);
    assert_eq!(destino.txns_da_sala(&sala_de(ana)), esperado_ana);
    assert_eq!(destino.txns_da_sala(&sala_de(bruno)), esperado_bruno);
}

#[tokio::test]
async fn queda_no_meio_nao_pula_nem_duplica() {
    let cenario = Cenario::new("queda");
    let store = cenario.store();
    let org = criar_org(&store, "a");
    let agent = criar_agente(&store, org);
    let ana = criar_conversa(&store, org, agent, "+5511111");
    let bruno = criar_conversa(&store, org, agent, "+5522222");
    store.matrix().enable(org).expect("liga");

    let mut todas = Vec::new();
    for i in 0..6 {
        let conversation = if i % 2 == 0 { ana } else { bruno };
        todas.push(txn_de(&msg_cliente(
            &store,
            org,
            conversation,
            &format!("msg {i}"),
        )));
    }

    let destino = Falso::new(Modo::FalhaNaChamada(3));
    let espelho = espelho(Arc::clone(&store), Arc::clone(&destino));

    let primeiro = espelho.cycle().await.expect("ciclo 1");
    assert!(primeiro.pending >= 1, "a falha no meio deixa pendência");
    // O cursor não passou da primeira linha não resolvida.
    let cursor = store.matrix().read_cursors(org).expect("cursor").messages;
    assert!(cursor < 6, "cursor não pode andar além da falha: {cursor}");

    espelho.cycle().await.expect("ciclo 2");
    espelho.cycle().await.expect("ciclo 3");

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
    let cenario = Cenario::new("venenosa");
    let store = cenario.store();
    let org = criar_org(&store, "a");
    let agent = criar_agente(&store, org);
    let conversation = criar_conversa(&store, org, agent, "+5511111");
    store.matrix().enable(org).expect("liga");

    let antes = txn_de(&msg_cliente(&store, org, conversation, "primeira"));
    let veneno = txn_de(&msg_cliente(&store, org, conversation, "VENENO aqui"));
    let depois = txn_de(&msg_cliente(&store, org, conversation, "terceira"));
    let ultima = txn_de(&msg_cliente(&store, org, conversation, "quarta"));

    let destino = Falso::new(Modo::Venenoso("VENENO"));
    let espelho = espelho(Arc::clone(&store), Arc::clone(&destino));

    // Até o limite de tentativas, a venenosa fica pendente e a fila espera.
    for _ in 0..(DEFINITIVE_ATTEMPTS - 1) {
        espelho.cycle().await.expect("ciclo");
    }
    assert!(
        !destino
            .txns_da_sala(&sala_de(conversation))
            .contains(&depois)
    );

    // Na tentativa limite, ela é isolada e a fila anda.
    espelho.cycle().await.expect("ciclo limite");
    espelho.cycle().await.expect("ciclo seguinte");

    let sent = destino.txns_da_sala(&sala_de(conversation));
    assert!(sent.contains(&antes));
    assert!(sent.contains(&depois));
    assert!(sent.contains(&ultima));
    assert!(!sent.contains(&veneno));

    let falhas = store.matrix().list_failures(org, 10).expect("falhas");
    assert_eq!(falhas.len(), 1, "a venenosa foi isolada uma vez");
    assert_eq!(falhas[0].motivo, "M_FORBIDDEN");
    assert_eq!(falhas[0].attempts, DEFINITIVE_ATTEMPTS);
}

#[tokio::test]
async fn synapse_fora_acumula_sem_avancar_e_depois_drena() {
    let cenario = Cenario::new("fora");
    let store = cenario.store();
    let org = criar_org(&store, "a");
    let agent = criar_agente(&store, org);
    let conversation = criar_conversa(&store, org, agent, "+5511111");
    store.matrix().enable(org).expect("liga");

    let mut todas = Vec::new();
    for i in 0..5 {
        todas.push(txn_de(&msg_cliente(
            &store,
            org,
            conversation,
            &format!("m{i}"),
        )));
    }

    let destino = Falso::new(Modo::Fora);
    let espelho = espelho(Arc::clone(&store), Arc::clone(&destino));
    for _ in 0..3 {
        let relatorio = espelho.cycle().await.expect("ciclo");
        assert_eq!(relatorio.sent, 0);
        assert!(relatorio.pending >= 1);
    }
    assert_eq!(
        store.matrix().read_cursors(org).expect("cursor").messages,
        0
    );
    assert!(
        espelho.delay_after(3) > espelho.delay_after(1),
        "o atraso cresce"
    );

    destino.definir_modo(Modo::Normal);
    espelho.cycle().await.expect("ciclo de volta");
    let criados: Vec<String> = destino.criados().into_iter().map(|c| c.txn).collect();
    assert_eq!(criados, todas, "tudo entregue, na ordem, uma vez");
}

#[tokio::test]
async fn org_desligada_nao_gera_chamada_alguma() {
    let cenario = Cenario::new("desligada");
    let store = cenario.store();
    let ligada = criar_org(&store, "ligada");
    let desligada = criar_org(&store, "desligada");
    let agente_l = criar_agente(&store, ligada);
    let agente_d = criar_agente(&store, desligada);
    let conv_l = criar_conversa(&store, ligada, agente_l, "+5511111");
    let conv_d = criar_conversa(&store, desligada, agente_d, "+5522222");
    store.matrix().enable(ligada).expect("liga");

    msg_cliente(&store, ligada, conv_l, "da ligada");
    msg_cliente(&store, desligada, conv_d, "da desligada");
    msg_cliente(&store, desligada, conv_d, "de novo na desligada");

    let destino = Falso::new(Modo::Normal);
    let relatorio = espelho(Arc::clone(&store), Arc::clone(&destino))
        .cycle()
        .await
        .expect("ciclo");

    assert_eq!(relatorio.sent, 1);
    assert_eq!(relatorio.from_disabled_org, 2);
    assert_eq!(destino.chamadas_sala.load(Ordering::SeqCst), 1);
    assert!(destino.criados().iter().all(|c| c.room == sala_de(conv_l)));
}

#[tokio::test]
async fn mensagem_anterior_ao_corte_nao_e_espelhada() {
    let cenario = Cenario::new("corte");
    let store = cenario.store();
    let org = criar_org(&store, "a");
    let agent = criar_agente(&store, org);
    let conversation = criar_conversa(&store, org, agent, "+5511111");

    msg_cliente(&store, org, conversation, "antes de ligar");
    store.matrix().enable(org).expect("liga");
    let depois = txn_de(&msg_cliente(&store, org, conversation, "depois de ligar"));

    let destino = Falso::new(Modo::Normal);
    let relatorio = espelho(Arc::clone(&store), Arc::clone(&destino))
        .cycle()
        .await
        .expect("ciclo");

    assert_eq!(relatorio.before_cutoff, 1);
    assert_eq!(destino.txns_da_sala(&sala_de(conversation)), vec![depois]);
}

#[tokio::test]
async fn resposta_humana_e_ferramenta_nao_viram_evento() {
    let cenario = Cenario::new("nao-espelha");
    let store = cenario.store();
    let org = criar_org(&store, "a");
    let agent = criar_agente(&store, org);
    let conversation = criar_conversa(&store, org, agent, "+5511111");
    store.matrix().enable(org).expect("liga");

    gravar(
        &store,
        org,
        conversation,
        Direction::Outbound,
        Role::Assistant,
        Origin::Human,
        "vindo do Element",
    );
    gravar(
        &store,
        org,
        conversation,
        Direction::Outbound,
        Role::System,
        Origin::Agente,
        "instrução",
    );
    msg_cliente(&store, org, conversation, "   ");
    let normal = txn_de(&msg_agente(&store, org, conversation, "resposta do agente"));

    let destino = Falso::new(Modo::Normal);
    let relatorio = espelho(Arc::clone(&store), Arc::clone(&destino))
        .cycle()
        .await
        .expect("ciclo");

    assert_eq!(relatorio.sent, 1);
    assert_eq!(destino.txns_da_sala(&sala_de(conversation)), vec![normal]);
}

#[tokio::test]
async fn rowid_reaproveitado_nao_perde_a_mensagem_nova() {
    let cenario = Cenario::new("rowid");
    let store = cenario.store();
    let org = criar_org(&store, "a");
    let agent = criar_agente(&store, org);
    let conversation = criar_conversa(&store, org, agent, "+5511111");
    store.matrix().enable(org).expect("liga");

    msg_cliente(&store, org, conversation, "um");
    msg_cliente(&store, org, conversation, "dois");
    let ultima = msg_cliente(&store, org, conversation, "tres");

    let destino = Falso::new(Modo::Normal);
    let espelho = espelho(Arc::clone(&store), Arc::clone(&destino));
    espelho.cycle().await.expect("ciclo 1");
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
    let fresh = msg_cliente(&store, org, conversation, "quatro, com rowid antigo");
    let rowid_novo: i64 = cenario
        .conexao()
        .query_row(
            "SELECT rowid FROM messages WHERE id = ?1",
            [fresh.as_str()],
            |linha| linha.get(0),
        )
        .expect("rowid novo");
    assert_eq!(rowid_novo, rowid_antigo, "o rowid foi reaproveitado");

    espelho.cycle().await.expect("ciclo 2");
    assert!(
        destino
            .txns_da_sala(&sala_de(conversation))
            .contains(&txn_de(&fresh)),
        "a mensagem com rowid reaproveitado sai"
    );
}

#[tokio::test]
async fn cursor_a_frente_do_maior_rowid_volta_ao_ponto_seguro() {
    let cenario = Cenario::new("recuo");
    let store = cenario.store();
    let org = criar_org(&store, "a");
    let agent = criar_agente(&store, org);
    let conversation = criar_conversa(&store, org, agent, "+5511111");
    store.matrix().enable(org).expect("liga");
    msg_cliente(&store, org, conversation, "um");
    msg_cliente(&store, org, conversation, "dois");

    // Cursor inventado, à frente de tudo que existe.
    store
        .matrix()
        .advance_messages_with_id(org, 99, "00000000-0000-7000-8000-000000000000")
        .expect("cursor");

    let destino = Falso::new(Modo::Normal);
    let espelho = espelho(Arc::clone(&store), Arc::clone(&destino));
    let relatorio = espelho.cycle().await.expect("ciclo");
    assert_eq!(relatorio.rewinds, 1);
    assert!(store.matrix().read_cursors(org).expect("cursor").messages <= 2);

    // Mensagem nova depois do recuo é lida normalmente.
    let fresh = txn_de(&msg_cliente(&store, org, conversation, "tres"));
    espelho.cycle().await.expect("ciclo 2");
    assert!(
        destino
            .txns_da_sala(&sala_de(conversation))
            .contains(&fresh)
    );
}

#[tokio::test]
async fn memoria_do_ciclo_fica_limitada_a_uma_pagina() {
    let cenario = Cenario::new("memoria");
    let store = cenario.store();
    let org = criar_org(&store, "a");
    let agent = criar_agente(&store, org);
    let conversation = criar_conversa(&store, org, agent, "+5511111");
    store.matrix().enable(org).expect("liga");

    let total = PAGE as usize * 3 + 17;
    for i in 0..total {
        msg_cliente(&store, org, conversation, &format!("m{i}"));
    }

    let destino = Falso::new(Modo::Normal);
    let relatorio = espelho(Arc::clone(&store), Arc::clone(&destino))
        .cycle()
        .await
        .expect("ciclo");
    assert!(
        relatorio.largest_page <= PAGE,
        "nunca mais que uma página em memória"
    );
    assert_eq!(
        relatorio.sent as usize,
        total.min(PAGE as usize * MAX_PAGES_PER_CYCLE as usize)
    );
}

#[tokio::test]
async fn atraso_cresce_com_falhas_e_para_no_teto() {
    let cenario = Cenario::new("atraso");
    let espelho = espelho(cenario.store(), Falso::new(Modo::Normal));
    assert_eq!(espelho.delay_after(0), Duration::from_millis(500));
    assert_eq!(espelho.delay_after(1), Duration::from_millis(1000));
    assert_eq!(espelho.delay_after(2), Duration::from_millis(2000));
    assert_eq!(espelho.delay_after(40), MAX_DELAY);
}

#[tokio::test]
async fn parada_encerra_o_laco() {
    let cenario = Cenario::new("parada");
    let store = cenario.store();
    let espelho = Arc::new(espelho(Arc::clone(&store), Falso::new(Modo::Normal)));
    let tarefa = tokio::spawn(Arc::clone(&espelho).run());
    tokio::time::sleep(Duration::from_millis(50)).await;
    espelho.request_stop();
    let resultado = tokio::time::timeout(Duration::from_secs(5), tarefa).await;
    assert!(resultado.is_ok(), "o laço sai depois do pedido de parada");
}

/// Cinquenta mil mensagens pendentes com o Synapse fora do ar: o ciclo lê uma página, para na
/// primeira falha e não carrega a fila inteira. Lento (grava 50 mil linhas): roda com `--ignored`.
#[tokio::test]
#[ignore = "grava 50 mil mensagens; rodar com --ignored"]
async fn memoria_com_50_mil_pendentes() {
    let cenario = Cenario::new("cinquenta-mil");
    let store = cenario.store();
    let org = criar_org(&store, "a");
    let agent = criar_agente(&store, org);
    let conversation = criar_conversa(&store, org, agent, "+5511111");
    store.matrix().enable(org).expect("liga");
    let inicio = std::time::Instant::now();
    for i in 0..50_000 {
        msg_cliente(&store, org, conversation, &format!("m{i}"));
    }
    eprintln!("gravação de 50 mil: {:?}", inicio.elapsed());

    let destino = Falso::new(Modo::Fora);
    let inicio = std::time::Instant::now();
    let relatorio = espelho(Arc::clone(&store), Arc::clone(&destino))
        .cycle()
        .await
        .expect("ciclo");
    eprintln!(
        "ciclo com 50 mil pendentes: {:?}, {relatorio:?}",
        inicio.elapsed()
    );
    assert_eq!(relatorio.sent, 0);
    assert!(relatorio.largest_page <= PAGE);
    assert_eq!(relatorio.pages, 1, "para na primeira página com falha");
    assert_eq!(
        store.matrix().read_cursors(org).expect("cursor").messages,
        0
    );
}

// ───────────────────────────── MX-42: mensagem do aparelho ─────────────────────────────

/// Peer do cliente nos testes do aparelho (o mesmo em toda a conversa).
const PEER_APARELHO: &str = "5511999990000";

/// Org com o módulo ligado, um agente e a integração do WhatsApp (a instância da Evolution).
fn org_com_aparelho(store: &Store) -> (OrgId, AgentId, IntegrationId) {
    let org = criar_org(store, "ap");
    let agent = criar_agente(store, org);
    let integracao: IntegrationId = store
        .create_integration(org, agent, "whatsapp".to_owned(), "active".to_owned())
        .expect("cria a integração")
        .id
        .parse()
        .expect("id da integração");
    store.matrix().enable(org).expect("liga");
    (org, agent, integracao)
}

/// Mensagem digitada no celular da loja, pelo caminho da MX-41 (`fromMe` gravado como do aparelho).
fn receber_aparelho(
    store: &Store,
    org: OrgId,
    agent: AgentId,
    integracao: IntegrationId,
    id_externo: &str,
    text: &str,
) -> (ConversationId, MessageId) {
    let recebido = store
        .inbox()
        .receive_from_device(
            org,
            NewDeviceMessage {
                integration_id: integracao,
                agent_id: agent,
                canal: "whatsapp".to_owned(),
                peer: PEER_APARELHO.to_owned(),
                external_id: id_externo.to_owned(),
                texto: text.to_owned(),
            },
        )
        .expect("recebe a mensagem do aparelho");
    (recebido.conversation, recebido.mensagem.id)
}

/// Id do usuário do aparelho desta integração, como o espelho monta.
fn fantasma_do_aparelho(integracao: IntegrationId) -> String {
    GhostIds::new("ph", "matrix-dev.local")
        .expect("ids")
        .device(&integracao.to_string())
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
    let cenario = Cenario::new("aparelho-sala");
    let store = cenario.store();
    let (org, agent, integracao) = org_com_aparelho(&store);
    let (conversation, message) = receber_aparelho(
        &store,
        org,
        agent,
        integracao,
        "AP-1",
        "Já enviei, obrigado",
    );
    // O nome que o WhatsApp deu ao cliente: a mensagem do aparelho não pode trocá-lo.
    store
        .conversations()
        .set_customer_name(org, conversation, "Maria")
        .expect("grava o nome do cliente");
    // Uma resposta do cliente na mesma conversa, para provar que cada um fala com o próprio usuário.
    msg_cliente(&store, org, conversation, "oi, tudo bem?");

    let destino = Falso::new(Modo::Normal);
    let outbox_antes = contar_outbox(&cenario);
    let relatorio = espelho(Arc::clone(&store), Arc::clone(&destino))
        .cycle()
        .await
        .expect("ciclo");

    assert_eq!(relatorio.sent, 2, "a do cliente e a do aparelho");
    let do_aparelho = destino.criados_do(&fantasma_do_aparelho(integracao));
    assert_eq!(do_aparelho.len(), 1, "o usuário do aparelho fala uma vez");
    assert_eq!(do_aparelho[0].txn, txn_de(&message.to_string()));
    assert_eq!(do_aparelho[0].body, "Já enviei, obrigado");
    assert_eq!(do_aparelho[0].room, sala_de(conversation));
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
    let name = store
        .conversations()
        .get(org, conversation)
        .expect("lê a conversa")
        .customer_name;
    assert_eq!(name.as_deref(), Some("Maria"), "o nome do cliente não muda");
}

#[tokio::test]
async fn aparelho_entra_uma_vez_por_passagem_e_repeticao_nao_duplica() {
    let cenario = Cenario::new("aparelho-repeticao");
    let store = cenario.store();
    let (org, agent, integracao) = org_com_aparelho(&store);
    let (conversation, primeira) =
        receber_aparelho(&store, org, agent, integracao, "AP-1", "primeira");
    let (_, segunda) = receber_aparelho(&store, org, agent, integracao, "AP-2", "segunda");
    assert_eq!(
        store
            .conversations()
            .get(org, conversation)
            .expect("conversa")
            .id,
        conversation,
        "as duas mensagens caem na mesma conversa"
    );

    let destino = Falso::new(Modo::Normal);
    let espelho = espelho(Arc::clone(&store), Arc::clone(&destino));
    let ciclo = espelho.cycle().await.expect("ciclo 1");
    assert_eq!(ciclo.sent, 2);
    assert_eq!(
        destino.chamadas_aparelho.load(Ordering::SeqCst),
        1,
        "um grupo da sala, um pedido de entrada"
    );

    // Passagem repetida do laço (como depois de uma queda): lê tudo de novo a partir do início.
    store
        .matrix()
        .rewind_cursor_messages(org, 0, "")
        .expect("recua o cursor");
    espelho.cycle().await.expect("ciclo 2");

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
    let cenario = Cenario::new("aparelho-adia");
    let store = cenario.store();
    let (org, agent, integracao) = org_com_aparelho(&store);
    let (_, message) = receber_aparelho(&store, org, agent, integracao, "AP-1", "oi do celular");

    let destino = Falso::new(Modo::Normal);
    destino.definir_falha_aparelho(true);
    let espelho = espelho(Arc::clone(&store), Arc::clone(&destino));
    let primeiro = espelho.cycle().await.expect("ciclo 1");
    assert_eq!(primeiro.sent, 0, "sem entrar na sala, nada é enviado");
    assert!(primeiro.pending >= 1, "a mensagem fica pendente");
    assert!(destino.criados().is_empty());

    // A mensagem continua gravada no Harness; quando a sala responder, sai uma vez.
    destino.definir_falha_aparelho(false);
    let segundo = espelho.cycle().await.expect("ciclo 2");
    assert_eq!(segundo.sent, 1);
    let do_aparelho = destino.criados_do(&fantasma_do_aparelho(integracao));
    assert_eq!(do_aparelho.len(), 1);
    assert_eq!(do_aparelho[0].txn, txn_de(&message.to_string()));
}
