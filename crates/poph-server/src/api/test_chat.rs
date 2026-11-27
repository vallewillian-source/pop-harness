use std::convert::Infallible;
use std::time::Duration;

use axum::Json;
use axum::extract::multipart::Field;
use axum::extract::{Multipart, Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use futures::stream;
use poph_agent::turno::{TransportConsole, TurnEvent, TurnInput, TurnObserver, TurnSummary};
use poph_files::{Content, FileError};
use poph_store::{
    AttachmentRecord, Conversa, Direction, Mensagem, NewChannelAttachment, NewConversation,
    NewMessage, OutboxItem, Page, Role, Turno,
};
use poph_types::{AgentId, AttachmentId, ConversationId, MessageId, OrgId};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::mpsc;
use utoipa::ToSchema;

use super::{ListJson, PageQuery, if_match_version, next_cursor};
use crate::{
    auth::Identidade,
    erro::{ApiError, id_da_url as parse_path_id},
    estado::Estado,
};

pub const CHANNEL_OF_TEST: &str = "test";

const LIMIT_LISTING: u32 = 200;

const LIMIT_HISTORY: u32 = 200;

const LIMIT_TURN: u32 = 50;

const LIMIT_FILE_SENT: u32 = 200;

#[derive(Debug, Deserialize)]
pub struct AgentPath {
    org_id: String,
    agent_id: String,
}

#[derive(Debug, Deserialize)]
pub struct ConversationPath {
    org_id: String,
    agent_id: String,
    conversation_id: String,
}

enum StreamOutput {
    Event(TurnEvent),
    Summary(Box<TurnSummary>),
    Error(String),
}

struct SseObserver {
    tx: mpsc::UnboundedSender<StreamOutput>,
}

impl TurnObserver for SseObserver {
    fn evento(&mut self, event: TurnEvent) {
        if matches!(event, TurnEvent::Fim { .. }) {
            return;
        }
        let _ = self.tx.send(StreamOutput::Event(event));
    }
}

struct SseGuard {
    state: Estado,
    conversation: ConversationId,
    entry: Option<TurnInput>,
}

impl SseGuard {
    fn new(state: Estado, conversation: ConversationId) -> Self {
        Self {
            state,
            conversation,
            entry: None,
        }
    }
}

impl Drop for SseGuard {
    fn drop(&mut self) {
        if let Some(entry) = &self.entry {
            entry.cancellation.cancel();
        }
        self.state.liberar_turno_de_teste(self.conversation);
    }
}

fn conversation_json(c: &Conversa, messages: u64) -> Value {
    json!({
        "id": c.id.to_string(),
        "org_id": c.org_id.to_string(),
        "agent_id": c.agent_id.to_string(),
        "channel": c.canal,
        "status": c.status.as_text(),
        "summary": c.summary,
        "version": c.version,
        "message_count": messages,
        "created_at": c.created_at,
        "updated_at": c.updated_at,
        "last_message_at": c.last_message_at,
    })
}

fn attachment_json(a: &AttachmentRecord) -> Value {
    json!({
        "id": a.id.to_string(),
        "name": a.name,
        "mime": a.mime,
        "size_bytes": a.size_bytes,
        "sha256": a.sha256,
        "extraction_status": a.extraction_status,
        "has_text": a.extracted_text.as_ref().is_some_and(|t| !t.trim().is_empty()),
    })
}

fn message_json(m: &Mensagem, attachments: &[AttachmentRecord]) -> Value {
    json!({
        "id": m.id.to_string(),
        "conversation_id": m.conversation_id.to_string(),
        "direction": m.direction.as_text(),
        "role": m.role.as_text(),
        "text": m.texto,
        "attachments": attachments.iter().map(attachment_json).collect::<Vec<_>>(),
        "turn_id": m.turn_id.map(|v| v.to_string()),
        "created_at": m.created_at,
    })
}

fn turn_json(t: &Turno, file: &[Value]) -> Value {
    let (names, call) = super::tool_parts(&t.tools_json);
    json!({
        "id": t.id.to_string(),
        "state": t.state.as_text(),
        "jev": serde_json::from_str::<Value>(&t.jev_json).unwrap_or(Value::Null),
        "retrieved": serde_json::from_str::<Value>(&t.passages_json).unwrap_or(Value::Null),
        "tools": names,
        "tool_calls": call,
        "timings": serde_json::from_str::<Value>(&t.times_json).unwrap_or(Value::Null),
        "input_tokens": t.input_tokens,
        "output_tokens": t.output_tokens,
        "cost_micros": t.cost_micros,
        "error": t.error,
        "started_at": t.started_at,
        "finished_at": t.finished_at,
        "files_out": file,
    })
}

fn sent_file_json(item: &OutboxItem) -> Value {
    let payload: Value = serde_json::from_str(&item.content_json).unwrap_or(Value::Null);
    let field = |name: &str| payload.get(name).cloned().unwrap_or(Value::Null);
    json!({
        "id": item.id.to_string(),
        "turn_id": item.turn_id.to_string(),
        "seq": item.seq,
        "arquivo_id": field("arquivo_id"),
        "name": field("nome"),
        "mime": field("mime"),
        "size_bytes": field("tamanho_bytes"),
        "sha256": field("sha256"),
        "state": item.state.as_text(),
        "created_at": item.created_at,
    })
}

fn agent_and_conversation(
    state: &Estado,
    org: OrgId,
    agent_id: AgentId,
    conversation: ConversationId,
) -> Result<Conversa, ApiError> {
    state.store.agents().get(org, agent_id)?;
    let conversation = state.store.conversations().get(org, conversation)?;
    if conversation.agent_id != agent_id || conversation.canal != CHANNEL_OF_TEST {
        return Err(ApiError::nao_encontrado(
            "conversa de teste",
            &conversation.id.to_string(),
        ));
    }
    Ok(conversation)
}

#[utoipa::path(post, path="/admin/v1/orgs/{org_id}/agents/{agent_id}/test-chat", tag="test-chat", operation_id="teste_chat_criar", security(("bearer"=[])), params(("org_id"=String, Path), ("agent_id"=String, Path)), responses((status=201, description="Conversa de teste criada", body=Value), (status=404, description="Agente não encontrado", body=crate::erro::RespostaErro)))]
pub async fn create(
    State(state): State<Estado>,
    ident: Identidade,
    Path(c): Path<AgentPath>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let org = parse_path_id::<OrgId>(&c.org_id)?;
    ident.exigir_org(org)?;
    let agent_id = parse_path_id::<AgentId>(&c.agent_id)?;
    state.store.agents().get(org, agent_id)?;

    let conversation = state.store.conversations().create(
        org,
        NewConversation {
            agent_id,
            integration_id: None,
            canal: CHANNEL_OF_TEST.to_owned(),
            peer_externo: None,
            target_department: None,
        },
    )?;
    super::audit::register(
        &state,
        &ident,
        Some(org),
        "test_chat_open",
        "conversation",
        conversation.id.to_string(),
    )?;
    Ok((
        StatusCode::CREATED,
        Json(conversation_json(&conversation, 0)),
    ))
}

#[utoipa::path(get, path="/admin/v1/orgs/{org_id}/agents/{agent_id}/test-chat", tag="test-chat", operation_id="teste_chat_listar", security(("bearer"=[])), params(("org_id"=String, Path), ("agent_id"=String, Path), ("limit"=Option<u32>, Query), ("cursor"=Option<String>, Query)), responses((status=200, description="Conversas de teste do agente, mais recentes primeiro", body=ListJson<Value>)))]
pub async fn list(
    State(state): State<Estado>,
    ident: Identidade,
    Path(c): Path<AgentPath>,
    Query(q): Query<PageQuery>,
) -> Result<Json<ListJson<Value>>, ApiError> {
    let org = parse_path_id::<OrgId>(&c.org_id)?;
    ident.exigir_org(org)?;
    let agent_id = parse_path_id::<AgentId>(&c.agent_id)?;
    state.store.agents().get(org, agent_id)?;
    let page = q.page()?;
    let limit = page.limit.min(LIMIT_LISTING);
    let after = page
        .after
        .map(|id| id.to_string().parse::<ConversationId>())
        .transpose()
        .map_err(|_| ApiError::invalido("cursor não é um id de conversa"))?;
    let conversations = state.store.conversations().list_by_agent_channel(
        org,
        agent_id,
        CHANNEL_OF_TEST,
        after,
        limit,
    )?;
    let items: Vec<Value> = conversations
        .iter()
        .map(|conversation| {
            let count = state
                .store
                .messages()
                .count_rows(org, conversation.id)
                .unwrap_or(0);
            conversation_json(conversation, count)
        })
        .collect();
    let next_cursor = next_cursor(&items, limit, |v| {
        v["id"].as_str().unwrap_or_default().to_owned()
    });
    Ok(Json(ListJson { items, next_cursor }))
}

#[utoipa::path(get, path="/admin/v1/orgs/{org_id}/agents/{agent_id}/test-chat/{conversation_id}", tag="test-chat", operation_id="teste_chat_historico", security(("bearer"=[])), params(("org_id"=String, Path), ("agent_id"=String, Path), ("conversation_id"=String, Path)), responses((status=200, description="Conversa, mensagens e turnos (com o que o painel do porquê lê)", body=Value), (status=404, description="Não encontrado", body=crate::erro::RespostaErro)))]
pub async fn history(
    State(state): State<Estado>,
    ident: Identidade,
    Path(c): Path<ConversationPath>,
) -> Result<Json<Value>, ApiError> {
    let org = parse_path_id::<OrgId>(&c.org_id)?;
    ident.exigir_org(org)?;
    let agent_id = parse_path_id::<AgentId>(&c.agent_id)?;
    let conversation_id = parse_path_id::<ConversationId>(&c.conversation_id)?;
    let conversation = agent_and_conversation(&state, org, agent_id, conversation_id)?;

    let messages =
        state
            .store
            .messages()
            .list(org, conversation_id, Page::with_limit(LIMIT_HISTORY))?;
    let messages: Vec<Value> = messages
        .iter()
        .map(|message| {
            let attachments = if message.attachments_json.trim() == "[]" {
                Vec::new()
            } else {
                state
                    .store
                    .messages()
                    .attachments(org, message.id)
                    .unwrap_or_default()
            };
            message_json(message, &attachments)
        })
        .collect();
    let turns = state
        .store
        .turns()
        .list(org, conversation_id, Page::with_limit(LIMIT_TURN))?;
    let file =
        state
            .store
            .outbox()
            .list_files_of_conversation(org, conversation_id, LIMIT_FILE_SENT)?;
    let mut by_turn: std::collections::HashMap<String, Vec<Value>> =
        std::collections::HashMap::new();
    for item in &file {
        by_turn
            .entry(item.turn_id.to_string())
            .or_default()
            .push(sent_file_json(item));
    }
    let turns: Vec<Value> = turns
        .iter()
        .map(|turn| {
            let file = by_turn
                .get(&turn.id.to_string())
                .map(Vec::as_slice)
                .unwrap_or_default();
            turn_json(turn, file)
        })
        .collect();
    Ok(Json(json!({
        "conversation": conversation_json(&conversation, messages.len() as u64),
        "messages": messages,
        "turns": turns,
    })))
}

#[derive(Debug, Deserialize)]
pub struct AttachmentPath {
    org_id: String,
    agent_id: String,
    conversation_id: String,
    message_id: String,
    attachment_id: String,
}

#[doc = " Conteúdo de um anexo do histórico (HR-46b): os bytes do blob, para o Console baixar o arquivo"]
#[doc = " que o cliente mandou ou que o agente enviou no turno."]
#[doc = ""]
#[doc = " O escopo é o caminho inteiro — org, agente, conversa, mensagem e anexo: qualquer id de outra"]
#[doc = " org (ou de outra conversa/agente da mesma org) dá `404`, e o conteúdo nunca é servido fora do"]
#[doc = " escopo. Blob ausente no disco também dá `404` com a mesma mensagem genérica."]
#[utoipa::path(get, path="/admin/v1/orgs/{org_id}/agents/{agent_id}/test-chat/{conversation_id}/messages/{message_id}/attachments/{attachment_id}/content", tag="test-chat", operation_id="teste_chat_anexo_conteudo", security(("bearer"=[])), params(("org_id"=String, Path), ("agent_id"=String, Path), ("conversation_id"=String, Path), ("message_id"=String, Path), ("attachment_id"=String, Path)), responses((status=200, description="Conteúdo do anexo (bytes com o MIME gravado e o sha256 no cabeçalho `x-content-sha256`)", body=Vec<u8>, content_type="application/octet-stream"), (status=403, description="Org de outro token", body=crate::erro::RespostaErro), (status=404, description="Anexo ou conteúdo não encontrado", body=crate::erro::RespostaErro)))]
pub async fn attachment_content(
    State(state): State<Estado>,
    ident: Identidade,
    Path(c): Path<AttachmentPath>,
) -> Result<axum::response::Response, ApiError> {
    let org = parse_path_id::<OrgId>(&c.org_id)?;
    ident.exigir_org(org)?;
    let agent_id = parse_path_id::<AgentId>(&c.agent_id)?;
    let conversation = parse_path_id::<ConversationId>(&c.conversation_id)?;
    let message = parse_path_id::<MessageId>(&c.message_id)?;
    let attachment = parse_path_id::<AttachmentId>(&c.attachment_id)?;
    agent_and_conversation(&state, org, agent_id, conversation)?;
    state.store.messages().get(org, conversation, message)?;
    let attachments = state.store.messages().attachments(org, message)?;
    let attachment = attachments
        .into_iter()
        .find(|a| a.id == attachment)
        .ok_or_else(|| ApiError::nao_encontrado("anexo", &c.attachment_id))?;
    let folder = state.config.blobs();
    let sha256 = attachment.sha256.clone();
    let bytes =
        tokio::task::spawn_blocking(move || super::content::read_blob(&folder, org, &sha256))
            .await
            .map_err(|_| ApiError::interno())??;
    Ok(super::content::response(
        bytes,
        &attachment.mime,
        &attachment.name,
        &attachment.sha256,
    ))
}

#[utoipa::path(post, path="/admin/v1/orgs/{org_id}/agents/{agent_id}/test-chat/{conversation_id}/reopen", tag="test-chat", operation_id="teste_chat_reabrir", security(("bearer"=[])), params(("org_id"=String, Path), ("agent_id"=String, Path), ("conversation_id"=String, Path), ("If-Match"=String, Header)), responses((status=200, description="Conversa reaberta", body=Value), (status=412, description="Versão desatualizada", body=crate::erro::RespostaErro)))]
pub async fn reopen(
    State(state): State<Estado>,
    ident: Identidade,
    Path(c): Path<ConversationPath>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let org = parse_path_id::<OrgId>(&c.org_id)?;
    ident.exigir_org(org)?;
    let agent_id = parse_path_id::<AgentId>(&c.agent_id)?;
    let conversation_id = parse_path_id::<ConversationId>(&c.conversation_id)?;
    agent_and_conversation(&state, org, agent_id, conversation_id)?;
    let version = if_match_version(&headers, "conversa")?;
    let conversation = state.store.conversations().reopen_audited(
        org,
        conversation_id,
        version,
        ident.ator_id(),
    )?;
    let count = state
        .store
        .messages()
        .count_rows(org, conversation.id)
        .unwrap_or(0);
    Ok(Json(conversation_json(&conversation, count)))
}

#[utoipa::path(post, path="/admin/v1/orgs/{org_id}/agents/{agent_id}/test-chat/{conversation_id}/messages", tag="test-chat", operation_id="teste_chat_mensagem", security(("bearer"=[])), params(("org_id"=String, Path), ("agent_id"=String, Path), ("conversation_id"=String, Path)), request_body(content = inline(TestMessageData), content_type = "multipart/form-data"), responses((status=200, description="Turno em `text/event-stream`: `mensagem`, `inicio`, `texto`, `base`, `ferramenta`, `transferencia`, `fim` ou `erro`", body=String, content_type="text/event-stream"), (status=409, description="Já existe turno em voo nesta conversa", body=crate::erro::RespostaErro), (status=503, description="Turno indisponível (sem chave do modelo)", body=crate::erro::RespostaErro)))]
pub async fn send_message(
    State(state): State<Estado>,
    ident: Identidade,
    Path(c): Path<ConversationPath>,
    multipart: Result<Multipart, axum::extract::multipart::MultipartRejection>,
) -> Result<Response, ApiError> {
    let org = parse_path_id::<OrgId>(&c.org_id)?;
    ident.exigir_org(org)?;
    let agent_id = parse_path_id::<AgentId>(&c.agent_id)?;
    let conversation_id = parse_path_id::<ConversationId>(&c.conversation_id)?;
    agent_and_conversation(&state, org, agent_id, conversation_id)?;
    let engine = state.motor.clone().ok_or_else(|| {
        ApiError::novo(
            StatusCode::SERVICE_UNAVAILABLE,
            "turno_indisponivel",
            "o turno não está ligado neste servidor: defina a chave do modelo para conversar no \
             chat de teste",
        )
    })?;

    let multipart = multipart.map_err(|_| {
        ApiError::novo(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "corpo_invalido",
            "envie o corpo em `multipart/form-data` com o texto em `text` e os arquivos em `file`",
        )
    })?;

    if !state.reservar_turno_de_teste(conversation_id) {
        return Err(ApiError::novo(
            StatusCode::CONFLICT,
            "turno_em_andamento",
            "esta conversa já tem um turno em andamento: feche a outra aba ou espere a resposta",
        ));
    }
    let mut guard = SseGuard::new(state.clone(), conversation_id);

    let (text, file) = read_multipart(multipart).await?;
    if text.is_empty() && file.is_empty() {
        return Err(ApiError::validacao(
            "text",
            "envie um texto ou ao menos um arquivo",
        ));
    }
    let (attachments, received) = receive_files(&state, org, file).await?;

    let message = state.store.messages().write_with_attachments(
        org,
        NewMessage {
            conversation_id,
            direction: Direction::Inbound,
            role: Role::User,
            texto: text.clone(),
            attachments_json: "[]".to_owned(),
            external_id: None,
            turn_id: None,
            origin: poph_store::Origin::Agente,
            author_name: None,
        },
        &attachments,
    )?;
    state
        .store
        .conversations()
        .touch(org, conversation_id, state.store.now())?;
    super::audit::register(
        &state,
        &ident,
        Some(org),
        "test_chat_message",
        "conversation",
        conversation_id.to_string(),
    )?;

    let entry = TurnInput::new(org, conversation_id, message.id);
    guard.entry = Some(entry.clone());

    let (tx, rx) = mpsc::unbounded_channel::<StreamOutput>();
    let delivery = state.turno.clone();
    tokio::spawn(async move {
        let mut observer = SseObserver { tx: tx.clone() };
        let result = engine.run(entry, &mut observer).await;
        if let Some(delivery) = delivery {
            let _ = delivery
                .send_pending_console(org, &TransportConsole::silencioso())
                .await;
        }
        match result {
            Ok(summary) => {
                let _ = tx.send(StreamOutput::Summary(Box::new(summary)));
            }
            Err(error) => {
                tracing::warn!(error = %error, "o turno do chat de teste não pôde começar");
                let _ = tx.send(StreamOutput::Error(
                    "não consegui abrir o turno desta mensagem".to_owned(),
                ));
            }
        }
    });

    let first = message_event(conversation_id, message.id, &text, &received);
    let flow_of_event = stream::unfold(
        (rx, guard, Some(first), 0u64),
        |(mut rx, guard, initial, mut seq)| async move {
            let event = match initial {
                Some(first) => first,
                None => {
                    let output = rx.recv().await?;
                    match output {
                        StreamOutput::Event(event) => sse_event(event),
                        StreamOutput::Summary(summary) => end_event(&summary),
                        StreamOutput::Error(error) => Event::default()
                            .event("erro")
                            .json_data(json!({
                                "code": "turno_falhou",
                                "message": format!("o turno terminou sem resposta: {error}"),
                            }))
                            .unwrap_or_else(|_| Event::default().event("erro").data("{}")),
                    }
                }
            };
            seq += 1;
            Some((
                Ok::<_, Infallible>(event.id(seq.to_string())),
                (rx, guard, None, seq),
            ))
        },
    );
    Ok((
        [("cache-control", "no-cache"), ("x-accel-buffering", "no")],
        Sse::new(flow_of_event).keep_alive(
            KeepAlive::new()
                .interval(Duration::from_secs(10))
                .text("ping"),
        ),
    )
        .into_response())
}

#[doc = " Corpo documentado do `POST …/messages` (multipart). O campo `file` pode repetir."]
#[doc = ""]
#[doc = " A rota lê o multipart pelos nomes dos campos; o tipo existe para o contrato publicado."]
#[allow(dead_code)]
#[derive(Debug, ToSchema)]
#[schema(as = DadosMensagemTeste)]
pub struct TestMessageData {
    #[doc = " Texto do cliente. Opcional quando há arquivo."]
    pub text: Option<String>,
    #[doc = " Arquivo do cliente (imagem, PDF ou Office). Pode haver mais de um."]
    #[schema(value_type = String, format = Binary)]
    pub file: Option<String>,
}

async fn read_multipart(
    mut multipart: Multipart,
) -> Result<(String, Vec<(String, Vec<u8>)>), ApiError> {
    let mut text = String::new();
    let mut file: Vec<(String, Vec<u8>)> = Vec::new();
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|_| ApiError::invalido("corpo multipart inválido ou maior que o limite"))?
    {
        read_field(&mut text, &mut file, field).await?;
    }
    Ok((text.trim().to_owned(), file))
}

async fn read_field(
    text: &mut String,
    file: &mut Vec<(String, Vec<u8>)>,
    field: Field<'_>,
) -> Result<(), ApiError> {
    match field.name() {
        Some("text") => {
            *text = field
                .text()
                .await
                .map_err(|_| ApiError::invalido("o campo `text` não é texto"))?;
        }
        Some("file") => {
            let name = field
                .file_name()
                .map(str::to_owned)
                .unwrap_or_else(|| "arquivo".to_owned());
            let bytes = field
                .bytes()
                .await
                .map_err(|_| ApiError::invalido("não consegui ler o arquivo enviado"))?
                .to_vec();
            file.push((name, bytes));
        }
        _ => {}
    }
    Ok(())
}

async fn receive_files(
    state: &Estado,
    org: OrgId,
    file: Vec<(String, Vec<u8>)>,
) -> Result<(Vec<NewChannelAttachment>, Vec<Value>), ApiError> {
    if file.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }
    let config = std::sync::Arc::clone(&state.arquivos);
    let folder_blobs = state.config.blobs();
    let processed = tokio::task::spawn_blocking(move || {
        file.into_iter()
            .map(|(name, bytes)| {
                poph_files::receive(&config, org, &name, &bytes).map(|r| (r, name))
            })
            .collect::<Vec<Result<_, FileError>>>()
    })
    .await
    .map_err(|_| ApiError::interno())?;

    let mut attachments = Vec::with_capacity(processed.len());
    let mut summary = Vec::with_capacity(processed.len());
    for item in processed {
        let (received, submitted_name) = item.map_err(file_error)?;
        let metadata = received.metadados;
        let blob_path = metadata
            .blob_path
            .strip_prefix(&folder_blobs)
            .map(|relative| relative.to_string_lossy().replace('\\', "/"))
            .map_err(|_| ApiError::interno())?;
        let (extracted_text, extraction_status) = match received.content {
            Content::Image { .. } => (None, "skipped"),
            Content::Texto { envelopado, .. } => (Some(envelopado), "ok"),
            Content::NoText => (None, "failed"),
        };
        summary.push(json!({
            "name": metadata.original_name,
            "sent_as": submitted_name,
            "mime": metadata.mime_real,
            "size_bytes": metadata.size_bytes,
            "sha256": metadata.sha256,
            "extraction_status": extraction_status,
        }));
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
    Ok((attachments, summary))
}

fn file_error(error: FileError) -> ApiError {
    match error {
        FileError::Large {
            size_mb: upload_size_mb,
            limit_mb: upload_limit_mb,
        } => ApiError::validacao(
            "file",
            format!(
                "o arquivo tem {upload_size_mb:.1} MB e o limite por arquivo é {upload_limit_mb:.0} MB"
            ),
        ),
        FileError::Vazio => ApiError::validacao("file", "o arquivo está vazio"),
        FileError::UnsupportedType(reason) => {
            ApiError::validacao("file", format!("arquivo recusado: {reason}"))
        }
        FileError::MisleadingExtension { declared, real } => ApiError::validacao(
            "file",
            format!("arquivo recusado: o nome termina em .{declared}, mas o conteúdo é {real}"),
        ),
        FileError::Reading(reason) => {
            ApiError::validacao("file", format!("não consegui ler o arquivo: {reason}"))
        }
        FileError::Time {
            limit_seconds: time_limit_seconds,
        } => ApiError::validacao(
            "file",
            format!("a leitura do arquivo passou de {time_limit_seconds} s e foi interrompida"),
        ),
        FileError::Extrator(detail) => {
            tracing::error!(detail = %detail, "leitor de arquivos indisponível");
            ApiError::interno()
        }
        FileError::WriteOutcome(detail) => {
            tracing::error!(detail = %detail, "não consegui gravar o blob do anexo");
            ApiError::interno()
        }
        FileError::Configuracao(detail) => {
            tracing::error!(detail = %detail, "configuração de arquivos inválida");
            ApiError::interno()
        }
        other => ApiError::validacao("file", format!("arquivo recusado: {other}")),
    }
}

fn message_event(
    conversation: ConversationId,
    message: MessageId,
    text: &str,
    received: &[Value],
) -> Event {
    Event::default()
        .event("mensagem")
        .json_data(json!({
            "conversation_id": conversation.to_string(),
            "message_id": message.to_string(),
            "text": text,
            "attachments": received,
        }))
        .unwrap_or_else(|_| Event::default().event("mensagem").data("{}"))
}

fn sse_event(event: TurnEvent) -> Event {
    match event {
        TurnEvent::Started {
            turno: turn,
            agente: agent_id,
            conversa: conversation,
        } => Event::default()
            .event("inicio")
            .json_data(json!({
                "turn_id": turn.to_string(),
                "agent_id": agent_id.to_string(),
                "conversation_id": conversation.to_string(),
            }))
            .unwrap_or_else(|_| Event::default().event("inicio").data("{}")),
        TurnEvent::Texto(delta) => Event::default()
            .event("texto")
            .json_data(json!({ "delta": delta }))
            .unwrap_or_else(|_| Event::default().event("texto").data("{}")),
        TurnEvent::Base { sources } => Event::default()
            .event("base")
            .json_data(json!({ "sources": sources }))
            .unwrap_or_else(|_| Event::default().event("base").data("{}")),
        TurnEvent::Tool { nome: name, round } => Event::default()
            .event("ferramenta")
            .json_data(json!({ "name": name, "round": round }))
            .unwrap_or_else(|_| Event::default().event("ferramenta").data("{}")),
        TurnEvent::Handoff {
            departamento: department,
            risk_kind: risk_type,
            guidance,
        } => Event::default()
            .event("transferencia")
            .json_data(json!({
                "department_id": department.map(|d| d.to_string()),
                "risk_type": risk_type,
                "guidance": guidance,
            }))
            .unwrap_or_else(|_| Event::default().event("transferencia").data("{}")),
        TurnEvent::Fim {
            estado: state,
            motivo: reason,
        } => Event::default()
            .event("fim")
            .json_data(json!({ "state": state, "reason": reason }))
            .unwrap_or_else(|_| Event::default().event("fim").data("{}")),
    }
}

fn end_event(summary: &TurnSummary) -> Event {
    Event::default()
        .event("fim")
        .json_data(json!({
            "turn_id": summary.turno.to_string(),
            "state": summary.estado.as_text(),
            "reason": summary.motivo_fim,
            "text": summary.texto,
            "error": summary.error,
            "model_call_failed": summary.model_failure,
            "messages": summary.messages.iter().map(|m| m.to_string()).collect::<Vec<_>>(),
            "input_tokens": summary.input_tokens,
            "output_tokens": summary.output_tokens,
            "model": summary.modelo,
            "timings": {
                "queue_ms": summary.timings.queue_ms,
                "assembly_ms": summary.timings.assembly_ms,
                "first_text_ms": summary.timings.first_text_ms,
                "total_ms": summary.timings.total_ms,
                "tool_rounds": summary.timings.tool_rounds,
            },
        }))
        .unwrap_or_else(|_| Event::default().event("fim").data("{}"))
}
