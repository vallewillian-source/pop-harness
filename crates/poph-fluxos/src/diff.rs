use serde::{Deserialize, Serialize};

use crate::ir::{Acao, Campo, Decisao, Destination, Effect, Etapa, Fim, Ir, Rule};
use poph_types::{ActionActor, FieldKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Impact {
    #[serde(rename = "nenhum")]
    None,
    #[serde(rename = "atendimentos_novos")]
    NewServices,
    #[serde(rename = "pede_dado_novo")]
    NewDataRequested,
    #[serde(rename = "muda_rota")]
    RouteChanged,
    #[serde(rename = "dispara_efeito")]
    EffectTriggered,
}

impl Impact {
    pub fn code(self) -> &'static str {
        match self {
            Self::None => "nenhum",
            Self::NewServices => "atendimentos_novos",
            Self::NewDataRequested => "pede_dado_novo",
            Self::RouteChanged => "muda_rota",
            Self::EffectTriggered => "dispara_efeito",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeType {
    #[serde(rename = "etapa_nova")]
    NewStage,
    #[serde(rename = "etapa_removida")]
    RemovedStage,
    #[serde(rename = "instrucao_mudou")]
    InstructionChanged,
    #[serde(rename = "atores_da_etapa_mudou")]
    StageActorsChanged,
    #[serde(rename = "campo_novo")]
    NewField,
    #[serde(rename = "campo_removido")]
    RemovedField,
    #[serde(rename = "campo_agora_obrigatorio")]
    FieldNowRequired,
    #[serde(rename = "campo_deixou_de_ser_obrigatorio")]
    FieldNoLongerRequired,
    #[serde(rename = "campo_mudou_tipo")]
    FieldTypeChanged,
    #[serde(rename = "campo_mudou_opcoes")]
    FieldOptionsChanged,
    #[serde(rename = "acao_passou_a_exigir")]
    ActionNowRequires,
    #[serde(rename = "acao_deixou_de_exigir")]
    ActionNoLongerRequires,
    #[serde(rename = "acao_nova")]
    NewAction,
    #[serde(rename = "acao_removida")]
    RemovedAction,
    #[serde(rename = "destino_mudou")]
    DestinationChanged,
    #[serde(rename = "ator_da_acao_mudou")]
    ActionActorChanged,
    #[serde(rename = "regra_mudou")]
    RuleChanged,
    #[serde(rename = "decisao_nova")]
    NewDecision,
    #[serde(rename = "decisao_removida")]
    RemovedDecision,
    #[serde(rename = "fim_novo")]
    NewEnd,
    #[serde(rename = "fim_removido")]
    RemovedEnd,
    #[serde(rename = "fim_mudou")]
    EndChanged,
    #[serde(rename = "efeito_novo")]
    NewEffect,
    #[serde(rename = "efeito_removido")]
    RemovedEffect,
    #[serde(rename = "gatilho_mudou")]
    TriggerChanged,
    #[serde(rename = "inicio_mudou")]
    StartChanged,
    #[serde(rename = "texto_mudou")]
    TextChanged,
}

impl ChangeType {
    pub fn code(self) -> &'static str {
        match self {
            Self::NewStage => "etapa_nova",
            Self::RemovedStage => "etapa_removida",
            Self::InstructionChanged => "instrucao_mudou",
            Self::StageActorsChanged => "atores_da_etapa_mudou",
            Self::NewField => "campo_novo",
            Self::RemovedField => "campo_removido",
            Self::FieldNowRequired => "campo_agora_obrigatorio",
            Self::FieldNoLongerRequired => "campo_deixou_de_ser_obrigatorio",
            Self::FieldTypeChanged => "campo_mudou_tipo",
            Self::FieldOptionsChanged => "campo_mudou_opcoes",
            Self::ActionNowRequires => "acao_passou_a_exigir",
            Self::ActionNoLongerRequires => "acao_deixou_de_exigir",
            Self::NewAction => "acao_nova",
            Self::RemovedAction => "acao_removida",
            Self::DestinationChanged => "destino_mudou",
            Self::ActionActorChanged => "ator_da_acao_mudou",
            Self::RuleChanged => "regra_mudou",
            Self::NewDecision => "decisao_nova",
            Self::RemovedDecision => "decisao_removida",
            Self::NewEnd => "fim_novo",
            Self::RemovedEnd => "fim_removido",
            Self::EndChanged => "fim_mudou",
            Self::NewEffect => "efeito_novo",
            Self::RemovedEffect => "efeito_removido",
            Self::TriggerChanged => "gatilho_mudou",
            Self::StartChanged => "inicio_mudou",
            Self::TextChanged => "texto_mudou",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Change {
    pub tipo: ChangeType,
    #[serde(rename = "impacto")]
    pub impact: Impact,
    #[serde(rename = "alvo")]
    pub target: String,
    pub texto: String,
}

impl Change {
    fn new(tipo: ChangeType, impact: Impact, target: &str, texto: String) -> Self {
        Self {
            tipo,
            impact,
            target: target.to_owned(),
            texto,
        }
    }
}

pub fn diff(old_ir: &Ir, new_ir: &Ir) -> Vec<Change> {
    let mut saida = Vec::new();
    header(old_ir, new_ir, &mut saida);
    stages(old_ir, new_ir, &mut saida);
    decisions(old_ir, new_ir, &mut saida);
    ends(old_ir, new_ir, &mut saida);
    defs(old_ir, new_ir, &mut saida);
    saida
}

fn header(a: &Ir, b: &Ir, out: &mut Vec<Change>) {
    if a.fluxo.nome != b.fluxo.nome {
        out.push(Change::new(
            ChangeType::TextChanged,
            Impact::None,
            "fluxo",
            format!(
                "o nome do fluxo mudou de \"{}\" para \"{}\".",
                a.fluxo.nome, b.fluxo.nome
            ),
        ));
    }
    if a.fluxo.descricao != b.fluxo.descricao {
        out.push(Change::new(
            ChangeType::TextChanged,
            Impact::None,
            "fluxo",
            format!(
                "a descrição do fluxo mudou de {} para {}.",
                text_or_empty(a.fluxo.descricao.as_deref()),
                text_or_empty(b.fluxo.descricao.as_deref())
            ),
        ));
    }
    if a.gatilho != b.gatilho {
        out.push(Change::new(
            ChangeType::TriggerChanged,
            Impact::NewServices,
            "gatilho",
            "o critério para iniciar este fluxo mudou: nos atendimentos novos, o Jev passa a \
             usar o novo critério."
                .to_owned(),
        ));
    }
    if a.start != b.start {
        out.push(Change::new(
            ChangeType::StartChanged,
            Impact::NewServices,
            "fluxo",
            format!(
                "os atendimentos novos passam a começar na etapa `{}`, e não mais na etapa `{}`.",
                b.start, a.start
            ),
        ));
    }
}

fn stages(a: &Ir, b: &Ir, out: &mut Vec<Change>) {
    for new in &b.stages {
        match a.stages.iter().find(|old| old.id == new.id) {
            Some(old) => compare_stage(a, b, old, new, out),
            None => new_stage(b, new, out),
        }
    }
    for old in &a.stages {
        if !b.stages.iter().any(|new| new.id == old.id) {
            removed_stage(a, old, out);
        }
    }
}

fn new_stage(b: &Ir, etapa: &Etapa, out: &mut Vec<Change>) {
    let target = stage_target(&etapa.id);
    let reaches = reachable(b, &etapa.id);
    let (impact, texto) = if reaches {
        (
            Impact::RouteChanged,
            format!(
                "a etapa `{}`{} é nova e já é destino de alguma ação ou decisão: os atendimentos \
                 novos passam por ela.",
                etapa.id,
                parenthesize(&etapa.nome)
            ),
        )
    } else {
        (
            Impact::None,
            format!(
                "a etapa `{}`{} é nova, mas nenhuma ação ou decisão leva a ela: nada muda no \
                 atendimento.",
                etapa.id,
                parenthesize(&etapa.nome)
            ),
        )
    };
    out.push(Change::new(ChangeType::NewStage, impact, &target, texto));
    if reaches {
        stage_effects(out, etapa, true);
    }
}

fn removed_stage(a: &Ir, etapa: &Etapa, out: &mut Vec<Change>) {
    let target = stage_target(&etapa.id);
    let reached = reachable(a, &etapa.id);
    let (impact, texto) = if reached {
        (
            Impact::RouteChanged,
            format!(
                "a etapa `{}`{} foi removida: os atendimentos novos não passam mais por ela.",
                etapa.id,
                parenthesize(&etapa.nome)
            ),
        )
    } else {
        (
            Impact::None,
            format!(
                "a etapa `{}`{} foi removida, mas nenhuma ação ou decisão a usava: nada muda no \
                 atendimento.",
                etapa.id,
                parenthesize(&etapa.nome)
            ),
        )
    };
    out.push(Change::new(
        ChangeType::RemovedStage,
        impact,
        &target,
        texto,
    ));
    if reached {
        stage_effects(out, etapa, false);
    }
}

fn compare_stage(a: &Ir, b: &Ir, ea: &Etapa, eb: &Etapa, out: &mut Vec<Change>) {
    let et = eb.id.as_str();
    let target = stage_target(et);

    if ea.nome != eb.nome {
        out.push(Change::new(
            ChangeType::TextChanged,
            Impact::None,
            &target,
            format!(
                "o nome da etapa `{et}` mudou de \"{}\" para \"{}\".",
                ea.nome, eb.nome
            ),
        ));
    }
    if ea.instruction != eb.instruction {
        out.push(Change::new(
            ChangeType::InstructionChanged,
            Impact::NewServices,
            &target,
            format!(
                "a instrução da etapa `{et}` mudou de {} para {}: vale para os atendimentos novos.",
                text_or_empty(ea.instruction.as_deref()),
                text_or_empty(eb.instruction.as_deref())
            ),
        ));
    }
    if ea.action_actors != eb.action_actors {
        out.push(Change::new(
            ChangeType::StageActorsChanged,
            Impact::RouteChanged,
            &target,
            format!(
                "as ações da etapa `{et}` passam a ser feitas por {}.",
                actors_text(&eb.action_actors)
            ),
        ));
    }

    for cb in &eb.campos {
        match ea.campos.iter().find(|ca| ca.id == cb.id) {
            Some(ca) => compare_field(et, ca, cb, out),
            None => new_field(et, cb, out),
        }
    }
    for ca in &ea.campos {
        if !eb.campos.iter().any(|cb| cb.id == ca.id) {
            removed_field(et, ca, out);
        }
    }

    for ab in &eb.actions {
        match ea.actions.iter().find(|aa| aa.id == ab.id) {
            Some(aa) => compare_action(a, b, eb, aa, ab, out),
            None => new_action(b, eb, ab, out),
        }
    }
    for aa in &ea.actions {
        if !eb.actions.iter().any(|ab| ab.id == aa.id) {
            removed_action(a, ea, aa, out);
        }
    }

    for effect in leftovers(&eb.on_enter, &ea.on_enter) {
        effect_changed(out, Location::Entry(et), effect, true);
    }
    for effect in leftovers(&ea.on_enter, &eb.on_enter) {
        effect_changed(out, Location::Entry(et), effect, false);
    }
}

fn compare_field(et: &str, ca: &Campo, cb: &Campo, out: &mut Vec<Change>) {
    let cid = cb.id.as_str();
    let target = format!("etapa:{et}/campo:{cid}");

    if ca.label != cb.label {
        out.push(Change::new(
            ChangeType::TextChanged,
            Impact::None,
            &target,
            format!(
                "o rótulo do campo `{cid}` na etapa `{et}` mudou de \"{}\" para \"{}\": o agente \
                 passa a usar o novo nome ao perguntar.",
                ca.label, cb.label
            ),
        ));
    }
    if ca.tipo != cb.tipo {
        out.push(Change::new(
            ChangeType::FieldTypeChanged,
            Impact::NewDataRequested,
            &target,
            format!(
                "o campo `{cid}` da etapa `{et}` mudou de {} para {}: o agente passará a pedir o \
                 dado do novo jeito.",
                field_type_text(ca.tipo),
                field_type_text(cb.tipo)
            ),
        ));
    }
    if ca.options != cb.options {
        out.push(Change::new(
            ChangeType::FieldOptionsChanged,
            Impact::NewDataRequested,
            &target,
            format!(
                "as opções do campo `{cid}` da etapa `{et}` mudaram de {} para {}.",
                options_text(&ca.options),
                options_text(&cb.options)
            ),
        ));
    }
    if !ca.required && cb.required {
        out.push(Change::new(
            ChangeType::FieldNowRequired,
            Impact::NewDataRequested,
            &target,
            format!(
                "o campo `{cid}` da etapa `{et}` agora é obrigatório: o agente vai pedi-lo antes \
                 de concluir a etapa."
            ),
        ));
    }
    if ca.required && !cb.required {
        out.push(Change::new(
            ChangeType::FieldNoLongerRequired,
            Impact::NewServices,
            &target,
            format!(
                "o campo `{cid}` da etapa `{et}` deixou de ser obrigatório: nos atendimentos \
                 novos, a etapa pode ser concluída sem ele."
            ),
        ));
    }
    if ca.help != cb.help {
        out.push(Change::new(
            ChangeType::TextChanged,
            Impact::None,
            &target,
            format!(
                "a dica do campo `{cid}` na etapa `{et}` mudou: ela é só para quem edita o \
                 fluxo, o atendimento segue igual."
            ),
        ));
    }
}

fn new_field(et: &str, campo: &Campo, out: &mut Vec<Change>) {
    let cid = campo.id.as_str();
    let required = if campo.required {
        " Ele é obrigatório."
    } else {
        ""
    };
    out.push(Change::new(
        ChangeType::NewField,
        Impact::NewDataRequested,
        &format!("etapa:{et}/campo:{cid}"),
        format!(
            "a etapa `{et}` passa a coletar o campo `{cid}`{}: o agente pedirá esse dado nos \
             atendimentos novos.{required}",
            parenthesize(&campo.label)
        ),
    ));
}

fn removed_field(et: &str, campo: &Campo, out: &mut Vec<Change>) {
    let cid = campo.id.as_str();
    out.push(Change::new(
        ChangeType::RemovedField,
        Impact::NewServices,
        &format!("etapa:{et}/campo:{cid}"),
        format!(
            "a etapa `{et}` deixa de coletar o campo `{cid}`{}: nos atendimentos novos, o agente \
             não pedirá esse dado.",
            parenthesize(&campo.label)
        ),
    ));
}

fn compare_action(a: &Ir, b: &Ir, et_b: &Etapa, aa: &Acao, ab: &Acao, out: &mut Vec<Change>) {
    let et = et_b.id.as_str();
    let ac = ab.id.as_str();
    let target = format!("etapa:{et}/acao:{ac}");

    if aa.label != ab.label {
        out.push(Change::new(
            ChangeType::TextChanged,
            Impact::None,
            &target,
            format!(
                "o rótulo da ação `{ac}` da etapa `{et}` mudou de \"{}\" para \"{}\".",
                aa.label, ab.label
            ),
        ));
    }
    for campo in &ab.requires {
        if !aa.requires.contains(campo) {
            out.push(Change::new(
                ChangeType::ActionNowRequires,
                Impact::NewDataRequested,
                &target,
                format!(
                    "a ação `{ac}` da etapa `{et}` agora exige o campo `{campo}`{}: o agente \
                     passará a pedir esse dado antes de avançar.",
                    parenthesize(field_label(et_b, campo))
                ),
            ));
        }
    }
    for campo in &aa.requires {
        if !ab.requires.contains(campo) {
            out.push(Change::new(
                ChangeType::ActionNoLongerRequires,
                Impact::NewServices,
                &target,
                format!(
                    "a ação `{ac}` da etapa `{et}` deixou de exigir o campo `{campo}`: nos \
                     atendimentos novos, o agente pode avançar sem ele."
                ),
            ));
        }
    }
    if aa.actor != ab.actor {
        let texto = match ab.actor {
            Some(actor) => format!(
                "a ação `{ac}` da etapa `{et}` passa a ser feita só pelo {}.",
                actor.as_text()
            ),
            None => format!(
                "a ação `{ac}` da etapa `{et}` passa a poder ser feita por qualquer ator da etapa."
            ),
        };
        out.push(Change::new(
            ChangeType::ActionActorChanged,
            Impact::RouteChanged,
            &target,
            texto,
        ));
    }
    if destination_key(&aa.goes_to) != destination_key(&ab.goes_to) {
        out.push(Change::new(
            ChangeType::DestinationChanged,
            Impact::RouteChanged,
            &target,
            format!(
                "a ação `{ac}` da etapa `{et}` agora leva para {}, e antes levava para {}.",
                destination_phrase(b, &ab.goes_to),
                destination_phrase(a, &aa.goes_to)
            ),
        ));
    }
    for effect in leftovers(&ab.effects, &aa.effects) {
        effect_changed(out, Location::Acao(et, ac), effect, true);
    }
    for effect in leftovers(&aa.effects, &ab.effects) {
        effect_changed(out, Location::Acao(et, ac), effect, false);
    }
}

fn new_action(b: &Ir, etapa: &Etapa, acao: &Acao, out: &mut Vec<Change>) {
    let et = etapa.id.as_str();
    let ac = acao.id.as_str();
    let reaches = reachable(b, et);
    let (impact, texto) = if reaches {
        (
            Impact::RouteChanged,
            format!(
                "a etapa `{et}` ganhou a ação `{ac}`{}, que leva para {}.",
                parenthesize(&acao.label),
                destination_phrase(b, &acao.goes_to)
            ),
        )
    } else {
        (
            Impact::None,
            format!(
                "a etapa `{et}` ganhou a ação `{ac}`{}, mas a etapa não é alcançada por nenhuma \
                 ação ou decisão: nada muda no atendimento.",
                parenthesize(&acao.label)
            ),
        )
    };
    out.push(Change::new(
        ChangeType::NewAction,
        impact,
        &format!("etapa:{et}/acao:{ac}"),
        texto,
    ));
    if reaches {
        for effect in &acao.effects {
            effect_changed(out, Location::Acao(et, ac), effect, true);
        }
    }
}

fn removed_action(a: &Ir, etapa: &Etapa, acao: &Acao, out: &mut Vec<Change>) {
    let et = etapa.id.as_str();
    let ac = acao.id.as_str();
    let reached = reachable(a, et);
    let (impact, texto) = if reached {
        (
            Impact::RouteChanged,
            format!(
                "a etapa `{et}` perdeu a ação `{ac}`{}: o agente não poderá mais usá-la.",
                parenthesize(&acao.label)
            ),
        )
    } else {
        (
            Impact::None,
            format!(
                "a etapa `{et}` perdeu a ação `{ac}`{}, mas a etapa não era alcançada por nenhuma \
                 ação ou decisão: nada muda no atendimento.",
                parenthesize(&acao.label)
            ),
        )
    };
    out.push(Change::new(
        ChangeType::RemovedAction,
        impact,
        &format!("etapa:{et}/acao:{ac}"),
        texto,
    ));
    if reached {
        for effect in &acao.effects {
            effect_changed(out, Location::Acao(et, ac), effect, false);
        }
    }
}

fn decisions(a: &Ir, b: &Ir, out: &mut Vec<Change>) {
    for new in &b.decisions {
        match a.decisions.iter().find(|old| old.id == new.id) {
            Some(old) => compare_decision(a, b, old, new, out),
            None => new_decision(b, new, out),
        }
    }
    for old in &a.decisions {
        if !b.decisions.iter().any(|new| new.id == old.id) {
            removed_decision(a, old, out);
        }
    }
}

fn compare_decision(a: &Ir, b: &Ir, old: &Decisao, db: &Decisao, out: &mut Vec<Change>) {
    let id = db.id.as_str();
    let target = format!("decisao:{id}");
    if old.se != db.se {
        out.push(Change::new(
            ChangeType::RuleChanged,
            Impact::RouteChanged,
            &target,
            format!(
                "a condição da decisão `{id}` mudou: agora é verdadeira quando {}.",
                rule_text(&db.se)
            ),
        ));
    }
    if destination_key(&old.then_branch) != destination_key(&db.then_branch) {
        out.push(Change::new(
            ChangeType::DestinationChanged,
            Impact::RouteChanged,
            &format!("{target}/ramo:entao"),
            format!(
                "o ramo verdadeiro da decisão `{id}` agora leva para {}, e antes levava para {}.",
                destination_phrase(b, &db.then_branch),
                destination_phrase(a, &old.then_branch)
            ),
        ));
    }
    if destination_key(&old.else_branch) != destination_key(&db.else_branch) {
        out.push(Change::new(
            ChangeType::DestinationChanged,
            Impact::RouteChanged,
            &format!("{target}/ramo:senao"),
            format!(
                "o ramo falso da decisão `{id}` agora leva para {}, e antes levava para {}.",
                destination_phrase(b, &db.else_branch),
                destination_phrase(a, &old.else_branch)
            ),
        ));
    }
}

fn new_decision(b: &Ir, decisao: &Decisao, out: &mut Vec<Change>) {
    let id = decisao.id.as_str();
    let (impact, texto) = if reachable(b, id) {
        (
            Impact::RouteChanged,
            format!(
                "a decisão `{id}` é nova e já é destino de alguma ação ou decisão: os atendimentos \
                 novos passam por ela."
            ),
        )
    } else {
        (
            Impact::None,
            format!(
                "a decisão `{id}` é nova, mas nenhuma ação ou decisão leva a ela: nada muda no \
                 atendimento."
            ),
        )
    };
    out.push(Change::new(
        ChangeType::NewDecision,
        impact,
        &format!("decisao:{id}"),
        texto,
    ));
}

fn removed_decision(a: &Ir, decisao: &Decisao, out: &mut Vec<Change>) {
    let id = decisao.id.as_str();
    let (impact, texto) = if reachable(a, id) {
        (
            Impact::RouteChanged,
            format!(
                "a decisão `{id}` foi removida: os atendimentos novos não passam mais por ela."
            ),
        )
    } else {
        (
            Impact::None,
            format!(
                "a decisão `{id}` foi removida, mas nenhuma ação ou decisão a usava: nada muda no \
                 atendimento."
            ),
        )
    };
    out.push(Change::new(
        ChangeType::RemovedDecision,
        impact,
        &format!("decisao:{id}"),
        texto,
    ));
}

fn ends(a: &Ir, b: &Ir, out: &mut Vec<Change>) {
    for new in &b.ends {
        match a.ends.iter().find(|old| old.id == new.id) {
            Some(old) => compare_end(old, new, out),
            None => new_end(b, new, out),
        }
    }
    for old in &a.ends {
        if !b.ends.iter().any(|new| new.id == old.id) {
            removed_end(a, old, out);
        }
    }
}

fn compare_end(old: &Fim, new: &Fim, out: &mut Vec<Change>) {
    let id = new.id.as_str();
    let target = format!("fim:{id}");
    if old.ends_service != new.ends_service {
        let texto = if new.ends_service {
            format!(
                "o fim `{id}` agora também encerra o atendimento: a conversa e a Ficha são \
                 fechadas ao chegar nele."
            )
        } else {
            format!("o fim `{id}` deixou de encerrar o atendimento: a conversa continua aberta.")
        };
        out.push(Change::new(
            ChangeType::EndChanged,
            Impact::RouteChanged,
            &target,
            texto,
        ));
    }
    if old.resultado != new.resultado {
        out.push(Change::new(
            ChangeType::EndChanged,
            Impact::None,
            &target,
            format!(
                "o resumo do fim `{id}` mudou de {} para {}.",
                text_or_empty(old.resultado.as_deref()),
                text_or_empty(new.resultado.as_deref())
            ),
        ));
    }
}

fn new_end(b: &Ir, fim: &Fim, out: &mut Vec<Change>) {
    let id = fim.id.as_str();
    let (impact, texto) = if reachable(b, id) {
        (
            Impact::RouteChanged,
            format!(
                "o fim `{id}` é novo e já é destino de alguma ação ou decisão: os atendimentos \
                 novos podem terminar nele."
            ),
        )
    } else {
        (
            Impact::None,
            format!(
                "o fim `{id}` é novo, mas nenhuma ação ou decisão leva a ele: nada muda no \
                 atendimento."
            ),
        )
    };
    out.push(Change::new(
        ChangeType::NewEnd,
        impact,
        &format!("fim:{id}"),
        texto,
    ));
}

fn removed_end(a: &Ir, fim: &Fim, out: &mut Vec<Change>) {
    let id = fim.id.as_str();
    let (impact, texto) = if reachable(a, id) {
        (
            Impact::RouteChanged,
            format!("o fim `{id}` foi removido: os atendimentos novos não terminam mais nele."),
        )
    } else {
        (
            Impact::None,
            format!(
                "o fim `{id}` foi removido, mas nenhuma ação ou decisão o usava: nada muda no \
                 atendimento."
            ),
        )
    };
    out.push(Change::new(
        ChangeType::RemovedEnd,
        impact,
        &format!("fim:{id}"),
        texto,
    ));
}

fn defs(a: &Ir, b: &Ir, out: &mut Vec<Change>) {
    for new in &b.defs {
        let target = format!("def:{}", new.nome);
        match a.defs.iter().find(|old| old.nome == new.nome) {
            None => {
                let used = used_by_decision(b, &new.nome);
                let (impact, texto) = if used {
                    (
                        Impact::RouteChanged,
                        format!(
                            "a regra especial `{}` é nova e já é usada por decisão: a condição \
                             dessas decisões passa a usá-la.",
                            new.nome
                        ),
                    )
                } else {
                    (
                        Impact::None,
                        format!(
                            "a regra especial `{}` é nova, mas nenhuma decisão a usa: nada muda \
                             no atendimento.",
                            new.nome
                        ),
                    )
                };
                out.push(Change::new(ChangeType::RuleChanged, impact, &target, texto));
            }
            Some(old) if old.fonte != new.fonte => {
                let used = used_by_decision(b, &new.nome);
                let (impact, texto) = if used {
                    (
                        Impact::RouteChanged,
                        format!(
                            "o texto da regra especial `{}` mudou: as decisões que a usam passam \
                             a decidir com a nova versão.",
                            new.nome
                        ),
                    )
                } else {
                    (
                        Impact::None,
                        format!(
                            "o texto da regra especial `{}` mudou, mas nenhuma decisão a usa: \
                             nada muda no atendimento.",
                            new.nome
                        ),
                    )
                };
                out.push(Change::new(ChangeType::RuleChanged, impact, &target, texto));
            }
            Some(_) => {}
        }
    }
    for old in &a.defs {
        if !b.defs.iter().any(|new| new.nome == old.nome) {
            let used = used_by_decision(a, &old.nome);
            let (impact, texto) = if used {
                (
                    Impact::RouteChanged,
                    format!(
                        "a regra especial `{}` foi removida: as decisões que a usavam deixam de \
                         ter essa condição.",
                        old.nome
                    ),
                )
            } else {
                (
                    Impact::None,
                    format!(
                        "a regra especial `{}` foi removida; nenhuma decisão a usava.",
                        old.nome
                    ),
                )
            };
            out.push(Change::new(
                ChangeType::RuleChanged,
                impact,
                &format!("def:{}", old.nome),
                texto,
            ));
        }
    }
}

#[derive(Clone, Copy)]
enum Location<'a> {
    Entry(&'a str),
    Acao(&'a str, &'a str),
}

fn stage_effects(out: &mut Vec<Change>, etapa: &Etapa, new: bool) {
    for effect in &etapa.on_enter {
        effect_changed(out, Location::Entry(&etapa.id), effect, new);
    }
    for acao in &etapa.actions {
        for effect in &acao.effects {
            effect_changed(out, Location::Acao(&etapa.id, &acao.id), effect, new);
        }
    }
}

fn effect_changed(out: &mut Vec<Change>, location: Location, effect: &Effect, new: bool) {
    let (tipo, verbo) = if new {
        (ChangeType::NewEffect, "passa a")
    } else {
        (ChangeType::RemovedEffect, "deixa de")
    };
    let descricao = describe_effect(effect);
    let (target, texto) = match location {
        Location::Entry(et) => (
            stage_target(et),
            format!("ao entrar na etapa `{et}`, o fluxo {verbo} {descricao}."),
        ),
        Location::Acao(et, ac) => (
            format!("etapa:{et}/acao:{ac}"),
            format!("a ação `{ac}` da etapa `{et}` {verbo} {descricao}."),
        ),
    };
    out.push(Change::new(tipo, Impact::EffectTriggered, &target, texto));
}

fn describe_effect(effect: &Effect) -> String {
    match effect {
        Effect::MessageToCustomer { texto } => format!("enviar ao cliente: \"{texto}\""),
        Effect::NotifyWhatsApp { para, mensagem } => {
            format!("enviar à equipe, no número {para}: \"{mensagem}\"")
        }
        Effect::Encaminhar { departamento } => {
            format!("encaminhar o atendimento ao departamento `{departamento}`")
        }
    }
}

fn leftovers<'a, T: PartialEq>(source: &'a [T], other: &[T]) -> Vec<&'a T> {
    let mut free: Vec<&T> = other.iter().collect();
    let mut sobra = Vec::new();
    for item in source {
        match free.iter().position(|free| *free == item) {
            Some(position) => {
                free.swap_remove(position);
            }
            None => sobra.push(item),
        }
    }
    sobra
}

fn reachable(ir: &Ir, id: &str) -> bool {
    ir.start == id
        || ir
            .stages
            .iter()
            .flat_map(|etapa| etapa.actions.iter())
            .any(|acao| acao.goes_to.id() == id)
        || ir
            .decisions
            .iter()
            .any(|decisao| decisao.then_branch.id() == id || decisao.else_branch.id() == id)
}

fn used_by_decision(ir: &Ir, nome: &str) -> bool {
    ir.decisions
        .iter()
        .any(|decisao| rule_calls(&decisao.se, nome))
}

fn rule_calls(rule: &Rule, nome: &str) -> bool {
    match rule {
        Rule::Funcao { nome: call } => call == nome,
        Rule::E { args } | Rule::Ou { args } | Rule::Nao { args } => {
            args.iter().any(|child| rule_calls(child, nome))
        }
        Rule::Igual { .. }
        | Rule::Diferente { .. }
        | Rule::Maior { .. }
        | Rule::Menor { .. }
        | Rule::Contem { .. }
        | Rule::Vazio { .. } => false,
    }
}

fn destination_key(destination: &Destination) -> (&'static str, &str) {
    match destination {
        Destination::Etapa { id } => ("etapa", id),
        Destination::Decisao { id } => ("decisao", id),
        Destination::Fim { id, .. } => ("fim", id),
    }
}

fn destination_phrase(ir: &Ir, destination: &Destination) -> String {
    match destination {
        Destination::Etapa { id } => match ir.stages.iter().find(|etapa| etapa.id == *id) {
            Some(etapa) => format!("a etapa `{id}`{}", parenthesize(&etapa.nome)),
            None => format!("a etapa `{id}`"),
        },
        Destination::Decisao { id } => format!("a decisão `{id}`"),
        Destination::Fim {
            id, ends_service, ..
        } => {
            if *ends_service {
                format!("o fim `{id}`, que encerra o atendimento")
            } else {
                format!("o fim `{id}`")
            }
        }
    }
}

fn rule_text(rule: &Rule) -> String {
    match rule {
        Rule::Igual { campo, value } => format!("`{campo}` é igual a \"{value}\""),
        Rule::Diferente { campo, value } => format!("`{campo}` é diferente de \"{value}\""),
        Rule::Maior { campo, value } => format!("`{campo}` é maior que {value}"),
        Rule::Menor { campo, value } => format!("`{campo}` é menor que {value}"),
        Rule::Contem { campo, value } => format!("`{campo}` contém \"{value}\""),
        Rule::Vazio { campo } => format!("`{campo}` está vazio"),
        Rule::E { args } => format!("({})", join(args, " e ")),
        Rule::Ou { args } => format!("({})", join(args, " ou ")),
        Rule::Nao { args } => format!("não ({})", join(args, " e ")),
        Rule::Funcao { nome } => format!("a regra especial `{nome}` for verdadeira"),
    }
}

fn join(rules: &[Rule], separador: &str) -> String {
    rules
        .iter()
        .map(rule_text)
        .collect::<Vec<_>>()
        .join(separador)
}

fn stage_target(id: &str) -> String {
    format!("etapa:{id}")
}

fn field_label<'a>(etapa: &'a Etapa, campo: &str) -> &'a str {
    etapa
        .campos
        .iter()
        .find(|candidate| candidate.id == campo)
        .map_or("", |candidate| candidate.label.as_str())
}

fn parenthesize(texto: &str) -> String {
    if texto.trim().is_empty() {
        String::new()
    } else {
        format!(" ({texto})")
    }
}

fn text_or_empty(texto: Option<&str>) -> String {
    match texto {
        Some(value) => format!("\"{value}\""),
        None => "(vazio)".to_owned(),
    }
}

fn actors_text(actors: &[ActionActor]) -> String {
    if actors.is_empty() {
        return "ninguém (a etapa não define ator)".to_owned();
    }
    actors
        .iter()
        .map(|actor| actor.as_text())
        .collect::<Vec<_>>()
        .join(" ou ")
}

fn options_text(options: &[String]) -> String {
    if options.is_empty() {
        "(nenhuma)".to_owned()
    } else {
        format!("({})", options.join(", "))
    }
}

fn field_type_text(tipo: FieldKind) -> &'static str {
    match tipo {
        FieldKind::Text => "texto",
        FieldKind::Number => "número",
        FieldKind::Data => "data",
        FieldKind::Boolean => "sim ou não",
        FieldKind::Attachment => "anexo",
        FieldKind::Option => "opção",
    }
}
