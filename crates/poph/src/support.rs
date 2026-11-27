use std::path::PathBuf;
use std::process::ExitCode;
use std::str::FromStr;

use poph_store::{Fluxo, Page, Status, Store, StoreError};
use poph_types::{AgentId, DomainError, FlowId, HarnessConfig, OrgId};
use serde_json::Value;

use crate::config;

pub fn require_offline(offline: bool) -> Result<(), String> {
    if offline {
        Ok(())
    } else {
        Err(
            "use `--offline`: por enquanto estes comandos só funcionam direto no banco \
             (o modo com servidor ainda não existe)"
                .to_owned(),
        )
    }
}

pub fn open_store(
    flag: Option<PathBuf>,
    environment: &config::Environment,
) -> Result<Store, String> {
    let folder = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let file = config::resolve_file(flag.as_deref(), environment, &folder);
    let configuracao = config::load(file.as_deref(), environment)
        .map_err(|error| format!("configuração inválida: {error}"))?;
    Store::open(&configuracao).map_err(|error| format!("não consegui abrir o banco: {error}"))
}

pub fn open_with_configuration(
    flag: Option<PathBuf>,
    environment: &config::Environment,
) -> Result<(Store, HarnessConfig), String> {
    let folder = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let file = config::resolve_file(flag.as_deref(), environment, &folder);
    let configuracao = config::load(file.as_deref(), environment)
        .map_err(|error| format!("configuração inválida: {error}"))?;
    let store = Store::open(&configuracao)
        .map_err(|error| format!("não consegui abrir o banco: {error}"))?;
    Ok((store, configuracao))
}

pub fn resolve_agent(store: &Store, org: OrgId, texto: &str) -> Result<poph_store::Agente, String> {
    let found = if uuid::Uuid::parse_str(texto).is_ok() {
        let id: AgentId = parse_id(texto)?;
        store.agents().get(org, id)
    } else {
        store.agents().by_slug(org, texto)
    };
    found.map_err(|error| {
        if error.is_not_found() {
            format!("não existe agente `{texto}` nesta org")
        } else {
            describe(&error)
        }
    })
}

pub fn resolve_flow(store: &Store, org: OrgId, texto: &str) -> Result<Fluxo, String> {
    let found = if uuid::Uuid::parse_str(texto).is_ok() {
        let id: FlowId = parse_id(texto)?;
        store.flows().get(org, id).map(Some)
    } else {
        store.flows().by_slug(org, texto)
    };
    match found {
        Ok(Some(fluxo)) => Ok(fluxo),
        Ok(None) => Err(format!("não existe fluxo `{texto}` nesta org")),
        Err(error) if error.is_not_found() => Err(format!("não existe fluxo `{texto}` nesta org")),
        Err(error) => Err(describe(&error)),
    }
}

pub fn parse_page(limit: u32, cursor: Option<&str>) -> Result<Page, String> {
    let mut page = validate_limit(limit)?;
    page.after =
        match cursor {
            None | Some("") => None,
            Some(texto) => Some(uuid::Uuid::parse_str(texto).map_err(|_| {
                "cursor inválido: use o `next_cursor` da página anterior".to_owned()
            })?),
        };
    Ok(page)
}

pub fn open_with_org(
    flag: Option<PathBuf>,
    environment: &config::Environment,
    offline: bool,
    org: Option<&str>,
) -> Result<(Store, OrgId), String> {
    require_offline(offline)?;
    let texto =
        org.ok_or_else(|| "informe --org com o id (UUID v7) ou o slug da org".to_owned())?;
    let store = open_store(flag, environment)?;
    let id = resolve_org(&store, texto)?;
    Ok((store, id))
}

pub fn resolve_org(store: &Store, texto: &str) -> Result<OrgId, String> {
    if uuid::Uuid::parse_str(texto).is_ok() {
        let id: OrgId = parse_id(texto)?;
        return match store.orgs().get(id) {
            Ok(org) => Ok(org.id),
            Err(error) if error.is_not_found() => Err(format!("não existe org com o id `{texto}`")),
            Err(error) => Err(describe(&error)),
        };
    }
    match store.orgs().by_slug(texto) {
        Ok(org) => Ok(org.id),
        Err(error) if error.is_not_found() => Err(format!("não existe org com o slug `{texto}`")),
        Err(error) => Err(describe(&error)),
    }
}

pub fn parse_id<T>(texto: &str) -> Result<T, String>
where
    T: FromStr<Err = DomainError>,
{
    texto.parse::<T>().map_err(|error| error.to_string())
}

pub fn describe(error: &StoreError) -> String {
    match error {
        StoreError::Domain(DomainError::VersionConflict {
            expected, current, ..
        }) => format!(
            "a versão não confere: você informou a versão {expected}, mas o registro está na \
             versão {current}. Leia de novo com `show` e repita com a versão atual"
        ),
        StoreError::Domain(DomainError::NotFound { entity, id }) => {
            format!("{entity} `{id}` não existe nesta org")
        }
        StoreError::Domain(DomainError::Validation { field, reason }) => {
            format!("{field}: {reason}")
        }
        StoreError::Domain(DomainError::Conflict { reason, .. }) => reason.clone(),
        StoreError::Domain(other) => other.to_string(),
        other => format!("não consegui gravar no banco: {other}"),
    }
}

pub fn parse_status(texto: &str) -> Result<Status, String> {
    match texto {
        "active" => Ok(Status::Active),
        "inactive" => Ok(Status::Inactive),
        "archived" => Ok(Status::Archived),
        other => Err(format!(
            "--status `{other}` inválido: use active, inactive ou archived"
        )),
    }
}

pub fn status_in_portuguese(status: Status) -> &'static str {
    match status {
        Status::Active => "ativo",
        Status::Inactive => "inativo",
        Status::Archived => "arquivado",
    }
}

pub fn validate_text(campo: &str, value: &str, max: usize) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(format!("{campo} não pode ficar vazio"));
    }
    if trimmed.chars().count() > max {
        return Err(format!("{campo} passa de {max} caracteres"));
    }
    Ok(trimmed.to_owned())
}

pub fn validate_limit(limit: u32) -> Result<Page, String> {
    if (1..=200).contains(&limit) {
        Ok(Page::with_limit(limit))
    } else {
        Err("--limit precisa estar entre 1 e 200".to_owned())
    }
}

pub fn count_all<T>(
    mut listar: impl FnMut(Page) -> Result<Vec<T>, StoreError>,
    id_of: impl Fn(&T) -> uuid::Uuid,
) -> Result<usize, String> {
    let mut total = 0;
    let mut after = None;
    loop {
        let batch = listar(Page { after, limit: 200 }).map_err(|error| describe(&error))?;
        total += batch.len();
        if batch.len() < 200 {
            return Ok(total);
        }
        after = batch.last().map(&id_of);
    }
}

pub fn validate_config_json(texto: &str) -> Result<String, String> {
    match serde_json::from_str::<Value>(texto) {
        Ok(value) if value.is_object() => Ok(texto.to_owned()),
        _ => Err("--config-json precisa ser um objeto JSON, por exemplo {}".to_owned()),
    }
}

pub fn json_from_text(texto: &str) -> Value {
    serde_json::from_str::<Value>(texto).unwrap_or_else(|_| serde_json::json!({}))
}

pub fn print_json(value: &Value) {
    println!(
        "{}",
        serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
    );
}

pub fn failure(mensagem: impl AsRef<str>) -> ExitCode {
    eprintln!("{}", mensagem.as_ref());
    ExitCode::from(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_accepts_only_database_texts() {
        assert_eq!(parse_status("active").unwrap(), Status::Active);
        assert_eq!(parse_status("inactive").unwrap(), Status::Inactive);
        assert_eq!(parse_status("archived").unwrap(), Status::Archived);
        let error = parse_status("ativo").unwrap_err();
        assert!(error.contains("active, inactive ou archived"), "{error}");
    }

    #[test]
    fn required_text_rejects_empty_and_long() {
        assert_eq!(validate_text("nome", "  Vendas  ", 10).unwrap(), "Vendas");
        assert!(
            validate_text("nome", "   ", 10)
                .unwrap_err()
                .contains("não pode ficar vazio")
        );
        let long = "x".repeat(11);
        assert!(
            validate_text("nome", &long, 10)
                .unwrap_err()
                .contains("passa de 10")
        );
    }

    #[test]
    fn limit_stays_between_1_and_200() {
        assert!(validate_limit(0).is_err());
        assert!(validate_limit(201).is_err());
        assert_eq!(validate_limit(200).unwrap().limit, 200);
    }

    #[test]
    fn config_json_must_be_object() {
        assert!(validate_config_json("{}").is_ok());
        assert!(validate_config_json("[1]").is_err());
        assert!(validate_config_json("não é json").is_err());
    }

    #[test]
    fn require_offline_rejects_without_flag() {
        assert!(require_offline(true).is_ok());
        assert!(require_offline(false).unwrap_err().contains("--offline"));
    }

    #[test]
    fn version_conflict_reports_both_versions() {
        let error = StoreError::Domain(DomainError::VersionConflict {
            entity: "categoria de produto",
            id: "x".to_owned(),
            expected: 2,
            current: 5,
        });
        let texto = describe(&error);
        assert!(texto.contains("informou a versão 2"), "{texto}");
        assert!(texto.contains("versão 5"), "{texto}");
    }
}
