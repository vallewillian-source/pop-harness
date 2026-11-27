use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value as Json, json};

use poph_types::{
    ActionActor, DomainError, DomainResult, EventActor, ExecutionStatus, FieldKind, RecordEventKind,
};

use crate::ir::{Campo, Decisao, Destination, Effect, Etapa, Ir};
use crate::rules::{Dados, RuleEvaluator, canonical_text, preenchido};

pub type Campos = Map<String, Json>;

pub const MAX_SALTOS_DECISAO: usize = 10;

#[derive(Debug, Clone, Copy)]
pub struct Context<'a> {
    pub ficha_id: &'a str,
    pub client_peer: &'a str,
    pub evaluator: &'a RuleEvaluator,
    pub max_saltos_decisao: usize,
}

impl<'a> Context<'a> {
    pub fn new(evaluator: &'a RuleEvaluator) -> Self {
        Self {
            ficha_id: "",
            client_peer: "",
            evaluator,
            max_saltos_decisao: MAX_SALTOS_DECISAO,
        }
    }

    pub fn with_case(mut self, ficha_id: &'a str) -> Self {
        self.ficha_id = ficha_id;
        self
    }

    pub fn with_client(mut self, client_peer: &'a str) -> Self {
        self.client_peer = client_peer;
        self
    }

    pub fn with_jump_limit(mut self, limit: usize) -> Self {
        self.max_saltos_decisao = limit;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Estado {
    pub fluxo_id: String,
    pub status: ExecutionStatus,
    pub no_atual: Destination,
    pub dados: Dados,
    #[serde(rename = "atores_que_agiram")]
    pub actors_who_acted: Vec<ActionActor>,
    pub resultado: Option<String>,
    pub motivo: Option<String>,
    pub keys: BTreeMap<String, Receipt>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Receipt {
    pub no_atual: Destination,
    #[serde(rename = "aguardando")]
    pub waiting: Vec<ActionActor>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Evento {
    pub tipo: RecordEventKind,
    #[serde(rename = "ator")]
    pub actor: EventActor,
    pub payload: Json,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resultado {
    pub new_state: Estado,
    pub events: Vec<Evento>,
    pub effects: Vec<ProducedEffect>,
    pub waiting: Vec<ActionActor>,
    pub repeated: bool,
    pub ends_service: bool,
    pub rule_error: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProducedEffect {
    pub chave: String,
    pub no: String,
    #[serde(rename = "indice")]
    pub index: usize,
    #[serde(rename = "efeito")]
    pub effect: Effect,
}

impl ProducedEffect {
    pub fn tipo(&self) -> &'static str {
        match self.effect {
            Effect::MessageToCustomer { .. } => "mensagem_ao_cliente",
            Effect::NotifyWhatsApp { .. } => "notificar_whatsapp",
            Effect::Encaminhar { .. } => "encaminhar",
        }
    }
}

pub fn start(ir: &Ir) -> DomainResult<Estado> {
    let etapa = step_by_id(ir, &ir.start)?;
    Ok(Estado {
        fluxo_id: ir.fluxo.id.clone(),
        status: ExecutionStatus::Active,
        no_atual: Destination::Etapa {
            id: etapa.id.clone(),
        },
        dados: Dados::new(),
        actors_who_acted: Vec::new(),
        resultado: None,
        motivo: None,
        keys: BTreeMap::new(),
    })
}

pub fn execute_action(
    ir: &Ir,
    estado: &Estado,
    acao: &str,
    campos: &Campos,
    actor: ActionActor,
    chave_idempotencia: Option<&str>,
    context: &Context<'_>,
) -> DomainResult<Resultado> {
    if estado.status != ExecutionStatus::Active {
        return Err(rejection(
            acao,
            format!(
                "a execução está {} e não aceita ação; comece outro fluxo ou leia o estado",
                estado.status
            ),
        ));
    }

    if let Some(chave) = chave_idempotencia
        && let Some(receipt) = estado.keys.get(chave)
    {
        return Ok(Resultado {
            new_state: estado.clone(),
            events: Vec::new(),
            effects: Vec::new(),
            waiting: receipt.waiting.clone(),
            repeated: true,
            ends_service: false,
            rule_error: false,
        });
    }

    let mut work = estado.clone();
    let mut events: Vec<Evento> = Vec::new();
    let mut effects: Vec<ProducedEffect> = Vec::new();

    if let Destination::Decisao { .. } = work.no_atual {
        let halted = work.no_atual.clone();
        let outcome = follow(ir, &mut work, halted, context, &mut events, &mut effects)?;
        if work.status != ExecutionStatus::Active {
            return Ok(halted_result(work, events, effects, outcome.rule_error));
        }
    }

    let step_id = match &work.no_atual {
        Destination::Etapa { id } => id.clone(),
        Destination::Decisao { id } => {
            return Err(rejection(
                acao,
                format!(
                    "a execução está na decisão `{id}`: ela precisa ser resolvida antes da ação"
                ),
            ));
        }
        Destination::Fim { id, .. } => {
            return Err(rejection(
                acao,
                format!("a execução chegou ao fim `{id}`: não há ação depois dele"),
            ));
        }
    };
    let etapa = step_by_id(ir, &step_id)?;

    let Some(action_def) = etapa.actions.iter().find(|candidate| candidate.id == acao) else {
        return Err(rejection(
            acao,
            format!(
                "a etapa `{}` não tem essa ação; válidas: {}",
                etapa.id,
                action_list(etapa)
            ),
        ));
    };

    if !etapa.action_actors.contains(&actor) {
        return Err(rejection(
            acao,
            format!(
                "o ator `{actor}` não age na etapa `{}`; atores: {}",
                etapa.id,
                actor_list(&etapa.action_actors)
            ),
        ));
    }

    if let Some(owner) = action_def.actor
        && owner != actor
    {
        return Err(rejection(
            acao,
            format!("a ação `{acao}` é restrita ao ator `{owner}`"),
        ));
    }

    let validated = match validate_fields(etapa, campos) {
        Ok(validated) => validated,
        Err(errors) => return Err(invalid_fields(etapa, errors)),
    };

    let mut recorded = step_data(&work.dados, &etapa.id);
    recorded.extend(validated.clone());
    let mut missing: Vec<String> = action_def
        .requires
        .iter()
        .filter(|campo| !preenchido(recorded.get(*campo)))
        .map(|campo| {
            if etapa
                .campos
                .iter()
                .any(|declaration| declaration.id == *campo)
            {
                format!("a ação `{acao}` exige o campo `{campo}`, que ainda não foi preenchido")
            } else {
                format!(
                    "a ação `{acao}` exige o campo `{campo}`, que a etapa `{}` não tem",
                    etapa.id
                )
            }
        })
        .collect();
    if !missing.is_empty() {
        missing.sort();
        return Err(invalid_fields(etapa, missing));
    }

    let mut progress = work.actors_who_acted.clone();
    if !progress.contains(&actor) {
        progress.push(actor);
    }
    sort_actors(&mut progress);

    let mut waiting: Vec<ActionActor> = required_actors(etapa)
        .into_iter()
        .filter(|required_actor| !progress.contains(required_actor))
        .collect();
    sort_actors(&mut waiting);

    let decisive = matches!(action_def.goes_to, Destination::Fim { .. });
    let advances = decisive || waiting.is_empty();

    let author = event_actor(actor);
    if !validated.is_empty() {
        events.push(Evento {
            tipo: RecordEventKind::FieldsFilled,
            actor: author,
            payload: json!({ "etapa": etapa.id, "campos": validated }),
        });
    }
    events.push(Evento {
        tipo: RecordEventKind::ActionExecuted,
        actor: author,
        payload: json!({
            "etapa": etapa.id,
            "acao": acao,
            "destino": action_def.goes_to,
            "parcial": !advances,
            "aguardando": waiting,
        }),
    });

    let mut new_state = work;
    new_state
        .dados
        .insert(etapa.id.clone(), Json::Object(recorded));

    let mut ends_service = false;
    let mut rule_error = false;
    let result_waiting = if advances {
        let action_node = format!("acao:{}.{acao}", etapa.id);
        for effect in declared_effects(&action_def.effects, action_node, &new_state.dados, context)
        {
            events.push(effect_event(&effect));
            effects.push(effect);
        }
        events.push(Evento {
            tipo: RecordEventKind::StepCompleted,
            actor: EventActor::System,
            payload: json!({ "etapa": etapa.id, "acao": acao }),
        });
        new_state.actors_who_acted = Vec::new();
        let outcome = follow(
            ir,
            &mut new_state,
            action_def.goes_to.clone(),
            context,
            &mut events,
            &mut effects,
        )?;
        ends_service = outcome.ends_service;
        rule_error = outcome.rule_error;
        Vec::new()
    } else {
        new_state.actors_who_acted = progress;
        waiting
    };

    if let Some(chave) = chave_idempotencia {
        new_state.keys.insert(
            chave.to_owned(),
            Receipt {
                no_atual: new_state.no_atual.clone(),
                waiting: result_waiting.clone(),
            },
        );
    }

    Ok(Resultado {
        new_state,
        events,
        effects,
        waiting: result_waiting,
        repeated: false,
        ends_service,
        rule_error,
    })
}

pub fn abandon(
    estado: &Estado,
    motivo: &str,
    actor: ActionActor,
    chave_idempotencia: Option<&str>,
) -> DomainResult<Resultado> {
    let motivo = motivo.trim();
    if motivo.is_empty() {
        return Err(DomainError::Validation {
            field: "motivo",
            reason: "o abandono precisa de um motivo: é ele que fica no histórico da Ficha"
                .to_owned(),
        });
    }
    if estado.status != ExecutionStatus::Active {
        return Err(DomainError::Conflict {
            entity: "execução",
            id: estado.no_atual.id().to_owned(),
            reason: format!(
                "a execução já está {} e não pode ser abandonada",
                estado.status
            ),
        });
    }
    if let Some(chave) = chave_idempotencia
        && let Some(receipt) = estado.keys.get(chave)
    {
        return Ok(Resultado {
            new_state: estado.clone(),
            events: Vec::new(),
            effects: Vec::new(),
            waiting: receipt.waiting.clone(),
            repeated: true,
            ends_service: false,
            rule_error: false,
        });
    }

    let no = estado.no_atual.id().to_owned();
    let mut new_state = estado.clone();
    new_state.status = ExecutionStatus::Abandoned;
    new_state.motivo = Some(motivo.to_owned());
    new_state.actors_who_acted = Vec::new();

    let events = vec![Evento {
        tipo: RecordEventKind::ExecutionAbandoned,
        actor: event_actor(actor),
        payload: json!({ "motivo": motivo, "no": no }),
    }];

    if let Some(chave) = chave_idempotencia {
        new_state.keys.insert(
            chave.to_owned(),
            Receipt {
                no_atual: new_state.no_atual.clone(),
                waiting: Vec::new(),
            },
        );
    }

    Ok(Resultado {
        new_state,
        events,
        effects: Vec::new(),
        waiting: Vec::new(),
        repeated: false,
        ends_service: false,
        rule_error: false,
    })
}

pub fn entry_effects(
    ir: &Ir,
    step_id: &str,
    dados: &Dados,
    context: &Context<'_>,
) -> DomainResult<Vec<ProducedEffect>> {
    let etapa = step_by_id(ir, step_id)?;
    Ok(step_effects(etapa, dados, context))
}

#[derive(Debug, Clone, Default)]
struct Outcome {
    ends_service: bool,
    rule_error: bool,
}

fn follow(
    ir: &Ir,
    estado: &mut Estado,
    mut destination: Destination,
    context: &Context<'_>,
    events: &mut Vec<Evento>,
    effects: &mut Vec<ProducedEffect>,
) -> DomainResult<Outcome> {
    let mut jumps = 0usize;
    let mut visited: Vec<String> = Vec::new();

    loop {
        match destination.clone() {
            Destination::Etapa { id } => {
                let etapa = step_by_id(ir, &id)?;
                estado.no_atual = Destination::Etapa { id: id.clone() };
                estado.actors_who_acted = Vec::new();
                events.push(Evento {
                    tipo: RecordEventKind::StepEntered,
                    actor: EventActor::System,
                    payload: json!({ "etapa": id }),
                });
                let entered_effects = step_effects(etapa, &estado.dados, context);
                for effect in entered_effects {
                    events.push(effect_event(&effect));
                    effects.push(effect);
                }
                return Ok(Outcome::default());
            }
            Destination::Decisao { id } => {
                let decisao = decision_by_id(ir, &id)?;
                if visited.contains(&id) {
                    let path = format!("{} → {id}", visited.join(" → "));
                    let motivo = format!(
                        "ciclo de decisões: {path}; a execução parou para não repetir o caminho \
                         (a publicação recusa este ciclo com FLX23)"
                    );
                    return Ok(stop_with_failure(
                        estado,
                        Destination::Decisao { id },
                        motivo,
                        events,
                        false,
                    ));
                }
                visited.push(id.clone());
                jumps += 1;
                if jumps > context.max_saltos_decisao {
                    let motivo = format!(
                        "a execução passou de {} decisões encadeadas (fluxos.max_saltos_decisao); \
                         a estrutura do fluxo entrou em laço",
                        context.max_saltos_decisao
                    );
                    return Ok(stop_with_failure(
                        estado,
                        Destination::Decisao { id },
                        motivo,
                        events,
                        false,
                    ));
                }
                let choice = match context.evaluator.evaluate(&decisao.se, &estado.dados) {
                    Ok(choice) => choice,
                    Err(error) => {
                        return Ok(stop_with_failure(
                            estado,
                            Destination::Decisao { id },
                            error.to_string(),
                            events,
                            true,
                        ));
                    }
                };
                let branch = if choice {
                    decisao.then_branch.clone()
                } else {
                    decisao.else_branch.clone()
                };
                events.push(Evento {
                    tipo: RecordEventKind::DecisionResolved,
                    actor: EventActor::System,
                    payload: json!({ "decisao": id, "resultado": choice, "destino": branch }),
                });
                destination = branch;
            }
            Destination::Fim {
                id,
                resultado,
                ends_service,
            } => {
                estado.status = ExecutionStatus::Completed;
                estado.resultado = resultado.clone();
                estado.no_atual = Destination::Fim {
                    id: id.clone(),
                    resultado: resultado.clone(),
                    ends_service,
                };
                events.push(Evento {
                    tipo: RecordEventKind::ExecutionCompleted,
                    actor: EventActor::System,
                    payload: json!({
                        "fim": id,
                        "resultado": resultado,
                        "encerra_atendimento": ends_service,
                    }),
                });
                return Ok(Outcome {
                    ends_service,
                    rule_error: false,
                });
            }
        }
    }
}

fn stop_with_failure(
    estado: &mut Estado,
    no: Destination,
    motivo: String,
    events: &mut Vec<Evento>,
    rule_error: bool,
) -> Outcome {
    estado.status = ExecutionStatus::Failed;
    estado.motivo = Some(motivo.clone());
    estado.actors_who_acted = Vec::new();
    estado.no_atual = no;
    events.push(Evento {
        tipo: RecordEventKind::ExecutionFailed,
        actor: EventActor::System,
        payload: json!({ "no": estado.no_atual.id(), "motivo": motivo }),
    });
    Outcome {
        ends_service: false,
        rule_error,
    }
}

fn halted_result(
    estado: Estado,
    events: Vec<Evento>,
    effects: Vec<ProducedEffect>,
    rule_error: bool,
) -> Resultado {
    Resultado {
        new_state: estado,
        events,
        effects,
        waiting: Vec::new(),
        repeated: false,
        ends_service: false,
        rule_error,
    }
}

fn step_effects(etapa: &Etapa, dados: &Dados, context: &Context<'_>) -> Vec<ProducedEffect> {
    declared_effects(
        &etapa.on_enter,
        format!("etapa:{}", etapa.id),
        dados,
        context,
    )
}

fn declared_effects(
    declarations: &[Effect],
    no: String,
    dados: &Dados,
    context: &Context<'_>,
) -> Vec<ProducedEffect> {
    declarations
        .iter()
        .enumerate()
        .map(|(index, declaration)| {
            let effect = match declaration {
                Effect::MessageToCustomer { texto } => Effect::MessageToCustomer {
                    texto: interpolate(texto, dados, context),
                },
                Effect::NotifyWhatsApp { para, mensagem } => Effect::NotifyWhatsApp {
                    para: para.clone(),
                    mensagem: interpolate(mensagem, dados, context),
                },
                Effect::Encaminhar { departamento } => Effect::Encaminhar {
                    departamento: departamento.clone(),
                },
            };
            ProducedEffect {
                chave: format!("{no}/{index}"),
                no: no.clone(),
                index,
                effect,
            }
        })
        .collect()
}

fn effect_event(effect: &ProducedEffect) -> Evento {
    Evento {
        tipo: RecordEventKind::EffectCreated,
        actor: EventActor::System,
        payload: json!({ "chave": effect.chave, "tipo": effect.tipo(), "no": effect.no }),
    }
}

fn interpolate(texto: &str, dados: &Dados, context: &Context<'_>) -> String {
    let mut saida = String::with_capacity(texto.len());
    let mut rest = texto;
    while let Some(opening) = rest.find('{') {
        saida.push_str(&rest[..opening]);
        let after = &rest[opening + 1..];
        match after.find('}') {
            Some(fim) => {
                saida.push_str(&variable_value(&after[..fim], dados, context));
                rest = &after[fim + 1..];
            }
            None => {
                saida.push_str(&rest[opening..]);
                rest = "";
            }
        }
    }
    saida.push_str(rest);
    saida
}

fn variable_value(nome: &str, dados: &Dados, context: &Context<'_>) -> String {
    match nome.trim() {
        "ficha.id" => context.ficha_id.to_owned(),
        "cliente.peer" => context.client_peer.to_owned(),
        nome => match nome.split_once('.') {
            Some((etapa, campo)) => dados
                .get(etapa)
                .and_then(|values| values.get(campo))
                .and_then(canonical_text)
                .unwrap_or_default(),
            None => String::new(),
        },
    }
}

fn decision_by_id<'a>(ir: &'a Ir, id: &str) -> DomainResult<&'a Decisao> {
    ir.decisions
        .iter()
        .find(|decisao| decisao.id == id)
        .ok_or_else(|| DomainError::IrDivergent {
            fluxo: ir.fluxo.id.clone(),
            reason: format!("a decisão `{id}` não existe na estrutura do fluxo"),
        })
}

pub fn required_actors(etapa: &Etapa) -> Vec<ActionActor> {
    let mut required: Vec<ActionActor> = Vec::new();
    for acao in &etapa.actions {
        if matches!(acao.goes_to, Destination::Fim { .. }) {
            continue;
        }
        let Some(actor) = acao.actor else {
            continue;
        };
        if !required.contains(&actor) {
            required.push(actor);
        }
    }
    sort_actors(&mut required);
    required
}

pub fn missing_fields(etapa: &Etapa, dados: &Dados) -> Vec<String> {
    let recorded = step_data(dados, &etapa.id);
    etapa
        .campos
        .iter()
        .filter(|campo| campo.required && !preenchido(recorded.get(&campo.id)))
        .map(|campo| campo.id.clone())
        .collect()
}

pub fn step_data(dados: &Dados, etapa: &str) -> Campos {
    match dados.get(etapa) {
        Some(Json::Object(map)) => map.clone(),
        _ => Campos::new(),
    }
}

fn step_by_id<'a>(ir: &'a Ir, id: &str) -> DomainResult<&'a Etapa> {
    ir.stages
        .iter()
        .find(|etapa| etapa.id == id)
        .ok_or_else(|| DomainError::IrDivergent {
            fluxo: ir.fluxo.id.clone(),
            reason: format!("a etapa `{id}` não existe na estrutura do fluxo"),
        })
}

fn rejection(acao: &str, motivo: String) -> DomainError {
    DomainError::InvalidAction {
        action: acao.to_owned(),
        reason: motivo,
    }
}

fn invalid_fields(etapa: &Etapa, errors: Vec<String>) -> DomainError {
    DomainError::InvalidStepFields {
        step: etapa.id.clone(),
        errors,
    }
}

fn sort_actors(actors: &mut [ActionActor]) {
    actors.sort_by_key(|actor| actor.as_text());
}

fn event_actor(actor: ActionActor) -> EventActor {
    match actor {
        ActionActor::Agent => EventActor::Agent,
        ActionActor::Attendant => EventActor::Attendant,
    }
}

fn validate_fields(etapa: &Etapa, campos: &Campos) -> Result<Campos, Vec<String>> {
    let mut validated = Campos::new();
    let mut errors: Vec<String> = Vec::new();
    for (chave, value) in campos {
        let Some(campo) = etapa.campos.iter().find(|candidate| candidate.id == *chave) else {
            errors.push(field_outside_step(etapa, chave));
            continue;
        };
        match validate_value(campo, value) {
            Ok(normalized) => {
                validated.insert(chave.clone(), normalized);
            }
            Err(motivo) => errors.push(format!("{chave}: {motivo}")),
        }
    }
    if errors.is_empty() {
        Ok(validated)
    } else {
        errors.sort();
        Err(errors)
    }
}

fn validate_value(campo: &Campo, value: &Json) -> Result<Json, String> {
    if value.is_null() {
        return if campo.required {
            Err("é obrigatório e não aceita valor vazio".to_owned())
        } else {
            Ok(Json::Null)
        };
    }
    match campo.tipo {
        FieldKind::Text => match value {
            Json::String(texto) => {
                let clean = texto.trim();
                if campo.required && clean.is_empty() {
                    return Err("é obrigatório e veio vazio".to_owned());
                }
                Ok(Json::String(clean.to_owned()))
            }
            other => Err(expected_type("texto", other)),
        },
        FieldKind::Number => match value {
            Json::Number(_) => Ok(value.clone()),
            other => Err(expected_type("número", other)),
        },
        FieldKind::Data => match value {
            Json::String(texto) if valid_iso_date(texto.trim()) => {
                Ok(Json::String(texto.trim().to_owned()))
            }
            Json::String(texto) => Err(format!(
                "esperava data ISO 8601 (`AAAA-MM-DD` ou `AAAA-MM-DDTHH:MM:SS-03:00`) e veio `{texto}`"
            )),
            other => Err(expected_type("data em texto ISO 8601", other)),
        },
        FieldKind::Boolean => match value {
            Json::Bool(_) => Ok(value.clone()),
            other => Err(expected_type("booleano (`true`/`false`)", other)),
        },
        FieldKind::Option => match value {
            Json::String(texto) => {
                let clean = texto.trim();
                if campo.options.iter().any(|option| option == clean) {
                    Ok(Json::String(clean.to_owned()))
                } else {
                    Err(format!("esperava uma das opções: {}", option_list(campo)))
                }
            }
            other => Err(expected_type("uma das opções em texto", other)),
        },
        FieldKind::Attachment => match value {
            Json::Object(map) if matches!(map.get("nome"), Some(Json::String(nome)) if !nome.trim().is_empty()) => {
                Ok(value.clone())
            }
            Json::Object(_) => Err("esperava um anexo com o nome do arquivo em `nome`".to_owned()),
            other => Err(expected_type("anexo (`{nome, tipo, tamanho}`)", other)),
        },
    }
}

fn expected_type(expected: &str, value: &Json) -> String {
    format!("esperava {expected} e veio {}", json_kind(value))
}

fn json_kind(value: &Json) -> &'static str {
    match value {
        Json::Null => "nulo",
        Json::Bool(_) => "booleano",
        Json::Number(_) => "número",
        Json::String(_) => "texto",
        Json::Array(_) => "lista",
        Json::Object(_) => "objeto",
    }
}

fn field_outside_step(etapa: &Etapa, chave: &str) -> String {
    let mut mensagem = format!(
        "`{chave}` não é campo da etapa `{}`; os campos desta etapa são: {}",
        etapa.id,
        field_list(etapa)
    );
    if chave.contains('.') {
        mensagem.push_str(" (informe só o nome do campo, sem o prefixo da etapa)");
    }
    mensagem
}

fn action_list(etapa: &Etapa) -> String {
    if etapa.actions.is_empty() {
        return "nenhuma (a etapa não tem saída)".to_owned();
    }
    etapa
        .actions
        .iter()
        .map(|acao| format!("`{}`", acao.id))
        .collect::<Vec<_>>()
        .join(", ")
}

fn field_list(etapa: &Etapa) -> String {
    if etapa.campos.is_empty() {
        return "nenhum".to_owned();
    }
    etapa
        .campos
        .iter()
        .map(|campo| format!("`{}`", campo.id))
        .collect::<Vec<_>>()
        .join(", ")
}

fn option_list(campo: &Campo) -> String {
    campo
        .options
        .iter()
        .map(|option| format!("`{option}`"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn actor_list(actors: &[ActionActor]) -> String {
    if actors.is_empty() {
        return "nenhum".to_owned();
    }
    actors
        .iter()
        .map(|actor| format!("`{actor}`"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn valid_iso_date(texto: &str) -> bool {
    if !texto.is_ascii() || texto.len() < 10 {
        return false;
    }
    let (data, rest) = texto.split_at(10);
    if !valid_date(data) {
        return false;
    }
    match rest.strip_prefix('T') {
        None => rest.is_empty(),
        Some(time) => valid_time(time),
    }
}

fn valid_date(data: &str) -> bool {
    let parts: Vec<&str> = data.split('-').collect();
    let [year, month, day] = parts.as_slice() else {
        return false;
    };
    if year.len() != 4 || month.len() != 2 || day.len() != 2 {
        return false;
    }
    if !digits(year) || !digits(month) || !digits(day) {
        return false;
    }
    let month = month.parse::<u32>().unwrap_or(0);
    let day = day.parse::<u32>().unwrap_or(0);
    (1..=12).contains(&month) && (1..=31).contains(&day)
}

fn valid_time(time: &str) -> bool {
    let (clock, zone) = match time.find(['Z', '+', '-']) {
        Some(position) => time.split_at(position),
        None => (time, ""),
    };
    let (hms, fraction) = match clock.split_once('.') {
        Some((hms, fraction)) => (hms, Some(fraction)),
        None => (clock, None),
    };
    let parts: Vec<&str> = hms.split(':').collect();
    let (h, m, s) = match parts.as_slice() {
        [h, m] => (*h, *m, None),
        [h, m, s] => (*h, *m, Some(*s)),
        _ => return false,
    };
    let within = |texto: &str, cap: u32| {
        texto.len() == 2 && digits(texto) && texto.parse::<u32>().is_ok_and(|value| value <= cap)
    };
    if !within(h, 23) || !within(m, 59) {
        return false;
    }
    if let Some(second) = s
        && !within(second, 59)
    {
        return false;
    }
    if let Some(fraction) = fraction
        && !digits(fraction)
    {
        return false;
    }
    match zone {
        "" | "Z" => true,
        texto if texto.starts_with('+') || texto.starts_with('-') => {
            let parts: Vec<&str> = texto[1..].split(':').collect();
            let [zone_hour, zone_minute] = parts.as_slice() else {
                return false;
            };
            within(zone_hour, 23) && within(zone_minute, 59)
        }
        _ => false,
    }
}

fn digits(texto: &str) -> bool {
    !texto.is_empty() && texto.bytes().all(|byte| byte.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_date_accepts_documented_forms() {
        for texto in [
            "2026-01-01",
            "2026-11-22T14:42",
            "2026-11-22T14:42:09",
            "2026-11-22T14:42:09Z",
            "2026-11-22T14:42:09-03:00",
            "2026-11-22T14:42:09.123+00:00",
        ] {
            assert!(valid_iso_date(texto), "recusou `{texto}`");
        }
    }

    #[test]
    fn iso_date_rejects_non_iso() {
        for texto in [
            "22/11/2026",
            "2026-11",
            "2026-13-01",
            "2026-00-10",
            "2026-01-32",
            "2026-01-01T24:00",
            "2026-01-01T10:61",
            "2026-01-01T10:00:61",
            "2026-01-01 10:00",
            "2026-01-01T10:00+3:00",
            "2026-01-01x",
            "ontem",
            "",
        ] {
            assert!(!valid_iso_date(texto), "aceitou `{texto}`");
        }
    }

    #[test]
    fn json_kind_describes_input() {
        assert_eq!(json_kind(&json!("x")), "texto");
        assert_eq!(json_kind(&json!(1)), "número");
        assert_eq!(json_kind(&json!(true)), "booleano");
        assert_eq!(json_kind(&json!([])), "lista");
        assert_eq!(json_kind(&json!({})), "objeto");
        assert_eq!(json_kind(&Json::Null), "nulo");
    }

    #[test]
    fn field_errors_follow_alphabetical_order_not_map_order() {
        let etapa = Etapa {
            id: "cadastro".to_owned(),
            nome: "Cadastro".to_owned(),
            instruction: None,
            action_actors: vec![ActionActor::Agent],
            campos: Vec::new(),
            actions: Vec::new(),
            on_enter: Vec::new(),
        };
        let mut campos = Campos::new();
        for chave in ["zeta", "alfa", "meio"] {
            campos.insert(chave.to_owned(), json!("x"));
        }

        let errors = validate_fields(&etapa, &campos).expect_err("a etapa não tem campo nenhum");
        assert_eq!(errors.len(), 3);
        assert!(errors[0].starts_with("`alfa`"), "{errors:?}");
        assert!(errors[1].starts_with("`meio`"), "{errors:?}");
        assert!(errors[2].starts_with("`zeta`"), "{errors:?}");
    }

    #[test]
    fn step_data_ignores_non_objects() {
        let mut dados = Dados::new();
        dados.insert("coleta".to_owned(), json!({ "cep": "20000-000" }));
        dados.insert("quebrada".to_owned(), json!("não é mapa"));
        assert_eq!(
            step_data(&dados, "coleta").get("cep"),
            Some(&json!("20000-000"))
        );
        assert!(step_data(&dados, "quebrada").is_empty());
        assert!(step_data(&dados, "ausente").is_empty());
    }
}
