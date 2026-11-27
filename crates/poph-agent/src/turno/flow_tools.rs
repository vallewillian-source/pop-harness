use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use poph_flows::engine::{self, Context as ContextEngine, Estado as StateEngine, execute_action};
use poph_flows::ir::{Destination, Ir};
use poph_store::{Execution, Ficha, Store, StoreError, TransitionExtras};
use poph_types::{ActionActor, FlowsConfig, RecordStatus};
use serde_json::{Map, Value};

use super::flow::{self, AdjustmentsFlow, Initiator};
use super::tools::{DefaultRegistry, InternalTool, ToolContext, ToolFuture};
use crate::flows_transition::transition_engine;

pub const NAME_RUN_ACTION: &str = "fluxo_executar_acao";
pub const NAME_ABANDON: &str = "fluxo_abandonar";
pub const NAME_START: &str = flow::NAME_TOOL_START;

pub fn is_flow_tool(nome: &str) -> bool {
    matches!(nome, NAME_RUN_ACTION | NAME_ABANDON | NAME_START)
}

#[derive(Clone)]
struct FlowTurn {
    store: Arc<Store>,
    adjustments: AdjustmentsFlow,
}

pub fn register(
    registro: DefaultRegistry,
    store: Arc<Store>,
    config: &FlowsConfig,
) -> DefaultRegistry {
    register_with(registro, store, AdjustmentsFlow::of_config(config))
}

pub fn register_with(
    registro: DefaultRegistry,
    store: Arc<Store>,
    adjustments: AdjustmentsFlow,
) -> DefaultRegistry {
    if !adjustments.ativo() {
        return registro;
    }
    let common = FlowTurn { store, adjustments };
    registro
        .com(Arc::new(RunAction(common.clone())) as Arc<dyn InternalTool>)
        .com(Arc::new(Abandon(common.clone())) as Arc<dyn InternalTool>)
        .com(Arc::new(Start(common)) as Arc<dyn InternalTool>)
}

pub fn register_closure(registro: DefaultRegistry, store: Arc<Store>) -> DefaultRegistry {
    registro.com(Arc::new(CloseService(store)) as Arc<dyn InternalTool>)
}

struct CloseService(Arc<Store>);

impl InternalTool for CloseService {
    fn nome(&self) -> &'static str {
        "atendimento_encerrar"
    }
    fn descricao(&self) -> &'static str {
        "Encerra o atendimento quando concluído. Escreva a despedida na resposta; esta ferramenta não envia mensagem."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({"type":"object", "properties":{"motivo":{"type":"string"}},
            "required":["motivo"], "additionalProperties":false})
    }
    fn run(&self, context: ToolContext, arguments: Value) -> ToolFuture {
        let store = Arc::clone(&self.0);
        Box::pin(async move {
            let motivo = text_required(&arguments, "motivo")?;
            match store.conversations().close_by_agent(
                context.org,
                context.conversa,
                context.agente,
                motivo,
            ) {
                Ok(true) => {
                    Ok("Atendimento encerrado. Responda ao cliente com a despedida.".to_owned())
                }
                Ok(false) => Ok("Atendimento já encerrado; nada mudou.".to_owned()),
                Err(_) => Err("Não foi possível encerrar o atendimento.".to_owned()),
            }
        })
    }
}

struct RunAction(FlowTurn);

impl InternalTool for RunAction {
    fn nome(&self) -> &'static str {
        NAME_RUN_ACTION
    }

    fn descricao(&self) -> &'static str {
        "Executa uma ação da etapa atual do fluxo de atendimento em andamento (a lista de ações \
         válidas está no bloco <fluxo_em_andamento> do prompt). Use com os campos que a etapa \
         coleta quando o cliente já os informou. Se a ação não existir na etapa, a resposta traz as \
         válidas e nada muda."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "acao": {
                    "type": "string",
                    "description": "Id da ação, exatamente como aparece em `Ações válidas` no bloco do fluxo."
                },
                "campos": {
                    "type": "object",
                    "description": "Campos da etapa que o cliente informou nesta mensagem, no formato {\"campo\": \"valor\"}."
                },
                "chave_idempotencia": {
                    "type": "string",
                    "description": "Opcional. Repetir a mesma chave devolve o mesmo desfecho, sem gravar de novo. Sem ela, a chave é derivada da própria ação."
                }
            },
            "required": ["acao"],
            "additionalProperties": false
        })
    }

    fn run(&self, context: ToolContext, arguments: Value) -> ToolFuture {
        let common = self.0.clone();
        Box::pin(async move { common.run_action(context, arguments) })
    }
}

impl FlowTurn {
    fn run_action(&self, context: ToolContext, arguments: Value) -> Result<String, String> {
        let acao = text_required(&arguments, "acao")?;
        let campos = fields_argument(&arguments)?;
        let informed = key_optional(&arguments)?;
        let (ficha, execution) = self.case_execution(&context)?;
        let chave = informed.unwrap_or_else(|| {
            key_derived(
                &execution,
                &execution.no_atual,
                &acao,
                &Value::Object(campos.clone()),
            )
        });
        if let Some(estado) = self.repetition_already_applied(&context, &execution, &chave)? {
            return Ok(format!(
                "Esta ação já tinha sido registrada; nada mudou.\n{}",
                estado
            ));
        }
        let load = match flow::load_for_act(&self.adjustments, &self.store, context.org, &execution)
        {
            Some(load) => load,
            None => return Err(could_not_read_the_flow()),
        };
        let text_of_case = ficha.id.to_string();
        let mut engine_context = ContextEngine::new(&load.evaluator)
            .with_case(&text_of_case)
            .with_jump_limit(self.adjustments.jumps());
        if let Some(peer) = ficha.peer_externo.as_deref() {
            engine_context = engine_context.with_client(peer);
        }
        let resultado = execute_action(
            &load.ir,
            &load.estado,
            &acao,
            &campos,
            ActionActor::Agent,
            Some(&chave),
            &engine_context,
        )
        .map_err(|error| format!("erro: {error}\n{}", describe(&load.ir, &load.estado)))?;
        let stored = self
            .store
            .executions()
            .apply_transition(
                context.org,
                execution.id,
                execution.version,
                &transition_engine(&resultado),
                &chave,
                TransitionExtras {
                    ator_ref: None,
                    fechada_por: None,
                    close_reason: None,
                },
            )
            .map_err(|error| error_of_transition(error, &acao))?;
        Ok(response_of_action(
            &load.ir,
            &load.estado,
            &resultado,
            stored.repeated,
        ))
    }

    fn repetition_already_applied(
        &self,
        context: &ToolContext,
        execution: &Execution,
        chave: &str,
    ) -> Result<Option<String>, String> {
        let applied = self
            .store
            .executions()
            .transition_already_applied(context.org, execution.id, chave)
            .map_err(|error| failure_of_read(error, "a chave da ação"))?;
        if !applied {
            return Ok(None);
        }
        let Some(load) = flow::load_for_act(&self.adjustments, &self.store, context.org, execution)
        else {
            return Ok(Some(String::new()));
        };
        Ok(Some(describe(&load.ir, &load.estado)))
    }

    fn case_execution(&self, context: &ToolContext) -> Result<(Ficha, Execution), String> {
        let ficha = self
            .store
            .cases()
            .by_conversation(context.org, context.conversa)
            .map_err(|error| failure_of_read(error, "a Ficha do atendimento"))?
            .ok_or_else(|| {
                "erro: este atendimento ainda não tem Ficha; não há fluxo para agir".to_owned()
            })?;
        if ficha.status == RecordStatus::Closed {
            return Err(
                "erro: o atendimento já foi encerrado; a próxima mensagem do cliente abre outro"
                    .to_owned(),
            );
        }
        let execution = self
            .store
            .executions()
            .active_of_case(context.org, ficha.id)
            .map_err(|error| failure_of_read(error, "a execução do fluxo"))?
            .ok_or_else(|| {
                "erro: não há fluxo em andamento neste atendimento; siga a conversa normalmente"
                    .to_owned()
            })?;
        Ok((ficha, execution))
    }
}

struct Abandon(FlowTurn);

impl InternalTool for Abandon {
    fn nome(&self) -> &'static str {
        NAME_ABANDON
    }

    fn descricao(&self) -> &'static str {
        "Abandona o fluxo de atendimento em andamento, com um motivo curto que fica no histórico. \
         Use quando o cliente desistir ou mudar de assunto e o roteiro não servir mais. Não envia \
         nada ao cliente: a despedida é o texto da sua resposta."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "motivo": {
                    "type": "string",
                    "description": "Por que o fluxo está sendo abandonado, em poucas palavras."
                },
                "chave_idempotencia": {
                    "type": "string",
                    "description": "Opcional. Repetir a mesma chave não repete o evento de abandono."
                }
            },
            "required": ["motivo"],
            "additionalProperties": false
        })
    }

    fn run(&self, context: ToolContext, arguments: Value) -> ToolFuture {
        let common = self.0.clone();
        Box::pin(async move { common.run_abandono(context, arguments) })
    }
}

impl FlowTurn {
    fn run_abandono(&self, context: ToolContext, arguments: Value) -> Result<String, String> {
        let motivo = text_required(&arguments, "motivo")?;
        let informed = key_optional(&arguments)?;
        let (_case, execution) = self.case_execution(&context)?;
        let chave = informed.unwrap_or_else(|| {
            key_derived(&execution, &execution.no_atual, "abandonar", &Value::Null)
        });
        if self
            .store
            .executions()
            .transition_already_applied(context.org, execution.id, &chave)
            .map_err(|error| failure_of_read(error, "a chave do abandono"))?
        {
            return Ok("Este fluxo já tinha sido abandonado; nada mudou.".to_owned());
        }
        let load = match flow::load_for_act(&self.adjustments, &self.store, context.org, &execution)
        {
            Some(load) => load,
            None => return Err(could_not_read_the_flow()),
        };
        let resultado = engine::abandon(&load.estado, &motivo, ActionActor::Agent, Some(&chave))
            .map_err(|error| format!("erro: {error}"))?;
        self.store
            .executions()
            .apply_transition(
                context.org,
                execution.id,
                execution.version,
                &transition_engine(&resultado),
                &chave,
                TransitionExtras {
                    ator_ref: None,
                    fechada_por: None,
                    close_reason: None,
                },
            )
            .map_err(|error| error_of_transition(error, "abandonar"))?;
        Ok(format!(
            "Fluxo \"{}\" abandonado. A Ficha continua aberta; siga a conversa sem o roteiro.",
            load.ir.fluxo.nome
        ))
    }
}

struct Start(FlowTurn);

impl InternalTool for Start {
    fn nome(&self) -> &'static str {
        NAME_START
    }

    fn descricao(&self) -> &'static str {
        "Inicia um fluxo de atendimento liberado para você, pelo id que aparece na lista \
         <fluxos_de_atendimento>. Use só quando a mensagem do cliente pedir claramente um dos \
         fluxos; depois de iniciar, siga a etapa que a resposta indicar."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "fluxo_id": {
                    "type": "string",
                    "description": "Id do fluxo, exatamente como aparece em <fluxos_de_atendimento>."
                }
            },
            "required": ["fluxo_id"],
            "additionalProperties": false
        })
    }

    fn run(&self, context: ToolContext, arguments: Value) -> ToolFuture {
        let common = self.0.clone();
        Box::pin(async move { common.run_beginning(context, arguments) })
    }
}

impl FlowTurn {
    fn run_beginning(&self, context: ToolContext, arguments: Value) -> Result<String, String> {
        let request = text_required(&arguments, "fluxo_id")?;
        let ficha = self
            .store
            .cases()
            .by_conversation(context.org, context.conversa)
            .map_err(|error| failure_of_read(error, "a Ficha do atendimento"))?
            .ok_or_else(|| {
                "erro: este atendimento ainda não tem Ficha; não há fluxo para iniciar".to_owned()
            })?;
        if ficha.status == RecordStatus::Closed {
            return Err(
                "erro: o atendimento já foi encerrado; a próxima mensagem do cliente abre outro"
                    .to_owned(),
            );
        }
        let slug = self.slug_request(context.org, ficha.agent_id, &request)?;
        match flow::start(
            &self.adjustments,
            &self.store,
            context.org,
            &ficha,
            &slug,
            Initiator::Agente,
            None,
        )
        .map_err(|error| failure_of_read(error, "a abertura do fluxo"))?
        {
            flow::Beginning::Open { execution, effects } => {
                let Some(load) =
                    flow::load_for_act(&self.adjustments, &self.store, context.org, &execution)
                else {
                    return Ok(format!(
                        "Fluxo `{slug}` iniciado. O bloco da etapa entra no próximo turno."
                    ));
                };
                let mut response = format!(
                    "Fluxo \"{}\" iniciado.\n{}",
                    load.ir.fluxo.nome,
                    describe(&load.ir, &load.estado)
                );
                if effects > 0 {
                    response.push_str(&note_of_effect());
                }
                Ok(response)
            }
            flow::Beginning::AlreadyActive => Err(
                "erro: já existe um fluxo em andamento neste atendimento; continue nele".to_owned(),
            ),
            flow::Beginning::Rejected(motivo) => Err(format!("erro: {motivo}")),
        }
    }

    fn slug_request(
        &self,
        org: poph_types::OrgId,
        agente: poph_types::AgentId,
        request: &str,
    ) -> Result<String, String> {
        let request = request.trim();
        let released = self
            .store
            .flows()
            .list_from_agent(org, agente)
            .map_err(|error| failure_of_read(error, "os fluxos liberados"))?;
        if let Some(fluxo) = released.iter().find(|fluxo| fluxo.slug == request) {
            return Ok(fluxo.slug.clone());
        }
        let lowercase = request.to_lowercase();
        let finding = released.iter().find(|fluxo| {
            fluxo.slug.to_lowercase() == lowercase || fluxo.name.to_lowercase() == lowercase
        });
        match finding {
            Some(fluxo) => Ok(fluxo.slug.clone()),
            None => Err(format!(
                "erro: o fluxo `{request}` não está entre os liberados para este atendimento; \
                 use um id de <fluxos_de_atendimento>"
            )),
        }
    }
}

fn note_of_effect() -> String {
    format!("\n{}", flow::NOTE_OF_EFFECT)
}

fn response_of_action(
    ir: &Ir,
    previous: &StateEngine,
    resultado: &engine::Resultado,
    repeated: bool,
) -> String {
    let mut saida = String::new();
    if repeated {
        saida.push_str("Esta ação já tinha sido registrada; nada mudou.\n");
    }
    let new = &resultado.new_state;
    let advanced = new.no_atual.id() != previous.no_atual.id();
    match &new.no_atual {
        Destination::Etapa { .. } if advanced => {
            saida.push_str(&format!(
                "Ação registrada: o fluxo \"{}\" avançou.\n",
                ir.fluxo.nome
            ));
        }
        Destination::Etapa { .. } => {
            saida.push_str(&format!(
                "Ação registrada: o fluxo \"{}\" continua na mesma etapa.\n",
                ir.fluxo.nome
            ));
        }
        Destination::Fim { .. } | Destination::Decisao { .. } => {
            saida.push_str(&format!(
                "Ação registrada: o fluxo \"{}\" chegou ao desfecho.\n",
                ir.fluxo.nome
            ));
        }
    }
    saida.push_str(&describe(ir, new));
    if !resultado.effects.is_empty() {
        saida.push_str(&note_of_effect());
    }
    saida
}

fn describe(ir: &Ir, estado: &StateEngine) -> String {
    let mut saida = String::new();
    match &estado.no_atual {
        Destination::Etapa { id } => {
            let Some(etapa) = ir.stages.iter().find(|etapa| &etapa.id == id) else {
                return format!(
                    "Fluxo \"{}\": a etapa atual não existe na revisão publicada.",
                    ir.fluxo.nome
                );
            };
            saida.push_str(&format!("Etapa atual: \"{}\".", etapa.nome));
            if let Some(instruction) = etapa
                .instruction
                .as_deref()
                .map(str::trim)
                .filter(|texto| !texto.is_empty())
            {
                saida.push_str(&format!("\nOrientação da etapa: {instruction}"));
            }
            let missing = engine::missing_fields(etapa, &estado.dados);
            saida.push_str(&format!(
                "\nCampos obrigatórios que faltam: {}.",
                list(&missing)
            ));
            let mut waiting = engine::required_actors(etapa);
            waiting.retain(|actor| !estado.actors_who_acted.contains(actor));
            if !waiting.is_empty() {
                saida.push_str(&format!(
                    "\nQuem ainda precisa agir nesta etapa: {}.",
                    list_of_actors(&waiting)
                ));
            }
            saida.push_str(&format!("\nAções válidas: {}.", list_of_actions(etapa)));
        }
        Destination::Decisao { .. } => {
            saida.push_str(
                "A execução está numa decisão do fluxo: o sistema resolve antes de qualquer ação.",
            );
        }
        Destination::Fim { resultado, .. } => {
            let desfecho = match resultado {
                Some(texto) => format!(" com o resultado \"{texto}\""),
                None => String::new(),
            };
            saida.push_str(&format!("A execução chegou ao fim{desfecho}."));
            if estado.status != poph_types::ExecutionStatus::Active {
                saida.push_str(" Não há mais ação a executar neste fluxo.");
            }
        }
    }
    saida
}

fn list(items: &[String]) -> String {
    if items.is_empty() {
        return "nenhum".to_owned();
    }
    items
        .iter()
        .map(|item| format!("`{item}`"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn list_of_actors(actors: &[ActionActor]) -> String {
    actors
        .iter()
        .map(|actor| actor.as_text().to_owned())
        .collect::<Vec<_>>()
        .join(", ")
}

fn list_of_actions(etapa: &poph_flows::ir::Etapa) -> String {
    if etapa.actions.is_empty() {
        return "nenhuma (a etapa não tem saída)".to_owned();
    }
    etapa
        .actions
        .iter()
        .map(|acao| format!("`{}` (\"{}\")", acao.id, acao.label))
        .collect::<Vec<_>>()
        .join(", ")
}

fn text_required(arguments: &Value, campo: &str) -> Result<String, String> {
    arguments
        .get(campo)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|texto| !texto.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| format!("erro: o argumento `{campo}` é obrigatório e não pode ser vazio"))
}

fn fields_argument(arguments: &Value) -> Result<Map<String, Value>, String> {
    match arguments.get("campos") {
        None | Some(Value::Null) => Ok(Map::new()),
        Some(Value::Object(map)) => Ok(map.clone()),
        Some(_) => {
            Err("erro: o argumento `campos` tem de ser um objeto {\"campo\": \"valor\"}".to_owned())
        }
    }
}

fn key_optional(arguments: &Value) -> Result<Option<String>, String> {
    match arguments.get("chave_idempotencia") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(texto)) if texto.trim().is_empty() => Ok(None),
        Some(Value::String(texto)) => Ok(Some(texto.trim().to_owned())),
        Some(_) => Err("erro: o argumento `chave_idempotencia` tem de ser texto".to_owned()),
    }
}

fn key_derived(execution: &Execution, no_atual: &str, acao: &str, campos: &Value) -> String {
    let mut hasher = DefaultHasher::new();
    campos.to_string().hash(&mut hasher);
    format!(
        "agente:{}:{no_atual}:{acao}:{:016x}",
        execution.id,
        hasher.finish()
    )
}

fn failure_of_read(error: StoreError, what: &str) -> String {
    tracing::error!(error = %error, what, "ferramenta do fluxo falhou no armazenamento");
    format!("erro: não consegui ler {what} agora; siga a conversa sem o fluxo nesta mensagem")
}

fn error_of_transition(error: StoreError, acao: &str) -> String {
    match error {
        StoreError::Domain(poph_types::DomainError::VersionConflict { .. }) => format!(
            "erro: outra pessoa agiu neste fluxo ao mesmo tempo e a etapa mudou; a ação `{acao}` \
             não foi gravada"
        ),
        StoreError::Domain(domain) => format!("erro: {domain}"),
        other => failure_of_read(other, "a gravação do fluxo"),
    }
}

fn could_not_read_the_flow() -> String {
    "erro: não consegui ler a etapa atual deste fluxo; siga a conversa sem ele nesta mensagem"
        .to_owned()
}

#[cfg(test)]
mod tests {
    use poph_types::{DomainError, ExecutionId};

    use poph_store::StoreError;

    use super::{error_of_transition, key_derived};

    fn execution() -> poph_store::Execution {
        poph_store::Execution {
            id: ExecutionId::generate(),
            org_id: poph_types::OrgId::generate(),
            ficha_id: poph_types::RecordId::generate(),
            fluxo_id: poph_types::FlowId::generate(),
            revisao_id: poph_types::RevisionId::generate(),
            status: poph_types::ExecutionStatus::Active,
            no_atual: "coleta".to_owned(),
            dados_json: "{}".to_owned(),
            result: None,
            motivo_fim: None,
            iniciada_em: 0,
            finalizada_em: None,
            version: 1,
        }
    }

    #[test]
    fn key_derived_stable_changes_with_a_call() {
        let execution = execution();
        let campos = serde_json::json!({ "nome": "Ana Teste" });
        let first = key_derived(&execution, "coleta", "seguir", &campos);
        assert_eq!(
            first,
            key_derived(&execution, "coleta", "seguir", &campos),
            "a mesma chamada tem a mesma chave"
        );
        assert_ne!(
            first,
            key_derived(
                &execution,
                "coleta",
                "seguir",
                &serde_json::json!({"nome": "Bia"})
            ),
            "outro valor de campo é outra chamada"
        );
        assert_ne!(
            first,
            key_derived(&execution, "confirmacao", "seguir", &campos),
            "a mesma ação noutra etapa é outra chamada"
        );
        assert!(first.starts_with("agente:"), "{first}");
    }

    #[test]
    fn conflict_of_version_becomes_notice_of_corrida() {
        let error = StoreError::Domain(DomainError::VersionConflict {
            entity: "execução de fluxo",
            id: "execucao-ficticia".to_owned(),
            expected: 1,
            current: 2,
        });
        let texto = error_of_transition(error, "seguir");
        assert!(texto.contains("outra pessoa agiu"), "{texto}");
        assert!(texto.contains("`seguir`"), "{texto}");
    }
}
