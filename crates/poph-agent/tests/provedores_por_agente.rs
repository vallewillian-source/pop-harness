#![allow(dead_code)]

mod apoio;

use std::collections::BTreeMap;
use std::sync::Arc;

use apoio::{Scenario, SemEspera};
use poph_agent::config_hot::{ConfigHot, HotConfigOptions};
use poph_agent::limites::{LimitOptions, Limiter};
use poph_agent::metrics::{MetricsMemory, MetricsTurn};
use poph_agent::providers::{Providers, ResolvedProvider};
use poph_agent::summary::SummaryOptions;
use poph_agent::turno::{TurnEngine, TurnOptions};
use poph_core::llm::{FakeProvider, FinishReason, LlmEvent, Script, Usage};
use poph_store::{AgentChange, Page};
use poph_types::AgentId;

fn roteiro(texto: &str) -> Script {
    Script::Events(vec![
        LlmEvent::Texto(texto.to_owned()),
        LlmEvent::Usage(Usage {
            input: 10,
            saida: 5,
        }),
        LlmEvent::Fim(FinishReason::Completed),
    ])
}

fn primeiro_agente(scenario: &Scenario) -> AgentId {
    scenario
        .store
        .agents()
        .list(scenario.org, Page::with_limit(10))
        .expect("lista os agentes")
        .first()
        .expect("o cenário cria um agente")
        .id
}

fn definir_provedor_do_agente(scenario: &Scenario, provedor: &str) {
    let id = primeiro_agente(scenario);
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
    nomeado: Arc<FakeProvider>,
}

impl Palco {
    fn new(label: &str, resposta_padrao: &str, resposta_nomeada: &str) -> Self {
        let scenario = Scenario::new(label);
        Self {
            default: Arc::new(FakeProvider::with_scripts(vec![roteiro(resposta_padrao)])),
            nomeado: Arc::new(FakeProvider::with_scripts(vec![roteiro(resposta_nomeada)])),
            scenario,
        }
    }

    fn engine(&self) -> TurnEngine {
        let mut named = BTreeMap::new();
        named.insert(
            "openai-forte".to_owned(),
            ResolvedProvider::new(
                "openai-forte",
                Arc::clone(&self.nomeado) as Arc<dyn poph_core::llm::LlmProvider>,
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
        .with_pause(Arc::new(SemEspera))
        .with_summary(SummaryOptions {
            ativo: false,
            ..SummaryOptions::default()
        })
        .with_providers(provider)
    }
}

#[tokio::test]
async fn agente_com_provedor_nomeado_recebe_a_resposta_dele() {
    let palco = Palco::new("nomeado", "resposta do padrão", "resposta do forte");
    definir_provedor_do_agente(&palco.scenario, "openai-forte");
    let engine = palco.engine();

    let gatilho = palco.scenario.falar_com_arquivos("oi", Vec::new());
    let summary = palco.scenario.turno(&engine, gatilho).await;

    assert_eq!(summary.modelo, "modelo-do-forte");
    assert_eq!(palco.nomeado.received().len(), 1);
    let request = &palco.nomeado.received()[0];
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
async fn escolha_vazia_e_nome_de_protocolo_caem_no_provedor_padrao() {
    for choice in ["", "antigravity2api", "openai_compativel"] {
        let palco = Palco::new("padrao", "resposta do padrão", "resposta do forte");
        definir_provedor_do_agente(&palco.scenario, choice);
        let engine = palco.engine();
        let gatilho = palco.scenario.falar_com_arquivos("oi", Vec::new());
        let summary = palco.scenario.turno(&engine, gatilho).await;
        assert_eq!(summary.modelo, "modelo-do-padrao", "escolha `{choice}`");
        assert_eq!(palco.default.received().len(), 1, "escolha `{choice}`");
        assert!(
            palco.nomeado.received().is_empty(),
            "escolha `{choice}`: o nomeado não deveria receber pedido"
        );
    }
}

#[tokio::test]
async fn provedor_desconhecido_falha_o_turno_sem_chamar_ninguem() {
    let palco = Palco::new("desconhecido", "resposta do padrão", "resposta do forte");
    definir_provedor_do_agente(&palco.scenario, "sumiu-do-toml");
    let engine = palco.engine();
    let gatilho = palco.scenario.falar_com_arquivos("oi", Vec::new());
    let summary = palco.scenario.turno(&engine, gatilho).await;

    assert_eq!(summary.estado, poph_store::TurnState::Failed);
    let error = summary.error.unwrap_or_default();
    assert!(error.contains("sumiu-do-toml"), "{error}");
    assert!(error.contains("openai-forte"), "{error}");
    assert!(palco.default.received().is_empty());
    assert!(palco.nomeado.received().is_empty());
}
