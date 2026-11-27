use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Subcommand};
use poph_store::{CategoryChange, NewCategory, ProductCategory, Store};
use poph_types::{OrgId, ProductCategoryId};
use serde_json::{Value, json};

use crate::config;
use crate::support::{
    describe, failure, open_with_org, parse_id, parse_status, print_json, status_in_portuguese,
    validate_limit, validate_text,
};

#[derive(Debug, Subcommand)]
pub enum CategoryAction {
    #[command(about = "Cria uma categoria de produto na org")]
    Create(CriarCategoria),
    #[command(about = "Lista as categorias da org (primeira página, por id)")]
    List(ListCategories),
    #[command(about = "Mostra uma categoria")]
    Show(CategoryReference),
    #[command(about = "Altera uma categoria com `--if-match`")]
    Update(AtualizarCategoria),
    #[command(about = "Exclui a categoria (e os produtos dela). Exige `--force`")]
    Delete(DeleteCategory),
}

#[derive(Debug, Args)]
pub struct CriarCategoria {
    #[arg(
        long,
        help = "Nome (até 120 caracteres; não pode repetir na org, sem diferenciar maiúsculas)"
    )]
    pub name: String,

    #[arg(long, default_value = "", help = "Descrição curta")]
    pub description: String,

    #[arg(long, help = "Saída em JSON (formato estável)")]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct ListCategories {
    #[arg(
        long,
        default_value_t = 50,
        help = "Quantas categorias trazer (1 a 200)"
    )]
    pub limit: u32,

    #[arg(long, help = "Saída em JSON (formato estável)")]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct CategoryReference {
    #[arg(value_name = "CATEGORIA", help = "Id (UUID v7) da categoria")]
    pub category: String,

    #[arg(long, help = "Saída em JSON (formato estável)")]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct AtualizarCategoria {
    #[arg(value_name = "CATEGORIA", help = "Id (UUID v7) da categoria")]
    pub category: String,

    #[arg(long, help = "Versão lida com `show` (`If-Match`). Obrigatória")]
    pub if_match: u64,

    #[arg(long, help = "Novo nome")]
    pub name: Option<String>,

    #[arg(long, help = "Nova descrição")]
    pub description: Option<String>,

    #[arg(long, help = "Nova situação: `active`, `inactive` ou `archived`")]
    pub status: Option<String>,

    #[arg(long, help = "Saída em JSON (formato estável)")]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct DeleteCategory {
    #[arg(value_name = "CATEGORIA", help = "Id (UUID v7) da categoria")]
    pub category: String,

    #[arg(long, help = "Confirma: os produtos da categoria saem junto")]
    pub force: bool,

    #[arg(long, help = "Saída em JSON (formato estável)")]
    pub json: bool,
}

pub fn run(
    flag: Option<PathBuf>,
    environment: &config::Environment,
    offline: bool,
    org: Option<&str>,
    acao: CategoryAction,
) -> ExitCode {
    let (store, org) = match open_with_org(flag, environment, offline, org) {
        Ok(pair) => pair,
        Err(error) => return failure(error),
    };
    match acao {
        CategoryAction::Create(args) => criar(&store, org, args),
        CategoryAction::List(args) => listar(&store, org, args),
        CategoryAction::Show(args) => mostrar(&store, org, args),
        CategoryAction::Update(args) => atualizar(&store, org, args),
        CategoryAction::Delete(args) => excluir(&store, org, args),
    }
}

fn criar(store: &Store, org: OrgId, args: CriarCategoria) -> ExitCode {
    let nome = match validate_text("nome", &args.name, 120) {
        Ok(nome) => nome,
        Err(error) => return failure(error),
    };
    let new_category = NewCategory {
        name: nome,
        description: args.description.trim().to_owned(),
    };
    match store.categories().create(org, new_category) {
        Ok(category) => saida(&category, args.json, "categoria criada"),
        Err(error) => failure(describe(&error)),
    }
}

fn listar(store: &Store, org: OrgId, args: ListCategories) -> ExitCode {
    let page = match validate_limit(args.limit) {
        Ok(page) => page,
        Err(error) => return failure(error),
    };
    match store.categories().list(org, page) {
        Ok(categories) => {
            if args.json {
                let items: Vec<Value> = categories.iter().map(category_json).collect();
                print_json(&json!({
                    "versao_relatorio": 1,
                    "org_id": org.to_string(),
                    "total": items.len(),
                    "categories": items,
                }));
            } else if categories.is_empty() {
                println!("Nenhuma categoria na org.");
            } else {
                for category in &categories {
                    println!(
                        "{}  {:<9} {}",
                        category.id,
                        status_in_portuguese(category.status),
                        category.name
                    );
                }
                println!("\n{} categoria(s).", categories.len());
            }
            ExitCode::SUCCESS
        }
        Err(error) => failure(describe(&error)),
    }
}

fn mostrar(store: &Store, org: OrgId, args: CategoryReference) -> ExitCode {
    let category = match load(store, org, &args.category) {
        Ok(category) => category,
        Err(error) => return failure(error),
    };
    saida(&category, args.json, "")
}

fn atualizar(store: &Store, org: OrgId, args: AtualizarCategoria) -> ExitCode {
    let id: ProductCategoryId = match parse_id(&args.category) {
        Ok(id) => id,
        Err(error) => return failure(error),
    };
    let nome = match args
        .name
        .as_deref()
        .map(|nome| validate_text("nome", nome, 120))
        .transpose()
    {
        Ok(nome) => nome,
        Err(error) => return failure(error),
    };
    let status = match args.status.as_deref().map(parse_status).transpose() {
        Ok(status) => status,
        Err(error) => return failure(error),
    };
    let descricao = args.description.map(|texto| texto.trim().to_owned());
    if nome.is_none() && descricao.is_none() && status.is_none() {
        return failure("nada a alterar: informe --name, --description ou --status");
    }
    let change = CategoryChange {
        name: nome,
        description: descricao,
        status,
    };
    match store.categories().update(org, id, args.if_match, change) {
        Ok(category) => saida(&category, args.json, "categoria alterada"),
        Err(error) => failure(describe(&error)),
    }
}

fn excluir(store: &Store, org: OrgId, args: DeleteCategory) -> ExitCode {
    let id: ProductCategoryId = match parse_id(&args.category) {
        Ok(id) => id,
        Err(error) => return failure(error),
    };
    if !args.force {
        return failure(
            "excluir a categoria apaga também os produtos dela e os vínculos com agentes. \
             Para confirmar, repita o comando com --force",
        );
    }
    if let Err(error) = store.categories().get(org, id) {
        return failure(absence_message(error, &args.category));
    }
    match store.categories().delete(org, id) {
        Ok(()) => {
            if args.json {
                print_json(&json!({
                    "versao_relatorio": 1,
                    "org_id": org.to_string(),
                    "removida": true,
                    "category_id": id.to_string(),
                }));
            } else {
                println!("categoria {id} excluída (e os produtos dela)");
            }
            ExitCode::SUCCESS
        }
        Err(error) => failure(describe(&error)),
    }
}

fn load(store: &Store, org: OrgId, texto: &str) -> Result<ProductCategory, String> {
    let id: ProductCategoryId = parse_id(texto)?;
    store
        .categories()
        .get(org, id)
        .map_err(|error| absence_message(error, texto))
}

fn absence_message(error: poph_store::StoreError, texto: &str) -> String {
    if error.is_not_found() {
        format!("não existe categoria `{texto}` nesta org")
    } else {
        describe(&error)
    }
}

fn saida(category: &ProductCategory, json: bool, mensagem: &str) -> ExitCode {
    if json {
        print_json(&json!({
            "versao_relatorio": 1,
            "org_id": category.org_id.to_string(),
            "category": category_json(category),
        }));
    } else {
        if !mensagem.is_empty() {
            println!("{mensagem}\n");
        }
        print!("{}", category_text(category));
    }
    ExitCode::SUCCESS
}

fn category_json(category: &ProductCategory) -> Value {
    json!({
        "id": category.id.to_string(),
        "org_id": category.org_id.to_string(),
        "name": category.name,
        "description": category.description,
        "status": category.status.as_text(),
        "version": category.version,
        "created_at": category.created_at,
        "updated_at": category.updated_at,
    })
}

fn category_text(category: &ProductCategory) -> String {
    format!(
        "categoria {}\n  nome: {}\n  descrição: {}\n  situação: {}\n  versão: {}\n",
        category.id,
        category.name,
        if category.description.is_empty() {
            "-"
        } else {
            &category.description
        },
        status_in_portuguese(category.status),
        category.version
    )
}
