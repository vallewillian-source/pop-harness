#![allow(dead_code)]

mod support;

use std::collections::BTreeMap;
use std::sync::Arc;

use poph_agent::config_hot::{ConfigHot, HotConfigOptions};
use poph_agent::limites::{LimitOptions, Limiter};
use poph_agent::metrics::{MetricsMemory, MetricsTurn};
use poph_agent::providers::{Providers, ResolvedProvider};
use poph_agent::summary::SummaryOptions;
use poph_agent::turno::{TurnEngine, TurnOptions};
use poph_core::llm::{FakeProvider, FinishReason, LlmEvent, Script, Usage};
use poph_store::{AgentChange, Page};
use poph_types::AgentId;
use support::{NoWait, Scenario};

fn script(texto: &str) -> Script {
    Script::Events(vec![
        LlmEvent::Texto(texto.to_owned()),
        LlmEvent::Usage(Usage {
            input: 10,
            saida: 5,
        }),
        LlmEvent::Fim(FinishReason::Completed),
    ])
}

fn first_agent(scenario: &Scenario) -> AgentId {
    scenario
        .store
        .agents()
        .list(scenario.org, Page::with_limit(10))
        .expect("lista os agentes")
        .first()
        .expect("o cenário cria um agente")
        .id
}

fn define_provider_agent(scenario: &Scenario, provedor: &str) {
    let id = first_agent(scenario);
    let agente = scenario
        .store
        .agents()
        .get(scenario.org, id)
        .expect("lê o agente");
    scenario
        .store
        .agents()
        .update(
            scenario.org,
            id,
            agente.version,
            AgentChange {
                provedor: Some(provedor.to_owned()),
                modelo: Some(String::new()),
                ..AgentChange::default()
            },
        )
        .expect("troca o provedor do agente");
}

struct Palco {
    scenario: Scenario,
    default: Arc<FakeProvider>,
    designated: Arc<FakeProvider>,
}

impl Palco {
    fn new(label: &str, reply_default: &str, reply_named: &str) -> Self {
        let scenario = Scenario::new(label);
        Self {
            default: Arc::new(FakeProvider::with_scripts(vec![script(reply_default)])),
            designated: Arc::new(FakeProvider::with_scripts(vec![script(reply_named)])),
            scenario,
        }
    }

    fn engine(&self) -> TurnEngine {
        let mut named = BTreeMap::new();
        named.insert(
            "openai-forte".to_owned(),
            ResolvedProvider::new(
                "openai-forte",
                Arc::clone(&self.designated) as Arc<dyn poph_core::llm::LlmProvider>,
                "modelo-do-forte",
            ),
        );
        let default = ResolvedProvider::new(
            "antigravity2api",
            Arc::clone(&self.default) as Arc<dyn poph_core::llm::LlmProvider>,
            "modelo-do-padrao",
        );
        let provider = Arc::new(Providers::new(default, named));
        let config = ConfigHot::new(
            Arc::clone(&self.scenario.store),
            HotConfigOptions::default(),
            MetricsMemory::own(),
        );
        let limiter = Arc::new(Limiter::new(LimitOptions::default(), MetricsMemory::own()));
        TurnEngine::new(
            Arc::clone(&self.scenario.store),
            config,
            limiter,
            Arc::clone(&self.default) as Arc<dyn poph_core::llm::LlmProvider>,
            TurnOptions::default(),
            MetricsTurn::own(),
        )
        .with_pause(Arc::new(NoWait))
        .with_summary(SummaryOptions {
            ativo: false,
            ..SummaryOptions::default()
        })
        .with_providers(provider)
    }
}

#[tokio::test]
async fn agent_with_provider_named_receives_reply_its() {
    let palco = Palco::new("nomeado", "resposta do padrão", "resposta do forte");
    define_provider_agent(&palco.scenario, "openai-forte");
    let engine = palco.engine();

    let gatilho = palco.scenario.speak_with_files("oi", Vec::new());
    let summary = palco.scenario.turno(&engine, gatilho).await;

    assert_eq!(summary.modelo, "modelo-do-forte");
    assert_eq!(palco.designated.received().len(), 1);
    let request = &palco.designated.received()[0];
    assert_eq!(request.modelo.as_deref(), Some("modelo-do-forte"));
    assert!(
        palco.default.received().is_empty(),
        "o provedor padrão não deveria receber pedido"
    );
    let texto = palco
        .scenario
        .store
        .messages()
        .list(
            palco.scenario.org,
            palco.scenario.conversa,
            Page::with_limit(50),
        )
        .expect("lista as mensagens")
        .into_iter()
        .filter(|mensagem| mensagem.role == poph_store::Role::Assistant)
        .map(|mensagem| mensagem.texto)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(texto.contains("resposta do forte"), "{texto}");
}

#[tokio::test]
async fn choice_empty_name_protocol_fall_provider_default() {
    for choice in ["", "antigravity2api", "openai_compativel"] {
        let palco = Palco::new("padrao", "resposta do padrão", "resposta do forte");
        define_provider_agent(&palco.scenario, choice);
        let engine = palco.engine();
        let gatilho = palco.scenario.speak_with_files("oi", Vec::new());
        let summary = palco.scenario.turno(&engine, gatilho).await;
        assert_eq!(summary.modelo, "modelo-do-padrao", "escolha `{choice}`");
        assert_eq!(palco.default.received().len(), 1, "escolha `{choice}`");
        assert!(
            palco.designated.received().is_empty(),
            "escolha `{choice}`: o nomeado não deveria receber pedido"
        );
    }
}

#[tokio::test]
async fn provider_unknown_failure_turn_without_call_nobody() {
    let palco = Palco::new("desconhecido", "resposta do padrão", "resposta do forte");
    define_provider_agent(&palco.scenario, "sumiu-do-toml");
    let engine = palco.engine();
    let gatilho = palco.scenario.speak_with_files("oi", Vec::new());
    let summary = palco.scenario.turno(&engine, gatilho).await;

    assert_eq!(summary.estado, poph_store::TurnState::Failed);
    let error = summary.error.unwrap_or_default();
    assert!(error.contains("sumiu-do-toml"), "{error}");
    assert!(error.contains("openai-forte"), "{error}");
    assert!(palco.default.received().is_empty());
    assert!(palco.designated.received().is_empty());
}
