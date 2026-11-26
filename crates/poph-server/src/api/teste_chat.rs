//! Chat de teste do Console (HR-22): conversa nova, mensagem com anexo e resposta por SSE.
//!
//! O Console abre a conversa de teste com memória zerada, envia o texto e os arquivos do cliente e
//! recebe o turno **enquanto ele acontece**: `mensagem`, `inicio`, `texto` (vários, na ordem),
//! `base`, `ferramenta`, `transferencia`, `fim` — ou `erro`, quando o turno não pôde nem começar.
//!
//! Decisões desta rota:
//!
//! - **Canal `test`.** A conversa de teste é uma conversa comum com `channel = "test"` e sem
//!   integração: o turno já trata conversa sem integração como canal de console, e nada do
//!   WhatsApp é tocado. Memória zerada é consequência de ser conversa nova (`summary` nulo).
//! - **O mesmo motor da produção.** O turno roda no [`Motor`] que o `poph serve` montou; o
//!   provedor, o Jev, a base, os produtos e as ferramentas são os de verdade. Sem chave do modelo,
//!   a rota responde `503 turno_indisponivel` em vez de abrir um turno que nunca responderia.
//! - **Um turno por conversa.** A reserva é em memória, por instância: a segunda aba recebe
//!   `409 turno_em_andamento` antes de gravar qualquer coisa.
//! - **Desconexão cancela.** Quando o cliente fecha o `text/event-stream`, o cancelamento do turno
//!   é acionado; o turno termina como falho, nada vai para a `outbox` (não existe resposta parcial
//!   enviada) e a reserva é liberada.
//! - **Anexos são os da HR-23.** Passam pelo mesmo `poph_files::receber` (16 MB, tipo real,
//!   extração isolada). O texto extraído entra no prompt pelo caminho que o turno já usa, marcado
//!   como conteúdo não confiável.

use std::convert::Infallible;
use std::time::Duration;

use axum::Json;
use axum::extract::multipart::Field;
use axum::extract::{Multipart, Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use futures::stream;
use poph_agent::turno::{TransporteConsole, TurnEvent, TurnInput, TurnObserver, TurnSummary};
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

use super::{ConsultaPagina, ListaJson, proximo_cursor, versao_do_if_match};
use crate::{
    auth::Identidade,
    erro::{ApiError, id_da_url},
    estado::Estado,
};

/// Canal das conversas do chat de teste. `cli` é o `poph chat`; `console` fica livre para quando o
/// Console deixar de ser só de teste.
pub const CANAL_DE_TESTE: &str = "test";

/// Quantas conversas de teste a listagem devolve por vez.
const LIMITE_LISTAGEM: u32 = 200;

/// Quantas mensagens a leitura do histórico devolve.
const LIMITE_HISTORICO: u32 = 200;

/// Quantos turnos a leitura do histórico devolve.
const LIMITE_TURNOS: u32 = 50;

/// Quantos arquivos enviados pelo agente a leitura do histórico devolve por conversa (HR-46d).
const LIMITE_ARQUIVOS_ENVIADOS: u32 = 200;

#[derive(Debug, Deserialize)]
pub struct CaminhoAgente {
    org_id: String,
    agent_id: String,
}

#[derive(Debug, Deserialize)]
pub struct CaminhoConversa {
    org_id: String,
    agent_id: String,
    conversation_id: String,
}

/// O que o turno entrega ao canal do SSE.
enum Saida {
    Evento(TurnEvent),
    Resumo(Box<TurnSummary>),
    Falha(String),
}

/// Observador que manda cada evento do turno para o fluxo do SSE, na ordem em que o motor emite.
struct ObservadorSse {
    tx: mpsc::UnboundedSender<Saida>,
}

impl TurnObserver for ObservadorSse {
    fn evento(&mut self, evento: TurnEvent) {
        // O `Fim` do motor traz só estado e motivo; o `fim` do SSE é mais rico (turno, tokens,
        // tempos) e sai do resumo, logo depois. Mandar os dois duplicaria o evento na tela.
        if matches!(evento, TurnEvent::Fim { .. }) {
            return;
        }
        let _ = self.tx.send(Saida::Evento(evento));
    }
}

/// Guarda da reserva e do cancelamento do turno de teste.
///
/// Solta a conversa e cancela o turno quando o fluxo do SSE é derrubado — pelo fim normal ou
/// porque o cliente fechou a conexão. É o único caminho de liberação.
struct GuardaSse {
    estado: Estado,
    conversa: ConversationId,
    entrada: Option<TurnInput>,
}

impl GuardaSse {
    fn nova(estado: Estado, conversa: ConversationId) -> Self {
        Self {
            estado,
            conversa,
            entrada: None,
        }
    }
}

impl Drop for GuardaSse {
    fn drop(&mut self) {
        if let Some(entrada) = &self.entrada {
            entrada.cancellation.cancel();
        }
        self.estado.liberar_turno_de_teste(self.conversa);
    }
}

fn conversa_json(c: &Conversa, mensagens: u64) -> Value {
    json!({
        "id": c.id.to_string(),
        "org_id": c.org_id.to_string(),
        "agent_id": c.agent_id.to_string(),
        "channel": c.canal,
        "status": c.status.as_text(),
        "summary": c.summary,
        "version": c.version,
        "message_count": mensagens,
        "created_at": c.created_at,
        "updated_at": c.updated_at,
        "last_message_at": c.last_message_at,
    })
}

fn anexo_json(a: &AttachmentRecord) -> Value {
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

fn mensagem_json(m: &Mensagem, anexos: &[AttachmentRecord]) -> Value {
    json!({
        "id": m.id.to_string(),
        "conversation_id": m.conversation_id.to_string(),
        "direction": m.direction.as_text(),
        "role": m.role.as_text(),
        "text": m.texto,
        "attachments": anexos.iter().map(anexo_json).collect::<Vec<_>>(),
        "turn_id": m.turn_id.map(|v| v.to_string()),
        "created_at": m.created_at,
    })
}

fn turno_json(t: &Turno, arquivos: &[Value]) -> Value {
    // HR-47b: `tools` continua sendo só a lista de nomes (contrato antigo); `tool_calls` traz o
    // detalhe por chamada (executada/recusada, motivo, tempo, argumentos e resultado resumidos).
    let (nomes, chamadas) = super::partes_das_ferramentas(&t.tools_json);
    json!({
        "id": t.id.to_string(),
        "state": t.state.as_text(),
        "jev": serde_json::from_str::<Value>(&t.jev_json).unwrap_or(Value::Null),
        "retrieved": serde_json::from_str::<Value>(&t.passages_json).unwrap_or(Value::Null),
        "tools": nomes,
        "tool_calls": chamadas,
        "timings": serde_json::from_str::<Value>(&t.times_json).unwrap_or(Value::Null),
        "input_tokens": t.input_tokens,
        "output_tokens": t.output_tokens,
        "cost_micros": t.cost_micros,
        "error": t.error,
        "started_at": t.started_at,
        "finished_at": t.finished_at,
        // HR-46d: arquivos que o **agente** enfileirou para enviar neste turno.
        "files_out": arquivos,
    })
}

/// Um item de saída de arquivo (`outbox`, `kind = 'file'`) como o Console o lê (HR-46d).
///
/// Só metadados de apresentação: o `arquivo_id` é a chave da biblioteca, que é o que a rota
/// `/files/{id}/content` aceita. O `blob_ref` e o `conteudo_json` cru **nunca** saem daqui — o
/// caminho do blob é interno, e o Console o obtém só pelo endpoint de conteúdo, com autorização.
fn arquivo_enviado_json(item: &OutboxItem) -> Value {
    let payload: Value = serde_json::from_str(&item.content_json).unwrap_or(Value::Null);
    let campo = |nome: &str| payload.get(nome).cloned().unwrap_or(Value::Null);
    json!({
        "id": item.id.to_string(),
        "turn_id": item.turn_id.to_string(),
        "seq": item.seq,
        "arquivo_id": campo("arquivo_id"),
        "name": campo("nome"),
        "mime": campo("mime"),
        "size_bytes": campo("tamanho_bytes"),
        "sha256": campo("sha256"),
        "state": item.state.as_text(),
        "created_at": item.created_at,
    })
}

/// Agente da org e conversa de teste daquele agente, ou o erro certo.
fn agente_e_conversa(
    estado: &Estado,
    org: OrgId,
    agente: AgentId,
    conversa: ConversationId,
) -> Result<Conversa, ApiError> {
    estado.store.agents().get(org, agente)?;
    let conversa = estado.store.conversations().get(org, conversa)?;
    if conversa.agent_id != agente || conversa.canal != CANAL_DE_TESTE {
        return Err(ApiError::nao_encontrado(
            "conversa de teste",
            &conversa.id.to_string(),
        ));
    }
    Ok(conversa)
}

#[utoipa::path(post, path="/admin/v1/orgs/{org_id}/agents/{agent_id}/test-chat", tag="test-chat", operation_id="teste_chat_criar", security(("bearer"=[])), params(("org_id"=String, Path), ("agent_id"=String, Path)), responses((status=201, description="Conversa de teste criada", body=Value), (status=404, description="Agente não encontrado", body=crate::erro::RespostaErro)))]
pub async fn criar(
    State(estado): State<Estado>,
    ident: Identidade,
    Path(c): Path<CaminhoAgente>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let org = id_da_url::<OrgId>(&c.org_id)?;
    ident.exigir_org(org)?;
    let agente = id_da_url::<AgentId>(&c.agent_id)?;
    estado.store.agents().get(org, agente)?;

    let conversa = estado.store.conversations().create(
        org,
        NewConversation {
            agent_id: agente,
            integration_id: None,
            canal: CANAL_DE_TESTE.to_owned(),
            peer_externo: None,
            target_department: None,
        },
    )?;
    super::auditoria::registrar(
        &estado,
        &ident,
        Some(org),
        "test_chat_open",
        "conversation",
        conversa.id.to_string(),
    )?;
    Ok((StatusCode::CREATED, Json(conversa_json(&conversa, 0))))
}

#[utoipa::path(get, path="/admin/v1/orgs/{org_id}/agents/{agent_id}/test-chat", tag="test-chat", operation_id="teste_chat_listar", security(("bearer"=[])), params(("org_id"=String, Path), ("agent_id"=String, Path), ("limit"=Option<u32>, Query), ("cursor"=Option<String>, Query)), responses((status=200, description="Conversas de teste do agente, mais recentes primeiro", body=ListaJson<Value>)))]
pub async fn listar(
    State(estado): State<Estado>,
    ident: Identidade,
    Path(c): Path<CaminhoAgente>,
    Query(q): Query<ConsultaPagina>,
) -> Result<Json<ListaJson<Value>>, ApiError> {
    let org = id_da_url::<OrgId>(&c.org_id)?;
    ident.exigir_org(org)?;
    let agente = id_da_url::<AgentId>(&c.agent_id)?;
    estado.store.agents().get(org, agente)?;
    let pagina = q.pagina()?;
    let limite = pagina.limit.min(LIMITE_LISTAGEM);
    let apos = pagina
        .after
        .map(|id| id.to_string().parse::<ConversationId>())
        .transpose()
        .map_err(|_| ApiError::invalido("cursor não é um id de conversa"))?;
    let conversas = estado.store.conversations().list_by_agent_channel(
        org,
        agente,
        CANAL_DE_TESTE,
        apos,
        limite,
    )?;
    let items: Vec<Value> = conversas
        .iter()
        .map(|conversa| {
            let contagem = estado
                .store
                .messages()
                .count_rows(org, conversa.id)
                .unwrap_or(0);
            conversa_json(conversa, contagem)
        })
        .collect();
    let next_cursor = proximo_cursor(&items, limite, |v| {
        v["id"].as_str().unwrap_or_default().to_owned()
    });
    Ok(Json(ListaJson { items, next_cursor }))
}

#[utoipa::path(get, path="/admin/v1/orgs/{org_id}/agents/{agent_id}/test-chat/{conversation_id}", tag="test-chat", operation_id="teste_chat_historico", security(("bearer"=[])), params(("org_id"=String, Path), ("agent_id"=String, Path), ("conversation_id"=String, Path)), responses((status=200, description="Conversa, mensagens e turnos (com o que o painel do porquê lê)", body=Value), (status=404, description="Não encontrado", body=crate::erro::RespostaErro)))]
pub async fn historico(
    State(estado): State<Estado>,
    ident: Identidade,
    Path(c): Path<CaminhoConversa>,
) -> Result<Json<Value>, ApiError> {
    let org = id_da_url::<OrgId>(&c.org_id)?;
    ident.exigir_org(org)?;
    let agente = id_da_url::<AgentId>(&c.agent_id)?;
    let conversa_id = id_da_url::<ConversationId>(&c.conversation_id)?;
    let conversa = agente_e_conversa(&estado, org, agente, conversa_id)?;

    let mensagens =
        estado
            .store
            .messages()
            .list(org, conversa_id, Page::with_limit(LIMITE_HISTORICO))?;
    let messages: Vec<Value> = mensagens
        .iter()
        .map(|mensagem| {
            let anexos = if mensagem.attachments_json.trim() == "[]" {
                Vec::new()
            } else {
                estado
                    .store
                    .messages()
                    .attachments(org, mensagem.id)
                    .unwrap_or_default()
            };
            mensagem_json(mensagem, &anexos)
        })
        .collect();
    let turnos = estado
        .store
        .turns()
        .list(org, conversa_id, Page::with_limit(LIMITE_TURNOS))?;
    // HR-46d: arquivos que o agente mandou nesta conversa, agrupados por turno. A leitura é uma
    // consulta só na `outbox`; o turno que não mandou arquivo fica com a lista vazia.
    let arquivos = estado.store.outbox().list_files_of_conversation(
        org,
        conversa_id,
        LIMITE_ARQUIVOS_ENVIADOS,
    )?;
    let mut por_turno: std::collections::HashMap<String, Vec<Value>> =
        std::collections::HashMap::new();
    for item in &arquivos {
        por_turno
            .entry(item.turn_id.to_string())
            .or_default()
            .push(arquivo_enviado_json(item));
    }
    let turns: Vec<Value> = turnos
        .iter()
        .map(|turno| {
            let arquivos = por_turno
                .get(&turno.id.to_string())
                .map(Vec::as_slice)
                .unwrap_or_default();
            turno_json(turno, arquivos)
        })
        .collect();
    Ok(Json(json!({
        "conversation": conversa_json(&conversa, messages.len() as u64),
        "messages": messages,
        "turns": turns,
    })))
}

#[derive(Debug, Deserialize)]
pub struct CaminhoAnexo {
    org_id: String,
    agent_id: String,
    conversation_id: String,
    message_id: String,
    attachment_id: String,
}

/// Conteúdo de um anexo do histórico (HR-46b): os bytes do blob, para o Console baixar o arquivo
/// que o cliente mandou ou que o agente enviou no turno.
///
/// O escopo é o caminho inteiro — org, agente, conversa, mensagem e anexo: qualquer id de outra
/// org (ou de outra conversa/agente da mesma org) dá `404`, e o conteúdo nunca é servido fora do
/// escopo. Blob ausente no disco também dá `404` com a mesma mensagem genérica.
#[utoipa::path(get, path="/admin/v1/orgs/{org_id}/agents/{agent_id}/test-chat/{conversation_id}/messages/{message_id}/attachments/{attachment_id}/content", tag="test-chat", operation_id="teste_chat_anexo_conteudo", security(("bearer"=[])), params(("org_id"=String, Path), ("agent_id"=String, Path), ("conversation_id"=String, Path), ("message_id"=String, Path), ("attachment_id"=String, Path)), responses((status=200, description="Conteúdo do anexo (bytes com o MIME gravado e o sha256 no cabeçalho `x-content-sha256`)", body=Vec<u8>, content_type="application/octet-stream"), (status=403, description="Org de outro token", body=crate::erro::RespostaErro), (status=404, description="Anexo ou conteúdo não encontrado", body=crate::erro::RespostaErro)))]
pub async fn anexo_conteudo(
    State(estado): State<Estado>,
    ident: Identidade,
    Path(c): Path<CaminhoAnexo>,
) -> Result<axum::response::Response, ApiError> {
    let org = id_da_url::<OrgId>(&c.org_id)?;
    ident.exigir_org(org)?;
    let agente = id_da_url::<AgentId>(&c.agent_id)?;
    let conversa = id_da_url::<ConversationId>(&c.conversation_id)?;
    let mensagem = id_da_url::<MessageId>(&c.message_id)?;
    let anexo = id_da_url::<AttachmentId>(&c.attachment_id)?;
    agente_e_conversa(&estado, org, agente, conversa)?;
    // A mensagem precisa ser desta conversa (o repositório já filtra por org e conversa).
    estado.store.messages().get(org, conversa, mensagem)?;
    let anexos = estado.store.messages().attachments(org, mensagem)?;
    let anexo = anexos
        .into_iter()
        .find(|a| a.id == anexo)
        .ok_or_else(|| ApiError::nao_encontrado("anexo", &c.attachment_id))?;
    let pasta = estado.config.blobs();
    let sha256 = anexo.sha256.clone();
    let bytes =
        tokio::task::spawn_blocking(move || super::conteudo::ler_blob(&pasta, org, &sha256))
            .await
            .map_err(|_| ApiError::interno())??;
    Ok(super::conteudo::resposta(
        bytes,
        &anexo.mime,
        &anexo.name,
        &anexo.sha256,
    ))
}

#[utoipa::path(post, path="/admin/v1/orgs/{org_id}/agents/{agent_id}/test-chat/{conversation_id}/reopen", tag="test-chat", operation_id="teste_chat_reabrir", security(("bearer"=[])), params(("org_id"=String, Path), ("agent_id"=String, Path), ("conversation_id"=String, Path), ("If-Match"=String, Header)), responses((status=200, description="Conversa reaberta", body=Value), (status=412, description="Versão desatualizada", body=crate::erro::RespostaErro)))]
pub async fn reabrir(
    State(estado): State<Estado>,
    ident: Identidade,
    Path(c): Path<CaminhoConversa>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let org = id_da_url::<OrgId>(&c.org_id)?;
    ident.exigir_org(org)?;
    let agente = id_da_url::<AgentId>(&c.agent_id)?;
    let conversa_id = id_da_url::<ConversationId>(&c.conversation_id)?;
    agente_e_conversa(&estado, org, agente, conversa_id)?;
    let versao = versao_do_if_match(&headers, "conversa")?;
    let conversa =
        estado
            .store
            .conversations()
            .reopen_audited(org, conversa_id, versao, ident.ator_id())?;
    let contagem = estado
        .store
        .messages()
        .count_rows(org, conversa.id)
        .unwrap_or(0);
    Ok(Json(conversa_json(&conversa, contagem)))
}

#[utoipa::path(post, path="/admin/v1/orgs/{org_id}/agents/{agent_id}/test-chat/{conversation_id}/messages", tag="test-chat", operation_id="teste_chat_mensagem", security(("bearer"=[])), params(("org_id"=String, Path), ("agent_id"=String, Path), ("conversation_id"=String, Path)), request_body(content = inline(DadosMensagemTeste), content_type = "multipart/form-data"), responses((status=200, description="Turno em `text/event-stream`: `mensagem`, `inicio`, `texto`, `base`, `ferramenta`, `transferencia`, `fim` ou `erro`", body=String, content_type="text/event-stream"), (status=409, description="Já existe turno em voo nesta conversa", body=crate::erro::RespostaErro), (status=503, description="Turno indisponível (sem chave do modelo)", body=crate::erro::RespostaErro)))]
pub async fn mensagem(
    State(estado): State<Estado>,
    ident: Identidade,
    Path(c): Path<CaminhoConversa>,
    multipart: Result<Multipart, axum::extract::multipart::MultipartRejection>,
) -> Result<Response, ApiError> {
    let org = id_da_url::<OrgId>(&c.org_id)?;
    ident.exigir_org(org)?;
    let agente = id_da_url::<AgentId>(&c.agent_id)?;
    let conversa_id = id_da_url::<ConversationId>(&c.conversation_id)?;
    agente_e_conversa(&estado, org, agente, conversa_id)?;
    let motor = estado.motor.clone().ok_or_else(|| {
        ApiError::novo(
            StatusCode::SERVICE_UNAVAILABLE,
            "turno_indisponivel",
            "o turno não está ligado neste servidor: defina a chave do modelo para conversar no \
             chat de teste",
        )
    })?;

    // O conteúdo do corpo só é lido depois do escopo: token de outra org recebe 403 mesmo com
    // corpo no formato errado.
    let multipart = multipart.map_err(|_| {
        ApiError::novo(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "corpo_invalido",
            "envie o corpo em `multipart/form-data` com o texto em `text` e os arquivos em `file`",
        )
    })?;

    // A reserva vem antes da mensagem: a segunda aba é barrada sem gravar nada, e o texto dela não
    // entra no turno que já está em voo. Qualquer erro abaixo solta a reserva pelo `Drop`.
    if !estado.reservar_turno_de_teste(conversa_id) {
        return Err(ApiError::novo(
            StatusCode::CONFLICT,
            "turno_em_andamento",
            "esta conversa já tem um turno em andamento: feche a outra aba ou espere a resposta",
        ));
    }
    let mut guarda = GuardaSse::nova(estado.clone(), conversa_id);

    // Multipart: `text` (opcional) e `file` (zero ou mais). O corpo é limitado pela rota (24 MB);
    // cada arquivo ainda passa pelo limite de 16 MB do recebimento.
    let (texto, arquivos) = ler_multipart(multipart).await?;
    if texto.is_empty() && arquivos.is_empty() {
        return Err(ApiError::validacao(
            "text",
            "envie um texto ou ao menos um arquivo",
        ));
    }
    let (anexos, recebidos) = receber_arquivos(&estado, org, arquivos).await?;

    let mensagem = estado.store.messages().write_with_attachments(
        org,
        NewMessage {
            conversation_id: conversa_id,
            direction: Direction::Inbound,
            role: Role::User,
            texto: texto.clone(),
            attachments_json: "[]".to_owned(),
            external_id: None,
            turn_id: None,
            origin: poph_store::Origin::Agente,
            author_name: None,
        },
        &anexos,
    )?;
    estado
        .store
        .conversations()
        .touch(org, conversa_id, estado.store.now())?;
    super::auditoria::registrar(
        &estado,
        &ident,
        Some(org),
        "test_chat_message",
        "conversation",
        conversa_id.to_string(),
    )?;

    let entrada = TurnInput::new(org, conversa_id, mensagem.id);
    guarda.entrada = Some(entrada.clone());

    let (tx, rx) = mpsc::unbounded_channel::<Saida>();
    let fluxo = estado.turno.clone();
    tokio::spawn(async move {
        let mut observador = ObservadorSse { tx: tx.clone() };
        let resultado = motor.run(entrada, &mut observador).await;
        // O texto já saiu pelo SSE: confirma a saída sem imprimir de novo, **antes** do `fim`, para
        // quem receber o `fim` poder contar com a fila confirmada. Sem o fluxo (montagem de teste
        // que só passou o motor), o item fica pendente na fila e nada se perde.
        if let Some(fluxo) = fluxo {
            let _ = fluxo
                .enviar_pendentes_do_console(org, &TransporteConsole::silencioso())
                .await;
        }
        match resultado {
            Ok(resumo) => {
                let _ = tx.send(Saida::Resumo(Box::new(resumo)));
            }
            Err(erro) => {
                // O detalhe fica no log do servidor; a resposta leva só o que o operador precisa.
                tracing::warn!(erro = %erro, "o turno do chat de teste não pôde começar");
                let _ = tx.send(Saida::Falha(
                    "não consegui abrir o turno desta mensagem".to_owned(),
                ));
            }
        }
    });

    // O primeiro evento confirma o que foi gravado (mensagem + anexos) antes de o turno abrir.
    let primeiro = evento_de_mensagem(conversa_id, mensagem.id, &texto, &recebidos);
    let fluxo_de_eventos = stream::unfold(
        (rx, guarda, Some(primeiro), 0u64),
        |(mut rx, guarda, inicial, mut seq)| async move {
            let evento = match inicial {
                Some(primeiro) => primeiro,
                None => {
                    let saida = rx.recv().await?;
                    match saida {
                        Saida::Evento(evento) => evento_sse(evento),
                        Saida::Resumo(resumo) => evento_fim(&resumo),
                        Saida::Falha(erro) => Event::default()
                            .event("erro")
                            .json_data(json!({
                                "code": "turno_falhou",
                                "message": format!("o turno terminou sem resposta: {erro}"),
                            }))
                            .unwrap_or_else(|_| Event::default().event("erro").data("{}")),
                    }
                }
            };
            seq += 1;
            Some((
                Ok::<_, Infallible>(evento.id(seq.to_string())),
                (rx, guarda, None, seq),
            ))
        },
    );
    Ok((
        [
            ("cache-control", "no-cache"),
            // Proxy reverso que respeita o cabeçalho não segura o fluxo (HR-50 vai medir isso).
            ("x-accel-buffering", "no"),
        ],
        Sse::new(fluxo_de_eventos).keep_alive(
            KeepAlive::new()
                .interval(Duration::from_secs(10))
                .text("ping"),
        ),
    )
        .into_response())
}

/// Corpo documentado do `POST …/messages` (multipart). O campo `file` pode repetir.
///
/// A rota lê o multipart pelos nomes dos campos; o tipo existe para o contrato publicado.
#[allow(dead_code)]
#[derive(Debug, ToSchema)]
pub struct DadosMensagemTeste {
    /// Texto do cliente. Opcional quando há arquivo.
    pub text: Option<String>,
    /// Arquivo do cliente (imagem, PDF ou Office). Pode haver mais de um.
    #[schema(value_type = String, format = Binary)]
    pub file: Option<String>,
}

/// Lê `text` e os `file` do multipart, com o nome original de cada arquivo.
async fn ler_multipart(
    mut multipart: Multipart,
) -> Result<(String, Vec<(String, Vec<u8>)>), ApiError> {
    let mut texto = String::new();
    let mut arquivos: Vec<(String, Vec<u8>)> = Vec::new();
    while let Some(campo) = multipart
        .next_field()
        .await
        .map_err(|_| ApiError::invalido("corpo multipart inválido ou maior que o limite"))?
    {
        ler_campo(&mut texto, &mut arquivos, campo).await?;
    }
    Ok((texto.trim().to_owned(), arquivos))
}

async fn ler_campo(
    texto: &mut String,
    arquivos: &mut Vec<(String, Vec<u8>)>,
    campo: Field<'_>,
) -> Result<(), ApiError> {
    match campo.name() {
        Some("text") => {
            *texto = campo
                .text()
                .await
                .map_err(|_| ApiError::invalido("o campo `text` não é texto"))?;
        }
        Some("file") => {
            let nome = campo
                .file_name()
                .map(str::to_owned)
                .unwrap_or_else(|| "arquivo".to_owned());
            let bytes = campo
                .bytes()
                .await
                .map_err(|_| ApiError::invalido("não consegui ler o arquivo enviado"))?
                .to_vec();
            arquivos.push((nome, bytes));
        }
        _ => {}
    }
    Ok(())
}

/// Recebe os arquivos pelo mesmo caminho da HR-23 e devolve o que vai para a mensagem.
async fn receber_arquivos(
    estado: &Estado,
    org: OrgId,
    arquivos: Vec<(String, Vec<u8>)>,
) -> Result<(Vec<NewChannelAttachment>, Vec<Value>), ApiError> {
    if arquivos.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }
    let config = std::sync::Arc::clone(&estado.arquivos);
    let pasta_blobs = estado.config.blobs();
    let processados = tokio::task::spawn_blocking(move || {
        arquivos
            .into_iter()
            .map(|(nome, bytes)| {
                poph_files::receive(&config, org, &nome, &bytes).map(|r| (r, nome))
            })
            .collect::<Vec<Result<_, FileError>>>()
    })
    .await
    .map_err(|_| ApiError::interno())?;

    let mut anexos = Vec::with_capacity(processados.len());
    let mut resumo = Vec::with_capacity(processados.len());
    for item in processados {
        let (recebido, nome_enviado) = item.map_err(erro_do_arquivo)?;
        let metadados = recebido.metadados;
        let blob_path = metadados
            .blob_path
            .strip_prefix(&pasta_blobs)
            .map(|relativo| relativo.to_string_lossy().replace('\\', "/"))
            .map_err(|_| ApiError::interno())?;
        let (texto_extraido, status_extracao) = match recebido.content {
            Content::Image { .. } => (None, "skipped"),
            Content::Texto { envelopado, .. } => (Some(envelopado), "ok"),
            Content::NoText => (None, "failed"),
        };
        resumo.push(json!({
            "name": metadados.original_name,
            "sent_as": nome_enviado,
            "mime": metadados.mime_real,
            "size_bytes": metadados.size_bytes,
            "sha256": metadados.sha256,
            "extraction_status": status_extracao,
        }));
        anexos.push(NewChannelAttachment {
            name: metadados.original_name,
            mime: metadados.mime_real,
            size_bytes: metadados.size_bytes,
            sha256: metadados.sha256,
            blob_path,
            extracted_text: texto_extraido,
            extraction_status: status_extracao,
        });
    }
    Ok((anexos, resumo))
}

/// Erro do arquivo para o operador: recusa de conteúdo é `400` com `details.field = "file"`;
/// falha do servidor (gravação, configuração, leitor ausente) é `500` sem caminho interno.
fn erro_do_arquivo(erro: FileError) -> ApiError {
    match erro {
        FileError::Large {
            size_mb: tamanho_mb,
            limit_mb: limite_mb,
        } => ApiError::validacao(
            "file",
            format!("o arquivo tem {tamanho_mb:.1} MB e o limite por arquivo é {limite_mb:.0} MB"),
        ),
        FileError::Vazio => ApiError::validacao("file", "o arquivo está vazio"),
        FileError::UnsupportedType(motivo) => {
            ApiError::validacao("file", format!("arquivo recusado: {motivo}"))
        }
        FileError::MisleadingExtension {
            declared: declarada,
            real,
        } => ApiError::validacao(
            "file",
            format!("arquivo recusado: o nome termina em .{declarada}, mas o conteúdo é {real}"),
        ),
        FileError::Reading(motivo) => {
            ApiError::validacao("file", format!("não consegui ler o arquivo: {motivo}"))
        }
        FileError::Time {
            limit_seconds: limite_segundos,
        } => ApiError::validacao(
            "file",
            format!("a leitura do arquivo passou de {limite_segundos} s e foi interrompida"),
        ),
        FileError::Extrator(detalhe) => {
            tracing::error!(detalhe = %detalhe, "leitor de arquivos indisponível");
            ApiError::interno()
        }
        FileError::WriteOutcome(detalhe) => {
            tracing::error!(detalhe = %detalhe, "não consegui gravar o blob do anexo");
            ApiError::interno()
        }
        FileError::Configuracao(detalhe) => {
            tracing::error!(detalhe = %detalhe, "configuração de arquivos inválida");
            ApiError::interno()
        }
        // `ErroArquivo` é `non_exhaustive`: variante nova cai aqui como recusa, nunca como 500 mudo.
        outro => ApiError::validacao("file", format!("arquivo recusado: {outro}")),
    }
}

fn evento_de_mensagem(
    conversa: ConversationId,
    mensagem: MessageId,
    texto: &str,
    recebidos: &[Value],
) -> Event {
    Event::default()
        .event("mensagem")
        .json_data(json!({
            "conversation_id": conversa.to_string(),
            "message_id": mensagem.to_string(),
            "text": texto,
            "attachments": recebidos,
        }))
        .unwrap_or_else(|_| Event::default().event("mensagem").data("{}"))
}

fn evento_sse(evento: TurnEvent) -> Event {
    match evento {
        TurnEvent::Started {
            turno,
            agente,
            conversa,
        } => Event::default()
            .event("inicio")
            .json_data(json!({
                "turn_id": turno.to_string(),
                "agent_id": agente.to_string(),
                "conversation_id": conversa.to_string(),
            }))
            .unwrap_or_else(|_| Event::default().event("inicio").data("{}")),
        TurnEvent::Texto(delta) => Event::default()
            .event("texto")
            .json_data(json!({ "delta": delta }))
            .unwrap_or_else(|_| Event::default().event("texto").data("{}")),
        TurnEvent::Base { sources: fontes } => Event::default()
            .event("base")
            .json_data(json!({ "sources": fontes }))
            .unwrap_or_else(|_| Event::default().event("base").data("{}")),
        TurnEvent::Tool {
            nome,
            round: rodada,
        } => Event::default()
            .event("ferramenta")
            .json_data(json!({ "name": nome, "round": rodada }))
            .unwrap_or_else(|_| Event::default().event("ferramenta").data("{}")),
        TurnEvent::Handoff {
            departamento,
            risk_kind: tipo_risco,
            guidance: orientacao,
        } => Event::default()
            .event("transferencia")
            .json_data(json!({
                "department_id": departamento.map(|d| d.to_string()),
                "risk_type": tipo_risco,
                "guidance": orientacao,
            }))
            .unwrap_or_else(|_| Event::default().event("transferencia").data("{}")),
        // O `Fim` do motor não é repassado: o `fim` rico sai do resumo (ver `evento_fim`).
        TurnEvent::Fim { estado, motivo } => Event::default()
            .event("fim")
            .json_data(json!({ "state": estado, "reason": motivo }))
            .unwrap_or_else(|_| Event::default().event("fim").data("{}")),
    }
}

fn evento_fim(resumo: &TurnSummary) -> Event {
    Event::default()
        .event("fim")
        .json_data(json!({
            "turn_id": resumo.turno.to_string(),
            "state": resumo.estado.as_text(),
            "reason": resumo.motivo_fim,
            "text": resumo.texto,
            "error": resumo.error,
            "model_call_failed": resumo.model_failure,
            "messages": resumo.messages.iter().map(|m| m.to_string()).collect::<Vec<_>>(),
            "input_tokens": resumo.input_tokens,
            "output_tokens": resumo.output_tokens,
            "model": resumo.modelo,
            "timings": {
                "queue_ms": resumo.timings.queue_ms,
                "assembly_ms": resumo.timings.assembly_ms,
                "first_text_ms": resumo.timings.first_text_ms,
                "total_ms": resumo.timings.total_ms,
                "tool_rounds": resumo.timings.tool_rounds,
            },
        }))
        .unwrap_or_else(|_| Event::default().event("fim").data("{}"))
}
