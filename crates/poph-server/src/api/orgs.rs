//! Orgs (tenants): CRUD em `/admin/v1/orgs`.

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use poph_store::{NewOrg, Org, OrgChange, Page, Status};
use poph_types::OrgId;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;

use super::paginacao::{ConsultaPagina, ListaJson, proximo_cursor};
use super::{status_pedido, status_texto, versao_do_if_match};
use crate::auth::Identidade;
use crate::erro::{ApiError, id_da_url};
use crate::estado::Estado;

/// Org na resposta da API.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct OrgJson {
    /// Id (UUID v7).
    pub id: String,
    /// Nome de exibição.
    pub name: String,
    /// Identificador na URL.
    pub slug: String,
    /// `active`, `inactive` ou `archived`.
    pub status: String,
    /// Configuração da org (retenção, limites, padrões).
    #[schema(value_type = Object)]
    pub config: Value,
    /// Versão para `If-Match`.
    pub version: u64,
    /// Criação (ms desde a época Unix).
    pub created_at: i64,
    /// Última alteração (ms).
    pub updated_at: i64,
    /// Quantos agentes **ativos** (`status = active`) a org tem hoje (CF-03a). Vem sempre, nas duas
    /// formas da listagem (com e sem `?cluster=true`) e na leitura de uma org só; `0` quando não há
    /// agente ativo. É contado por uma consulta agregada por página, nunca uma por linha.
    ///
    /// `serde(default)` só vale na **leitura**: é o que faz a listagem do cluster continuar
    /// funcionando quando uma instância vizinha ainda não tem o campo (versão anterior).
    #[serde(default)]
    pub agents_active_count: u32,
    /// Instância dona da org (HR-50b-2). Só aparece com `?cluster=true`; sem o parâmetro, a
    /// resposta é a de sempre, sem este campo.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instancia: Option<String>,
}

impl OrgJson {
    fn de(org: &Org) -> Self {
        let config = serde_json::from_str(&org.config_json)
            .unwrap_or_else(|_| Value::Object(Default::default()));
        Self {
            id: org.id.to_string(),
            name: org.name.clone(),
            slug: org.slug.clone(),
            status: status_texto(org.status).to_owned(),
            config,
            version: org.version,
            created_at: org.created_at,
            updated_at: org.updated_at,
            agents_active_count: 0,
            instancia: None,
        }
    }

    fn na_instancia(mut self, instancia: &str) -> Self {
        self.instancia = Some(instancia.to_owned());
        self
    }

    fn com_agentes_ativos(mut self, ativos: u32) -> Self {
        self.agents_active_count = ativos;
        self
    }
}

/// Monta os itens da resposta com `agents_active_count` preenchido.
///
/// A contagem das orgs da página sai de **uma** consulta agregada
/// ([`poph_store::Agentes::contar_ativos_por_org`]) — a listagem não faz consulta por linha, e o
/// número não é inventado: org que aparece na consulta recebe o total, as demais ficam com `0`.
fn itens_com_agentes_ativos(
    estado: &Estado,
    orgs: &[Org],
    instancia: Option<&str>,
) -> Result<Vec<OrgJson>, ApiError> {
    let ids: Vec<OrgId> = orgs.iter().map(|org| org.id).collect();
    let contagem = estado.store.agents().count_active_by_org(&ids)?;
    Ok(orgs
        .iter()
        .map(|org| {
            let item =
                OrgJson::de(org).com_agentes_ativos(contagem.get(&org.id).copied().unwrap_or(0));
            match instancia {
                Some(instancia) => item.na_instancia(instancia),
                None => item,
            }
        })
        .collect())
}

/// Lista de orgs. Sem `?cluster=true` é a mesma resposta de sempre (`items` + `next_cursor`);
/// com o parâmetro, `instancias` soma o estado de cada instância do mapa e cada item traz a sua
/// instância em `instancia`.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ListaOrgsJson {
    /// Os registros desta página (ou todas as orgs, com `?cluster=true`).
    pub items: Vec<OrgJson>,
    /// Cursor para a próxima página; `null` na última (e sempre `null` com `?cluster=true`).
    pub next_cursor: Option<String>,
    /// Estado das instâncias do mapa, quando `?cluster=true`. Ausente sem o parâmetro.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instancias: Option<Vec<InstanciaStatusJson>>,
}

impl ListaOrgsJson {
    fn local(lista: ListaJson<OrgJson>) -> Self {
        Self {
            items: lista.items,
            next_cursor: lista.next_cursor,
            instancias: None,
        }
    }
}

/// Estado de uma instância na listagem com `?cluster=true`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct InstanciaStatusJson {
    /// Id da instância no mapa.
    pub id: String,
    /// Endereço base declarado no mapa.
    pub url: String,
    /// `local` (esta instância), `ok` (respondeu) ou `inacessivel`.
    pub situacao: String,
    /// Quantas orgs desta instância vieram nesta resposta.
    pub orgs: u32,
    /// Frase de operação quando a instância não respondeu (nada de segredo nem caminho).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mensagem: Option<String>,
}

/// Corpo de criação de org.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CriarOrg {
    /// Nome de exibição (1 a 120 caracteres).
    pub name: String,
    /// Identificador na URL; sem ele, é derivado do nome.
    pub slug: Option<String>,
    /// Configuração inicial (objeto JSON).
    #[schema(value_type = Option<Object>)]
    pub config: Option<Value>,
}

/// Corpo de alteração de org (só o que vier muda).
#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
pub struct AtualizarOrg {
    /// Novo nome.
    pub name: Option<String>,
    /// Nova situação.
    pub status: Option<String>,
    /// Nova configuração (objeto JSON).
    #[schema(value_type = Option<Object>)]
    pub config: Option<Value>,
}

/// Caminho com o id da org.
#[derive(Debug, Clone, Deserialize)]
pub struct CaminhoOrg {
    /// Id da org (UUID v7).
    pub org_id: String,
}

/// Parâmetros de exclusão.
#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
pub struct ExclusaoOrg {
    /// Confirma a exclusão de uma org que tem conteúdo.
    pub force: Option<bool>,
}

/// Parâmetros de `GET /admin/v1/orgs`.
///
/// Os campos da paginação são repetidos aqui em vez de `#[serde(flatten)]`: o extrator de query do
/// axum (serde_urlencoded) não lê struct achatada e responderia `400` a qualquer listagem.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ConsultaOrgs {
    /// Quantos registros trazer (1 a 200). Padrão 50.
    pub limit: Option<u32>,
    /// Último id da página anterior.
    pub cursor: Option<String>,
    /// `true` soma as orgs das outras instâncias do mapa (HR-50b-2). Exige token global.
    pub cluster: Option<bool>,
}

impl ConsultaOrgs {
    /// Paginação comum (cursor inválido vira 400).
    fn pagina(&self) -> Result<Page, ApiError> {
        ConsultaPagina {
            limit: self.limit,
            cursor: self.cursor.clone(),
        }
        .pagina()
    }
}

/// Prazo de cada consulta às instâncias vizinhas em `?cluster=true` (HR-50b-2).
///
/// As consultas saem em paralelo, então o pior caso da resposta é este prazo — não a soma. Uma
/// instância fora do ar aparece marcada e **não** derruba a lista (a mesma regra dos plugins).
const PRAZO_DA_LISTA_REMOTA: std::time::Duration = std::time::Duration::from_secs(2);

/// Quantas páginas de orgs uma instância vizinha pode devolver (200 por página).
const PAGINAS_DA_LISTA_REMOTA: u32 = 20;

/// `GET /admin/v1/orgs`.
#[utoipa::path(
    get,
    path = "/admin/v1/orgs",
    tag = "orgs",
    operation_id = "orgs_listar",
    security(("bearer" = [])),
    params(
        ("limit" = Option<u32>, Query, description = "Registros por página (1 a 200, padrão 50); ignorado com `cluster=true`"),
        ("cursor" = Option<String>, Query, description = "Último id da página anterior; ignorado com `cluster=true`"),
        ("cluster" = Option<bool>, Query, description = "Com `true`, soma as orgs das outras instâncias do mapa (token global) e devolve o estado de cada instância em `instancias`; uma instância fora do ar aparece como `inacessivel` sem derrubar a lista")
    ),
    responses(
        (status = 200, description = "Página de orgs. Cada item traz `agents_active_count` (agentes com `status = active`, `0` quando não há nenhum), nas duas formas da listagem. Com `cluster=true`, `items` traz todas as orgs do cluster (cada uma com `instancia`) e `instancias` traz o estado de cada instância; sem o parâmetro é a página de sempre", body = ListaJson<OrgJson>),
        (status = 401, description = "Token ausente ou inválido", body = crate::erro::RespostaErro)
    )
)]
pub async fn listar(
    State(estado): State<Estado>,
    identidade: Identidade,
    Query(consulta): Query<ConsultaOrgs>,
    cabecalhos: HeaderMap,
) -> Result<Json<ListaOrgsJson>, ApiError> {
    // Token de org só vê a própria org: devolve ela e ignora a paginação e o `cluster=true`
    // (um token de org nunca enxerga outra org, nem para listar).
    if let Some(propria) = identidade.org {
        let org = estado.store.orgs().get(propria)?;
        return Ok(Json(ListaOrgsJson::local(ListaJson {
            items: itens_com_agentes_ativos(&estado, std::slice::from_ref(&org), None)?,
            next_cursor: None,
        })));
    }
    if consulta.cluster.unwrap_or(false) {
        return listar_no_cluster(&estado, &cabecalhos).await.map(Json);
    }
    let pagina = consulta.pagina()?;
    let orgs = estado.store.orgs().list(pagina)?;
    let items = itens_com_agentes_ativos(&estado, &orgs, None)?;
    let next_cursor = proximo_cursor(&items, pagina.limit, |item| item.id.clone());
    Ok(Json(ListaOrgsJson::local(ListaJson { items, next_cursor })))
}

/// Listagem com `?cluster=true`: as orgs desta instância e as das vizinhas do mapa.
///
/// Sem paginação de propósito — o resultado é o retrato do cluster, não uma coleção que cresce a
/// cada segundo. A instância local é lida do banco; cada vizinha é consultada no próprio
/// `/admin/v1/orgs` (rota global, que **não** é encaminhada), com o `Authorization` do chamador,
/// em paralelo e com [`PRAZO_DA_LISTA_REMOTA`]. Vizinha que não responde entra em `instancias` como
/// `inacessivel` e a lista continua.
async fn listar_no_cluster(
    estado: &Estado,
    cabecalhos: &HeaderMap,
) -> Result<ListaOrgsJson, ApiError> {
    let cluster = estado.cluster();
    let local = cluster.config.instance_id.trim().to_owned();
    let mut todas = Vec::new();
    let mut apos = None;
    for _ in 0..PAGINAS_DA_LISTA_REMOTA {
        let pagina = estado.store.orgs().list(Page {
            after: apos,
            limit: 200,
        })?;
        apos = pagina.last().map(|org| org.id.into());
        let ultima = pagina.len() < 200 || apos.is_none();
        todas.extend(pagina);
        if ultima {
            break;
        }
    }
    let mut items: Vec<OrgJson> = itens_com_agentes_ativos(estado, &todas, Some(&local))?;
    let mut instancias = vec![InstanciaStatusJson {
        id: local.clone(),
        url: cluster
            .config
            .instances
            .iter()
            .find(|instancia| instancia.id.trim() == local)
            .map(|instancia| instancia.url.trim().to_owned())
            .unwrap_or_default(),
        situacao: "local".to_owned(),
        orgs: u32::try_from(todas.len()).unwrap_or(u32::MAX),
        mensagem: None,
    }];

    let autorizacao = cabecalhos
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|valor| valor.to_str().ok())
        .map(str::to_owned);
    let cliente = reqwest::Client::builder()
        .connect_timeout(PRAZO_DA_LISTA_REMOTA)
        .timeout(PRAZO_DA_LISTA_REMOTA)
        .build()
        .map_err(|_: reqwest::Error| {
            ApiError::novo(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal",
                "não consegui montar o cliente HTTP da listagem entre instâncias",
            )
        })?;

    let consultas = cluster
        .config
        .instances
        .iter()
        .filter(|instancia| instancia.id.trim() != local)
        .map(|instancia| {
            let cliente = cliente.clone();
            let autorizacao = autorizacao.clone();
            let id = instancia.id.trim().to_owned();
            let url = instancia.url.trim().to_owned();
            async move {
                let resultado = orgs_da_instancia(&cliente, &url, autorizacao.as_deref()).await;
                (id, url, resultado)
            }
        })
        .collect::<Vec<_>>();

    for (id, url, resultado) in futures::future::join_all(consultas).await {
        match resultado {
            Ok(remotas) => {
                let quantidade = u32::try_from(remotas.len()).unwrap_or(u32::MAX);
                items.extend(
                    remotas
                        .into_iter()
                        .map(|org| org.na_instancia(&id))
                        .collect::<Vec<_>>(),
                );
                instancias.push(InstanciaStatusJson {
                    id,
                    url,
                    situacao: "ok".to_owned(),
                    orgs: quantidade,
                    mensagem: None,
                });
            }
            Err(motivo) => instancias.push(InstanciaStatusJson {
                id,
                url,
                situacao: "inacessivel".to_owned(),
                orgs: 0,
                mensagem: Some(motivo),
            }),
        }
    }

    items.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(ListaOrgsJson {
        items,
        next_cursor: None,
        instancias: Some(instancias),
    })
}

/// Lê as orgs de uma instância vizinha, seguindo os cursores (até [`PAGINAS_DA_LISTA_REMOTA`]).
///
/// Erro devolve a frase para o operador (`inacessivel`), nunca segredo nem caminho interno.
async fn orgs_da_instancia(
    cliente: &reqwest::Client,
    url: &str,
    autorizacao: Option<&str>,
) -> Result<Vec<OrgJson>, String> {
    let base = url.trim().trim_end_matches('/');
    let mut orgs: Vec<OrgJson> = Vec::new();
    let mut cursor: Option<String> = None;
    for _ in 0..PAGINAS_DA_LISTA_REMOTA {
        let mut endereco = format!("{base}/admin/v1/orgs?limit=200");
        if let Some(cursor) = cursor.as_deref() {
            endereco.push_str("&cursor=");
            endereco.push_str(cursor);
        }
        let mut pedido = cliente.get(&endereco);
        if let Some(autorizacao) = autorizacao {
            pedido = pedido.header(axum::http::header::AUTHORIZATION, autorizacao);
        }
        let resposta = pedido
            .send()
            .await
            .map_err(|erro| format!("não respondeu: {erro}"))?;
        if !resposta.status().is_success() {
            return Err(format!(
                "o /admin/v1/orgs respondeu {} para este token",
                resposta.status().as_u16()
            ));
        }
        let texto = resposta
            .text()
            .await
            .map_err(|erro| format!("não consegui ler a resposta: {erro}"))?;
        let corpo: Value = serde_json::from_str(&texto)
            .map_err(|_| "a resposta do /admin/v1/orgs não é JSON".to_owned())?;
        let pagina: Vec<OrgJson> = serde_json::from_value(
            corpo
                .get("items")
                .cloned()
                .unwrap_or_else(|| Value::Array(Vec::new())),
        )
        .map_err(|_| {
            "a resposta do /admin/v1/orgs não trouxe orgs no formato esperado".to_owned()
        })?;
        orgs.extend(pagina);
        cursor = corpo
            .get("next_cursor")
            .and_then(|valor| valor.as_str())
            .map(str::to_owned);
        if cursor.is_none() {
            break;
        }
    }
    Ok(orgs)
}

/// `POST /admin/v1/orgs` (só token global).
#[utoipa::path(
    post,
    path = "/admin/v1/orgs",
    tag = "orgs",
    operation_id = "orgs_criar",
    security(("bearer" = [])),
    request_body = CriarOrg,
    responses(
        (status = 201, description = "Org criada", body = OrgJson),
        (status = 400, description = "Campo inválido", body = crate::erro::RespostaErro),
        (status = 403, description = "Token de org não cria org", body = crate::erro::RespostaErro),
        (status = 409, description = "Slug já existe", body = crate::erro::RespostaErro)
    )
)]
pub async fn criar(
    State(estado): State<Estado>,
    identidade: Identidade,
    Json(corpo): Json<CriarOrg>,
) -> Result<(StatusCode, Json<OrgJson>), ApiError> {
    identidade.exigir_global()?;
    let nome = validar_nome(&corpo.name)?;
    let slug = match corpo.slug.as_deref().map(str::trim) {
        Some(texto) if !texto.is_empty() => texto.to_owned(),
        _ => derivar_slug(&nome),
    };
    if !slug_valido(&slug) {
        return Err(ApiError::validacao(
            "slug",
            format!("`{slug}`: use de 3 a 64 caracteres, só a-z, 0-9 e `-`"),
        ));
    }
    if estado.store.orgs().by_slug(&slug).is_ok() {
        return Err(ApiError::conflito(format!(
            "já existe uma org com o slug `{slug}`"
        )));
    }
    let config_json = match &corpo.config {
        None | Some(Value::Null) => "{}".to_owned(),
        Some(Value::Object(_)) => serde_json::to_string(corpo.config.as_ref().unwrap())
            .map_err(|_| ApiError::validacao("config", "não consegui serializar o objeto"))?,
        Some(_) => {
            return Err(ApiError::validacao(
                "config",
                "deve ser um objeto JSON (ex.: {\"retencao\":{\"dias\":30}})",
            ));
        }
    };
    let org = estado.store.orgs().create(NewOrg {
        name: nome,
        slug,
        config_json,
    })?;
    super::auditoria::registrar(
        &estado,
        &identidade,
        Some(org.id),
        "create",
        "org",
        org.id.to_string(),
    )?;
    // Org recém-criada não tem agente: `agents_active_count` é 0 sem precisar consultar.
    Ok((StatusCode::CREATED, Json(OrgJson::de(&org))))
}

/// `GET /admin/v1/orgs/{org_id}`.
#[utoipa::path(
    get,
    path = "/admin/v1/orgs/{org_id}",
    tag = "orgs",
    operation_id = "orgs_mostrar",
    security(("bearer" = [])),
    params(("org_id" = String, Path, description = "Id da org (UUID v7)")),
    responses(
        (status = 200, description = "Org", body = OrgJson),
        (status = 403, description = "Org de outro token", body = crate::erro::RespostaErro),
        (status = 404, description = "Org não encontrada", body = crate::erro::RespostaErro)
    )
)]
pub async fn mostrar(
    State(estado): State<Estado>,
    identidade: Identidade,
    Path(caminho): Path<CaminhoOrg>,
) -> Result<Json<OrgJson>, ApiError> {
    let id = id_da_url::<OrgId>(&caminho.org_id)?;
    identidade.exigir_org(id)?;
    let org = estado.store.orgs().get(id)?;
    let item = itens_com_agentes_ativos(&estado, std::slice::from_ref(&org), None)?
        .pop()
        .unwrap_or_else(|| OrgJson::de(&org));
    Ok(Json(item))
}

/// `PATCH /admin/v1/orgs/{org_id}` (exige `If-Match`).
#[utoipa::path(
    patch,
    path = "/admin/v1/orgs/{org_id}",
    tag = "orgs",
    operation_id = "orgs_atualizar",
    security(("bearer" = [])),
    params(
        ("org_id" = String, Path, description = "Id da org"),
        ("If-Match" = String, Header, description = "Versão lida antes")
    ),
    request_body = AtualizarOrg,
    responses(
        (status = 200, description = "Org alterada", body = OrgJson),
        (status = 412, description = "Versão desatualizada", body = crate::erro::RespostaErro),
        (status = 428, description = "Faltou If-Match", body = crate::erro::RespostaErro)
    )
)]
pub async fn atualizar(
    State(estado): State<Estado>,
    identidade: Identidade,
    Path(caminho): Path<CaminhoOrg>,
    headers: HeaderMap,
    Json(corpo): Json<AtualizarOrg>,
) -> Result<Json<OrgJson>, ApiError> {
    let id = id_da_url::<OrgId>(&caminho.org_id)?;
    identidade.exigir_org(id)?;
    let esperada = versao_do_if_match(&headers, "org")?;
    let status = match &corpo.status {
        Some(texto) => Some(status_pedido(texto)?),
        None => None,
    };
    let config_json = match &corpo.config {
        None => None,
        Some(Value::Object(_)) => Some(
            serde_json::to_string(corpo.config.as_ref().unwrap())
                .map_err(|_| ApiError::validacao("config", "não consegui serializar o objeto"))?,
        ),
        Some(_) => {
            return Err(ApiError::validacao("config", "deve ser um objeto JSON"));
        }
    };
    let mudanca = OrgChange {
        name: corpo.name.as_deref().map(validar_nome).transpose()?,
        status,
        config_json,
    };
    let org = estado.store.orgs().update(id, esperada, mudanca)?;
    super::auditoria::registrar(
        &estado,
        &identidade,
        Some(id),
        "update",
        "org",
        id.to_string(),
    )?;
    let item = itens_com_agentes_ativos(&estado, std::slice::from_ref(&org), None)?
        .pop()
        .unwrap_or_else(|| OrgJson::de(&org));
    Ok(Json(item))
}

/// `DELETE /admin/v1/orgs/{org_id}?force=true` (só token global).
#[utoipa::path(
    delete,
    path = "/admin/v1/orgs/{org_id}",
    tag = "orgs",
    operation_id = "orgs_excluir",
    security(("bearer" = [])),
    params(
        ("org_id" = String, Path, description = "Id da org"),
        ("force" = Option<bool>, Query, description = "Confirma a exclusão de org com conteúdo")
    ),
    responses(
        (status = 204, description = "Org excluída"),
        (status = 409, description = "Org tem agentes e faltou force", body = crate::erro::RespostaErro)
    )
)]
pub async fn excluir(
    State(estado): State<Estado>,
    identidade: Identidade,
    Path(caminho): Path<CaminhoOrg>,
    Query(exclusao): Query<ExclusaoOrg>,
) -> Result<StatusCode, ApiError> {
    let id = id_da_url::<OrgId>(&caminho.org_id)?;
    identidade.exigir_global()?;
    if exclusao.force.unwrap_or(false) {
        estado.store.orgs().delete_forced(id)?;
    } else {
        estado.store.orgs().delete(id)?;
    }
    super::auditoria::registrar(
        &estado,
        &identidade,
        None,
        if exclusao.force.unwrap_or(false) {
            "delete_force"
        } else {
            "delete"
        },
        "org",
        id.to_string(),
    )?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) fn validar_nome(texto: &str) -> Result<String, ApiError> {
    let limpo = texto.trim();
    let tamanho = limpo.chars().count();
    if !(1..=120).contains(&tamanho) {
        return Err(ApiError::validacao(
            "name",
            "deve ter de 1 a 120 caracteres",
        ));
    }
    Ok(limpo.to_owned())
}

pub(crate) fn slug_valido(slug: &str) -> bool {
    let tamanho = slug.chars().count();
    (3..=64).contains(&tamanho)
        && slug
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !slug.starts_with('-')
        && !slug.ends_with('-')
}

fn derivar_slug(nome: &str) -> String {
    let mut slug = String::with_capacity(nome.len());
    let mut ultimo_traco = false;
    for caractere in nome.chars().flat_map(char::to_lowercase) {
        if caractere.is_ascii_alphanumeric() {
            slug.push(caractere);
            ultimo_traco = false;
        } else if !ultimo_traco && !slug.is_empty() {
            slug.push('-');
            ultimo_traco = true;
        }
    }
    let slug = slug.trim_end_matches('-').to_owned();
    if slug.len() < 3 {
        format!("org-{slug}")
    } else {
        slug
    }
}

/// Página usada pelos testes de listagem.
#[allow(dead_code)]
pub(crate) fn pagina_teste() -> Page {
    Page::default()
}

/// Situação usada pelos testes (evita importar `Status` em dois lugares).
#[allow(dead_code)]
pub(crate) fn status_ativo() -> Status {
    Status::Active
}
