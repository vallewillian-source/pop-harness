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

impl Motor {
    pub async fn run_site(
        &self,
        job: SiteRun,
        policy: &SiteChatConfig,
        cancel: &Cancellation,
        observer: &mut dyn Observador,
    ) -> Result<SiteReply, SiteTurnError> {
        let work = self.generate_site(job, policy, cancel, observer);
        tokio::select! {
            biased;
            () = cancel.wait() => Err(SiteTurnError::Cancelled),
            result = tokio::time::timeout(self.opcoes.tempo_maximo_turno, work) => {
                result.unwrap_or(Err(SiteTurnError::Failed))
            }
        }
    }

    async fn generate_site(
        &self,
        job: SiteRun,
        policy: &SiteChatConfig,
        cancel: &Cancellation,
        observer: &mut dyn Observador,
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
            .carregar_verificando(org, job.agent)
            .map_err(|_| SiteTurnError::AgentUnavailable)?;
        if !hot.pode_abrir_turno() {
            return Err(SiteTurnError::AgentUnavailable);
        }
        if (hot.instrucoes.is_some() && hot.estruturado.is_none())
            || hot
                .ferramentas
                .iter()
                .any(|name| !policy.permits_tool(name))
        {
            return Err(SiteTurnError::Policy);
        }

        let _permit = self
            .limitador
            .adquirir_turno(org, job.agent, self.opcoes.espera_vaga)
            .await
            .map_err(|_| SiteTurnError::Failed)?;
        let mut recent = self
            .ultimas_mensagens(org, id)
            .map_err(|_| SiteTurnError::Failed)?;
        recent.retain(|message| {
            matches!(message.role, Role::User | Role::Assistant)
                && message.origin == poph_store::Origin::Agente
        });
        let trigger = recent
            .iter()
            .find(|message| message.id == job.message)
            .ok_or(SiteTurnError::Failed)?;
        let passages = if hot.ferramenta_habilitada("consultar_base") {
            match self.iniciar_busca(org, job.agent, std::slice::from_ref(trigger), &hot) {
                Some(search) => search.await.unwrap_or_default(),
                None => Vec::new(),
            }
        } else {
            Vec::new()
        };
        let summary = resumo::Resumo::decodificar(conversation.summary.as_deref());
        let context = resumo::montar(
            &recent,
            &summary,
            u64::from(hot.agente.max_context_tokens),
            hot.agente
                .max_context_messages
                .clamp(1, self.opcoes.teto_leitura_mensagens.max(1)),
        );
        let block = prompt::BlocoVolatil {
            agora_ms: self.store.now(),
            canal: &conversation.canal,
            status: status_do_atendimento(conversation.status),
            conversa: &id.to_string(),
            resumo: context.resumo.as_deref(),
            trechos: &passages,
            catalogo_de_produtos: hot.ferramenta_habilitada("buscar_produto"),
            produto_escolhido: None,
            fluxos: None,
        }
        .montar();
        let messages = Prompt::novo(hot.markdown.clone())
            .com_volatil(block)
            .mensagens(
                context
                    .mensagens
                    .iter()
                    .filter_map(prompt::da_mensagem_gravada)
                    .collect(),
            );
        self.provedor_do_turno(&hot)
            .map_err(|_| SiteTurnError::ModelUnavailable)?;
        let (reply, _) = self
            .conversar(
                Alvo {
                    org,
                    conversa: id,
                    turno: job.key.turn,
                },
                &hot,
                Preparo {
                    trigger_message_id: job.message,
                    mensagens: messages,
                    uso: UsoFerramentas::nova(),
                    chamadas: RegistroChamadas::nova(),
                    sem_base_produto: false,
                    sem_buscar_produto: false,
                    fluxo_em_andamento: false,
                    fluxo_reserva: false,
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
            input_tokens: u32::try_from(reply.uso.input).unwrap_or(u32::MAX),
            output_tokens: u32::try_from(reply.uso.saida).unwrap_or(u32::MAX),
        })
    }
}
