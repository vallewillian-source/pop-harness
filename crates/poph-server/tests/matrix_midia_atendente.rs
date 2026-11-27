//! Testes da MX-44: o arquivo que o atendente manda pelo Element chega ao cliente.
//!
//! Tudo aqui é de ponta a ponta, com os três servidores de verdade e banco em pasta temporária:
//!
//! - o Harness no ar (`servir_em`), com as rotas do Application Service e o consumidor da fila de
//!   saída, como o Synapse e a Evolution o veem;
//! - um **Synapse falso** em `127.0.0.1`, que serve o perfil do atendente, baixa a mídia que ele
//!   mandou (a rota de download do Matrix) e anota cada evento escrito na sala (é por ele que se
//!   prova o aviso ao atendente);
//! - uma **Evolution falsa** em `127.0.0.1`, que anota cada envio de texto e de mídia (é por ela
//!   que se prova o que o cliente recebeu).
//!
//! O que se prova:
//! - imagem e PDF do Element viram `sendMedia` na Evolution, com o tipo certo e a legenda
//!   `*[Nome do atendente]*`;
//! - a mensagem fica no histórico como `human`, com o anexo, e o item `file` na fila de saída tem o
//!   payload do HR-24;
//! - o **download** e o **aviso** saem em nome do fantasma do agente, e o Synapse falso recusa
//!   `403 M_FORBIDDEN` quem está fora do namespace do Application Service, como o de verdade: sem
//!   isso, assinar no nome do atendente humano passava batido aqui e só quebrava no Synapse real
//!   (foi o que o aceite da fase 2 pegou);
//! - reentrega do mesmo evento (transação repetida ou transação nova) não gera segundo envio;
//! - arquivo acima de 16 MB e tipo recusado viram aviso na sala, **sem** envio ao cliente;
//! - conversa encerrada não envia o arquivo e avisa na sala;
//! - nenhum token do Synapse ou do Harness aparece no que vai ao cliente.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use axum::extract::{Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use poph_evolution::{ATTACHMENT_CAP, ConfigEvolution, EvolutionClient};
use poph_matrix::{ClientConfig, GhostIds, MatrixClient};
use poph_server::evolution::CanalEvolution;
use poph_server::{AsReceiver, AttendantResponse, Estado, HarnessResponsePort};
use poph_store::{NewAgent, NewConversation, NewOrg, Origin, OutboxState, Page, Store};
use poph_types::{ConversationId, HarnessConfig, IntegrationId, OrgId, Secret};
use reqwest::{Client, StatusCode as StatusHttp};
use serde_json::{Value, json};
use tokio::sync::oneshot;

// ───────────────────────────────── constantes ─────────────────────────────────

const PREFIXO: &str = "ph";
const SERVER_NAME: &str = "matrix-dev.local";

/// Valor-canário do token do Synapse: não pode aparecer no que vai ao cliente.
const HS_TOKEN: &str = "CANARIO-HS-mx44-7b21-TAO-SECRETO";

/// Valor-canário do token com que o Harness age no Synapse.
const AS_TOKEN: &str = "CANARIO-AS-mx44-c4d8-TAO-SECRETO";

const INSTANCIA: &str = "loja";
const CHAVE_EVO: &str = "chave-evo-mx44-teste";
const SEGREDO_WEBHOOK: &str = "segredo-webhook-mx44";

/// Número do cliente. Não é de ninguém.
const TELEFONE: &str = "5511999990000";

/// Atendente humano: `localpart` que **não** pode ser confundida com fantasma.
const ATENDENTE: &str = "@ana:matrix-dev.local";
const NOME_ATENDENTE: &str = "Ana Atendente";

/// Sala da conversa do cenário.
const SALA: &str = "!sala-a:matrix-dev.local";

/// PNG 1×1 transparente: imagem de verdade, pequena e sem dado de ninguém.
const PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4,
    0x89, 0x00, 0x00, 0x00, 0x0a, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0x00, 0x01, 0x00, 0x00,
    0x05, 0x00, 0x01, 0x0d, 0x0a, 0x2d, 0xb4, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae,
    0x42, 0x60, 0x82,
];

/// PDF mínimo com a assinatura que o `poph-files` reconhece (documento de mentira, sem conteúdo).
const PDF: &[u8] =
    b"%PDF-1.4\n1 0 obj\n<< /Type /Catalog >>\nendobj\ntrailer\n<< /Root 1 0 R >>\n%%EOF\n";

/// ZIP qualquer: tipo que o Harness não envia ao cliente (o `poph-files` recusa compactado).
const ZIP: &[u8] = b"PK\x03\x04\x14\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00";

/// Áudio OGG com cabeçalho `OpusHead`: o formato da gravação de voz do Element (MX-48).
///
/// Bytes fixos de fixture, sem áudio de ninguém: o teste só precisa que o conteúdo seja
/// reconhecido como áudio e que chegue inteiro ao outro lado.
const AUDIO_OGG: &[u8] = &[
    0x4f, 0x67, 0x67, 0x53, 0x00, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x01, 0x00, 0x00, 0x00, 0x4f, 0x70, 0x75, 0x73, 0x48, 0x65, 0x61, 0x64, 0x01, 0x02, 0x38, 0x01,
    0x80, 0xbb, 0x00, 0x00,
];

/// Vídeo MP4 mínimo: caixa `ftyp` com a marca `isom` (MX-48). Também sem conteúdo de ninguém.
const VIDEO_MP4: &[u8] = &[
    0x00, 0x00, 0x00, 0x18, 0x66, 0x74, 0x79, 0x70, 0x69, 0x73, 0x6f, 0x6d, 0x00, 0x00, 0x02, 0x00,
    0x69, 0x73, 0x6f, 0x6d, 0x69, 0x73, 0x6f, 0x32,
];

// ───────────────────────────────── Synapse falso ─────────────────────────────────

/// Um evento de mensagem que o Harness escreveu na sala (o aviso ao atendente, por exemplo).
#[derive(Clone, Debug)]
struct Escrita {
    room: String,
    tipo: String,
    body: Value,
    /// `?user_id=` com que o evento foi escrito (o fantasma que assinou).
    disfarce: Option<String>,
}

/// Namespace do Application Service no registro do ambiente (`@<prefixo>_.*`). O Synapse de
/// verdade só deixa o `as_token` personificar quem casa com ele; quem está fora recebe
/// `403 M_FORBIDDEN` ("Application service cannot masquerade as this user").
const NAMESPACE: &str = "@ph_";

/// O Synapse aceitaria este disfarce? Sem `?user_id=` a chamada age como o bot do AS, que é do
/// namespace.
fn pode_personificar(usuario: Option<&str>) -> bool {
    match usuario {
        None => true,
        Some(usuario) => usuario.starts_with(NAMESPACE),
    }
}

fn recusa_de_disfarce() -> (StatusCode, Json<Value>) {
    (
        StatusCode::FORBIDDEN,
        Json(json!({
            "errcode": "M_FORBIDDEN",
            "error": "Application service cannot masquerade as this user"
        })),
    )
}

#[derive(Clone, Default)]
struct SynapseFalso {
    perfis: Arc<Mutex<HashMap<String, String>>>,
    escritas: Arc<Mutex<Vec<Escrita>>>,
    /// Mídia por id, como o repositório de mídia do Synapse: é daqui que o download responde.
    midias: Arc<Mutex<HashMap<String, Vec<u8>>>>,
    /// Ids que o download deve recusar com um status (o Synapse fora do ar, por exemplo).
    erros: Arc<Mutex<HashMap<String, u16>>>,
    /// `?user_id=` de cada download de mídia, na ordem (o fantasma que baixou).
    downloads: Arc<Mutex<Vec<Option<String>>>>,
    /// Downloads recusados pelo namespace (o pedido não chegou ao conteúdo).
    recusas_de_disfarce: Arc<Mutex<u64>>,
}

async fn perfil_falso(
    State(falso): State<SynapseFalso>,
    Path(usuario): Path<String>,
) -> (StatusCode, Json<Value>) {
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
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    let disfarce = consulta.get("user_id").cloned();
    if !pode_personificar(disfarce.as_deref()) {
        *falso.recusas_de_disfarce.lock().unwrap() += 1;
        return recusa_de_disfarce();
    }
    let mut escritas = falso.escritas.lock().unwrap();
    let posicao = escritas.len() + 1;
    escritas.push(Escrita {
        room,
        tipo,
        body,
        disfarce,
    });
    let _ = txn;
    (
        StatusCode::OK,
        Json(json!({ "event_id": format!("$escrito-{posicao}:matrix-dev.local") })),
    )
}

/// Rota nova de download (`/_matrix/client/v1/media/download/{servidor}/{id}`) e a depreciada —
/// o Synapse atual só atende a nova, então o falso serve as duas: é o caminho principal que interessa.
async fn baixar_falso_novo(
    state: State<SynapseFalso>,
    path: Path<(String, String)>,
    Query(consulta): Query<HashMap<String, String>>,
) -> Response {
    baixar_falso(state, path, Query(consulta)).await
}

/// `GET /_matrix/media/v3/download/{servidor}/{id}`: o conteúdo da mídia, ou `404 M_NOT_FOUND`.
///
/// Como o Synapse de verdade, recusa `403 M_FORBIDDEN` quando o `?user_id=` está fora do namespace
/// do Application Service: é o que impede o defeito de "baixar no nome do atendente humano" de
/// passar batido no teste.
async fn baixar_falso(
    State(falso): State<SynapseFalso>,
    Path((_servidor, id)): Path<(String, String)>,
    Query(consulta): Query<HashMap<String, String>>,
) -> Response {
    let disfarce = consulta.get("user_id").cloned();
    falso.downloads.lock().unwrap().push(disfarce.clone());
    if !pode_personificar(disfarce.as_deref()) {
        *falso.recusas_de_disfarce.lock().unwrap() += 1;
        return recusa_de_disfarce().into_response();
    }
    if let Some(status) = falso.erros.lock().unwrap().get(&id).copied() {
        return (
            StatusCode::from_u16(status).expect("status válido"),
            Json(json!({ "errcode": "M_UNKNOWN", "error": "indisponível" })),
        )
            .into_response();
    }
    let bytes = falso.midias.lock().unwrap().get(&id).cloned();
    match bytes {
        Some(bytes) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "application/octet-stream")],
            bytes,
        )
            .into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({ "errcode": "M_NOT_FOUND", "error": "Not found" })),
        )
            .into_response(),
    }
}

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
        .route(
            "/_matrix/client/v1/media/download/{server}/{id}",
            get(baixar_falso_novo),
        )
        .route(
            "/_matrix/media/v3/download/{server}/{id}",
            get(baixar_falso),
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

async fn esperar_synapse(url: &str) {
    let client = Client::new();
    let inicio = Instant::now();
    loop {
        if client.get(format!("{url}/pronto")).send().await.is_ok() {
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

/// Um envio que a Evolution falsa recebeu.
#[derive(Clone, Debug)]
struct Send {
    rota: String,
    body: Value,
}

#[derive(Clone, Default)]
struct EvolucaoFalsa {
    envios: Arc<Mutex<Vec<Send>>>,
}

async fn enviar_pelo_whatsapp(
    State(falsa): State<EvolucaoFalsa>,
    Path((rota, _instancia)): Path<(String, String)>,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    let mut envios = falsa.envios.lock().unwrap();
    envios.push(Send { rota, body });
    let posicao = envios.len();
    (
        StatusCode::OK,
        Json(json!({ "key": { "id": format!("MSG-EVO-{posicao}") } })),
    )
}

async fn subir_evolution_falsa() -> (String, EvolucaoFalsa) {
    let falsa = EvolucaoFalsa::default();
    let roteador = Router::new()
        .route("/message/{rota}/{instance}", post(enviar_pelo_whatsapp))
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

// ───────────────────────────────── cenário ─────────────────────────────────

struct Cenario {
    pasta: PathBuf,
    store: Arc<Store>,
    org: OrgId,
    conversation: ConversationId,
    synapse: SynapseFalso,
    evo: EvolucaoFalsa,
    url_synapse: String,
    url_evo: String,
    /// Onde os blobs das orgs ficam, como na operação (`<dados>/blobs`).
    ///
    /// É a **mesma** pasta que o consumidor da fila de saída lê (`config.blobs()`): quem grava e
    /// quem envia precisam apontar para o mesmo lugar, como no servidor de verdade.
    pasta_blobs: PathBuf,
    /// Diretório de dados do servidor (`dados.diretorio` da configuração).
    diretorio: PathBuf,
}

impl Cenario {
    async fn new(rotulo: &str) -> Self {
        let carimbo = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|duracao| duracao.as_nanos())
            .unwrap_or(0);
        let pasta = std::env::temp_dir().join(format!(
            "poph-matrix-midia-atendente-{rotulo}-{}-{carimbo}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&pasta);
        fs::create_dir_all(&pasta).expect("pasta do teste");
        let store = Arc::new(
            Store::open_at(&pasta.join("harness.db"), &pasta.join("rag.db")).expect("abre o banco"),
        );

        let org = store
            .orgs()
            .create(NewOrg {
                name: "Org do cenário".to_owned(),
                slug: "loja-a".to_owned(),
                config_json: String::new(),
            })
            .expect("cria a org")
            .id;
        let agent = store
            .agents()
            .create(
                org,
                NewAgent {
                    name: "Agente de Teste".to_owned(),
                    slug: "agente-teste".to_owned(),
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

        store.matrix().enable(org).expect("liga o módulo na org");
        store
            .matrix()
            .ensure_room(org, conversation, SALA, None)
            .expect("grava a sala");
        store
            .matrix()
            .mark_room_ready(org, conversation)
            .expect("sala pronta");

        let (url_synapse, synapse) = subir_synapse_falso().await;
        esperar_synapse(&url_synapse).await;
        let (url_evo, evo) = subir_evolution_falsa().await;
        let diretorio = pasta.join("dados");
        Self {
            pasta_blobs: diretorio.join("blobs"),
            diretorio,
            pasta,
            store,
            org,
            conversation,
            synapse,
            evo,
            url_synapse,
            url_evo,
        }
    }

    /// Faz o download desta mídia responder o status dado (o Synapse fora do ar, por exemplo).
    fn recusar(&self, id: &str, status: u16) {
        self.synapse
            .erros
            .lock()
            .unwrap()
            .insert(id.to_owned(), status);
    }

    /// Publica uma mídia no Synapse falso e devolve o `mxc://` que o evento do atendente carrega.
    fn publicar(&self, id: &str, bytes: &[u8]) -> String {
        self.synapse
            .midias
            .lock()
            .unwrap()
            .insert(id.to_owned(), bytes.to_vec());
        format!("mxc://{SERVER_NAME}/{id}")
    }

    fn channel(&self) -> CanalEvolution {
        let client = EvolutionClient::new(
            ConfigEvolution::new(
                self.url_evo.clone(),
                INSTANCIA,
                Secret::new(CHAVE_EVO.to_owned()),
            )
            .expect("config da Evolution falsa"),
        )
        .expect("cliente da Evolution falsa");
        let integracao = self
            .store
            .conversations()
            .get(self.org, self.conversation)
            .expect("lê a conversa")
            .integration_id
            .expect("a conversa tem integração");
        CanalEvolution {
            org: self.org,
            instancia: INSTANCIA.to_owned(),
            integracao,
            segredo: Some(Secret::new(SEGREDO_WEBHOOK.to_owned())),
            cliente: Some(client),
            teto_midia: ATTACHMENT_CAP,
        }
    }

    async fn subir(&self) -> Servidor {
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
        let entrega = Arc::new(
            AttendantResponse::new(
                Arc::new(HarnessResponsePort::fresh(Arc::clone(&self.store))),
                Some(cliente_matrix),
                ids,
            )
            .with_blob_folder(self.pasta_blobs.clone()),
        );

        let mut config = HarnessConfig::default();
        config.data.directory = self.diretorio.clone();
        let state = Estado::novo(Arc::clone(&self.store), config)
            .expect("estado do servidor")
            .com_canais_evolution(vec![self.channel()])
            .com_matrix_as(Arc::new(AsReceiver::new(
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

    fn messages(&self) -> Vec<poph_store::Mensagem> {
        self.store
            .messages()
            .list(self.org, self.conversation, Page::with_limit(50))
            .expect("lista as mensagens")
    }

    fn anexos(&self, indice: usize) -> Vec<poph_store::AttachmentRecord> {
        let mensagens = self.messages();
        self.store
            .messages()
            .attachments(self.org, mensagens[indice].id)
            .expect("lista os anexos")
    }

    fn arquivos_na_fila(&self) -> Vec<poph_store::OutboxItem> {
        self.store
            .outbox()
            .list_files_of_conversation(self.org, self.conversation, 50)
            .expect("lista os itens de arquivo da fila")
    }

    fn envios(&self) -> Vec<Send> {
        self.evo.envios.lock().unwrap().clone()
    }

    /// O fantasma do agente dono da conversa: quem o Synapse aceita personificar (namespace do AS)
    /// e quem está na sala. É ele que baixa o arquivo e assina o aviso.
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

    /// `?user_id=` de cada download de mídia que o Synapse falso recebeu.
    fn downloads(&self) -> Vec<Option<String>> {
        self.synapse.downloads.lock().unwrap().clone()
    }

    /// Quantos pedidos o Synapse falso recusou por disfarce fora do namespace do AS.
    fn recusas_de_disfarce(&self) -> u64 {
        *self.synapse.recusas_de_disfarce.lock().unwrap()
    }

    fn avisos_na_sala(&self) -> Vec<Escrita> {
        self.synapse.escritas.lock().unwrap().clone()
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

    async fn transacao(&self, txn: &str, body: &Value) -> StatusHttp {
        let resposta = self
            .client
            .put(format!("{}/_matrix/app/v1/transactions/{txn}", self.base))
            .header("authorization", format!("Bearer {HS_TOKEN}"))
            .json(body)
            .send()
            .await
            .expect("entrega a transação");
        resposta.status()
    }

    /// Entrega um arquivo do atendente numa transação própria.
    async fn arquivo_do_atendente(
        &self,
        txn: &str,
        event: &str,
        msgtype: &str,
        name: &str,
        url: Option<&str>,
    ) -> StatusHttp {
        let mut conteudo = json!({ "msgtype": msgtype, "body": name, "filename": name });
        if let Some(url) = url {
            conteudo["url"] = json!(url);
        }
        self.transacao(
            txn,
            &json!({
                "events": [{
                    "type": "m.room.message",
                    "room_id": SALA,
                    "sender": ATENDENTE,
                    "event_id": event,
                    "origin_server_ts": 1_795_300_000_000i64,
                    "content": conteudo
                }]
            }),
        )
        .await
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

/// Decodifica base64 (alfabeto padrão, com `=` de preenchimento).
///
/// Vinte linhas no lugar de uma dependência nova: o teste só precisa conferir que os bytes chegam
/// inteiros à Evolution, e o `Cargo.lock` não muda por causa de um teste.
fn decodificar_base64(text: &str) -> Vec<u8> {
    fn valor(caractere: u8) -> Option<u8> {
        match caractere {
            b'A'..=b'Z' => Some(caractere - b'A'),
            b'a'..=b'z' => Some(caractere - b'a' + 26),
            b'0'..=b'9' => Some(caractere - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    let mut saida = Vec::new();
    let mut grupo = [0u8; 4];
    let mut tamanho = 0;
    for byte in text.bytes() {
        if byte == b'=' || byte.is_ascii_whitespace() {
            continue;
        }
        grupo[tamanho] = valor(byte).unwrap_or_else(|| panic!("base64 inválido: {byte}"));
        tamanho += 1;
        if tamanho == 4 {
            saida.push((grupo[0] << 2) | (grupo[1] >> 4));
            saida.push((grupo[1] << 4) | (grupo[2] >> 2));
            saida.push((grupo[2] << 6) | grupo[3]);
            tamanho = 0;
        }
    }
    match tamanho {
        2 => saida.push((grupo[0] << 2) | (grupo[1] >> 4)),
        3 => {
            saida.push((grupo[0] << 2) | (grupo[1] >> 4));
            saida.push((grupo[1] << 4) | (grupo[2] >> 2));
        }
        _ => {}
    }
    saida
}

/// Corpo do aviso escrito na sala, se houver exatamente um — conferindo **quem assinou**.
///
/// O aviso tem de sair em nome do fantasma do agente: o atendente humano está fora do namespace do
/// Application Service, e o Synapse de verdade recusaria o disfarce com `403 M_FORBIDDEN` (o
/// atendente ficaria sem aviso nenhum). O Synapse falso daqui faz a mesma recusa.
fn unico_aviso(cenario: &Cenario) -> String {
    let avisos = cenario.avisos_na_sala();
    assert_eq!(avisos.len(), 1, "esperava um aviso na sala: {avisos:?}");
    assert_eq!(avisos[0].room, SALA, "o aviso vai para a sala do atendente");
    assert_eq!(avisos[0].tipo, "m.room.message");
    assert_eq!(avisos[0].body["msgtype"], json!("m.notice"));
    assert_eq!(
        avisos[0].disfarce.as_deref(),
        Some(cenario.agent_ghost().as_str()),
        "o aviso é assinado pelo fantasma do agente, nunca pelo atendente humano"
    );
    assert_eq!(
        cenario.recusas_de_disfarce(),
        0,
        "nenhum pedido pode ter sido recusado por disfarce fora do namespace do AS"
    );
    avisos[0].body["body"]
        .as_str()
        .expect("corpo do aviso é texto")
        .to_owned()
}

// ───────────────────────────────── testes ─────────────────────────────────

#[tokio::test]
async fn imagem_e_pdf_do_element_chegam_ao_whatsapp_com_a_legenda_do_atendente() {
    let cenario = Cenario::new("ok").await;
    let servidor = cenario.subir().await;
    let url_png = cenario.publicar("png-mx44", PNG);
    let url_pdf = cenario.publicar("pdf-mx44", PDF);

    let status = servidor
        .transacao(
            "txn-midia",
            &json!({
                "events": [
                    {
                        "type": "m.room.message",
                        "room_id": SALA,
                        "sender": ATENDENTE,
                        "event_id": "$img-mx44:matrix-dev.local",
                        "content": {
                            "msgtype": "m.image",
                            "body": "foto.png",
                            "filename": "foto.png",
                            "url": url_png,
                            "info": { "mimetype": "image/png", "size": PNG.len() }
                        }
                    },
                    {
                        "type": "m.room.message",
                        "room_id": SALA,
                        "sender": ATENDENTE,
                        "event_id": "$pdf-mx44:matrix-dev.local",
                        "content": {
                            "msgtype": "m.file",
                            "body": "laudo.pdf",
                            "filename": "laudo.pdf",
                            "url": url_pdf,
                            "info": { "mimetype": "application/pdf", "size": PDF.len() }
                        }
                    }
                ]
            }),
        )
        .await;
    assert_eq!(status, StatusHttp::OK);

    esperar("os dois arquivos saírem para o cliente", || {
        cenario
            .envios()
            .iter()
            .filter(|send| send.rota == "sendMedia")
            .count()
            == 2
    })
    .await;

    let envios = cenario.envios();
    assert!(
        envios.iter().all(|send| send.rota == "sendMedia"),
        "nenhum texto deveria sair: {envios:?}"
    );
    let midias: Vec<&Send> = envios
        .iter()
        .filter(|send| send.rota == "sendMedia")
        .collect();
    let (primeira, segunda) = (midias[0], midias[1]);

    // Imagem: `image`, o mime real e o nome do arquivo.
    assert_eq!(primeira.body["mediatype"], json!("image"));
    assert_eq!(primeira.body["mimetype"], json!("image/png"));
    assert_eq!(primeira.body["fileName"], json!("foto.png"));
    assert_eq!(primeira.body["number"], json!(TELEFONE));
    assert_eq!(primeira.body["caption"], json!("*[Ana Atendente]*"));
    let enviado = decodificar_base64(primeira.body["media"].as_str().expect("mídia em base64"));
    assert_eq!(enviado, PNG, "os bytes chegam inteiros");

    // PDF: `document`, o mime real e a mesma legenda.
    assert_eq!(segunda.body["mediatype"], json!("document"));
    assert_eq!(segunda.body["mimetype"], json!("application/pdf"));
    assert_eq!(segunda.body["fileName"], json!("laudo.pdf"));
    assert_eq!(segunda.body["caption"], json!("*[Ana Atendente]*"));
    let enviado = decodificar_base64(segunda.body["media"].as_str().expect("mídia em base64"));
    assert_eq!(enviado, PDF, "os bytes chegam inteiros");

    // Nada de aviso na sala: os dois arquivos foram.
    assert!(
        cenario.avisos_na_sala().is_empty(),
        "sem aviso quando o arquivo vai: {:?}",
        cenario.avisos_na_sala()
    );

    // O download saiu em nome do fantasma do agente: o atendente humano está fora do namespace do
    // Application Service e o Synapse recusaria o disfarce com `403 M_FORBIDDEN`.
    let fantasma = cenario.agent_ghost();
    assert_eq!(
        cenario.downloads(),
        vec![Some(fantasma.clone()), Some(fantasma)],
        "um download por arquivo, os dois disfarçados do fantasma do agente"
    );
    assert_eq!(
        cenario.recusas_de_disfarce(),
        0,
        "nenhum pedido recusado por disfarce"
    );

    // Histórico: duas mensagens `human`, com anexo e sem texto.
    let mensagens = cenario.messages();
    assert_eq!(mensagens.len(), 2);
    for (indice, (name, mime, tamanho)) in [
        ("foto.png", "image/png", PNG.len()),
        ("laudo.pdf", "application/pdf", PDF.len()),
    ]
    .into_iter()
    .enumerate()
    {
        let message = &mensagens[indice];
        assert_eq!(message.origin, Origin::Human);
        assert_eq!(message.author_name.as_deref(), Some(NOME_ATENDENTE));
        assert_eq!(message.texto, "", "o atendente não digitou texto");
        assert_eq!(
            message.external_id.as_deref(),
            Some(if indice == 0 {
                "$img-mx44:matrix-dev.local"
            } else {
                "$pdf-mx44:matrix-dev.local"
            })
        );
        let anexos = cenario.anexos(indice);
        assert_eq!(anexos.len(), 1, "cada mensagem leva o seu anexo");
        assert_eq!(anexos[0].name, name);
        assert_eq!(anexos[0].mime, mime);
        assert_eq!(anexos[0].size_bytes, tamanho as u64);
        assert_eq!(anexos[0].extraction_status, "skipped");
        let blob = cenario.pasta_blobs.join(&anexos[0].blob_path);
        assert!(blob.is_file(), "o blob ficou no disco: {}", blob.display());
    }

    // Fila de saída: dois itens `file`, com o payload do HR-24 e a legenda em Markdown.
    let arquivos = cenario.arquivos_na_fila();
    assert_eq!(arquivos.len(), 2);
    for (item, (name, mime)) in arquivos
        .iter()
        .zip([("foto.png", "image/png"), ("laudo.pdf", "application/pdf")])
    {
        assert_eq!(item.tipo, "file");
        let payload: Value = serde_json::from_str(&item.content_json).expect("payload em JSON");
        assert_eq!(payload["nome"], json!(name));
        assert_eq!(payload["mime"], json!(mime));
        assert_eq!(payload["legenda"], json!("**[Ana Atendente]**"));
        assert!(
            payload["blob_ref"]
                .as_str()
                .is_some_and(|referencia| referencia.starts_with(&format!("{}/", cenario.org))),
            "blob_ref dentro da pasta da org: {}",
            payload["blob_ref"]
        );
        assert_eq!(payload["sha256"].as_str().map(str::len), Some(64));
        assert!(payload["arquivo_id"].is_string());
    }

    // A conversa fica transferida (o atendente assumiu) e a fila foi entregue.
    assert_eq!(
        cenario
            .store
            .conversations()
            .get(cenario.org, cenario.conversation)
            .expect("lê a conversa")
            .status,
        poph_store::ConversationStatus::Transferred
    );
    assert_eq!(
        cenario
            .store
            .outbox()
            .count_by_state(cenario.org, OutboxState::Sent)
            .expect("conta a fila"),
        2
    );

    // Nada de token no que foi para o cliente.
    let corpos: Vec<&Value> = envios.iter().map(|send| &send.body).collect();
    let para_o_cliente = serde_json::to_string(&corpos).expect("serializa os envios");
    assert!(!para_o_cliente.contains(HS_TOKEN));
    assert!(!para_o_cliente.contains(AS_TOKEN));
}

/// O defeito que o aceite da fase 2 pegou, preso no teste: o Application Service não pode
/// personificar o atendente humano, e o Synapse falso precisa recusar isso como o de verdade.
///
/// Com o falso permissivo (o que existia antes), a MX-44 passava aqui e só quebrava no Synapse real:
/// o download voltava `403 M_FORBIDDEN`, nenhum arquivo chegava ao cliente e o aviso — que usava o
/// mesmo remetente — também não aparecia. Aqui as duas metades ficam provadas: recusa para o
/// humano, conteúdo inteiro para o fantasma do agente.
#[tokio::test]
async fn synapse_falso_recusa_disfarce_de_humano_e_entrega_para_o_fantasma_do_agente() {
    let cenario = Cenario::new("disfarce").await;
    let url = cenario.publicar("png-disfarce", PNG);
    let fantasma = cenario.agent_ghost();
    let client = MatrixClient::new(
        ClientConfig::new(
            cenario.url_synapse.clone(),
            SERVER_NAME.to_owned(),
            Secret::new(AS_TOKEN.to_owned()),
        )
        .expect("config do cliente do Matrix"),
    )
    .expect("cliente do Matrix");

    let negado = client
        .download_asset(ATENDENTE, &url, poph_matrix::asset::ASSET_BYTES_LIMIT)
        .await
        .expect_err("o humano está fora do namespace do Application Service");
    assert!(
        !negado.transient(),
        "recusa por namespace é definitiva: {negado:?}"
    );
    assert_eq!(
        cenario.recusas_de_disfarce(),
        1,
        "o Synapse falso recusou o disfarce de humano, como o de verdade"
    );

    let bytes = client
        .download_asset(&fantasma, &url, poph_matrix::asset::ASSET_BYTES_LIMIT)
        .await
        .unwrap_or_else(|error| panic!("o fantasma do agente baixa o arquivo ({error:?})"));
    assert_eq!(
        bytes, PNG,
        "o conteúdo desce inteiro pelo fantasma do agente"
    );
}

#[tokio::test]
async fn reentrega_do_mesmo_arquivo_nao_duplica_o_envio() {
    let cenario = Cenario::new("reentrega").await;
    let servidor = cenario.subir().await;
    let url = cenario.publicar("png-reentrega", PNG);

    let primeiro = servidor
        .arquivo_do_atendente(
            "txn-1",
            "$img-rep:matrix-dev.local",
            "m.image",
            "foto.png",
            Some(&url),
        )
        .await;
    // Mesma transação outra vez: a MX-16 lembra o `txnId` e nem chama a entrega.
    let segundo = servidor
        .arquivo_do_atendente(
            "txn-1",
            "$img-rep:matrix-dev.local",
            "m.image",
            "foto.png",
            Some(&url),
        )
        .await;
    // Transação nova com o mesmo evento (o Synapse pode remontar o lote): a idempotência é do
    // `event_id`, na operação atômica da MX-44.
    let terceiro = servidor
        .arquivo_do_atendente(
            "txn-2",
            "$img-rep:matrix-dev.local",
            "m.image",
            "foto.png",
            Some(&url),
        )
        .await;
    assert_eq!(primeiro, StatusHttp::OK);
    assert_eq!(segundo, StatusHttp::OK);
    assert_eq!(terceiro, StatusHttp::OK);

    esperar("o arquivo sair uma vez", || {
        cenario
            .store
            .outbox()
            .count_by_state(cenario.org, OutboxState::Sent)
            .unwrap_or(0)
            == 1
    })
    .await;
    tokio::time::sleep(Duration::from_millis(300)).await;

    let midias = cenario
        .envios()
        .into_iter()
        .filter(|send| send.rota == "sendMedia")
        .count();
    assert_eq!(midias, 1, "reentrega não manda o arquivo de novo");
    assert_eq!(cenario.messages().len(), 1, "uma mensagem no histórico");
    assert_eq!(cenario.arquivos_na_fila().len(), 1, "um item na fila");
    assert_eq!(
        servidor
            .entrega
            .counters()
            .duplicates
            .load(Ordering::SeqCst),
        1,
        "a transação nova com o mesmo evento foi reconhecida como duplicada"
    );
}

#[tokio::test]
async fn arquivo_acima_do_limite_vira_aviso_na_sala_e_nao_e_enviado() {
    let cenario = Cenario::new("grande").await;
    let servidor = cenario.subir().await;
    let grande = vec![0u8; (16 * 1024 * 1024) + 1];
    let url = cenario.publicar("grande-mx44", &grande);

    let status = servidor
        .arquivo_do_atendente(
            "txn-grande",
            "$img-grande:matrix-dev.local",
            "m.image",
            "foto.png",
            Some(&url),
        )
        .await;
    assert_eq!(
        status,
        StatusHttp::OK,
        "arquivo grande não é erro de transporte"
    );

    esperar("o aviso do limite aparecer na sala", || {
        !cenario.avisos_na_sala().is_empty()
    })
    .await;
    assert_eq!(
        unico_aviso(&cenario),
        "Não consegui enviar o arquivo ao cliente: o arquivo passa de 16 MB"
    );
    // Tempo para um envio aparecer, se ele existisse.
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        cenario.envios().is_empty(),
        "arquivo grande não vai ao cliente: {:?}",
        cenario.envios()
    );
    assert!(cenario.messages().is_empty(), "nada no histórico");
    assert!(cenario.arquivos_na_fila().is_empty(), "nada na fila");
}

#[tokio::test]
async fn tipo_recusado_vira_aviso_na_sala_e_nao_e_enviado() {
    let cenario = Cenario::new("tipo").await;
    let servidor = cenario.subir().await;
    let url = cenario.publicar("zip-mx44", ZIP);

    // O atendente manda um ZIP dizendo que é PDF: o tipo é conferido pelo conteúdo, não pelo nome.
    let status = servidor
        .arquivo_do_atendente(
            "txn-zip",
            "$zip-mx44:matrix-dev.local",
            "m.file",
            "laudo.pdf",
            Some(&url),
        )
        .await;
    assert_eq!(status, StatusHttp::OK);

    esperar("o aviso de tipo aparecer na sala", || {
        !cenario.avisos_na_sala().is_empty()
    })
    .await;
    assert_eq!(
        unico_aviso(&cenario),
        "Não consegui enviar o arquivo ao cliente: o tipo deste arquivo não é aceito (o cliente recebe imagem, PDF, Office, áudio e vídeo)"
    );
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        cenario.envios().is_empty(),
        "tipo recusado não vai ao cliente"
    );
    assert!(cenario.messages().is_empty(), "nada no histórico");
}

#[tokio::test]
async fn arquivo_sem_url_no_evento_vira_aviso_na_sala() {
    let cenario = Cenario::new("sem-url").await;
    let servidor = cenario.subir().await;

    let status = servidor
        .arquivo_do_atendente(
            "txn-sem-url",
            "$sem-url:matrix-dev.local",
            "m.image",
            "foto.png",
            None,
        )
        .await;
    assert_eq!(status, StatusHttp::OK);

    esperar("o aviso aparecer na sala", || {
        !cenario.avisos_na_sala().is_empty()
    })
    .await;
    assert_eq!(
        unico_aviso(&cenario),
        "Não consegui enviar o arquivo ao cliente: o evento não trouxe o arquivo"
    );
    assert!(cenario.envios().is_empty());
}

#[tokio::test]
async fn conversa_encerrada_nao_envia_o_arquivo_e_avisa_na_sala() {
    let cenario = Cenario::new("encerrada").await;
    let url = cenario.publicar("png-encerrada", PNG);
    let versao = cenario
        .store
        .conversations()
        .get(cenario.org, cenario.conversation)
        .expect("lê a conversa")
        .version;
    cenario
        .store
        .conversations()
        .close_audited(
            cenario.org,
            cenario.conversation,
            versao,
            "teste".to_owned(),
        )
        .expect("encerra a conversa");

    let servidor = cenario.subir().await;
    let status = servidor
        .arquivo_do_atendente(
            "txn-fechada",
            "$fechada:matrix-dev.local",
            "m.image",
            "foto.png",
            Some(&url),
        )
        .await;
    assert_eq!(
        status,
        StatusHttp::OK,
        "conversa encerrada não é erro de transporte"
    );

    esperar("o aviso de conversa encerrada aparecer na sala", || {
        !cenario.avisos_na_sala().is_empty()
    })
    .await;
    assert_eq!(
        unico_aviso(&cenario),
        poph_server::matrix_response::CLOSED_CONVERSATION_NOTICE
    );
    assert!(
        cenario.envios().is_empty(),
        "conversa encerrada não manda o arquivo"
    );
    assert!(cenario.messages().is_empty(), "nada no histórico");
    assert_eq!(
        cenario
            .store
            .conversations()
            .get(cenario.org, cenario.conversation)
            .expect("lê a conversa")
            .status,
        poph_store::ConversationStatus::Closed
    );
}

#[tokio::test]
async fn synapse_fora_nao_vira_aviso_e_volta_no_reenvio() {
    // Falha transitória do download é falha **nossa**: `5xx` para o Synapse reenviar, sem aviso e
    // sem envio. Quando o Synapse volta, a mesma mensagem (transação nova) entrega o arquivo.
    let cenario = Cenario::new("transitoria").await;
    let servidor = cenario.subir().await;
    cenario.recusar("png-instavel", 503);
    let url = format!("mxc://{SERVER_NAME}/png-instavel");

    let status = servidor
        .arquivo_do_atendente(
            "txn-instavel",
            "$instavel:matrix-dev.local",
            "m.image",
            "foto.png",
            Some(&url),
        )
        .await;
    assert_eq!(
        status,
        StatusHttp::SERVICE_UNAVAILABLE,
        "falha transitória do download é 5xx (o Synapse reenvia), nunca aviso"
    );
    assert!(
        cenario.avisos_na_sala().is_empty(),
        "sem aviso quando o arquivo pode voltar: {:?}",
        cenario.avisos_na_sala()
    );
    assert!(cenario.envios().is_empty());
    assert!(cenario.messages().is_empty());

    // O Synapse volta: a entrega seguinte (transação nova, mesmo evento) leva o arquivo.
    cenario.publicar("png-instavel", PNG);
    cenario.synapse.erros.lock().unwrap().clear();
    let status = servidor
        .arquivo_do_atendente(
            "txn-instavel-2",
            "$instavel:matrix-dev.local",
            "m.image",
            "foto.png",
            Some(&url),
        )
        .await;
    assert_eq!(status, StatusHttp::OK);
    esperar("o arquivo sair no reenvio", || {
        cenario.envios().iter().any(|send| send.rota == "sendMedia")
    })
    .await;
    assert_eq!(cenario.messages().len(), 1, "uma mensagem, no reenvio");
}

/// MX-48: áudio e vídeo do atendente saem para o cliente pelo mesmo caminho da imagem, sem aviso
/// de tipo não aceito.
///
/// Antes, os dois caíam em "tipo não aceito" e só aparecia o aviso na sala. O tipo real continua
/// vindo do conteúdo (a lista fechada de saída do `poph-files`), e o que muda para o cliente é que
/// o arquivo chega: a Evolution recebe um `sendMedia` com o `mimetype` real, entregue como
/// documento (o tocador nativo do WhatsApp exigiria a rota de voz, que não foi provada no ambiente
/// real).
#[tokio::test]
async fn audio_e_video_do_atendente_chegam_ao_whatsapp_sem_aviso() {
    let cenario = Cenario::new("audio-video").await;
    let servidor = cenario.subir().await;
    let url_audio = cenario.publicar("ogg-mx48", AUDIO_OGG);
    let url_video = cenario.publicar("mp4-mx48", VIDEO_MP4);

    let transacao = json!({
        "events": [
            {
                "type": "m.room.message",
                "room_id": SALA,
                "sender": ATENDENTE,
                "event_id": "$audio-mx48:matrix-dev.local",
                "content": {
                    "msgtype": "m.audio",
                    "body": "voice-message.ogg",
                    "filename": "voice-message.ogg",
                    "url": url_audio,
                    "info": { "mimetype": "audio/ogg", "size": AUDIO_OGG.len() }
                }
            },
            {
                "type": "m.room.message",
                "room_id": SALA,
                "sender": ATENDENTE,
                "event_id": "$video-mx48:matrix-dev.local",
                "content": {
                    "msgtype": "m.video",
                    "body": "video.mp4",
                    "filename": "video.mp4",
                    "url": url_video,
                    "info": { "mimetype": "video/mp4", "size": VIDEO_MP4.len() }
                }
            }
        ]
    });

    let status = servidor.transacao("txn-mx48", &transacao).await;
    assert_eq!(status, StatusHttp::OK);

    esperar("o áudio e o vídeo saírem para o cliente", || {
        cenario
            .envios()
            .iter()
            .filter(|send| send.rota == "sendMedia")
            .count()
            == 2
    })
    .await;

    let envios = cenario.envios();
    let midias: Vec<&Send> = envios
        .iter()
        .filter(|send| send.rota == "sendMedia")
        .collect();
    let (audio, video) = (midias[0], midias[1]);

    // Áudio: documento com o MIME real e a legenda do atendente; os bytes chegam inteiros.
    assert_eq!(audio.body["mediatype"], json!("document"));
    assert_eq!(audio.body["mimetype"], json!("audio/ogg"));
    assert_eq!(audio.body["fileName"], json!("voice-message.ogg"));
    assert_eq!(audio.body["number"], json!(TELEFONE));
    assert_eq!(audio.body["caption"], json!("*[Ana Atendente]*"));
    let enviado = decodificar_base64(audio.body["media"].as_str().expect("mídia em base64"));
    assert_eq!(enviado, AUDIO_OGG, "o áudio chega inteiro ao cliente");

    // Vídeo: mesmo caminho, `video/mp4`.
    assert_eq!(video.body["mediatype"], json!("document"));
    assert_eq!(video.body["mimetype"], json!("video/mp4"));
    assert_eq!(video.body["fileName"], json!("video.mp4"));
    assert_eq!(video.body["caption"], json!("*[Ana Atendente]*"));
    let enviado = decodificar_base64(video.body["media"].as_str().expect("mídia em base64"));
    assert_eq!(enviado, VIDEO_MP4, "o vídeo chega inteiro ao cliente");

    // Nenhum aviso de tipo não aceito na sala: os dois arquivos foram.
    assert!(
        cenario.avisos_na_sala().is_empty(),
        "áudio e vídeo não podem virar aviso: {:?}",
        cenario.avisos_na_sala()
    );

    // O download continua saindo em nome do fantasma do agente (regra da MX-44).
    let fantasma = cenario.agent_ghost();
    assert_eq!(
        cenario.downloads(),
        vec![Some(fantasma.clone()), Some(fantasma)],
        "um download por arquivo, os dois disfarçados do fantasma do agente"
    );
    assert_eq!(
        cenario.recusas_de_disfarce(),
        0,
        "nenhum pedido recusado por disfarce fora do namespace do AS"
    );

    // Histórico: duas mensagens `human` com o anexo do tipo real e o blob no disco.
    let mensagens = cenario.messages();
    assert_eq!(mensagens.len(), 2);
    for (indice, (name, mime, tamanho)) in [
        ("voice-message.ogg", "audio/ogg", AUDIO_OGG.len()),
        ("video.mp4", "video/mp4", VIDEO_MP4.len()),
    ]
    .into_iter()
    .enumerate()
    {
        let message = &mensagens[indice];
        assert_eq!(message.origin, Origin::Human);
        assert_eq!(message.author_name.as_deref(), Some(NOME_ATENDENTE));
        let anexos = cenario.anexos(indice);
        assert_eq!(anexos.len(), 1, "cada mensagem leva o seu anexo");
        assert_eq!(anexos[0].name, name);
        assert_eq!(anexos[0].mime, mime);
        assert_eq!(anexos[0].size_bytes, tamanho as u64);
        assert_eq!(anexos[0].extraction_status, "skipped");
        let blob = cenario.pasta_blobs.join(&anexos[0].blob_path);
        assert!(blob.is_file(), "o blob ficou no disco: {}", blob.display());
    }

    // Fila de saída: dois itens `file`, com o MIME real no payload do HR-24.
    let arquivos = cenario.arquivos_na_fila();
    assert_eq!(arquivos.len(), 2);
    for (item, mime) in arquivos.iter().zip(["audio/ogg", "video/mp4"]) {
        assert_eq!(item.tipo, "file");
        let payload: Value = serde_json::from_str(&item.content_json).expect("payload em JSON");
        assert_eq!(payload["mime"], json!(mime));
        assert_eq!(payload["legenda"], json!("**[Ana Atendente]**"));
    }

    // Reentrega do mesmo lote numa transação nova: o evento é o mesmo, então nada é enviado de novo.
    let status = servidor.transacao("txn-mx48-2", &transacao).await;
    assert_eq!(status, StatusHttp::OK);
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        cenario
            .envios()
            .into_iter()
            .filter(|send| send.rota == "sendMedia")
            .count(),
        2,
        "reentrega de áudio e vídeo não manda os arquivos de novo"
    );
    assert_eq!(cenario.messages().len(), 2, "duas mensagens no histórico");
    assert_eq!(cenario.arquivos_na_fila().len(), 2, "dois itens na fila");
    assert_eq!(
        servidor
            .entrega
            .counters()
            .duplicates
            .load(Ordering::SeqCst),
        2,
        "os dois eventos da transação nova foram reconhecidos como duplicados"
    );
}
