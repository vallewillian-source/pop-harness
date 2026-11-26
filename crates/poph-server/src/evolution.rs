//! Entrada do WhatsApp pela EvolutionAPI (HR-26b): a rota pública de webhook.
//!
//! `POST /webhook/evolution/{org_id}/{instancia}` fica fora do `/admin/v1`: não usa token de
//! administração. Quem chama é a Evolution, e a identidade dela é o segredo do webhook, um por
//! instância, no cabeçalho `X-Evolution-Webhook-Secret`.
//!
//! Respostas, pensadas para a Evolution (medida na integração): ela reenvia 5xx e timeout até 10
//! vezes em ~21 min, e não reenvia 4xx.
//!
//! - 200: recebido, já recebido antes (reentrega), evento sem efeito ou estado atualizado;
//! - 400: corpo que não é um objeto JSON;
//! - 401: segredo ausente ou errado (sem dizer qual dos dois);
//! - 404: instância não configurada, ou integração de mensagem que não existe ou está arquivada;
//! - 503: falha nossa ao gravar (banco ou blob). A Evolution reenvia depois.
//!
//! O corpo do evento traz o `apikey` da instância: ele nunca é gravado cru, logado nem devolvido.
//! Erros e logs saem só com ids internos e categorias, sem telefone, JID ou chave.

use axum::Json;
use axum::body::Bytes;
use axum::extract::rejection::BytesRejection;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use poph_evolution::webhook::{
    CABECALHO_SEGREDO, Decisao, EntradaRecebida, MidiaParaBaixar, Politica, conferir_segredo,
    interpretar,
};
use poph_evolution::{ClienteEvolution, ConfigEvolution, ErroEvolution, TETO_MIDIA};
use poph_files::{Content, FileConfig, FileError};
use poph_store::{
    NewChannelAttachment, NewChannelInboxItem, NewDeviceMessage, Received, ReceivedFromDevice,
    ReceivedWithMessage,
};
use poph_types::{HarnessConfig, InboundEvent, IntegrationId, OrgId, Secret, StateEvent};
use serde_json::{Value, json};

use crate::erro::ApiError;
use crate::estado::Estado;

/// Canal de WhatsApp de uma instância da Evolution, já resolvido na subida.
#[derive(Clone)]
pub struct CanalEvolution {
    /// Org dona da instância.
    pub org: OrgId,
    /// Nome da instância na Evolution (parte do caminho do webhook).
    pub instancia: String,
    /// Integração do agente que recebe as mensagens.
    pub integracao: IntegrationId,
    /// Segredo do webhook. `None` quando a variável não está no ambiente: a instância responde 401.
    pub segredo: Option<Secret>,
    /// Cliente para baixar mídia. `None` quando falta a chave ou o endereço: a mensagem segue sem
    /// o anexo, com o texto.
    pub cliente: Option<ClienteEvolution>,
    /// Teto de mídia em bytes.
    pub teto_midia: u64,
}

impl std::fmt::Debug for CanalEvolution {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CanalEvolution")
            .field("org", &self.org)
            .field("instancia", &self.instancia)
            .field("integracao", &self.integracao)
            .field("segredo", &self.segredo.is_some())
            .field("cliente", &self.cliente.is_some())
            .finish()
    }
}

/// Resolve as instâncias da configuração em canais prontos para a rota.
///
/// `chave_evolution` é a `EVOLUTION_API_KEY` (só para baixar mídia). `ler_variavel` lê as variáveis
/// de segredo pelo nome. O valor de nenhum segredo sai daqui para log.
pub fn canais_da_configuracao(
    config: &HarnessConfig,
    chave_evolution: Option<Secret>,
    ler_variavel: &dyn Fn(&str) -> Option<String>,
) -> Result<Vec<CanalEvolution>, String> {
    let Some(evolution) = config.evolution.as_ref() else {
        return Ok(Vec::new());
    };
    let mut canais = Vec::with_capacity(evolution.instances.len());
    for instancia in &evolution.instances {
        let org: OrgId = instancia
            .org_id
            .trim()
            .parse()
            .map_err(|_| format!("org inválida na instância `{}`", instancia.instance))?;
        let integracao: IntegrationId =
            instancia.integration_id.trim().parse().map_err(|_| {
                format!("integração inválida na instância `{}`", instancia.instance)
            })?;
        let nome = instancia.instance.trim().to_owned();

        let segredo = ler_variavel(&instancia.secret_env)
            .map(|valor| valor.trim().to_owned())
            .filter(|valor| !valor.is_empty())
            .map(Secret::new);
        if segredo.is_none() {
            tracing::warn!(
                target: "poph_server::evolution",
                instancia = %nome,
                variavel = %instancia.secret_env,
                "variável do segredo do webhook não definida: a instância responde 401 até ela existir"
            );
        }

        let cliente = match chave_evolution.clone() {
            None => {
                tracing::warn!(
                    target: "poph_server::evolution",
                    instancia = %nome,
                    "EVOLUTION_API_KEY ausente: a instância recebe texto, mas não baixa mídia"
                );
                None
            }
            Some(chave) => ConfigEvolution::novo(evolution.url_base.clone(), nome.clone(), chave)
                .and_then(ClienteEvolution::novo)
                .map_err(|erro| {
                    tracing::warn!(
                        target: "poph_server::evolution",
                        instancia = %nome,
                        motivo = %categoria_de_configuracao(&erro),
                        "cliente da Evolution não montado: a instância não baixa mídia"
                    );
                })
                .ok(),
        };

        canais.push(CanalEvolution {
            org,
            instancia: nome,
            integracao,
            segredo,
            cliente,
            teto_midia: TETO_MIDIA,
        });
    }
    Ok(canais)
}

fn categoria_de_configuracao(erro: &ErroEvolution) -> &'static str {
    match erro {
        ErroEvolution::Configuracao(_) => "configuração",
        _ => "erro interno do cliente",
    }
}

/// Recebe uma entrega da Evolution para uma instância.
pub async fn receber(
    State(estado): State<Estado>,
    Path((org_id, instancia)): Path<(String, String)>,
    cabecalhos: HeaderMap,
    corpo: Result<Bytes, BytesRejection>,
) -> Result<Json<Value>, ApiError> {
    let canal = estado
        .canal_evolution(&org_id, &instancia)
        .cloned()
        .ok_or_else(|| {
            ApiError::novo(
                StatusCode::NOT_FOUND,
                "not_found",
                "instância do WhatsApp não configurada neste servidor",
            )
        })?;

    // Autenticação antes de ler o corpo: entrega sem segredo certo não entra nem é processada.
    let recebido = cabecalhos
        .get(CABECALHO_SEGREDO)
        .and_then(|valor| valor.to_str().ok());
    let autorizado = canal
        .segredo
        .as_ref()
        .is_some_and(|segredo| conferir_segredo(segredo, recebido).is_ok());
    if !autorizado {
        return Err(ApiError::novo(
            StatusCode::UNAUTHORIZED,
            "webhook_nao_autorizado",
            "webhook recusado: segredo ausente ou inválido",
        ));
    }

    let corpo = corpo.map_err(|_| {
        ApiError::invalido("corpo do webhook inválido ou maior que o limite de 1 MB")
    })?;
    let valor: Value = serde_json::from_slice(&corpo)
        .map_err(|_| ApiError::invalido("corpo do webhook não é JSON"))?;
    if !valor.is_object() {
        return Err(ApiError::invalido(
            "corpo do webhook deve ser um objeto JSON",
        ));
    }

    // MX-41: com o módulo Matrix ligado nesta org, `fromMe` deixa de ser descartado na leitura —
    // quem decide é `receber_entrada`, que separa o eco do nosso envio da mensagem do aparelho.
    // Sem o módulo, nada muda: o descarte é aqui, antes de qualquer gravação.
    let politica = Politica {
        aceitar_de_mim: modulo_matrix_ligado(&estado, &canal)?,
    };
    let decisao = interpretar(canal.integracao, &valor, &politica);
    match decisao {
        Decisao::Ignorado(_) => Ok(Json(json!({"recebido": true, "efeito": "nenhum"}))),
        Decisao::Estado(evento) => receber_estado(&estado, &canal, &evento),
        Decisao::Entrada(entrada) => receber_entrada(&estado, &canal, entrada).await,
    }
}

/// Estado de conexão: atualiza a integração quando ela existe; sem integração, só registra.
fn receber_estado(
    estado: &Estado,
    canal: &CanalEvolution,
    evento: &StateEvent,
) -> Result<Json<Value>, ApiError> {
    let Some(texto) = texto_do_estado(evento) else {
        return Ok(Json(json!({"recebido": true, "efeito": "nenhum"})));
    };
    let atualizada = estado
        .store
        .set_integration_connection_state(canal.org, canal.integracao, texto)
        .map_err(|erro| falha_de_gravacao("estado da integração", &erro))?;
    if !atualizada {
        tracing::warn!(
            target: "poph_server::evolution",
            instancia = %canal.instancia,
            "estado recebido para uma integração que não existe nesta org; nada foi criado"
        );
        return Ok(Json(
            json!({"recebido": true, "efeito": "integracao_inexistente"}),
        ));
    }
    tracing::info!(
        target: "poph_server::evolution",
        org = %canal.org,
        instancia = %canal.instancia,
        estado = texto,
        "estado de conexão do WhatsApp atualizado"
    );
    Ok(Json(json!({"recebido": true, "efeito": "estado"})))
}

/// Texto gravado em `connection_state` para cada estado. Erro transitório não muda o estado.
fn texto_do_estado(evento: &StateEvent) -> Option<&'static str> {
    match evento {
        StateEvent::Connected { .. } => Some("connected"),
        StateEvent::Disconnected { .. } => Some("disconnected"),
        StateEvent::Qr { .. } => Some("awaiting_scan"),
        StateEvent::Error { .. } => None,
    }
}

/// Mensagem do cliente: confere se já foi recebida, baixa as mídias e grava tudo numa transação.
async fn receber_entrada(
    estado: &Estado,
    canal: &CanalEvolution,
    entrada: EntradaRecebida,
) -> Result<Json<Value>, ApiError> {
    // 0. Mensagem digitada no aparelho do cliente (MX-41): caminho próprio, decidido antes de
    //    qualquer gravação de entrada — ela não passa pela `inbox` (não há turno) nem pela
    //    `outbox` (já saiu pelo celular).
    if digitada_no_aparelho(&entrada.evento) {
        return receber_do_aparelho(estado, canal, entrada);
    }

    // 1. Reentrega: devolve o que já está gravado, sem baixar mídia de novo.
    let ja_recebida = estado
        .store
        .inbox()
        .delivery_received(canal.integracao, &entrada.evento.id_externo)
        .map_err(|erro| falha_de_gravacao("consulta da entrada", &erro))?;
    if let Some(recebido) = ja_recebida {
        return Ok(Json(resposta_da_entrada(&recebido)));
    }

    // 2. A mensagem precisa de um agente dono (a integração dele).
    let integracao = integracao_do_webhook(estado, canal)?;

    // 3. Mídias: as que falham viram aviso no log, e a mensagem segue com o texto.
    let anexos = baixar_anexos(estado, canal, &entrada.midias).await?;

    let evento = entrada.evento;
    let evento_json = serde_json::to_string(&evento).map_err(|_| ApiError::interno())?;
    // Nome do cliente (MX-32): já vem filtrado pelo webhook (contato individual recebido).
    let nome_cliente = evento
        .metadata
        .get("push_name")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned);
    let nova = NewChannelInboxItem {
        integration_id: canal.integracao,
        agent_id: integracao.agent_id,
        canal: "whatsapp".to_owned(),
        external_id: evento.id_externo,
        peer: evento.peer,
        event_json: evento_json,
        texto: evento.text,
        attachments: anexos,
        customer_name: nome_cliente,
    };
    let recebido = estado
        .store
        .inbox()
        .receive_from_channel(canal.org, nova)
        .map_err(|erro| falha_de_gravacao("mensagem", &erro))?;

    tracing::info!(
        target: "poph_server::evolution",
        instancia = %canal.instancia,
        situacao = if matches!(recebido.situation, Received::New) { "nova" } else { "duplicada" },
        anexos = recebido_anexos(&recebido),
        "mensagem do WhatsApp registrada"
    );
    Ok(Json(resposta_da_entrada(&recebido)))
}

/// O módulo Matrix está ligado nesta org? É a chave do `fromMe` na MX-41.
fn modulo_matrix_ligado(estado: &Estado, canal: &CanalEvolution) -> Result<bool, ApiError> {
    estado
        .store
        .matrix()
        .state(canal.org)
        .map(|estado| estado.linked)
        .map_err(|erro| falha_de_gravacao("estado do módulo Matrix", &erro))
}

/// `true` para a entrega que a Evolution marca como `fromMe` — foi digitada no aparelho do dono
/// da instância, não pelo cliente, na visão dela.
fn digitada_no_aparelho(evento: &InboundEvent) -> bool {
    evento
        .metadata
        .get("from_me")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

/// A integração do WhatsApp desta instância, com o agente dono. Vale para a mensagem do cliente e
/// para a digitada no aparelho: sem integração viva não há conversa para gravar.
fn integracao_do_webhook(
    estado: &Estado,
    canal: &CanalEvolution,
) -> Result<poph_store::ChannelIntegration, ApiError> {
    estado
        .store
        .channel_integration(canal.org, canal.integracao)
        .map_err(|erro| falha_de_gravacao("integração", &erro))?
        .filter(|integracao| integracao.kind == "whatsapp" && integracao.status != "archived")
        .ok_or_else(|| {
            ApiError::novo(
                StatusCode::NOT_FOUND,
                "not_found",
                "integração do WhatsApp não encontrada nesta org",
            )
        })
}

/// Mensagem digitada no aparelho do cliente (MX-41).
///
/// Duas decisões, nesta ordem:
///
/// 1. **Eco do nosso próprio envio é descartado.** O que o Harness manda pela `outbox` volta no
///    webhook como `fromMe`; a MX-40 guarda o id que a Evolution deu ao envio, e é ele que
///    identifica o eco. Descartar cedo evita mensagem duplicada e não cria nada.
/// 2. **O resto é mensagem do aparelho.** Entra gravada uma vez (idempotência pelo id externo),
///    como `out`/`device`, sem turno e sem fila de saída, e a conversa passa a `transferred` com a
///    auditoria `device_takeover`: o agente para de responder e quem está atendendo é o humano no
///    celular.
///
/// Mídia do aparelho não é baixada nesta tarefa (o caminho de arquivo das duas direções é a MX-43 e
/// a MX-44): o texto entra, o anexo fica para depois.
fn receber_do_aparelho(
    estado: &Estado,
    canal: &CanalEvolution,
    entrada: EntradaRecebida,
) -> Result<Json<Value>, ApiError> {
    let evento = entrada.evento;

    // 1. Eco: o id externo é de um envio nosso já confirmado.
    let eco = estado
        .store
        .outbox()
        .sent_with_external_id(canal.org, &evento.id_externo)
        .map_err(|erro| falha_de_gravacao("consulta da fila de saída", &erro))?;
    if eco {
        tracing::debug!(
            target: "poph_server::evolution",
            instancia = %canal.instancia,
            "eco do nosso envio descartado"
        );
        return Ok(Json(json!({"recebido": true, "efeito": "eco"})));
    }

    // 2. A mensagem precisa de um agente dono (a integração dele).
    let integracao = integracao_do_webhook(estado, canal)?;

    if !entrada.midias.is_empty() {
        tracing::warn!(
            target: "poph_server::evolution",
            instancia = %canal.instancia,
            "mensagem do aparelho com mídia: só o texto é gravado nesta versão"
        );
    }

    let recebido = estado
        .store
        .inbox()
        .receive_from_device(
            canal.org,
            NewDeviceMessage {
                integration_id: canal.integracao,
                agent_id: integracao.agent_id,
                canal: "whatsapp".to_owned(),
                peer: evento.peer,
                external_id: evento.id_externo,
                texto: evento.text,
            },
        )
        .map_err(|erro| falha_de_gravacao("mensagem do aparelho", &erro))?;

    tracing::info!(
        target: "poph_server::evolution",
        instancia = %canal.instancia,
        situacao = if matches!(recebido.situation, Received::New) { "nova" } else { "duplicada" },
        pausou = recebido.just_transferred,
        "mensagem digitada no aparelho registrada"
    );
    Ok(Json(resposta_do_aparelho(&recebido)))
}

fn resposta_do_aparelho(recebido: &ReceivedFromDevice) -> Value {
    json!({
        "recebido": true,
        "efeito": "aparelho",
        "situacao": if matches!(recebido.situation, Received::New) { "nova" } else { "duplicada" },
        "conversa_id": recebido.conversation.to_string(),
        "mensagem_id": recebido.mensagem.id.to_string(),
    })
}

fn recebido_anexos(recebido: &ReceivedWithMessage) -> usize {
    serde_json::from_str::<Vec<String>>(&recebido.mensagem.attachments_json)
        .map(|ids| ids.len())
        .unwrap_or(0)
}

fn resposta_da_entrada(recebido: &ReceivedWithMessage) -> Value {
    json!({
        "recebido": true,
        "situacao": if matches!(recebido.situation, Received::New) { "nova" } else { "duplicada" },
        "entrada_id": recebido.item.id.to_string(),
        "mensagem_id": recebido.mensagem.id.to_string(),
    })
}

/// Baixa e guarda as mídias. Falha de download ou de tipo não derruba a mensagem; falha de gravação
/// (banco ou blob) vira 503, para a Evolution reenviar depois.
async fn baixar_anexos(
    estado: &Estado,
    canal: &CanalEvolution,
    midias: &[MidiaParaBaixar],
) -> Result<Vec<NewChannelAttachment>, ApiError> {
    let config = FileConfig::default(estado.config.blobs());
    let mut anexos = Vec::new();
    for midia in midias {
        let Some(cliente) = canal.cliente.as_ref() else {
            tracing::warn!(target: "poph_server::evolution", "mídia sem cliente da Evolution; segue sem anexo");
            continue;
        };
        if midia
            .tamanho_declarado
            .is_some_and(|declarado| declarado > canal.teto_midia)
        {
            tracing::warn!(target: "poph_server::evolution", "mídia acima do teto; segue sem anexo");
            continue;
        }
        let baixada = match cliente
            .baixar_midia(&midia.message_id, &midia.dados, canal.teto_midia)
            .await
        {
            Ok(baixada) => baixada,
            Err(erro) => {
                tracing::warn!(
                    target: "poph_server::evolution",
                    motivo = %categoria_de_download(&erro),
                    "mídia do WhatsApp não baixada; a mensagem segue sem o anexo"
                );
                continue;
            }
        };

        let nome = midia.nome.clone();
        let org = canal.org;
        let config_tarefa = config.clone();
        let gravado = tokio::task::spawn_blocking(move || {
            poph_files::receive(&config_tarefa, org, &nome, &baixada.bytes)
        })
        .await
        .map_err(|_| ApiError::interno())?;

        let arquivo = match gravado {
            Ok(arquivo) => arquivo,
            Err(FileError::WriteOutcome(detalhe)) => {
                tracing::error!(target: "poph_server::evolution", motivo = %detalhe, "blob da mídia não gravado");
                return Err(indisponivel());
            }
            Err(erro) => {
                tracing::warn!(
                    target: "poph_server::evolution",
                    motivo = %categoria_de_arquivo(&erro),
                    "arquivo recebido recusado ou ilegível; a mensagem segue sem o anexo"
                );
                continue;
            }
        };

        let metadados = arquivo.metadados;
        let blob_path = metadados
            .blob_path
            .strip_prefix(&config.blob_folder)
            .map(|relativo| relativo.to_string_lossy().replace('\\', "/"))
            .map_err(|_| ApiError::interno())?;
        let (texto_extraido, status_extracao) = match arquivo.content {
            Content::Image { .. } => (None, "skipped"),
            Content::Texto { envelopado, .. } => (Some(envelopado), "ok"),
            Content::NoText => (None, "failed"),
        };
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
    Ok(anexos)
}

fn categoria_de_download(erro: &ErroEvolution) -> &'static str {
    match erro {
        ErroEvolution::MidiaExpirada(_) => "mídia expirada na Evolution",
        ErroEvolution::MidiaGrandeDemais { .. } => "mídia acima do teto",
        _ => "falha no download da mídia",
    }
}

fn categoria_de_arquivo(erro: &FileError) -> &'static str {
    match erro {
        FileError::Vazio => "arquivo vazio",
        FileError::Large { .. } => "arquivo acima do limite",
        FileError::UnsupportedType(_) | FileError::MisleadingExtension { .. } => "tipo não aceito",
        FileError::Reading(_) | FileError::Time { .. } => "arquivo ilegível",
        _ => "falha na leitura do arquivo",
    }
}

/// Falha nossa ao gravar: registra a categoria (sem o conteúdo) e responde 503.
fn falha_de_gravacao(onde: &'static str, erro: &poph_store::StoreError) -> ApiError {
    tracing::error!(
        target: "poph_server::evolution",
        etapa = onde,
        motivo = %erro,
        "não consegui gravar a entrada do WhatsApp"
    );
    indisponivel()
}

fn indisponivel() -> ApiError {
    ApiError::novo(
        StatusCode::SERVICE_UNAVAILABLE,
        "indisponivel",
        "não consegui gravar agora; a Evolution deve reenviar a entrega",
    )
}
