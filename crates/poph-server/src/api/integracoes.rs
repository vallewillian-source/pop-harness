//! Administração de integrações: ciclo de vida, configuração validada pelo manifesto do plugin e
//! segredos cifrados (HR-44, HR-44c).
//!
//! Regras:
//! - **quem manda no formato é o manifesto.** `config_json` e `secrets` só são aceitos quando o
//!   `kind` tem plugin declarado em `[plugins].integracoes`; sem manifesto a recusa continua
//!   `422 manifest_indisponivel`, como antes da HR-44c.
//! - **configuração é pública, segredo não.** `config_json` volta na leitura; segredo nunca volta —
//!   a resposta diz `tem_segredo` e, quando a chave de cifra abre o blob, `segredos_preenchidos`
//!   (nomes, nunca valores).
//! - **segredo sai deste processo só no handshake do plugin** (`docs/PLUGIN_PROTOCOLO.md`).
//! - **toda escrita exige `If-Match`** e sobe a versão; configuração e segredo são edição de
//!   configuração, ao contrário de `connection_state` (estado de execução).

use std::collections::BTreeMap;

use super::{ConsultaPagina, ListaJson, proximo_cursor, versao_do_if_match};
use crate::{
    auth::Identidade,
    cifra::ChaveDeCifra,
    erro::{ApiError, id_da_url},
    estado::Estado,
    manifestos::{FalhaManifesto, validar_objeto},
};
use axum::{
    Json,
    extract::{
        Path, Query, State,
        rejection::{JsonRejection, QueryRejection},
    },
    http::{HeaderMap, StatusCode},
};
use poph_store::{AlteracaoIntegracao, IntegracaoAdmin};
use poph_types::{AgentId, IntegrationId, Manifesto, OrgId};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize)]
pub struct Caminho {
    org_id: String,
    agent_id: String,
}
#[derive(Deserialize)]
pub struct CaminhoItem {
    org_id: String,
    agent_id: String,
    integration_id: String,
}
#[derive(Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Dados {
    kind: Option<String>,
    /// Configuração do operador. Validada contra o JSON Schema do manifesto do plugin.
    config_json: Option<Value>,
    /// Segredos do canal: nome → valor. `""` remove o segredo. Nunca volta na resposta nem no log.
    secrets: Option<BTreeMap<String, String>>,
    status: Option<String>,
    connection_state: Option<String>,
}
fn validar_status(status: Option<String>) -> Result<Option<String>, ApiError> {
    if let Some(s) = &status {
        super::status_pedido(s)?;
    }
    Ok(status)
}
fn ids(org: &str, agente: &str, ident: &Identidade) -> Result<(OrgId, AgentId), ApiError> {
    let org = id_da_url::<OrgId>(org)?;
    ident.exigir_org(org)?;
    Ok((org, id_da_url::<AgentId>(agente)?))
}

/// Resposta de uma integração.
///
/// `segredos_preenchidos` é `null` quando há segredo gravado mas a chave de cifra atual não o abre:
/// a API prefere dizer "não sei quais" a mentir que nenhum está preenchido.
fn valor(v: IntegracaoAdmin, chave: Option<&ChaveDeCifra>) -> Value {
    let preenchidos = match (&v.segredos_cifrados, chave) {
        (None, _) => Some(Vec::<String>::new()),
        (Some(_), None) => None,
        (Some(blob), Some(chave)) => chave
            .decifrar(blob)
            .ok()
            .map(|segredos| segredos.into_keys().collect()),
    };
    json!({
        "id": v.id,
        "kind": v.kind,
        "status": v.status,
        "connection_state": v.connection_state,
        "version": v.version,
        "tem_segredo": v.tem_segredo,
        "configuracao": v.configuracao,
        "segredos_preenchidos": preenchidos,
    })
}

/// Objeto vazio (configuração ausente e segredos vazios).
fn objeto_vazio() -> Value {
    Value::Object(Default::default())
}

/// Configuração "de verdade": `null` e `{}` contam como ausente (era o contrato antes da HR-44c).
fn configuracao_enviada(valor: &Option<Value>) -> bool {
    match valor {
        None | Some(Value::Null) => false,
        Some(Value::Object(mapa)) => !mapa.is_empty(),
        Some(_) => true,
    }
}

/// Segredo "de verdade": mapa ausente ou vazio conta como nada enviado.
fn segredos_enviados(valor: &Option<BTreeMap<String, String>>) -> bool {
    valor.as_ref().is_some_and(|mapa| !mapa.is_empty())
}

/// Valida um objeto contra um dos esquemas do manifesto, no formato de erro da API.
fn conferir(
    campo: &'static str,
    valores: Option<&Value>,
    esquema: &poph_types::EsquemaJson,
) -> Result<(), ApiError> {
    let Some(valores) = valores else {
        return Ok(());
    };
    validar_objeto(valores, &esquema.0, campo)
        .map_err(|falha| ApiError::validacao(falha.campo, falha.motivo))
}

/// Manifesto do `kind`. `None` = tipo sem plugin declarado (integração sem configuração, como o
/// `whatsapp` da Evolution). Plugin declarado que não responde vira `503` e não é silenciado.
async fn manifesto_do_kind(e: &Estado, kind: &str) -> Result<Option<Manifesto>, ApiError> {
    match e.manifestos.manifesto(kind).await {
        Ok(manifesto) => Ok(Some(manifesto)),
        Err(FalhaManifesto::NaoDeclarado) => Ok(None),
        Err(falha @ FalhaManifesto::Indisponivel { .. }) => Err(ApiError::novo(
            StatusCode::SERVICE_UNAVAILABLE,
            "manifesto_indisponivel",
            falha.mensagem(),
        )
        .com_detalhes(json!({ "kind": kind }))),
    }
}

/// Sem manifesto não há esquema: gravar configuração seria gravar às cegas.
fn sem_manifesto() -> ApiError {
    ApiError::novo(
        StatusCode::UNPROCESSABLE_ENTITY,
        "manifest_indisponivel",
        "configuração de integração exige o manifesto do plugin, disponível na entrega de \
         hospedagem de plugins",
    )
}

/// O que a requisição pediu para gravar, já validado.
struct Preparado {
    /// Configuração a substituir (o `PATCH` que só mexe em segredo mantém a anterior).
    configuracao: Option<Value>,
    /// `None` mantém os segredos atuais; `Some(None)` limpa; `Some(Some(blob))` troca o blob.
    segredos: Option<Option<Vec<u8>>>,
}

/// Valida configuração e segredos contra o manifesto e devolve o que gravar.
fn preparar(
    e: &Estado,
    manifesto: &Manifesto,
    configuracao: Option<&Value>,
    segredos: Option<BTreeMap<String, String>>,
) -> Result<Preparado, ApiError> {
    let configuracao = match configuracao {
        None => None,
        Some(Value::Null) => Some(objeto_vazio()),
        Some(valor) => {
            conferir("config_json", Some(valor), &manifesto.configuracao)?;
            Some(valor.clone())
        }
    };
    let Some(segredos) = segredos else {
        return Ok(Preparado {
            configuracao,
            segredos: None,
        });
    };
    // Valor vazio remove o segredo (não vai ao plugin). O mapa resultante é o conjunto completo,
    // então `required` do esquema vale sobre ele.
    let segredos: BTreeMap<String, String> = segredos
        .into_iter()
        .filter(|(_, valor)| !valor.is_empty())
        .collect();
    let como_valor = Value::Object(
        segredos
            .iter()
            .map(|(nome, valor)| (nome.clone(), Value::String(valor.clone())))
            .collect(),
    );
    conferir("secrets", Some(&como_valor), &manifesto.segredos)?;
    if segredos.is_empty() {
        return Ok(Preparado {
            configuracao,
            segredos: Some(None),
        });
    }
    let chave = e.chave_de_cifra.as_deref().ok_or_else(|| {
        ApiError::novo(
            StatusCode::SERVICE_UNAVAILABLE,
            "cifra_indisponivel",
            "o servidor está sem a chave de cifra (`POPH_CHAVE_CIFRA`): segredo de integração não \
             é gravado em claro",
        )
    })?;
    let blob = chave.cifrar(&segredos).map_err(|falha| {
        tracing::error!(%falha, "falha ao cifrar segredo de integração");
        ApiError::novo(
            StatusCode::INTERNAL_SERVER_ERROR,
            "cifra_falhou",
            "não consegui cifrar o segredo da integração",
        )
    })?;
    Ok(Preparado {
        configuracao,
        segredos: Some(Some(blob)),
    })
}

/// Segredos que o `PATCH` deve gravar: mescla os novos sobre os já gravados.
///
/// Sem a chave de cifra não há como ler os anteriores — e gravar só os novos apagaria os demais.
/// Nesse caso a resposta é `503 cifra_indisponivel`, nunca uma perda silenciosa.
fn mesclar_segredos(
    e: &Estado,
    novos: BTreeMap<String, String>,
    anteriores: Option<&[u8]>,
) -> Result<BTreeMap<String, String>, ApiError> {
    let mut mesclados = match anteriores {
        None => BTreeMap::new(),
        Some(blob) => {
            let chave = e.chave_de_cifra.as_deref().ok_or_else(|| {
                ApiError::novo(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "cifra_indisponivel",
                    "o servidor está sem a chave de cifra (`POPH_CHAVE_CIFRA`): não consigo ler os \
                     segredos já gravados para mesclar os novos",
                )
            })?;
            chave.decifrar(blob).map_err(|falha| {
                tracing::error!(%falha, "segredo de integração ilegível com a chave atual");
                ApiError::novo(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "cifra_indisponivel",
                    format!("os segredos gravados não abrem com a chave atual: {falha}"),
                )
            })?
        }
    };
    for (nome, valor) in novos {
        if valor.is_empty() {
            mesclados.remove(&nome);
        } else {
            mesclados.insert(nome, valor);
        }
    }
    Ok(mesclados)
}

#[utoipa::path(get,path="/admin/v1/orgs/{org_id}/agents/{agent_id}/integrations",tag="integrations",params(("org_id"=String,Path),("agent_id"=String,Path),("limit"=Option<u32>,Query),("cursor"=Option<String>,Query)),security(("bearer"=[])),responses((status=200,description="Integrações",body=ListaJson<Value>)))]
pub async fn listar(
    State(e): State<Estado>,
    ident: Identidade,
    Path(c): Path<Caminho>,
    query: Result<Query<ConsultaPagina>, QueryRejection>,
) -> Result<Json<ListaJson<Value>>, ApiError> {
    let (org, agente) = ids(&c.org_id, &c.agent_id, &ident)?;
    let Query(q) = query.map_err(|_| ApiError::invalido("parâmetros de paginação inválidos"))?;
    let pagina = q.pagina()?;
    let items: Vec<_> = e
        .store
        .integracoes_listar(org, agente, pagina)?
        .into_iter()
        .map(|v| valor(v, e.chave_de_cifra.as_deref()))
        .collect();
    let next_cursor = proximo_cursor(&items, pagina.limite, |v| {
        v["id"].as_str().unwrap_or_default().to_owned()
    });
    Ok(Json(ListaJson { items, next_cursor }))
}

#[utoipa::path(get,path="/admin/v1/orgs/{org_id}/agents/{agent_id}/integrations/{integration_id}",tag="integrations",params(("org_id"=String,Path),("agent_id"=String,Path),("integration_id"=String,Path)),security(("bearer"=[])),responses((status=200,description="Integração",body=Value)))]
pub async fn mostrar(
    State(e): State<Estado>,
    ident: Identidade,
    Path(c): Path<CaminhoItem>,
) -> Result<Json<Value>, ApiError> {
    let (org, agente) = ids(&c.org_id, &c.agent_id, &ident)?;
    let id = id_da_url::<IntegrationId>(&c.integration_id)?;
    Ok(Json(valor(
        e.store.integracao_obter(org, agente, id)?,
        e.chave_de_cifra.as_deref(),
    )))
}

#[utoipa::path(post,path="/admin/v1/orgs/{org_id}/agents/{agent_id}/integrations",tag="integrations",params(("org_id"=String,Path),("agent_id"=String,Path)),security(("bearer"=[])),request_body=Dados,responses((status=201,description="Integração criada",body=Value)))]
pub async fn criar(
    State(e): State<Estado>,
    ident: Identidade,
    Path(c): Path<Caminho>,
    corpo: Result<Json<Dados>, JsonRejection>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let (org, agente) = ids(&c.org_id, &c.agent_id, &ident)?;
    let Json(d) = corpo.map_err(|_| ApiError::invalido("corpo JSON de integração inválido"))?;
    if d.connection_state.is_some() {
        return Err(ApiError::validacao(
            "connection_state",
            "não permitido na criação",
        ));
    }
    let kind = d
        .kind
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| ApiError::validacao("kind", "obrigatório"))?;
    let status = validar_status(d.status)?.unwrap_or_else(|| "active".into());
    // Com manifesto, a integração nasce validada: o que o esquema exige é cobrado já na criação.
    // Sem manifesto (ex.: `whatsapp`, que usa a Evolution), configurar continua recusado.
    let enviou = configuracao_enviada(&d.config_json) || segredos_enviados(&d.secrets);
    let (configuracao, segredos) = match manifesto_do_kind(&e, &kind).await? {
        Some(manifesto) => {
            let sem_configuracao = objeto_vazio();
            let preparado = preparar(
                &e,
                &manifesto,
                Some(d.config_json.as_ref().unwrap_or(&sem_configuracao)),
                // Sem segredos enviados, o conjunto é vazio: `required` do esquema vale igual.
                Some(d.secrets.clone().unwrap_or_default()),
            )?;
            (
                preparado.configuracao.unwrap_or_else(objeto_vazio),
                preparado.segredos.flatten(),
            )
        }
        None if enviou => return Err(sem_manifesto()),
        None => (objeto_vazio(), None),
    };
    let v = e.store.integracao_criar_com_configuracao(
        org,
        agente,
        kind,
        status,
        configuracao,
        segredos,
    )?;
    super::auditoria::registrar(&e, &ident, Some(org), "create", "integration", v.id.clone())?;
    Ok((
        StatusCode::CREATED,
        Json(valor(v, e.chave_de_cifra.as_deref())),
    ))
}

#[utoipa::path(patch,path="/admin/v1/orgs/{org_id}/agents/{agent_id}/integrations/{integration_id}",tag="integrations",params(("org_id"=String,Path),("agent_id"=String,Path),("integration_id"=String,Path),("If-Match"=String,Header)),security(("bearer"=[])),request_body=Dados,responses((status=200,description="Integração alterada",body=Value),(status=412,description="Conflito",body=crate::erro::RespostaErro)))]
pub async fn atualizar(
    State(e): State<Estado>,
    ident: Identidade,
    Path(c): Path<CaminhoItem>,
    h: HeaderMap,
    corpo: Result<Json<Dados>, JsonRejection>,
) -> Result<Json<Value>, ApiError> {
    let (org, agente) = ids(&c.org_id, &c.agent_id, &ident)?;
    let id = id_da_url::<IntegrationId>(&c.integration_id)?;
    let versao = versao_do_if_match(&h, "integração")?;
    let Json(d) = corpo.map_err(|_| ApiError::invalido("corpo JSON de integração inválido"))?;
    if d.kind.is_some() {
        return Err(ApiError::validacao("kind", "não pode ser alterado"));
    }
    let mexe_configuracao = configuracao_enviada(&d.config_json) || segredos_enviados(&d.secrets);
    if d.status.is_none() && d.connection_state.is_none() && !mexe_configuracao {
        return Err(ApiError::invalido("nada a alterar"));
    }
    let status = validar_status(d.status)?;
    let preparado = if mexe_configuracao {
        let atual = e.store.integracao_obter(org, agente, id)?;
        match manifesto_do_kind(&e, &atual.kind).await? {
            Some(manifesto) => {
                let segredos = match d.secrets.clone() {
                    None => None,
                    Some(novos) => Some(mesclar_segredos(
                        &e,
                        novos,
                        atual.segredos_cifrados.as_deref(),
                    )?),
                };
                Some(preparar(&e, &manifesto, d.config_json.as_ref(), segredos)?)
            }
            None => return Err(sem_manifesto()),
        }
    } else {
        None
    };
    let (configuracao, segredos) = match preparado {
        None => (None, None),
        Some(preparado) => (preparado.configuracao, preparado.segredos),
    };
    let v = e.store.integracao_atualizar(
        org,
        agente,
        id,
        AlteracaoIntegracao {
            versao,
            status: status.as_deref(),
            estado: d.connection_state.as_deref(),
            configuracao: configuracao.as_ref(),
            segredos: segredos.as_ref().map(|blob| blob.as_deref()),
        },
    )?;
    super::auditoria::registrar(
        &e,
        &ident,
        Some(org),
        "update",
        "integration",
        id.to_string(),
    )?;
    Ok(Json(valor(v, e.chave_de_cifra.as_deref())))
}

#[utoipa::path(delete,path="/admin/v1/orgs/{org_id}/agents/{agent_id}/integrations/{integration_id}",tag="integrations",params(("org_id"=String,Path),("agent_id"=String,Path),("integration_id"=String,Path)),security(("bearer"=[])),responses((status=200,description="Integração excluída",body=Value)))]
pub async fn excluir(
    State(e): State<Estado>,
    ident: Identidade,
    Path(c): Path<CaminhoItem>,
) -> Result<Json<Value>, ApiError> {
    let (org, agente) = ids(&c.org_id, &c.agent_id, &ident)?;
    let id = id_da_url::<IntegrationId>(&c.integration_id)?;
    e.store.integracao_excluir(org, agente, id)?;
    super::auditoria::registrar(
        &e,
        &ident,
        Some(org),
        "delete",
        "integration",
        id.to_string(),
    )?;
    Ok(Json(json!({"deleted":true})))
}
