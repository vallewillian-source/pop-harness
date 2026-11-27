//! Testes da MX-18: a resposta do atendente no Element chega ao cliente.
//!
//! Tudo aqui é de ponta a ponta, com os três servidores de verdade e banco em pasta temporária:
//!
//! - o Harness no ar (`servir_em`), do jeito que o Synapse o vê, com as rotas do Application
//!   Service e o consumidor da fila de saída;
//! - um **Synapse falso** em `127.0.0.1`, que responde o perfil do atendente e anota cada evento de
//!   mensagem escrito na sala (é por ele que se prova que o aviso saiu, e que não sai resposta
//!   quando a conversa está encerrada);
//! - uma **Evolution falsa** em `127.0.0.1`, que anota cada envio (é por ela que se prova que o
//!   cliente recebeu **uma** mensagem, com o prefixo do atendente).
//!
//! O que se prova:
//! - A2: uma mensagem do atendente vira um envio com `*[Nome]*:`, a conversa fica `transferred` e o
//!   reenvio da mesma transação não envia de novo;
//! - A3 (do PRD): o Synapse repetindo a mesma transação gera um envio só;
//! - A9: evento de sala da org A não altera a conversa da org B (e vice-versa);
//! - conversa encerrada → nada ao cliente e aviso na sala;
//! - sala que nenhuma org conhece → `200`, nada gravado;
//! - falha de banco na entrega → `503` (nunca `4xx`), e o reenvio processa;
//! - mídia do atendente que o Synapse não entrega mais → nada ao cliente e aviso na sala
//!   (o caminho completo do arquivo, com Evolution falsa e Synapse falso servindo o download, está
//!   em `tests/matrix_midia_atendente.rs`);
//! - o nome do atendente é lido uma vez e fica no cache com validade.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use poph_evolution::webhook::SECRET_HEADER;
use poph_evolution::{ATTACHMENT_CAP, ConfigEvolution, EvolutionClient};
use poph_matrix::{ClientConfig, GhostIds, MatrixClient};
use poph_server::evolution::EvolutionChannel;
use poph_server::{AsReceiver, AttendantResponse, HarnessResponsePort, ResponsePort, ServerState};
use poph_store::error::StoreError;
use poph_store::{
    HumanResponse, NewAgent, NewConversation, NewHumanResponse, NewOrg, Origin, OutboxState, Page,
    Store,
};
use poph_types::{ConversationId, HarnessConfig, IntegrationId, OrgId, Secret};
use reqwest::{Client, StatusCode as StatusHttp};
use serde_json::{Value, json};
use tokio::sync::oneshot;

// ───────────────────────────────── constantes ─────────────────────────────────

const PREFIXO: &str = "ph";
const SERVER_NAME: &str = "matrix-dev.local";

/// Valor-canário do token do Synapse: não pode aparecer em resposta, log nem erro.
const HS_TOKEN: &str = "CANARIO-HS-mx18-5f3c-TAO-SECRETO";

/// Valor-canário do token com que o Harness age no Synapse.
const AS_TOKEN: &str = "CANARIO-AS-mx18-91ab-TAO-SECRETO";

const INSTANCIA: &str = "loja";
const CHAVE_EVO: &str = "chave-evo-mx18-teste";
const SEGREDO_WEBHOOK: &str = "segredo-webhook-mx18";

/// Número do cliente. Não é de ninguém.
const TELEFONE: &str = "5511999990000";
const JID: &str = "5511999990000@s.whatsapp.net";

/// Atendente humano: `localpart` que **não** pode ser confundida com fantasma.
const ATENDENTE: &str = "@ana:matrix-dev.local";
const NOME_ATENDENTE: &str = "Ana Atendente";

/// Sala da conversa da org do cenário.
const SALA: &str = "!sala-a:matrix-dev.local";
/// Sala da conversa da org vizinha (teste de isolamento).
const SALA_B: &str = "!sala-b:matrix-dev.local";

// ───────────────────────────────── Synapse falso ─────────────────────────────────

/// Um evento de mensagem que o Harness escreveu na sala (aviso, por exemplo).
#[derive(Clone, Debug)]
struct Escrita {
    room: String,
    tipo: String,
    txn: String,
    disfarcado: Option<String>,
    autorizacao: Option<String>,
    body: Value,
}

#[derive(Clone, Default)]
struct SynapseFalso {
    perfis: Arc<Mutex<HashMap<String, String>>>,
    escritas: Arc<Mutex<Vec<Escrita>>>,
    perfis_lidos: Arc<AtomicUsize>,
}

async fn perfil_falso(
    State(falso): State<SynapseFalso>,
    Path(usuario): Path<String>,
    cabecalhos: HeaderMap,
) -> (StatusCode, Json<Value>) {
    falso.perfis_lidos.fetch_add(1, Ordering::SeqCst);
    let _ = cabecalhos;
    let perfis = falso.perfis.lock().unwrap();
    match perfis.get(&usuario) {
        Some(name) => (
            StatusCode::OK,
            Json(json!({ "displayname": name, "avatar_url": null })),
        ),
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({ "errcode": "M_NOT_FOUND" })),
        ),
    }
}

async fn enviar_falso(
    State(falso): State<SynapseFalso>,
    Path((room, tipo, txn)): Path<(String, String, String)>,
    Query(consulta): Query<HashMap<String, String>>,
    cabecalhos: HeaderMap,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    let autorizacao = cabecalhos
        .get("authorization")
        .and_then(|valor| valor.to_str().ok())
        .map(str::to_owned);
    let mut escritas = falso.escritas.lock().unwrap();
    let posicao = escritas.len() + 1;
    escritas.push(Escrita {
        room,
        tipo,
        txn,
        disfarcado: consulta.get("user_id").cloned(),
        autorizacao,
        body,
    });
    (
        StatusCode::OK,
        Json(json!({ "event_id": format!("$escrito-{posicao}:matrix-dev.local") })),
    )
}

/// Sobe o Synapse falso numa porta efêmera e devolve o endereço.
async fn subir_synapse_falso() -> (String, SynapseFalso) {
    let falso = SynapseFalso::default();
    {
        let mut perfis = falso.perfis.lock().unwrap();
        perfis.insert(ATENDENTE.to_owned(), NOME_ATENDENTE.to_owned());
    }
    let roteador = Router::new()
        .route("/pronto", get(|| async { StatusCode::OK }))
        .route("/_matrix/client/v3/profile/{user}", get(perfil_falso))
        .route(
            "/_matrix/client/v3/rooms/{room}/send/{kind}/{txn}",
            put(enviar_falso),
        )
        .with_state(falso.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("porta efêmera do Synapse falso");
    let endereco = listener.local_addr().expect("endereço do Synapse falso");
    tokio::spawn(async move {
        let _ = axum::serve(listener, roteador).await;
    });
    (format!("http://{endereco}"), falso)
}

/// Espera as rotas do Synapse falso responderem (o servidor sobe na tarefa acima).
async fn esperar_synapse(url: &str) {
    let client = Client::new();
    let inicio = Instant::now();
    loop {
        let resposta = client.get(format!("{url}/pronto")).send().await;
        if resposta.is_ok() {
            return;
        }
        assert!(
            inicio.elapsed() < Duration::from_secs(10),
            "Synapse falso não subiu"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

// ───────────────────────────────── Evolution falsa ─────────────────────────────────

#[derive(Clone, Default)]
struct EvolucaoFalsa {
    envios: Arc<Mutex<Vec<Value>>>,
}

async fn enviar_pelo_whatsapp(
    State(falsa): State<EvolucaoFalsa>,
    Path(_instancia): Path<String>,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    let mut envios = falsa.envios.lock().unwrap();
    envios.push(body);
    let posicao = envios.len();
    (
        StatusCode::OK,
        Json(json!({ "key": { "id": format!("MSG-EVO-{posicao}") } })),
    )
}

async fn subir_evolution_falsa() -> (String, EvolucaoFalsa) {
    let falsa = EvolucaoFalsa::default();
    let roteador = Router::new()
        .route("/message/sendText/{instance}", post(enviar_pelo_whatsapp))
        .with_state(falsa.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("porta efêmera da Evolution falsa");
    let endereco = listener.local_addr().expect("endereço da Evolution falsa");
    tokio::spawn(async move {
        let _ = axum::serve(listener, roteador).await;
    });
    (format!("http://{endereco}"), falsa)
}

// ───────────────────────────────── porta que sabe falhar ─────────────────────────────────

/// A porta de verdade, com um interruptor para fingir banco fora do ar.
struct PortaComFalha {
    real: HarnessResponsePort,
    falhar: AtomicBool,
}

impl PortaComFalha {
    fn fresh(store: Arc<Store>) -> Self {
        Self {
            real: HarnessResponsePort::fresh(store),
            falhar: AtomicBool::new(true),
        }
    }

    fn consertar(&self) {
        self.falhar.store(false, Ordering::SeqCst);
    }
}

impl ResponsePort for PortaComFalha {
    fn room_conversation(
        &self,
        room: &str,
    ) -> Result<Option<poph_server::matrix_response::RoomConversation>, StoreError> {
        if self.falhar.load(Ordering::SeqCst) {
            return Err(StoreError::Internal("banco fora do ar (teste)".to_owned()));
        }
        self.real.room_conversation(room)
    }

    fn conversation_agent(
        &self,
        alvo: poph_server::matrix_response::RoomConversation,
    ) -> Result<Option<poph_types::AgentId>, StoreError> {
        if self.falhar.load(Ordering::SeqCst) {
            return Err(StoreError::Internal("banco fora do ar (teste)".to_owned()));
        }
        self.real.conversation_agent(alvo)
    }

    fn record_response(
        &self,
        alvo: poph_server::matrix_response::RoomConversation,
        fresh: NewHumanResponse<'_>,
    ) -> Result<HumanResponse, StoreError> {
        self.real.record_response(alvo, fresh)
    }

    fn record_response_with_file(
        &self,
        alvo: poph_server::matrix_response::RoomConversation,
        fresh: poph_store::HumanResponseWithFile<'_>,
    ) -> Result<HumanResponse, StoreError> {
        self.real.record_response_with_file(alvo, fresh)
    }
}

/// A porta de verdade, mas sem saber o agente da conversa: o caso da conversa antiga, da org
/// migrada ou do registro que sumiu. Não há fantasma para assinar o arquivo do atendente.
struct PortaSemAgente {
    real: HarnessResponsePort,
}

impl ResponsePort for PortaSemAgente {
    fn room_conversation(
        &self,
        room: &str,
    ) -> Result<Option<poph_server::matrix_response::RoomConversation>, StoreError> {
        self.real.room_conversation(room)
    }

    fn conversation_agent(
        &self,
        _alvo: poph_server::matrix_response::RoomConversation,
    ) -> Result<Option<poph_types::AgentId>, StoreError> {
        Ok(None)
    }

    fn record_response(
        &self,
        alvo: poph_server::matrix_response::RoomConversation,
        fresh: NewHumanResponse<'_>,
    ) -> Result<HumanResponse, StoreError> {
        self.real.record_response(alvo, fresh)
    }

    fn record_response_with_file(
        &self,
        alvo: poph_server::matrix_response::RoomConversation,
        fresh: poph_store::HumanResponseWithFile<'_>,
    ) -> Result<HumanResponse, StoreError> {
        self.real.record_response_with_file(alvo, fresh)
    }
}

// ───────────────────────────────── cenário ─────────────────────────────────

struct ConversaDoCenario {
    org: OrgId,
    conversation: ConversationId,
}

struct Cenario {
    pasta: PathBuf,
    store: Arc<Store>,
    /// Org do cenário (a dona da sala [`SALA`]).
    org: OrgId,
    conversation: ConversationId,
    /// Org vizinha, com conversa própria e sem sala: o isolamento é provado contra ela.
    org_b: OrgId,
    conversa_b: ConversationId,
    synapse: SynapseFalso,
    evo: EvolucaoFalsa,
    /// Endereço do Synapse falso (o cliente do módulo aponta para ele).
    url_synapse: String,
    /// Endereço da Evolution falsa (o canal de saída aponta para ele).
    url_evo: String,
}

impl Cenario {
    async fn new(rotulo: &str) -> Self {
        let carimbo = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|duracao| duracao.as_nanos())
            .unwrap_or(0);
        let pasta = std::env::temp_dir().join(format!(
            "poph-matrix-resposta-{rotulo}-{}-{carimbo}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&pasta);
        fs::create_dir_all(&pasta).expect("pasta do teste");
        let store = Arc::new(
            Store::open_at(&pasta.join("harness.db"), &pasta.join("rag.db")).expect("abre o banco"),
        );

        let a = Self::org_com_conversa(&store, "loja-a", "Sala A", "bia-a");
        let b = Self::org_com_conversa(&store, "loja-b", "Sala B", "bia-b");

        // Sala da org A pronta no banco (a provisão de sala é a MX-11).
        store
            .matrix()
            .enable(a.org)
            .expect("liga o módulo na org A");
        store
            .matrix()
            .ensure_room(a.org, a.conversation, SALA, None)
            .expect("grava a sala da org A");
        store
            .matrix()
            .mark_room_ready(a.org, a.conversation)
            .expect("sala pronta");
        // A org B também tem o módulo ligado e uma sala, para o isolamento valer nos dois sentidos.
        store
            .matrix()
            .enable(b.org)
            .expect("liga o módulo na org B");
        store
            .matrix()
            .ensure_room(b.org, b.conversation, SALA_B, None)
            .expect("grava a sala da org B");
        store
            .matrix()
            .mark_room_ready(b.org, b.conversation)
            .expect("sala B pronta");

        let (url_synapse, synapse) = subir_synapse_falso().await;
        esperar_synapse(&url_synapse).await;
        let (url_evo, evo) = subir_evolution_falsa().await;
        Self {
            pasta,
            store,
            org: a.org,
            conversation: a.conversation,
            org_b: b.org,
            conversa_b: b.conversation,
            synapse,
            evo,
            url_synapse,
            url_evo,
        }
    }

    /// Cria org + agente + integração de WhatsApp + conversa, com sala em [`SALA`].
    fn org_com_conversa(
        store: &Store,
        slug: &str,
        agent_name: &str,
        slug_agente: &str,
    ) -> ConversaDoCenario {
        let org = store
            .orgs()
            .create(NewOrg {
                name: format!("Org {slug}"),
                slug: slug.to_owned(),
                config_json: String::new(),
            })
            .expect("cria a org")
            .id;
        let agent = store
            .agents()
            .create(
                org,
                NewAgent {
                    name: agent_name.to_owned(),
                    slug: slug_agente.to_owned(),
                    ..NewAgent::default()
                },
            )
            .expect("cria o agente")
            .id;
        let integracao: IntegrationId = store
            .create_integration(org, agent, "whatsapp".to_owned(), "active".to_owned())
            .expect("cria a integração")
            .id
            .parse()
            .expect("id da integração");
        let conversation = store
            .conversations()
            .create(
                org,
                NewConversation {
                    agent_id: agent,
                    integration_id: Some(integracao),
                    canal: "whatsapp".to_owned(),
                    peer_externo: Some(TELEFONE.to_owned()),
                    target_department: None,
                },
            )
            .expect("cria a conversa")
            .id;
        ConversaDoCenario { org, conversation }
    }

    /// O canal da Evolution que o servidor usa para entregar a fila de saída de uma conversa.
    fn channel(&self, org: OrgId, conversation: ConversationId, url_evo: &str) -> EvolutionChannel {
        let client = EvolutionClient::new(
            ConfigEvolution::new(
                url_evo.to_owned(),
                INSTANCIA,
                Secret::new(CHAVE_EVO.to_owned()),
            )
            .expect("config da Evolution falsa"),
        )
        .expect("cliente da Evolution falsa");
        let integracao = self
            .store
            .conversations()
            .get(org, conversation)
            .expect("lê a conversa")
            .integration_id
            .expect("a conversa tem integração");
        EvolutionChannel {
            org,
            instance: INSTANCIA.to_owned(),
            integration: integracao,
            secret: Some(Secret::new(SEGREDO_WEBHOOK.to_owned())),
            client: Some(client),
            attachment_limit: ATTACHMENT_CAP,
        }
    }

    /// Sobe o Harness com o receptor do Application Service e o consumidor da fila de saída.
    async fn subir(&self, porta: Arc<dyn ResponsePort>) -> Servidor {
        // Um canal por org, como na operação: a fila de cada org sai pelo canal dela.
        let canais = vec![
            self.channel(self.org, self.conversation, &self.url_evo),
            self.channel(self.org_b, self.conversa_b, &self.url_evo),
        ];
        let cliente_matrix = MatrixClient::new(
            ClientConfig::new(
                self.url_synapse.clone(),
                SERVER_NAME.to_owned(),
                Secret::new(AS_TOKEN.to_owned()),
            )
            .expect("config do cliente do Matrix"),
        )
        .expect("cliente do Matrix");
        let ids = GhostIds::new(PREFIXO, SERVER_NAME).expect("ids dos fantasmas");
        // Pasta de blobs como na operação: o arquivo do atendente (MX-44) é gravado ali.
        let entrega = Arc::new(
            AttendantResponse::new(porta, Some(cliente_matrix), ids)
                .with_blob_folder(self.pasta.join("blobs")),
        );

        let state = ServerState::new(Arc::clone(&self.store), HarnessConfig::default())
            .expect("estado do servidor")
            .with_evolution_channels(canais)
            .with_matrix_as(Arc::new(AsReceiver::new(
                PREFIXO,
                SERVER_NAME,
                Secret::new(HS_TOKEN.to_owned()),
                entrega.clone(),
            )));

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("porta efêmera");
        let endereco = listener.local_addr().expect("endereço local");
        let (desligar, sinal) = oneshot::channel::<()>();
        tokio::spawn(async move {
            let _ = poph_server::serve_at(listener, state, async {
                let _ = sinal.await;
            })
            .await;
        });
        let servidor = Servidor {
            base: format!("http://{endereco}"),
            client: Client::builder()
                .timeout(Duration::from_secs(20))
                .build()
                .expect("cliente HTTP"),
            entrega,
            desligar: Some(desligar),
        };
        servidor.esperar_subir().await;
        servidor
    }

    fn conversa_da_org(&self, org: OrgId) -> ConversationId {
        if org == self.org {
            self.conversation
        } else {
            self.conversa_b
        }
    }

    fn status(&self, org: OrgId) -> poph_store::ConversationStatus {
        self.store
            .conversations()
            .get(org, self.conversa_da_org(org))
            .expect("lê a conversa")
            .status
    }

    fn messages(&self, org: OrgId) -> Vec<poph_store::Mensagem> {
        self.store
            .messages()
            .list(org, self.conversa_da_org(org), Page::with_limit(50))
            .expect("lista as mensagens")
    }

    fn enviados(&self, org: OrgId) -> u64 {
        self.store
            .outbox()
            .count_by_state(org, OutboxState::Sent)
            .expect("conta a fila de saída")
    }

    fn pendentes(&self, org: OrgId) -> u64 {
        self.store
            .outbox()
            .count_by_state(org, OutboxState::Pending)
            .expect("conta a fila de saída")
    }

    fn envios_no_whatsapp(&self) -> Vec<Value> {
        self.evo.envios.lock().unwrap().clone()
    }

    fn escritas_no_synapse(&self) -> Vec<Escrita> {
        self.synapse.escritas.lock().unwrap().clone()
    }

    /// O fantasma do agente dono da conversa: quem o Synapse deixa personificar (namespace do
    /// Application Service) e quem está na sala. É ele que baixa o arquivo do atendente e assina os
    /// avisos.
    fn agent_ghost(&self) -> String {
        let agent = self
            .store
            .conversations()
            .get(self.org, self.conversation)
            .expect("lê a conversa")
            .agent_id;
        GhostIds::new(PREFIXO, SERVER_NAME)
            .expect("ids dos fantasmas")
            .agent(&agent.to_string())
            .expect("fantasma do agente")
            .user_id()
            .to_owned()
    }
}

impl Drop for Cenario {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.pasta);
    }
}

/// O Harness no ar: entrega transações como o Synapse entregaria.
struct Servidor {
    base: String,
    client: Client,
    entrega: Arc<AttendantResponse>,
    desligar: Option<oneshot::Sender<()>>,
}

impl Servidor {
    async fn esperar_subir(&self) {
        let inicio = Instant::now();
        loop {
            match self
                .client
                .get(format!("{}/healthz", self.base))
                .send()
                .await
            {
                Ok(resposta) if resposta.status().is_success() => return,
                _ if inicio.elapsed() < Duration::from_secs(10) => {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                outro => panic!("servidor não subiu: {outro:?}"),
            }
        }
    }

    /// `PUT /_matrix/app/v1/transactions/{txnId}` com o `hs_token`, como o Synapse faz.
    async fn transacao(&self, txn: &str, body: &Value) -> (StatusHttp, Value) {
        let resposta = self
            .client
            .put(format!("{}/_matrix/app/v1/transactions/{txn}", self.base))
            .header("authorization", format!("Bearer {HS_TOKEN}"))
            .json(body)
            .send()
            .await
            .expect("entrega a transação");
        let status = resposta.status();
        let valor = resposta.json::<Value>().await.unwrap_or(Value::Null);
        (status, valor)
    }

    /// Entrega uma mensagem de texto do atendente numa transação própria.
    async fn texto_do_atendente(
        &self,
        txn: &str,
        room: &str,
        event: &str,
        text: &str,
    ) -> (StatusHttp, Value) {
        self.transacao(
            txn,
            &json!({
                "events": [{
                    "type": "m.room.message",
                    "room_id": room,
                    "sender": ATENDENTE,
                    "event_id": event,
                    "origin_server_ts": 1_795_300_000_000i64,
                    "content": { "msgtype": "m.text", "body": text }
                }]
            }),
        )
        .await
    }

    /// Entrega um webhook da Evolution como o cliente faria (mensagem recebida).
    async fn webhook(&self, org: OrgId, body: &Value) -> (StatusHttp, Value) {
        let resposta = self
            .client
            .post(format!("{}/webhook/evolution/{org}/{INSTANCIA}", self.base))
            .header(SECRET_HEADER, SEGREDO_WEBHOOK)
            .json(body)
            .send()
            .await
            .expect("entrega o webhook");
        let status = resposta.status();
        let valor = resposta.json::<Value>().await.unwrap_or(Value::Null);
        (status, valor)
    }

    fn contadores(&self) -> &poph_server::matrix_response::ResponseCounters {
        self.entrega.counters()
    }
}

impl Drop for Servidor {
    fn drop(&mut self) {
        if let Some(desligar) = self.desligar.take() {
            let _ = desligar.send(());
        }
    }
}

// ───────────────────────────────── utilidades ─────────────────────────────────

/// Espera uma condição virar verdadeira (até 15 s), com a última checagem no erro.
async fn esperar(rotulo: &str, mut condicao: impl FnMut() -> bool) {
    let inicio = Instant::now();
    loop {
        if condicao() {
            return;
        }
        assert!(
            inicio.elapsed() < Duration::from_secs(15),
            "não aconteceu a tempo: {rotulo}"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

/// Texto que o cliente recebeu pela Evolution falsa (o corpo de `text` de cada envio).
fn textos_enviados(envios: &[Value]) -> Vec<String> {
    envios
        .iter()
        .map(|send| {
            send.get("text")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        })
        .collect()
}

// ───────────────────────────────── testes ─────────────────────────────────

#[tokio::test]
async fn resposta_do_atendente_vira_um_envio_com_prefixo_e_transfere_a_conversa() {
    let cenario = Cenario::new("a2").await;
    let servidor = cenario
        .subir(Arc::new(HarnessResponsePort::fresh(Arc::clone(
            &cenario.store,
        ))))
        .await;

    let (status, body) = servidor
        .texto_do_atendente(
            "txn-1",
            SALA,
            "$evento-1:matrix-dev.local",
            "olá, tudo bem?",
        )
        .await;
    assert_eq!(status, StatusHttp::OK);
    assert_eq!(body, json!({}));

    // O cliente recebe **uma** mensagem, com o nome do atendente no prefixo.
    esperar("o envio sair pela Evolution", || {
        !cenario.envios_no_whatsapp().is_empty()
    })
    .await;
    let envios = cenario.envios_no_whatsapp();
    assert_eq!(envios.len(), 1, "uma mensagem do atendente = um envio");
    assert_eq!(
        textos_enviados(&envios),
        vec!["*[Ana Atendente]*:\nolá, tudo bem?".to_owned()]
    );

    // A conversa fica transferida: o agente não responde mais.
    assert_eq!(
        cenario.status(cenario.org),
        poph_store::ConversationStatus::Transferred
    );

    // A mensagem fica no histórico como humana, com o id do evento e o nome do autor.
    let mensagens = cenario.messages(cenario.org);
    assert_eq!(mensagens.len(), 1);
    let message = &mensagens[0];
    assert_eq!(message.origin, Origin::Human);
    assert_eq!(message.texto, "olá, tudo bem?");
    assert_eq!(
        message.external_id.as_deref(),
        Some("$evento-1:matrix-dev.local")
    );
    assert_eq!(message.author_name.as_deref(), Some("Ana Atendente"));

    // Nada sobrou na fila: o único item foi entregue.
    assert_eq!(cenario.enviados(cenario.org), 1);
    assert_eq!(cenario.pendentes(cenario.org), 0);

    // A mensagem seguinte do cliente é gravada e não reabre a conversa: o agente segue pausado.
    let (status, _) = servidor
        .webhook(
            cenario.org,
            &json!({
                "event": "messages.upsert",
                "instance": INSTANCIA,
                "apikey": CHAVE_EVO,
                "destination": "http://harness.teste/webhook/evolution/x/loja",
                "data": {
                    "key": { "id": "MSG-CLIENTE-1", "remoteJid": JID, "fromMe": false },
                    "pushName": "Cliente Teste",
                    "messageType": "conversation",
                    "message": { "conversation": "e agora?" },
                    "messageTimestamp": 1_795_300_100i64
                }
            }),
        )
        .await;
    assert_eq!(status, StatusHttp::OK);
    esperar("a mensagem do cliente ser gravada", || {
        cenario.messages(cenario.org).len() == 2
    })
    .await;
    // Nada novo vai ao cliente e a conversa continua transferida (o agente não responde).
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        cenario.envios_no_whatsapp().len(),
        1,
        "o agente não respondeu"
    );
    assert_eq!(
        cenario.status(cenario.org),
        poph_store::ConversationStatus::Transferred
    );

    // A org vizinha não foi tocada (A9, conferido também no teste próprio).
    assert_eq!(
        cenario.status(cenario.org_b),
        poph_store::ConversationStatus::Open
    );
    assert_eq!(cenario.enviados(cenario.org_b), 0);
}

#[tokio::test]
async fn reenvio_da_mesma_transacao_e_do_mesmo_evento_nao_gera_segundo_envio() {
    let cenario = Cenario::new("a3").await;
    let porta = Arc::new(HarnessResponsePort::fresh(Arc::clone(&cenario.store)));
    let servidor = cenario.subir(porta).await;

    let (primeiro, _) = servidor
        .texto_do_atendente(
            "txn-repetida",
            SALA,
            "$evento-repetido:matrix-dev.local",
            "oi",
        )
        .await;
    // Mesma transação outra vez: a MX-16 lembra o `txnId` e nem chama a entrega.
    let (segundo, _) = servidor
        .texto_do_atendente(
            "txn-repetida",
            SALA,
            "$evento-repetido:matrix-dev.local",
            "oi",
        )
        .await;
    // Transação nova com o mesmo evento (o Synapse pode remontar o lote): a idempotência é do
    // `event_id`, na operação atômica da MX-04.
    let (terceiro, _) = servidor
        .texto_do_atendente("txn-outra", SALA, "$evento-repetido:matrix-dev.local", "oi")
        .await;

    assert_eq!(primeiro, StatusHttp::OK);
    assert_eq!(segundo, StatusHttp::OK);
    assert_eq!(terceiro, StatusHttp::OK);

    esperar("o envio sair pela Evolution", || {
        cenario.enviados(cenario.org) == 1
    })
    .await;
    // Tempo para um segundo envio aparecer, se ele existisse.
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        cenario.envios_no_whatsapp().len(),
        1,
        "reenvio não duplica o envio"
    );
    assert_eq!(
        cenario.messages(cenario.org).len(),
        1,
        "uma mensagem no histórico"
    );
    assert_eq!(
        servidor.contadores().duplicates.load(Ordering::SeqCst),
        1,
        "o reenvio com transação nova foi reconhecido como duplicado"
    );
}

#[tokio::test]
async fn evento_de_uma_org_nao_toca_a_conversa_da_outra() {
    let cenario = Cenario::new("a9").await;
    let porta = Arc::new(HarnessResponsePort::fresh(Arc::clone(&cenario.store)));
    let servidor = cenario.subir(porta).await;

    // Atendente responde na sala da org A.
    let (status, _) = servidor
        .texto_do_atendente(
            "txn-a",
            SALA,
            "$evento-a:matrix-dev.local",
            "resposta da org A",
        )
        .await;
    assert_eq!(status, StatusHttp::OK);
    esperar("o envio da org A sair", || {
        cenario.enviados(cenario.org) == 1
    })
    .await;

    assert_eq!(
        cenario.status(cenario.org),
        poph_store::ConversationStatus::Transferred
    );
    assert_eq!(
        cenario.status(cenario.org_b),
        poph_store::ConversationStatus::Open,
        "a conversa da org B continua aberta"
    );
    assert!(
        cenario.messages(cenario.org_b).is_empty(),
        "nada gravado na org B"
    );
    assert_eq!(
        cenario.enviados(cenario.org_b),
        0,
        "nada enviado pela org B"
    );

    // E o contrário: resposta na sala da org B não mexe na org A.
    let (status, _) = servidor
        .texto_do_atendente(
            "txn-b",
            SALA_B,
            "$evento-b:matrix-dev.local",
            "resposta da org B",
        )
        .await;
    assert_eq!(status, StatusHttp::OK);
    esperar("o envio da org B sair", || {
        cenario.enviados(cenario.org_b) == 1
    })
    .await;
    assert_eq!(
        cenario.messages(cenario.org).len(),
        1,
        "a org A ficou como estava"
    );
    assert_eq!(cenario.enviados(cenario.org), 1);
}

#[tokio::test]
async fn conversa_encerrada_nao_envia_e_avisa_na_sala() {
    let cenario = Cenario::new("encerrada").await;
    let conversation = cenario
        .store
        .conversations()
        .get(cenario.org, cenario.conversation)
        .expect("lê a conversa");
    cenario
        .store
        .conversations()
        .close_audited(
            cenario.org,
            cenario.conversation,
            conversation.version,
            "teste".to_owned(),
        )
        .expect("encerra a conversa");
    let porta = Arc::new(HarnessResponsePort::fresh(Arc::clone(&cenario.store)));
    let servidor = cenario.subir(porta).await;

    let (status, _) = servidor
        .texto_do_atendente(
            "txn-fechada",
            SALA,
            "$evento-fechado:matrix-dev.local",
            "oi",
        )
        .await;
    assert_eq!(
        status,
        StatusHttp::OK,
        "conversa encerrada não é erro de transporte"
    );

    esperar("o aviso aparecer na sala", || {
        !cenario.escritas_no_synapse().is_empty()
    })
    .await;
    let escritas = cenario.escritas_no_synapse();
    assert_eq!(escritas.len(), 1);
    assert_eq!(
        escritas[0].body.get("body").and_then(Value::as_str),
        Some(poph_server::matrix_response::CLOSED_CONVERSATION_NOTICE)
    );
    assert_eq!(
        escritas[0].body.get("msgtype").and_then(Value::as_str),
        Some("m.notice")
    );
    assert_eq!(cenario.envios_no_whatsapp().len(), 0, "nada vai ao cliente");
    assert!(
        cenario.messages(cenario.org).is_empty(),
        "nada gravado no histórico"
    );
    assert_eq!(
        cenario.status(cenario.org),
        poph_store::ConversationStatus::Closed
    );
}

#[tokio::test]
async fn sala_que_ninguem_conhece_e_ignorada_com_200() {
    let cenario = Cenario::new("desconhecida").await;
    let porta = Arc::new(HarnessResponsePort::fresh(Arc::clone(&cenario.store)));
    let servidor = cenario.subir(porta).await;

    let (status, body) = servidor
        .texto_do_atendente(
            "txn-estranha",
            "!sala-que-nao-existe:matrix-dev.local",
            "$evento-x:matrix-dev.local",
            "oi",
        )
        .await;
    assert_eq!(status, StatusHttp::OK);
    assert_eq!(body, json!({}));
    assert_eq!(
        servidor.contadores().unknown_rooms.load(Ordering::SeqCst),
        1
    );
    assert!(cenario.messages(cenario.org).is_empty());
    assert!(cenario.messages(cenario.org_b).is_empty());
    assert_eq!(cenario.enviados(cenario.org), 0);
}

#[tokio::test]
async fn falha_de_banco_da_503_e_o_reenvio_processa() {
    let cenario = Cenario::new("falha").await;
    let porta = Arc::new(PortaComFalha::fresh(Arc::clone(&cenario.store)));
    let servidor = cenario.subir(porta.clone()).await;

    let (status, _) = servidor
        .texto_do_atendente(
            "txn-instavel",
            SALA,
            "$evento-instavel:matrix-dev.local",
            "oi",
        )
        .await;
    assert_eq!(
        status,
        StatusHttp::SERVICE_UNAVAILABLE,
        "erro transitório nosso é 5xx (o Synapse reenvia), nunca 4xx"
    );

    porta.consertar();
    let (status, _) = servidor
        .texto_do_atendente(
            "txn-instavel",
            SALA,
            "$evento-instavel:matrix-dev.local",
            "oi",
        )
        .await;
    assert_eq!(status, StatusHttp::OK, "a mesma transação é reprocessada");

    esperar("o envio sair depois do reenvio", || {
        cenario.enviados(cenario.org) == 1
    })
    .await;
    assert_eq!(cenario.envios_no_whatsapp().len(), 1);
}

#[tokio::test]
async fn arquivo_que_o_synapse_nao_entrega_mais_vira_aviso_e_nao_vai_ao_cliente() {
    // MX-44: o arquivo do atendente agora é baixado da sala. Quando o Synapse não o entrega (aqui,
    // porque o `mxc://` não existe), o cliente **não** recebe nada e o atendente é avisado na sala.
    let cenario = Cenario::new("midia").await;
    let porta = Arc::new(HarnessResponsePort::fresh(Arc::clone(&cenario.store)));
    let servidor = cenario.subir(porta).await;

    let (status, _) = servidor
        .transacao(
            "txn-midia",
            &json!({
                "events": [{
                    "type": "m.room.message",
                    "room_id": SALA,
                    "sender": ATENDENTE,
                    "event_id": "$evento-midia:matrix-dev.local",
                    "content": {
                        "msgtype": "m.image",
                        "body": "foto.png",
                        "url": "mxc://matrix-dev.local/nao-existe"
                    }
                }]
            }),
        )
        .await;
    assert_eq!(status, StatusHttp::OK);

    esperar("o aviso de mídia aparecer na sala", || {
        !cenario.escritas_no_synapse().is_empty()
    })
    .await;
    let escritas = cenario.escritas_no_synapse();
    assert_eq!(escritas.len(), 1);
    assert_eq!(
        escritas[0].body.get("body").and_then(Value::as_str),
        Some("Não consegui enviar o arquivo ao cliente: não consegui baixar o arquivo da sala")
    );
    assert_eq!(
        escritas[0].body.get("msgtype").and_then(Value::as_str),
        Some("m.notice")
    );
    assert_eq!(
        cenario.envios_no_whatsapp().len(),
        0,
        "sem o arquivo, nada sai para o cliente"
    );
    assert!(cenario.messages(cenario.org).is_empty());
    assert_eq!(
        cenario.status(cenario.org),
        poph_store::ConversationStatus::Open,
        "sem arquivo, a conversa não é transferida"
    );
}

#[tokio::test]
async fn o_nome_do_atendente_e_lido_uma_vez_e_fica_no_cache() {
    let cenario = Cenario::new("cache-nome").await;
    let porta = Arc::new(HarnessResponsePort::fresh(Arc::clone(&cenario.store)));
    let servidor = cenario.subir(porta).await;

    let (status, _) = servidor
        .texto_do_atendente("txn-n1", SALA, "$evento-n1:matrix-dev.local", "primeira")
        .await;
    assert_eq!(status, StatusHttp::OK);
    let (status, _) = servidor
        .texto_do_atendente("txn-n2", SALA, "$evento-n2:matrix-dev.local", "segunda")
        .await;
    assert_eq!(status, StatusHttp::OK);

    esperar("os dois envios saírem", || {
        cenario.envios_no_whatsapp().len() == 2
    })
    .await;
    assert_eq!(
        cenario.synapse.perfis_lidos.load(Ordering::SeqCst),
        1,
        "o perfil do mesmo atendente é lido uma vez só dentro da validade"
    );
    let envios = cenario.envios_no_whatsapp();
    assert!(
        textos_enviados(&envios)
            .iter()
            .all(|text| text.starts_with("*[Ana Atendente]*:\n"))
    );
    assert!(servidor.entrega.name_ttl() > Duration::ZERO);
}

#[tokio::test]
async fn conversa_sem_agente_nao_baixa_a_midia_nem_inventa_assinatura() {
    // Sem o agente da conversa não existe fantasma do Application Service para baixar o arquivo nem
    // para avisar na sala. O caso é relatado (`error` + contador) e a mensagem não volta para a fila
    // do Synapse: reenviar não traria o agente de volta.
    let cenario = Cenario::new("sem-agente").await;
    let porta = Arc::new(PortaSemAgente {
        real: HarnessResponsePort::fresh(Arc::clone(&cenario.store)),
    });
    let servidor = cenario.subir(porta).await;

    let (status, _) = servidor
        .transacao(
            "txn-sem-agente",
            &json!({
                "events": [{
                    "type": "m.room.message",
                    "room_id": SALA,
                    "sender": ATENDENTE,
                    "event_id": "$sem-agente:matrix-dev.local",
                    "content": {
                        "msgtype": "m.image",
                        "body": "foto.png",
                        "url": "mxc://matrix-dev.local/sem-agente"
                    }
                }]
            }),
        )
        .await;
    assert_eq!(
        status,
        StatusHttp::OK,
        "sem assinatura é falha relatada, não erro de transporte"
    );

    // Nada foi escrito na sala e nada foi ao cliente.
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        cenario.escritas_no_synapse().is_empty(),
        "sem fantasma do agente não há aviso na sala: {:?}",
        cenario.escritas_no_synapse()
    );
    assert!(
        cenario.envios_no_whatsapp().is_empty(),
        "o arquivo não vai ao cliente"
    );
    assert_eq!(
        servidor
            .entrega
            .counters()
            .unsigned_assets
            .load(Ordering::SeqCst),
        1,
        "o caso é contado para aparecer na operação"
    );
}

#[tokio::test]
async fn token_do_synapse_e_do_harness_nao_vazam_para_o_cliente_nem_para_a_sala() {
    let cenario = Cenario::new("canario").await;
    let porta = Arc::new(HarnessResponsePort::fresh(Arc::clone(&cenario.store)));
    let servidor = cenario.subir(porta).await;

    // Uma resposta do atendente (vai ao cliente) e uma mídia (vira aviso na sala): os dois
    // caminhos que escrevem fora do Harness.
    let (status, body) = servidor
        .texto_do_atendente(
            "txn-canario",
            SALA,
            "$evento-canario:matrix-dev.local",
            "oi",
        )
        .await;
    assert_eq!(status, StatusHttp::OK);
    let texto_da_resposta = body.to_string();
    assert!(!texto_da_resposta.contains(HS_TOKEN));
    assert!(!texto_da_resposta.contains(AS_TOKEN));

    let (status, _) = servidor
        .transacao(
            "txn-canario-midia",
            &json!({
                "events": [{
                    "type": "m.room.message",
                    "room_id": SALA,
                    "sender": ATENDENTE,
                    "event_id": "$evento-canario-midia:matrix-dev.local",
                    "content": { "msgtype": "m.file", "body": "contrato.pdf", "url": "mxc://x/y" }
                }]
            }),
        )
        .await;
    assert_eq!(status, StatusHttp::OK);

    esperar("os envios saírem", || {
        cenario.envios_no_whatsapp().len() == 1 && !cenario.escritas_no_synapse().is_empty()
    })
    .await;

    let para_o_cliente =
        serde_json::to_string(&cenario.envios_no_whatsapp()).expect("serializa o envio");
    assert!(!para_o_cliente.contains(HS_TOKEN));
    assert!(!para_o_cliente.contains(AS_TOKEN));

    let escritas = cenario.escritas_no_synapse();
    let para_o_synapse = escritas
        .iter()
        .map(|escrita| {
            format!(
                "{} {} {} {} {}",
                escrita.room,
                escrita.tipo,
                escrita.txn,
                escrita.disfarcado.clone().unwrap_or_default(),
                serde_json::to_string(&escrita.body).unwrap_or_default()
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!para_o_synapse.contains(HS_TOKEN));
    assert!(
        !para_o_synapse.contains(AS_TOKEN),
        "o token vai só no cabeçalho"
    );
    assert!(
        escritas[0]
            .autorizacao
            .as_deref()
            .is_some_and(|valor| valor == format!("Bearer {AS_TOKEN}")),
        "o token do Harness viaja no cabeçalho Authorization"
    );
    // O aviso sai em nome do **fantasma do agente**: o atendente humano está fora do namespace do
    // Application Service e o Synapse de verdade recusaria o disfarce com `403 M_FORBIDDEN`.
    let fantasma = cenario.agent_ghost();
    assert_eq!(
        escritas[0].disfarcado.as_deref(),
        Some(fantasma.as_str()),
        "o aviso é escrito em nome do fantasma do agente, nunca do atendente humano"
    );
    assert!(
        !fantasma.starts_with(ATENDENTE),
        "sanidade: o fantasma do agente é do namespace do AS, e o atendente não"
    );
}
