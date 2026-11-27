use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use poph_flows::engine::{self, Campos, Context, Estado as EngineState};
use poph_flows::{Limites, RuleEvaluator, compile};
use poph_server::ServerState;
use poph_server::flow_transition::transition_engine;
use poph_store::metrics::ExecutionMetrics;
use poph_store::{
    ExecutionStart, ExecutionState, NewAgent, NewConversation, NewFlow, NewOrg,
    RevisionPublication, Store, TransitionExtras,
};
use poph_types::{ActionActor, EventActor, ExecutionStatus, FlowId, HarnessConfig, RevisionId};

const CONCLUSIVO: &str = r#"fluxo(
    id = "triagem_contadores",
    nome = "Triagem dos contadores",
    inicio = "coleta",
    gatilho = gatilho(
        quando = "O cliente precisa de ajuda com o equipamento.",
        nao_quando = "O cliente quer apenas o preço.",
        exemplos = ["preciso de ajuda"],
    ),
)

etapa(
    id = "coleta",
    nome = "Coleta",
    campos = [campo("resumo", "Resumo", tipo = "texto")],
    acoes = [
        acao("concluir", "Concluir", ator = "agente", vai_para = fim("pronto", resultado = "Resolvido")),
    ],
)
"#;

const BROKEN_RULE: &str = r#"fluxo(
    id = "quebra_de_regra",
    nome = "Quebra de regra",
    inicio = "coleta",
    gatilho = gatilho(
        quando = "O cliente relata um defeito.",
        nao_quando = "O cliente quer comprar.",
        exemplos = ["deu defeito"],
    ),
)

etapa(
    id = "coleta",
    nome = "Coleta",
    acoes = [
        acao("ir", "Ir para a decisão", ator = "agente", vai_para = "escolher"),
    ],
)

decisao(
    id = "escolher",
    se = funcao("quebrada"),
    entao = "etapa_a",
    senao = "etapa_b",
)

etapa(id = "etapa_a", nome = "A", acoes = [acao("ok", "Ok", ator = "agente", vai_para = fim("fim_a"))])
etapa(id = "etapa_b", nome = "B", acoes = [acao("ok", "Ok", ator = "agente", vai_para = fim("fim_b"))])

def quebrada(dados):
    return "sim"
"#;

const TWO_DECISIONS: &str = r#"fluxo(
    id = "laco_de_decisoes",
    nome = "Laço de decisões",
    inicio = "coleta",
    gatilho = gatilho(
        quando = "O cliente quer falar sobre a fatura.",
        nao_quando = "O cliente quer marcar visita.",
        exemplos = ["fatura"],
    ),
)

etapa(
    id = "coleta",
    nome = "Coleta",
    campos = [campo("marca", "Marca", tipo = "texto")],
    acoes = [
        acao("ir", "Ir para a decisão", ator = "agente", vai_para = "primeira"),
    ],
)

decisao(id = "primeira", se = igual("coleta.marca", "x"), entao = "segunda", senao = "segunda")

decisao(id = "segunda", se = igual("coleta.marca", "x"), entao = "etapa_a", senao = "etapa_b")

etapa(id = "etapa_a", nome = "A", acoes = [acao("ok", "Ok", ator = "agente", vai_para = fim("fim_a"))])
etapa(id = "etapa_b", nome = "B", acoes = [acao("ok", "Ok", ator = "agente", vai_para = fim("fim_b"))])
"#;

struct Scenario {
    estado: Option<ServerState>,
    org: poph_types::OrgId,
    agente: poph_types::AgentId,
    dir: PathBuf,
}

impl Scenario {
    fn fresh(label: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "poph-fx40-{label}-{}-{}",
            std::process::id(),
            poph_types::AgentId::generate()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("dados")).expect("pasta de dados");
        let mut config = HarnessConfig::default();
        config.data.directory = dir.join("dados");
        let store = Arc::new(Store::open(&config).expect("abre o banco"));
        let estado = ServerState::new(store, config).expect("estado do servidor");
        assert!(
            estado.flow_metrics().is_some(),
            "o motor de fluxos ligado declara os contadores"
        );
        let org = estado
            .store
            .orgs()
            .create(NewOrg {
                name: format!("Org {label}"),
                slug: format!("org-{label}-{}", poph_types::AgentId::generate()),
                config_json: "{}".to_owned(),
            })
            .expect("cria a org")
            .id;
        let agente = estado
            .store
            .agents()
            .create(
                org,
                NewAgent {
                    name: format!("Bia {label}"),
                    slug: format!("bia-{label}"),
                    ..NewAgent::default()
                },
            )
            .expect("cria o agente")
            .id;
        Self {
            estado: Some(estado),
            org,
            agente,
            dir,
        }
    }

    fn estado(&self) -> &ServerState {
        self.estado.as_ref().expect("o estado ainda está aqui")
    }

    fn take_state(&mut self) -> ServerState {
        self.estado.take().expect("o estado ainda está aqui")
    }

    fn store(&self) -> &Store {
        &self.estado().store
    }

    fn fluxo(&self, script: &str, slug: &str) -> (FlowId, RevisionId) {
        let fluxo = self
            .store()
            .flows()
            .create(
                self.org,
                NewFlow {
                    slug: slug.to_owned(),
                    name: format!("Fluxo {slug}"),
                    description: String::new(),
                    rascunho_fonte: script.to_owned(),
                },
            )
            .expect("cria o fluxo");
        let compiled = compile(script, &Limites::default());
        let ir = compiled
            .ir()
            .unwrap_or_else(|diagnostics| panic!("script do teste não compila: {diagnostics:?}"));
        let revision = self
            .store()
            .revisions()
            .publish_revision(
                self.org,
                fluxo.id,
                RevisionPublication {
                    expected: fluxo.version,
                    fonte: script.to_owned(),
                    definicao_json: serde_json::to_string(ir).expect("IR em JSON"),
                    schema_versao: Some(ir.schema_versao),
                    resolucoes_json: "{}".to_owned(),
                    ator_tipo: "system".to_owned(),
                    actor_id: None,
                },
            )
            .expect("publica a revisão");
        self.store()
            .flows()
            .release(self.org, self.agente, fluxo.id)
            .expect("libera o fluxo");
        (fluxo.id, revision.id)
    }

    fn ficha(&self) -> (poph_types::ConversationId, poph_types::RecordId) {
        let conversa = self
            .store()
            .conversations()
            .create(
                self.org,
                NewConversation {
                    agent_id: self.agente,
                    integration_id: None,
                    canal: "console".to_owned(),
                    peer_externo: Some(format!(
                        "cliente-ficticio-{}",
                        poph_types::AgentId::generate()
                    )),
                    target_department: None,
                },
            )
            .expect("abre a conversa");
        let ficha = self
            .store()
            .cases()
            .by_conversation(self.org, conversa.id)
            .expect("lê a Ficha")
            .expect("a conversa tem Ficha");
        (conversa.id, ficha.id)
    }

    fn open(
        &self,
        ficha: poph_types::RecordId,
        fluxo: FlowId,
        revision: RevisionId,
        chave: &str,
    ) -> poph_store::Execution {
        self.store()
            .executions()
            .start_execution(
                self.org,
                ExecutionStart {
                    case: ficha,
                    flow: fluxo,
                    revision,
                    actor: EventActor::Agent,
                    ator_ref: Some("atendente-de-teste"),
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
                    chave_idempotencia: Some(chave),
                },
            )
            .expect("abre a execução")
            .execution
    }

    fn act(
        &self,
        script: &str,
        execution: &poph_store::Execution,
        acao: &str,
        jumps: Option<usize>,
        chave: &str,
    ) -> poph_store::StoredExecution {
        let compiled = compile(script, &Limites::default());
        let ir = compiled
            .ir()
            .unwrap_or_else(|diagnostics| panic!("script do teste não compila: {diagnostics:?}"));
        let evaluator = RuleEvaluator::new(script).expect("as regras do script carregam");
        let estado = db_state(execution);
        let mut context = Context::new(&evaluator);
        if let Some(jumps) = jumps {
            context = context.with_jump_limit(jumps);
        }
        let resultado = engine::execute_action(
            ir,
            &estado,
            acao,
            &Campos::new(),
            ActionActor::Agent,
            Some(chave),
            &context,
        )
        .unwrap_or_else(|error| panic!("a passagem devia voltar desfecho: {error}"));
        self.store()
            .executions()
            .apply_transition(
                self.org,
                execution.id,
                execution.version,
                &transition_engine(&resultado),
                chave,
                TransitionExtras {
                    ator_ref: Some("atendente-de-teste"),
                    ..TransitionExtras::default()
                },
            )
            .expect("grava a transição")
    }

    fn counters(&self) -> ExecutionMetrics {
        self.estado()
            .flow_metrics()
            .expect("motor ligado")
            .execution_metrics()
    }

    fn values(&self) -> (u64, u64, u64, u64, u64) {
        let c = self.counters();
        (
            c.started.value(),
            c.completed.value(),
            c.abandoned.value(),
            c.failures.value(),
            c.rule_errors.value(),
        )
    }
}

impl Drop for Scenario {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn db_state(execution: &poph_store::Execution) -> EngineState {
    let ir = { &execution.no_atual };
    EngineState {
        fluxo_id: execution.fluxo_id.to_string(),
        status: execution.status,
        no_atual: poph_flows::ir::Destination::Etapa { id: ir.clone() },
        dados: poph_flows::Dados::new(),
        actors_who_acted: Vec::new(),
        resultado: execution.result.clone(),
        motivo: execution.motivo_fim.clone(),
        keys: Default::default(),
    }
}

#[test]
fn open_counts_once_and_repeat_key_does_not_count_again() {
    let scenario = Scenario::fresh("abertura");
    let (fluxo, revision) = scenario.fluxo(CONCLUSIVO, "triagem_contadores");
    let (_, ficha) = scenario.ficha();

    let first = scenario.open(ficha, fluxo, revision, "abertura-1");
    assert_eq!(
        scenario.values().0,
        1,
        "a abertura conta uma execução iniciada"
    );

    let repeated = scenario
        .store()
        .executions()
        .start_execution(
            scenario.org,
            ExecutionStart {
                case: ficha,
                flow: fluxo,
                revision,
                actor: EventActor::Agent,
                ator_ref: Some("atendente-de-teste"),
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
                chave_idempotencia: Some("abertura-1"),
            },
        )
        .expect("repetir a chave devolve a execução");
    assert!(repeated.repeated, "a segunda abertura é repetição");
    assert_eq!(repeated.execution.id, first.id);
    assert_eq!(scenario.values().0, 1, "repetir não soma outra iniciada");
}

#[test]
fn completion_counts_completed_and_repeat_key_does_not_add() {
    let scenario = Scenario::fresh("conclusao");
    let (fluxo, revision) = scenario.fluxo(CONCLUSIVO, "triagem_contadores");
    let (_, ficha) = scenario.ficha();
    let execution = scenario.open(ficha, fluxo, revision, "conclusao-1");

    let stored = scenario.act(CONCLUSIVO, &execution, "concluir", None, "acao-1");
    assert_eq!(stored.execution.status, ExecutionStatus::Completed);
    assert_eq!(
        scenario.values(),
        (1, 1, 0, 0, 0),
        "iniciada e concluída, nada em abandono nem em falha"
    );

    let again = scenario
        .store()
        .executions()
        .apply_transition(
            scenario.org,
            stored.execution.id,
            stored.execution.version,
            &transition_engine(&engine::Resultado {
                new_state: db_state(&stored.execution),
                events: Vec::new(),
                effects: Vec::new(),
                waiting: Vec::new(),
                repeated: true,
                ends_service: false,
                rule_error: false,
            }),
            "acao-1",
            TransitionExtras::default(),
        )
        .expect("repetir devolve o desfecho");
    assert!(again.repeated, "a repetição é marcada");
    assert_eq!(scenario.values(), (1, 1, 0, 0, 0), "nada somou");
}

#[test]
fn abandon_by_agent_counts_abandoned() {
    let scenario = Scenario::fresh("abandono");
    let (fluxo, revision) = scenario.fluxo(CONCLUSIVO, "triagem_contadores");
    let (_, ficha) = scenario.ficha();
    let execution = scenario.open(ficha, fluxo, revision, "abandono-1");

    let estado = db_state(&execution);
    let resultado = engine::abandon(
        &estado,
        "cliente desistiu",
        ActionActor::Agent,
        Some("ab-1"),
    )
    .expect("o abandono volta com o desfecho");
    scenario
        .store()
        .executions()
        .apply_transition(
            scenario.org,
            execution.id,
            execution.version,
            &transition_engine(&resultado),
            "ab-1",
            TransitionExtras::default(),
        )
        .expect("grava o abandono");

    assert_eq!(
        scenario.values(),
        (1, 0, 1, 0, 0),
        "uma iniciada, uma abandonada"
    );
}

#[test]
fn closing_case_with_active_execution_counts_abandon() {
    let scenario = Scenario::fresh("fechamento");
    let (fluxo, revision) = scenario.fluxo(CONCLUSIVO, "triagem_contadores");
    let (conversa, ficha) = scenario.ficha();
    scenario.open(ficha, fluxo, revision, "fechamento-1");
    assert_eq!(scenario.values().0, 1);

    let versao = scenario
        .store()
        .conversations()
        .get(scenario.org, conversa)
        .expect("lê a conversa")
        .version;
    scenario
        .store()
        .conversations()
        .close_audited(
            scenario.org,
            conversa,
            versao,
            "atendente-de-teste".to_owned(),
        )
        .expect("fecha o atendimento");

    assert_eq!(
        scenario.values(),
        (1, 0, 1, 0, 0),
        "o fechamento abandonou a execução ativa"
    );
}

#[test]
fn rule_failure_counts_failure_and_rule_error() {
    let scenario = Scenario::fresh("regra");
    let (fluxo, revision) = scenario.fluxo(BROKEN_RULE, "quebra_de_regra");
    let (_, ficha) = scenario.ficha();
    let execution = scenario.open(ficha, fluxo, revision, "regra-1");

    let stored = scenario.act(BROKEN_RULE, &execution, "ir", None, "acao-regra");
    assert_eq!(stored.execution.status, ExecutionStatus::Failed);
    assert_eq!(
        scenario.values(),
        (1, 0, 0, 1, 1),
        "a falha conta em falhas e a avaliação que não deu conta em erro de regra"
    );
}

#[test]
fn non_rule_failure_does_not_count_rule_error() {
    let scenario = Scenario::fresh("saltos");
    let (fluxo, revision) = scenario.fluxo(TWO_DECISIONS, "laco_de_decisoes");
    let (_, ficha) = scenario.ficha();
    let execution = scenario.open(ficha, fluxo, revision, "saltos-1");

    let stored = scenario.act(TWO_DECISIONS, &execution, "ir", Some(1), "acao-saltos");
    assert_eq!(stored.execution.status, ExecutionStatus::Failed);
    assert_eq!(
        scenario.values(),
        (1, 0, 0, 1, 0),
        "falha conta em falhas; erro de regra fica zerado"
    );
}

#[tokio::test]
async fn metrics_publishes_execution_counters() {
    let mut scenario = Scenario::fresh("metrics");
    let (fluxo, revision) = scenario.fluxo(CONCLUSIVO, "triagem_contadores");
    let (_, ficha) = scenario.ficha();
    let execution = scenario.open(ficha, fluxo, revision, "metrics-1");
    scenario.act(CONCLUSIVO, &execution, "concluir", None, "acao-metrics");
    let (_, other) = scenario.ficha();
    scenario.open(other, fluxo, revision, "metrics-2");
    assert_eq!(scenario.values(), (2, 1, 0, 0, 0));

    let estado = scenario.take_state();
    let texto = metrics_http(estado).await;
    let _ = fs::remove_dir_all(&scenario.dir);
    assert!(
        texto.contains("poph_fluxos_execucoes_iniciadas_total 2\n"),
        "{texto}"
    );
    assert!(
        texto.contains("poph_fluxos_execucoes_concluidas_total 1\n"),
        "{texto}"
    );
    assert!(
        texto.contains("poph_fluxos_execucoes_abandonadas_total 0\n")
            && texto.contains("poph_fluxos_execucoes_falhas_total 0\n")
            && texto.contains("poph_fluxos_erros_regra_total 0\n"),
        "os contadores sem uso aparecem zerados: {texto}"
    );
}

async fn metrics_http(estado: ServerState) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("porta efêmera");
    let endereco = listener.local_addr().expect("endereço");
    let (desligar, signal) = tokio::sync::oneshot::channel::<()>();
    let servidor = tokio::spawn(async move {
        let _ = poph_server::serve_at(listener, estado, async {
            let _ = signal.await;
        })
        .await;
    });
    let customer = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .expect("cliente HTTP");
    let mut body = None;
    for _ in 0..100 {
        if let Ok(reply) = customer
            .get(format!("http://{endereco}/metrics"))
            .send()
            .await
        {
            body = Some(reply.text().await.expect("corpo do /metrics"));
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let _ = desligar.send(());
    let _ = servidor.await;
    body.expect("o /metrics respondeu")
}
