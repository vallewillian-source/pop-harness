//! Rota do manifesto de plugin (HR-44c) e da lista de tipos declarados (HR-48b).
//!
//! Entrega o manifesto que o plugin declarou no handshake — o contrato completo: nome, versão,
//! capacidades, limites e os **JSON Schema** da configuração e dos segredos. É o que o Console
//! precisa para desenhar o formulário sem código por integração.
//!
//! **Lista (`GET …/manifests`, HR-48b):** devolve os tipos declarados em `[plugins].integracoes`,
//! um item por `kind`, com nome, versão e situação (`ok`/`indisponivel`). Existe porque a tela não
//! tem como adivinhar o `kind`: sem ela, quem não sabe o nome do plugin não o encontra. Um plugin
//! que não responde sai marcado e **não derruba a lista**.
//!
//! **Escopo:** as rotas ficam sob a organização (`/orgs/{org_id}/manifests…`) só para herdar o
//! escopo de token das outras rotas da API. O manifesto em si é da instalação, não da org: é
//! metadado do plugin, sem dado de cliente — por isso quem vê é qualquer token da org, global ou
//! de escopo, e nada é auditado (é leitura). Token de outra org recebe `403`, como nas demais.
//!
//! **Onde o manifesto mora:** memória, preenchida na primeira chamada por
//! [`crate::manifestos::CatalogoManifestos`], que sobe o plugin uma vez pelo supervisor, lê o
//! manifesto e encerra o processo. Tipo sem plugin declarado na configuração responde `404`; plugin
//! declarado que não responde ao handshake responde `503 manifesto_indisponivel`.

use crate::{
    auth::Identidade,
    erro::{ApiError, id_da_url},
    estado::Estado,
};
use axum::{Json, extract::Path, extract::State, http::StatusCode};
use poph_types::OrgId;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize)]
pub struct Caminho {
    org_id: String,
    kind: String,
}

#[derive(Deserialize)]
pub struct CaminhoOrg {
    org_id: String,
}

#[utoipa::path(get,operation_id="listar_manifests",path="/admin/v1/orgs/{org_id}/manifests",tag="manifests",params(("org_id"=String,Path)),security(("bearer"=[])),responses((status=200,description="Plugins declarados na configuração da instância: um item por tipo (`kind`) com nome, versão e situação (`ok`/`indisponivel`); plugin que não responde sai marcado e não derruba a lista",body=Value)))]
pub async fn listar(
    State(e): State<Estado>,
    ident: Identidade,
    Path(c): Path<CaminhoOrg>,
) -> Result<Json<Value>, ApiError> {
    let org = id_da_url::<OrgId>(&c.org_id)?;
    ident.exigir_org(org)?;
    let itens = e.manifestos.listar().await;
    // Lista da instância: não pagina (é a configuração, não cresce com o uso) e não tem cursor.
    Ok(Json(json!({ "items": itens })))
}

#[utoipa::path(get,path="/admin/v1/orgs/{org_id}/manifests/{kind}",tag="manifests",params(("org_id"=String,Path),("kind"=String,Path)),security(("bearer"=[])),responses((status=200,description="Manifesto do plugin (nome, versão, capacidades, limites e JSON Schema de configuração e segredos)",body=Value),(status=404,description="Tipo sem plugin declarado",body=crate::erro::RespostaErro),(status=503,description="Plugin declarado que não respondeu ao handshake",body=crate::erro::RespostaErro)))]
pub async fn mostrar(
    State(e): State<Estado>,
    ident: Identidade,
    Path(c): Path<Caminho>,
) -> Result<Json<Value>, ApiError> {
    let org = id_da_url::<OrgId>(&c.org_id)?;
    ident.exigir_org(org)?;
    let kind = c.kind.trim();
    if kind.is_empty() {
        return Err(ApiError::validacao("kind", "obrigatório"));
    }
    match e.manifestos.manifesto(kind).await {
        Ok(manifesto) => Ok(Json(
            serde_json::to_value(&manifesto).unwrap_or_else(|_| json!({ "kind": kind })),
        )),
        Err(falha @ crate::manifestos::FalhaManifesto::NaoDeclarado) => Err(ApiError::novo(
            StatusCode::NOT_FOUND,
            "not_found",
            format!("manifesto `{kind}` não encontrado: {}", falha.mensagem()),
        )
        .com_detalhes(json!({ "kind": kind }))),
        Err(falha @ crate::manifestos::FalhaManifesto::Indisponivel { .. }) => Err(ApiError::novo(
            StatusCode::SERVICE_UNAVAILABLE,
            "manifesto_indisponivel",
            falha.mensagem(),
        )
        .com_detalhes(json!({ "kind": kind }))),
    }
}
