use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Subcommand};
use poph_store::{NewOrg, Org, OrgChange, Store};
use serde_json::{Value, json};

use crate::agent::{derive_slug, is_valid_slug};
use crate::config;
use crate::support::{
    count_all, describe, failure, json_from_text, open_store, parse_status, print_json,
    require_offline, resolve_org, status_in_portuguese, validate_config_json, validate_limit,
    validate_text,
};

#[derive(Debug, Subcommand)]
pub enum OrgAction {
    #[command(about = "Cria uma org")]
    Create(CreateOrg),
    #[command(about = "Lista as orgs (primeira página, por id)")]
    List(ListOrgs),
    #[command(about = "Mostra uma org e quantos agentes ela tem")]
    Show(OrgRef),
    #[command(about = "Altera uma org com `--if-match` (a versão lida antes)")]
    Update(UpdateOrg),
    #[command(about = "Exclui a org. Se ela tiver conteúdo, exige `--force` e apaga tudo junto")]
    Delete(DeleteOrg),
}

#[derive(Debug, Args)]
pub struct CreateOrg {
    #[arg(long, help = "Nome de exibição (até 120 caracteres)")]
    pub name: String,

    #[arg(
        long,
        help = "Slug único (`^[a-z0-9-]{3,64}$`). Sem ele, é derivado do nome"
    )]
    pub slug: Option<String>,

    #[arg(long, help = "Saída em JSON (formato estável)")]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct ListOrgs {
    #[arg(long, default_value_t = 50, help = "Quantas orgs trazer (1 a 200)")]
    pub limit: u32,

    #[arg(long, help = "Saída em JSON (formato estável)")]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct OrgRef {
    #[arg(help = "Id (UUID v7) ou slug da org")]
    pub org: String,

    #[arg(long, help = "Saída em JSON (formato estável)")]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct UpdateOrg {
    #[arg(help = "Id (UUID v7) ou slug da org")]
    pub org: String,

    #[arg(long, help = "Versão lida com `show` (`If-Match`). Obrigatória")]
    pub if_match: u64,

    #[arg(long, help = "Novo nome")]
    pub name: Option<String>,

    #[arg(long, help = "Nova situação: `active`, `inactive` ou `archived`")]
    pub status: Option<String>,

    #[arg(
        long,
        help = "Nova configuração da org (objeto JSON: retenção, limites, padrões de modelo)"
    )]
    pub config_json: Option<String>,

    #[arg(long, help = "Saída em JSON (formato estável)")]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct DeleteOrg {
    #[arg(help = "Id (UUID v7) ou slug da org")]
    pub org: String,

    #[arg(
        long,
        help = "Confirma a exclusão quando a org tem conteúdo (agentes, categorias, departamentos, tokens)"
    )]
    pub force: bool,

    #[arg(long, help = "Saída em JSON (formato estável)")]
    pub json: bool,
}

pub fn run(
    flag: Option<PathBuf>,
    environment: &config::Environment,
    offline: bool,
    action: OrgAction,
) -> ExitCode {
    if let Err(error) = require_offline(offline) {
        return failure(error);
    }
    let store = match open_store(flag, environment) {
        Ok(store) => store,
        Err(error) => return failure(error),
    };
    match action {
        OrgAction::Create(args) => create(&store, args),
        OrgAction::List(args) => list(&store, args),
        OrgAction::Show(args) => show(&store, args),
        OrgAction::Update(args) => update(&store, args),
        OrgAction::Delete(args) => delete(&store, args),
    }
}

fn create(store: &Store, args: CreateOrg) -> ExitCode {
    let name = match validate_text("nome", &args.name, 120) {
        Ok(name) => name,
        Err(error) => return failure(error),
    };
    let slug = args.slug.clone().unwrap_or_else(|| derive_slug(&name));
    if !is_valid_slug(&slug) {
        return failure(format!(
            "slug `{slug}` inválido: use de 3 a 64 caracteres, só a-z, 0-9 e `-`"
        ));
    }
    match store.orgs().by_slug(&slug) {
        Ok(_) => return failure(format!("já existe uma org com o slug `{slug}`")),
        Err(error) if error.is_not_found() => {}
        Err(error) => return failure(describe(&error)),
    }
    let new_org = NewOrg {
        name,
        slug,
        config_json: String::new(),
    };
    match store.orgs().create(new_org) {
        Ok(org) => print_org(&org, args.json, "org criada", None),
        Err(error) => failure(describe(&error)),
    }
}

fn list(store: &Store, args: ListOrgs) -> ExitCode {
    let page = match validate_limit(args.limit) {
        Ok(page) => page,
        Err(error) => return failure(error),
    };
    match store.orgs().list(page) {
        Ok(orgs) => {
            if args.json {
                let items: Vec<Value> = orgs.iter().map(json_org).collect();
                print_json(&json!({
                    "versao_relatorio": 1,
                    "total": items.len(),
                    "orgs": items,
                }));
            } else if orgs.is_empty() {
                println!("Nenhuma org cadastrada.");
            } else {
                for org in &orgs {
                    println!(
                        "{}  slug={:<24} status={:<9} {}",
                        org.id,
                        org.slug,
                        status_in_portuguese(org.status),
                        org.name
                    );
                }
                println!("\n{} org(s).", orgs.len());
            }
            ExitCode::SUCCESS
        }
        Err(error) => failure(describe(&error)),
    }
}

fn show(store: &Store, args: OrgRef) -> ExitCode {
    let id = match resolve_org(store, &args.org) {
        Ok(id) => id,
        Err(error) => return failure(error),
    };
    let org = match store.orgs().get(id) {
        Ok(org) => org,
        Err(error) => return failure(describe(&error)),
    };
    let agents = match count_agents(store, id) {
        Ok(total) => total,
        Err(error) => return failure(error),
    };
    print_org(&org, args.json, "", Some(agents))
}

fn update(store: &Store, args: UpdateOrg) -> ExitCode {
    let id = match resolve_org(store, &args.org) {
        Ok(id) => id,
        Err(error) => return failure(error),
    };
    let name = match args
        .name
        .as_deref()
        .map(|name| validate_text("nome", name, 120))
        .transpose()
    {
        Ok(name) => name,
        Err(error) => return failure(error),
    };
    let status = match args.status.as_deref().map(parse_status).transpose() {
        Ok(status) => status,
        Err(error) => return failure(error),
    };
    let config_json = match args
        .config_json
        .as_deref()
        .map(validate_config_json)
        .transpose()
    {
        Ok(config) => config,
        Err(error) => return failure(error),
    };
    if name.is_none() && status.is_none() && config_json.is_none() {
        return failure("nada a alterar: informe --name, --status ou --config-json");
    }
    let change = OrgChange {
        name,
        status,
        config_json,
    };
    match store.orgs().update(id, args.if_match, change) {
        Ok(org) => print_org(&org, args.json, "org alterada", None),
        Err(error) => failure(describe(&error)),
    }
}

fn delete(store: &Store, args: DeleteOrg) -> ExitCode {
    let id = match resolve_org(store, &args.org) {
        Ok(id) => id,
        Err(error) => return failure(error),
    };
    let content = match org_content(store, id) {
        Ok(content) => content,
        Err(error) => return failure(error),
    };
    if content.total() > 0 && !args.force {
        return failure(format!(
            "a org tem conteúdo ({}). Excluir a org apaga tudo isso junto, e os produtos das \
             categorias. Para confirmar, repita o comando com --force",
            content.summary()
        ));
    }
    let result = if args.force {
        store
            .orgs()
            .delete_forced(id)
            .map_err(|error| describe(&error))
    } else {
        store.orgs().delete(id).map(|()| 0u64).map_err(|error| {
            if matches!(
                error,
                poph_store::StoreError::Domain(poph_types::DomainError::Conflict { .. })
            ) {
                "a org tem agentes: exclua os agentes antes, ou repita com --force".to_owned()
            } else {
                describe(&error)
            }
        })
    };
    match result {
        Ok(agents_removed) => {
            if args.json {
                print_json(&json!({
                    "versao_relatorio": 1,
                    "removida": true,
                    "org_id": id.to_string(),
                    "agentes_removidos": agents_removed,
                    "categorias_removidas": content.categories,
                    "departamentos_removidos": content.departments,
                    "tokens_removidos": content.tokens,
                }));
            } else {
                println!("org {id} excluída ({})", content.summary());
            }
            ExitCode::SUCCESS
        }
        Err(error) => failure(error),
    }
}

struct Content {
    agents: usize,
    categories: usize,
    departments: usize,
    tokens: usize,
}

impl Content {
    fn total(&self) -> usize {
        self.agents + self.categories + self.departments + self.tokens
    }

    fn summary(&self) -> String {
        format!(
            "{} agente(s), {} categoria(s), {} departamento(s), {} token(s)",
            self.agents, self.categories, self.departments, self.tokens
        )
    }
}

fn org_content(store: &Store, org: poph_types::OrgId) -> Result<Content, String> {
    Ok(Content {
        agents: count_agents(store, org)?,
        categories: count_all(
            |page| store.categories().list(org, page),
            |item| item.id.as_uuid(),
        )?,
        departments: count_all(
            |page| store.departments().list(org, page),
            |item| item.id.as_uuid(),
        )?,
        tokens: count_all(
            |page| store.tokens().list(org, page),
            |item| item.id.as_uuid(),
        )?,
    })
}

fn count_agents(store: &Store, org: poph_types::OrgId) -> Result<usize, String> {
    count_all(
        |page| store.agents().list(org, page),
        |item| item.id.as_uuid(),
    )
}

fn print_org(org: &Org, json: bool, message: &str, agents: Option<usize>) -> ExitCode {
    if json {
        let mut value = json_org(org);
        if let (Some(total), Value::Object(map)) = (agents, &mut value) {
            map.insert("agents_count".to_owned(), json!(total));
        }
        print_json(&json!({
            "versao_relatorio": 1,
            "org": value,
        }));
    } else {
        if !message.is_empty() {
            println!("{message}\n");
        }
        print!("{}", org_text(org, agents));
    }
    ExitCode::SUCCESS
}

fn json_org(org: &Org) -> Value {
    json!({
        "id": org.id.to_string(),
        "name": org.name,
        "slug": org.slug,
        "status": org.status.as_text(),
        "config": json_from_text(&org.config_json),
        "version": org.version,
        "created_at": org.created_at,
        "updated_at": org.updated_at,
    })
}

fn org_text(org: &Org, agents: Option<usize>) -> String {
    let mut text = format!(
        "org {}\n  nome: {}\n  slug: {}\n  situação: {}\n",
        org.id,
        org.name,
        org.slug,
        status_in_portuguese(org.status)
    );
    if let Some(total) = agents {
        text.push_str(&format!("  agentes: {total}\n"));
    }
    text.push_str(&format!("  versão: {}\n", org.version));
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_content_does_not_require_force() {
        let empty = Content {
            agents: 0,
            categories: 0,
            departments: 0,
            tokens: 0,
        };
        assert_eq!(empty.total(), 0);
        let with_category = Content {
            categories: 1,
            ..empty
        };
        assert_eq!(with_category.total(), 1);
        assert!(with_category.summary().contains("1 categoria(s)"));
    }
}
