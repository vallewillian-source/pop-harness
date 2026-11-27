use poph_fluxos::ir::{
    Acao, Campo, Decisao, Def, Destination, Effect, Etapa, Fim, Fluxo, Gatilho, Ir, Rule,
};
use poph_fluxos::{Change, ChangeType, Impact, diff};
use poph_types::{ActionActor, FieldKind};

fn base() -> Ir {
    let path = format!(
        "{}/../../tests/fluxos/ir/agendar_visita.json",
        env!("CARGO_MANIFEST_DIR")
    );
    let texto = std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("{path}: {error}"));
    Ir::from_json(&texto).expect("a IR dourada é JSON válido")
}

fn com(ir: &Ir, modify: impl FnOnce(&mut Ir)) -> Ir {
    let mut copy = ir.clone();
    modify(&mut copy);
    copy
}

fn in_step<'a>(ir: &'a mut Ir, id: &str) -> &'a mut Etapa {
    ir.stages
        .iter_mut()
        .find(|etapa| etapa.id == id)
        .expect("a etapa existe na base")
}

fn in_action<'a>(ir: &'a mut Ir, etapa: &str, acao: &str) -> &'a mut Acao {
    in_step(ir, etapa)
        .actions
        .iter_mut()
        .find(|candidate| candidate.id == acao)
        .expect("a ação existe na base")
}

fn campo(id: &str, label: &str, tipo: FieldKind, required: bool, options: &[&str]) -> Campo {
    Campo {
        id: id.to_owned(),
        label: label.to_owned(),
        tipo,
        required,
        options: options.iter().map(|option| (*option).to_owned()).collect(),
        help: None,
    }
}

fn acao(
    id: &str,
    label: &str,
    requires: &[&str],
    actor: Option<ActionActor>,
    goes_for: Destination,
) -> Acao {
    Acao {
        id: id.to_owned(),
        label: label.to_owned(),
        requires: requires.iter().map(|campo| (*campo).to_owned()).collect(),
        actor,
        goes_to: goes_for,
        effects: Vec::new(),
    }
}

fn etapa(id: &str, nome: &str, instruction: Option<&str>, actions: Vec<Acao>) -> Etapa {
    Etapa {
        id: id.to_owned(),
        nome: nome.to_owned(),
        instruction: instruction.map(str::to_owned),
        action_actors: vec![ActionActor::Agent],
        campos: Vec::new(),
        actions,
        on_enter: Vec::new(),
    }
}

fn step(id: &str) -> Destination {
    Destination::Etapa { id: id.to_owned() }
}

fn end(id: &str, resultado: Option<&str>, ends_service: bool) -> Destination {
    Destination::Fim {
        id: id.to_owned(),
        resultado: resultado.map(str::to_owned),
        ends_service,
    }
}

fn fim(id: &str, resultado: Option<&str>, ends_service: bool) -> Fim {
    Fim {
        id: id.to_owned(),
        resultado: resultado.map(str::to_owned),
        ends_service,
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

fn igual(campo: &str, value: &str) -> Rule {
    Rule::Igual {
        campo: campo.to_owned(),
        value: value.to_owned(),
    }
}

fn single(list: &[Change], tipo: ChangeType) -> &Change {
    let found: Vec<&Change> = list.iter().filter(|change| change.tipo == tipo).collect();
    assert_eq!(
        found.len(),
        1,
        "esperava uma mudança {tipo:?}; veio {list:#?}"
    );
    found[0]
}

fn kinds(list: &[Change]) -> Vec<ChangeType> {
    list.iter().map(|change| change.tipo).collect()
}

const ALL_KINDS: [ChangeType; 27] = [
    ChangeType::NewStage,
    ChangeType::RemovedStage,
    ChangeType::InstructionChanged,
    ChangeType::StageActorsChanged,
    ChangeType::NewField,
    ChangeType::RemovedField,
    ChangeType::FieldNowRequired,
    ChangeType::FieldNoLongerRequired,
    ChangeType::FieldTypeChanged,
    ChangeType::FieldOptionsChanged,
    ChangeType::ActionNowRequires,
    ChangeType::ActionNoLongerRequires,
    ChangeType::NewAction,
    ChangeType::RemovedAction,
    ChangeType::DestinationChanged,
    ChangeType::ActionActorChanged,
    ChangeType::RuleChanged,
    ChangeType::NewDecision,
    ChangeType::RemovedDecision,
    ChangeType::NewEnd,
    ChangeType::RemovedEnd,
    ChangeType::EndChanged,
    ChangeType::NewEffect,
    ChangeType::RemovedEffect,
    ChangeType::TriggerChanged,
    ChangeType::StartChanged,
    ChangeType::TextChanged,
];

#[test]
fn flows_identical_return_list_empty() {
    let ir = base();
    assert!(diff(&ir, &ir).is_empty());
}

#[test]
fn json_round_trip_produces_no_change() {
    let ir = base();
    let r#return = Ir::from_json(&ir.to_json()).expect("a IR gravada volta a ser lida");
    assert!(diff(&ir, &r#return).is_empty());
}

#[test]
fn same_input_always_same_list_in_same_order() {
    let a = base();
    let b = com(&a, |ir| {
        in_step(ir, "coleta").nome = "Colher dados".to_owned();
        in_action(ir, "coleta", "confirmar").goes_to =
            end("concluida", Some("Visita agendada"), false);
        ir.gatilho.when = "Quando o cliente pedir remarcação.".to_owned();
    });
    let first_pass = diff(&a, &b);
    let second_pass = diff(&a, &b);
    assert_eq!(first_pass, second_pass);
    assert_eq!(
        kinds(&first_pass),
        vec![
            ChangeType::TriggerChanged,
            ChangeType::TextChanged,
            ChangeType::DestinationChanged,
        ],
        "o fluxo vem primeiro, depois as etapas"
    );
}

#[test]
fn item_order_in_script_produces_no_change() {
    let a = base();
    let b = com(&a, |ir| ir.stages.reverse());
    assert!(diff(&a, &b).is_empty());
}

#[test]
fn codes_kind_and_impact_match_with_json_api() {
    for tipo in ALL_KINDS {
        let json = serde_json::to_string(&tipo).expect("o tipo serializa");
        assert_eq!(json, format!("\"{}\"", tipo.code()), "tipo {tipo:?}");
    }
    for impact in [
        Impact::None,
        Impact::NewServices,
        Impact::NewDataRequested,
        Impact::RouteChanged,
        Impact::EffectTriggered,
    ] {
        let json = serde_json::to_string(&impact).expect("o impacto serializa");
        assert_eq!(json, format!("\"{}\"", impact.code()), "impacto {impact:?}");
    }
}

#[test]
fn new_step_without_arrival_does_not_change_conversation() {
    let a = base();
    let b = com(&a, |ir| {
        ir.stages
            .push(etapa("revisao", "Revisar dados", None, Vec::new()));
    });
    let list = diff(&a, &b);
    assert_eq!(list.len(), 1, "{list:#?}");
    let new_stage = single(&list, ChangeType::NewStage);
    assert_eq!(new_stage.impact, Impact::None);
    assert_eq!(new_stage.target, "etapa:revisao");
    assert_eq!(
        new_stage.texto,
        "a etapa `revisao` (Revisar dados) é nova, mas nenhuma ação ou decisão leva a ela: nada \
         muda no atendimento."
    );
}

#[test]
fn new_step_receiving_arrival_changes_route() {
    let a = base();
    let b = com(&a, |ir| {
        ir.stages
            .push(etapa("revisao", "Revisar dados", None, Vec::new()));
        in_action(ir, "coleta", "confirmar").goes_to = step("revisao");
    });
    let list = diff(&a, &b);
    let new_stage = single(&list, ChangeType::NewStage);
    assert_eq!(new_stage.impact, Impact::RouteChanged);
    assert_eq!(
        new_stage.texto,
        "a etapa `revisao` (Revisar dados) é nova e já é destino de alguma ação ou decisão: os \
         atendimentos novos passam por ela."
    );
}

#[test]
fn removed_step_that_was_reached_changes_route() {
    let a = base();
    let b = com(&a, |ir| {
        ir.stages.retain(|etapa| etapa.id != "confirmacao");
        in_action(ir, "coleta", "confirmar").goes_to =
            end("concluida", Some("Visita agendada"), false);
    });
    let list = diff(&a, &b);
    let removed_stage = single(&list, ChangeType::RemovedStage);
    assert_eq!(removed_stage.impact, Impact::RouteChanged);
    assert_eq!(removed_stage.target, "etapa:confirmacao");
    assert_eq!(
        removed_stage.texto,
        "a etapa `confirmacao` (Confirmar visita) foi removida: os atendimentos novos não passam \
         mais por ela."
    );
}

#[test]
fn removed_step_without_arrival_does_not_change_conversation() {
    let a = com(&base(), |ir| {
        ir.stages.push(etapa("sobra", "Sobra", None, Vec::new()));
    });
    let b = base();
    let list = diff(&a, &b);
    assert_eq!(list.len(), 1, "{list:#?}");
    let removed_stage = single(&list, ChangeType::RemovedStage);
    assert_eq!(removed_stage.impact, Impact::None);
    assert_eq!(
        removed_stage.texto,
        "a etapa `sobra` (Sobra) foi removida, mas nenhuma ação ou decisão a usava: nada muda no \
         atendimento."
    );
}

#[test]
fn instruction_changed_applies_only_to_new_conversations() {
    let a = base();
    let b = com(&a, |ir| {
        in_step(ir, "coleta").instruction = Some("Pergunte só o endereço.".to_owned());
    });
    let list = diff(&a, &b);
    let changed = single(&list, ChangeType::InstructionChanged);
    assert_eq!(changed.impact, Impact::NewServices);
    assert_eq!(changed.target, "etapa:coleta");
    assert_eq!(
        changed.texto,
        "a instrução da etapa `coleta` mudou de \"Pergunte o endereço e o período preferido. Uma \
         pergunta por vez.\" para \"Pergunte só o endereço.\": vale para os atendimentos novos."
    );
}

#[test]
fn actors_step_changed_changes_route() {
    let a = base();
    let b = com(&a, |ir| {
        in_step(ir, "coleta").action_actors = vec![ActionActor::Agent, ActionActor::Attendant];
    });
    let list = diff(&a, &b);
    let changed = single(&list, ChangeType::StageActorsChanged);
    assert_eq!(changed.impact, Impact::RouteChanged);
    assert_eq!(
        changed.texto,
        "as ações da etapa `coleta` passam a ser feitas por agente ou atendente."
    );
}

#[test]
fn renaming_step_and_field_does_not_change_conversation() {
    let a = base();
    let b = com(&a, |ir| {
        in_step(ir, "coleta").nome = "Colher dados".to_owned();
        in_step(ir, "coleta").campos[0].label = "Endereço".to_owned();
    });
    let list = diff(&a, &b);
    assert_eq!(list.len(), 2, "{list:#?}");
    assert!(list.iter().all(|m| m.impact == Impact::None), "{list:#?}");
    let nome = list
        .iter()
        .find(|m| m.target == "etapa:coleta")
        .expect("o nome da etapa aparece");
    assert_eq!(nome.tipo, ChangeType::TextChanged);
    assert_eq!(nome.target, "etapa:coleta");
    assert_eq!(
        nome.texto,
        "o nome da etapa `coleta` mudou de \"Coletar dados\" para \"Colher dados\"."
    );
    let label = list
        .iter()
        .find(|m| m.target == "etapa:coleta/campo:endereco")
        .expect("o rótulo do campo aparece");
    assert_eq!(label.tipo, ChangeType::TextChanged);
    assert_eq!(
        label.texto,
        "o rótulo do campo `endereco` na etapa `coleta` mudou de \"Endereço completo\" para \
         \"Endereço\": o agente passa a usar o novo nome ao perguntar."
    );
}

#[test]
fn field_new_asks_given_new() {
    let a = base();
    let b = com(&a, |ir| {
        in_step(ir, "coleta")
            .campos
            .push(campo("cep", "CEP", FieldKind::Text, false, &[]));
    });
    let list = diff(&a, &b);
    let new_item = single(&list, ChangeType::NewField);
    assert_eq!(new_item.impact, Impact::NewDataRequested);
    assert_eq!(new_item.target, "etapa:coleta/campo:cep");
    assert_eq!(
        new_item.texto,
        "a etapa `coleta` passa a coletar o campo `cep` (CEP): o agente pedirá esse dado nos \
         atendimentos novos."
    );
}

#[test]
fn removed_field_no_longer_requested_in_new_conversations() {
    let a = base();
    let b = com(&a, |ir| {
        in_step(ir, "coleta")
            .campos
            .retain(|campo| campo.id != "periodo");
        in_action(ir, "coleta", "confirmar").requires = vec!["endereco".to_owned()];
    });
    let list = diff(&a, &b);
    let removed_item = single(&list, ChangeType::RemovedField);
    assert_eq!(removed_item.impact, Impact::NewServices);
    assert_eq!(removed_item.target, "etapa:coleta/campo:periodo");
    assert_eq!(
        removed_item.texto,
        "a etapa `coleta` deixa de coletar o campo `periodo` (Período): nos atendimentos novos, o \
         agente não pedirá esse dado."
    );
}

#[test]
fn field_becoming_required_asks_new_value() {
    let a = com(&base(), |ir| {
        in_step(ir, "coleta").campos[0].required = false;
    });
    let b = base();
    let list = diff(&a, &b);
    let now = single(&list, ChangeType::FieldNowRequired);
    assert_eq!(now.impact, Impact::NewDataRequested);
    assert_eq!(now.target, "etapa:coleta/campo:endereco");
    assert_eq!(
        now.texto,
        "o campo `endereco` da etapa `coleta` agora é obrigatório: o agente vai pedi-lo antes de \
         concluir a etapa."
    );
}

#[test]
fn field_no_longer_required_applies_to_new_conversations() {
    let a = base();
    let b = com(&a, |ir| {
        in_step(ir, "coleta").campos[0].required = false;
    });
    let list = diff(&a, &b);
    let left = single(&list, ChangeType::FieldNoLongerRequired);
    assert_eq!(left.impact, Impact::NewServices);
    assert_eq!(
        left.texto,
        "o campo `endereco` da etapa `coleta` deixou de ser obrigatório: nos atendimentos novos, a \
         etapa pode ser concluída sem ele."
    );
}

#[test]
fn field_with_other_kind_asks_given_new_way() {
    let a = base();
    let b = com(&a, |ir| {
        in_step(ir, "coleta").campos[1].tipo = FieldKind::Text;
    });
    let list = diff(&a, &b);
    let changed = single(&list, ChangeType::FieldTypeChanged);
    assert_eq!(changed.impact, Impact::NewDataRequested);
    assert_eq!(
        changed.texto,
        "o campo `periodo` da etapa `coleta` mudou de opção para texto: o agente passará a pedir o \
         dado do novo jeito."
    );
}

#[test]
fn option_field_with_other_options_asks_new_value() {
    let a = base();
    let b = com(&a, |ir| {
        in_step(ir, "coleta").campos[1]
            .options
            .push("noite".to_owned());
    });
    let list = diff(&a, &b);
    let changed = single(&list, ChangeType::FieldOptionsChanged);
    assert_eq!(changed.impact, Impact::NewDataRequested);
    assert_eq!(
        changed.texto,
        "as opções do campo `periodo` da etapa `coleta` mudaram de (manhã, tarde) para (manhã, \
         tarde, noite)."
    );
}

#[test]
fn action_new_changes_route() {
    let a = base();
    let b = com(&a, |ir| {
        in_step(ir, "coleta").actions.push(acao(
            "remarcar",
            "Cliente quer remarcar",
            &[],
            None,
            step("confirmacao"),
        ));
    });
    let list = diff(&a, &b);
    let new_stage = single(&list, ChangeType::NewAction);
    assert_eq!(new_stage.impact, Impact::RouteChanged);
    assert_eq!(new_stage.target, "etapa:coleta/acao:remarcar");
    assert_eq!(
        new_stage.texto,
        "a etapa `coleta` ganhou a ação `remarcar` (Cliente quer remarcar), que leva para a etapa \
         `confirmacao` (Confirmar visita)."
    );
}

#[test]
fn action_removed_changes_route() {
    let a = com(&base(), |ir| {
        in_step(ir, "coleta").actions.push(acao(
            "remarcar",
            "Cliente quer remarcar",
            &[],
            None,
            step("confirmacao"),
        ));
    });
    let b = base();
    let list = diff(&a, &b);
    let removed_stage = single(&list, ChangeType::RemovedAction);
    assert_eq!(removed_stage.impact, Impact::RouteChanged);
    assert_eq!(removed_stage.target, "etapa:coleta/acao:remarcar");
    assert_eq!(
        removed_stage.texto,
        "a etapa `coleta` perdeu a ação `remarcar` (Cliente quer remarcar): o agente não poderá \
         mais usá-la."
    );
}

#[test]
fn target_action_changed_changes_route() {
    let a = base();
    let b = com(&a, |ir| {
        in_action(ir, "coleta", "confirmar").goes_to =
            end("concluida", Some("Visita agendada"), false);
    });
    let list = diff(&a, &b);
    let target = single(&list, ChangeType::DestinationChanged);
    assert_eq!(target.impact, Impact::RouteChanged);
    assert_eq!(target.target, "etapa:coleta/acao:confirmar");
    assert_eq!(
        target.texto,
        "a ação `confirmar` da etapa `coleta` agora leva para o fim `concluida`, e antes levava \
         para a etapa `confirmacao` (Confirmar visita)."
    );
}

#[test]
fn action_that_passes_require_field_asks_given() {
    let a = base();
    let b = com(&a, |ir| {
        in_step(ir, "coleta")
            .campos
            .push(campo("cep", "CEP", FieldKind::Text, false, &[]));
        in_action(ir, "coleta", "confirmar")
            .requires
            .push("cep".to_owned());
    });
    let list = diff(&a, &b);
    assert_eq!(
        kinds(&list),
        vec![ChangeType::NewField, ChangeType::ActionNowRequires]
    );
    let requires = single(&list, ChangeType::ActionNowRequires);
    assert_eq!(requires.impact, Impact::NewDataRequested);
    assert_eq!(requires.target, "etapa:coleta/acao:confirmar");
    assert_eq!(
        requires.texto,
        "a ação `confirmar` da etapa `coleta` agora exige o campo `cep` (CEP): o agente passará a \
         pedir esse dado antes de avançar."
    );
}

#[test]
fn action_that_leaves_require_field_applies_for_services_new() {
    let a = base();
    let b = com(&a, |ir| {
        in_action(ir, "coleta", "confirmar")
            .requires
            .retain(|campo| campo != "endereco");
    });
    let list = diff(&a, &b);
    let left = single(&list, ChangeType::ActionNoLongerRequires);
    assert_eq!(left.impact, Impact::NewServices);
    assert_eq!(
        left.texto,
        "a ação `confirmar` da etapa `coleta` deixou de exigir o campo `endereco`: nos atendimentos \
         novos, o agente pode avançar sem ele."
    );
}

#[test]
fn actor_action_changed_changes_route() {
    let a = base();
    let b = com(&a, |ir| {
        in_action(ir, "coleta", "confirmar").actor = Some(ActionActor::Attendant);
    });
    let list = diff(&a, &b);
    let actor = single(&list, ChangeType::ActionActorChanged);
    assert_eq!(actor.impact, Impact::RouteChanged);
    assert_eq!(
        actor.texto,
        "a ação `confirmar` da etapa `coleta` passa a ser feita só pelo atendente."
    );
}

#[test]
fn rule_decision_changed_changes_route() {
    let with_decision = |se: Rule| {
        com(&base(), |ir| {
            ir.decisions.push(decisao(
                "classificar",
                se,
                step("confirmacao"),
                end("desistencia", Some("Cliente desistiu"), false),
            ));
        })
    };
    let a = with_decision(igual("coleta.periodo", "manhã"));
    let b = with_decision(Rule::Diferente {
        campo: "coleta.periodo".to_owned(),
        value: "manhã".to_owned(),
    });
    let list = diff(&a, &b);
    let rule = single(&list, ChangeType::RuleChanged);
    assert_eq!(rule.impact, Impact::RouteChanged);
    assert_eq!(rule.target, "decisao:classificar");
    assert_eq!(
        rule.texto,
        "a condição da decisão `classificar` mudou: agora é verdadeira quando `coleta.periodo` é \
         diferente de \"manhã\"."
    );
}

#[test]
fn branch_decision_that_changes_target_changes_route() {
    let with_decision = |r#else: Destination| {
        com(&base(), |ir| {
            ir.decisions.push(decisao(
                "classificar",
                igual("coleta.periodo", "manhã"),
                step("confirmacao"),
                r#else,
            ));
        })
    };
    let a = with_decision(end("desistencia", Some("Cliente desistiu"), false));
    let b = with_decision(end("concluida", Some("Visita agendada"), false));
    let list = diff(&a, &b);
    let target = single(&list, ChangeType::DestinationChanged);
    assert_eq!(target.impact, Impact::RouteChanged);
    assert_eq!(target.target, "decisao:classificar/ramo:senao");
    assert_eq!(
        target.texto,
        "o ramo falso da decisão `classificar` agora leva para o fim `concluida`, e antes levava \
         para o fim `desistencia`."
    );
}

#[test]
fn branch_true_decision_that_changes_target_changes_route() {
    let with_decision = |then: Destination| {
        com(&base(), |ir| {
            ir.decisions.push(decisao(
                "classificar",
                igual("coleta.periodo", "manhã"),
                then,
                end("desistencia", Some("Cliente desistiu"), false),
            ));
        })
    };
    let a = with_decision(step("confirmacao"));
    let b = with_decision(end("concluida", Some("Visita agendada"), false));
    let list = diff(&a, &b);
    let target = single(&list, ChangeType::DestinationChanged);
    assert_eq!(target.impact, Impact::RouteChanged);
    assert_eq!(target.target, "decisao:classificar/ramo:entao");
    assert_eq!(
        target.texto,
        "o ramo verdadeiro da decisão `classificar` agora leva para o fim `concluida`, e antes \
         levava para a etapa `confirmacao` (Confirmar visita)."
    );
}

#[test]
fn new_decision_without_arrival_does_not_change_conversation() {
    let a = base();
    let b = com(&a, |ir| {
        ir.decisions.push(decisao(
            "classificar",
            igual("coleta.periodo", "manhã"),
            step("confirmacao"),
            end("desistencia", Some("Cliente desistiu"), false),
        ));
    });
    let list = diff(&a, &b);
    let new_stage = single(&list, ChangeType::NewDecision);
    assert_eq!(new_stage.impact, Impact::None);
    assert_eq!(new_stage.target, "decisao:classificar");
    assert_eq!(
        new_stage.texto,
        "a decisão `classificar` é nova, mas nenhuma ação ou decisão leva a ela: nada muda no \
         atendimento."
    );
}

#[test]
fn new_decision_receiving_arrival_changes_route() {
    let a = base();
    let b = com(&a, |ir| {
        ir.decisions.push(decisao(
            "classificar",
            igual("coleta.periodo", "manhã"),
            step("confirmacao"),
            end("desistencia", Some("Cliente desistiu"), false),
        ));
        in_action(ir, "coleta", "confirmar").goes_to = Destination::Decisao {
            id: "classificar".to_owned(),
        };
    });
    let list = diff(&a, &b);
    let new_stage = single(&list, ChangeType::NewDecision);
    assert_eq!(new_stage.impact, Impact::RouteChanged);
    assert_eq!(
        new_stage.texto,
        "a decisão `classificar` é nova e já é destino de alguma ação ou decisão: os atendimentos \
         novos passam por ela."
    );
}

#[test]
fn removed_decision_without_arrival_does_not_change_conversation() {
    let a = com(&base(), |ir| {
        ir.decisions.push(decisao(
            "classificar",
            igual("coleta.periodo", "manhã"),
            step("confirmacao"),
            end("desistencia", Some("Cliente desistiu"), false),
        ));
    });
    let b = base();
    let list = diff(&a, &b);
    let removed_stage = single(&list, ChangeType::RemovedDecision);
    assert_eq!(removed_stage.impact, Impact::None);
    assert_eq!(
        removed_stage.texto,
        "a decisão `classificar` foi removida, mas nenhuma ação ou decisão a usava: nada muda no \
         atendimento."
    );
}

#[test]
fn new_end_without_arrival_does_not_change_conversation() {
    let a = base();
    let b = com(&a, |ir| ir.ends.push(fim("nova_saida", None, false)));
    let list = diff(&a, &b);
    let new_item = single(&list, ChangeType::NewEnd);
    assert_eq!(new_item.impact, Impact::None);
    assert_eq!(new_item.target, "fim:nova_saida");
    assert_eq!(
        new_item.texto,
        "o fim `nova_saida` é novo, mas nenhuma ação ou decisão leva a ele: nada muda no \
         atendimento."
    );
}

#[test]
fn removed_end_without_arrival_does_not_change_conversation() {
    let a = com(&base(), |ir| ir.ends.push(fim("extra", None, false)));
    let b = base();
    let list = diff(&a, &b);
    let removed_item = single(&list, ChangeType::RemovedEnd);
    assert_eq!(removed_item.impact, Impact::None);
    assert_eq!(
        removed_item.texto,
        "o fim `extra` foi removido, mas nenhuma ação ou decisão o usava: nada muda no atendimento."
    );
}

#[test]
fn end_that_starts_closing_service_changes_route() {
    let a = base();
    let b = com(&a, |ir| {
        ir.ends
            .iter_mut()
            .find(|fim| fim.id == "desistencia")
            .expect("o fim existe")
            .ends_service = true;
    });
    let list = diff(&a, &b);
    let changed = single(&list, ChangeType::EndChanged);
    assert_eq!(changed.impact, Impact::RouteChanged);
    assert_eq!(changed.target, "fim:desistencia");
    assert_eq!(
        changed.texto,
        "o fim `desistencia` agora também encerra o atendimento: a conversa e a Ficha são \
         fechadas ao chegar nele."
    );
}

#[test]
fn summary_end_changed_does_not_change_conversation() {
    let a = base();
    let b = com(&a, |ir| {
        ir.ends
            .iter_mut()
            .find(|fim| fim.id == "concluida")
            .expect("o fim existe")
            .resultado = Some("Visita confirmada".to_owned());
    });
    let list = diff(&a, &b);
    let changed = single(&list, ChangeType::EndChanged);
    assert_eq!(changed.impact, Impact::None);
    assert_eq!(
        changed.texto,
        "o resumo do fim `concluida` mudou de \"Visita agendada\" para \"Visita confirmada\"."
    );
}

#[test]
fn special_rule_used_by_decision_changes_route_when_text_changes() {
    let with_def = |fonte: &str| {
        com(&base(), |ir| {
            ir.decisions.push(decisao(
                "classificar",
                Rule::Funcao {
                    nome: "afinidade".to_owned(),
                },
                step("confirmacao"),
                end("desistencia", Some("Cliente desistiu"), false),
            ));
            ir.defs.push(Def {
                nome: "afinidade".to_owned(),
                fonte: fonte.to_owned(),
            });
        })
    };
    let a = with_def("def afinidade(dados):\n    return True\n");
    let b = with_def("def afinidade(dados):\n    return False\n");
    let list = diff(&a, &b);
    let rule = single(&list, ChangeType::RuleChanged);
    assert_eq!(rule.impact, Impact::RouteChanged);
    assert_eq!(rule.target, "def:afinidade");
    assert_eq!(
        rule.texto,
        "o texto da regra especial `afinidade` mudou: as decisões que a usam passam a decidir com \
         a nova versão."
    );
}

#[test]
fn new_special_rule_unused_does_not_change_conversation() {
    let a = base();
    let b = com(&a, |ir| {
        ir.defs.push(Def {
            nome: "sem_uso".to_owned(),
            fonte: "def sem_uso(dados):\n    return True\n".to_owned(),
        });
    });
    let list = diff(&a, &b);
    let new_stage = single(&list, ChangeType::RuleChanged);
    assert_eq!(new_stage.impact, Impact::None);
    assert_eq!(new_stage.target, "def:sem_uso");
    assert_eq!(
        new_stage.texto,
        "a regra especial `sem_uso` é nova, mas nenhuma decisão a usa: nada muda no atendimento."
    );
}

#[test]
fn new_effect_in_action_fires_effect() {
    let a = base();
    let b = com(&a, |ir| {
        in_action(ir, "coleta", "confirmar")
            .effects
            .push(Effect::MessageToCustomer {
                texto: "Visita marcada!".to_owned(),
            });
    });
    let list = diff(&a, &b);
    let new_item = single(&list, ChangeType::NewEffect);
    assert_eq!(new_item.impact, Impact::EffectTriggered);
    assert_eq!(new_item.target, "etapa:coleta/acao:confirmar");
    assert_eq!(
        new_item.texto,
        "a ação `confirmar` da etapa `coleta` passa a enviar ao cliente: \"Visita marcada!\"."
    );
}

#[test]
fn effect_removed_from_action_fires_effect() {
    let a = com(&base(), |ir| {
        in_action(ir, "coleta", "confirmar")
            .effects
            .push(Effect::MessageToCustomer {
                texto: "Visita marcada!".to_owned(),
            });
    });
    let b = base();
    let list = diff(&a, &b);
    let removed_item = single(&list, ChangeType::RemovedEffect);
    assert_eq!(removed_item.impact, Impact::EffectTriggered);
    assert_eq!(
        removed_item.texto,
        "a ação `confirmar` da etapa `coleta` deixa de enviar ao cliente: \"Visita marcada!\"."
    );
}

#[test]
fn entry_effect_and_team_notice_have_own_text() {
    let a = base();
    let b = com(&a, |ir| {
        in_step(ir, "confirmacao")
            .on_enter
            .push(Effect::NotifyWhatsApp {
                para: "+55 11 90000-0000".to_owned(),
                mensagem: "Nova visita".to_owned(),
            });
        in_step(ir, "confirmacao")
            .on_enter
            .push(Effect::Encaminhar {
                departamento: "Comercial".to_owned(),
            });
    });
    let list = diff(&a, &b);
    assert_eq!(
        kinds(&list),
        vec![ChangeType::NewEffect, ChangeType::NewEffect]
    );
    assert_eq!(
        list[0].texto,
        "ao entrar na etapa `confirmacao`, o fluxo passa a enviar à equipe, no número +55 11 \
         90000-0000: \"Nova visita\"."
    );
    assert_eq!(
        list[1].texto,
        "ao entrar na etapa `confirmacao`, o fluxo passa a encaminhar o atendimento ao \
         departamento `Comercial`."
    );
    assert!(list.iter().all(|m| m.impact == Impact::EffectTriggered));
}

#[test]
fn effect_of_new_step_without_arrival_does_not_fire() {
    let a = base();
    let b = com(&a, |ir| {
        let mut sobra = etapa("sobra", "Sobra", None, Vec::new());
        sobra.on_enter.push(Effect::MessageToCustomer {
            texto: "Oi".to_owned(),
        });
        ir.stages.push(sobra);
    });
    let list = diff(&a, &b);
    assert_eq!(kinds(&list), vec![ChangeType::NewStage], "{list:#?}");
    assert_eq!(list[0].impact, Impact::None);
}

#[test]
fn effect_that_already_existed_does_not_appear_again() {
    let with_effect_1 = |ir: &mut Ir| {
        in_action(ir, "coleta", "confirmar")
            .effects
            .push(Effect::MessageToCustomer {
                texto: "Visita marcada!".to_owned(),
            });
    };
    let a = com(&base(), with_effect_1);
    let b = com(&a, |ir| {
        in_action(ir, "coleta", "confirmar")
            .effects
            .push(Effect::Encaminhar {
                departamento: "Comercial".to_owned(),
            });
    });
    let list = diff(&a, &b);
    assert_eq!(kinds(&list), vec![ChangeType::NewEffect], "{list:#?}");
    assert_eq!(
        list[0].texto,
        "a ação `confirmar` da etapa `coleta` passa a encaminhar o atendimento ao departamento \
         `Comercial`."
    );
}

#[test]
fn trigger_changed_applies_to_new_conversations() {
    let a = base();
    let b = com(&a, |ir| {
        ir.gatilho = Gatilho {
            when: "Quando o cliente pedir remarcação.".to_owned(),
            not_when: None,
            examples: Vec::new(),
        };
    });
    let list = diff(&a, &b);
    let gatilho = single(&list, ChangeType::TriggerChanged);
    assert_eq!(gatilho.impact, Impact::NewServices);
    assert_eq!(gatilho.target, "gatilho");
    assert_eq!(
        gatilho.texto,
        "o critério para iniciar este fluxo mudou: nos atendimentos novos, o Jev passa a usar o \
         novo critério."
    );
}

#[test]
fn initial_step_changed_applies_to_new_conversations() {
    let a = base();
    let b = com(&a, |ir| ir.start = "confirmacao".to_owned());
    let list = diff(&a, &b);
    let beginning = single(&list, ChangeType::StartChanged);
    assert_eq!(beginning.impact, Impact::NewServices);
    assert_eq!(beginning.target, "fluxo");
    assert_eq!(
        beginning.texto,
        "os atendimentos novos passam a começar na etapa `confirmacao`, e não mais na etapa \
         `coleta`."
    );
}

#[test]
fn flow_name_changed_does_not_change_conversation() {
    let a = base();
    let b = com(&a, |ir| {
        ir.fluxo = Fluxo {
            id: ir.fluxo.id.clone(),
            nome: "Marcar visita".to_owned(),
            descricao: ir.fluxo.descricao.clone(),
        };
    });
    let list = diff(&a, &b);
    let nome = single(&list, ChangeType::TextChanged);
    assert_eq!(nome.impact, Impact::None);
    assert_eq!(nome.target, "fluxo");
    assert_eq!(
        nome.texto,
        "o nome do fluxo mudou de \"Agendar visita\" para \"Marcar visita\"."
    );
}

#[test]
fn list_order_in_large_change_is_always_same() {
    let a = base();
    let b = com(&a, |ir| {
        ir.start = "confirmacao".to_owned();
        in_step(ir, "coleta").campos[0].required = false;
        in_step(ir, "coleta")
            .campos
            .push(campo("cep", "CEP", FieldKind::Text, false, &[]));
        in_action(ir, "coleta", "confirmar")
            .requires
            .push("cep".to_owned());
        in_action(ir, "coleta", "confirmar").goes_to =
            end("concluida", Some("Visita agendada"), false);
        ir.stages.retain(|etapa| etapa.id != "confirmacao");
    });
    let list = diff(&a, &b);
    assert_eq!(
        kinds(&list),
        vec![
            ChangeType::StartChanged,
            ChangeType::FieldNoLongerRequired,
            ChangeType::NewField,
            ChangeType::ActionNowRequires,
            ChangeType::DestinationChanged,
            ChangeType::RemovedStage,
        ]
    );
    assert_eq!(list, diff(&a, &b), "duas chamadas dão a mesma lista");
}
