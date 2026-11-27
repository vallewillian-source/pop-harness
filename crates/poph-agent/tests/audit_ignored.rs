use std::path::PathBuf;
use std::sync::Arc;

use poph_agent::config_hot::{ConfigHot, HotConfigOptions};
use poph_agent::limites::{LimitOptions, Limiter};
use poph_agent::metrics::{MetricsMemory, MetricsTurn};
use poph_agent::turno::{NoopObserver, TurnEngine, TurnInput, TurnOptions};
use poph_core::llm::{FakeProvider, LlmProvider};
use poph_store::{ConversationStatus, NewAgent, NewConversation, NewOrg, Page, Store};
use poph_types::{AgentId, ConversationId, OrgId};

#[tokio::test]
async fn turn_ignored_records_audit_with_actor_system() {
    let folder: PathBuf = std::env::temp_dir().join(format!(
        "poph-auditoria-ignorado-{}-{}",
        std::process::id(),
        AgentId::generate()
    ));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).expect("cria a pasta");
    let store = Arc::new(
        Store::open_at(&folder.join("harness.db"), &folder.join("rag.db")).expect("abre o banco"),
    );
    let org: OrgId = store
        .orgs()
        .create(NewOrg {
            name: "Org auditoria".to_owned(),
            slug: format!("org-auditoria-{}", AgentId::generate()),
            config_json: "{}".to_owned(),
        })
        .expect("cria a org")
        .id;
    let agente = store
        .agents()
        .create(
            org,
            NewAgent {
                name: "Bia".to_owned(),
                slug: "bia-auditoria".to_owned(),
                grouping_window_ms: 0,
                ..NewAgent::default()
            },
        )
        .expect("cria o agente")
        .id;
    let conversa: ConversationId = store
        .conversations()
        .create(
            org,
            NewConversation {
                agent_id: agente,
                integration_id: None,
                canal: "cli".to_owned(),
                peer_externo: None,
                target_department: None,
            },
        )
        .expect("abre a conversa")
        .id;
    let versao = store
        .conversations()
        .get(org, conversa)
        .expect("lê")
        .version;
    store
        .conversations()
        .change_status(org, conversa, versao, ConversationStatus::Closed, None)
        .expect("fecha a conversa");

    let provedor: Arc<dyn LlmProvider> = Arc::new(FakeProvider::with_scripts(Vec::new()));
    let config = ConfigHot::new(
        Arc::clone(&store),
        HotConfigOptions::default(),
        MetricsMemory::own(),
    );
    let limiter = Arc::new(Limiter::new(LimitOptions::default(), MetricsMemory::own()));
    let engine = TurnEngine::new(
        Arc::clone(&store),
        config,
        limiter,
        provedor,
        TurnOptions::default(),
        MetricsTurn::own(),
    );
    let mensagem = store
        .messages()
        .write(
            org,
            poph_store::NewMessage {
                conversation_id: conversa,
                direction: poph_store::Direction::Inbound,
                role: poph_store::Role::User,
                texto: "oi".to_owned(),
                attachments_json: "[]".to_owned(),
                external_id: None,
                turn_id: None,
                origin: poph_store::Origin::Agente,
                author_name: None,
            },
        )
        .expect("grava a mensagem")
        .id;

    let summary = engine
        .run(TurnInput::new(org, conversa, mensagem), &mut NoopObserver)
        .await
        .expect("o turno roda");
    assert_eq!(summary.estado.as_text(), "skipped");

    let records = store
        .audit()
        .list(org, Page::with_limit(50))
        .expect("lê a auditoria");
    let turn_ignored = records
        .iter()
        .find(|registro| registro.acao == "skip")
        .expect("a linha de turno ignorado foi gravada");
    assert_eq!(turn_ignored.ator_tipo, "system");
    assert_eq!(turn_ignored.entity, "turn");
    assert_eq!(
        turn_ignored.entity_id.as_deref(),
        Some(summary.turno.to_string().as_str())
    );

    let _ = std::fs::remove_dir_all(&folder);
}
