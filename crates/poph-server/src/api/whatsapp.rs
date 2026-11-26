//! WhatsApp administrativo (HR-44b): pareamento com QR, estado ao vivo e desconexão.
//!
//! A aba Integrações do Console (HR-44) ficou sem estas três operações: a integração sabia dizer
//! que era `whatsapp`, mas não havia como parear o aparelho, ver se ele está conectado nem
//! desconectar. Aqui elas entram no contrato de **admin** — nada muda para o cliente final.
//!
//! Onde mora a configuração: numa instância da Evolution por integração (`[evolution].instancias`
//! na configuração, com `org_id`, `instancia` e `integracao_id`). O endereço vem de
//! `[evolution].url_base` e a chave de `EVOLUTION_API_KEY` no ambiente; as duas já eram usadas pelo
//! webhook (HR-26b). Quem não declarou a instância recebe `409 whatsapp_nao_configurado`, e a
//! mensagem diz o que falta — o servidor não inventa nome de instância.
//!
//! Regras que valem para as três rotas:
//!
//! - **A integração é do agente e da org do caminho**: outro agente/org recebe `404 not_found`,
//!   como nas demais rotas administrativas.
//! - **`If-Match` obrigatório nas duas escritas** (`qrcode` e `disconnect`): as duas agem na
//!   instância da Evolution, e o operador tem de estar vendo a configuração atual. Sem o
//!   cabeçalho `428`; versão velha `412 version_conflict`. Elas **não** sobem a versão: o estado de
//!   conexão é execução, não edição de configuração (mesma regra do webhook, que também grava
//!   `connection_state` sem tocar `version`).
//! - **Segredo nunca sai**: a chave da Evolution vai só no cabeçalho `apikey`, e o texto de erro do
//!   provedor já é limpo pelo cliente (`poph-evolution`). A resposta traz no máximo o nome da
//!   instância, o estado e o QR.
//! - **O QR não tem prazo**: a Evolution v2 devolve o QR e um contador (`count`), sem validade.
//!   A resposta devolve `expira_em: null` e o contador — quem desenha o QR pede outro quando o
//!   cliente demorar (o código gira a cada ~20 s).

use axum::{
    Json,
    extract::{Path, State, rejection::JsonRejection},
    http::{HeaderMap, StatusCode},
};
use poph_evolution::{ClienteEvolution, ErroEvolution};
use poph_store::AdminIntegration;
use poph_types::{AgentId, IntegrationId, OrgId};
use serde::Deserialize;
use serde_json::{Value, json};

use super::versao_do_if_match;
use crate::{
    auth::Identidade,
    erro::{ApiError, id_da_url},
    estado::Estado,
};

/// Caminho de uma integração dentro do agente.
#[derive(Deserialize)]
pub struct Caminho {
    org_id: String,
    agent_id: String,
    integration_id: String,
}

/// Corpo opcional do pareamento.
#[derive(Deserialize, utoipa::ToSchema, Default)]
#[serde(deny_unknown_fields)]
pub struct DadosPareamento {
    /// Número do WhatsApp que vai parear, só dígitos com DDI (`5511999998888`). Sem ele, o
    /// pareamento é pelo QR; com ele, a Evolution também devolve o código de 8 caracteres.
    number: Option<String>,
}

/// O que a rota precisa ter resolvido antes de falar com a Evolution.
struct Alvo {
    org: OrgId,
    id: IntegrationId,
    integracao: AdminIntegration,
    /// Nome da instância na Evolution (o mesmo do caminho do webhook).
    instancia: String,
    cliente: ClienteEvolution,
}

impl Alvo {
    fn id(&self) -> &str {
        &self.integracao.id
    }
}

/// Resolve a integração, confere que ela é WhatsApp e acha a instância configurada.
fn alvo(e: &Estado, ident: &Identidade, c: &Caminho) -> Result<Alvo, ApiError> {
    let org: OrgId = id_da_url(&c.org_id)?;
    ident.exigir_org(org)?;
    let agente: AgentId = id_da_url(&c.agent_id)?;
    let id: IntegrationId = id_da_url(&c.integration_id)?;
    let integracao = e.store.get_integration(org, agente, id)?;
    if !integracao.kind.trim().eq_ignore_ascii_case("whatsapp") {
        return Err(ApiError::novo(
            StatusCode::CONFLICT,
            "integracao_nao_e_whatsapp",
            format!(
                "esta integração é do tipo `{}`; só a de tipo `whatsapp` tem pareamento, estado e desconexão",
                integracao.kind
            ),
        )
        .com_detalhes(json!({ "kind": integracao.kind })));
    }
    let canal = e.canal_evolution_da_integracao(org, id).ok_or_else(|| {
        ApiError::novo(
            StatusCode::CONFLICT,
            "whatsapp_nao_configurado",
            "esta integração não tem instância da Evolution declarada na configuração \
                 do servidor (`[evolution].instancias`: org, instância e integração)",
        )
        .com_detalhes(json!({ "integration_id": id.to_string() }))
    })?;
    let cliente = canal.cliente.clone().ok_or_else(|| {
        ApiError::novo(
            StatusCode::SERVICE_UNAVAILABLE,
            "evolution_indisponivel",
            "o servidor não tem como falar com a Evolution: falta `EVOLUTION_API_KEY` no ambiente",
        )
    })?;
    Ok(Alvo {
        org,
        id,
        integracao,
        instancia: canal.instancia.clone(),
        cliente,
    })
}

/// Confere o `If-Match` contra a versão da integração (428 sem cabeçalho, 412 com versão velha).
fn conferir_versao(alvo: &Alvo, h: &HeaderMap) -> Result<(), ApiError> {
    let versao = versao_do_if_match(h, "integração")?;
    if versao != alvo.integracao.version {
        return Err(ApiError::conflito_de_versao(
            "integração",
            alvo.id(),
            versao,
            alvo.integracao.version,
        ));
    }
    Ok(())
}

/// Grava o estado de conexão na integração (sem subir a versão).
fn gravar_estado(e: &Estado, alvo: &Alvo, estado: &str) -> Result<(), ApiError> {
    e.store
        .set_integration_connection_state(alvo.org, alvo.id, estado)?;
    Ok(())
}

/// Erro da Evolution no formato da API. A mensagem do provedor já vem limpa (sem chave, sem
/// número e cortada) do cliente.
fn erro_evolution(erro: ErroEvolution, operacao: &str) -> ApiError {
    match &erro {
        ErroEvolution::Rede(_) => ApiError::novo(
            StatusCode::SERVICE_UNAVAILABLE,
            "evolution_indisponivel",
            format!("não consegui falar com a Evolution em `{operacao}`; tente de novo"),
        ),
        ErroEvolution::Resposta { status, mensagem } => ApiError::novo(
            StatusCode::BAD_GATEWAY,
            "evolution_recusou",
            format!("a Evolution recusou `{operacao}`: {mensagem}"),
        )
        .com_detalhes(json!({ "evolution_status": status })),
        ErroEvolution::RespostaInvalida(motivo) => ApiError::novo(
            StatusCode::BAD_GATEWAY,
            "evolution_resposta_invalida",
            format!("a resposta da Evolution não serve para `{operacao}`: {motivo}"),
        ),
        outro => ApiError::novo(
            StatusCode::BAD_GATEWAY,
            "evolution_falhou",
            format!("falha em `{operacao}` com a Evolution: {outro}"),
        ),
    }
}

/// A Evolution recusa a criação quando a instância já existe (403/409/422 com "already in use").
///
/// Nesse caso a instância é reaproveitada: quem clica em "gerar QR" de novo não pode receber erro
/// só porque a instância foi criada na primeira vez.
fn instancia_ja_existe(erro: &ErroEvolution) -> bool {
    match erro {
        ErroEvolution::Resposta { status, mensagem } => {
            let texto = mensagem.to_ascii_lowercase();
            (*status == 403 || *status == 409 || *status == 422)
                && ["already", "in use", "existe"]
                    .iter()
                    .any(|pista| texto.contains(pista))
        }
        _ => false,
    }
}

/// Estado da Evolution (`open`/`connecting`/`close`) no vocabulário da integração.
fn connection_state(bruto: &str) -> &'static str {
    match bruto {
        "open" => "connected",
        "connecting" => "awaiting_scan",
        "close" | "closed" => "disconnected",
        _ => "desconhecido",
    }
}

/// Lê o estado ao vivo, sem derrubar a rota quando a Evolution não responde.
async fn estado_opcional(cliente: &ClienteEvolution) -> Option<String> {
    cliente
        .estado_conexao()
        .await
        .ok()
        .map(|estado| estado.trim().to_ascii_lowercase())
}

#[utoipa::path(post,path="/admin/v1/orgs/{org_id}/agents/{agent_id}/integrations/{integration_id}/whatsapp/qrcode",tag="integrations",params(("org_id"=String,Path),("agent_id"=String,Path),("integration_id"=String,Path),("If-Match"=String,Header)),security(("bearer"=[])),request_body=DadosPareamento,responses((status=200,description="QR de pareamento",body=Value),(status=412,description="Conflito de versão",body=crate::erro::RespostaErro)))]
pub async fn qrcode(
    State(e): State<Estado>,
    ident: Identidade,
    Path(c): Path<Caminho>,
    h: HeaderMap,
    corpo: Result<Json<DadosPareamento>, JsonRejection>,
) -> Result<Json<Value>, ApiError> {
    let alvo = alvo(&e, &ident, &c)?;
    conferir_versao(&alvo, &h)?;
    let numero = corpo
        .ok()
        .and_then(|Json(dados)| dados.number)
        .map(|numero| numero.trim().to_owned())
        .filter(|numero| !numero.is_empty());

    // Instância já conectada: não há QR a mostrar, e pedir pareamento de novo só confunde.
    if estado_opcional(&alvo.cliente).await.as_deref() == Some("open") {
        gravar_estado(&e, &alvo, "connected")?;
        return Ok(Json(json!({
            "integration_id": alvo.id(),
            "instance": alvo.instancia,
            "state": "open",
            "connection_state": "connected",
            "qrcode": Value::Null,
            "pairing_code": Value::Null,
        })));
    }

    // Cria a instância quando ela ainda não existe; se já existe, segue para o pareamento.
    match alvo.cliente.criar_instancia().await {
        Ok(_) => {}
        Err(erro) if instancia_ja_existe(&erro) => {}
        Err(erro) => return Err(erro_evolution(erro, "criar instância")),
    }
    let pareamento = alvo
        .cliente
        .conectar(numero.as_deref())
        .await
        .map_err(|erro| erro_evolution(erro, "parear"))?;

    if pareamento.qr_base64.is_none()
        && pareamento.qr_texto.is_none()
        && pareamento.codigo_pareamento.is_none()
    {
        return Err(ApiError::novo(
            StatusCode::BAD_GATEWAY,
            "evolution_sem_qr",
            "a Evolution respondeu o pareamento sem QR e sem código; peça de novo em alguns segundos",
        ));
    }

    gravar_estado(&e, &alvo, "awaiting_scan")?;
    super::auditoria::registrar(
        &e,
        &ident,
        Some(alvo.org),
        "whatsapp_qrcode",
        "integration",
        alvo.id().to_owned(),
    )?;
    Ok(Json(json!({
        "integration_id": alvo.id(),
        "instance": alvo.instancia,
        "state": "connecting",
        "connection_state": "awaiting_scan",
        "qrcode": {
            "base64": pareamento.qr_base64,
            "code": pareamento.qr_texto,
            "count": pareamento.contador,
        },
        // A Evolution v2 não informa prazo: o QR gira a cada ~20 s e o `count` é o sinal.
        "expira_em": Value::Null,
        "pairing_code": pareamento.codigo_pareamento,
    })))
}

#[utoipa::path(get,path="/admin/v1/orgs/{org_id}/agents/{agent_id}/integrations/{integration_id}/whatsapp/state",tag="integrations",params(("org_id"=String,Path),("agent_id"=String,Path),("integration_id"=String,Path)),security(("bearer"=[])),responses((status=200,description="Estado da conexão",body=Value)))]
pub async fn estado(
    State(e): State<Estado>,
    ident: Identidade,
    Path(c): Path<Caminho>,
) -> Result<Json<Value>, ApiError> {
    let alvo = alvo(&e, &ident, &c)?;
    let bruto = alvo
        .cliente
        .estado_conexao()
        .await
        .map_err(|erro| erro_evolution(erro, "estado"))?
        .trim()
        .to_ascii_lowercase();
    let nosso = connection_state(&bruto);
    gravar_estado(&e, &alvo, nosso)?;
    Ok(Json(json!({
        "integration_id": alvo.id(),
        "instance": alvo.instancia,
        "state": bruto,
        "connection_state": nosso,
    })))
}

#[utoipa::path(post,path="/admin/v1/orgs/{org_id}/agents/{agent_id}/integrations/{integration_id}/whatsapp/disconnect",tag="integrations",params(("org_id"=String,Path),("agent_id"=String,Path),("integration_id"=String,Path),("If-Match"=String,Header)),security(("bearer"=[])),responses((status=200,description="Instância desconectada",body=Value),(status=412,description="Conflito de versão",body=crate::erro::RespostaErro)))]
pub async fn desconectar(
    State(e): State<Estado>,
    ident: Identidade,
    Path(c): Path<Caminho>,
    h: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let alvo = alvo(&e, &ident, &c)?;
    conferir_versao(&alvo, &h)?;
    alvo.cliente
        .desconectar()
        .await
        .map_err(|erro| erro_evolution(erro, "desconectar"))?;
    gravar_estado(&e, &alvo, "disconnected")?;
    super::auditoria::registrar(
        &e,
        &ident,
        Some(alvo.org),
        "whatsapp_disconnect",
        "integration",
        alvo.id().to_owned(),
    )?;
    Ok(Json(json!({
        "integration_id": alvo.id(),
        "instance": alvo.instancia,
        "connection_state": "disconnected",
        "disconnected": true,
    })))
}
