use poph_fluxos::ir::{Acao, Campo, Destination, Etapa, Fim, Fluxo, Gatilho, Ir};
use poph_fluxos::motor::{
    Campos, Context, Estado, execute_action, missing_fields, required_actors, start,
};
use poph_fluxos::regras::RuleEvaluator;
use poph_types::{ActionActor, DomainError, ExecutionStatus, FieldKind, RecordEventKind};
use serde_json::{Value as Json, json};
use std::sync::OnceLock;

fn context() -> Context<'static> {
    static EVALUATOR: OnceLock<RuleEvaluator> = OnceLock::new();
    Context::new(EVALUATOR.get_or_init(RuleEvaluator::without_functions))
}

fn campo(id: &str, tipo: FieldKind, required: bool, options: &[&str]) -> Campo {
    Campo {
        id: id.to_owned(),
        label: id.to_uppercase(),
        tipo,
        required,
        options: options.iter().map(|option| (*option).to_owned()).collect(),
        help: None,
    }
}

fn acao(id: &str, actor: Option<ActionActor>, requires: &[&str], goes_for: &str) -> Acao {
    Acao {
        id: id.to_owned(),
        label: id.to_uppercase(),
        requires: requires.iter().map(|nome| (*nome).to_owned()).collect(),
        actor,
        goes_to: Destination::Etapa {
            id: goes_for.to_owned(),
        },
        effects: Vec::new(),
    }
}

fn action_for_end(id: &str, actor: Option<ActionActor>) -> Acao {
    Acao {
        id: id.to_owned(),
        label: id.to_uppercase(),
        requires: Vec::new(),
        actor,
        goes_to: Destination::Fim {
            id: "concluido".to_owned(),
            resultado: Some("Concluído".to_owned()),
            ends_service: false,
        },
        effects: Vec::new(),
    }
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

fn fluxo(steps: Vec<Etapa>) -> Ir {
    let beginning = steps
        .first()
        .map(|etapa| etapa.id.clone())
        .unwrap_or_default();
    Ir {
        schema_versao: poph_fluxos::ir::SCHEMA_VERSAO,
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
        decisions: Vec::new(),
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

fn flow_simple() -> Ir {
    fluxo(vec![
        etapa(
            "coleta",
            &[ActionActor::Agent],
            vec![campo("endereco", FieldKind::Text, true, &[])],
            vec![
                acao("confirmar", None, &["endereco"], "confirmacao"),
                acao("desistir", None, &[], "confirmacao"),
            ],
        ),
        etapa(
            "confirmacao",
            &[ActionActor::Agent],
            Vec::new(),
            vec![action_for_end("ok", None)],
        ),
    ])
}

fn flow_with_two_actors() -> Ir {
    fluxo(vec![
        etapa(
            "triagem",
            &[ActionActor::Agent, ActionActor::Attendant],
            vec![campo("resumo", FieldKind::Text, false, &[])],
            vec![
                acao("registrar", Some(ActionActor::Agent), &[], "depois"),
                acao("assumir", Some(ActionActor::Attendant), &[], "depois"),
            ],
        ),
        etapa(
            "depois",
            &[ActionActor::Agent],
            Vec::new(),
            vec![action_for_end("ok", None)],
        ),
    ])
}

fn motivo(error: &DomainError) -> String {
    match error {
        DomainError::InvalidAction { reason: motivo, .. } => motivo.clone(),
        other => panic!("esperava acao_invalida e veio {other:?}"),
    }
}

fn errors_fields(error: &DomainError) -> Vec<String> {
    match error {
        DomainError::InvalidStepFields { errors, .. } => errors.clone(),
        other => panic!("esperava campos_invalidos e veio {other:?}"),
    }
}

fn kinds(resultado: &poph_fluxos::motor::Resultado) -> Vec<RecordEventKind> {
    resultado.events.iter().map(|evento| evento.tipo).collect()
}

#[test]
fn start_begins_at_start_step_with_empty_state() {
    let ir = flow_simple();
    let estado = start(&ir).expect("a etapa de início existe");

    assert_eq!(estado.fluxo_id, "teste");
    assert_eq!(estado.status, ExecutionStatus::Active);
    assert_eq!(
        estado.no_atual,
        Destination::Etapa {
            id: "coleta".to_owned()
        }
    );
    assert!(estado.dados.is_empty(), "a execução nova não tem dado");
    assert!(estado.actors_who_acted.is_empty());
    assert!(estado.keys.is_empty());
    assert_eq!(estado.resultado, None);
}

#[test]
fn start_refuses_ir_pointing_to_missing_step() {
    let mut ir = flow_simple();
    ir.start = "nao_existe".to_owned();

    let error = start(&ir).expect_err("a etapa de início não existe");
    assert!(
        matches!(error, DomainError::IrDivergent { .. }),
        "esperava ir_divergente e veio {error:?}"
    );
    assert_eq!(error.api_code(), "ir_divergente");
}

#[test]
fn action_validates_records_fields_and_advances_for_step_target() {
    let ir = flow_simple();
    let estado = start(&ir).expect("início");

    let resultado = execute_action(
        &ir,
        &estado,
        "confirmar",
        &campos(json!({ "endereco": "Rua das Flores, 10" })),
        ActionActor::Agent,
        None,
        &context(),
    )
    .expect("a ação é válida");

    assert_eq!(
        resultado.new_state.no_atual,
        Destination::Etapa {
            id: "confirmacao".to_owned()
        }
    );
    assert_eq!(
        resultado.new_state.dados.get("coleta"),
        Some(&json!({ "endereco": "Rua das Flores, 10" })),
        "os campos ficam em dados[etapa][campo]"
    );
    assert!(
        resultado.new_state.actors_who_acted.is_empty(),
        "a etapa nova começa sem ator que agiu"
    );
    assert!(resultado.waiting.is_empty());
    assert!(!resultado.repeated);
    assert!(resultado.effects.is_empty(), "efeitos são da FX-14");
    assert_eq!(
        kinds(&resultado),
        vec![
            RecordEventKind::FieldsFilled,
            RecordEventKind::ActionExecuted,
            RecordEventKind::StepCompleted,
            RecordEventKind::StepEntered,
        ]
    );
    assert_eq!(
        resultado.events[3].payload,
        json!({ "etapa": "confirmacao" }),
        "o evento de entrada diz a etapa nova"
    );
}

#[test]
fn data_from_previous_steps_remains_in_execution() {
    let ir = flow_simple();
    let mut estado = start(&ir).expect("início");
    estado = execute_action(
        &ir,
        &estado,
        "confirmar",
        &campos(json!({ "endereco": "Rua A" })),
        ActionActor::Agent,
        None,
        &context(),
    )
    .expect("primeira ação")
    .new_state;

    let resultado = execute_action(
        &ir,
        &estado,
        "ok",
        &without_fields(),
        ActionActor::Agent,
        None,
        &context(),
    )
    .expect("segunda ação");

    assert_eq!(
        resultado.new_state.dados.get("coleta"),
        Some(&json!({ "endereco": "Rua A" })),
        "o dado da etapa anterior não é perdido"
    );
    assert_eq!(
        resultado.new_state.no_atual,
        Destination::Fim {
            id: "concluido".to_owned(),
            resultado: Some("Concluído".to_owned()),
            ends_service: false,
        },
        "ação que vai para fim deixa o estado no nó do fim (a FX-14 conclui)"
    );
}

#[test]
fn field_text_stored_without_edge_spaces() {
    let ir = flow_simple();
    let estado = start(&ir).expect("início");

    let resultado = execute_action(
        &ir,
        &estado,
        "confirmar",
        &campos(json!({ "endereco": "  Rua B  " })),
        ActionActor::Agent,
        None,
        &context(),
    )
    .expect("a ação é válida");

    assert_eq!(
        resultado.new_state.dados.get("coleta"),
        Some(&json!({ "endereco": "Rua B" }))
    );
}

#[test]
fn required_field_missing_returns_list_of_what_is_missing() {
    let ir = flow_simple();
    let estado = start(&ir).expect("início");

    let error = execute_action(
        &ir,
        &estado,
        "confirmar",
        &without_fields(),
        ActionActor::Agent,
        None,
        &context(),
    )
    .expect_err("`endereco` é exigido");

    let errors = errors_fields(&error);
    assert_eq!(error.api_code(), "campos_invalidos");
    assert!(
        errors
            .iter()
            .any(|error| error.contains("`confirmar`") && error.contains("`endereco`")),
        "a mensagem diz a ação e o campo: {errors:?}"
    );
}

#[test]
fn requires_field_step_does_not_declare_warns_that() {
    let ir = fluxo(vec![
        etapa(
            "coleta",
            &[ActionActor::Agent],
            Vec::new(),
            vec![acao("confirmar", None, &["cep"], "depois")],
        ),
        etapa(
            "depois",
            &[ActionActor::Agent],
            Vec::new(),
            vec![action_for_end("ok", None)],
        ),
    ]);
    let estado = start(&ir).expect("início");

    let error = execute_action(
        &ir,
        &estado,
        "confirmar",
        &without_fields(),
        ActionActor::Agent,
        None,
        &context(),
    )
    .expect_err("`cep` não existe na etapa");

    let errors = errors_fields(&error);
    assert!(
        errors[0].contains("`cep`") && errors[0].contains("não tem"),
        "a mensagem diz que a etapa não declara o campo: {errors:?}"
    );
}

#[test]
fn field_informed_before_counts_for_requires_of_next_action() {
    let ir = fluxo(vec![
        etapa(
            "coleta",
            &[ActionActor::Agent],
            vec![campo("endereco", FieldKind::Text, true, &[])],
            vec![
                acao("anotar", None, &[], "coleta"),
                acao("confirmar", None, &["endereco"], "confirmacao"),
            ],
        ),
        etapa(
            "confirmacao",
            &[ActionActor::Agent],
            Vec::new(),
            vec![action_for_end("ok", None)],
        ),
    ]);
    let mut estado = start(&ir).expect("início");

    estado = execute_action(
        &ir,
        &estado,
        "anotar",
        &campos(json!({ "endereco": "Rua C" })),
        ActionActor::Agent,
        None,
        &context(),
    )
    .expect("anotar o campo")
    .new_state;

    let resultado = execute_action(
        &ir,
        &estado,
        "confirmar",
        &without_fields(),
        ActionActor::Agent,
        None,
        &context(),
    )
    .expect("o campo já estava gravado");
    assert_eq!(
        resultado.new_state.no_atual,
        Destination::Etapa {
            id: "confirmacao".to_owned()
        }
    );
}

#[test]
fn field_with_wrong_kind_refused_with_what_came() {
    let ir = fluxo(vec![
        etapa(
            "coleta",
            &[ActionActor::Agent],
            vec![campo("idade", FieldKind::Number, false, &[])],
            vec![acao("ok", None, &[], "depois")],
        ),
        etapa(
            "depois",
            &[ActionActor::Agent],
            Vec::new(),
            vec![action_for_end("fim", None)],
        ),
    ]);
    let estado = start(&ir).expect("início");

    let error = execute_action(
        &ir,
        &estado,
        "ok",
        &campos(json!({ "idade": "quarenta" })),
        ActionActor::Agent,
        None,
        &context(),
    )
    .expect_err("número recebeu texto");

    let errors = errors_fields(&error);
    assert_eq!(errors.len(), 1);
    assert!(
        errors[0].contains("idade") && errors[0].contains("número") && errors[0].contains("texto"),
        "a mensagem diz o campo, o esperado e o que veio: {errors:?}"
    );
}

#[test]
fn each_field_kind_refuses_value_not_its_own() {
    let ir = fluxo(vec![
        etapa(
            "cadastro",
            &[ActionActor::Agent],
            vec![
                campo("texto", FieldKind::Text, false, &[]),
                campo("data", FieldKind::Data, false, &[]),
                campo("booleano", FieldKind::Boolean, false, &[]),
                campo("anexo", FieldKind::Attachment, false, &[]),
            ],
            vec![acao("ok", None, &[], "depois")],
        ),
        etapa(
            "depois",
            &[ActionActor::Agent],
            Vec::new(),
            vec![action_for_end("fim", None)],
        ),
    ]);
    let estado = start(&ir).expect("início");

    let error = execute_action(
        &ir,
        &estado,
        "ok",
        &campos(json!({
            "texto": 10,
            "data": "22/11/2026",
            "booleano": "sim",
            "anexo": { "tipo": "application/pdf" },
        })),
        ActionActor::Agent,
        None,
        &context(),
    )
    .expect_err("todos os valores estão no tipo errado");

    let errors = errors_fields(&error);
    assert_eq!(errors.len(), 4, "{errors:?}");
    assert!(errors[0].starts_with("anexo:"), "{errors:?}");
    assert!(errors[1].starts_with("booleano:"), "{errors:?}");
    assert!(errors[2].starts_with("data:"), "{errors:?}");
    assert!(errors[3].starts_with("texto:"), "{errors:?}");
}

#[test]
fn valid_values_of_each_kind_are_stored() {
    let ir = fluxo(vec![
        etapa(
            "cadastro",
            &[ActionActor::Agent],
            vec![
                campo("texto", FieldKind::Text, false, &[]),
                campo("numero", FieldKind::Number, false, &[]),
                campo("data", FieldKind::Data, false, &[]),
                campo("booleano", FieldKind::Boolean, false, &[]),
                campo("anexo", FieldKind::Attachment, false, &[]),
            ],
            vec![acao("ok", None, &[], "depois")],
        ),
        etapa(
            "depois",
            &[ActionActor::Agent],
            Vec::new(),
            vec![action_for_end("fim", None)],
        ),
    ]);
    let estado = start(&ir).expect("início");
    let attachment = json!({ "nome": "laudo.pdf", "tipo": "application/pdf", "tamanho": 1024 });

    let resultado = execute_action(
        &ir,
        &estado,
        "ok",
        &campos(json!({
            "texto": "ok",
            "numero": 42,
            "data": "2026-11-22",
            "booleano": true,
            "anexo": attachment,
        })),
        ActionActor::Agent,
        None,
        &context(),
    )
    .expect("todos os valores são do tipo certo");

    assert_eq!(
        resultado.new_state.dados.get("cadastro"),
        Some(&json!({
            "texto": "ok",
            "numero": 42,
            "data": "2026-11-22",
            "booleano": true,
            "anexo": attachment,
        }))
    );
}

#[test]
fn option_outside_list_refused_with_options() {
    let ir = fluxo(vec![
        etapa(
            "coleta",
            &[ActionActor::Agent],
            vec![campo(
                "periodo",
                FieldKind::Option,
                false,
                &["manhã", "tarde"],
            )],
            vec![acao("ok", None, &[], "depois")],
        ),
        etapa(
            "depois",
            &[ActionActor::Agent],
            Vec::new(),
            vec![action_for_end("fim", None)],
        ),
    ]);
    let estado = start(&ir).expect("início");

    let error = execute_action(
        &ir,
        &estado,
        "ok",
        &campos(json!({ "periodo": "noite" })),
        ActionActor::Agent,
        None,
        &context(),
    )
    .expect_err("`noite` não está na lista");

    let errors = errors_fields(&error);
    assert!(
        errors[0].contains("`manhã`") && errors[0].contains("`tarde`"),
        "a mensagem lista as opções: {errors:?}"
    );
}

#[test]
fn field_other_step_and_refused_with_fields_step() {
    let ir = flow_simple();
    let estado = start(&ir).expect("início");

    let error = execute_action(
        &ir,
        &estado,
        "confirmar",
        &campos(json!({ "cep": "20000-000", "endereco": "Rua D" })),
        ActionActor::Agent,
        None,
        &context(),
    )
    .expect_err("`cep` não é campo de `coleta`");

    let errors = errors_fields(&error);
    assert_eq!(errors.len(), 1);
    assert!(
        errors[0].contains("`cep`") && errors[0].contains("`endereco`"),
        "a mensagem cita o campo recusado e os válidos: {errors:?}"
    );
}

#[test]
fn field_with_prefix_step_and_refused_with_hint() {
    let ir = flow_simple();
    let estado = start(&ir).expect("início");

    let error = execute_action(
        &ir,
        &estado,
        "confirmar",
        &campos(json!({ "coleta.endereco": "Rua E" })),
        ActionActor::Agent,
        None,
        &context(),
    )
    .expect_err("o valor vem por id de campo, sem prefixo");

    let errors = errors_fields(&error);
    assert!(
        errors[0].contains("sem o prefixo da etapa"),
        "a mensagem ensina a forma certa: {errors:?}"
    );
}

#[test]
fn field_required_not_accepts_value_empty() {
    let ir = flow_simple();
    let estado = start(&ir).expect("início");

    let error = execute_action(
        &ir,
        &estado,
        "confirmar",
        &campos(json!({ "endereco": "   " })),
        ActionActor::Agent,
        None,
        &context(),
    )
    .expect_err("campo obrigatório veio vazio");

    assert!(errors_fields(&error)[0].contains("obrigatório"));
}

#[test]
fn field_optional_accepts_null_and_field_counts_as_empty() {
    let ir = fluxo(vec![
        etapa(
            "coleta",
            &[ActionActor::Agent],
            vec![campo("bairro", FieldKind::Text, false, &[])],
            vec![
                acao("limpar", None, &[], "coleta"),
                acao("confirmar", None, &["bairro"], "depois"),
            ],
        ),
        etapa(
            "depois",
            &[ActionActor::Agent],
            Vec::new(),
            vec![action_for_end("fim", None)],
        ),
    ]);
    let estado = start(&ir).expect("início");

    let with_null = execute_action(
        &ir,
        &estado,
        "limpar",
        &campos(json!({ "bairro": Json::Null })),
        ActionActor::Agent,
        None,
        &context(),
    )
    .expect("campo opcional aceita nulo");
    assert_eq!(
        with_null.new_state.dados.get("coleta"),
        Some(&json!({ "bairro": Json::Null }))
    );

    let error = execute_action(
        &ir,
        &with_null.new_state,
        "confirmar",
        &without_fields(),
        ActionActor::Agent,
        None,
        &context(),
    )
    .expect_err("nulo não preenche");
    assert!(errors_fields(&error)[0].contains("`bairro`"));
}

#[test]
fn actor_outside_actors_step_and_refused() {
    let ir = flow_simple();
    let estado = start(&ir).expect("início");

    let error = execute_action(
        &ir,
        &estado,
        "confirmar",
        &campos(json!({ "endereco": "Rua F" })),
        ActionActor::Attendant,
        None,
        &context(),
    )
    .expect_err("a etapa só permite o agente");

    let texto = motivo(&error);
    assert!(
        texto.contains("`atendente`") && texto.contains("`agente`"),
        "a mensagem diz quem tentou e quem pode: {texto}"
    );
}

#[test]
fn action_restricted_attendant_refused_for_agent() {
    let ir = flow_with_two_actors();
    let estado = start(&ir).expect("início");

    let error = execute_action(
        &ir,
        &estado,
        "assumir",
        &without_fields(),
        ActionActor::Agent,
        None,
        &context(),
    )
    .expect_err("`assumir` é do atendente");

    let texto = motivo(&error);
    assert!(
        texto.contains("`assumir`") && texto.contains("`atendente`"),
        "a mensagem diz a ação e o dono: {texto}"
    );
}

#[test]
fn step_with_two_actors_stays_partial_and_says_who_missing() {
    let ir = flow_with_two_actors();
    let estado = start(&ir).expect("início");

    let resultado = execute_action(
        &ir,
        &estado,
        "registrar",
        &campos(json!({ "resumo": "cliente quer troca" })),
        ActionActor::Agent,
        None,
        &context(),
    )
    .expect("a ação é válida");

    assert_eq!(
        resultado.new_state.no_atual,
        Destination::Etapa {
            id: "triagem".to_owned()
        },
        "a etapa não avança enquanto o atendente não agir"
    );
    assert_eq!(resultado.waiting, vec![ActionActor::Attendant]);
    assert_eq!(
        resultado.new_state.actors_who_acted,
        vec![ActionActor::Agent]
    );
    assert_eq!(
        kinds(&resultado),
        vec![
            RecordEventKind::FieldsFilled,
            RecordEventKind::ActionExecuted,
        ],
        "etapa parcial não tem evento de conclusão nem de entrada"
    );
    assert_eq!(
        resultado.events[1].payload["parcial"],
        json!(true),
        "o evento da ação registra que a etapa ficou parcial"
    );
}

#[test]
fn step_advances_when_second_actor_acts() {
    let ir = flow_with_two_actors();
    let mut estado = start(&ir).expect("início");
    estado = execute_action(
        &ir,
        &estado,
        "registrar",
        &without_fields(),
        ActionActor::Agent,
        None,
        &context(),
    )
    .expect("primeiro ator")
    .new_state;

    let resultado = execute_action(
        &ir,
        &estado,
        "assumir",
        &without_fields(),
        ActionActor::Attendant,
        None,
        &context(),
    )
    .expect("segundo ator");

    assert_eq!(
        resultado.new_state.no_atual,
        Destination::Etapa {
            id: "depois".to_owned()
        }
    );
    assert!(resultado.waiting.is_empty());
    assert!(
        resultado.new_state.actors_who_acted.is_empty(),
        "a etapa nova não herda quem agiu na anterior"
    );
    assert_eq!(
        kinds(&resultado),
        vec![
            RecordEventKind::ActionExecuted,
            RecordEventKind::StepCompleted,
            RecordEventKind::StepEntered,
        ],
        "o segundo ator conclui a etapa e entra na próxima"
    );
}

#[test]
fn both_actors_arrive_same_target_in_any_order() {
    let ir = flow_with_two_actors();

    let mut agent_first = start(&ir).expect("início");
    agent_first = execute_action(
        &ir,
        &agent_first,
        "registrar",
        &without_fields(),
        ActionActor::Agent,
        None,
        &context(),
    )
    .expect("agente primeiro")
    .new_state;
    let end_agent_first = execute_action(
        &ir,
        &agent_first,
        "assumir",
        &without_fields(),
        ActionActor::Attendant,
        None,
        &context(),
    )
    .expect("atendente depois");

    let mut attendant_first = start(&ir).expect("início");
    attendant_first = execute_action(
        &ir,
        &attendant_first,
        "assumir",
        &without_fields(),
        ActionActor::Attendant,
        None,
        &context(),
    )
    .expect("atendente primeiro")
    .new_state;
    let end_attendant_first = execute_action(
        &ir,
        &attendant_first,
        "registrar",
        &without_fields(),
        ActionActor::Agent,
        None,
        &context(),
    )
    .expect("agente depois");

    assert_eq!(
        end_agent_first.new_state.no_atual, end_attendant_first.new_state.no_atual,
        "as duas ordens terminam na mesma etapa"
    );
    assert_eq!(
        end_agent_first.new_state.no_atual,
        Destination::Etapa {
            id: "depois".to_owned()
        }
    );
}

#[test]
fn unrestricted_action_advances_alone() {
    let ir = fluxo(vec![
        etapa(
            "triagem",
            &[ActionActor::Agent, ActionActor::Attendant],
            Vec::new(),
            vec![
                acao("resolver", None, &[], "outro"),
                action_for_end("encaminhar", Some(ActionActor::Attendant)),
            ],
        ),
        etapa(
            "outro",
            &[ActionActor::Agent],
            Vec::new(),
            vec![action_for_end("ok", None)],
        ),
    ]);
    let estado = start(&ir).expect("início");

    let resultado = execute_action(
        &ir,
        &estado,
        "resolver",
        &without_fields(),
        ActionActor::Agent,
        None,
        &context(),
    )
    .expect("a ação irrestrita é válida");

    assert_eq!(
        resultado.new_state.no_atual,
        Destination::Etapa {
            id: "outro".to_owned()
        }
    );
    assert!(resultado.waiting.is_empty());
}

#[test]
fn unrestricted_action_still_waits_restricted_action_other_actor() {
    let ir = fluxo(vec![
        etapa(
            "triagem",
            &[ActionActor::Agent, ActionActor::Attendant],
            Vec::new(),
            vec![
                acao("encaminhar", Some(ActionActor::Attendant), &[], "depois"),
                acao("resolver", None, &[], "outro"),
            ],
        ),
        etapa(
            "depois",
            &[ActionActor::Agent],
            Vec::new(),
            vec![action_for_end("ok", None)],
        ),
        etapa(
            "outro",
            &[ActionActor::Agent],
            Vec::new(),
            vec![action_for_end("ok", None)],
        ),
    ]);
    let mut estado = start(&ir).expect("início");

    let partial = execute_action(
        &ir,
        &estado,
        "resolver",
        &without_fields(),
        ActionActor::Agent,
        None,
        &context(),
    )
    .expect("a ação irrestrita é válida");
    assert_eq!(
        partial.new_state.no_atual,
        Destination::Etapa {
            id: "triagem".to_owned()
        },
        "o atendente ainda não agiu"
    );
    assert_eq!(partial.waiting, vec![ActionActor::Attendant]);

    estado = partial.new_state;
    let advanced = execute_action(
        &ir,
        &estado,
        "encaminhar",
        &without_fields(),
        ActionActor::Attendant,
        None,
        &context(),
    )
    .expect("o atendente age");
    assert_eq!(
        advanced.new_state.no_atual,
        Destination::Etapa {
            id: "depois".to_owned()
        },
        "a rota é a da última ação executada"
    );
}

#[test]
fn action_that_goes_for_end_and_decisive_same_with_other_actor_pending() {
    let ir = fluxo(vec![
        etapa(
            "triagem",
            &[ActionActor::Agent, ActionActor::Attendant],
            Vec::new(),
            vec![
                acao("assumir", Some(ActionActor::Attendant), &[], "depois"),
                action_for_end("encerrar", None),
            ],
        ),
        etapa(
            "depois",
            &[ActionActor::Agent],
            Vec::new(),
            vec![action_for_end("ok", None)],
        ),
    ]);
    let estado = start(&ir).expect("início");

    let resultado = execute_action(
        &ir,
        &estado,
        "encerrar",
        &without_fields(),
        ActionActor::Agent,
        None,
        &context(),
    )
    .expect("a ação é válida");

    assert!(
        matches!(resultado.new_state.no_atual, Destination::Fim { .. }),
        "a ação de saída leva ao fim: {:?}",
        resultado.new_state.no_atual
    );
    assert!(
        resultado.waiting.is_empty(),
        "não há ninguém a esperar depois do fim"
    );
    assert_eq!(
        kinds(&resultado),
        vec![
            RecordEventKind::ActionExecuted,
            RecordEventKind::StepCompleted,
            RecordEventKind::ExecutionCompleted,
        ],
        "sem `etapa_entrou`: o destino é o fim, que conclui a execução"
    );
    assert_eq!(
        resultado.new_state.status,
        ExecutionStatus::Completed,
        "o fim conclui a execução na mesma passagem"
    );
    assert!(
        !resultado.ends_service,
        "este fim não encerra o atendimento"
    );
}

#[test]
fn step_with_restricted_action_single_actor_does_not_wait_other() {
    let ir = fluxo(vec![
        etapa(
            "coleta",
            &[ActionActor::Agent, ActionActor::Attendant],
            Vec::new(),
            vec![
                acao("confirmar", None, &[], "depois"),
                action_for_end("desistir", Some(ActionActor::Attendant)),
            ],
        ),
        etapa(
            "depois",
            &[ActionActor::Agent],
            Vec::new(),
            vec![action_for_end("ok", None)],
        ),
    ]);
    let estado = start(&ir).expect("início");

    let resultado = execute_action(
        &ir,
        &estado,
        "confirmar",
        &without_fields(),
        ActionActor::Agent,
        None,
        &context(),
    )
    .expect("a ação é válida");

    assert_eq!(
        resultado.new_state.no_atual,
        Destination::Etapa {
            id: "depois".to_owned()
        }
    );
    assert!(resultado.waiting.is_empty());
}

#[test]
fn required_actors_come_from_actions_in_text_order() {
    let etapa = etapa(
        "mista",
        &[ActionActor::Agent, ActionActor::Attendant],
        Vec::new(),
        vec![
            acao("assumir", Some(ActionActor::Attendant), &[], "depois"),
            action_for_end("desistir", Some(ActionActor::Attendant)),
            acao("livre", None, &[], "depois"),
            acao("registrar", Some(ActionActor::Agent), &[], "depois"),
            acao("registrar_de_novo", Some(ActionActor::Agent), &[], "depois"),
        ],
    );

    assert_eq!(
        required_actors(&etapa),
        vec![ActionActor::Agent, ActionActor::Attendant],
        "sem repetição, sem o ator da ação de fim, na ordem alfabética do texto"
    );
}

#[test]
fn missing_fields_lists_only_required_in_script_order() {
    let etapa = etapa(
        "coleta",
        &[ActionActor::Agent],
        vec![
            campo("endereco", FieldKind::Text, true, &[]),
            campo("bairro", FieldKind::Text, false, &[]),
            campo("periodo", FieldKind::Option, true, &["manhã", "tarde"]),
        ],
        Vec::new(),
    );
    let mut dados = poph_fluxos::Dados::new();
    dados.insert(
        "coleta".to_owned(),
        json!({ "endereco": "Rua G", "bairro": "Centro" }),
    );

    assert_eq!(missing_fields(&etapa, &dados), vec!["periodo".to_owned()]);

    dados.insert(
        "coleta".to_owned(),
        json!({ "endereco": "Rua G", "periodo": "tarde" }),
    );
    assert!(missing_fields(&etapa, &dados).is_empty());
}

#[test]
fn repeating_key_generates_no_second_event() {
    let ir = flow_simple();
    let estado = start(&ir).expect("início");
    let entry = campos(json!({ "endereco": "Rua H" }));

    let first_pass = execute_action(
        &ir,
        &estado,
        "confirmar",
        &entry,
        ActionActor::Agent,
        Some("chave-1"),
        &context(),
    )
    .expect("primeira passagem");
    assert!(!first_pass.repeated);
    assert!(!first_pass.events.is_empty());

    let second_pass = execute_action(
        &ir,
        &first_pass.new_state,
        "confirmar",
        &entry,
        ActionActor::Agent,
        Some("chave-1"),
        &context(),
    )
    .expect("a repetição não é erro");

    assert!(second_pass.repeated);
    assert!(second_pass.events.is_empty(), "repetir não grava evento");
    assert!(second_pass.effects.is_empty());
    assert_eq!(
        second_pass.new_state, first_pass.new_state,
        "o estado não muda na repetição"
    );

    let third = execute_action(
        &ir,
        &second_pass.new_state,
        "ok",
        &without_fields(),
        ActionActor::Agent,
        Some("chave-2"),
        &context(),
    )
    .expect("chave nova aplica");
    assert!(!third.repeated);
    assert!(!third.events.is_empty());
}

#[test]
fn repeated_key_returns_who_was_missing_when_step_became_partial() {
    let ir = flow_with_two_actors();
    let estado = start(&ir).expect("início");
    let first_pass = execute_action(
        &ir,
        &estado,
        "registrar",
        &without_fields(),
        ActionActor::Agent,
        Some("chave-triagem"),
        &context(),
    )
    .expect("primeira passagem");
    assert_eq!(first_pass.waiting, vec![ActionActor::Attendant]);

    let repeated = execute_action(
        &ir,
        &first_pass.new_state,
        "registrar",
        &without_fields(),
        ActionActor::Agent,
        Some("chave-triagem"),
        &context(),
    )
    .expect("repetição");

    assert!(repeated.repeated);
    assert_eq!(
        repeated.waiting,
        vec![ActionActor::Attendant],
        "a repetição devolve o mesmo desfecho"
    );
    assert!(repeated.events.is_empty());
}

#[test]
fn missing_action_returns_valid_ones_and_changes_nothing() {
    let ir = flow_simple();
    let estado = start(&ir).expect("início");

    let error = execute_action(
        &ir,
        &estado,
        "concluir",
        &campos(json!({ "endereco": "Rua I" })),
        ActionActor::Agent,
        None,
        &context(),
    )
    .expect_err("`concluir` não existe");

    let texto = motivo(&error);
    assert!(
        texto.contains("`confirmar`") && texto.contains("`desistir`"),
        "a mensagem lista as ações válidas: {texto}"
    );
    assert_eq!(error.http_status(), 409);
    assert!(estado.dados.is_empty());
    assert_eq!(
        estado.no_atual,
        Destination::Etapa {
            id: "coleta".to_owned()
        }
    );
}

#[test]
fn step_without_action_warns_it_has_no_exit() {
    let ir = fluxo(vec![
        etapa("coleta", &[ActionActor::Agent], Vec::new(), Vec::new()),
        etapa(
            "depois",
            &[ActionActor::Agent],
            Vec::new(),
            vec![action_for_end("ok", None)],
        ),
    ]);
    let estado = start(&ir).expect("início");

    let error = execute_action(
        &ir,
        &estado,
        "qualquer",
        &without_fields(),
        ActionActor::Agent,
        None,
        &context(),
    )
    .expect_err("a etapa não tem ação");
    assert!(motivo(&error).contains("nenhuma (a etapa não tem saída)"));
}

#[test]
fn already_closed_execution_refuses_action() {
    let ir = flow_simple();
    let mut estado = start(&ir).expect("início");
    estado.status = ExecutionStatus::Completed;

    let error = execute_action(
        &ir,
        &estado,
        "confirmar",
        &campos(json!({ "endereco": "Rua J" })),
        ActionActor::Agent,
        None,
        &context(),
    )
    .expect_err("execução concluída não aceita ação");
    assert!(motivo(&error).contains("concluida"));
}

#[test]
fn execution_stopped_at_decision_settles_before_running_action() {
    let mut ir = flow_simple();
    ir.decisions = vec![poph_fluxos::ir::Decisao {
        id: "escolher_rota".to_owned(),
        se: poph_fluxos::ir::Rule::Vazio {
            campo: "coleta.endereco".to_owned(),
        },
        then_branch: Destination::Etapa {
            id: "coleta".to_owned(),
        },
        else_branch: Destination::Etapa {
            id: "confirmacao".to_owned(),
        },
    }];
    let mut estado = start(&ir).expect("início");
    estado.no_atual = Destination::Decisao {
        id: "escolher_rota".to_owned(),
    };

    let resultado = execute_action(
        &ir,
        &estado,
        "confirmar",
        &campos(json!({ "endereco": "Rua das Flores, 10" })),
        ActionActor::Agent,
        None,
        &context(),
    )
    .expect("a decisão é resolvida e a ação vale na etapa de destino");

    assert_eq!(
        kinds(&resultado),
        vec![
            RecordEventKind::DecisionResolved,
            RecordEventKind::StepEntered,
            RecordEventKind::FieldsFilled,
            RecordEventKind::ActionExecuted,
            RecordEventKind::StepCompleted,
            RecordEventKind::StepEntered,
        ],
        "a decisão é resolvida antes da ação"
    );
    assert_eq!(
        resultado.new_state.no_atual,
        Destination::Etapa {
            id: "confirmacao".to_owned()
        }
    );
}

#[test]
fn action_for_step_missing_and_ir_divergent() {
    let ir = fluxo(vec![etapa(
        "coleta",
        &[ActionActor::Agent],
        Vec::new(),
        vec![acao("ok", None, &[], "etapa_que_nao_existe")],
    )]);
    let estado = start(&ir).expect("início");

    let error = execute_action(
        &ir,
        &estado,
        "ok",
        &without_fields(),
        ActionActor::Agent,
        None,
        &context(),
    )
    .expect_err("o destino não existe na estrutura");
    assert!(
        matches!(error, DomainError::IrDivergent { .. }),
        "{error:?}"
    );
}

#[test]
fn same_entry_produces_same_result() {
    let ir = flow_with_two_actors();
    let estado = start(&ir).expect("início");
    let entry = campos(json!({ "resumo": "troca de produto" }));

    let first_result = execute_action(
        &ir,
        &estado,
        "registrar",
        &entry,
        ActionActor::Agent,
        Some("chave"),
        &context(),
    )
    .expect("primeira");
    let second_result = execute_action(
        &ir,
        &estado,
        "registrar",
        &entry,
        ActionActor::Agent,
        Some("chave"),
        &context(),
    )
    .expect("segunda");

    assert_eq!(first_result, second_result);
}

#[test]
fn golden_ir_schedule_visit_runs_from_start_to_end_of_step() {
    let path = format!(
        "{}/../../tests/fluxos/ir/agendar_visita.json",
        env!("CARGO_MANIFEST_DIR")
    );
    let texto = std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("{path}: {error}"));
    let ir = Ir::from_json(&texto).expect("a IR dourada é JSON válido");

    let estado = start(&ir).expect("início");
    assert_eq!(
        estado.no_atual,
        Destination::Etapa {
            id: "coleta".to_owned()
        }
    );

    let coletado = execute_action(
        &ir,
        &estado,
        "confirmar",
        &campos(json!({ "endereco": "Rua das Flores, 10", "periodo": "manhã" })),
        ActionActor::Agent,
        Some("acao-1"),
        &context(),
    )
    .expect("a ação confirmar é válida");
    assert_eq!(
        coletado.new_state.no_atual,
        Destination::Etapa {
            id: "confirmacao".to_owned()
        }
    );

    let confirmed = execute_action(
        &ir,
        &coletado.new_state,
        "ok",
        &without_fields(),
        ActionActor::Agent,
        Some("acao-2"),
        &context(),
    )
    .expect("a ação ok é válida");
    assert!(
        matches!(confirmed.new_state.no_atual, Destination::Fim { .. }),
        "{:?}",
        confirmed.new_state.no_atual
    );
}

#[test]
fn customer_data_stored_as_value_never_interpreted() {
    let ir = flow_simple();
    let estado = start(&ir).expect("início");
    let client = "__import__('os').system('ls')";

    let resultado = execute_action(
        &ir,
        &estado,
        "confirmar",
        &campos(json!({ "endereco": client })),
        ActionActor::Agent,
        None,
        &context(),
    )
    .expect("o texto do cliente é só um texto");

    assert_eq!(
        resultado.new_state.dados["coleta"]["endereco"],
        json!(client),
        "o valor sai como entrou, sem virar código"
    );
}

#[test]
fn state_survives_json_round_trip() {
    let ir = flow_simple();
    let resultado = execute_action(
        &ir,
        &start(&ir).expect("início"),
        "confirmar",
        &campos(json!({ "endereco": "Rua K" })),
        ActionActor::Agent,
        Some("chave"),
        &context(),
    )
    .expect("ação válida");

    let json = serde_json::to_string(&resultado.new_state).expect("serializa");
    let returned: Estado = serde_json::from_str(&json).expect("lê");
    assert_eq!(returned, resultado.new_state);
}
