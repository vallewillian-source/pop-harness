use std::sync::OnceLock;

use poph_flows::engine::{Campos, Context, entry_effects, execute_action, required_actors, start};
use poph_flows::ir::{
    Acao, Campo, Decisao, Destination, Effect, Etapa, Fim, Fluxo, Gatilho, Ir, Rule,
};
use poph_flows::rules::{Dados, RuleEvaluator};
use poph_types::{ActionActor, ExecutionStatus, FieldKind, RecordEventKind};
use serde_json::{Value as Json, json};

fn context() -> Context<'static> {
    static EVALUATOR: OnceLock<RuleEvaluator> = OnceLock::new();
    Context::new(EVALUATOR.get_or_init(RuleEvaluator::without_functions))
}

fn campo(id: &str, tipo: FieldKind, required: bool) -> Campo {
    Campo {
        id: id.to_owned(),
        label: id.to_uppercase(),
        tipo,
        required,
        options: Vec::new(),
        help: None,
    }
}

fn field_option(id: &str, options: &[&str]) -> Campo {
    Campo {
        id: id.to_owned(),
        label: id.to_uppercase(),
        tipo: FieldKind::Option,
        required: true,
        options: options.iter().map(|option| (*option).to_owned()).collect(),
        help: None,
    }
}

fn acao(id: &str, actor: Option<ActionActor>, requires: &[&str], target: Destination) -> Acao {
    Acao {
        id: id.to_owned(),
        label: id.to_uppercase(),
        requires: requires.iter().map(|nome| (*nome).to_owned()).collect(),
        actor,
        goes_to: target,
        effects: Vec::new(),
    }
}

fn action_for_step(
    id: &str,
    actor: Option<ActionActor>,
    requires: &[&str],
    goes_for: &str,
) -> Acao {
    acao(
        id,
        actor,
        requires,
        Destination::Etapa {
            id: goes_for.to_owned(),
        },
    )
}

fn action_for_end(
    id: &str,
    actor: Option<ActionActor>,
    resultado: &str,
    closes_service: bool,
) -> Acao {
    acao(
        id,
        actor,
        &[],
        Destination::Fim {
            id: "concluido".to_owned(),
            resultado: Some(resultado.to_owned()),
            ends_service: closes_service,
        },
    )
}

fn etapa(id: &str, actors: &[ActionActor], campos: Vec<Campo>, actions: Vec<Acao>) -> Etapa {
    Etapa {
        id: id.to_owned(),
        nome: id.to_uppercase(),
        instruction: None,
        action_actors: actors.to_vec(),
        campos,
        actions,
        on_enter: Vec::new(),
    }
}

fn step_with_entry(
    id: &str,
    actors: &[ActionActor],
    campos: Vec<Campo>,
    actions: Vec<Acao>,
    enter: Vec<Effect>,
) -> Etapa {
    Etapa {
        on_enter: enter,
        ..etapa(id, actors, campos, actions)
    }
}

fn decisao(id: &str, se: Rule, then: Destination, r#else: Destination) -> Decisao {
    Decisao {
        id: id.to_owned(),
        se,
        then_branch: then,
        else_branch: r#else,
    }
}

fn goes_for(id: &str) -> Destination {
    Destination::Etapa { id: id.to_owned() }
}

fn fluxo(beginning: &str, steps: Vec<Etapa>, decisions: Vec<Decisao>) -> Ir {
    Ir {
        schema_versao: poph_flows::ir::SCHEMA_VERSAO,
        fluxo: Fluxo {
            id: "teste_paridade".to_owned(),
            nome: "Teste da paridade".to_owned(),
            descricao: None,
        },
        gatilho: Gatilho {
            when: "o cliente pede um orçamento".to_owned(),
            not_when: None,
            examples: vec!["quero um orçamento".to_owned()],
        },
        start: beginning.to_owned(),
        stages: steps,
        decisions,
        ends: vec![
            Fim {
                id: "concluido".to_owned(),
                resultado: Some("Concluído".to_owned()),
                ends_service: false,
            },
            Fim {
                id: "encerrado".to_owned(),
                resultado: Some("Atendimento encerrado".to_owned()),
                ends_service: true,
            },
        ],
        defs: Vec::new(),
    }
}

fn campos(value: Json) -> Campos {
    value
        .as_object()
        .expect("o caso informa um objeto de campos")
        .clone()
}

fn without_fields() -> Campos {
    Campos::new()
}

fn dados(value: Json) -> Dados {
    value
        .as_object()
        .expect("o caso informa um objeto de dados")
        .clone()
}

fn kinds(resultado: &poph_flows::engine::Resultado) -> Vec<RecordEventKind> {
    resultado.events.iter().map(|evento| evento.tipo).collect()
}

fn flow_two_actors() -> Ir {
    fluxo(
        "execucao",
        vec![
            etapa(
                "execucao",
                &[ActionActor::Agent, ActionActor::Attendant],
                Vec::new(),
                vec![
                    action_for_step("agente_conclui", Some(ActionActor::Agent), &[], "pos"),
                    action_for_step("atendente_valida", Some(ActionActor::Attendant), &[], "pos"),
                ],
            ),
            etapa(
                "pos",
                &[ActionActor::Agent],
                Vec::new(),
                vec![action_for_end(
                    "fechar",
                    Some(ActionActor::Agent),
                    "Feito",
                    false,
                )],
            ),
        ],
        Vec::new(),
    )
}

#[test]
fn auto_advances_when_only_required_actor_is_who_acted() {
    let ir = fluxo(
        "abertura",
        vec![
            etapa(
                "abertura",
                &[ActionActor::Agent],
                vec![campo("relato", FieldKind::Text, true)],
                vec![action_for_step(
                    "registrar_relato",
                    None,
                    &["relato"],
                    "orcamento",
                )],
            ),
            etapa(
                "orcamento",
                &[ActionActor::Agent],
                Vec::new(),
                vec![action_for_end("fechar", None, "Feito", false)],
            ),
        ],
        Vec::new(),
    );
    let estado = start(&ir).expect("a execução começa");

    let resultado = execute_action(
        &ir,
        &estado,
        "registrar_relato",
        &campos(json!({ "relato": "Chuveiro queimado" })),
        ActionActor::Agent,
        Some("k1"),
        &context(),
    )
    .expect("a ação é aceita");

    assert_eq!(resultado.new_state.no_atual, goes_for("orcamento"));
    assert_eq!(resultado.new_state.status, ExecutionStatus::Active);
    assert_eq!(
        resultado.new_state.dados["abertura"]["relato"],
        json!("Chuveiro queimado")
    );
    assert!(
        resultado.waiting.is_empty(),
        "quem age sozinho não deixa ninguém pendente"
    );
    assert!(kinds(&resultado).contains(&RecordEventKind::StepEntered));
}

#[test]
fn auto_stays_partial_when_other_restricted_actor_missing() {
    let ir = flow_two_actors();
    let estado = start(&ir).expect("a execução começa");

    let resultado = execute_action(
        &ir,
        &estado,
        "atendente_valida",
        &without_fields(),
        ActionActor::Attendant,
        Some("k1"),
        &context(),
    )
    .expect("a ação é aceita");

    assert_eq!(resultado.new_state.no_atual, goes_for("execucao"));
    assert_eq!(resultado.new_state.status, ExecutionStatus::Active);
    assert_eq!(
        resultado.new_state.actors_who_acted,
        vec![ActionActor::Attendant]
    );
    assert_eq!(
        resultado.waiting,
        vec![ActionActor::Agent],
        "o Python devolve `waiting_on == [agent]`"
    );
    assert_eq!(kinds(&resultado), vec![RecordEventKind::ActionExecuted]);
    assert_eq!(
        required_actors(ir.stages.first().expect("há etapa")),
        vec![ActionActor::Agent, ActionActor::Attendant]
    );
}

#[test]
fn auto_advances_when_second_actor_restricted_acts() {
    let ir = flow_two_actors();
    let estado = start(&ir).expect("a execução começa");
    let partial = execute_action(
        &ir,
        &estado,
        "atendente_valida",
        &without_fields(),
        ActionActor::Attendant,
        Some("k1"),
        &context(),
    )
    .expect("a primeira ação é aceita");

    let resultado = execute_action(
        &ir,
        &partial.new_state,
        "agente_conclui",
        &without_fields(),
        ActionActor::Agent,
        Some("k2"),
        &context(),
    )
    .expect("a segunda ação é aceita");

    assert_eq!(resultado.new_state.no_atual, goes_for("pos"));
    assert!(resultado.waiting.is_empty());
    assert_eq!(resultado.new_state.actors_who_acted, Vec::new());
    assert_eq!(kinds(&resultado)[0], RecordEventKind::ActionExecuted);
    assert!(kinds(&resultado).contains(&RecordEventKind::StepCompleted));
}

#[test]
fn auto_both_actors_arrive_same_target_in_any_order() {
    let ir = flow_two_actors();
    let estado = start(&ir).expect("a execução começa");

    let agent_first = execute_action(
        &ir,
        &estado,
        "agente_conclui",
        &without_fields(),
        ActionActor::Agent,
        Some("k1"),
        &context(),
    )
    .expect("a ação do agente é aceita");
    assert_eq!(agent_first.waiting, vec![ActionActor::Attendant]);
    let after = execute_action(
        &ir,
        &agent_first.new_state,
        "atendente_valida",
        &without_fields(),
        ActionActor::Attendant,
        Some("k2"),
        &context(),
    )
    .expect("a ação do atendente é aceita");
    assert_eq!(after.new_state.no_atual, goes_for("pos"));

    let attendant_first = execute_action(
        &ir,
        &estado,
        "atendente_valida",
        &without_fields(),
        ActionActor::Attendant,
        Some("k1"),
        &context(),
    )
    .expect("a ação do atendente é aceita");
    assert_eq!(attendant_first.waiting, vec![ActionActor::Agent]);
    let after = execute_action(
        &ir,
        &attendant_first.new_state,
        "agente_conclui",
        &without_fields(),
        ActionActor::Agent,
        Some("k2"),
        &context(),
    )
    .expect("a ação do agente é aceita");
    assert_eq!(after.new_state.no_atual, goes_for("pos"));
}

#[test]
fn auto_actor_without_own_action_does_not_lock_step() {
    let ir = fluxo(
        "abertura",
        vec![
            etapa(
                "abertura",
                &[ActionActor::Agent, ActionActor::Attendant],
                Vec::new(),
                vec![action_for_step(
                    "relatar",
                    Some(ActionActor::Agent),
                    &[],
                    "agendamento",
                )],
            ),
            etapa(
                "agendamento",
                &[ActionActor::Agent],
                Vec::new(),
                vec![action_for_end("fechar", None, "Feito", false)],
            ),
        ],
        Vec::new(),
    );
    let estado = start(&ir).expect("a execução começa");

    let resultado = execute_action(
        &ir,
        &estado,
        "relatar",
        &without_fields(),
        ActionActor::Agent,
        Some("k1"),
        &context(),
    )
    .expect("a ação é aceita");

    assert_eq!(resultado.new_state.no_atual, goes_for("agendamento"));
    assert!(resultado.waiting.is_empty());
}

#[test]
fn auto_action_leading_to_end_is_decisive_with_other_actor_pending() {
    let ir = fluxo(
        "orcamento",
        vec![
            etapa(
                "orcamento",
                &[ActionActor::Agent, ActionActor::Attendant],
                Vec::new(),
                vec![
                    action_for_end("cancelar", Some(ActionActor::Agent), "Cancelado", false),
                    action_for_step("aprovar", Some(ActionActor::Attendant), &[], "fechamento"),
                ],
            ),
            etapa(
                "fechamento",
                &[ActionActor::Agent],
                Vec::new(),
                vec![action_for_end("fechar", None, "Feito", false)],
            ),
        ],
        Vec::new(),
    );
    let estado = start(&ir).expect("a execução começa");

    let resultado = execute_action(
        &ir,
        &estado,
        "cancelar",
        &without_fields(),
        ActionActor::Agent,
        Some("k1"),
        &context(),
    )
    .expect("a ação é aceita");

    assert_eq!(resultado.new_state.status, ExecutionStatus::Completed);
    assert_eq!(resultado.new_state.resultado.as_deref(), Some("Cancelado"));
    assert_eq!(
        kinds(&resultado).last(),
        Some(&RecordEventKind::ExecutionCompleted)
    );
    assert!(resultado.waiting.is_empty());
}

#[test]
fn action_restricted_attendant_refused_for_agent() {
    let ir = fluxo(
        "validacao",
        vec![
            etapa(
                "validacao",
                &[ActionActor::Agent, ActionActor::Attendant],
                Vec::new(),
                vec![
                    action_for_step("validar", Some(ActionActor::Attendant), &[], "pos"),
                    action_for_end("encerrar", None, "Feito", false),
                ],
            ),
            etapa(
                "pos",
                &[ActionActor::Agent],
                Vec::new(),
                vec![action_for_end("fechar", None, "Feito", false)],
            ),
        ],
        Vec::new(),
    );
    let estado = start(&ir).expect("a execução começa");

    let error = execute_action(
        &ir,
        &estado,
        "validar",
        &without_fields(),
        ActionActor::Agent,
        Some("k1"),
        &context(),
    )
    .expect_err("o agente não executa ação do atendente");

    assert_eq!(error.api_code(), "acao_invalida");
    assert!(
        error.to_string().contains("restrita ao ator `atendente`"),
        "{error}"
    );
}

#[test]
fn actor_outside_actors_step_and_refused() {
    let ir = fluxo(
        "atendimento_humano",
        vec![etapa(
            "atendimento_humano",
            &[ActionActor::Attendant],
            Vec::new(),
            vec![action_for_end("encerrar", None, "Feito", false)],
        )],
        Vec::new(),
    );
    let estado = start(&ir).expect("a execução começa");

    let error = execute_action(
        &ir,
        &estado,
        "encerrar",
        &without_fields(),
        ActionActor::Agent,
        Some("k1"),
        &context(),
    )
    .expect_err("o agente não age nesta etapa");

    assert_eq!(error.api_code(), "acao_invalida");
    assert!(
        error
            .to_string()
            .contains("não age na etapa `atendimento_humano`"),
        "{error}"
    );
}

#[test]
fn missing_action_returns_valid_ones_and_changes_nothing() {
    let ir = fluxo(
        "orcamento",
        vec![
            etapa(
                "orcamento",
                &[ActionActor::Agent],
                Vec::new(),
                vec![
                    action_for_step("enviar_orcamento", None, &[], "fechamento"),
                    action_for_end("cancelar", None, "Cancelado", false),
                ],
            ),
            etapa(
                "fechamento",
                &[ActionActor::Agent],
                Vec::new(),
                vec![action_for_end("fechar", None, "Feito", false)],
            ),
        ],
        Vec::new(),
    );
    let estado = start(&ir).expect("a execução começa");

    let error = execute_action(
        &ir,
        &estado,
        "aprovar",
        &without_fields(),
        ActionActor::Agent,
        Some("k1"),
        &context(),
    )
    .expect_err("a ação não existe na etapa");

    let mensagem = error.to_string();
    assert_eq!(error.api_code(), "acao_invalida");
    assert!(mensagem.contains("`enviar_orcamento`"), "{mensagem}");
    assert!(mensagem.contains("`cancelar`"), "{mensagem}");
    assert!(estado.dados.is_empty());
    assert!(estado.keys.is_empty());
    assert_eq!(estado.no_atual, goes_for("orcamento"));
}

#[test]
fn closed_execution_does_not_accept_action() {
    let ir = fluxo(
        "coleta",
        vec![etapa(
            "coleta",
            &[ActionActor::Agent],
            Vec::new(),
            vec![
                action_for_end("concluir", None, "Feito", false),
                action_for_step("voltar", None, &[], "coleta"),
            ],
        )],
        Vec::new(),
    );
    let estado = start(&ir).expect("a execução começa");
    let finished = execute_action(
        &ir,
        &estado,
        "concluir",
        &without_fields(),
        ActionActor::Agent,
        Some("k1"),
        &context(),
    )
    .expect("a ação é aceita");
    assert_eq!(finished.new_state.status, ExecutionStatus::Completed);

    let error = execute_action(
        &ir,
        &finished.new_state,
        "voltar",
        &without_fields(),
        ActionActor::Agent,
        Some("k2"),
        &context(),
    )
    .expect_err("execução concluída não aceita ação");

    assert_eq!(error.api_code(), "acao_invalida");
    assert!(error.to_string().contains("não aceita ação"), "{error}");
}

#[test]
fn no_state_closing_pending() {
    let error = ExecutionStatus::from_text("closing_pending")
        .expect_err("`closing_pending` não é situação de execução no Harness");

    assert_eq!(error.api_code(), "validation_error");
    assert!(
        error
            .to_string()
            .contains("aceitos: ativa, concluida, abandonada, falha"),
        "{error}"
    );
    for situacao in ["ativa", "concluida", "abandonada", "falha"] {
        assert!(
            ExecutionStatus::from_text(situacao).is_ok(),
            "`{situacao}` é situação válida"
        );
    }
}

#[test]
fn conditional_with_operators_pop_atende_chooses_branch() {
    let evaluator = RuleEvaluator::without_functions();
    let execution = dados(json!({
        "orcamento": {
            "tipo": "hidraulica",
            "valor": 800,
            "confirmou": true,
            "ativo": false,
            "servicos": "agua e gas",
            "a": 10,
            "b": 20,
        }
    }));

    let cases: Vec<(&str, Rule, bool)> = vec![
        (
            "`tipo == \"hidraulica\"`",
            Rule::Igual {
                campo: "orcamento.tipo".to_owned(),
                value: "hidraulica".to_owned(),
            },
            true,
        ),
        (
            "`tipo == \"eletrica\"`",
            Rule::Igual {
                campo: "orcamento.tipo".to_owned(),
                value: "eletrica".to_owned(),
            },
            false,
        ),
        (
            "`valor > 500`",
            Rule::Maior {
                campo: "orcamento.valor".to_owned(),
                value: "500".to_owned(),
            },
            true,
        ),
        (
            "`valor > 1000`",
            Rule::Maior {
                campo: "orcamento.valor".to_owned(),
                value: "1000".to_owned(),
            },
            false,
        ),
        (
            "`valor >= 800`, montado como `nao(menor(...))`",
            Rule::Nao {
                args: vec![Rule::Menor {
                    campo: "orcamento.valor".to_owned(),
                    value: "800".to_owned(),
                }],
            },
            true,
        ),
        (
            "`confirmou == true`",
            Rule::Igual {
                campo: "orcamento.confirmou".to_owned(),
                value: "sim".to_owned(),
            },
            true,
        ),
        (
            "`a > 5 and b > 15`",
            Rule::E {
                args: vec![
                    Rule::Maior {
                        campo: "orcamento.a".to_owned(),
                        value: "5".to_owned(),
                    },
                    Rule::Maior {
                        campo: "orcamento.b".to_owned(),
                        value: "15".to_owned(),
                    },
                ],
            },
            true,
        ),
        (
            "`a > 15 and b > 5`",
            Rule::E {
                args: vec![
                    Rule::Maior {
                        campo: "orcamento.a".to_owned(),
                        value: "15".to_owned(),
                    },
                    Rule::Maior {
                        campo: "orcamento.b".to_owned(),
                        value: "5".to_owned(),
                    },
                ],
            },
            false,
        ),
        (
            "`a > 15 or b > 15`",
            Rule::Ou {
                args: vec![
                    Rule::Maior {
                        campo: "orcamento.a".to_owned(),
                        value: "15".to_owned(),
                    },
                    Rule::Maior {
                        campo: "orcamento.b".to_owned(),
                        value: "15".to_owned(),
                    },
                ],
            },
            true,
        ),
        (
            "`not ativo`",
            Rule::Nao {
                args: vec![Rule::Igual {
                    campo: "orcamento.ativo".to_owned(),
                    value: "sim".to_owned(),
                }],
            },
            true,
        ),
        (
            "`\"agua\" in servicos`",
            Rule::Contem {
                campo: "orcamento.servicos".to_owned(),
                value: "agua".to_owned(),
            },
            true,
        ),
        (
            "`\"eletrica\" in servicos`",
            Rule::Contem {
                campo: "orcamento.servicos".to_owned(),
                value: "eletrica".to_owned(),
            },
            false,
        ),
    ];

    for (nome, rule, expected_json) in cases {
        let obtained = evaluator
            .evaluate(&rule, &execution)
            .unwrap_or_else(|error| panic!("{nome}: {error}"));
        assert_eq!(obtained, expected_json, "{nome}");
    }
}

#[test]
fn two_conditionals_chained_choose_step_final() {
    let ir = fluxo(
        "triagem",
        vec![
            etapa(
                "triagem",
                &[ActionActor::Agent],
                vec![
                    field_option("tipo", &["hidraulica", "eletrica"]),
                    campo("valor", FieldKind::Number, true),
                ],
                vec![acao(
                    "seguir",
                    None,
                    &["tipo", "valor"],
                    Destination::Decisao {
                        id: "tipo_define".to_owned(),
                    },
                )],
            ),
            etapa(
                "caro",
                &[ActionActor::Agent],
                Vec::new(),
                vec![action_for_end("fechar", None, "Caro", false)],
            ),
            etapa(
                "barato",
                &[ActionActor::Agent],
                Vec::new(),
                vec![action_for_end("fechar", None, "Barato", false)],
            ),
        ],
        vec![
            decisao(
                "tipo_define",
                Rule::Igual {
                    campo: "triagem.tipo".to_owned(),
                    value: "hidraulica".to_owned(),
                },
                Destination::Decisao {
                    id: "valor_define".to_owned(),
                },
                goes_for("barato"),
            ),
            decisao(
                "valor_define",
                Rule::Maior {
                    campo: "triagem.valor".to_owned(),
                    value: "500".to_owned(),
                },
                goes_for("caro"),
                goes_for("barato"),
            ),
        ],
    );
    let estado = start(&ir).expect("a execução começa");

    let resultado = execute_action(
        &ir,
        &estado,
        "seguir",
        &campos(json!({ "tipo": "hidraulica", "valor": 800 })),
        ActionActor::Agent,
        Some("k1"),
        &context(),
    )
    .expect("a ação é aceita");

    assert_eq!(resultado.new_state.no_atual, goes_for("caro"));
    let resolvidas: Vec<&poph_flows::engine::Evento> = resultado
        .events
        .iter()
        .filter(|evento| evento.tipo == RecordEventKind::DecisionResolved)
        .collect();
    assert_eq!(resolvidas.len(), 2, "duas decisões numa passagem só");
    assert_eq!(resolvidas[0].payload["decisao"], json!("tipo_define"));
    assert_eq!(resolvidas[1].payload["decisao"], json!("valor_define"));
}

const SCRIPT_WITH_DEF: &str = r#"
fluxo(
    id = "teste_paridade_def",
    nome = "Teste da paridade com def",
    inicio = "triagem",
    gatilho = gatilho(
        quando = "o cliente relata um problema",
        nao_quando = "o cliente quer falar com uma pessoa",
        exemplos = ["o chuveiro parou"],
    ),
)

etapa(
    id = "triagem",
    nome = "Triagem",
    campos = [campo("relato", "Relato", obrigatorio = True)],
    acoes = [acao("seguir", "Seguir", exige = ["relato"], vai_para = fim("pronto"))],
)

fim("pronto", resultado = "Registrado")

def grave(dados):
    resumo = dados.get("triagem", {}).get("relato", "").lower()
    return "parou" in resumo
"#;

#[test]
fn conditional_with_def_chooses_branch() {
    let evaluator = RuleEvaluator::new(SCRIPT_WITH_DEF).expect("o script de apoio carrega");
    let ir = fluxo(
        "triagem",
        vec![
            etapa(
                "triagem",
                &[ActionActor::Agent],
                vec![campo("relato", FieldKind::Text, true)],
                vec![acao(
                    "seguir",
                    None,
                    &["relato"],
                    Destination::Decisao {
                        id: "grave_ou_simples".to_owned(),
                    },
                )],
            ),
            etapa(
                "grave",
                &[ActionActor::Agent],
                Vec::new(),
                vec![action_for_end("fechar", None, "Grave", false)],
            ),
            etapa(
                "simples",
                &[ActionActor::Agent],
                Vec::new(),
                vec![action_for_end("fechar", None, "Simples", false)],
            ),
        ],
        vec![decisao(
            "grave_ou_simples",
            Rule::Funcao {
                nome: "grave".to_owned(),
            },
            goes_for("grave"),
            goes_for("simples"),
        )],
    );
    let context = Context::new(&evaluator);
    let estado = start(&ir).expect("a execução começa");

    let case_severe = execute_action(
        &ir,
        &estado,
        "seguir",
        &campos(json!({ "relato": "O chuveiro parou de esquentar" })),
        ActionActor::Agent,
        Some("k1"),
        &context,
    )
    .expect("a ação é aceita");
    assert_eq!(case_severe.new_state.no_atual, goes_for("grave"));

    let case_simple = execute_action(
        &ir,
        &estado,
        "seguir",
        &campos(json!({ "relato": "Quero trocar a torneira da pia" })),
        ActionActor::Agent,
        Some("k2"),
        &context,
    )
    .expect("a ação é aceita");
    assert_eq!(case_simple.new_state.no_atual, goes_for("simples"));
}

#[test]
fn conditional_to_missing_step_fails_closed() {
    let ir = fluxo(
        "triagem",
        vec![etapa(
            "triagem",
            &[ActionActor::Agent],
            Vec::new(),
            vec![acao(
                "seguir",
                None,
                &[],
                Destination::Decisao {
                    id: "bifurcar".to_owned(),
                },
            )],
        )],
        vec![decisao(
            "bifurcar",
            Rule::Vazio {
                campo: "triagem.nada".to_owned(),
            },
            goes_for("fantasma"),
            goes_for("triagem"),
        )],
    );
    let estado = start(&ir).expect("a execução começa");

    let error = execute_action(
        &ir,
        &estado,
        "seguir",
        &without_fields(),
        ActionActor::Agent,
        Some("k1"),
        &context(),
    )
    .expect_err("destino inexistente é divergência de IR");

    assert_eq!(error.api_code(), "ir_divergente");
    assert!(error.to_string().contains("`fantasma`"), "{error}");
}

#[test]
fn field_required_missing_returns_list() {
    let ir = fluxo(
        "abertura",
        vec![
            etapa(
                "abertura",
                &[ActionActor::Agent],
                vec![
                    campo("valor", FieldKind::Number, true),
                    campo("descricao", FieldKind::Text, true),
                ],
                vec![action_for_step(
                    "enviar_orcamento",
                    None,
                    &["valor", "descricao"],
                    "fechamento",
                )],
            ),
            etapa(
                "fechamento",
                &[ActionActor::Agent],
                Vec::new(),
                vec![action_for_end("fechar", None, "Feito", false)],
            ),
        ],
        Vec::new(),
    );
    let estado = start(&ir).expect("a execução começa");

    let error = execute_action(
        &ir,
        &estado,
        "enviar_orcamento",
        &campos(json!({ "valor": 1000.0 })),
        ActionActor::Agent,
        Some("k1"),
        &context(),
    )
    .expect_err("falta o campo obrigatório");

    assert_eq!(error.api_code(), "campos_invalidos");
    let mensagem = error.to_string();
    assert!(mensagem.contains("`descricao`"), "{mensagem}");
    assert!(mensagem.contains("ainda não foi preenchido"), "{mensagem}");
}

#[test]
fn field_text_refusal_number() {
    let ir = fluxo(
        "abertura",
        vec![etapa(
            "abertura",
            &[ActionActor::Agent],
            vec![campo("relato", FieldKind::Text, true)],
            vec![action_for_end("registrar", None, "Feito", false)],
        )],
        Vec::new(),
    );
    let estado = start(&ir).expect("a execução começa");

    let accepted = execute_action(
        &ir,
        &estado,
        "registrar",
        &campos(json!({ "relato": "  texto curto  " })),
        ActionActor::Agent,
        Some("k1"),
        &context(),
    )
    .expect("texto é aceito");
    assert_eq!(
        accepted.new_state.dados["abertura"]["relato"],
        json!("texto curto"),
        "o texto é gravado sem os espaços das pontas"
    );

    let error = execute_action(
        &ir,
        &estado,
        "registrar",
        &campos(json!({ "relato": 42 })),
        ActionActor::Agent,
        Some("k2"),
        &context(),
    )
    .expect_err("número não é texto");

    assert_eq!(error.api_code(), "campos_invalidos");
    assert!(error.to_string().contains("esperava texto"), "{error}");
}

#[test]
fn field_number_refusal_text() {
    let ir = fluxo(
        "orcamento",
        vec![etapa(
            "orcamento",
            &[ActionActor::Agent],
            vec![campo("valor", FieldKind::Number, true)],
            vec![action_for_end("enviar", None, "Enviado", false)],
        )],
        Vec::new(),
    );
    let estado = start(&ir).expect("a execução começa");

    let accepted = execute_action(
        &ir,
        &estado,
        "enviar",
        &campos(json!({ "valor": 1000.0 })),
        ActionActor::Agent,
        Some("k1"),
        &context(),
    )
    .expect("número é aceito");
    assert_eq!(
        accepted.new_state.dados["orcamento"]["valor"],
        json!(1000.0)
    );

    let error = execute_action(
        &ir,
        &estado,
        "enviar",
        &campos(json!({ "valor": "mil" })),
        ActionActor::Agent,
        Some("k2"),
        &context(),
    )
    .expect_err("texto não é número");

    assert_eq!(error.api_code(), "campos_invalidos");
    assert!(error.to_string().contains("esperava número"), "{error}");
}

#[test]
fn field_option_refusal_value_outside_list() {
    let ir = fluxo(
        "triagem",
        vec![etapa(
            "triagem",
            &[ActionActor::Agent],
            vec![field_option("periodo", &["manha", "tarde"])],
            vec![action_for_end("agendar", None, "Agendado", false)],
        )],
        Vec::new(),
    );
    let estado = start(&ir).expect("a execução começa");

    let accepted = execute_action(
        &ir,
        &estado,
        "agendar",
        &campos(json!({ "periodo": "tarde" })),
        ActionActor::Agent,
        Some("k1"),
        &context(),
    )
    .expect("opção declarada é aceita");
    assert_eq!(
        accepted.new_state.dados["triagem"]["periodo"],
        json!("tarde")
    );

    let error = execute_action(
        &ir,
        &estado,
        "agendar",
        &campos(json!({ "periodo": "madrugada" })),
        ActionActor::Agent,
        Some("k2"),
        &context(),
    )
    .expect_err("valor fora da lista é recusado");

    assert_eq!(error.api_code(), "campos_invalidos");
    let mensagem = error.to_string();
    assert!(mensagem.contains("esperava uma das opções"), "{mensagem}");
    assert!(mensagem.contains("manha"), "{mensagem}");
    assert!(mensagem.contains("tarde"), "{mensagem}");
}

#[test]
fn date_field_accepts_iso_refuses_free_text() {
    let ir = fluxo(
        "agendamento",
        vec![etapa(
            "agendamento",
            &[ActionActor::Agent],
            vec![campo("data_hora_visita", FieldKind::Data, true)],
            vec![action_for_end("confirmar", None, "Confirmado", false)],
        )],
        Vec::new(),
    );
    let estado = start(&ir).expect("a execução começa");

    let accepted = execute_action(
        &ir,
        &estado,
        "confirmar",
        &campos(json!({ "data_hora_visita": "2026-03-15T14:00:00" })),
        ActionActor::Agent,
        Some("k1"),
        &context(),
    )
    .expect("data ISO é aceita");
    assert_eq!(
        accepted.new_state.dados["agendamento"]["data_hora_visita"],
        json!("2026-03-15T14:00:00")
    );

    let error = execute_action(
        &ir,
        &estado,
        "confirmar",
        &campos(json!({ "data_hora_visita": "amanhã de manhã" })),
        ActionActor::Agent,
        Some("k2"),
        &context(),
    )
    .expect_err("texto livre não é data");

    assert_eq!(error.api_code(), "campos_invalidos");
    assert!(
        error.to_string().contains("esperava data ISO 8601"),
        "{error}"
    );
}

#[test]
fn pop_atende_closed_became_end_that_concludes_execution() {
    let ir = fluxo(
        "fechamento",
        vec![etapa(
            "fechamento",
            &[ActionActor::Agent],
            Vec::new(),
            vec![action_for_end(
                "concluir",
                None,
                "Atendimento concluído",
                false,
            )],
        )],
        Vec::new(),
    );
    let estado = start(&ir).expect("a execução começa");

    let resultado = execute_action(
        &ir,
        &estado,
        "concluir",
        &without_fields(),
        ActionActor::Agent,
        Some("k1"),
        &context(),
    )
    .expect("a ação é aceita");

    assert_eq!(resultado.new_state.status, ExecutionStatus::Completed);
    assert_eq!(
        resultado.new_state.resultado.as_deref(),
        Some("Atendimento concluído")
    );
    assert_eq!(resultado.new_state.motivo, None);
    assert_eq!(
        kinds(&resultado),
        vec![
            RecordEventKind::ActionExecuted,
            RecordEventKind::StepCompleted,
            RecordEventKind::ExecutionCompleted
        ]
    );
}

#[test]
fn end_only_asks_for_closure_when_it_declares_that() {
    let ir = fluxo(
        "fechamento",
        vec![etapa(
            "fechamento",
            &[ActionActor::Agent],
            Vec::new(),
            vec![
                action_for_end("concluir", None, "Concluído", false),
                action_for_end("encerrar_tudo", None, "Encerrado", true),
            ],
        )],
        Vec::new(),
    );
    let estado = start(&ir).expect("a execução começa");

    let without_close = execute_action(
        &ir,
        &estado,
        "concluir",
        &without_fields(),
        ActionActor::Agent,
        Some("k1"),
        &context(),
    )
    .expect("a ação é aceita");
    assert_eq!(without_close.new_state.status, ExecutionStatus::Completed);
    assert!(
        !without_close.ends_service,
        "concluir o fluxo não fecha o atendimento"
    );

    let closing = execute_action(
        &ir,
        &estado,
        "encerrar_tudo",
        &without_fields(),
        ActionActor::Agent,
        Some("k2"),
        &context(),
    )
    .expect("a ação é aceita");
    assert_eq!(closing.new_state.status, ExecutionStatus::Completed);
    assert!(
        closing.ends_service,
        "o fim que declara o encerramento pede o fechamento ao chamador"
    );
}

#[test]
fn closure_is_declared_not_deduced_from_step_name() {
    let mut closed = etapa(
        "encerrado",
        &[ActionActor::Agent],
        Vec::new(),
        vec![
            action_for_step("reabrir", None, &[], "abertura"),
            action_for_end("concluir", None, "Concluído", false),
        ],
    );
    closed.nome = "Encerrado".to_owned();
    let ir = fluxo(
        "abertura",
        vec![
            etapa(
                "abertura",
                &[ActionActor::Agent],
                Vec::new(),
                vec![action_for_step("seguir", None, &[], "encerrado")],
            ),
            closed,
        ],
        Vec::new(),
    );
    let estado = start(&ir).expect("a execução começa");

    let resultado = execute_action(
        &ir,
        &estado,
        "seguir",
        &without_fields(),
        ActionActor::Agent,
        Some("k1"),
        &context(),
    )
    .expect("a ação é aceita");

    assert_eq!(resultado.new_state.no_atual, goes_for("encerrado"));
    assert_eq!(
        resultado.new_state.status,
        ExecutionStatus::Active,
        "chegar numa etapa chamada `Encerrado` não encerra nada"
    );
    assert!(!resultado.ends_service);
}

#[test]
fn there_are_only_two_action_actors() {
    for accepted in ["agente", "atendente"] {
        assert!(
            ActionActor::from_text(accepted).is_ok(),
            "`{accepted}` é ator de ação"
        );
    }
    for refused in [
        "cliente", "client", "provider", "operator", "system", "sistema",
    ] {
        let error =
            ActionActor::from_text(refused).expect_err("ator do POP Atende que o Harness não tem");
        assert_eq!(error.api_code(), "validation_error");
        assert!(
            error.to_string().contains("aceitos: agente, atendente"),
            "`{refused}`: {error}"
        );
    }
}

#[test]
fn notification_and_forwarding_come_as_action_effect() {
    let mut notify = action_for_end("concluir", None, "Concluído", false);
    notify.effects = vec![
        Effect::NotifyWhatsApp {
            para: "5521999990000".to_owned(),
            mensagem: "O atendimento do cliente {cliente.peer} foi concluído".to_owned(),
        },
        Effect::Encaminhar {
            departamento: "financeiro".to_owned(),
        },
    ];
    let ir = fluxo(
        "coleta",
        vec![etapa(
            "coleta",
            &[ActionActor::Agent],
            Vec::new(),
            vec![notify],
        )],
        Vec::new(),
    );
    let evaluator = RuleEvaluator::without_functions();
    let context = Context::new(&evaluator).with_client("cli_ficticio");
    let estado = start(&ir).expect("a execução começa");

    let resultado = execute_action(
        &ir,
        &estado,
        "concluir",
        &without_fields(),
        ActionActor::Agent,
        Some("k1"),
        &context,
    )
    .expect("a ação é aceita");

    assert_eq!(resultado.effects.len(), 2);
    assert_eq!(resultado.effects[0].chave, "acao:coleta.concluir/0");
    assert_eq!(resultado.effects[0].tipo(), "notificar_whatsapp");
    assert_eq!(
        resultado.effects[0].effect,
        Effect::NotifyWhatsApp {
            para: "5521999990000".to_owned(),
            mensagem: "O atendimento do cliente cli_ficticio foi concluído".to_owned(),
        }
    );
    assert_eq!(resultado.effects[1].chave, "acao:coleta.concluir/1");
    assert_eq!(resultado.effects[1].tipo(), "encaminhar");
    assert_eq!(
        resultado.effects[1].effect,
        Effect::Encaminhar {
            departamento: "financeiro".to_owned(),
        }
    );
    let created = kinds(&resultado)
        .into_iter()
        .filter(|tipo| *tipo == RecordEventKind::EffectCreated)
        .count();
    assert_eq!(created, 2);
}

#[test]
fn step_entry_message_comes_out_when_execution_starts() {
    let ir = fluxo(
        "abertura",
        vec![
            step_with_entry(
                "abertura",
                &[ActionActor::Agent],
                Vec::new(),
                vec![action_for_end("fechar", None, "Feito", false)],
                vec![Effect::MessageToCustomer {
                    texto: "Olá, {cliente.peer}! Já vou te atender.".to_owned(),
                }],
            ),
            etapa(
                "fechamento",
                &[ActionActor::Agent],
                Vec::new(),
                vec![action_for_end("fechar", None, "Feito", false)],
            ),
        ],
        Vec::new(),
    );
    let evaluator = RuleEvaluator::without_functions();
    let context = Context::new(&evaluator).with_client("cli_ficticio");
    let estado = start(&ir).expect("a execução começa");

    let effects = entry_effects(&ir, "abertura", &estado.dados, &context).expect("a etapa existe");

    assert_eq!(effects.len(), 1);
    assert_eq!(effects[0].chave, "etapa:abertura/0");
    assert_eq!(
        effects[0].effect,
        Effect::MessageToCustomer {
            texto: "Olá, cli_ficticio! Já vou te atender.".to_owned(),
        }
    );
}

#[test]
fn missing_variable_in_text_renders_empty() {
    let ir = fluxo(
        "triagem",
        vec![step_with_entry(
            "triagem",
            &[ActionActor::Agent],
            Vec::new(),
            vec![action_for_end("fechar", None, "Feito", false)],
            vec![
                Effect::MessageToCustomer {
                    texto: "Endereço registrado: {triagem.cep}.".to_owned(),
                },
                Effect::MessageToCustomer {
                    texto: "Chave sem par: {triagem.cep".to_owned(),
                },
            ],
        )],
        Vec::new(),
    );
    let evaluator = RuleEvaluator::without_functions();
    let context = Context::new(&evaluator);
    let estado = start(&ir).expect("a execução começa");

    let effects = entry_effects(&ir, "triagem", &estado.dados, &context).expect("a etapa existe");

    assert_eq!(
        effects[0].effect,
        Effect::MessageToCustomer {
            texto: "Endereço registrado: .".to_owned(),
        },
        "a variável que não existe sai vazia, sem quebrar o texto"
    );
    assert_eq!(
        effects[1].effect,
        Effect::MessageToCustomer {
            texto: "Chave sem par: {triagem.cep".to_owned(),
        },
        "`{{` sem fechamento é texto do autor, não variável"
    );
}

#[test]
fn start_is_declared_in_flow_not_marked_on_step() {
    let ir = fluxo(
        "meio",
        vec![
            etapa(
                "primeira_nao_e_a_de_entrada",
                &[ActionActor::Agent],
                Vec::new(),
                vec![action_for_step("ir", None, &[], "meio")],
            ),
            etapa(
                "meio",
                &[ActionActor::Agent],
                Vec::new(),
                vec![action_for_step(
                    "ir",
                    None,
                    &[],
                    "primeira_nao_e_a_de_entrada",
                )],
            ),
        ],
        Vec::new(),
    );

    let estado = start(&ir).expect("a execução começa");

    assert_eq!(estado.no_atual, goes_for("meio"));
    assert_eq!(estado.status, ExecutionStatus::Active);
    assert!(estado.dados.is_empty());
    assert!(estado.actors_who_acted.is_empty());
}
