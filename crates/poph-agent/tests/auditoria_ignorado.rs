//! Turno ignorado deixa trilha na auditoria (correção da HR-14).
//!
//! Antes, a linha saía com `actor_kind = 'engine'`, que a tabela não aceita: o erro era descartado em
//! silêncio e a trilha de turnos ignorados se perdia. Aqui a prova é a linha gravada e lida de volta.

use std::path::PathBuf;
use std::sync::Arc;

use poph_agent::config_quente::{ConfigQuente, OpcoesConfigQuente};
use poph_agent::limites::{Limitador, OpcoesLimites};
use poph_agent::metricas::{MetricasMemoria, MetricasTurno};
use poph_agent::turno::{Entrada, Motor, ObservadorNulo, OpcoesTurno};
use poph_core::llm::{LlmProvider, ProvedorFake};
use poph_store::{NovaConversa, NovaOrg, NovoAgente, Pagina, StatusConversa, Store};
use poph_types::{AgentId, ConversationId, OrgId};

#[tokio::test]
async fn turno_ignorado_grava_auditoria_com_ator_system() {
    let pasta: PathBuf = std::env::temp_dir().join(format!(
        "poph-auditoria-ignorado-{}-{}",
        std::process::id(),
        AgentId::generate()
    ));
    let _ = std::fs::remove_dir_all(&pasta);
    std::fs::create_dir_all(&pasta).expect("cria a pasta");
    let store = Arc::new(
        Store::abrir_em(&pasta.join("harness.db"), &pasta.join("rag.db")).expect("abre o banco"),
    );
    let org: OrgId = store
        .orgaos()
        .criar(NovaOrg {
            nome: "Org auditoria".to_owned(),
            slug: format!("org-auditoria-{}", AgentId::generate()),
            config_json: "{}".to_owned(),
        })
        .expect("cria a org")
        .id;
    let agente = store
        .agentes()
        .criar(
            org,
            NovoAgente {
                nome: "Bia".to_owned(),
                slug: "bia-auditoria".to_owned(),
                janela_agrupamento_ms: 0,
                ..NovoAgente::default()
            },
        )
        .expect("cria o agente")
        .id;
    let conversa: ConversationId = store
        .conversas()
        .criar(
            org,
            NovaConversa {
                agent_id: agente,
                integration_id: None,
                canal: "cli".to_owned(),
                peer_externo: None,
                departamento_alvo: None,
            },
        )
        .expect("abre a conversa")
        .id;
    // Conversa fechada: o turno é ignorado (D12) e cai no caminho de auditoria.
    let versao = store.conversas().obter(org, conversa).expect("lê").versao;
    store
        .conversas()
        .mudar_status(org, conversa, versao, StatusConversa::Fechada, None)
        .expect("fecha a conversa");

    let provedor: Arc<dyn LlmProvider> = Arc::new(ProvedorFake::com_roteiros(Vec::new()));
    let config = ConfigQuente::nova(
        Arc::clone(&store),
        OpcoesConfigQuente::default(),
        MetricasMemoria::proprio(),
    );
    let limitador = Arc::new(Limitador::novo(
        OpcoesLimites::default(),
        MetricasMemoria::proprio(),
    ));
    let motor = Motor::novo(
        Arc::clone(&store),
        config,
        limitador,
        provedor,
        OpcoesTurno::default(),
        MetricasTurno::proprio(),
    );
    let mensagem = store
        .mensagens()
        .gravar(
            org,
            poph_store::NovaMensagem {
                conversation_id: conversa,
                direcao: poph_store::Direcao::Entrada,
                papel: poph_store::Papel::Usuario,
                texto: "oi".to_owned(),
                anexos_json: "[]".to_owned(),
                id_externo: None,
                turn_id: None,
                origem: poph_store::Origem::Agente,
                autor_nome: None,
            },
        )
        .expect("grava a mensagem")
        .id;

    let resumo = motor
        .executar(Entrada::nova(org, conversa, mensagem), &mut ObservadorNulo)
        .await
        .expect("o turno roda");
    assert_eq!(resumo.estado.como_texto(), "skipped");

    let registros = store
        .auditoria()
        .listar(org, Pagina::com_limite(50))
        .expect("lê a auditoria");
    let turno_ignorado = registros
        .iter()
        .find(|registro| registro.acao == "skip")
        .expect("a linha de turno ignorado foi gravada");
    assert_eq!(turno_ignorado.ator_tipo, "system");
    assert_eq!(turno_ignorado.entidade, "turn");
    assert_eq!(
        turno_ignorado.entidade_id.as_deref(),
        Some(resumo.turno.to_string().as_str())
    );

    let _ = std::fs::remove_dir_all(&pasta);
}
