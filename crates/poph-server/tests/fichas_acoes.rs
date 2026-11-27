use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use argon2::Argon2;
use argon2::password_hash::{PasswordHasher, SaltString};
use poph_flows::engine::{Context, Estado as EngineState, execute_action};
use poph_flows::ir::Destination;
use poph_flows::{Ir, Limites, RuleEvaluator, compile};
use poph_server::ServerState;
use poph_server::flow_transition::transition_engine;
use poph_store::{
    Execution, NewConversation, NewFlow, NewRecord, RevisionPublication, Store, TransitionExtras,
};
use poph_types::config::GlobalTokenConfig;
use poph_types::{
    ActionActor, AgentId, ExecutionId, FlowId, HarnessConfig, OrgId, RecordId, RevisionId,
};
use rand_core::OsRng;
use reqwest::{Client, Method, StatusCode};
use serde_json::{Value, json};
use tokio::sync::oneshot;

const TOKEN_GLOBAL: &str = "pophg_teste-global-fx30";

const SCRIPT: &str = r#"fluxo(
    id = "agendar",
    nome = "Agendar visita",
    inicio = "coleta",
    gatilho = gatilho(
        quando = "O cliente quer agendar uma visita.",
        nao_quando = "O cliente nao quer agendar.",
        exemplos = ["quero agendar"],
    ),
)

etapa(
    id = "coleta",
    nome = "Coleta",
    atores = ["agente", "atendente"],
    campos = [campo("nome", "Nome", obrigatorio = True)],
    acoes = [
        acao("registrar", "Registrar", ator = "agente", exige = ["nome"], vai_para = "revisao"),
        acao("confirmar", "Confirmar", ator = "atendente", vai_para = "revisao"),
    ],
)

etapa(
    id = "revisao",
    nome = "Revisão",
    atores = ["atendente"],
    acoes = [
        acao("concluir", "Concluir", ator = "atendente", vai_para = fim("concluido", resultado = "Agendado")),
    ],
)
"#;

fn hash_argon2(value: &str) -> String {
    let sal = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(value.as_bytes(), &sal)
        .expect("hash argon2")
        .to_string()
}

fn parse_org(id: &str) -> OrgId {
    id.parse().expect("id de org")
}

fn parse_agent(id: &str) -> AgentId {
    id.parse().expect("id de agente")
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
        let directory =
            std::env::temp_dir().join(format!("poph-acoes-{label}-{}-{stamp}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(directory.join("dados")).expect("pasta de dados");

        let mut config = HarnessConfig::default();
        config.data.directory = directory.join("dados");
        config.server.port = 7483;
        config.admin.tokens_globais = vec![GlobalTokenConfig {
            label: "teste".to_owned(),
            hash: hash_argon2(TOKEN_GLOBAL),
        }];
        config.validar().expect("configuração válida");

        let store = Arc::new(Store::open(&config).expect("abre o banco"));
        let state = ServerState::new(Arc::clone(&store), config).expect("estado do servidor");

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("porta efêmera");
        let endereco = listener.local_addr().expect("endereço local");
        let (desligar, signal) = oneshot::channel();
        tokio::spawn(async move {
            let _ = poph_server::serve_at(listener, state, async {
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
        versao: Option<u64>,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let mut request = self
            .client
            .request(method, format!("{}{path}", self.base))
            .bearer_auth(token);
        if let Some(versao) = versao {
            request = request.header("If-Match", versao.to_string());
        }
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

    async fn create_org(&self, slug: &str) -> String {
        let (status, body) = self
            .send_request(
                Method::POST,
                "/admin/v1/orgs",
                TOKEN_GLOBAL,
                None,
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
                None,
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
                None,
                Some(json!({ "name": format!("Agente {slug}"), "slug": slug })),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "criar agente: {body}");
        body["id"].as_str().expect("id do agente").to_owned()
    }

    fn released_flow(&self, org: &str, agente: &str, slug: &str) -> (FlowId, RevisionId) {
        let (fluxo, revision) = self.published_flow(org, slug);
        self.store
            .flows()
            .release(parse_org(org), parse_agent(agente), fluxo)
            .expect("libera o fluxo ao agente");
        (fluxo, revision)
    }

    fn published_flow(&self, org: &str, slug: &str) -> (FlowId, RevisionId) {
        let org_id = parse_org(org);
        let fluxo = self
            .store
            .flows()
            .create(
                org_id,
                NewFlow {
                    slug: slug.to_owned(),
                    name: format!("Fluxo {slug}"),
                    description: String::new(),
                    rascunho_fonte: SCRIPT.to_owned(),
                },
            )
            .expect("cria o fluxo");
        let compiled = compile(SCRIPT, &Limites::default());
        let ir = match compiled.ir() {
            Ok(ir) => ir,
            Err(errors) => panic!("script de teste inválido: {errors:?}"),
        };
        let revision = self
            .store
            .revisions()
            .publish_revision(
                org_id,
                fluxo.id,
                RevisionPublication {
                    expected: fluxo.version,
                    fonte: SCRIPT.to_owned(),
                    definicao_json: ir.to_json(),
                    schema_versao: Some(ir.schema_versao),
                    resolucoes_json: "{}".to_owned(),
                    ator_tipo: "admin_token".to_owned(),
                    actor_id: None,
                },
            )
            .expect("publica a revisão");
        (fluxo.id, revision.id)
    }

    fn standalone_case(&self, org: &str, agente: &str) -> RecordId {
        self.store
            .cases()
            .create(
                parse_org(org),
                NewRecord {
                    conversation_id: None,
                    agent_id: parse_agent(agente),
                    canal: "whatsapp".to_owned(),
                    peer_externo: Some(format!("cliente-ficticio-{}", uuid::Uuid::now_v7())),
                },
            )
            .expect("cria a Ficha")
            .id
    }

    fn case_with_conversation(&self, org: &str, agente: &str) -> (poph_store::Conversa, RecordId) {
        let conversa = self
            .store
            .conversations()
            .create(
                parse_org(org),
                NewConversation {
                    agent_id: parse_agent(agente),
                    integration_id: None,
                    canal: "whatsapp".to_owned(),
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

    fn case_version(&self, org: &str, ficha: RecordId) -> u64 {
        self.store
            .cases()
            .get(parse_org(org), ficha)
            .expect("lê a Ficha")
            .version
    }

    fn execution(&self, org: &str, run: &str) -> Execution {
        self.store
            .executions()
            .get(parse_org(org), run.parse().expect("id da execução"))
            .expect("lê a execução")
    }

    fn count_executions(&self, org: &str, ficha: RecordId) -> usize {
        self.store
            .executions()
            .from_case(parse_org(org), ficha, 100)
            .expect("lista as execuções")
            .len()
    }

    fn count_events(&self, org: &str, ficha: RecordId) -> usize {
        self.store
            .cases()
            .events(parse_org(org), ficha, None, 500)
            .expect("lista os eventos")
            .len()
    }

    fn agent_acts(&self, org: &str, run: &str, acao: &str, campos: Value, chave: &str) {
        let org_id = parse_org(org);
        let run_id: ExecutionId = run.parse().expect("id da execução");
        let execution = self.execution(org, run);
        let revision = self
            .store
            .revisions()
            .get_by_id(org_id, execution.revisao_id)
            .expect("lê a revisão");
        let ir = Ir::from_json(&revision.definicao_json).expect("IR da revisão");
        let acting_actors = self
            .store
            .executions()
            .stages(org_id, run_id)
            .expect("lê as etapas")
            .iter()
            .rev()
            .find(|etapa| etapa.no_id == execution.no_atual)
            .map(|etapa| etapa.acting_actors.clone())
            .unwrap_or_default();
        let state = EngineState {
            fluxo_id: ir.fluxo.id.clone(),
            status: execution.status,
            no_atual: Destination::Etapa {
                id: execution.no_atual.clone(),
            },
            dados: serde_json::from_str(&execution.dados_json).expect("dados da execução"),
            actors_who_acted: acting_actors,
            resultado: None,
            motivo: None,
            keys: BTreeMap::new(),
        };
        let evaluator = RuleEvaluator::without_functions();
        let case_text = execution.ficha_id.to_string();
        let context = Context::new(&evaluator)
            .with_case(&case_text)
            .with_client("cliente-ficticio");
        let campos = campos.as_object().cloned().unwrap_or_default();
        let resultado = execute_action(
            &ir,
            &state,
            acao,
            &campos,
            ActionActor::Agent,
            Some(chave),
            &context,
        )
        .expect("o agente age na etapa");
        self.store
            .executions()
            .apply_transition(
                org_id,
                run_id,
                execution.version,
                &transition_engine(&resultado),
                chave,
                TransitionExtras {
                    ator_ref: Some("agente-de-teste"),
                    fechada_por: None,
                    close_reason: None,
                },
            )
            .expect("grava a ação do agente");
    }

    async fn start_flow(
        &self,
        org: &str,
        token: &str,
        ficha: RecordId,
        versao: Option<u64>,
        fluxo: FlowId,
        chave: &str,
    ) -> (StatusCode, Value) {
        self.send_request(
            Method::POST,
            &format!("/admin/v1/orgs/{org}/fichas/{ficha}/flow-runs"),
            token,
            versao,
            Some(json!({ "fluxo_id": fluxo.to_string(), "chave_idempotencia": chave })),
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn acao(
        &self,
        org: &str,
        token: &str,
        ficha: RecordId,
        run: &str,
        versao: Option<u64>,
        acao: &str,
        campos: Value,
        chave: &str,
    ) -> (StatusCode, Value) {
        self.send_request(
            Method::POST,
            &format!("/admin/v1/orgs/{org}/fichas/{ficha}/flow-runs/{run}/actions"),
            token,
            versao,
            Some(json!({ "acao": acao, "campos": campos, "chave_idempotencia": chave })),
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn abandon(
        &self,
        org: &str,
        token: &str,
        ficha: RecordId,
        run: &str,
        versao: Option<u64>,
        motivo: &str,
        chave: &str,
    ) -> (StatusCode, Value) {
        self.send_request(
            Method::POST,
            &format!("/admin/v1/orgs/{org}/fichas/{ficha}/flow-runs/{run}/abandon"),
            token,
            versao,
            Some(json!({ "motivo": motivo, "chave_idempotencia": chave })),
        )
        .await
    }

    async fn close(
        &self,
        org: &str,
        token: &str,
        ficha: RecordId,
        versao: Option<u64>,
    ) -> (StatusCode, Value) {
        self.send_request(
            Method::POST,
            &format!("/admin/v1/orgs/{org}/fichas/{ficha}/close"),
            token,
            versao,
            None,
        )
        .await
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

fn code(body: &Value) -> &str {
    body["code"].as_str().unwrap_or("")
}

struct Base {
    org: String,
    token: String,
    agente: String,
    fluxo: FlowId,
}

async fn base(c: &Scenario, slug: &str) -> Base {
    let org = c.create_org(slug).await;
    let token = c.create_token(&org).await;
    let agente = c
        .create_agent(&org, &token, &format!("agente-{slug}"))
        .await;
    let (fluxo, _revision) = c.released_flow(
        &org,
        &agente,
        &format!("agendar_{}", slug.replace('-', "_")),
    );
    Base {
        org,
        token,
        agente,
        fluxo,
    }
}

#[tokio::test]
async fn start_requires_if_match_key_and_correct_version() {
    let c = Scenario::new("inicio-exige").await;
    let b = base(&c, "inicio-exige").await;
    let ficha = c.standalone_case(&b.org, &b.agente);
    let versao = c.case_version(&b.org, ficha);

    let (status, body) = c
        .start_flow(&b.org, &b.token, ficha, None, b.fluxo, "chave-1")
        .await;
    assert_eq!(status, StatusCode::PRECONDITION_REQUIRED, "{body}");
    assert_eq!(code(&body), "precondition_required");

    let (status, body) = c
        .start_flow(&b.org, &b.token, ficha, Some(versao), b.fluxo, "   ")
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(code(&body), "validation_error");

    let (status, body) = c
        .start_flow(
            &b.org,
            &b.token,
            ficha,
            Some(versao + 7),
            b.fluxo,
            "chave-1",
        )
        .await;
    assert_eq!(status, StatusCode::PRECONDITION_FAILED, "{body}");
    assert_eq!(code(&body), "version_conflict");
    assert_eq!(c.count_executions(&b.org, ficha), 0, "nada foi aberto");
}

#[tokio::test]
async fn start_on_closed_case_returns_409() {
    let c = Scenario::new("ficha-fechada").await;
    let b = base(&c, "ficha-fechada").await;
    let ficha = c.standalone_case(&b.org, &b.agente);
    c.store
        .cases()
        .close_without_conversation(parse_org(&b.org), ficha, 1, "encerrada no teste")
        .expect("fecha a Ficha");
    let versao = c.case_version(&b.org, ficha);

    let (status, body) = c
        .start_flow(&b.org, &b.token, ficha, Some(versao), b.fluxo, "chave-1")
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(code(&body), "ficha_encerrada");
    assert_eq!(c.count_executions(&b.org, ficha), 0);
}

#[tokio::test]
async fn unreleased_flow_returns_403_and_opens_no_execution() {
    let c = Scenario::new("nao-liberado").await;
    let b = base(&c, "nao-liberado").await;
    let (other, _) = c.published_flow(&b.org, "sem_permissao");
    let ficha = c.standalone_case(&b.org, &b.agente);
    let versao = c.case_version(&b.org, ficha);

    let (status, body) = c
        .start_flow(&b.org, &b.token, ficha, Some(versao), other, "chave-1")
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(code(&body), "fluxo_nao_liberado");
    assert_eq!(c.count_executions(&b.org, ficha), 0);
}

#[tokio::test]
async fn repeating_start_key_does_not_duplicate_even_after_finishing() {
    let c = Scenario::new("inicio-repetido").await;
    let b = base(&c, "inicio-repetido").await;
    let ficha = c.standalone_case(&b.org, &b.agente);
    let versao = c.case_version(&b.org, ficha);

    let (status, first) = c
        .start_flow(
            &b.org,
            &b.token,
            ficha,
            Some(versao),
            b.fluxo,
            "abrir-visita",
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{first}");
    let run = first["id"].as_str().expect("id da execução").to_owned();
    let events = c.count_events(&b.org, ficha);

    let (status, repeated) = c
        .start_flow(
            &b.org,
            &b.token,
            ficha,
            Some(versao),
            b.fluxo,
            "abrir-visita",
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{repeated}");
    assert_eq!(repeated["id"], first["id"]);
    assert_eq!(repeated["repetida"], json!(true));
    assert_eq!(c.count_executions(&b.org, ficha), 1);
    assert_eq!(c.count_events(&b.org, ficha), events, "sem evento novo");

    let (status, _) = c
        .abandon(
            &b.org,
            &b.token,
            ficha,
            &run,
            Some(c.execution(&b.org, &run).version),
            "cliente desistiu",
            "parar-visita",
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (status, after) = c
        .start_flow(
            &b.org,
            &b.token,
            ficha,
            Some(versao),
            b.fluxo,
            "abrir-visita",
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{after}");
    assert_eq!(after["id"], first["id"]);
    assert_eq!(
        c.count_executions(&b.org, ficha),
        1,
        "nenhuma execução nova"
    );
}

#[tokio::test]
async fn attendant_confirms_agent_completes_and_attendant_concludes() {
    let c = Scenario::new("criterio-auto").await;
    let b = base(&c, "criterio-auto").await;
    let ficha = c.standalone_case(&b.org, &b.agente);
    let versao = c.case_version(&b.org, ficha);
    let (_, opened) = c
        .start_flow(&b.org, &b.token, ficha, Some(versao), b.fluxo, "abrir")
        .await;
    let run = opened["id"].as_str().expect("id").to_owned();

    let (status, partial) = c
        .acao(
            &b.org,
            &b.token,
            ficha,
            &run,
            Some(c.execution(&b.org, &run).version),
            "confirmar",
            json!({}),
            "atendente-confirma",
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{partial}");
    assert_eq!(partial["no_atual"], json!("coleta"), "{partial}");
    assert_eq!(
        partial["etapas"][0]["status"],
        json!("parcial"),
        "{partial}"
    );

    c.agent_acts(
        &b.org,
        &run,
        "registrar",
        json!({ "nome": "Ana Teste" }),
        "agente-registra",
    );
    assert_eq!(c.execution(&b.org, &run).no_atual, "revisao");

    let (status, completed) = c
        .acao(
            &b.org,
            &b.token,
            ficha,
            &run,
            Some(c.execution(&b.org, &run).version),
            "concluir",
            json!({}),
            "atendente-conclui",
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{completed}");
    assert_eq!(completed["status"], json!("concluida"), "{completed}");
    assert_eq!(completed["resultado"], json!("Agendado"));
}

#[tokio::test]
async fn attendant_agent_race_returns_412_and_writes_nothing() {
    let c = Scenario::new("corrida").await;
    let b = base(&c, "corrida").await;
    let ficha = c.standalone_case(&b.org, &b.agente);
    let versao = c.case_version(&b.org, ficha);
    let (_, opened) = c
        .start_flow(&b.org, &b.token, ficha, Some(versao), b.fluxo, "abrir")
        .await;
    let run = opened["id"].as_str().expect("id").to_owned();
    let read_version = c.execution(&b.org, &run).version;

    c.agent_acts(
        &b.org,
        &run,
        "registrar",
        json!({ "nome": "Bia Teste" }),
        "agente-primeiro",
    );
    let events = c.count_events(&b.org, ficha);

    let (status, body) = c
        .acao(
            &b.org,
            &b.token,
            ficha,
            &run,
            Some(read_version),
            "confirmar",
            json!({}),
            "atendente-atrasado",
        )
        .await;
    assert_eq!(status, StatusCode::PRECONDITION_FAILED, "{body}");
    assert_eq!(code(&body), "version_conflict");
    assert_eq!(
        c.count_events(&b.org, ficha),
        events,
        "a recusa não gravou evento"
    );
    assert_eq!(c.execution(&b.org, &run).no_atual, "coleta");
}

#[tokio::test]
async fn repeating_action_key_returns_same_without_new_event() {
    let c = Scenario::new("acao-repetida").await;
    let b = base(&c, "acao-repetida").await;
    let ficha = c.standalone_case(&b.org, &b.agente);
    let versao = c.case_version(&b.org, ficha);
    let (_, opened) = c
        .start_flow(&b.org, &b.token, ficha, Some(versao), b.fluxo, "abrir")
        .await;
    let run = opened["id"].as_str().expect("id").to_owned();
    let read_version = c.execution(&b.org, &run).version;

    let (status, first) = c
        .acao(
            &b.org,
            &b.token,
            ficha,
            &run,
            Some(read_version),
            "confirmar",
            json!({}),
            "confirma-uma-vez",
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{first}");
    let events = c.count_events(&b.org, ficha);
    let version_after = c.execution(&b.org, &run).version;

    let (status, repeated) = c
        .acao(
            &b.org,
            &b.token,
            ficha,
            &run,
            Some(read_version),
            "confirmar",
            json!({}),
            "confirma-uma-vez",
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{repeated}");
    assert_eq!(repeated["repetida"], json!(true));
    assert_eq!(c.count_events(&b.org, ficha), events, "sem evento novo");
    assert_eq!(
        c.execution(&b.org, &run).version,
        version_after,
        "versão não andou"
    );
}

#[tokio::test]
async fn repeating_open_after_closing_case_returns_same() {
    let c = Scenario::new("inicio-depois-fechar").await;
    let b = base(&c, "inicio-depois-fechar").await;
    let ficha = c.standalone_case(&b.org, &b.agente);
    let versao = c.case_version(&b.org, ficha);

    let (status, first) = c
        .start_flow(
            &b.org,
            &b.token,
            ficha,
            Some(versao),
            b.fluxo,
            "abrir-visita",
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{first}");

    let (status, body) = c.close(&b.org, &b.token, ficha, Some(versao)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let events = c.count_events(&b.org, ficha);

    let (status, repeated) = c
        .start_flow(
            &b.org,
            &b.token,
            ficha,
            Some(versao),
            b.fluxo,
            "abrir-visita",
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{repeated}");
    assert_eq!(repeated["id"], first["id"]);
    assert_eq!(repeated["repetida"], json!(true));
    assert_eq!(c.count_executions(&b.org, ficha), 1);
    assert_eq!(c.count_events(&b.org, ficha), events, "sem evento novo");

    let (status, body) = c
        .start_flow(
            &b.org,
            &b.token,
            ficha,
            Some(versao),
            b.fluxo,
            "outra-abertura",
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
}

#[tokio::test]
async fn agent_only_action_called_by_attendant_returns_409() {
    let c = Scenario::new("acao-restrita").await;
    let b = base(&c, "acao-restrita").await;
    let ficha = c.standalone_case(&b.org, &b.agente);
    let versao = c.case_version(&b.org, ficha);
    let (_, opened) = c
        .start_flow(&b.org, &b.token, ficha, Some(versao), b.fluxo, "abrir")
        .await;
    let run = opened["id"].as_str().expect("id").to_owned();
    let read_version = c.execution(&b.org, &run).version;

    let (status, body) = c
        .acao(
            &b.org,
            &b.token,
            ficha,
            &run,
            Some(read_version),
            "registrar",
            json!({ "nome": "Carla Teste" }),
            "atendente-tenta-agente",
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(code(&body), "acao_invalida");
    assert_eq!(
        c.execution(&b.org, &run).version,
        read_version,
        "nada mudou"
    );
}

#[tokio::test]
async fn abandon_stores_reason_and_repeating_key_does_not_repeat_event() {
    let c = Scenario::new("abandono").await;
    let b = base(&c, "abandono").await;
    let ficha = c.standalone_case(&b.org, &b.agente);
    let versao = c.case_version(&b.org, ficha);
    let (_, opened) = c
        .start_flow(&b.org, &b.token, ficha, Some(versao), b.fluxo, "abrir")
        .await;
    let run = opened["id"].as_str().expect("id").to_owned();
    let read_version = c.execution(&b.org, &run).version;

    let (status, body) = c
        .abandon(
            &b.org,
            &b.token,
            ficha,
            &run,
            Some(read_version),
            "",
            "sem-motivo",
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");

    let (status, body) = c
        .abandon(
            &b.org,
            &b.token,
            ficha,
            &run,
            Some(read_version),
            "cliente desistiu",
            "parar",
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], json!("abandonada"));
    assert_eq!(body["motivo_fim"], json!("cliente desistiu"));
    let events = c.count_events(&b.org, ficha);

    let (status, repeated) = c
        .abandon(
            &b.org,
            &b.token,
            ficha,
            &run,
            Some(read_version),
            "cliente desistiu",
            "parar",
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{repeated}");
    assert_eq!(repeated["repetida"], json!(true));
    assert_eq!(c.count_events(&b.org, ficha), events, "sem evento novo");
}

#[tokio::test]
async fn close_closes_conversation_case_and_abandons_execution() {
    let c = Scenario::new("close-conversa").await;
    let org = c.create_org("close-conversa").await;
    let token = c.create_token(&org).await;
    let agente = c.create_agent(&org, &token, "agente-close").await;
    let (fluxo, _) = c.released_flow(&org, &agente, "agendar_close");
    let (conversa, ficha) = c.case_with_conversation(&org, &agente);
    let versao = c.case_version(&org, ficha);
    let (status, opened) = c
        .start_flow(&org, &token, ficha, Some(versao), fluxo, "abrir")
        .await;
    assert_eq!(status, StatusCode::CREATED, "{opened}");
    let run = opened["id"].as_str().expect("id").to_owned();

    let (status, body) = c.close(&org, &token, ficha, Some(versao + 3)).await;
    assert_eq!(status, StatusCode::PRECONDITION_FAILED, "{body}");
    assert_eq!(c.execution(&org, &run).status.as_text(), "ativa");

    let (status, body) = c.close(&org, &token, ficha, Some(versao)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], json!("closed"));

    let conversation_after = c
        .store
        .conversations()
        .get(parse_org(&org), conversa.id)
        .expect("lê a conversa");
    assert_eq!(conversation_after.status.as_text(), "closed");
    let execution = c.execution(&org, &run);
    assert_eq!(execution.status.as_text(), "abandonada");
    assert_eq!(
        execution.motivo_fim.as_deref(),
        Some("atendimento_encerrado")
    );

    let events = c.count_events(&org, ficha);
    let (status, body) = c.close(&org, &token, ficha, Some(versao)).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(c.count_events(&org, ficha), events);
}

#[tokio::test]
async fn close_without_conversation_closes_only_case() {
    let c = Scenario::new("close-avulsa").await;
    let b = base(&c, "close-avulsa").await;
    let ficha = c.standalone_case(&b.org, &b.agente);
    let versao = c.case_version(&b.org, ficha);

    let (status, body) = c.close(&b.org, &b.token, ficha, Some(versao + 3)).await;
    assert_eq!(status, StatusCode::PRECONDITION_FAILED, "{body}");

    let (status, body) = c.close(&b.org, &b.token, ficha, Some(versao)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], json!("closed"));
    assert_eq!(
        body["fechada_motivo"],
        json!("encerrada pelo administrador")
    );

    let (status, body) = c.close(&b.org, &b.token, ficha, Some(versao)).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
}

#[tokio::test]
async fn action_routes_isolated_by_org_and_case() {
    let c = Scenario::new("isolamento").await;
    let a = base(&c, "iso-a").await;
    let b = base(&c, "iso-b").await;
    let case_a = c.standalone_case(&a.org, &a.agente);
    let case_a2 = c.standalone_case(&a.org, &a.agente);
    let version_a = c.case_version(&a.org, case_a);
    let (_, opened) = c
        .start_flow(&a.org, &a.token, case_a, Some(version_a), a.fluxo, "abrir")
        .await;
    let run_a = opened["id"].as_str().expect("id").to_owned();
    let run_version = c.execution(&a.org, &run_a).version;

    let (status, body) = c
        .start_flow(
            &b.org,
            &b.token,
            case_a,
            Some(version_a),
            a.fluxo,
            "outra-org",
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    let (status, body) = c.close(&b.org, &b.token, case_a, Some(version_a)).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    let (status, body) = c
        .acao(
            &b.org,
            &b.token,
            case_a,
            &run_a,
            Some(run_version),
            "confirmar",
            json!({}),
            "outra-org-acao",
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    let (status, body) = c
        .abandon(
            &b.org,
            &b.token,
            case_a,
            &run_a,
            Some(run_version),
            "x",
            "outra-org-ab",
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");

    let (status, body) = c
        .start_flow(
            &a.org,
            &b.token,
            case_a,
            Some(version_a),
            a.fluxo,
            "caminho-alheio",
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");

    let version_a2 = c.case_version(&a.org, case_a2);
    let (status, body) = c
        .acao(
            &a.org,
            &a.token,
            case_a2,
            &run_a,
            Some(run_version),
            "confirmar",
            json!({}),
            "outra-ficha",
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(c.execution(&a.org, &run_a).version, run_version);
    let _ = version_a2;
}
