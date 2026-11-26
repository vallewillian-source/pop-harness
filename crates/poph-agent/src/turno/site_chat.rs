use poph_store::{SiteReply, SiteTurnKey};
use poph_types::config::SiteChatConfig;

use super::*;

pub struct SiteRun {
    pub key: SiteTurnKey,
    pub agent: AgentId,
    pub message: MessageId,
}

pub enum SiteTurnError {
    Policy,
    AgentUnavailable,
    ModelUnavailable,
    Failed,
    Cancelled,
}

impl SiteTurnError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Policy => "agent_not_public",
            Self::AgentUnavailable => "agent_unavailable",
            Self::ModelUnavailable => "model_unavailable",
            Self::Failed => "turn_failed",
            Self::Cancelled => "turn_cancelled",
        }
    }
}

impl TurnEngine {
    pub async fn run_site(
        &self,
        job: SiteRun,
        policy: &SiteChatConfig,
        cancel: &Cancellation,
        observer: &mut dyn TurnObserver,
    ) -> Result<SiteReply, SiteTurnError> {
        let work = self.generate_site(job, policy, cancel, observer);
        tokio::select! {
            biased;
            () = cancel.wait() => Err(SiteTurnError::Cancelled),
            result = tokio::time::timeout(self.options.turn_timeout, work) => {
                result.unwrap_or(Err(SiteTurnError::Failed))
            }
        }
    }

    async fn generate_site(
        &self,
        job: SiteRun,
        policy: &SiteChatConfig,
        cancel: &Cancellation,
        observer: &mut dyn TurnObserver,
    ) -> Result<SiteReply, SiteTurnError> {
        let org = job.key.org;
        let id = job.key.conversation;
        let started = Instant::now();
        let conversation = self
            .store
            .conversations()
            .get(org, id)
            .map_err(|_| SiteTurnError::Failed)?;
        if conversation.canal != "site"
            || conversation.agent_id != job.agent
            || conversation.status != poph_store::ConversationStatus::Open
        {
            return Err(SiteTurnError::Failed);
        }
        let hot = self
            .config
            .load_verify(org, job.agent)
            .map_err(|_| SiteTurnError::AgentUnavailable)?;
        if !hot.can_open_turn() {
            return Err(SiteTurnError::AgentUnavailable);
        }
        if (hot.instructions.is_some() && hot.structured.is_none())
            || hot.tools.iter().any(|name| !policy.permits_tool(name))
        {
            return Err(SiteTurnError::Policy);
        }

        let _permit = self
            .limiter
            .acquire_turn(org, job.agent, self.options.slot_wait)
            .await
            .map_err(|_| SiteTurnError::Failed)?;
        let mut recent = self
            .read_recent_messages(org, id)
            .map_err(|_| SiteTurnError::Failed)?;
        recent.retain(|message| {
            matches!(message.role, Role::User | Role::Assistant)
                && message.origin == poph_store::Origin::Agente
        });
        let trigger = recent
            .iter()
            .find(|message| message.id == job.message)
            .ok_or(SiteTurnError::Failed)?;
        let passages = if hot.tool_enabled("consultar_base") {
            match self.start_search(org, job.agent, std::slice::from_ref(trigger), &hot) {
                Some(search) => search.await.unwrap_or_default(),
                None => Vec::new(),
            }
        } else {
            Vec::new()
        };
        let summary = summary::Summary::decode(conversation.summary.as_deref());
        let context = summary::build(
            &recent,
            &summary,
            u64::from(hot.agente.max_context_tokens),
            hot.agente
                .max_context_messages
                .clamp(1, self.options.message_read_limit.max(1)),
        );
        let block = prompt::BlockVolatile {
            now_ms: self.store.now(),
            canal: &conversation.canal,
            status: conversation_status(conversation.status),
            conversa: &id.to_string(),
            summary: context.summary.as_deref(),
            passages: &passages,
            catalog_of_products: hot.tool_enabled("buscar_produto"),
            product_chosen: None,
            fluxos: None,
        }
        .build();
        let messages = Prompt::new(hot.markdown.clone())
            .with_volatile(block)
            .messages(
                context
                    .messages
                    .iter()
                    .filter_map(prompt::of_message_stored)
                    .collect(),
            );
        self.turn_provider(&hot)
            .map_err(|_| SiteTurnError::ModelUnavailable)?;
        let (reply, _) = self
            .run_model_loop(
                TurnTarget {
                    org,
                    conversa: id,
                    turno: job.key.turn,
                },
                &hot,
                PreparedTurn {
                    trigger_message_id: job.message,
                    messages,
                    usage: UseTools::new(),
                    calls: RecordCalls::new(),
                    skip_knowledge_and_products: false,
                    skip_product_search: false,
                    active_flow: false,
                    flow_fallback: false,
                },
                cancel,
                observer,
                started,
            )
            .await
            .map_err(|_| SiteTurnError::Failed)?;
        let text = reply.texto.trim().to_owned();
        if text.is_empty() {
            return Err(SiteTurnError::Failed);
        }
        Ok(SiteReply {
            text,
            input_tokens: u32::try_from(reply.usage.input).unwrap_or(u32::MAX),
            output_tokens: u32::try_from(reply.usage.saida).unwrap_or(u32::MAX),
        })
    }
}
