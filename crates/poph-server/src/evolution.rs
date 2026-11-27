use axum::Json;
use axum::body::Bytes;
use axum::extract::rejection::BytesRejection;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use poph_evolution::webhook::{
    AttachmentToDownload, Decision, Policy, ReceivedEntry, SECRET_HEADER, check_secret, interpret,
};
use poph_evolution::{ATTACHMENT_CAP, ConfigEvolution, EvolutionClient, EvolutionError};
use poph_files::{Content, FileConfig, FileError};
use poph_store::{
    NewChannelAttachment, NewChannelInboxItem, NewDeviceMessage, Received, ReceivedFromDevice,
    ReceivedWithMessage,
};
use poph_types::{HarnessConfig, InboundEvent, IntegrationId, OrgId, Secret, StateEvent};
use serde_json::{Value, json};

use crate::error::ApiError;
use crate::state::ServerState;

#[derive(Clone)]
pub struct EvolutionChannel {
    pub org: OrgId,
    pub instance: String,
    pub integration: IntegrationId,
    pub secret: Option<Secret>,
    pub client: Option<EvolutionClient>,
    pub attachment_limit: u64,
}

impl std::fmt::Debug for EvolutionChannel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CanalEvolution")
            .field("org", &self.org)
            .field("instancia", &self.instance)
            .field("integracao", &self.integration)
            .field("segredo", &self.secret.is_some())
            .field("cliente", &self.client.is_some())
            .finish()
    }
}

pub fn channels_from_config(
    config: &HarnessConfig,
    evolution_key: Option<Secret>,
    read_env: &dyn Fn(&str) -> Option<String>,
) -> Result<Vec<EvolutionChannel>, String> {
    let Some(evolution) = config.evolution.as_ref() else {
        return Ok(Vec::new());
    };
    let mut channels = Vec::with_capacity(evolution.instances.len());
    for instance in &evolution.instances {
        let org: OrgId = instance
            .org_id
            .trim()
            .parse()
            .map_err(|_| format!("org inválida na instância `{}`", instance.instance))?;
        let integration: IntegrationId = instance
            .integration_id
            .trim()
            .parse()
            .map_err(|_| format!("integração inválida na instância `{}`", instance.instance))?;
        let name = instance.instance.trim().to_owned();

        let secret = read_env(&instance.secret_env)
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
            .map(Secret::new);
        if secret.is_none() {
            tracing::warn!(
                target: "poph_server::evolution",
                instance = %name,
                variable = %instance.secret_env,
                "variável do segredo do webhook não definida: a instância responde 401 até ela existir"
            );
        }

        let client = match evolution_key.clone() {
            None => {
                tracing::warn!(
                    target: "poph_server::evolution",
                    instance = %name,
                    "EVOLUTION_API_KEY ausente: a instância recebe texto, mas não baixa mídia"
                );
                None
            }
            Some(key) => ConfigEvolution::new(evolution.url_base.clone(), name.clone(), key)
                .and_then(EvolutionClient::new)
                .map_err(|error| {
                    tracing::warn!(
                        target: "poph_server::evolution",
                        instance = %name,
                        reason = %config_error_category(&error),
                        "cliente da Evolution não montado: a instância não baixa mídia"
                    );
                })
                .ok(),
        };

        channels.push(EvolutionChannel {
            org,
            instance: name,
            integration,
            secret,
            client,
            attachment_limit: ATTACHMENT_CAP,
        });
    }
    Ok(channels)
}

fn config_error_category(error: &EvolutionError) -> &'static str {
    match error {
        EvolutionError::InvalidConfiguration(_) => "configuração",
        _ => "erro interno do cliente",
    }
}

pub async fn receive(
    State(state): State<ServerState>,
    Path((org_id, instance)): Path<(String, String)>,
    headers: HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> Result<Json<Value>, ApiError> {
    let channel = state
        .evolution_channel(&org_id, &instance)
        .cloned()
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::NOT_FOUND,
                "not_found",
                "instância do WhatsApp não configurada neste servidor",
            )
        })?;

    let received = headers
        .get(SECRET_HEADER)
        .and_then(|value| value.to_str().ok());
    let authorized = channel
        .secret
        .as_ref()
        .is_some_and(|secret| check_secret(secret, received).is_ok());
    if !authorized {
        return Err(ApiError::new(
            StatusCode::UNAUTHORIZED,
            "webhook_nao_autorizado",
            "webhook recusado: segredo ausente ou inválido",
        ));
    }

    let body = body.map_err(|_| {
        ApiError::invalid("corpo do webhook inválido ou maior que o limite de 1 MB")
    })?;
    let value: Value = serde_json::from_slice(&body)
        .map_err(|_| ApiError::invalid("corpo do webhook não é JSON"))?;
    if !value.is_object() {
        return Err(ApiError::invalid(
            "corpo do webhook deve ser um objeto JSON",
        ));
    }

    let policy = Policy {
        accept_from_me: matrix_module_enabled(&state, &channel)?,
    };
    let decision = interpret(channel.integration, &value, &policy);
    match decision {
        Decision::Ignored(_) => Ok(Json(json!({"recebido": true, "efeito": "nenhum"}))),
        Decision::State(event) => receive_state(&state, &channel, &event),
        Decision::Inbound(inbound) => receive_inbound(&state, &channel, inbound).await,
    }
}

fn receive_state(
    state: &ServerState,
    channel: &EvolutionChannel,
    event: &StateEvent,
) -> Result<Json<Value>, ApiError> {
    let Some(text) = state_label(event) else {
        return Ok(Json(json!({"recebido": true, "efeito": "nenhum"})));
    };
    let updated = state
        .store
        .set_integration_connection_state(channel.org, channel.integration, text)
        .map_err(|error| store_failure("estado da integração", &error))?;
    if !updated {
        tracing::warn!(
            target: "poph_server::evolution",
            instance = %channel.instance,
            "estado recebido para uma integração que não existe nesta org; nada foi criado"
        );
        return Ok(Json(
            json!({"recebido": true, "efeito": "integracao_inexistente"}),
        ));
    }
    tracing::info!(
        target: "poph_server::evolution",
        org = %channel.org,
        instance = %channel.instance,
        state = text,
        "estado de conexão do WhatsApp atualizado"
    );
    Ok(Json(json!({"recebido": true, "efeito": "estado"})))
}

fn state_label(event: &StateEvent) -> Option<&'static str> {
    match event {
        StateEvent::Connected { .. } => Some("connected"),
        StateEvent::Disconnected { .. } => Some("disconnected"),
        StateEvent::Qr { .. } => Some("awaiting_scan"),
        StateEvent::Error { .. } => None,
    }
}

async fn receive_inbound(
    state: &ServerState,
    channel: &EvolutionChannel,
    inbound: ReceivedEntry,
) -> Result<Json<Value>, ApiError> {
    if typed_on_device(&inbound.event) {
        return receive_from_device(state, channel, inbound);
    }

    let already_received = state
        .store
        .inbox()
        .delivery_received(channel.integration, &inbound.event.id_externo)
        .map_err(|error| store_failure("consulta da entrada", &error))?;
    if let Some(received) = already_received {
        return Ok(Json(inbound_response(&received)));
    }

    let integration = webhook_integration(state, channel)?;

    let attachments = download_attachments(state, channel, &inbound.attachments).await?;

    let event = inbound.event;
    let event_json = serde_json::to_string(&event).map_err(|_| ApiError::internal())?;
    let customer_name = event
        .metadata
        .get("push_name")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned);
    let new = NewChannelInboxItem {
        integration_id: channel.integration,
        agent_id: integration.agent_id,
        canal: "whatsapp".to_owned(),
        external_id: event.id_externo,
        peer: event.peer,
        event_json,
        texto: event.text,
        attachments,
        customer_name,
    };
    let received = state
        .store
        .inbox()
        .receive_from_channel(channel.org, new)
        .map_err(|error| store_failure("mensagem", &error))?;

    tracing::info!(
        target: "poph_server::evolution",
        instance = %channel.instance,
        status = if matches!(received.situation, Received::New) { "nova" } else { "duplicada" },
        attachment_count = received_attachment_count(&received),
        "mensagem do WhatsApp registrada"
    );
    Ok(Json(inbound_response(&received)))
}

fn matrix_module_enabled(
    state: &ServerState,
    channel: &EvolutionChannel,
) -> Result<bool, ApiError> {
    state
        .store
        .matrix()
        .state(channel.org)
        .map(|state| state.linked)
        .map_err(|error| store_failure("estado do módulo Matrix", &error))
}

fn typed_on_device(event: &InboundEvent) -> bool {
    event
        .metadata
        .get("from_me")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn webhook_integration(
    state: &ServerState,
    channel: &EvolutionChannel,
) -> Result<poph_store::ChannelIntegration, ApiError> {
    state
        .store
        .channel_integration(channel.org, channel.integration)
        .map_err(|error| store_failure("integração", &error))?
        .filter(|integration| integration.kind == "whatsapp" && integration.status != "archived")
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::NOT_FOUND,
                "not_found",
                "integração do WhatsApp não encontrada nesta org",
            )
        })
}

fn receive_from_device(
    state: &ServerState,
    channel: &EvolutionChannel,
    inbound: ReceivedEntry,
) -> Result<Json<Value>, ApiError> {
    let event = inbound.event;

    let is_echo = state
        .store
        .outbox()
        .sent_with_external_id(channel.org, &event.id_externo)
        .map_err(|error| store_failure("consulta da fila de saída", &error))?;
    if is_echo {
        tracing::debug!(
            target: "poph_server::evolution",
            instance = %channel.instance,
            "eco do nosso envio descartado"
        );
        return Ok(Json(json!({"recebido": true, "efeito": "eco"})));
    }

    let integration = webhook_integration(state, channel)?;

    if !inbound.attachments.is_empty() {
        tracing::warn!(
            target: "poph_server::evolution",
            instance = %channel.instance,
            "mensagem do aparelho com mídia: só o texto é gravado nesta versão"
        );
    }

    let received = state
        .store
        .inbox()
        .receive_from_device(
            channel.org,
            NewDeviceMessage {
                integration_id: channel.integration,
                agent_id: integration.agent_id,
                canal: "whatsapp".to_owned(),
                peer: event.peer,
                external_id: event.id_externo,
                texto: event.text,
            },
        )
        .map_err(|error| store_failure("mensagem do aparelho", &error))?;

    tracing::info!(
        target: "poph_server::evolution",
        instance = %channel.instance,
        status = if matches!(received.situation, Received::New) { "nova" } else { "duplicada" },
        paused = received.just_transferred,
        "mensagem digitada no aparelho registrada"
    );
    Ok(Json(device_response(&received)))
}

fn device_response(received: &ReceivedFromDevice) -> Value {
    json!({
        "recebido": true,
        "efeito": "aparelho",
        "situacao": if matches!(received.situation, Received::New) { "nova" } else { "duplicada" },
        "conversa_id": received.conversation.to_string(),
        "mensagem_id": received.mensagem.id.to_string(),
    })
}

fn received_attachment_count(received: &ReceivedWithMessage) -> usize {
    serde_json::from_str::<Vec<String>>(&received.mensagem.attachments_json)
        .map(|ids| ids.len())
        .unwrap_or(0)
}

fn inbound_response(received: &ReceivedWithMessage) -> Value {
    json!({
        "recebido": true,
        "situacao": if matches!(received.situation, Received::New) { "nova" } else { "duplicada" },
        "entrada_id": received.item.id.to_string(),
        "mensagem_id": received.mensagem.id.to_string(),
    })
}

async fn download_attachments(
    state: &ServerState,
    channel: &EvolutionChannel,
    pending: &[AttachmentToDownload],
) -> Result<Vec<NewChannelAttachment>, ApiError> {
    let config = FileConfig::default(state.config.blobs());
    let mut attachments = Vec::new();
    for attachment in pending {
        let Some(client) = channel.client.as_ref() else {
            tracing::warn!(target: "poph_server::evolution", "mídia sem cliente da Evolution; segue sem anexo");
            continue;
        };
        if attachment
            .declared_size
            .is_some_and(|declared| declared > channel.attachment_limit)
        {
            tracing::warn!(target: "poph_server::evolution", "mídia acima do teto; segue sem anexo");
            continue;
        }
        let downloaded = match client
            .download_attachment(
                &attachment.message_id,
                &attachment.data,
                channel.attachment_limit,
            )
            .await
        {
            Ok(downloaded) => downloaded,
            Err(error) => {
                tracing::warn!(
                    target: "poph_server::evolution",
                    reason = %download_error_category(&error),
                    "mídia do WhatsApp não baixada; a mensagem segue sem o anexo"
                );
                continue;
            }
        };

        let name = attachment.name.clone();
        let org = channel.org;
        let worker_config = config.clone();
        let stored = tokio::task::spawn_blocking(move || {
            poph_files::receive(&worker_config, org, &name, &downloaded.bytes)
        })
        .await
        .map_err(|_| ApiError::internal())?;

        let file = match stored {
            Ok(file) => file,
            Err(FileError::WriteOutcome(detail)) => {
                tracing::error!(target: "poph_server::evolution", reason = %detail, "blob da mídia não gravado");
                return Err(unavailable());
            }
            Err(error) => {
                tracing::warn!(
                    target: "poph_server::evolution",
                    reason = %file_error_category(&error),
                    "arquivo recebido recusado ou ilegível; a mensagem segue sem o anexo"
                );
                continue;
            }
        };

        let metadata = file.metadados;
        let blob_path = metadata
            .blob_path
            .strip_prefix(&config.blob_folder)
            .map(|relative| relative.to_string_lossy().replace('\\', "/"))
            .map_err(|_| ApiError::internal())?;
        let (extracted_text, extraction_status) = match file.content {
            Content::Image { .. } => (None, "skipped"),
            Content::Texto { envelopado, .. } => (Some(envelopado), "ok"),
            Content::NoText => (None, "failed"),
        };
        attachments.push(NewChannelAttachment {
            name: metadata.original_name,
            mime: metadata.mime_real,
            size_bytes: metadata.size_bytes,
            sha256: metadata.sha256,
            blob_path,
            extracted_text,
            extraction_status,
        });
    }
    Ok(attachments)
}

fn download_error_category(error: &EvolutionError) -> &'static str {
    match error {
        EvolutionError::AttachmentExpired(_) => "mídia expirada na Evolution",
        EvolutionError::AttachmentTooLarge { .. } => "mídia acima do teto",
        _ => "falha no download da mídia",
    }
}

fn file_error_category(error: &FileError) -> &'static str {
    match error {
        FileError::Vazio => "arquivo vazio",
        FileError::Large { .. } => "arquivo acima do limite",
        FileError::UnsupportedType(_) | FileError::MisleadingExtension { .. } => "tipo não aceito",
        FileError::Reading(_) | FileError::Time { .. } => "arquivo ilegível",
        _ => "falha na leitura do arquivo",
    }
}

fn store_failure(operation: &'static str, error: &poph_store::StoreError) -> ApiError {
    tracing::error!(
        target: "poph_server::evolution",
        operation = operation,
        reason = %error,
        "não consegui gravar a entrada do WhatsApp"
    );
    unavailable()
}

fn unavailable() -> ApiError {
    ApiError::new(
        StatusCode::SERVICE_UNAVAILABLE,
        "indisponivel",
        "não consegui gravar agora; a Evolution deve reenviar a entrega",
    )
}
