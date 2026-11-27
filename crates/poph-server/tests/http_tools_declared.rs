use std::sync::Arc;
use std::time::Duration;

use argon2::Argon2;
use argon2::password_hash::{PasswordHasher, SaltString};
use axum::extract::State;
use axum::http::HeaderMap;
use axum::{Json, Router, routing::post};
use poph_agent::actors::{ActorOptions, Actors};
use poph_agent::config_hot::{ConfigHot, HotConfigOptions};
use poph_agent::entry_output::{FlowOptions, Fluxo};
use poph_agent::limites::{LimitOptions, Limiter};
use poph_agent::metrics::{MetricsMemory, MetricsTurn};
use poph_agent::turno::{HttpToolRegistry, TurnEngine, TurnOptions, TurnProcessor};
use poph_core::llm::{FakeProvider, FinishReason, LlmEvent, Message, Script, ToolCall};
use poph_server::ServerState;
use poph_store::{NewAgent, NewOrg, Page, Store};
use poph_types::config::GlobalTokenConfig;
use poph_types::{AgentId, DomainError, HarnessConfig, OrgId, Secret};
use rand_core::OsRng;
use reqwest::{Client, Response, StatusCode};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::sync::mpsc;

const TOOL: &str = "consultar_pedido";
const DESCRIPTION: &str = "Consulta o estado de um pedido.";
const GLOBAL_TOKEN: &str = "pophg_credencial-ficticia-do-teste";
const AUTHORIZATION: &str = "Bearer credencial-ficticia-da-loja";
const CALL_ID: &str = "consulta-1";

fn configured_tool(org: OrgId, name: &str, url: &str) -> HarnessConfig {
    toml::from_str(&format!(
        r#"
[http_tools."{org}"."{name}"]
url = "{url}"
method = "POST"
authorization_env = "POPH_HTTP_TOOL_TEST"
description = "{DESCRIPTION}"
parameters = {{ type = "object", properties = {{ pedido = {{ type = "string" }} }}, required = ["pedido"] }}
response_fields = {{ estado = "/pedido/estado", token = "/pedido/token" }}
"#
    ))
    .expect("declaração TOML")
}

#[test]
fn invalid_and_reserved_names_are_rejected_at_configuration_load() {
    let org = OrgId::generate();
    let url = "http://127.0.0.1:1/pedidos";
    let too_long = "a".repeat(60);
    for (name, expected) in [
        ("Consultar-Pedido", "inválido"),
        ("ab", "inválido"),
        (too_long.as_str(), "inválido"),
        ("transferir", "reservado"),
        ("registerlead", "reservado"),
        ("register__lead", "reservado"),
    ] {
        assert!(
            matches!(
                configured_tool(org, name, url).validar(),
                Err(DomainError::Validation { field: "http_tools", reason })
                    if reason.contains(expected)
            ),
            "{name}"
        );
    }

    for name in ["a_0", &"a".repeat(41), TOOL, "register_lead"] {
        configured_tool(org, name, url).validar().expect(name);
    }

    let mut config = configured_tool(org, TOOL, url);
    config.site_chat.allowed_tools.insert(TOOL.to_owned());
    assert!(matches!(
        config.validar(),
        Err(DomainError::Validation {
            field: "site_chat.allowed_tools",
            ..
        })
    ));
}

fn add_agent(store: &Store, slug: &str) -> (OrgId, AgentId) {
    let org = store
        .orgs()
        .create(NewOrg {
            name: "Loja de teste".to_owned(),
            slug: slug.to_owned(),
            config_json: "{}".to_owned(),
        })
        .expect("organização")
        .id;
    let agent = store
        .agents()
        .create(
            org,
            NewAgent {
                name: "Atendente de teste".to_owned(),
                slug: "atendente".to_owned(),
                grouping_window_ms: 1,
                ..Default::default()
            },
        )
        .expect("agente")
        .id;
    (org, agent)
}

async fn lookup_order(
    State(calls): State<mpsc::UnboundedSender<(HeaderMap, Value)>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Json<Value> {
    calls.send((headers, body)).expect("receber chamada HTTP");
    Json(json!({
        "pedido": {"estado": "enviado", "token": "valor-ficticio"},
        "interno": "somente na loja"
    }))
}

async fn read(response: Response, status: StatusCode) -> Value {
    let actual = response.status();
    let body: Value = response.json().await.expect("JSON da API");
    assert_eq!(actual, status, "{body}");
    body
}

#[tokio::test]
async fn declared_tool_runs_only_for_its_org_and_outside_site_chat() {
    let backend_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("porta da loja");
    let backend_url = format!("http://{}/pedidos", backend_listener.local_addr().unwrap());
    let (calls, mut received) = mpsc::unbounded_channel();
    let backend = Router::new()
        .route("/pedidos", post(lookup_order))
        .with_state(calls);
    let backend_task = tokio::spawn(axum::serve(backend_listener, backend).into_future());

    let mut config = HarnessConfig::default();
    config.data.directory =
        std::env::temp_dir().join(format!("poph-http-tools-{}", OrgId::generate()));
    let store = Arc::new(Store::open(&config).expect("banco local"));
    let (org, agent) = add_agent(&store, "loja-principal");
    let (other_org, other_agent) = add_agent(&store, "outra-loja");
    config.http_tools = configured_tool(org, TOOL, &backend_url).http_tools;
    config.admin.tokens_globais.push(GlobalTokenConfig {
        label: "teste".to_owned(),
        hash: Argon2::default()
            .hash_password(GLOBAL_TOKEN.as_bytes(), &SaltString::generate(&mut OsRng))
            .expect("hash")
            .to_string(),
    });
    config.validar().expect("configuração válida");

    let arguments = json!({"pedido": "123"});
    let fake = Arc::new(FakeProvider::with_scripts(vec![
        Script::Events(vec![LlmEvent::Tool(ToolCall {
            id: CALL_ID.to_owned(),
            nome: TOOL.to_owned(),
            arguments: arguments.clone(),
        })]),
        Script::Events(vec![
            LlmEvent::Texto("Seu pedido foi enviado.".to_owned()),
            LlmEvent::Fim(FinishReason::Completed),
        ]),
    ]));
    let registry = HttpToolRegistry::from_config(&config.http_tools, |name| {
        (name == "POPH_HTTP_TOOL_TEST").then(|| Secret::new(AUTHORIZATION.to_owned()))
    })
    .expect("ferramentas HTTP");
    let memory = MetricsMemory::own();
    let engine = Arc::new(
        TurnEngine::new(
            store.clone(),
            ConfigHot::new(store.clone(), HotConfigOptions::default(), memory.clone()),
            Arc::new(Limiter::new(LimitOptions::default(), memory.clone())),
            fake.clone(),
            TurnOptions::default(),
            MetricsTurn::own(),
        )
        .with_http_tools(registry),
    );
    let processor = Arc::new(TurnProcessor::new(engine.clone()));
    let actors = Actors::new(processor.clone(), ActorOptions::default(), memory);
    let flow = Fluxo::new(store.clone(), actors, FlowOptions::default());
    processor.attach_io(flow.clone());
    let directory = config.data.directory.clone();
    let state = ServerState::new(store.clone(), config)
        .expect("estado do servidor")
        .with_engine(engine)
        .with_turn(flow, Duration::from_millis(20));
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("porta da API");
    let base = format!("http://{}/admin/v1", listener.local_addr().unwrap());
    let server = tokio::spawn(poph_server::serve_at(
        listener,
        state,
        std::future::pending(),
    ));

    let client = Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .expect("cliente");
    let agent_url = format!("{base}/orgs/{org}/agents/{agent}");
    let instructions = json!({"structured": {"tools": [{"name": TOOL}]}});
    let write_instructions = |url: &str| {
        client
            .post(format!("{url}/instructions"))
            .bearer_auth(GLOBAL_TOKEN)
    };
    let error = read(
        write_instructions(&format!("{base}/orgs/{other_org}/agents/{other_agent}"))
            .json(&instructions)
            .send()
            .await
            .expect("escrita em outra organização"),
        StatusCode::BAD_REQUEST,
    )
    .await;
    assert_eq!(error["details"]["field"], "structured");
    let error = read(
        write_instructions(&agent_url)
            .json(&json!({"structured": {"tools": [{"name": "nao_declarada"}]}}))
            .send()
            .await
            .expect("escrita com ferramenta desconhecida"),
        StatusCode::BAD_REQUEST,
    )
    .await;
    assert_eq!(error["details"]["field"], "structured");
    read(
        write_instructions(&agent_url)
            .json(&instructions)
            .send()
            .await
            .expect("escrita das instruções"),
        StatusCode::CREATED,
    )
    .await;

    let refused = read(
        client
            .post(format!("{agent_url}/site-conversations"))
            .bearer_auth(GLOBAL_TOKEN)
            .json(&json!({"external_ref": "visitante"}))
            .send()
            .await
            .expect("abertura de conversa pública"),
        StatusCode::UNPROCESSABLE_ENTITY,
    )
    .await;
    assert_eq!(refused["code"], "agent_not_public");
    assert!(fake.received().is_empty());

    let chat = format!("{agent_url}/test-chat");
    let conversation = read(
        client
            .post(&chat)
            .bearer_auth(GLOBAL_TOKEN)
            .send()
            .await
            .unwrap(),
        StatusCode::CREATED,
    )
    .await;
    let conversation_id = conversation["id"].as_str().expect("conversa");
    let response = client
        .post(format!("{chat}/{conversation_id}/messages"))
        .bearer_auth(GLOBAL_TOKEN)
        .header("content-type", "multipart/form-data; boundary=pedido")
        .body("--pedido\r\nContent-Disposition: form-data; name=\"text\"\r\n\r\nComo está meu pedido?\r\n--pedido--\r\n")
        .send()
        .await
        .expect("mensagem");
    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        response
            .text()
            .await
            .expect("eventos do turno")
            .contains("Seu pedido foi enviado.")
    );

    let turns = store
        .turns()
        .list(org, conversation_id.parse().unwrap(), Page::with_limit(2))
        .expect("turno gravado");
    assert_eq!(turns.len(), 1);
    let turn = &turns[0];
    let record: Value = serde_json::from_str(&turn.tools_json).expect("JSON das chamadas");
    assert_eq!(record["chamadas"].as_array().unwrap().len(), 1);
    assert_eq!(record["chamadas"][0]["nome"], TOOL);
    assert_eq!(record["chamadas"][0]["estado"], "executada");
    assert_eq!(record["chamadas"][0]["argumentos"], "null");
    assert_eq!(
        record["chamadas"][0]["resultado"],
        r#"{"estado":"enviado","token":"[redigido]"}"#
    );

    let (headers, body) = received.try_recv().expect("chamada da ferramenta");
    assert_eq!(headers["authorization"], AUTHORIZATION);
    assert_eq!(headers["idempotency-key"], format!("{}:{CALL_ID}", turn.id));
    assert_eq!(body["call_id"], CALL_ID);
    assert_eq!(body["arguments"], arguments);
    assert_eq!(body["context"]["org_id"], json!(org));
    assert_eq!(body["context"]["agent_id"], json!(agent));
    assert_eq!(body["context"]["conversation_id"], conversation_id);
    assert_eq!(body["context"]["turn_id"], json!(turn.id));
    assert!(
        body["context"]["message_id"]
            .as_str()
            .is_some_and(|id| !id.is_empty())
    );
    assert!(received.try_recv().is_err());

    let requests = fake.received();
    let tools = &requests[0].tools;
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].nome, TOOL);
    assert_eq!(tools[0].descricao, DESCRIPTION);
    assert_eq!(
        tools[0].parameters,
        json!({
            "type": "object", "properties": {"pedido": {"type": "string"}}, "required": ["pedido"]
        })
    );
    assert_eq!(
        requests[1].messages.last(),
        Some(&Message::Resultado {
            id: CALL_ID.to_owned(),
            content: r#"{"estado":"enviado","token":"valor-ficticio"}"#.to_owned(),
        })
    );

    server.abort();
    backend_task.abort();
    let _ = server.await;
    let _ = backend_task.await;
    drop(store);
    let _ = std::fs::remove_dir_all(directory);
}
