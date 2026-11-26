//! API administrativa (`/admin/v1`, PRD §8.1) e rotas de operação.

/// Permissão de fluxos por agente (FX-28, PRD §6.2).
mod agente_fluxos;
mod agentes;
mod analytics;
mod auditoria;
mod backup;
mod biblioteca;
mod categorias;
mod cluster;
mod conhecimento;
mod conteudo;
mod conversas;
mod conversation_analytics;
mod departamentos;
/// Leitura das Fichas (FX-29): lista com filtros, detalhe com execuções e linha do tempo.
mod fichas;
/// API dos fluxos (FX-27): validar, rascunho, publicar, revisões, diff, preview e formatar.
mod fluxos;
mod instrucoes;
mod integracoes;
mod manifestos;
/// API administrativa do módulo Matrix (MX-22): estado, liga/desliga e espaços por departamento.
mod matrix;
/// Receptor do Application Service do Matrix (MX-16): rotas que o Synapse chama.
pub mod matrix_as;
mod orgs;
mod paginacao;
mod produtos;
mod saude;
mod sla_analytics;
pub mod teste_chat;
mod tokens;
mod usage;
mod whatsapp;

use axum::Router;
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::IntoResponse;
use axum::routing::get;
use poph_store::Status;
use serde_json::json;

use crate::erro::ApiError;
use crate::estado::Estado;

pub use backup::{LIMITE_IMPORTACAO, LIMITE_IMPORTACAO_BYTES};
pub use paginacao::{ConsultaPagina, ListaJson, proximo_cursor};
pub use saude::{HealthJson, MetricasJson};

/// Partes do registro de ferramentas de um turno (`turns.tools_json`).
///
/// Formato atual (HR-47b): `{"nomes": [...], "chamadas": [...]}`. Turno antigo gravou só a lista de
/// nomes — nesse caso os nomes são a própria lista e não há chamadas detalhadas. Valor ilegível
/// vira lista vazia nas duas partes, nunca erro de leitura.
pub(crate) fn partes_das_ferramentas(bruto: &str) -> (serde_json::Value, serde_json::Value) {
    let vazio = || serde_json::Value::Array(Vec::new());
    match serde_json::from_str::<serde_json::Value>(bruto) {
        Ok(serde_json::Value::Array(nomes)) => (serde_json::Value::Array(nomes), vazio()),
        Ok(serde_json::Value::Object(mapa)) => (
            mapa.get("nomes").cloned().unwrap_or_else(vazio),
            mapa.get("chamadas").cloned().unwrap_or_else(vazio),
        ),
        _ => (vazio(), vazio()),
    }
}

/// Rotas da aplicação (API + saúde + métricas + OpenAPI).
pub fn router(estado: Estado) -> Router {
    let admin = Router::new()
        // ── orgs ──
        .route("/admin/v1/orgs", get(orgs::listar).post(orgs::criar))
        .route(
            "/admin/v1/orgs/{org_id}",
            get(orgs::mostrar)
                .patch(orgs::atualizar)
                .delete(orgs::excluir),
        )
        .route(
            "/admin/v1/orgs/{org_id}/agents",
            get(agentes::listar).post(agentes::criar),
        )
        .route(
            "/admin/v1/orgs/{org_id}/agents/{agent_id}",
            get(agentes::mostrar)
                .patch(agentes::atualizar)
                .delete(agentes::excluir),
        )
        // ── fluxos liberados ao agente (FX-28) ──
        .route(
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/flows",
            get(agente_fluxos::listar).put(agente_fluxos::substituir),
        )
        // ── fluxos: validar, rascunho, publicar, revisões, diff, preview, formatar (FX-27) ──
        .route(
            "/admin/v1/orgs/{org_id}/flows/validate",
            axum::routing::post(fluxos::validar_script),
        )
        .route(
            "/admin/v1/orgs/{org_id}/flows/format",
            axum::routing::post(fluxos::formatar),
        )
        .route(
            "/admin/v1/orgs/{org_id}/flows",
            get(fluxos::listar).post(fluxos::criar),
        )
        .route(
            "/admin/v1/orgs/{org_id}/flows/{flow_id}",
            get(fluxos::mostrar)
                .patch(fluxos::atualizar)
                .delete(fluxos::excluir),
        )
        .route(
            "/admin/v1/orgs/{org_id}/flows/{flow_id}/publish",
            axum::routing::post(fluxos::publicar),
        )
        .route(
            "/admin/v1/orgs/{org_id}/flows/{flow_id}/revisions",
            get(fluxos::listar_revisoes),
        )
        .route(
            "/admin/v1/orgs/{org_id}/flows/{flow_id}/revisions/{number}",
            get(fluxos::mostrar_revisao),
        )
        .route(
            "/admin/v1/orgs/{org_id}/flows/{flow_id}/diff",
            get(fluxos::diff),
        )
        .route(
            "/admin/v1/orgs/{org_id}/flows/{flow_id}/preview",
            get(fluxos::preview),
        )
        // ── Fichas: leitura (FX-29) ──
        .route("/admin/v1/orgs/{org_id}/fichas", get(fichas::listar))
        .route(
            "/admin/v1/orgs/{org_id}/fichas/{ficha_id}",
            get(fichas::mostrar),
        )
        .route(
            "/admin/v1/orgs/{org_id}/fichas/{ficha_id}/events",
            get(fichas::eventos),
        )
        // ── Fichas: ações humanas do atendente (FX-30) ──
        .route(
            "/admin/v1/orgs/{org_id}/fichas/{ficha_id}/flow-runs",
            axum::routing::post(fichas::iniciar_fluxo),
        )
        .route(
            "/admin/v1/orgs/{org_id}/fichas/{ficha_id}/flow-runs/{run_id}/actions",
            axum::routing::post(fichas::acao),
        )
        .route(
            "/admin/v1/orgs/{org_id}/fichas/{ficha_id}/flow-runs/{run_id}/abandon",
            axum::routing::post(fichas::abandonar),
        )
        .route(
            "/admin/v1/orgs/{org_id}/fichas/{ficha_id}/close",
            axum::routing::post(fichas::fechar),
        )
        .route("/admin/v1/orgs/{org_id}/analytics/conversations", get(conversation_analytics::report))
        .route("/admin/v1/orgs/{org_id}/analytics/sla", get(sla_analytics::report))
        .route("/admin/v1/orgs/{org_id}/usage/tokens", get(usage::tokens))
        .route(
            "/admin/v1/orgs/{org_id}/analytics/operational-summary",
            get(analytics::operational_summary),
        )
        .route(
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/instructions",
            get(instrucoes::listar).post(instrucoes::criar),
        )
        .route(
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/instructions/current",
            get(instrucoes::mostrar)
                .patch(instrucoes::atualizar)
                .delete(instrucoes::excluir),
        )
        .route(
            "/admin/v1/orgs/{org_id}/knowledge",
            get(conhecimento::listar).post(conhecimento::criar),
        )
        .route(
            "/admin/v1/orgs/{org_id}/knowledge/{group_id}",
            get(conhecimento::mostrar)
                .patch(conhecimento::atualizar)
                .delete(conhecimento::excluir),
        )
        .route(
            "/admin/v1/orgs/{org_id}/knowledge/{group_id}/items",
            get(conhecimento::listar_itens).post(conhecimento::criar_item),
        )
        .route(
            "/admin/v1/orgs/{org_id}/knowledge/{group_id}/items/{item_id}",
            get(conhecimento::mostrar_item)
                .patch(conhecimento::atualizar_item)
                .delete(conhecimento::excluir_item),
        )
        .route(
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/knowledge/{group_id}",
            axum::routing::post(conhecimento::vincular).delete(conhecimento::desvincular),
        )
        .route(
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/knowledge/reindex",
            axum::routing::post(conhecimento::reindexar),
        )
        .route(
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/integrations",
            get(integracoes::listar).post(integracoes::criar),
        )
        .route(
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/integrations/{integration_id}",
            get(integracoes::mostrar)
                .patch(integracoes::atualizar)
                .delete(integracoes::excluir),
        )
        // ── WhatsApp da integração (HR-44b) ──
        .route(
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/integrations/{integration_id}/whatsapp/qrcode",
            axum::routing::post(whatsapp::qrcode),
        )
        .route(
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/integrations/{integration_id}/whatsapp/state",
            get(whatsapp::estado),
        )
        .route(
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/integrations/{integration_id}/whatsapp/disconnect",
            axum::routing::post(whatsapp::desconectar),
        )
        // ── backup da org (HR-48e-1) ──
        .route(
            "/admin/v1/orgs/{org_id}/backup",
            get(backup::exportar),
        )
        .route(
            "/admin/v1/orgs/{org_id}/backup/import",
            axum::routing::post(backup::importar).layer(
                axum::extract::DefaultBodyLimit::max(backup::LIMITE_IMPORTACAO),
            ),
        )
        // ── cluster (HR-50a) ──
        .route(
            "/admin/v1/cluster/reload",
            axum::routing::post(cluster::recarregar),
        )
        // ── manifesto de plugin (HR-44c, lista na HR-48b) ──
        .route(
            "/admin/v1/orgs/{org_id}/manifests",
            get(manifestos::listar),
        )
        .route(
            "/admin/v1/orgs/{org_id}/manifests/{kind}",
            get(manifestos::mostrar),
        )
        // ── tokens ──
        .route(
            "/admin/v1/orgs/{org_id}/tokens",
            get(tokens::listar).post(tokens::criar),
        )
        .route(
            "/admin/v1/orgs/{org_id}/tokens/{token_id}",
            get(tokens::mostrar).delete(tokens::revogar),
        )
        // ── categorias ──
        .route(
            "/admin/v1/orgs/{org_id}/categories",
            get(categorias::listar).post(categorias::criar),
        )
        .route(
            "/admin/v1/orgs/{org_id}/categories/{category_id}",
            get(categorias::mostrar)
                .patch(categorias::atualizar)
                .delete(categorias::excluir),
        )
        .route(
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/files",
            get(biblioteca::listar).post(biblioteca::criar),
        )
        .route(
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/files/{file_id}",
            get(biblioteca::mostrar).delete(biblioteca::excluir),
        )
        .route(
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/files/{file_id}/content",
            get(biblioteca::conteudo),
        )
        .route(
            "/admin/v1/orgs/{org_id}/conversations",
            get(conversas::listar),
        )
        .route(
            "/admin/v1/orgs/{org_id}/conversations/{conversation_id}",
            get(conversas::mostrar),
        )
        .route(
            "/admin/v1/orgs/{org_id}/conversations/{conversation_id}/messages",
            get(conversas::mensagens),
        )
        .route(
            "/admin/v1/orgs/{org_id}/conversations/{conversation_id}/turns",
            get(conversas::turnos),
        )
        .route(
            "/admin/v1/orgs/{org_id}/conversations/{conversation_id}/reopen",
            axum::routing::post(conversas::reabrir),
        )
        .route(
            "/admin/v1/orgs/{org_id}/conversations/{conversation_id}/close",
            axum::routing::post(conversas::fechar),
        )
        .route(
            "/admin/v1/orgs/{org_id}/conversations/{conversation_id}/transfer",
            axum::routing::post(conversas::transferir),
        )
        // ── chat de teste do Console (HR-22) ──
        .route(
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/test-chat",
            get(teste_chat::listar).post(teste_chat::criar),
        )
        .route(
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/test-chat/{conversation_id}",
            get(teste_chat::historico),
        )
        .route(
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/test-chat/{conversation_id}/messages",
            axum::routing::post(teste_chat::mensagem)
                .layer(axum::extract::DefaultBodyLimit::max(24 * 1024 * 1024)),
        )
        .route(
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/test-chat/{conversation_id}/reopen",
            axum::routing::post(teste_chat::reabrir),
        )
        .route(
            "/admin/v1/orgs/{org_id}/agents/{agent_id}/test-chat/{conversation_id}/messages/{message_id}/attachments/{attachment_id}/content",
            get(teste_chat::anexo_conteudo),
        )
        .route(
            "/admin/v1/orgs/{org_id}/products",
            get(produtos::listar).post(produtos::criar),
        )
        .route(
            "/admin/v1/orgs/{org_id}/products/{product_id}",
            get(produtos::mostrar)
                .patch(produtos::atualizar)
                .delete(produtos::excluir),
        )
        // ── departamentos ──
        .route(
            "/admin/v1/orgs/{org_id}/departments",
            get(departamentos::listar).post(departamentos::criar),
        )
        .route(
            "/admin/v1/orgs/{org_id}/departments/{department_id}",
            get(departamentos::mostrar)
                .patch(departamentos::atualizar)
                .delete(departamentos::excluir),
        )
        // ── módulo Matrix: estado e espaços por departamento (MX-22) ──
        .route(
            "/admin/v1/orgs/{org_id}/matrix",
            get(matrix::mostrar).put(matrix::atualizar),
        )
        .route(
            "/admin/v1/orgs/{org_id}/matrix/spaces",
            get(matrix::listar_espacos).put(matrix::ligar_espaco),
        )
        .route(
            "/admin/v1/orgs/{org_id}/matrix/spaces/{department_id}",
            axum::routing::delete(matrix::desligar_espaco),
        )
        // ── Application Service do Matrix (MX-16) ──
        // Contrato do Synapse, não do Console: ficam fora do OpenAPI administrativo e respondem
        // `404` quando o módulo está desligado (`Estado::matrix_as` = `None`).
        .merge(matrix_as::rotas())
        .with_state(estado.clone());

    Router::new()
        .merge(admin)
        .route("/healthz", get(saude::healthz))
        .route("/metrics", get(saude::metrics))
        // Entrada do WhatsApp (HR-26b): pública, sem token de administração; quem autentica é o
        // segredo do webhook de cada instância. Fica fora do OpenAPI administrativo.
        .route(
            "/webhook/evolution/{org_id}/{instancia}",
            axum::routing::post(crate::evolution::receber).layer(
                axum::extract::DefaultBodyLimit::max(poph_evolution::DEFAULT_BODY_CAP as usize),
            ),
        )
        .route("/openapi.json", get(saude::openapi))
        .route("/admin/v1/openapi.json", get(saude::openapi))
        // Console embutido (HR-38): qualquer rota fora das rotas do servidor é página do Console.
        // Rota inexistente e método errado também respondem no formato de erro do PRD §8.1: o
        // Console e os scripts sempre casam pelo `code`, nunca pelo status sozinho.
        .fallback(pagina_do_console_ou_404)
        .method_not_allowed_fallback(metodo_nao_permitido)
        .with_state(estado.clone())
        // Roteamento entre instâncias (HR-50b): pedido de org que vive em outra instância é
        // encaminhado assinado para a dona; pedido que chega encaminhado passa pela conferência da
        // assinatura antes da rota. Rotas sem org no caminho (saúde, métricas, OpenAPI, recarga,
        // Console, `/admin/v1/orgs`) seguem locais como sempre.
        .layer(axum::middleware::from_fn_with_state(
            estado,
            crate::encaminhamento::middleware,
        ))
}

/// 404 — rota que não existe, no mesmo formato de erro das demais.
async fn rota_inexistente(uri: Uri) -> ApiError {
    let caminho = uri.path().to_owned();
    ApiError::novo(
        StatusCode::NOT_FOUND,
        "not_found",
        format!("a rota `{caminho}` não existe"),
    )
    .com_detalhes(json!({ "path": caminho }))
}

/// Última rota do servidor: `GET`/`HEAD` fora das rotas do servidor recebe o Console; o resto
/// continua sendo o erro de rota inexistente em JSON (o Console nunca responde a `POST` alheio).
async fn pagina_do_console_ou_404(metodo: Method, uri: Uri) -> axum::response::Response {
    if metodo == Method::GET || metodo == Method::HEAD {
        return crate::console::servir(&uri, &metodo);
    }
    rota_inexistente(uri).await.into_response()
}

/// 405 — rota existe, método não. Traz o mesmo formato de erro.
async fn metodo_nao_permitido() -> ApiError {
    ApiError::novo(
        StatusCode::METHOD_NOT_ALLOWED,
        "method_not_allowed",
        "método não permitido nesta rota",
    )
}

/// Cabeçalho `If-Match` com a versão lida antes. Aceita `3`, `"3"` e `W/"3"`.
pub fn versao_do_if_match(headers: &HeaderMap, entidade: &str) -> Result<u64, ApiError> {
    let valor = headers
        .get(axum::http::header::IF_MATCH)
        .ok_or_else(|| ApiError::if_match_obrigatorio(entidade))?;
    let texto = valor
        .to_str()
        .map_err(|_| ApiError::invalido("If-Match não é texto ASCII"))?;
    let limpo = texto
        .trim()
        .trim_start_matches("W/")
        .trim_matches('"')
        .trim();
    limpo.parse::<u64>().map_err(|_| {
        ApiError::invalido(format!(
            "If-Match `{texto}` não é uma versão numérica (ex.: 3)"
        ))
        .com_detalhes(serde_json::json!({ "header": "If-Match", "value": texto }))
    })
}

/// Lê a situação (`active`, `inactive`, `archived`) do corpo da requisição.
pub fn status_pedido(texto: &str) -> Result<Status, ApiError> {
    match texto {
        "active" => Ok(Status::Active),
        "inactive" => Ok(Status::Inactive),
        "archived" => Ok(Status::Archived),
        outro => Err(ApiError::validacao(
            "status",
            format!("`{outro}` não é uma situação; use active, inactive ou archived"),
        )),
    }
}

/// Situação em texto para a resposta.
pub fn status_texto(status: Status) -> &'static str {
    status.as_text()
}

/// Documento OpenAPI da API administrativa.
pub fn documento() -> utoipa::openapi::OpenApi {
    use utoipa::OpenApi as _;
    OpenApiDoc::openapi()
}

/// Documento OpenAPI da API administrativa.
#[derive(utoipa::OpenApi)]
#[openapi(
    info(
        title = "POP Harness — API administrativa",
        description = "API `/admin/v1` do motor de agentes multi-tenant. Autenticação por token \
                       (`Authorization: Bearer`), escopo por org, erros em `{code, message, details}` \
                       e paginação por cursor.",
        version = "0.1.0"
    ),
    paths(
        integracoes::listar, integracoes::mostrar, integracoes::criar, integracoes::atualizar, integracoes::excluir,
        manifestos::listar, manifestos::mostrar,
        cluster::recarregar,
        backup::exportar, backup::importar,
        whatsapp::qrcode, whatsapp::estado, whatsapp::desconectar,
        conhecimento::vincular, conhecimento::desvincular, conhecimento::reindexar,
        conhecimento::listar_itens, conhecimento::mostrar_item, conhecimento::criar_item, conhecimento::atualizar_item, conhecimento::excluir_item,
        conhecimento::listar, conhecimento::mostrar, conhecimento::criar, conhecimento::atualizar, conhecimento::excluir,
        agentes::listar, agentes::mostrar, agentes::criar, agentes::atualizar, agentes::excluir,
        agente_fluxos::listar, agente_fluxos::substituir,
        instrucoes::listar, instrucoes::mostrar, instrucoes::criar, instrucoes::atualizar, instrucoes::excluir,
        orgs::listar,
        orgs::criar,
        orgs::mostrar,
        orgs::atualizar,
        orgs::excluir,
        tokens::listar,
        tokens::criar,
        tokens::mostrar,
        tokens::revogar,
        categorias::listar,
        categorias::criar,
        categorias::mostrar,
        categorias::atualizar,
        categorias::excluir,
        departamentos::listar,
        departamentos::criar,
        departamentos::mostrar,
        departamentos::atualizar,
        departamentos::excluir,
        biblioteca::criar,
        biblioteca::listar,
        biblioteca::mostrar,
        biblioteca::conteudo,
        biblioteca::excluir,
        conversas::listar,
        conversas::mostrar,
        conversas::mensagens,
        conversas::turnos,
        conversas::reabrir,
        conversas::fechar,
        conversas::transferir,
        matrix::mostrar,
        matrix::atualizar,
        matrix::listar_espacos,
        matrix::ligar_espaco,
        matrix::desligar_espaco,
        teste_chat::criar,
        teste_chat::listar,
        teste_chat::historico,
        teste_chat::reabrir,
        teste_chat::mensagem,
        teste_chat::anexo_conteudo,
        produtos::criar,
        produtos::atualizar,
        produtos::excluir,
        produtos::listar,
        produtos::mostrar,
        // FX-27 e FX-29/30: fluxos e Fichas (rotas novas do módulo de Fluxos e Fichas).
        fluxos::validar_script, fluxos::listar, fluxos::criar, fluxos::mostrar, fluxos::atualizar,
        fluxos::excluir, fluxos::publicar, fluxos::listar_revisoes, fluxos::mostrar_revisao,
        fluxos::diff, fluxos::preview, fluxos::formatar,
        fichas::listar, fichas::mostrar, fichas::eventos, fichas::iniciar_fluxo, fichas::acao,
        fichas::abandonar, fichas::fechar,
        analytics::operational_summary,
        conversation_analytics::report,
        sla_analytics::report,
        usage::tokens,
        saude::healthz,
        saude::metrics,
    ),
    components(schemas(
        integracoes::Dados,
        whatsapp::DadosPareamento,
        conhecimento::DadosItem,
        conhecimento::Dados,
        agentes::DadosAgente,
        instrucoes::Dados,
        crate::erro::RespostaErro,
        orgs::OrgJson,
        orgs::CriarOrg,
        orgs::AtualizarOrg,
        tokens::TokenJson,
        tokens::CriarToken,
        tokens::TokenCriadoJson,
        produtos::DadosProduto,
        categorias::CategoriaJson,
        categorias::CriarCategoria,
        categorias::AtualizarCategoria,
        departamentos::DepartamentoJson,
        departamentos::CriarDepartamento,
        departamentos::AtualizarDepartamento,
        paginacao::ListaJson<orgs::OrgJson>,
        orgs::InstanciaStatusJson,
        paginacao::ListaJson<tokens::TokenJson>,
        paginacao::ListaJson<categorias::CategoriaJson>,
        paginacao::ListaJson<departamentos::DepartamentoJson>,
        biblioteca::ArquivoJson,
        paginacao::ListaJson<biblioteca::ArquivoJson>,
        teste_chat::DadosMensagemTeste,
        matrix::EstadoMatrixJson,
        matrix::AtrasoJson,
        matrix::EspacoJson,
        matrix::EspacosJson,
        matrix::LigarDesligar,
        matrix::LigarEspaco,
        matrix::EspacoDesligadoJson,
        saude::HealthJson,
        saude::MetricasJson,
        sla_analytics::SlaReportJson, sla_analytics::AgeBucket, sla_analytics::StaleItem,
        conversation_analytics::ConversationReportJson, conversation_analytics::SummaryJson, conversation_analytics::DayJson, conversation_analytics::AgentJson, conversation_analytics::ChannelJson,
        analytics::OperationalSummaryJson,
        analytics::NodeCountJson,
        usage::TokenUsageJson, usage::AgentUsageJson, usage::DayUsageJson,
    )),
    tags(
        (name = "orgs", description = "Orgs (tenants)"),
        (name = "tokens", description = "Tokens de acesso à API"),
        (name = "categories", description = "Categorias de produto"),
        (name = "departments", description = "Departamentos"),
        (name = "test-chat", description = "Chat de teste do Console (SSE e anexos)"),
        (name = "operacao", description = "Saúde, métricas e contrato"),
        (name = "backup", description = "Exportar e importar a organização num arquivo único"),
        (name = "cluster", description = "Roteamento estático entre instâncias (cluster.toml)"),
        (name = "matrix", description = "Módulo Matrix: estado por org e espaços por departamento"),
        (name = "flows", description = "Fluxos: script, validação, publicação, revisões, diff e preview"),
        (name = "fichas", description = "Fichas do atendimento, execuções de fluxo e ações humanas"),
        (name = "usage", description = "Uso agregado de tokens"),
        (name = "analytics", description = "Leitura agregada do atendimento: contadores e execuções ativas por nó"),
    ),
    modifiers(&EsquemaDeSeguranca)
)]
struct OpenApiDoc;

struct EsquemaDeSeguranca;

impl utoipa::Modify for EsquemaDeSeguranca {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        use utoipa::openapi::security::{Http, HttpAuthScheme, SecurityScheme};
        let componentes = openapi.components.get_or_insert_with(Default::default);
        componentes.add_security_scheme(
            "bearer",
            SecurityScheme::Http(Http::new(HttpAuthScheme::Bearer)),
        );
    }
}

#[cfg(test)]
mod testes_ferramentas {
    use super::partes_das_ferramentas;
    use serde_json::json;

    #[test]
    fn formato_atual_separa_nomes_e_chamadas() {
        let bruto = json!({
            "nomes": ["consultar_base"],
            "chamadas": [{"nome": "consultar_base", "estado": "executada"}]
        })
        .to_string();
        let (nomes, chamadas) = partes_das_ferramentas(&bruto);
        assert_eq!(nomes, json!(["consultar_base"]));
        assert_eq!(chamadas.as_array().expect("lista").len(), 1);
    }

    #[test]
    fn turno_antigo_com_lista_de_nomes_le_sem_erro() {
        let (nomes, chamadas) = partes_das_ferramentas("[\"buscar_produto\",\"consultar_base\"]");
        assert_eq!(nomes, json!(["buscar_produto", "consultar_base"]));
        assert_eq!(chamadas, json!([]));
    }

    #[test]
    fn valor_ilegivel_ou_vazio_vira_lista_vazia() {
        for bruto in ["", "não é json", "null", "{}"] {
            let (nomes, chamadas) = partes_das_ferramentas(bruto);
            assert_eq!(nomes, json!([]), "nomes de `{bruto}`");
            assert_eq!(chamadas, json!([]), "chamadas de `{bruto}`");
        }
    }
}
