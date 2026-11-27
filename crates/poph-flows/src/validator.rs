use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use poph_types::FieldKind;

use crate::diagnostics::{Code, Diagnostic};
use crate::ir::{Destination, Effect, Etapa, Ir, Rule};

pub const MAX_NOS: usize = crate::compilador::form::MAX_NOS;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Departamento {
    pub id: String,
    pub nome: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Resolution {
    pub nome: String,
    #[serde(rename = "departamento_id")]
    pub department_id: Option<String>,
}

pub fn validar(ir: &Ir) -> Vec<Diagnostic> {
    let mut validator = Validator::new(ir);
    validator.check_stages();
    validator.check_decisions();
    validator.check_ends();
    validator.check_start();
    validator.check_cycle();
    validator.check_reachability();
    validator.check_limit();
    validator.check_field_usage();
    validator.check_def_usage();
    validator.diagnostics
}

pub fn resolve_departments(
    ir: &Ir,
    departments: &[Departamento],
) -> (Vec<Resolution>, Vec<Diagnostic>) {
    let mut citations: Vec<(String, String)> = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut citar = |nome: &str, path: String| {
        let chave = same_key(nome);
        if seen.insert(chave) {
            citations.push((nome.trim().to_owned(), path));
        }
    };

    for (i, etapa) in ir.stages.iter().enumerate() {
        let base = stage_path(i, etapa);
        for (j, effect) in etapa.on_enter.iter().enumerate() {
            if let Effect::Encaminhar { departamento } = effect {
                citar(departamento, sub(&base, &format!("ao_entrar[{j}]")));
            }
        }
        for (j, acao) in etapa.actions.iter().enumerate() {
            let path_action = sub(&base, &format!("acoes[{j}]"));
            for (k, effect) in acao.effects.iter().enumerate() {
                if let Effect::Encaminhar { departamento } = effect {
                    citar(departamento, sub(&path_action, &format!("efeitos[{k}]")));
                }
            }
        }
    }

    let mut resolutions = Vec::with_capacity(citations.len());
    let mut diagnostics = Vec::new();
    for (nome, path) in citations {
        let findings: Vec<&Departamento> = departments
            .iter()
            .filter(|departamento| same_key(&departamento.nome) == same_key(&nome))
            .collect();
        let resolution = match findings.as_slice() {
            [single] => Resolution {
                nome,
                department_id: Some(single.id.clone()),
            },
            [] => {
                diagnostics.push(Diagnostic::warning_at(
                    Code::UnknownDepartment,
                    format!(
                        "O encaminhamento manda o atendimento para o departamento `{nome}`, que não existe nesta organização: o efeito vai falhar no atendimento. Confira o nome do departamento."
                    ),
                    path,
                ));
                Resolution {
                    nome,
                    department_id: None,
                }
            }
            several => {
                diagnostics.push(Diagnostic::warning_at(
                    Code::UnknownDepartment,
                    format!(
                        "O encaminhamento manda o atendimento para `{nome}`, mas há {} departamentos com esse nome na organização: renomeie um deles para o encaminhamento ter destino. O efeito vai falhar no atendimento.",
                        several.len()
                    ),
                    path,
                ));
                Resolution {
                    nome,
                    department_id: None,
                }
            }
        };
        resolutions.push(resolution);
    }

    (resolutions, diagnostics)
}

fn stage_path(index: usize, etapa: &Etapa) -> String {
    format!("etapas[{index}] (id \"{}\")", etapa.id)
}

fn decision_path(index: usize, decisao: &crate::ir::Decisao) -> String {
    format!("decisoes[{index}] (id \"{}\")", decisao.id)
}

fn sub(base: &str, piece: &str) -> String {
    if base.is_empty() {
        piece.to_owned()
    } else {
        format!("{base} > {piece}")
    }
}

fn same_key(nome: &str) -> String {
    nome.trim().to_lowercase()
}

struct Validator<'a> {
    ir: &'a Ir,
    stages: BTreeMap<&'a str, usize>,
    decisions: BTreeMap<&'a str, usize>,
    ends: BTreeMap<&'a str, usize>,
    defs: BTreeMap<&'a str, usize>,
    read: BTreeMap<&'a str, BTreeSet<&'a str>>,
    required: BTreeMap<&'a str, BTreeSet<&'a str>>,
    used: BTreeSet<&'a str>,
    diagnostics: Vec<Diagnostic>,
}

impl<'a> Validator<'a> {
    fn new(ir: &'a Ir) -> Self {
        let mut validator = Self {
            ir,
            stages: BTreeMap::new(),
            decisions: BTreeMap::new(),
            ends: BTreeMap::new(),
            defs: BTreeMap::new(),
            read: BTreeMap::new(),
            required: BTreeMap::new(),
            used: BTreeSet::new(),
            diagnostics: Vec::new(),
        };
        for (i, etapa) in ir.stages.iter().enumerate() {
            validator.stages.entry(etapa.id.as_str()).or_insert(i);
        }
        for (i, decisao) in ir.decisions.iter().enumerate() {
            validator.decisions.entry(decisao.id.as_str()).or_insert(i);
        }
        for (i, fim) in ir.ends.iter().enumerate() {
            validator.ends.entry(fim.id.as_str()).or_insert(i);
        }
        for (i, def) in ir.defs.iter().enumerate() {
            validator.defs.entry(def.nome.as_str()).or_insert(i);
        }
        validator
    }

    fn error_at(&mut self, code: Code, mensagem: impl Into<String>, path: impl Into<String>) {
        self.diagnostics
            .push(Diagnostic::error_at(code, mensagem, path));
    }

    fn warning_at(&mut self, code: Code, mensagem: impl Into<String>, path: impl Into<String>) {
        self.diagnostics
            .push(Diagnostic::warning_at(code, mensagem, path));
    }

    fn check_stages(&mut self) {
        let ir = self.ir;
        for (i, etapa) in ir.stages.iter().enumerate() {
            let base = stage_path(i, etapa);
            if self.stages.get(etapa.id.as_str()) != Some(&i) {
                self.error_at(
                    Code::DuplicateId,
                    format!(
                        "O id `{}` está declarado duas vezes. Cada etapa, decisão, campo e ação tem id único no seu escopo.",
                        etapa.id
                    ),
                    base.clone(),
                );
            } else if self.decisions.contains_key(etapa.id.as_str()) {
                self.error_at(
                    Code::DuplicateId,
                    format!(
                        "O id `{}` é de uma etapa e de uma decisão ao mesmo tempo: um destino não saberia para onde ir. Use ids diferentes.",
                        etapa.id
                    ),
                    base.clone(),
                );
            }

            self.check_fields(etapa, &base);
            self.check_actions(etapa, &base);
            self.check_variables(etapa, &base);

            if etapa.actions.is_empty() {
                self.warning_at(
                    Code::StageWithoutAction,
                    format!(
                        "A etapa `{}` não tem nenhuma `acao(...)`: o atendimento não sai dela. Acrescente uma ação com `vai_para`.",
                        etapa.id
                    ),
                    base.clone(),
                );
            }

            if !etapa.actions.is_empty() && etapa.actions.iter().all(|acao| acao.actor.is_some()) {
                for actor in &etapa.action_actors {
                    if etapa.actions.iter().any(|acao| acao.actor == Some(*actor)) {
                        continue;
                    }
                    self.warning_at(
                        Code::ActorWithoutAction,
                        format!(
                            "A etapa `{}` permite o `{}`, mas nenhuma ação é dele: na prática ele não tem o que fazer aqui.",
                            etapa.id,
                            actor.as_text()
                        ),
                        sub(&base, "atores_acao"),
                    );
                }
            }
        }
    }

    fn check_fields(&mut self, etapa: &'a Etapa, base: &str) {
        let mut ids: BTreeSet<&str> = BTreeSet::new();
        for (j, campo) in etapa.campos.iter().enumerate() {
            let path = sub(base, &format!("campos[{j}]"));
            if !ids.insert(campo.id.as_str()) {
                self.error_at(
                    Code::DuplicateId,
                    format!(
                        "O campo `{}` está declarado duas vezes na etapa `{}`.",
                        campo.id, etapa.id
                    ),
                    path.clone(),
                );
            }
            match campo.tipo {
                FieldKind::Option => {
                    if campo.options.len() < 2 {
                        self.error_at(
                            Code::InvalidOptions,
                            format!(
                                "O campo `{}.{}` é do tipo `opcao` e precisa de pelo menos duas opções; hoje tem {}.",
                                etapa.id,
                                campo.id,
                                campo.options.len()
                            ),
                            path,
                        );
                    } else if duplicates(&campo.options) {
                        self.error_at(
                            Code::InvalidOptions,
                            format!(
                                "O campo `{}.{}` repete uma opção: as opções precisam ser diferentes entre si.",
                                etapa.id, campo.id
                            ),
                            path,
                        );
                    }
                }
                _ if !campo.options.is_empty() => {
                    self.error_at(
                        Code::InvalidOptions,
                        format!(
                            "O campo `{}.{}` é do tipo `{}` e não aceita `opcoes`: isso só vale no tipo `opcao`.",
                            etapa.id,
                            campo.id,
                            campo.tipo.as_text()
                        ),
                        path,
                    );
                }
                _ => {}
            }
        }
    }

    fn check_actions(&mut self, etapa: &'a Etapa, base: &str) {
        let mut ids: BTreeSet<&str> = BTreeSet::new();
        for (j, acao) in etapa.actions.iter().enumerate() {
            let path = sub(base, &format!("acoes[{j}]"));
            if !ids.insert(acao.id.as_str()) {
                self.error_at(
                    Code::DuplicateId,
                    format!(
                        "A ação `{}` está declarada duas vezes na etapa `{}`.",
                        acao.id, etapa.id
                    ),
                    path.clone(),
                );
            }
            let context = format!("A ação `{}.{}`", etapa.id, acao.id);

            for required in &acao.requires {
                if etapa.campos.iter().any(|campo| campo.id == *required) {
                    self.required
                        .entry(etapa.id.as_str())
                        .or_default()
                        .insert(required.as_str());
                } else {
                    self.error_at(
                        Code::UnknownRequirement,
                        format!(
                            "{context} exige `{required}`, que não é campo da etapa `{}`.",
                            etapa.id
                        ),
                        sub(&path, "exige"),
                    );
                }
            }

            if let Some(actor) = acao.actor
                && !etapa.action_actors.contains(&actor)
            {
                let allowed: Vec<&str> = etapa.action_actors.iter().map(|a| a.as_text()).collect();
                self.error_at(
                    Code::InvalidRuleOrActor,
                    format!(
                        "{context} é do ator `{}`, que não está em `atores` da etapa `{}` (lá estão: {}).",
                        actor.as_text(),
                        etapa.id,
                        if allowed.is_empty() {
                            "nenhum".to_owned()
                        } else {
                            allowed.join(", ")
                        }
                    ),
                    sub(&path, "ator"),
                );
            }

            self.check_destination(&acao.goes_to, &sub(&path, "vai_para"), &context);
        }
    }

    fn check_variables(&mut self, etapa: &'a Etapa, base: &str) {
        for (j, effect) in etapa.on_enter.iter().enumerate() {
            let path = sub(base, &format!("ao_entrar[{j}]"));
            self.check_effect_variables(effect, &path, Some(etapa));
        }
        for (j, acao) in etapa.actions.iter().enumerate() {
            let path_action = sub(base, &format!("acoes[{j}]"));
            for (k, effect) in acao.effects.iter().enumerate() {
                let path = sub(&path_action, &format!("efeitos[{k}]"));
                self.check_effect_variables(effect, &path, None);
            }
        }
    }

    fn check_effect_variables(
        &mut self,
        effect: &Effect,
        path: &str,
        to_enter_of_stage: Option<&'a Etapa>,
    ) {
        let (texto, campo, what) = match effect {
            Effect::MessageToCustomer { texto } => (texto.as_str(), "texto", "mensagem ao cliente"),
            Effect::NotifyWhatsApp { mensagem, .. } => {
                (mensagem.as_str(), "mensagem", "notificação para a equipe")
            }
            Effect::Encaminhar { .. } => return,
        };
        let path = sub(path, campo);
        let ir = self.ir;
        for variable in crate::compilador::form::variables(texto) {
            let Some((prefix, nome)) = variable.split_once('.') else {
                self.error_at(
                    Code::InvalidVariable,
                    format!(
                        "A {what} usa `{{{variable}}}`, que não existe. As variáveis são `{{etapa.campo}}`, `{{ficha.id}}` e `{{cliente.peer}}`."
                    ),
                    path.clone(),
                );
                continue;
            };
            if variable == "ficha.id" || variable == "cliente.peer" {
                continue;
            }
            if prefix == "ficha" || prefix == "cliente" {
                self.error_at(
                    Code::InvalidVariable,
                    format!(
                        "A {what} usa `{{{variable}}}`, que não existe. As variáveis são `{{etapa.campo}}`, `{{ficha.id}}` e `{{cliente.peer}}`."
                    ),
                    path.clone(),
                );
                continue;
            }
            let Some(index) = self.stages.get(prefix).copied() else {
                self.warning_at(
                    Code::VariableWithoutField,
                    format!(
                        "A {what} usa `{{{variable}}}`, mas a etapa `{prefix}` não existe: no atendimento sai vazio."
                    ),
                    path.clone(),
                );
                continue;
            };
            if !ir.stages[index].campos.iter().any(|c| c.id == nome) {
                self.warning_at(
                    Code::VariableWithoutField,
                    format!(
                        "A {what} usa `{{{variable}}}`, mas a etapa `{prefix}` não tem o campo `{nome}`: no atendimento sai vazio."
                    ),
                    path.clone(),
                );
                continue;
            }
            if let Some(current) = to_enter_of_stage
                && current.id == prefix
            {
                self.warning_at(
                    Code::VariableFromOwnStep,
                    format!(
                        "`ao_entrar` da etapa `{}` usa `{{{variable}}}`, que só é preenchido dentro da própria etapa: a mensagem sai com o texto vazio.",
                        current.id
                    ),
                    path.clone(),
                );
            }
        }
    }

    fn check_decisions(&mut self) {
        let ir = self.ir;
        for (i, decisao) in ir.decisions.iter().enumerate() {
            let base = decision_path(i, decisao);
            if self.decisions.get(decisao.id.as_str()) != Some(&i) {
                self.error_at(
                    Code::DuplicateId,
                    format!(
                        "O id `{}` está declarado duas vezes entre as decisões do fluxo.",
                        decisao.id
                    ),
                    base.clone(),
                );
            } else if self.stages.contains_key(decisao.id.as_str()) {
                self.error_at(
                    Code::DuplicateId,
                    format!(
                        "O id `{}` é de uma decisão e de uma etapa ao mesmo tempo: um destino não saberia para onde ir. Use ids diferentes.",
                        decisao.id
                    ),
                    base.clone(),
                );
            }
            let context = format!("A decisão `{}`", decisao.id);
            self.check_destination(&decisao.then_branch, &sub(&base, "entao"), &context);
            self.check_destination(&decisao.else_branch, &sub(&base, "senao"), &context);
            self.check_rule(&decisao.se, &sub(&base, "se"), &context);
        }
    }

    fn check_rule(&mut self, rule: &'a Rule, path: &str, context: &str) {
        match rule {
            Rule::Igual { campo, value } => {
                self.check_leaf(campo, Some(value), "igual", path, context)
            }
            Rule::Diferente { campo, value } => {
                self.check_leaf(campo, Some(value), "diferente", path, context)
            }
            Rule::Maior { campo, value } => {
                self.check_leaf(campo, Some(value), "maior", path, context)
            }
            Rule::Menor { campo, value } => {
                self.check_leaf(campo, Some(value), "menor", path, context)
            }
            Rule::Contem { campo, value } => {
                self.check_leaf(campo, Some(value), "contem", path, context)
            }
            Rule::Vazio { campo } => self.check_leaf(campo, None, "vazio", path, context),
            Rule::Funcao { nome } => {
                self.used.insert(nome.as_str());
                if !self.defs.contains_key(nome.as_str()) {
                    self.error_at(
                        Code::InvalidRuleOrActor,
                        format!("{context} usa `funcao(\"{nome}\")`, mas não existe `def {nome}(dados)` neste fluxo."),
                        path,
                    );
                }
            }
            Rule::E { args } | Rule::Ou { args } | Rule::Nao { args } => {
                self.check_group(rule, args, path, context);
            }
        }
    }

    fn check_group(&mut self, rule: &'a Rule, args: &'a [Rule], path: &str, context: &str) {
        if matches!(rule, Rule::Nao { .. }) && args.len() != 1 {
            self.error_at(
                Code::MalformedCombinator,
                format!(
                    "`nao` recebe exatamente uma condição; {context} usa {}.",
                    args.len()
                ),
                path,
            );
        }
        if matches!(rule, Rule::E { .. } | Rule::Ou { .. }) && args.len() < 2 {
            self.error_at(
                Code::MalformedCombinator,
                format!(
                    "`{}` precisa de pelo menos duas condições; {context} usa {}.",
                    if matches!(rule, Rule::E { .. }) {
                        "e"
                    } else {
                        "ou"
                    },
                    args.len()
                ),
                path,
            );
        }

        let leaves: Vec<(&str, Option<&str>, &str)> = args.iter().filter_map(leaf_key).collect();
        if matches!(rule, Rule::E { .. }) {
            for (i, a) in leaves.iter().enumerate() {
                for b in leaves.iter().skip(i + 1) {
                    let opostas =
                        matches!((a.2, b.2), ("igual", "diferente") | ("diferente", "igual"));
                    if a.0 == b.0 && a.1 == b.1 && opostas {
                        self.warning_at(
                            Code::ContradictoryCondition,
                            format!(
                                "{context} não pode ser verdadeira com `{}` igual e diferente de `{}` ao mesmo tempo.",
                                a.0,
                                a.1.unwrap_or("")
                            ),
                            path,
                        );
                        break;
                    }
                }
            }
        }
        if matches!(rule, Rule::Ou { .. }) {
            for (i, a) in args.iter().enumerate() {
                if args.iter().skip(i + 1).any(|b| b == a) {
                    self.warning_at(
                        Code::ContradictoryCondition,
                        format!(
                            "{context} repete o mesmo teste duas vezes na mesma disjunção (`ou`)."
                        ),
                        path,
                    );
                    break;
                }
            }
        }

        for (k, child) in args.iter().enumerate() {
            self.check_rule(child, &sub(path, &format!("args[{k}]")), context);
        }
    }

    fn check_leaf(
        &mut self,
        campo: &str,
        value: Option<&str>,
        tipo: &str,
        path: &str,
        context: &str,
    ) {
        let Some((stage_id, field_id)) = campo.split_once('.') else {
            self.error_at(
                Code::MalformedCombinator,
                format!(
                    "{context} compara `{campo}`; use `etapa.campo`, por exemplo `coleta.endereco`."
                ),
                path,
            );
            return;
        };
        if stage_id.is_empty() || field_id.is_empty() || field_id.contains('.') {
            self.error_at(
                Code::MalformedCombinator,
                format!(
                    "{context} compara `{campo}`; use `etapa.campo`, com ids em minúsculas (por exemplo `coleta.endereco`)."
                ),
                path,
            );
            return;
        }
        let ir = self.ir;
        let Some(index) = self.stages.get(stage_id).copied() else {
            self.error_at(
                Code::UnknownField,
                format!("{context} compara `{campo}`, mas a etapa `{stage_id}` não existe."),
                path,
            );
            return;
        };
        let Some(field_info) = ir.stages[index].campos.iter().find(|c| c.id == field_id) else {
            self.error_at(
                Code::UnknownField,
                format!(
                    "{context} compara `{campo}`, mas a etapa `{stage_id}` não tem o campo `{field_id}`."
                ),
                path,
            );
            return;
        };
        let (field_kind, options) = (field_info.tipo, field_info.options.clone());
        self.read
            .entry(ir.stages[index].id.as_str())
            .or_default()
            .insert(field_info.id.as_str());

        match tipo {
            "igual" | "diferente" => {
                if let Some(value) = value
                    && !options.is_empty()
                    && !options.iter().any(|o| o == value)
                {
                    self.warning_at(
                        Code::OutsideOptions,
                        format!(
                            "{context} compara `{campo}` com `{value}`, que não é uma das opções ({}): o teste nunca casa.",
                            options.join(", ")
                        ),
                        path,
                    );
                }
            }
            "maior" | "menor" if field_kind != FieldKind::Number => {
                self.warning_at(
                    Code::TextComparison,
                    format!(
                        "{context} usa `{tipo}` com `{campo}`, mas o campo é `{}`: mude o tipo para `numero` ou use `igual`.",
                        field_kind.as_text()
                    ),
                    path,
                );
            }
            _ => {}
        }
    }

    fn check_destination(&mut self, destination: &Destination, path: &str, context: &str) {
        match destination {
            Destination::Etapa { id } => {
                if !self.stages.contains_key(id.as_str()) {
                    self.error_at(
                        Code::UnknownDestination,
                        format!(
                            "{context} vai para a etapa `{id}`, que não existe. Use o id de uma etapa ou de uma decisão."
                        ),
                        path,
                    );
                }
            }
            Destination::Decisao { id } => {
                if !self.decisions.contains_key(id.as_str()) {
                    self.error_at(
                        Code::UnknownDestination,
                        format!(
                            "{context} vai para a decisão `{id}`, que não existe. Use o id de uma etapa ou de uma decisão."
                        ),
                        path,
                    );
                }
            }
            Destination::Fim {
                id,
                resultado,
                ends_service,
            } => match self.ends.get(id.as_str()).copied() {
                None => self.error_at(
                    Code::UnknownDestination,
                    format!(
                        "{context} termina no fim `{id}`, que não está declarado em `fins` deste fluxo."
                    ),
                    path,
                ),
                Some(index) => {
                    let fim = &self.ir.ends[index];
                    if fim.resultado != *resultado
                        || fim.ends_service != *ends_service
                    {
                        self.error_at(
                            Code::DivergentEnd,
                            format!(
                                "O fim `{id}` diz uma coisa aqui e outra em `fins[{index}]` ({}): use a mesma declaração de `fim(...)` nos dois lugares.",
                                if fim.resultado != *resultado {
                                    "o resultado é diferente"
                                } else {
                                    "o `encerra_atendimento` é diferente"
                                }
                            ),
                            path.to_owned(),
                        );
                    }
                }
            },
        }
    }

    fn check_ends(&mut self) {
        let ir = self.ir;
        let mut seen: BTreeSet<&str> = BTreeSet::new();
        for (i, fim) in ir.ends.iter().enumerate() {
            if !seen.insert(fim.id.as_str()) {
                self.error_at(
                    Code::DuplicateId,
                    format!(
                        "O fim `{}` está declarado duas vezes em `fins`; use a mesma declaração nos dois lugares.",
                        fim.id
                    ),
                    format!("fins[{i}] (id \"{}\")", fim.id),
                );
            }
        }
    }

    fn check_start(&mut self) {
        if self.ir.start.is_empty() {
            self.error_at(
                Code::UnknownDestination,
                "O fluxo não declara a etapa de início. Indique o id da etapa em que o atendimento começa.",
                "inicio",
            );
        } else if !self.stages.contains_key(self.ir.start.as_str()) {
            self.error_at(
                Code::UnknownDestination,
                format!(
                    "O fluxo começa em `{}`, que não é uma etapa deste fluxo. Use o id de uma etapa.",
                    self.ir.start
                ),
                "inicio",
            );
        }
    }

    fn check_cycle(&mut self) {
        let ir = self.ir;
        let mut graph: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for decisao in &ir.decisions {
            let mut arestas: Vec<&str> = Vec::new();
            for destination in [&decisao.then_branch, &decisao.else_branch] {
                if let Destination::Decisao { id } = destination {
                    arestas.push(id.as_str());
                }
            }
            graph.insert(decisao.id.as_str(), arestas);
        }
        let Some(cycle) = find_cycle(&graph) else {
            return;
        };
        let first = cycle.first().map(String::as_str).unwrap_or("");
        let index = self.decisions.get(first).copied().unwrap_or(0);
        let path = ir
            .decisions
            .get(index)
            .map(|decisao| decision_path(index, decisao))
            .unwrap_or_else(|| "decisoes".to_owned());
        self.error_at(
            Code::DecisionCycle,
            format!(
                "As decisões formam um ciclo ({}): um fluxo não pode voltar a uma decisão já avaliada.",
                cycle.join(" → ")
            ),
            path,
        );
    }

    fn check_reachability(&mut self) {
        let ir = self.ir;
        let start = ir.start.as_str();
        if start.is_empty() || !self.stages.contains_key(start) {
            return;
        }
        let mut graph: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for etapa in &ir.stages {
            let arestas = graph.entry(etapa.id.as_str()).or_default();
            for acao in &etapa.actions {
                if let Some(id) = acao.goes_to.is_open() {
                    arestas.push(id);
                }
            }
        }
        for decisao in &ir.decisions {
            let arestas = graph.entry(decisao.id.as_str()).or_default();
            for destination in [&decisao.then_branch, &decisao.else_branch] {
                if let Some(id) = destination.is_open() {
                    arestas.push(id);
                }
            }
        }

        let mut reached: BTreeSet<&str> = BTreeSet::new();
        let mut pilha = vec![start];
        while let Some(no) = pilha.pop() {
            if !reached.insert(no) {
                continue;
            }
            if let Some(destinations) = graph.get(no) {
                for destination in destinations {
                    pilha.push(destination);
                }
            }
        }

        for (i, etapa) in ir.stages.iter().enumerate() {
            if !reached.contains(etapa.id.as_str()) {
                self.warning_at(
                    Code::Unreachable,
                    format!(
                        "A etapa `{}` não é alcançável a partir de `inicio = \"{start}\"`. Confira os `vai_para`.",
                        etapa.id
                    ),
                    stage_path(i, etapa),
                );
            }
        }
        for (i, decisao) in ir.decisions.iter().enumerate() {
            if !reached.contains(decisao.id.as_str()) {
                self.warning_at(
                    Code::Unreachable,
                    format!(
                        "A decisão `{}` não é alcançável a partir de `inicio = \"{start}\"`. Confira os `vai_para` e os ramos `entao`/`senao`.",
                        decisao.id
                    ),
                    decision_path(i, decisao),
                );
            }
        }
    }

    fn check_limit(&mut self) {
        let nodes = self.ir.stages.len() + self.ir.decisions.len() + self.ir.ends.len();
        if nodes > MAX_NOS {
            self.error_at(
                Code::NodeLimit,
                format!(
                    "O fluxo tem {nodes} nós e o limite é {MAX_NOS}. Junte etapas parecidas ou divida o fluxo."
                ),
                format!("fluxo (id \"{}\")", self.ir.fluxo.id),
            );
        }
    }

    fn check_field_usage(&mut self) {
        let ir = self.ir;
        for (i, etapa) in ir.stages.iter().enumerate() {
            let base = stage_path(i, etapa);
            for (j, campo) in etapa.campos.iter().enumerate() {
                let required = self
                    .required
                    .get(etapa.id.as_str())
                    .is_some_and(|required| required.contains(campo.id.as_str()));
                let read = self
                    .read
                    .get(etapa.id.as_str())
                    .is_some_and(|read| read.contains(campo.id.as_str()));
                if required || read {
                    continue;
                }
                self.warning_at(
                    Code::UnusedField,
                    format!(
                        "O campo `{}.{}` não é exigido por nenhuma ação nem lido por nenhuma regra: ninguém usa o valor.",
                        etapa.id, campo.id
                    ),
                    sub(&base, &format!("campos[{j}]")),
                );
            }
        }
    }

    fn check_def_usage(&mut self) {
        let ir = self.ir;
        let defs: Vec<(&str, usize)> = ir
            .defs
            .iter()
            .enumerate()
            .map(|(i, def)| (def.nome.as_str(), i))
            .collect();
        for (nome, index) in defs {
            if self.used.contains(nome) {
                continue;
            }
            self.warning_at(
                Code::UnusedDef,
                format!(
                    "A função `{nome}` está definida e nenhuma decisão usa `funcao(\"{nome}\")`. Chame ou apague."
                ),
                format!("defs[{index}] (nome \"{nome}\")"),
            );
        }
    }
}

fn leaf_key(rule: &Rule) -> Option<(&str, Option<&str>, &'static str)> {
    match rule {
        Rule::Igual { campo, value } => Some((campo, Some(value), "igual")),
        Rule::Diferente { campo, value } => Some((campo, Some(value), "diferente")),
        Rule::Maior { campo, value } => Some((campo, Some(value), "maior")),
        Rule::Menor { campo, value } => Some((campo, Some(value), "menor")),
        Rule::Contem { campo, value } => Some((campo, Some(value), "contem")),
        Rule::Vazio { campo } => Some((campo, None, "vazio")),
        Rule::E { .. } | Rule::Ou { .. } | Rule::Nao { .. } | Rule::Funcao { .. } => None,
    }
}

fn duplicates(values: &[String]) -> bool {
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    values.iter().any(|value| !seen.insert(value.as_str()))
}

fn find_cycle(graph: &BTreeMap<&str, Vec<&str>>) -> Option<Vec<String>> {
    fn visit(
        no: &str,
        graph: &BTreeMap<&str, Vec<&str>>,
        estado: &mut BTreeMap<String, u8>,
        path: &mut Vec<String>,
        found: &mut Option<Vec<String>>,
    ) {
        if found.is_some() || !graph.contains_key(no) {
            return;
        }
        match estado.get(no).copied().unwrap_or(0) {
            1 => {
                let mut cycle: Vec<String> = path
                    .iter()
                    .skip_while(|current| current.as_str() != no)
                    .cloned()
                    .collect();
                cycle.push(no.to_owned());
                *found = Some(cycle);
                return;
            }
            2 => return,
            _ => {}
        }
        estado.insert(no.to_owned(), 1);
        path.push(no.to_owned());
        if let Some(destinations) = graph.get(no) {
            for destination in destinations {
                visit(destination, graph, estado, path, found);
            }
        }
        path.pop();
        estado.insert(no.to_owned(), 2);
    }

    let mut estado: BTreeMap<String, u8> = BTreeMap::new();
    let mut path: Vec<String> = Vec::new();
    let mut found: Option<Vec<String>> = None;
    for root in graph.keys() {
        if found.is_some() {
            break;
        }
        visit(root, graph, &mut estado, &mut path, &mut found);
    }
    found
}
