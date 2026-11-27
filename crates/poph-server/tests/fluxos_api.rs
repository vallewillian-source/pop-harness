use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use argon2::Argon2;
use argon2::password_hash::{PasswordHasher, SaltString};
use poph_server::ServerState;
use poph_store::{NewFlow, Page, Store};
use poph_types::config::GlobalTokenConfig;
use poph_types::{
    AgentId, EventActor, ExecutionStatus, FlowId, HarnessConfig, OrgId, RecordId, RevisionId,
};
use rand_core::OsRng;
use reqwest::{Client, Method, StatusCode};
use serde_json::{Value, json};
use tokio::sync::oneshot;

const TOKEN_GLOBAL: &str = "pophg_teste-global-fx27";

const SCRIPT_V1: &str = r#"fluxo(
    id = "agendar_visita",
    nome = "Agendar visita",
    descricao = "Marca uma visita técnica no endereço do cliente.",
    inicio = "coleta",
    gatilho = gatilho(
        quando = "O cliente quer marcar, remarcar ou cancelar uma visita técnica.",
        nao_quando = "Só quer saber preço ou prazo, sem pedir visita.",
        exemplos = ["queria agendar uma visita", "pode vir aqui amanhã?"],
    ),
)

etapa(
    id = "coleta",
    nome = "Coletar dados",
    instrucao = "Pergunte o endereço e o período preferido. Uma pergunta por vez.",
    campos = [
        campo("endereco", "Endereço completo", tipo = "texto", obrigatorio = True),
        campo("periodo", "Período", tipo = "opcao", opcoes = ["manhã", "tarde"], obrigatorio = True),
    ],
    acoes = [
        acao("confirmar", "Dados completos", exige = ["endereco", "periodo"], vai_para = "confirmacao"),
        acao("desistir", "Cliente desistiu", vai_para = fim("desistencia", resultado = "Cliente desistiu")),
    ],
)

etapa(
    id = "confirmacao",
    nome = "Confirmar visita",
    instrucao = "Repita o endereço e o período combinados e peça a confirmação do cliente.",
    acoes = [
        acao("ok", "Visita combinada", vai_para = fim("concluida", resultado = "Visita agendada")),
    ],
)
"#;

const SCRIPT_V2: &str = r#"fluxo(
    id = "agendar_visita",
    nome = "Agendar visita",
    descricao = "Marca uma visita técnica no endereço do cliente.",
    inicio = "coleta",
    gatilho = gatilho(
        quando = "O cliente quer marcar, remarcar ou cancelar uma visita técnica.",
        nao_quando = "Só quer saber preço ou prazo, sem pedir visita.",
        exemplos = ["queria agendar uma visita", "pode vir aqui amanhã?"],
    ),
)

etapa(
    id = "coleta",
    nome = "Coletar dados",
    instrucao = "Pergunte o endereço e o período preferido. Uma pergunta por vez.",
    campos = [
        campo("endereco", "Endereço completo", tipo = "texto", obrigatorio = True),
        campo("periodo", "Período", tipo = "opcao", opcoes = ["manhã", "tarde"], obrigatorio = True),
    ],
    acoes = [
        acao("confirmar", "Dados completos", exige = ["endereco", "periodo"], vai_para = "confirmacao"),
        acao("desistir", "Cliente desistiu", vai_para = fim("desistencia", resultado = "Cliente desistiu")),
    ],
)

etapa(
    id = "confirmacao",
    nome = "Confirmar visita",
    instrucao = "Confirme o endereço e o período e pergunte se o cliente está no local.",
    acoes = [
        acao("ok", "Visita combinada", vai_para = fim("concluida", resultado = "Visita agendada")),
    ],
)
"#;

const SCRIPT_TORTO: &str = r#"fluxo(id = "agendar_visita", nome = "Agendar visita", descricao = "Marca uma visita técnica no endereço do cliente.", inicio = "coleta", gatilho = gatilho(quando = "O cliente quer marcar, remarcar ou cancelar uma visita técnica.", nao_quando = "Só quer saber preço ou prazo, sem pedir visita.", exemplos = ["queria agendar uma visita", "pode vir aqui amanhã?"]))

etapa(id = "coleta", nome = "Coletar dados", instrucao = "Pergunte o endereço e o período preferido. Uma pergunta por vez.", campos = [campo("endereco", "Endereço completo", tipo = "texto", obrigatorio = True), campo("periodo", "Período", tipo = "opcao", opcoes = ["manhã", "tarde"], obrigatorio = True)], acoes = [acao("confirmar", "Dados completos", exige = ["endereco", "periodo"], vai_para = "confirmacao"), acao("desistir", "Cliente desistiu", vai_para = fim("desistencia", resultado = "Cliente desistiu"))])

etapa(id = "confirmacao", nome = "Confirmar visita", instrucao = "Repita o endereço e o período combinados e peça a confirmação do cliente.", acoes = [acao("ok", "Visita combinada", vai_para = fim("concluida", resultado = "Visita agendada"))])
"#;

const DELIVERY_SCRIPT: &str = r#"fluxo(
    id = "confirmar_entrega",
    nome = "Confirmar entrega",
    descricao = "Confirma o horário da entrega com o cliente.",
    inicio = "confirma",
    gatilho = gatilho(
        quando = "O cliente quer confirmar o horário de uma entrega.",
        nao_quando = "O cliente quer remarcar a entrega para outro dia.",
        exemplos = ["a entrega sai hoje?", "que horas chega?"],
    ),
)

etapa(
    id = "confirma",
    nome = "Confirmar",
    instrucao = "Confirme o horário previsto e pergunte se alguém recebe.",
    acoes = [
        acao("ok", "Confirmado", vai_para = fim("confirmada", resultado = "Entrega confirmada")),
    ],
)
"#;

const SCRIPT_WITH_ERROR: &str = "fluxo(\n    id = \"quebrado\",\n    nome = ,\n)\n";

fn hash_argon2(value: &str) -> String {
    let sal = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(value.as_bytes(), &sal)
        .expect("hash argon2")
        .to_string()
}

struct Scenario {
    base: String,
    customer: Client,
    dir: PathBuf,
    store: Arc<Store>,
    desligar: Option<oneshot::Sender<()>>,
}

impl Scenario {
    async fn fresh(label: &str) -> Self {
        Self::fresh_with(label, |_| {}).await
    }

    async fn fresh_with(label: &str, adjustment: impl FnOnce(&mut HarnessConfig)) -> Self {
        let stamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!(
            "poph-server-{label}-{}-{stamp}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("dados")).expect("pasta de dados");

        let mut config = HarnessConfig::default();
        config.data.directory = dir.join("dados");
        config.server.port = 7481;
        config.admin.tokens_globais = vec![GlobalTokenConfig {
            label: "teste".to_owned(),
            hash: hash_argon2(TOKEN_GLOBAL),
        }];
        adjustment(&mut config);
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
            customer: Client::builder()
                .timeout(Duration::from_secs(20))
                .build()
                .expect("cliente HTTP"),
            dir,
            store,
            desligar: Some(desligar),
        };
        scenario.wait_for_boot().await;
        scenario
    }

    async fn wait_for_boot(&self) {
        let start = Instant::now();
        loop {
            match self
                .customer
                .get(format!("{}/healthz", self.base))
                .send()
                .await
            {
                Ok(reply) if reply.status().is_success() => return,
                _ if start.elapsed() < Duration::from_secs(10) => {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                another => panic!("servidor não subiu: {another:?}"),
            }
        }
    }

    fn store(&self) -> &Store {
        &self.store
    }

    async fn ask(
        &self,
        method: Method,
        path: &str,
        token: &str,
        body: Option<Value>,
        if_match: Option<u64>,
    ) -> (StatusCode, Value) {
        let mut request = self
            .customer
            .request(method, format!("{}{path}", self.base))
            .bearer_auth(token);
        if let Some(versao) = if_match {
            request = request.header("If-Match", versao.to_string());
        }
        if let Some(value) = body {
            request = request.json(&value);
        }
        let reply = request.send().await.expect("pedido HTTP");
        let status = reply.status();
        let texto = reply.text().await.unwrap_or_default();
        let json = if texto.trim().is_empty() {
            Value::Null
        } else {
            serde_json::from_str(&texto).unwrap_or(Value::String(texto))
        };
        (status, json)
    }

    async fn create_org(&self, nome: &str, slug: &str) -> String {
        let (status, body) = self
            .ask(
                Method::POST,
                "/admin/v1/orgs",
                TOKEN_GLOBAL,
                Some(json!({ "name": nome, "slug": slug })),
                None,
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "criar org: {body}");
        body["id"].as_str().expect("id da org").to_owned()
    }

    async fn create_token(&self, org_id: &str, label: &str) -> String {
        let (status, body) = self
            .ask(
                Method::POST,
                &format!("/admin/v1/orgs/{org_id}/tokens"),
                TOKEN_GLOBAL,
                Some(json!({ "label": label })),
                None,
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "criar token: {body}");
        body["value"].as_str().expect("valor do token").to_owned()
    }

    async fn create_agent(&self, org_id: &str, token: &str, slug: &str) -> String {
        let (status, body) = self
            .ask(
                Method::POST,
                &format!("/admin/v1/orgs/{org_id}/agents"),
                token,
                Some(json!({ "name": format!("Agente {slug}"), "slug": slug })),
                None,
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "criar agente: {body}");
        body["id"].as_str().expect("id do agente").to_owned()
    }

    fn create_case(&self, org_id: &str, agent_id: &str) -> RecordId {
        let org: OrgId = org_id.parse().expect("org válida");
        let agente: AgentId = agent_id.parse().expect("agente válido");
        self.store()
            .cases()
            .create(
                org,
                poph_store::NewRecord {
                    conversation_id: None,
                    agent_id: agente,
                    canal: "console".to_owned(),
                    peer_externo: Some(format!("cliente-ficticio-{}", uuid::Uuid::now_v7())),
                },
            )
            .expect("cria a Ficha")
            .id
    }

    fn open_execution(
        &self,
        org_id: &str,
        agent_id: &str,
        ficha: RecordId,
        fluxo_id: &str,
        revisao_id: &str,
    ) {
        let org: OrgId = org_id.parse().expect("org válida");
        let agente: AgentId = agent_id.parse().expect("agente válido");
        let fluxo: FlowId = fluxo_id.parse().expect("fluxo válido");
        let revision: RevisionId = revisao_id.parse().expect("revisão válida");
        self.store()
            .flows()
            .release(org, agente, fluxo)
            .expect("libera o fluxo ao agente");
        self.store()
            .executions()
            .start_execution(
                org,
                poph_store::ExecutionStart {
                    chave_idempotencia: None,
                    case: ficha,
                    flow: fluxo,
                    revision,
                    actor: EventActor::Agent,
                    ator_ref: Some("atendente-de-teste"),
                    jev_decision: None,
                    initial_state: poph_store::ExecutionState {
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
            .expect("abre a execução");
    }

    fn create_flow(&self, org_id: &str, slug: &str) -> FlowId {
        let org: OrgId = org_id.parse().expect("org válida");
        self.store()
            .flows()
            .create(
                org,
                NewFlow {
                    slug: slug.to_owned(),
                    name: format!("Fluxo {slug}"),
                    description: String::new(),
                    rascunho_fonte: String::new(),
                },
            )
            .expect("cria o fluxo")
            .id
    }

    fn audit(&self, org_id: &str) -> Vec<poph_store::AuditEntry> {
        let org: OrgId = org_id.parse().expect("org válida");
        self.store()
            .audit()
            .list(org, Page::default())
            .expect("lê a auditoria")
    }
}

impl Drop for Scenario {
    fn drop(&mut self) {
        if let Some(desligar) = self.desligar.take() {
            let _ = desligar.send(());
        }
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn code(body: &Value) -> &str {
    body["code"].as_str().unwrap_or("")
}

fn first_diagnostic(body: &Value) -> &Value {
    &body["details"]["diagnostics"][0]
}

#[tokio::test]
async fn validate_format_create_and_isolate() {
    let c = Scenario::fresh("fluxos-fx27-validar").await;
    let oa = c.create_org("Org A", "fx27-org-a").await;
    let ob = c.create_org("Org B", "fx27-org-b").await;
    let ta = c.create_token(&oa, "a").await;
    let tb = c.create_token(&ob, "b").await;

    let (s, _) = c
        .ask(
            Method::GET,
            &format!("/admin/v1/orgs/{oa}/flows"),
            "",
            None,
            None,
        )
        .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    let (s, body) = c
        .ask(
            Method::GET,
            &format!("/admin/v1/orgs/{oa}/flows"),
            &tb,
            None,
            None,
        )
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(code(&body), "forbidden");

    let (s, body) = c
        .ask(
            Method::POST,
            &format!("/admin/v1/orgs/{oa}/flows/validate"),
            &ta,
            Some(json!({ "source": SCRIPT_V1 })),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert_eq!(body["valid"], json!(true), "{body}");
    assert_eq!(body["diagnostics"], json!([]), "{body}");
    assert_eq!(body["ir"]["schema_versao"], json!(1));
    assert_eq!(body["ir"]["fluxo"]["id"], json!("agendar_visita"));
    assert_eq!(body["ir"]["inicio"], json!("coleta"));

    let (s, body) = c
        .ask(
            Method::POST,
            &format!("/admin/v1/orgs/{oa}/flows/validate"),
            &ta,
            Some(json!({ "source": SCRIPT_WITH_ERROR })),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert_eq!(body["valid"], json!(false), "{body}");
    assert_eq!(body["ir"], Value::Null);
    let diagnostic = &body["diagnostics"][0];
    assert_eq!(diagnostic["codigo"], json!("FLX01"), "{body}");
    assert_eq!(diagnostic["severidade"], json!("erro"));
    assert_eq!(diagnostic["linha"], json!(3), "{body}");
    assert!(
        diagnostic["coluna"].as_u64().unwrap_or(0) >= 1,
        "coluna do diagnóstico: {diagnostic}"
    );

    let (s, body) = c
        .ask(
            Method::POST,
            &format!("/admin/v1/orgs/{oa}/flows/format"),
            &ta,
            Some(json!({ "source": SCRIPT_V1 })),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{body}");
    let canonical = body["source"].as_str().expect("texto canônico").to_owned();
    assert!(canonical.contains("etapa("), "{canonical}");
    let (s, body) = c
        .ask(
            Method::POST,
            &format!("/admin/v1/orgs/{oa}/flows/format"),
            &ta,
            Some(json!({ "source": SCRIPT_TORTO })),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert_ne!(body["source"], json!(SCRIPT_TORTO), "formatar não fez nada");
    assert_eq!(
        body["source"],
        json!(canonical),
        "formas canônicas diferentes"
    );
    let (s, body) = c
        .ask(
            Method::POST,
            &format!("/admin/v1/orgs/{oa}/flows/format"),
            &ta,
            Some(json!({ "source": canonical })),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert_eq!(
        body["source"],
        json!(canonical),
        "formatar duas vezes difere"
    );
    let (s, body) = c
        .ask(
            Method::POST,
            &format!("/admin/v1/orgs/{oa}/flows/format"),
            &ta,
            Some(json!({ "source": SCRIPT_WITH_ERROR })),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(code(&body), "fluxo_invalido");
    assert_eq!(first_diagnostic(&body)["linha"], json!(3));

    let (s, body) = c
        .ask(
            Method::POST,
            &format!("/admin/v1/orgs/{oa}/flows"),
            &ta,
            Some(json!({ "source": SCRIPT_WITH_ERROR })),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(code(&body), "fluxo_invalido");
    let (s, body) = c
        .ask(
            Method::GET,
            &format!("/admin/v1/orgs/{oa}/flows"),
            &ta,
            None,
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(body["items"], json!([]), "{body}");

    let (s, body) = c
        .ask(
            Method::POST,
            &format!("/admin/v1/orgs/{oa}/flows"),
            &ta,
            Some(json!({ "source": SCRIPT_V1 })),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{body}");
    let fluxo = body["id"].as_str().expect("id do fluxo").to_owned();
    assert_eq!(body["slug"], json!("agendar_visita"));
    assert_eq!(body["name"], json!("Agendar visita"));
    assert_eq!(body["current_revision"], Value::Null);
    assert_eq!(body["version"], json!(1));
    assert_eq!(body["draft_source"], json!(SCRIPT_V1));

    let (s, body) = c
        .ask(
            Method::GET,
            &format!("/admin/v1/orgs/{oa}/flows"),
            &ta,
            None,
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(body["items"].as_array().expect("items").len(), 1);
    assert_eq!(body["items"][0]["id"], json!(fluxo));
    let (s, body) = c
        .ask(
            Method::GET,
            &format!("/admin/v1/orgs/{oa}/flows/{fluxo}"),
            &ta,
            None,
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert_eq!(body["revision"], Value::Null);

    let route = format!("/admin/v1/orgs/{oa}/flows/{fluxo}");
    let (s, body) = c
        .ask(
            Method::PATCH,
            &route,
            &ta,
            Some(json!({ "name": "Outro nome" })),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::PRECONDITION_REQUIRED, "{body}");
    let (s, body) = c
        .ask(
            Method::PATCH,
            &route,
            &ta,
            Some(json!({ "name": "Outro nome" })),
            Some(9),
        )
        .await;
    assert_eq!(s, StatusCode::PRECONDITION_FAILED, "{body}");
    assert_eq!(code(&body), "version_conflict");
    let (s, body) = c
        .ask(
            Method::PATCH,
            &route,
            &ta,
            Some(json!({ "name": "Visita técnica", "source": SCRIPT_V2 })),
            Some(1),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert_eq!(body["name"], json!("Visita técnica"));
    assert_eq!(body["version"], json!(2));
    assert_eq!(body["draft_source"], json!(SCRIPT_V2));

    let (s, _) = c
        .ask(
            Method::GET,
            &format!("/admin/v1/orgs/{oa}/flows/{fluxo}"),
            &tb,
            None,
            None,
        )
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, body) = c
        .ask(
            Method::GET,
            &format!("/admin/v1/orgs/{ob}/flows/{fluxo}"),
            &tb,
            None,
            None,
        )
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND, "{body}");
}

#[tokio::test]
async fn publish_refusals_immutable_revisions_and_diff() {
    let c = Scenario::fresh("fluxos-fx27-publicar").await;
    let oa = c.create_org("Org A", "fx27-pub-a").await;
    let ob = c.create_org("Org B", "fx27-pub-b").await;
    let ta = c.create_token(&oa, "a").await;
    let tb = c.create_token(&ob, "b").await;
    let agente = c.create_agent(&oa, &ta, "atendente").await;

    let (s, body) = c
        .ask(
            Method::POST,
            &format!("/admin/v1/orgs/{oa}/flows"),
            &ta,
            Some(json!({ "source": SCRIPT_V1 })),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{body}");
    let fluxo = body["id"].as_str().expect("id do fluxo").to_owned();
    let route = format!("/admin/v1/orgs/{oa}/flows/{fluxo}");
    let revisions = format!("{route}/revisions");

    let (s, body) = c
        .ask(
            Method::POST,
            &format!("{route}/publish"),
            &ta,
            Some(Value::Null),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::PRECONDITION_REQUIRED, "{body}");
    assert_eq!(code(&body), "precondition_required");
    let (s, body) = c
        .ask(
            Method::POST,
            &format!("{route}/publish"),
            &ta,
            Some(Value::Null),
            Some(7),
        )
        .await;
    assert_eq!(s, StatusCode::PRECONDITION_FAILED, "{body}");
    assert_eq!(code(&body), "version_conflict");
    let (s, body) = c.ask(Method::GET, &revisions, &ta, None, None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(body["items"], json!([]), "{body}");

    let (s, body) = c
        .ask(
            Method::POST,
            &format!("{route}/publish"),
            &ta,
            Some(json!({ "source": SCRIPT_WITH_ERROR })),
            Some(1),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(code(&body), "fluxo_invalido");
    assert_eq!(first_diagnostic(&body)["linha"], json!(3));
    let (s, body) = c.ask(Method::GET, &revisions, &ta, None, None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(body["items"], json!([]), "revisão órfã: {body}");
    let (s, body) = c.ask(Method::GET, &route, &ta, None, None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(body["version"], json!(1), "{body}");

    let (s, body) = c
        .ask(
            Method::POST,
            &format!("{route}/publish"),
            &ta,
            Some(Value::Null),
            Some(1),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{body}");
    assert_eq!(body["revision"]["number"], json!(1));
    assert_eq!(body["revision"]["schema_version"], json!(1));
    assert_eq!(body["version"], json!(2));
    let revision_one_id = body["revision"]["id"]
        .as_str()
        .expect("id da revisão")
        .to_owned();

    let (s, one_before) = c
        .ask(Method::GET, &format!("{revisions}/1"), &ta, None, None)
        .await;
    assert_eq!(s, StatusCode::OK, "{one_before}");
    assert_eq!(one_before["number"], json!(1));
    assert_eq!(one_before["source"], json!(SCRIPT_V1));
    assert_eq!(
        one_before["definition"]["fluxo"]["id"],
        json!("agendar_visita")
    );
    assert!(one_before["source_sha256"].as_str().unwrap_or("").len() == 64);

    let (s, body) = c
        .ask(
            Method::POST,
            &format!("{route}/publish"),
            &ta,
            Some(json!({ "source": SCRIPT_V2 })),
            Some(2),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{body}");
    assert_eq!(body["revision"]["number"], json!(2));
    assert_eq!(body["version"], json!(3));

    let (s, one_after) = c
        .ask(Method::GET, &format!("{revisions}/1"), &ta, None, None)
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(one_after, one_before, "a revisão 1 mudou depois da 2");
    assert_eq!(one_after["id"], json!(revision_one_id));
    let (s, list) = c.ask(Method::GET, &revisions, &ta, None, None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(list["items"].as_array().expect("items").len(), 2);
    assert_eq!(list["items"][0]["number"], json!(1));
    assert_eq!(list["items"][1]["number"], json!(2));
    assert_eq!(list["next_cursor"], Value::Null);

    let (s, body) = c
        .ask(Method::GET, &format!("{revisions}/9"), &ta, None, None)
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND, "{body}");

    let (s, body) = c
        .ask(
            Method::GET,
            &format!("{route}/diff?de=1&para=2"),
            &ta,
            None,
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert_eq!(body["from"]["revision"], json!(1));
    assert_eq!(body["to"]["revision"], json!(2));
    assert_eq!(body["from"]["active_runs"], json!(0));
    assert_eq!(body["to"]["active_runs"], json!(0));
    let changes = body["changes"].as_array().expect("changes");
    assert!(!changes.is_empty(), "diff vazio: {body}");
    let instruction = changes
        .iter()
        .find(|m| m["tipo"] == json!("instrucao_mudou"))
        .unwrap_or_else(|| panic!("sem `instrucao_mudou` no diff: {body}"));
    assert_eq!(instruction["alvo"], json!("etapa:confirmacao"));
    assert!(
        instruction["texto"]
            .as_str()
            .unwrap_or("")
            .contains("confirmação"),
        "frase do diff: {instruction}"
    );

    let (s, body) = c
        .ask(
            Method::GET,
            &format!("{route}/preview?etapa=coleta"),
            &ta,
            None,
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert_eq!(body["stage"], json!("coleta"));
    assert_eq!(body["revision"], json!(2));
    let prompt = body["prompt"].as_str().expect("bloco do prompt");
    assert!(prompt.contains("Pergunte o endereço"), "{prompt}");
    assert!(prompt.contains("Coletar dados"), "{prompt}");
    let (s, body) = c
        .ask(Method::GET, &format!("{route}/preview"), &ta, None, None)
        .await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert_eq!(body["stage"], json!("coleta"));
    let (s, body) = c
        .ask(
            Method::GET,
            &format!("{route}/preview?etapa=nao_existe"),
            &ta,
            None,
            None,
        )
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND, "{body}");

    let ficha = c.create_case(&oa, &agente);
    let (s, detail) = c.ask(Method::GET, &route, &ta, None, None).await;
    assert_eq!(s, StatusCode::OK, "{detail}");
    assert_eq!(detail["revision"]["number"], json!(2));
    c.open_execution(&oa, &agente, ficha, &fluxo, &revision_one_id);
    let (s, body) = c
        .ask(
            Method::GET,
            &format!("{route}/diff?de=1&para=2"),
            &ta,
            None,
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert_eq!(body["from"]["active_runs"], json!(1), "{body}");
    assert_eq!(body["to"]["active_runs"], json!(0), "{body}");

    let (s, body) = c
        .ask(
            Method::GET,
            &format!("/admin/v1/orgs/{ob}/flows/{fluxo}/revisions"),
            &tb,
            None,
            None,
        )
        .await;
    assert!(!s.is_success(), "{body}");
}

#[tokio::test]
async fn delete_with_active_execution_requires_force_and_audits() {
    let c = Scenario::fresh("fluxos-fx27-excluir").await;
    let oa = c.create_org("Org A", "fx27-del-a").await;
    let ta = c.create_token(&oa, "a").await;
    let agente = c.create_agent(&oa, &ta, "atendente").await;

    let (s, body) = c
        .ask(
            Method::POST,
            &format!("/admin/v1/orgs/{oa}/flows"),
            &ta,
            Some(json!({ "source": SCRIPT_V1 })),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{body}");
    let fluxo = body["id"].as_str().expect("id do fluxo").to_owned();
    let route = format!("/admin/v1/orgs/{oa}/flows/{fluxo}");

    let (s, body) = c
        .ask(
            Method::POST,
            &format!("{route}/publish"),
            &ta,
            Some(Value::Null),
            Some(1),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{body}");
    let revision = body["revision"]["id"]
        .as_str()
        .expect("id da revisão")
        .to_owned();

    let ficha = c.create_case(&oa, &agente);
    c.open_execution(&oa, &agente, ficha, &fluxo, &revision);

    let (s, body) = c.ask(Method::DELETE, &route, &ta, None, None).await;
    assert_eq!(s, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["details"]["active_runs"], json!(1), "{body}");
    let (s, _) = c.ask(Method::GET, &route, &ta, None, None).await;
    assert_eq!(s, StatusCode::OK);

    let (s, body) = c
        .ask(
            Method::DELETE,
            &format!("{route}?force=talvez"),
            &ta,
            None,
            None,
        )
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{body}");

    let (s, body) = c
        .ask(
            Method::DELETE,
            &format!("{route}?force=true"),
            &ta,
            None,
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert_eq!(body["deleted"], json!(true));
    assert_eq!(body["abandoned_runs"], json!(1), "{body}");
    let (s, body) = c.ask(Method::GET, &route, &ta, None, None).await;
    assert_eq!(s, StatusCode::NOT_FOUND, "{body}");
    let deletions: Vec<_> = c
        .audit(&oa)
        .into_iter()
        .filter(|registro| registro.acao == "flow_delete")
        .collect();
    assert_eq!(deletions.len(), 1, "a exclusão tem de ser auditada");
    assert_eq!(deletions[0].entity, "flow");
    assert_eq!(deletions[0].entity_id.as_deref(), Some(fluxo.as_str()));

    let publications: Vec<_> = c
        .audit(&oa)
        .into_iter()
        .filter(|registro| registro.acao == "flow_publish")
        .collect();
    assert_eq!(publications.len(), 1);
}

#[tokio::test]
async fn per_org_flow_cap_applies_on_create() {
    let c = Scenario::fresh_with("fluxos-fx27-teto", |config| {
        config.flows.max_flows_per_org = 1;
    })
    .await;
    let oa = c.create_org("Org A", "fx27-teto-a").await;
    let ta = c.create_token(&oa, "a").await;

    c.create_flow(&oa, "fluxo_existente");

    let (s, body) = c
        .ask(
            Method::POST,
            &format!("/admin/v1/orgs/{oa}/flows"),
            &ta,
            Some(json!({ "source": SCRIPT_V1 })),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "{body}");
    assert_eq!(code(&body), "teto_de_fluxos_por_org");
    assert_eq!(body["details"]["flows"], json!(1));
    assert_eq!(body["details"]["limit"], json!(1));
}

#[tokio::test]
async fn create_with_first_publication() {
    let c = Scenario::fresh("fluxos-fx27-criar-pub").await;
    let oa = c.create_org("Org A", "fx27-criar-a").await;
    let ta = c.create_token(&oa, "a").await;

    let (s, body) = c
        .ask(
            Method::POST,
            &format!("/admin/v1/orgs/{oa}/fluxos"),
            &ta,
            None,
            None,
        )
        .await;
    assert_eq!(
        s,
        StatusCode::NOT_FOUND,
        "rota de fluxo não é a de Ficha: {body}"
    );

    let (s, body) = c
        .ask(
            Method::POST,
            &format!("/admin/v1/orgs/{oa}/flows"),
            &ta,
            Some(json!({ "source": DELIVERY_SCRIPT, "publish": true })),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{body}");
    let fluxo = body["id"].as_str().expect("id do fluxo").to_owned();
    assert_eq!(body["slug"], json!("confirmar_entrega"));
    assert_eq!(body["current_revision"], json!(1));
    assert_eq!(body["version"], json!(2));
    assert_eq!(body["revision"]["number"], json!(1));

    let (s, revision) = c
        .ask(
            Method::GET,
            &format!("/admin/v1/orgs/{oa}/flows/{fluxo}/revisions/1"),
            &ta,
            None,
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{revision}");
    assert_eq!(revision["source"], json!(DELIVERY_SCRIPT));
    assert_eq!(
        revision["definition"]["fluxo"]["id"],
        json!("confirmar_entrega")
    );
    assert_eq!(revision["published_by"], json!("admin_token"));
}
