use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use argon2::Argon2;
use argon2::password_hash::{PasswordHasher, SaltString};
use poph_server::ServerState;
use poph_store::{
    Conversa, ExecutionStart, ExecutionState, FlowTransition, NewConversation, NewRecord, Store,
    StoredExecution, TransitionEvent, TransitionExtras,
};
use poph_types::config::GlobalTokenConfig;
use poph_types::{
    ActionActor, AgentId, EventActor, ExecutionStatus, FlowId, HarnessConfig, OrgId,
    RecordEventKind, RecordId, RevisionId,
};
use rand_core::OsRng;
use reqwest::{Client, Method, StatusCode};
use serde_json::{Value, json};
use tokio::sync::oneshot;

const TOKEN_GLOBAL: &str = "pophg_teste-global-fx29";

fn hash_argon2(value: &str) -> String {
    let sal = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(value.as_bytes(), &sal)
        .expect("hash argon2")
        .to_string()
}

struct Scenario {
    base: String,
    client: Client,
    directory: PathBuf,
    store: Arc<Store>,
    desligar: Option<oneshot::Sender<()>>,
}

impl Scenario {
    async fn new(label: &str) -> Self {
        let stamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let directory = std::env::temp_dir().join(format!(
            "poph-fichas-{label}-{}-{stamp}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(directory.join("dados")).expect("pasta de dados");

        let mut config = HarnessConfig::default();
        config.data.directory = directory.join("dados");
        config.server.port = 7482;
        config.admin.tokens_globais = vec![GlobalTokenConfig {
            label: "teste".to_owned(),
            hash: hash_argon2(TOKEN_GLOBAL),
        }];
        config.validar().expect("configuração válida");

        let store = Arc::new(Store::open(&config).expect("abre o banco"));
        let estado = ServerState::new(Arc::clone(&store), config).expect("estado do servidor");

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("porta efêmera");
        let endereco = listener.local_addr().expect("endereço local");
        let (desligar, signal) = oneshot::channel();
        tokio::spawn(async move {
            let _ = poph_server::serve_at(listener, estado, async {
                let _ = signal.await;
            })
            .await;
        });

        let scenario = Self {
            base: format!("http://{endereco}"),
            client: Client::builder()
                .timeout(Duration::from_secs(20))
                .build()
                .expect("cliente HTTP"),
            directory,
            store,
            desligar: Some(desligar),
        };
        scenario.wait_until_up().await;
        scenario
    }

    async fn wait_until_up(&self) {
        let start = Instant::now();
        loop {
            match self
                .client
                .get(format!("{}/healthz", self.base))
                .send()
                .await
            {
                Ok(response) if response.status().is_success() => return,
                _ if start.elapsed() < Duration::from_secs(10) => {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                other => panic!("servidor não subiu: {other:?}"),
            }
        }
    }

    async fn send_request(
        &self,
        method: Method,
        path: &str,
        token: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let mut request = self
            .client
            .request(method, format!("{}{path}", self.base))
            .bearer_auth(token);
        if let Some(value) = body {
            request = request.json(&value);
        }
        let response = request.send().await.expect("pedido HTTP");
        let status = response.status();
        let texto = response.text().await.unwrap_or_default();
        let json = if texto.trim().is_empty() {
            Value::Null
        } else {
            serde_json::from_str(&texto).unwrap_or(Value::String(texto))
        };
        (status, json)
    }

    async fn get(&self, path: &str, token: &str) -> (StatusCode, Value) {
        self.send_request(Method::GET, path, token, None).await
    }

    async fn create_org(&self, slug: &str) -> String {
        let (status, body) = self
            .send_request(
                Method::POST,
                "/admin/v1/orgs",
                TOKEN_GLOBAL,
                Some(json!({ "name": format!("Org {slug}"), "slug": slug })),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "criar org: {body}");
        body["id"].as_str().expect("id da org").to_owned()
    }

    async fn create_token(&self, org: &str) -> String {
        let (status, body) = self
            .send_request(
                Method::POST,
                &format!("/admin/v1/orgs/{org}/tokens"),
                TOKEN_GLOBAL,
                Some(json!({ "label": "token de teste" })),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "criar token: {body}");
        body["value"].as_str().expect("valor do token").to_owned()
    }

    async fn create_agent(&self, org: &str, token: &str, slug: &str) -> String {
        let (status, body) = self
            .send_request(
                Method::POST,
                &format!("/admin/v1/orgs/{org}/agents"),
                token,
                Some(json!({ "name": format!("Agente {slug}"), "slug": slug })),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "criar agente: {body}");
        body["id"].as_str().expect("id do agente").to_owned()
    }

    fn connection(&self) -> rusqlite::Connection {
        let conn =
            rusqlite::Connection::open(self.store.harness_path()).expect("abre o banco do teste");
        conn.busy_timeout(Duration::from_secs(10))
            .expect("tempo de espera do banco");
        conn
    }

    fn published_flow(&self, org: &str, slug: &str) -> (FlowId, RevisionId) {
        let conn = self.connection();
        let fluxo = uuid::Uuid::now_v7().to_string();
        let revision = uuid::Uuid::now_v7().to_string();
        conn.execute(
            "INSERT INTO fluxos (id, org_id, slug, nome, descricao, status, rascunho_fonte,
                                 revisao_atual, version)
             VALUES (?1, ?2, ?3, ?4, '', 'active', '', 1, 1)",
            rusqlite::params![fluxo, org, slug, format!("Fluxo {slug}")],
        )
        .expect("grava o fluxo");
        conn.execute(
            "INSERT INTO fluxo_revisoes (id, org_id, fluxo_id, numero, fonte, fonte_sha256,
                                        definicao_json, schema_versao, publicada_em, publicada_por,
                                        resolucoes_json)
             VALUES (?1, ?2, ?3, 1, 'fluxo()', 'sha', '{}', 1, 1, 'admin_token', '{}')",
            rusqlite::params![revision, org, fluxo],
        )
        .expect("grava a revisão");
        (
            fluxo.parse().expect("id do fluxo"),
            revision.parse().expect("id da revisão"),
        )
    }

    fn liberar(&self, org: &str, agente: &str, fluxo: FlowId) {
        self.store
            .flows()
            .release(parse_org(org), parse_agent(agente), fluxo)
            .expect("libera o fluxo ao agente");
    }

    fn standalone_case(&self, org: &str, agente: &str, canal: &str) -> RecordId {
        self.store
            .cases()
            .create(
                parse_org(org),
                NewRecord {
                    conversation_id: None,
                    agent_id: parse_agent(agente),
                    canal: canal.to_owned(),
                    peer_externo: Some(format!("cliente-ficticio-{}", uuid::Uuid::now_v7())),
                },
            )
            .expect("cria a Ficha")
            .id
    }

    fn conversa(&self, org: &str, agente: &str, canal: &str) -> (Conversa, RecordId) {
        let conversa = self
            .store
            .conversations()
            .create(
                parse_org(org),
                NewConversation {
                    agent_id: parse_agent(agente),
                    integration_id: None,
                    canal: canal.to_owned(),
                    peer_externo: Some(format!("cliente-ficticio-{}", uuid::Uuid::now_v7())),
                    target_department: None,
                },
            )
            .expect("cria a conversa");
        let ficha = self
            .store
            .cases()
            .by_conversation(parse_org(org), conversa.id)
            .expect("consulta a Ficha")
            .expect("a conversa tem Ficha")
            .id;
        (conversa, ficha)
    }

    fn close_conversation(&self, org: &str, conversa: &Conversa) {
        self.store
            .conversations()
            .close_audited(
                parse_org(org),
                conversa.id,
                conversa.version,
                "teste-fx29".to_owned(),
            )
            .expect("fecha a conversa");
    }

    fn start_execution(
        &self,
        org: &str,
        agente: &str,
        ficha: RecordId,
        fluxo: FlowId,
        revision: RevisionId,
    ) -> StoredExecution {
        self.liberar(org, agente, fluxo);
        self.store
            .executions()
            .start_execution(
                parse_org(org),
                ExecutionStart {
                    chave_idempotencia: None,
                    case: ficha,
                    flow: fluxo,
                    revision,
                    actor: EventActor::Agent,
                    ator_ref: Some("agente-de-teste"),
                    jev_decision: None,
                    initial_state: ExecutionState {
                        status: ExecutionStatus::Active,
                        no_atual: "coleta".to_owned(),
                        dados_json: "{}".to_owned(),
                        acting_actors: Vec::new(),
                        result: None,
                        reason: None,
                    },
                    inbound_effects: Vec::new(),
                },
            )
            .expect("abre a execução")
    }

    fn answer_name(&self, org: &str, stored: &StoredExecution, nome: &str) {
        let transition = FlowTransition {
            new_state: ExecutionState {
                status: ExecutionStatus::Active,
                no_atual: "coleta".to_owned(),
                dados_json: json!({ "coleta": { "nome": nome } }).to_string(),
                acting_actors: vec![ActionActor::Agent],
                result: None,
                reason: None,
            },
            events: vec![TransitionEvent {
                tipo: RecordEventKind::FieldsFilled,
                ator_tipo: EventActor::Agent,
                payload_json: json!({ "etapa": "coleta", "campos": ["nome"] }).to_string(),
            }],
            effects: Vec::new(),
            closes_service: false,
            repeated: false,
            rule_error: false,
        };
        self.store
            .executions()
            .apply_transition(
                parse_org(org),
                stored.execution.id,
                stored.execution.version,
                &transition,
                &format!("coleta-{}", uuid::Uuid::now_v7()),
                TransitionExtras {
                    ator_ref: None,
                    fechada_por: None,
                    close_reason: None,
                },
            )
            .expect("registra o campo respondido");
    }

    async fn paginate(&self, org: &str, token: &str, filters: &str, limit: u32) -> Vec<String> {
        let mut ids = Vec::new();
        let mut cursor: Option<String> = None;
        for _ in 0..100 {
            let mut path = format!("/admin/v1/orgs/{org}/fichas?limit={limit}{filters}");
            if let Some(current) = &cursor {
                path.push_str(&format!("&cursor={current}"));
            }
            let (status, body) = self.get(&path, token).await;
            assert_eq!(status, StatusCode::OK, "listar: {body}");
            let items = body["items"].as_array().expect("items");
            assert!(
                items.len() <= limit as usize,
                "página maior que o limite: {} itens",
                items.len()
            );
            ids.extend(items.iter().map(item_id));
            cursor = body["next_cursor"].as_str().map(str::to_owned);
            if cursor.is_none() {
                return ids;
            }
        }
        panic!("paginação não terminou em 100 páginas");
    }
}

impl Drop for Scenario {
    fn drop(&mut self) {
        if let Some(desligar) = self.desligar.take() {
            let _ = desligar.send(());
        }
        let _ = fs::remove_dir_all(&self.directory);
    }
}

fn parse_org(id: &str) -> OrgId {
    id.parse().expect("id de org")
}

fn parse_agent(id: &str) -> AgentId {
    id.parse().expect("id de agente")
}

fn item_id(item: &Value) -> String {
    item["id"].as_str().expect("id no item").to_owned()
}

fn code(body: &Value) -> &str {
    body["code"].as_str().unwrap_or("")
}

fn id_set(ids: &[String]) -> BTreeSet<String> {
    ids.iter().cloned().collect()
}

#[tokio::test]
async fn isolated_and_combined_filters() {
    let c = Scenario::new("filtros").await;
    let org = c.create_org("filtros-a").await;
    let token = c.create_token(&org).await;
    let x = c.create_agent(&org, &token, "agente-x").await;
    let y = c.create_agent(&org, &token, "agente-y").await;

    let f1 = c.standalone_case(&org, &x, "whatsapp");
    let (conversation2, f2) = c.conversa(&org, &x, "console");
    c.close_conversation(&org, &conversation2);
    let f3 = c.standalone_case(&org, &y, "whatsapp");
    let (conversation4, f4) = c.conversa(&org, &y, "whatsapp");
    c.close_conversation(&org, &conversation4);

    let org_b = c.create_org("filtros-b").await;
    let token_b = c.create_token(&org_b).await;
    let xb = c.create_agent(&org_b, &token_b, "agente-b").await;
    c.standalone_case(&org_b, &xb, "whatsapp");

    let (f1, f2, f3, f4) = (
        f1.to_string(),
        f2.to_string(),
        f3.to_string(),
        f4.to_string(),
    );
    let cases: Vec<(&str, String, Vec<String>)> = vec![
        (
            "sem filtro",
            String::new(),
            vec![f1.clone(), f2.clone(), f3.clone(), f4.clone()],
        ),
        (
            "status=open",
            "&status=open".to_owned(),
            vec![f1.clone(), f3.clone()],
        ),
        (
            "status=closed",
            "&status=closed".to_owned(),
            vec![f2.clone(), f4.clone()],
        ),
        (
            "agente X",
            format!("&agente={x}"),
            vec![f1.clone(), f2.clone()],
        ),
        (
            "agente Y",
            format!("&agente={y}"),
            vec![f3.clone(), f4.clone()],
        ),
        (
            "canal=whatsapp",
            "&canal=whatsapp".to_owned(),
            vec![f1.clone(), f3.clone(), f4.clone()],
        ),
        (
            "canal=console",
            "&canal=console".to_owned(),
            vec![f2.clone()],
        ),
        (
            "status=closed + agente Y + canal=whatsapp",
            format!("&status=closed&agente={y}&canal=whatsapp"),
            vec![f4.clone()],
        ),
        (
            "status=open + canal=console",
            "&status=open&canal=console".to_owned(),
            vec![],
        ),
        (
            "desde=0",
            "&desde=0".to_owned(),
            vec![f1.clone(), f2.clone(), f3.clone(), f4.clone()],
        ),
        (
            "desde no futuro",
            "&desde=9223372036854775807".to_owned(),
            vec![],
        ),
    ];
    for (nome, filters, expected) in &cases {
        let (status, body) = c
            .get(
                &format!("/admin/v1/orgs/{org}/fichas?limit=200{filters}"),
                &token,
            )
            .await;
        assert_eq!(status, StatusCode::OK, "caso {nome}: {body}");
        let seen: Vec<String> = body["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(item_id)
            .collect();
        assert_eq!(
            id_set(&seen),
            id_set(expected),
            "caso `{nome}` devolveu conjunto diferente"
        );
        assert_eq!(seen.len(), expected.len(), "caso `{nome}` repetiu item");
    }

    for (filter, campo) in [
        ("status=aberta", "status"),
        ("agente=nao-e-um-id", ""),
        ("canal=", "canal"),
        ("desde=ontem", "desde"),
    ] {
        let (status, body) = c
            .get(&format!("/admin/v1/orgs/{org}/fichas?{filter}"), &token)
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{filter}: {body}");
        assert!(
            matches!(code(&body), "validation_error" | "invalid_id"),
            "{filter} tem de sair com código de validação: {body}"
        );
        if !campo.is_empty() {
            assert_eq!(body["details"]["field"], campo, "{filter}: {body}");
        }
    }
}

#[tokio::test]
async fn cursor_pagination_does_not_repeat_or_skip() {
    let c = Scenario::new("paginacao").await;
    let org = c.create_org("paginacao-a").await;
    let token = c.create_token(&org).await;
    let x = c.create_agent(&org, &token, "agente-x").await;
    let y = c.create_agent(&org, &token, "agente-y").await;
    for _ in 0..5 {
        c.standalone_case(&org, &x, "whatsapp");
    }
    for _ in 0..2 {
        c.standalone_case(&org, &y, "whatsapp");
    }

    let (status, body) = c
        .get(&format!("/admin/v1/orgs/{org}/fichas?limit=200"), &token)
        .await;
    assert_eq!(status, StatusCode::OK);
    let entire: Vec<String> = body["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(item_id)
        .collect();
    assert_eq!(entire.len(), 7);

    let paged = c.paginate(&org, &token, "", 2).await;
    assert_eq!(paged, entire, "páginas de 2 diferem da lista inteira");
    assert_eq!(id_set(&paged).len(), 7, "item repetido entre páginas");

    let (_, only_x) = c
        .get(
            &format!("/admin/v1/orgs/{org}/fichas?limit=200&agente={x}"),
            &token,
        )
        .await;
    let only_x: Vec<String> = only_x["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(item_id)
        .collect();
    assert_eq!(only_x.len(), 5);
    let paged_x = c.paginate(&org, &token, &format!("&agente={x}"), 2).await;
    assert_eq!(paged_x, only_x, "paginação com filtro diferente");

    let (_, full_page) = c
        .get(&format!("/admin/v1/orgs/{org}/fichas?limit=7"), &token)
        .await;
    assert_eq!(full_page["items"].as_array().unwrap().len(), 7);
    let cursor = full_page["next_cursor"]
        .as_str()
        .expect("cursor da página cheia");
    let (_, after) = c
        .get(
            &format!("/admin/v1/orgs/{org}/fichas?limit=7&cursor={cursor}"),
            &token,
        )
        .await;
    assert!(after["items"].as_array().unwrap().is_empty());
    assert!(after["next_cursor"].is_null());

    let (status, error) = c
        .get(
            &format!("/admin/v1/orgs/{org}/fichas?cursor=nao-e-cursor"),
            &token,
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{error}");
    assert_eq!(code(&error), "invalid_request");
    assert_eq!(error["details"]["field"], "cursor");
}

#[tokio::test]
async fn case_from_another_org_returns_404_on_read_routes() {
    let c = Scenario::new("isolamento").await;
    let org_a = c.create_org("isolamento-a").await;
    let token_a = c.create_token(&org_a).await;
    let agent_a = c.create_agent(&org_a, &token_a, "agente-a").await;
    let case_a = c.standalone_case(&org_a, &agent_a, "whatsapp");

    let org_b = c.create_org("isolamento-b").await;
    let token_b = c.create_token(&org_b).await;
    let agent_b = c.create_agent(&org_b, &token_b, "agente-b").await;
    let case_b = c.standalone_case(&org_b, &agent_b, "whatsapp");

    let case_b_text = case_b.to_string();
    let missing_id = uuid::Uuid::now_v7().to_string();

    for path in [
        format!("/admin/v1/orgs/{org_a}/fichas/{case_b_text}"),
        format!("/admin/v1/orgs/{org_a}/fichas/{case_b_text}/events"),
        format!("/admin/v1/orgs/{org_a}/fichas/{missing_id}"),
        format!("/admin/v1/orgs/{org_a}/fichas/{missing_id}/events"),
    ] {
        let (status, body) = c.get(&path, &token_a).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}: {body}");
        assert_eq!(code(&body), "not_found", "{path}: {body}");
    }
    let (status, body) = c
        .get(
            &format!("/admin/v1/orgs/{org_a}/fichas/{case_b_text}"),
            TOKEN_GLOBAL,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");

    let (status, list) = c
        .get(&format!("/admin/v1/orgs/{org_a}/fichas"), &token_a)
        .await;
    assert_eq!(status, StatusCode::OK);
    let ids: Vec<String> = list["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(item_id)
        .collect();
    assert_eq!(ids, vec![case_a.to_string()]);

    for path in [
        format!("/admin/v1/orgs/{org_b}/fichas"),
        format!("/admin/v1/orgs/{org_b}/fichas/{case_b_text}"),
        format!("/admin/v1/orgs/{org_b}/fichas/{case_b_text}/events"),
    ] {
        let (status, body) = c.get(&path, &token_a).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{path}: {body}");
    }

    let (status, _) = c
        .get(
            &format!("/admin/v1/orgs/{org_b}/fichas/{case_b_text}"),
            &token_b,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn events_in_seq_order_with_pagination() {
    let c = Scenario::new("eventos").await;
    let org = c.create_org("eventos-a").await;
    let token = c.create_token(&org).await;
    let agente = c.create_agent(&org, &token, "agente-x").await;
    let (fluxo, revision) = c.published_flow(&org, "agendar-visita");
    let ficha = c.standalone_case(&org, &agente, "whatsapp");
    let stored = c.start_execution(&org, &agente, ficha, fluxo, revision);
    c.answer_name(&org, &stored, "Ana Teste");

    let path = format!("/admin/v1/orgs/{org}/fichas/{ficha}/events");
    let (status, all_events) = c.get(&format!("{path}?limit=200"), &token).await;
    assert_eq!(status, StatusCode::OK, "{all_events}");
    let items = all_events["items"].as_array().unwrap();
    let seqs: Vec<i64> = items.iter().map(|i| i["seq"].as_i64().unwrap()).collect();
    assert!(
        seqs.len() >= 3,
        "esperava abertura, início e campo: {seqs:?}"
    );
    let expected_seqs: Vec<i64> = (1..=seqs.len() as i64).collect();
    assert_eq!(seqs, expected_seqs, "seq fora de ordem ou com buraco");
    let kinds: Vec<&str> = items.iter().map(|i| i["tipo"].as_str().unwrap()).collect();
    assert_eq!(kinds.first(), Some(&"ficha_aberta"));
    assert!(kinds.contains(&"fluxo_iniciado"), "{kinds:?}");
    assert_eq!(kinds.last(), Some(&"campos_preenchidos"), "{kinds:?}");

    let mut paginados: Vec<i64> = Vec::new();
    let mut cursor: Option<String> = None;
    for _ in 0..100 {
        let mut current = format!("{path}?limit=1");
        if let Some(value) = &cursor {
            current.push_str(&format!("&cursor={value}"));
        }
        let (status, page) = c.get(&current, &token).await;
        assert_eq!(status, StatusCode::OK, "{page}");
        assert!(page["items"].as_array().unwrap().len() <= 1);
        paginados.extend(
            page["items"]
                .as_array()
                .unwrap()
                .iter()
                .map(|i| i["seq"].as_i64().unwrap()),
        );
        cursor = page["next_cursor"].as_str().map(str::to_owned);
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(
        paginados, seqs,
        "paginação por seq diferente da lista inteira"
    );

    let (_, after_second) = c.get(&format!("{path}?cursor=2&limit=200"), &token).await;
    let from_third: Vec<i64> = after_second["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["seq"].as_i64().unwrap())
        .collect();
    assert_eq!(from_third, seqs[2..].to_vec());

    let (status, error) = c.get(&format!("{path}?cursor=abc"), &token).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{error}");
    assert_eq!(code(&error), "invalid_request");
}

#[tokio::test]
async fn execution_with_stages_and_fields_appears_inside_case() {
    let c = Scenario::new("w6").await;
    let org = c.create_org("w6-a").await;
    let token = c.create_token(&org).await;
    let agente = c.create_agent(&org, &token, "agente-x").await;
    let (fluxo, revision) = c.published_flow(&org, "agendar-visita");
    let ficha = c.standalone_case(&org, &agente, "whatsapp");
    let stored = c.start_execution(&org, &agente, ficha, fluxo, revision);
    c.answer_name(&org, &stored, "Ana Teste");

    let (status, body) = c
        .get(&format!("/admin/v1/orgs/{org}/fichas/{ficha}"), &token)
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["id"], ficha.to_string());
    assert_eq!(body["status"], "open");

    let executions = body["execucoes"].as_array().expect("execucoes na Ficha");
    assert_eq!(executions.len(), 1, "{body}");
    let execution = &executions[0];
    assert_eq!(execution["id"], stored.execution.id.to_string());
    assert_eq!(execution["fluxo_id"], fluxo.to_string());
    assert_eq!(execution["revisao_id"], revision.to_string());
    assert_eq!(execution["status"], "ativa");
    assert_eq!(execution["no_atual"], "coleta");

    let stages = execution["etapas"].as_array().expect("etapas na execução");
    assert_eq!(stages.len(), 1, "{execution}");
    assert_eq!(stages[0]["no_id"], "coleta");
    assert_eq!(stages[0]["ordem"], 1);
    assert_eq!(stages[0]["status"], "parcial");
    assert_eq!(
        stages[0]["dados"]["nome"], "Ana Teste",
        "o campo respondido tem de estar na etapa dentro da Ficha: {stages:?}"
    );
    assert_eq!(stages[0]["atores_que_agiram"], json!(["agente"]));

    let released = body["permissoes"]["fluxos_liberados_ao_agente"]
        .as_array()
        .expect("permissões");
    assert_eq!(released.len(), 1, "{body}");
    assert_eq!(released[0]["slug"], "agendar-visita");
    assert_eq!(released[0]["id"], fluxo.to_string());

    assert!(!body.to_string().contains("mensagem_ao_cliente"));
}

#[tokio::test]
async fn closed_case_remains_readable() {
    let c = Scenario::new("fechada").await;
    let org = c.create_org("fechada-a").await;
    let token = c.create_token(&org).await;
    let agente = c.create_agent(&org, &token, "agente-x").await;
    let (fluxo, revision) = c.published_flow(&org, "agendar-visita");
    let (conversa, ficha) = c.conversa(&org, &agente, "whatsapp");
    let stored = c.start_execution(&org, &agente, ficha, fluxo, revision);
    c.answer_name(&org, &stored, "Ana Teste");
    c.close_conversation(&org, &conversa);

    let (status, body) = c
        .get(&format!("/admin/v1/orgs/{org}/fichas/{ficha}"), &token)
        .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "Ficha fechada tem de ser legível: {body}"
    );
    assert_eq!(body["status"], "closed");
    assert!(body["fechada_em"].is_i64(), "{body}");
    assert_eq!(
        body["fechada_por"], "admin",
        "fechar pela conversa grava o autor: {body}"
    );
    assert!(body["fechada_motivo"].is_string(), "{body}");
    let execution = &body["execucoes"][0];
    assert_eq!(execution["status"], "abandonada", "{execution}");
    assert_eq!(execution["motivo_fim"], "atendimento_encerrado");
    assert_eq!(execution["etapas"][0]["dados"]["nome"], "Ana Teste");

    let (status, events) = c
        .get(
            &format!("/admin/v1/orgs/{org}/fichas/{ficha}/events?limit=200"),
            &token,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{events}");
    let kinds: Vec<&str> = events["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["tipo"].as_str().unwrap())
        .collect();
    assert!(kinds.contains(&"execucao_abandonada"), "{kinds:?}");
    assert_eq!(kinds.last(), Some(&"ficha_fechada"), "{kinds:?}");

    let (_, closed_cases) = c
        .get(
            &format!("/admin/v1/orgs/{org}/fichas?status=closed"),
            &token,
        )
        .await;
    let ids: Vec<String> = closed_cases["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(item_id)
        .collect();
    assert_eq!(ids, vec![ficha.to_string()]);
}
