use poph_flows::engine::{Campos, Context, abandon, entry_effects, execute_action, start};
use poph_flows::ir::{
    Acao, Campo, Decisao, Destination, Effect, Etapa, Fim, Fluxo, Gatilho, Ir, Rule,
};
use poph_flows::rules::{Dados, RuleEvaluator};
use poph_types::{ActionActor, DomainError, ExecutionStatus, FieldKind, RecordEventKind};
use serde_json::{Value as Json, json};

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

fn action_for(
    id: &str,
    actor: Option<ActionActor>,
    target: Destination,
    effects: Vec<Effect>,
) -> Acao {
    Acao {
        id: id.to_owned(),
        label: id.to_uppercase(),
        requires: Vec::new(),
        actor,
        goes_to: target,
        effects,
    }
}

fn etapa(id: &str, campos: Vec<Campo>, actions: Vec<Acao>, enter: Vec<Effect>) -> Etapa {
    Etapa {
        id: id.to_owned(),
        nome: id.to_uppercase(),
        instruction: None,
        action_actors: vec![ActionActor::Agent],
        campos,
        actions,
        on_enter: enter,
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

fn decision_always(id: &str, then: Destination, r#else: Destination) -> Decisao {
    decisao(
        id,
        Rule::Vazio {
            campo: "coleta.nada".to_owned(),
        },
        then,
        r#else,
    )
}

fn goes_for(id: &str) -> Destination {
    Destination::Etapa { id: id.to_owned() }
}

fn goes_for_decision(id: &str) -> Destination {
    Destination::Decisao { id: id.to_owned() }
}

fn fim(id: &str, closes_service: bool) -> Destination {
    Destination::Fim {
        id: id.to_owned(),
        resultado: Some(format!("Resultado de {id}")),
        ends_service: closes_service,
    }
}

fn fluxo(steps: Vec<Etapa>, decisions: Vec<Decisao>) -> Ir {
    let beginning = steps
        .first()
        .map(|etapa| etapa.id.clone())
        .unwrap_or_default();
    Ir {
        schema_versao: poph_flows::ir::SCHEMA_VERSAO,
        fluxo: Fluxo {
            id: "teste".to_owned(),
            nome: "Teste".to_owned(),
            descricao: None,
        },
        gatilho: Gatilho {
            when: "o cliente pede".to_owned(),
            not_when: None,
            examples: Vec::new(),
        },
        start: beginning,
        stages: steps,
        decisions,
        ends: vec![Fim {
            id: "concluido".to_owned(),
            resultado: Some("Concluído".to_owned()),
            ends_service: false,
        }],
        defs: Vec::new(),
    }
}

fn campos(value: Json) -> Campos {
    value
        .as_object()
        .expect("o teste informa um objeto de campos")
        .clone()
}

fn without_fields() -> Campos {
    Campos::new()
}

fn kinds(resultado: &poph_flows::engine::Resultado) -> Vec<RecordEventKind> {
    resultado.events.iter().map(|evento| evento.tipo).collect()
}

fn motivo(error: &DomainError) -> String {
    error.to_string()
}

fn flow_with_decision(then: Destination, r#else: Destination) -> Ir {
    fluxo(
        vec![
            etapa(
                "coleta",
                vec![campo("marca", FieldKind::Text, false)],
                vec![action_for(
                    "ir",
                    None,
                    goes_for_decision("escolher"),
                    Vec::new(),
                )],
                Vec::new(),
            ),
            etapa("urgente", Vec::new(), Vec::new(), Vec::new()),
            etapa("normal", Vec::new(), Vec::new(), Vec::new()),
        ],
        vec![decisao(
            "escolher",
            Rule::Igual {
                campo: "coleta.marca".to_owned(),
                value: "urgente".to_owned(),
            },
            then,
            r#else,
        )],
    )
}

#[test]
fn decision_goes_to_true_branch() {
    let ir = flow_with_decision(goes_for("urgente"), goes_for("normal"));
    let estado = start(&ir).expect("início");
    let evaluator = RuleEvaluator::without_functions();

    let resultado = execute_action(
        &ir,
        &estado,
        "ir",
        &campos(json!({ "marca": "urgente" })),
        ActionActor::Agent,
        Some("chave-1"),
        &Context::new(&evaluator),
    )
    .expect("a ação é válida");

    assert_eq!(resultado.new_state.no_atual, goes_for("urgente"));
    assert_eq!(
        kinds(&resultado),
        vec![
            RecordEventKind::FieldsFilled,
            RecordEventKind::ActionExecuted,
            RecordEventKind::StepCompleted,
            RecordEventKind::DecisionResolved,
            RecordEventKind::StepEntered,
        ],
        "a decisão é resolvida na mesma passagem e a etapa de destino é a que entra"
    );
    assert_eq!(
        resultado.events[3].payload,
        json!({
            "decisao": "escolher",
            "resultado": true,
            "destino": goes_for("urgente"),
        }),
        "o evento diz a decisão, o resultado e o ramo"
    );
}

#[test]
fn decision_goes_to_false_branch() {
    let ir = flow_with_decision(goes_for("urgente"), goes_for("normal"));
    let estado = start(&ir).expect("início");
    let evaluator = RuleEvaluator::without_functions();

    let resultado = execute_action(
        &ir,
        &estado,
        "ir",
        &campos(json!({ "marca": "calmo" })),
        ActionActor::Agent,
        None,
        &Context::new(&evaluator),
    )
    .expect("a ação é válida");

    assert_eq!(resultado.new_state.no_atual, goes_for("normal"));
    assert_eq!(
        resultado.events[3].payload,
        json!({ "decisao": "escolher", "resultado": false, "destino": goes_for("normal") })
    );
}

#[test]
fn two_chained_decisions_arrive_at_step_in_one_pass() {
    let ir = fluxo(
        vec![
            etapa(
                "coleta",
                Vec::new(),
                vec![action_for(
                    "ir",
                    None,
                    goes_for_decision("primeira"),
                    Vec::new(),
                )],
                Vec::new(),
            ),
            etapa("chegada", Vec::new(), Vec::new(), Vec::new()),
        ],
        vec![
            decision_always(
                "primeira",
                goes_for_decision("segunda"),
                goes_for("chegada"),
            ),
            decision_always("segunda", goes_for("chegada"), goes_for("chegada")),
        ],
    );
    let estado = start(&ir).expect("início");
    let evaluator = RuleEvaluator::without_functions();

    let resultado = execute_action(
        &ir,
        &estado,
        "ir",
        &without_fields(),
        ActionActor::Agent,
        None,
        &Context::new(&evaluator),
    )
    .expect("a ação é válida");

    assert_eq!(resultado.new_state.no_atual, goes_for("chegada"));
    let resolvidas: Vec<&Json> = resultado
        .events
        .iter()
        .filter(|evento| evento.tipo == RecordEventKind::DecisionResolved)
        .map(|evento| &evento.payload)
        .collect();
    assert_eq!(
        resolvidas.len(),
        2,
        "as duas decisões viram evento, na ordem: {resolvidas:?}"
    );
    assert_eq!(resolvidas[0]["decisao"], json!("primeira"));
    assert_eq!(resolvidas[1]["decisao"], json!("segunda"));
}

#[test]
fn decision_cycle_puts_execution_in_failure() {
    let ir = fluxo(
        vec![etapa(
            "coleta",
            Vec::new(),
            vec![action_for("ir", None, goes_for_decision("a"), Vec::new())],
            Vec::new(),
        )],
        vec![
            decision_always("a", goes_for_decision("b"), goes_for_decision("b")),
            decision_always("b", goes_for_decision("a"), goes_for_decision("a")),
        ],
    );
    let estado = start(&ir).expect("início");
    let evaluator = RuleEvaluator::without_functions();

    let resultado = execute_action(
        &ir,
        &estado,
        "ir",
        &without_fields(),
        ActionActor::Agent,
        None,
        &Context::new(&evaluator),
    )
    .expect("a passagem volta com o desfecho, não como erro da chamada");

    assert_eq!(
        resultado.new_state.status,
        ExecutionStatus::Failed,
        "ciclo de decisões põe a execução em falha"
    );
    let stored = resultado
        .new_state
        .motivo
        .clone()
        .expect("o motivo fica no estado");
    assert!(stored.contains("ciclo de decisões"), "{stored}");
    assert!(
        stored.contains("a → b → a"),
        "o motivo mostra o caminho: {stored}"
    );
    assert_eq!(
        kinds(&resultado).last(),
        Some(&RecordEventKind::ExecutionFailed),
        "o último evento é a falha"
    );
    assert_eq!(
        resultado.events.last().expect("evento").payload["motivo"],
        json!(stored),
        "o evento carrega o mesmo motivo"
    );
    assert!(resultado.effects.is_empty());
}

#[test]
fn ten_jumps_pass_and_eleven_crash_execution() {
    fn cadeia(total: usize) -> Ir {
        let mut decisions = Vec::new();
        for index in 0..total {
            let seguinte = if index + 1 == total {
                goes_for("chegada")
            } else {
                goes_for_decision(&format!("d{}", index + 1))
            };
            decisions.push(decision_always(
                &format!("d{index}"),
                seguinte.clone(),
                seguinte,
            ));
        }
        fluxo(
            vec![
                etapa(
                    "coleta",
                    Vec::new(),
                    vec![action_for("ir", None, goes_for_decision("d0"), Vec::new())],
                    Vec::new(),
                ),
                etapa("chegada", Vec::new(), Vec::new(), Vec::new()),
            ],
            decisions,
        )
    }

    let evaluator = RuleEvaluator::without_functions();

    let ir = cadeia(10);
    let estado = start(&ir).expect("início");
    let dez = execute_action(
        &ir,
        &estado,
        "ir",
        &without_fields(),
        ActionActor::Agent,
        None,
        &Context::new(&evaluator),
    )
    .expect("dez saltos cabem no teto");
    assert_eq!(dez.new_state.status, ExecutionStatus::Active);
    assert_eq!(dez.new_state.no_atual, goes_for("chegada"));
    assert_eq!(
        dez.events
            .iter()
            .filter(|evento| evento.tipo == RecordEventKind::DecisionResolved)
            .count(),
        10
    );

    let ir = cadeia(11);
    let estado = start(&ir).expect("início");
    let onze = execute_action(
        &ir,
        &estado,
        "ir",
        &without_fields(),
        ActionActor::Agent,
        None,
        &Context::new(&evaluator),
    )
    .expect("a passagem volta com o desfecho");
    assert_eq!(onze.new_state.status, ExecutionStatus::Failed);
    let stored = onze.new_state.motivo.clone().expect("motivo");
    assert!(stored.contains("10 decisões encadeadas"), "{stored}");
    assert!(
        onze.effects.is_empty(),
        "a falha não enfileira efeito nenhum"
    );
}

#[test]
fn jump_cap_comes_from_context() {
    let ir = fluxo(
        vec![
            etapa(
                "coleta",
                Vec::new(),
                vec![action_for("ir", None, goes_for_decision("d0"), Vec::new())],
                Vec::new(),
            ),
            etapa("chegada", Vec::new(), Vec::new(), Vec::new()),
        ],
        vec![
            decision_always("d0", goes_for_decision("d1"), goes_for_decision("d1")),
            decision_always("d1", goes_for_decision("d2"), goes_for_decision("d2")),
            decision_always("d2", goes_for_decision("d3"), goes_for_decision("d3")),
            decision_always("d3", goes_for("chegada"), goes_for("chegada")),
        ],
    );
    let estado = start(&ir).expect("início");
    let evaluator = RuleEvaluator::without_functions();

    let with_cap_three = execute_action(
        &ir,
        &estado,
        "ir",
        &without_fields(),
        ActionActor::Agent,
        None,
        &Context::new(&evaluator).with_jump_limit(3),
    )
    .expect("a passagem volta com o desfecho");
    assert_eq!(with_cap_three.new_state.status, ExecutionStatus::Failed);

    let with_cap_four = execute_action(
        &ir,
        &estado,
        "ir",
        &without_fields(),
        ActionActor::Agent,
        None,
        &Context::new(&evaluator).with_jump_limit(4),
    )
    .expect("a passagem volta com o desfecho");
    assert_eq!(with_cap_four.new_state.status, ExecutionStatus::Active);
    assert_eq!(with_cap_four.new_state.no_atual, goes_for("chegada"));
}

#[test]
fn decision_that_points_for_step_missing_and_ir_divergent() {
    let ir = flow_with_decision(goes_for("fantasma"), goes_for("normal"));
    let estado = start(&ir).expect("início");
    let evaluator = RuleEvaluator::without_functions();

    let error = execute_action(
        &ir,
        &estado,
        "ir",
        &campos(json!({ "marca": "urgente" })),
        ActionActor::Agent,
        None,
        &Context::new(&evaluator),
    )
    .expect_err("o destino do ramo não existe na estrutura");
    assert!(
        matches!(error, DomainError::IrDivergent { .. }),
        "{error:?}"
    );
    assert!(motivo(&error).contains("fantasma"), "{}", motivo(&error));
}

#[test]
fn rule_that_does_not_evaluate_puts_execution_in_failure() {
    let ir = fluxo(
        vec![
            etapa(
                "coleta",
                Vec::new(),
                vec![action_for(
                    "ir",
                    None,
                    goes_for_decision("escolher"),
                    Vec::new(),
                )],
                Vec::new(),
            ),
            etapa("normal", Vec::new(), Vec::new(), Vec::new()),
        ],
        vec![decisao(
            "escolher",
            Rule::Funcao {
                nome: "grave".to_owned(),
            },
            goes_for("normal"),
            goes_for("normal"),
        )],
    );
    let estado = start(&ir).expect("início");
    let evaluator = RuleEvaluator::without_functions();

    let resultado = execute_action(
        &ir,
        &estado,
        "ir",
        &without_fields(),
        ActionActor::Agent,
        None,
        &Context::new(&evaluator),
    )
    .expect("a passagem volta com o desfecho");

    assert_eq!(resultado.new_state.status, ExecutionStatus::Failed);
    let stored = resultado.new_state.motivo.clone().expect("motivo");
    assert!(stored.contains("regra inválida"), "{stored}");
    assert_eq!(
        resultado.new_state.no_atual,
        goes_for_decision("escolher"),
        "a execução para na decisão que não pôde avaliar"
    );
    assert_eq!(
        kinds(&resultado).last(),
        Some(&RecordEventKind::ExecutionFailed)
    );
}

#[test]
fn end_without_closing_service_concludes_execution() {
    let ir = fluxo(
        vec![etapa(
            "coleta",
            Vec::new(),
            vec![action_for(
                "concluir",
                None,
                fim("concluido", false),
                Vec::new(),
            )],
            Vec::new(),
        )],
        Vec::new(),
    );
    let estado = start(&ir).expect("início");
    let evaluator = RuleEvaluator::without_functions();

    let resultado = execute_action(
        &ir,
        &estado,
        "concluir",
        &without_fields(),
        ActionActor::Agent,
        None,
        &Context::new(&evaluator),
    )
    .expect("a ação é válida");

    assert_eq!(resultado.new_state.status, ExecutionStatus::Completed);
    assert_eq!(
        resultado.new_state.resultado,
        Some("Resultado de concluido".to_owned())
    );
    assert!(
        !resultado.ends_service,
        "sem `encerra_atendimento`, a conversa continua com o agente"
    );
    assert_eq!(
        kinds(&resultado),
        vec![
            RecordEventKind::ActionExecuted,
            RecordEventKind::StepCompleted,
            RecordEventKind::ExecutionCompleted,
        ]
    );
    assert_eq!(
        resultado.events[2].payload,
        json!({
            "fim": "concluido",
            "resultado": "Resultado de concluido",
            "encerra_atendimento": false,
        })
    );
}

#[test]
fn end_with_ends_service_asks_for_closure() {
    let ir = fluxo(
        vec![etapa(
            "coleta",
            Vec::new(),
            vec![action_for(
                "concluir",
                None,
                fim("concluido", true),
                Vec::new(),
            )],
            Vec::new(),
        )],
        Vec::new(),
    );
    let estado = start(&ir).expect("início");
    let evaluator = RuleEvaluator::without_functions();

    let resultado = execute_action(
        &ir,
        &estado,
        "concluir",
        &without_fields(),
        ActionActor::Agent,
        None,
        &Context::new(&evaluator),
    )
    .expect("a ação é válida");

    assert_eq!(resultado.new_state.status, ExecutionStatus::Completed);
    assert!(
        resultado.ends_service,
        "o fim pede o encerramento; quem fecha é o chamador"
    );
    assert_eq!(
        resultado.events.last().expect("evento").payload["encerra_atendimento"],
        json!(true)
    );
}

#[test]
fn decision_leading_to_end_also_concludes() {
    let ir = flow_with_decision(fim("concluido", true), goes_for("normal"));
    let estado = start(&ir).expect("início");
    let evaluator = RuleEvaluator::without_functions();

    let resultado = execute_action(
        &ir,
        &estado,
        "ir",
        &campos(json!({ "marca": "urgente" })),
        ActionActor::Agent,
        None,
        &Context::new(&evaluator),
    )
    .expect("a ação é válida");

    assert_eq!(resultado.new_state.status, ExecutionStatus::Completed);
    assert!(resultado.ends_service);
    assert_eq!(
        kinds(&resultado),
        vec![
            RecordEventKind::FieldsFilled,
            RecordEventKind::ActionExecuted,
            RecordEventKind::StepCompleted,
            RecordEventKind::DecisionResolved,
            RecordEventKind::ExecutionCompleted,
        ]
    );
}

fn flow_with_effects() -> Ir {
    fluxo(
        vec![
            etapa(
                "coleta",
                vec![campo("endereco", FieldKind::Text, true)],
                vec![action_for(
                    "confirmar",
                    None,
                    goes_for("confirmacao"),
                    vec![
                        Effect::MessageToCustomer {
                            texto: "Pedido para {coleta.endereco}, do contato {cliente.peer}"
                                .to_owned(),
                        },
                        Effect::NotifyWhatsApp {
                            para: "5521999990000".to_owned(),
                            mensagem: "Ficha {ficha.id}: {coleta.endereco}".to_owned(),
                        },
                        Effect::Encaminhar {
                            departamento: "Logística".to_owned(),
                        },
                    ],
                )],
                Vec::new(),
            ),
            etapa(
                "confirmacao",
                Vec::new(),
                Vec::new(),
                vec![Effect::MessageToCustomer {
                    texto: "Confirmando o pedido.".to_owned(),
                }],
            ),
        ],
        Vec::new(),
    )
}

#[test]
fn effects_come_in_ir_order_with_resolved_text() {
    let ir = flow_with_effects();
    let estado = start(&ir).expect("início");
    let evaluator = RuleEvaluator::without_functions();
    let context = Context::new(&evaluator)
        .with_case("ficha-1")
        .with_client("5521900000000");

    let resultado = execute_action(
        &ir,
        &estado,
        "confirmar",
        &campos(json!({ "endereco": "Rua das Flores, 10" })),
        ActionActor::Agent,
        None,
        &context,
    )
    .expect("a ação é válida");

    assert_eq!(resultado.effects.len(), 4, "três da ação e um da entrada");
    assert_eq!(
        resultado
            .effects
            .iter()
            .map(|effect| effect.chave.as_str())
            .collect::<Vec<_>>(),
        vec![
            "acao:coleta.confirmar/0",
            "acao:coleta.confirmar/1",
            "acao:coleta.confirmar/2",
            "etapa:confirmacao/0",
        ],
        "a ordem é a declarada, e a chave diz o nó e a posição"
    );
    assert_eq!(
        resultado.effects[0].effect,
        Effect::MessageToCustomer {
            texto: "Pedido para Rua das Flores, 10, do contato 5521900000000".to_owned()
        }
    );
    assert_eq!(
        resultado.effects[1].effect,
        Effect::NotifyWhatsApp {
            para: "5521999990000".to_owned(),
            mensagem: "Ficha ficha-1: Rua das Flores, 10".to_owned()
        }
    );
    assert_eq!(
        resultado.effects[2].effect,
        Effect::Encaminhar {
            departamento: "Logística".to_owned()
        }
    );
    assert_eq!(resultado.effects[3].tipo(), "mensagem_ao_cliente");
    assert_eq!(
        resultado
            .events
            .iter()
            .filter(|evento| evento.tipo == RecordEventKind::EffectCreated)
            .count(),
        4,
        "cada efeito entra na fila com o seu evento"
    );
}

#[test]
fn effect_key_stable_for_same_input() {
    let ir = flow_with_effects();
    let estado = start(&ir).expect("início");
    let evaluator = RuleEvaluator::without_functions();
    let context = Context::new(&evaluator).with_case("ficha-1");
    let entry = campos(json!({ "endereco": "Rua A" }));

    let first_result = execute_action(
        &ir,
        &estado,
        "confirmar",
        &entry,
        ActionActor::Agent,
        None,
        &context,
    )
    .expect("primeira");
    let second_result = execute_action(
        &ir,
        &estado,
        "confirmar",
        &entry,
        ActionActor::Agent,
        None,
        &context,
    )
    .expect("segunda");

    assert_eq!(
        first_result.effects, second_result.effects,
        "mesma entrada, mesmas chaves"
    );
}

#[test]
fn effect_variable_that_does_not_exist_becomes_empty_text() {
    let ir = fluxo(
        vec![etapa(
            "coleta",
            Vec::new(),
            vec![action_for(
                "confirmar",
                None,
                fim("concluido", false),
                vec![Effect::MessageToCustomer {
                    texto: "Pedido para {coleta.endereco}.".to_owned(),
                }],
            )],
            Vec::new(),
        )],
        Vec::new(),
    );
    let estado = start(&ir).expect("início");
    let evaluator = RuleEvaluator::without_functions();

    let resultado = execute_action(
        &ir,
        &estado,
        "confirmar",
        &without_fields(),
        ActionActor::Agent,
        None,
        &Context::new(&evaluator),
    )
    .expect("a ação é válida");

    assert_eq!(
        resultado.effects[0].effect,
        Effect::MessageToCustomer {
            texto: "Pedido para .".to_owned()
        },
        "variável inexistente sai vazia (FLX59 avisa na publicação)"
    );
}

#[test]
fn customer_data_enters_as_value_in_effect_text() {
    let ir = fluxo(
        vec![etapa(
            "coleta",
            vec![campo("endereco", FieldKind::Text, true)],
            vec![action_for(
                "confirmar",
                None,
                fim("concluido", false),
                vec![Effect::MessageToCustomer {
                    texto: "Pedido: {coleta.endereco}".to_owned(),
                }],
            )],
            Vec::new(),
        )],
        Vec::new(),
    );
    let estado = start(&ir).expect("início");
    let evaluator = RuleEvaluator::without_functions();

    let resultado = execute_action(
        &ir,
        &estado,
        "confirmar",
        &campos(json!({ "endereco": "  Rua A  " })),
        ActionActor::Agent,
        None,
        &Context::new(&evaluator),
    )
    .expect("a ação é válida");

    assert_eq!(
        resultado.effects[0].effect,
        Effect::MessageToCustomer {
            texto: "Pedido: Rua A".to_owned()
        },
        "o texto canônico do campo entra como valor"
    );
}

#[test]
fn entry_effects_of_first_step_come_from_public_function() {
    let ir = fluxo(
        vec![etapa(
            "abertura",
            Vec::new(),
            Vec::new(),
            vec![Effect::NotifyWhatsApp {
                para: "5521999990000".to_owned(),
                mensagem: "Atendimento {ficha.id} começou".to_owned(),
            }],
        )],
        Vec::new(),
    );
    let estado = start(&ir).expect("início");
    let evaluator = RuleEvaluator::without_functions();
    let context = Context::new(&evaluator).with_case("ficha-9");
    let dados: Dados = estado.dados.clone();

    let effects = entry_effects(&ir, "abertura", &dados, &context).expect("etapa existe");
    assert_eq!(effects.len(), 1);
    assert_eq!(effects[0].chave, "etapa:abertura/0");
    assert_eq!(
        effects[0].effect,
        Effect::NotifyWhatsApp {
            para: "5521999990000".to_owned(),
            mensagem: "Atendimento ficha-9 começou".to_owned()
        }
    );

    let error = entry_effects(&ir, "fantasma", &dados, &context).expect_err("etapa não existe");
    assert!(
        matches!(error, DomainError::IrDivergent { .. }),
        "{error:?}"
    );
}

#[test]
fn step_with_entry_effect_becomes_effect_on_advance() {
    let ir = flow_with_effects();
    let estado = start(&ir).expect("início");
    let evaluator = RuleEvaluator::without_functions();

    let resultado = execute_action(
        &ir,
        &estado,
        "confirmar",
        &campos(json!({ "endereco": "Rua B" })),
        ActionActor::Agent,
        None,
        &Context::new(&evaluator),
    )
    .expect("a ação é válida");

    assert_eq!(resultado.new_state.no_atual, goes_for("confirmacao"));
    assert_eq!(
        resultado.effects.last().expect("efeito").chave,
        "etapa:confirmacao/0"
    );
    assert_eq!(
        resultado.events.last().expect("evento").tipo,
        RecordEventKind::EffectCreated,
        "o efeito da etapa nova é o último evento da passagem"
    );
}

#[test]
fn abandonment_marks_execution_and_produces_no_effect() {
    let ir = flow_with_effects();
    let estado = start(&ir).expect("início");

    let resultado = abandon(
        &estado,
        "  o cliente pediu para parar  ",
        ActionActor::Agent,
        None,
    )
    .expect("a execução está ativa");

    assert_eq!(resultado.new_state.status, ExecutionStatus::Abandoned);
    assert_eq!(
        resultado.new_state.motivo,
        Some("o cliente pediu para parar".to_owned()),
        "o motivo é gravado sem os espaços das pontas"
    );
    assert!(resultado.effects.is_empty(), "abandono não dispara efeito");
    assert!(resultado.waiting.is_empty());
    assert!(!resultado.ends_service);
    assert_eq!(kinds(&resultado), vec![RecordEventKind::ExecutionAbandoned]);
    assert_eq!(
        resultado.events[0].payload,
        json!({ "motivo": "o cliente pediu para parar", "no": "coleta" })
    );
}

#[test]
fn abandonment_without_reason_refused_and_closed_execution_is_not_abandoned_again() {
    let ir = flow_with_effects();
    let estado = start(&ir).expect("início");

    let error = abandon(&estado, "   ", ActionActor::Agent, None).expect_err("motivo vazio");
    assert!(matches!(error, DomainError::Validation { .. }), "{error:?}");

    let abandoned = abandon(&estado, "desistiu", ActionActor::Attendant, None)
        .expect("abandona")
        .new_state;
    let error =
        abandon(&abandoned, "de novo", ActionActor::Agent, None).expect_err("já está abandonada");
    assert!(matches!(error, DomainError::Conflict { .. }), "{error:?}");
}

#[test]
fn repeating_key_duplicates_neither_event_nor_effect() {
    let ir = flow_with_effects();
    let estado = start(&ir).expect("início");
    let evaluator = RuleEvaluator::without_functions();
    let context = Context::new(&evaluator).with_case("ficha-1");
    let entry = campos(json!({ "endereco": "Rua C" }));

    let first_result = execute_action(
        &ir,
        &estado,
        "confirmar",
        &entry,
        ActionActor::Agent,
        Some("chave-1"),
        &context,
    )
    .expect("primeira");

    let second_result = execute_action(
        &ir,
        &first_result.new_state,
        "confirmar",
        &entry,
        ActionActor::Agent,
        Some("chave-1"),
        &context,
    )
    .expect("repetida");

    assert!(second_result.repeated);
    assert!(second_result.events.is_empty(), "repetir não grava evento");
    assert!(
        second_result.effects.is_empty(),
        "repetir não enfileira efeito"
    );
    assert!(!second_result.ends_service, "repetir não pede encerramento");
    assert_eq!(
        second_result.new_state, first_result.new_state,
        "o estado não muda na repetição"
    );
    assert_eq!(first_result.new_state.keys.len(), 1);
}

#[test]
fn same_input_with_decision_and_effect_gives_same_result() {
    let ir = flow_with_effects();
    let estado = start(&ir).expect("início");
    let evaluator = RuleEvaluator::without_functions();
    let context = Context::new(&evaluator)
        .with_case("ficha-1")
        .with_client("5521900000000");
    let entry = campos(json!({ "endereco": "Rua D" }));

    let first_result = execute_action(
        &ir,
        &estado,
        "confirmar",
        &entry,
        ActionActor::Agent,
        Some("chave"),
        &context,
    )
    .expect("primeira");
    let second_result = execute_action(
        &ir,
        &estado,
        "confirmar",
        &entry,
        ActionActor::Agent,
        Some("chave"),
        &context,
    )
    .expect("segunda");

    assert_eq!(first_result, second_result);
    assert!(first_result.ends_service == second_result.ends_service);
}

fn flow_with_calendar_effects() -> Ir {
    fluxo(
        vec![
            etapa(
                "coleta",
                vec![
                    campo("data", FieldKind::Text, true),
                    campo("fim", FieldKind::Text, false),
                    campo("endereco", FieldKind::Text, false),
                    campo("executor", FieldKind::Text, false),
                ],
                vec![action_for(
                    "confirmar",
                    None,
                    goes_for("fim_visita"),
                    vec![Effect::SyncCalendar {
                        operation: "create".to_owned(),
                        start: Some("{coleta.data}".to_owned()),
                        end: Some("{coleta.fim}".to_owned()),
                        title: Some("Visita {ficha.id}".to_owned()),
                        location: Some("{coleta.endereco}".to_owned()),
                        executor: Some("{coleta.executor}".to_owned()),
                    }],
                )],
                Vec::new(),
            ),
            etapa(
                "fim_visita",
                Vec::new(),
                Vec::new(),
                vec![Effect::SyncCalendar {
                    operation: "update".to_owned(),
                    start: Some("{coleta.data}".to_owned()),
                    end: None,
                    title: None,
                    location: None,
                    executor: None,
                }],
            ),
        ],
        Vec::new(),
    )
}

#[test]
fn calendar_effect_is_interpolated_and_keyed_on_action_and_entry() {
    let ir = flow_with_calendar_effects();
    let estado = start(&ir).expect("início");
    let evaluator = RuleEvaluator::without_functions();
    let context = Context::new(&evaluator)
        .with_case("ficha-42")
        .with_client("5521900000000");

    let resultado = execute_action(
        &ir,
        &estado,
        "confirmar",
        &campos(json!({
            "data": "2027-01-10T18:00:00-03:00",
            "fim": "2027-01-10T19:00:00-03:00",
            "endereco": "Rua das Flores, 10",
            "executor": "0199a11d-2000-7000-8000-000000000001",
        })),
        ActionActor::Agent,
        None,
        &context,
    )
    .expect("a ação é válida");

    assert_eq!(
        resultado
            .effects
            .iter()
            .map(|effect| (effect.chave.as_str(), effect.tipo()))
            .collect::<Vec<_>>(),
        vec![
            ("acao:coleta.confirmar/0", "sincronizar_agenda"),
            ("etapa:fim_visita/0", "sincronizar_agenda"),
        ]
    );
    assert_eq!(
        resultado.effects[0].effect,
        Effect::SyncCalendar {
            operation: "create".to_owned(),
            start: Some("2027-01-10T18:00:00-03:00".to_owned()),
            end: Some("2027-01-10T19:00:00-03:00".to_owned()),
            title: Some("Visita ficha-42".to_owned()),
            location: Some("Rua das Flores, 10".to_owned()),
            executor: Some("0199a11d-2000-7000-8000-000000000001".to_owned()),
        }
    );
    assert_eq!(
        resultado.effects[1].effect,
        Effect::SyncCalendar {
            operation: "update".to_owned(),
            start: Some("2027-01-10T18:00:00-03:00".to_owned()),
            end: None,
            title: None,
            location: None,
            executor: None,
        }
    );
    let json = serde_json::to_value(&resultado.effects[1].effect).expect("JSON do efeito");
    assert_eq!(json["tipo"], json!("sincronizar_agenda"));
    assert_eq!(json["fim"], Json::Null, "opcional ausente sai como null");
    assert_eq!(
        resultado
            .events
            .iter()
            .filter(|evento| evento.tipo == RecordEventKind::EffectCreated)
            .count(),
        2,
        "cada efeito entra na fila com o seu evento"
    );
}
