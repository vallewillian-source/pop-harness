#![allow(dead_code)]

mod apoio;

use std::collections::BTreeMap;
use std::sync::Arc;

use apoio::{Cenario, SemEspera};
use poph_agent::config_quente::{ConfigQuente, OpcoesConfigQuente};
use poph_agent::limites::{Limitador, OpcoesLimites};
use poph_agent::metricas::{MetricasMemoria, MetricasTurno};
use poph_agent::provedores::{ProvedorResolvido, Provedores};
use poph_agent::resumo::OpcoesResumo;
use poph_agent::turno::{Motor, OpcoesTurno};
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

fn primeiro_agente(cenario: &Cenario) -> AgentId {
    cenario
        .store
        .agents()
        .list(cenario.org, Page::with_limit(10))
        .expect("lista os agentes")
        .first()
        .expect("o cenário cria um agente")
        .id
}

fn definir_provedor_do_agente(cenario: &Cenario, provedor: &str) {
    let id = primeiro_agente(cenario);
    let agente = cenario
        .store
        .agents()
        .get(cenario.org, id)
        .expect("lê o agente");
    cenario
        .store
        .agents()
        .update(
            cenario.org,
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
    cenario: Cenario,
    padrao: Arc<FakeProvider>,
    nomeado: Arc<FakeProvider>,
}

impl Palco {
    fn novo(rotulo: &str, resposta_padrao: &str, resposta_nomeada: &str) -> Self {
        let cenario = Cenario::novo(rotulo);
        Self {
            padrao: Arc::new(FakeProvider::with_scripts(vec![roteiro(resposta_padrao)])),
            nomeado: Arc::new(FakeProvider::with_scripts(vec![roteiro(resposta_nomeada)])),
            cenario,
        }
    }

    fn motor(&self) -> Motor {
        let mut nomeados = BTreeMap::new();
        nomeados.insert(
            "openai-forte".to_owned(),
            ProvedorResolvido::novo(
                "openai-forte",
                Arc::clone(&self.nomeado) as Arc<dyn poph_core::llm::LlmProvider>,
                "modelo-do-forte",
            ),
        );
        let padrao = ProvedorResolvido::novo(
            "antigravity2api",
            Arc::clone(&self.padrao) as Arc<dyn poph_core::llm::LlmProvider>,
            "modelo-do-padrao",
        );
        let provedores = Arc::new(Provedores::novo(padrao, nomeados));
        let config = ConfigQuente::nova(
            Arc::clone(&self.cenario.store),
            OpcoesConfigQuente::default(),
            MetricasMemoria::proprio(),
        );
        let limitador = Arc::new(Limitador::novo(
            OpcoesLimites::default(),
            MetricasMemoria::proprio(),
        ));
        Motor::novo(
            Arc::clone(&self.cenario.store),
            config,
            limitador,
            Arc::clone(&self.padrao) as Arc<dyn poph_core::llm::LlmProvider>,
            OpcoesTurno::default(),
            MetricasTurno::proprio(),
        )
        .com_pausa(Arc::new(SemEspera))
        .com_resumo(OpcoesResumo {
            ativo: false,
            ..OpcoesResumo::default()
        })
        .com_provedores(provedores)
    }
}

#[tokio::test]
async fn agente_com_provedor_nomeado_recebe_a_resposta_dele() {
    let palco = Palco::novo("nomeado", "resposta do padrão", "resposta do forte");
    definir_provedor_do_agente(&palco.cenario, "openai-forte");
    let motor = palco.motor();

    let gatilho = palco.cenario.falar_com_arquivos("oi", Vec::new());
    let resumo = palco.cenario.turno(&motor, gatilho).await;

    assert_eq!(resumo.modelo, "modelo-do-forte");
    assert_eq!(palco.nomeado.received().len(), 1);
    let pedido = &palco.nomeado.received()[0];
    assert_eq!(pedido.modelo.as_deref(), Some("modelo-do-forte"));
    assert!(
        palco.padrao.received().is_empty(),
        "o provedor padrão não deveria receber pedido"
    );
    let texto = palco
        .cenario
        .store
        .messages()
        .list(
            palco.cenario.org,
            palco.cenario.conversa,
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
    for escolha in ["", "antigravity2api", "openai_compativel"] {
        let palco = Palco::novo("padrao", "resposta do padrão", "resposta do forte");
        definir_provedor_do_agente(&palco.cenario, escolha);
        let motor = palco.motor();
        let gatilho = palco.cenario.falar_com_arquivos("oi", Vec::new());
        let resumo = palco.cenario.turno(&motor, gatilho).await;
        assert_eq!(resumo.modelo, "modelo-do-padrao", "escolha `{escolha}`");
        assert_eq!(palco.padrao.received().len(), 1, "escolha `{escolha}`");
        assert!(
            palco.nomeado.received().is_empty(),
            "escolha `{escolha}`: o nomeado não deveria receber pedido"
        );
    }
}

#[tokio::test]
async fn provedor_desconhecido_falha_o_turno_sem_chamar_ninguem() {
    let palco = Palco::novo("desconhecido", "resposta do padrão", "resposta do forte");
    definir_provedor_do_agente(&palco.cenario, "sumiu-do-toml");
    let motor = palco.motor();
    let gatilho = palco.cenario.falar_com_arquivos("oi", Vec::new());
    let resumo = palco.cenario.turno(&motor, gatilho).await;

    assert_eq!(resumo.estado, poph_store::TurnState::Failed);
    let erro = resumo.erro.unwrap_or_default();
    assert!(erro.contains("sumiu-do-toml"), "{erro}");
    assert!(erro.contains("openai-forte"), "{erro}");
    assert!(palco.padrao.received().is_empty());
    assert!(palco.nomeado.received().is_empty());
}
