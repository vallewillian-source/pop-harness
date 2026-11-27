use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use clap::{Args, Subcommand};
use poph_agent::ProductCatalog;
use poph_agent::price_text;
use poph_store::{NewProduct, Page, Product, ProductChange, Store};
use poph_types::{AgentId, HarnessConfig, OrgId, ProductCategoryId, ProductId};
use serde_json::{Value, json};

use crate::config;
use crate::support::{
    describe, failure, parse_id, parse_status, print_json, status_in_portuguese, validate_limit,
    validate_text,
};

#[derive(Debug, Subcommand)]
pub enum ProductAction {
    #[command(about = "Cria um produto na org")]
    Create(CreateProduct),
    #[command(about = "Lista os produtos da org (primeira página, por id)")]
    List(ListProducts),
    #[command(about = "Mostra um produto")]
    Show(ProductReference),
    #[command(about = "Altera um produto com `--if-match`")]
    Update(UpdateProduct),
    #[command(about = "Exclui o produto. Exige `--force`")]
    Delete(DeleteProduct),
    #[command(
        about = "Reconstrói o índice de produtos de um agente (ou de todos os agentes da org)"
    )]
    Reindex(ReindexProducts),
}

#[derive(Debug, Args)]
pub struct CreateProduct {
    #[arg(long, help = "Nome comercial (até 160 caracteres)")]
    pub name: String,

    #[arg(long, help = "Categoria dona: id (UUID v7) ou nome exato")]
    pub category: String,

    #[arg(
        long,
        default_value_t = 0,
        help = "Preço em centavos (ex.: 32000 = R$ 320,00)"
    )]
    pub price: i64,

    #[arg(
        long,
        default_value = "",
        help = "Preço como o cliente vê (ex.: \"R$ 320,00\"). Vazio = formatado a partir dos centavos"
    )]
    pub price_text: String,

    #[arg(long, help = "Slug (vazio deriva do nome)")]
    pub slug: Option<String>,

    #[arg(
        long = "alias",
        help = "Outro nome pelo qual o cliente chama o produto (pode repetir)"
    )]
    pub alias: Vec<String>,

    #[arg(long = "tag", help = "Etiqueta de busca (pode repetir)")]
    pub tag: Vec<String>,

    #[arg(long, default_value = "", help = "Descrição")]
    pub description: String,

    #[arg(
        long,
        default_value = "",
        help = "Unidade (\"por m²\", \"por visita\")"
    )]
    pub unit: String,

    #[arg(long, default_value = "", help = "O que está incluso")]
    pub includes: String,

    #[arg(long, default_value = "", help = "O que não está incluso")]
    pub excludes: String,

    #[arg(long, default_value = "", help = "Condições (prazo, pagamento)")]
    pub conditions: String,

    #[arg(long, help = "Saída em JSON (formato estável)")]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct ListProducts {
    #[arg(long, default_value_t = 50, help = "Quantos produtos trazer (1 a 200)")]
    pub limit: u32,

    #[arg(long, help = "Inclui inativos e arquivados")]
    pub all: bool,

    #[arg(long, help = "Saída em JSON (formato estável)")]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct ProductReference {
    #[arg(
        id = "produto",
        value_name = "PRODUTO",
        help = "Id (UUID v7) ou slug do produto"
    )]
    pub product: String,

    #[arg(long, help = "Saída em JSON (formato estável)")]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct UpdateProduct {
    #[arg(
        id = "produto",
        value_name = "PRODUTO",
        help = "Id (UUID v7) ou slug do produto"
    )]
    pub product: String,

    #[arg(long, help = "Versão lida com `show` (`If-Match`). Obrigatória")]
    pub if_match: u64,

    #[arg(long, help = "Nova categoria (id ou nome exato)")]
    pub category: Option<String>,

    #[arg(long, help = "Novo nome")]
    pub name: Option<String>,

    #[arg(long, help = "Novo slug")]
    pub slug: Option<String>,

    #[arg(long, help = "Novo preço em centavos")]
    pub price: Option<i64>,

    #[arg(long, help = "Novo texto de preço")]
    pub price_text: Option<String>,

    #[arg(long = "alias", help = "Novos aliases (substitui a lista)")]
    pub alias: Vec<String>,

    #[arg(long = "tag", help = "Novas tags (substitui a lista)")]
    pub tag: Vec<String>,

    #[arg(long, help = "Nova descrição")]
    pub description: Option<String>,

    #[arg(long, help = "Nova unidade")]
    pub unit: Option<String>,

    #[arg(long, help = "Novo inclui")]
    pub includes: Option<String>,

    #[arg(long, help = "Novo exclui")]
    pub excludes: Option<String>,

    #[arg(long, help = "Novas condições")]
    pub conditions: Option<String>,

    #[arg(
        long,
        help = "Nova situação: `active`, `inactive` ou `archived` (inativo sai da busca, D-05)"
    )]
    pub status: Option<String>,

    #[arg(long, help = "Saída em JSON (formato estável)")]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct DeleteProduct {
    #[arg(
        id = "produto",
        value_name = "PRODUTO",
        help = "Id (UUID v7) ou slug do produto"
    )]
    pub product: String,

    #[arg(long, help = "Confirma a exclusão")]
    pub force: bool,

    #[arg(long, help = "Saída em JSON (formato estável)")]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct ReindexProducts {
    #[arg(
        long,
        help = "Só este agente (id ou slug). Sem ele, todos os agentes da org"
    )]
    pub agent: Option<String>,

    #[arg(long, help = "Saída em JSON (formato estável)")]
    pub json: bool,
}

pub fn run(
    flag: Option<PathBuf>,
    environment: &config::Environment,
    offline: bool,
    org: Option<&str>,
    action: ProductAction,
) -> ExitCode {
    if let Err(error) = crate::support::require_offline(offline) {
        return failure(error);
    }
    let (store, org, configuration) = match open(flag, environment, org) {
        Ok(pair) => pair,
        Err(error) => return failure(error),
    };
    match action {
        ProductAction::Create(args) => create(&configuration, &store, org, args),
        ProductAction::List(args) => list(&store, org, args),
        ProductAction::Show(args) => show(&store, org, args),
        ProductAction::Update(args) => update(&configuration, &store, org, args),
        ProductAction::Delete(args) => delete(&configuration, &store, org, args),
        ProductAction::Reindex(args) => reindex(&configuration, &store, org, args),
    }
}

fn open(
    flag: Option<PathBuf>,
    environment: &config::Environment,
    org: Option<&str>,
) -> Result<(Arc<Store>, OrgId, HarnessConfig), String> {
    let text = org.ok_or_else(|| "informe --org com o id (UUID v7) ou o slug da org".to_owned())?;
    let folder = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let file = config::resolve_file(flag.as_deref(), environment, &folder);
    let configuration =
        config::load(file.as_deref(), environment).map_err(|error| format!("{error}"))?;
    let store = Arc::new(
        Store::open(&configuration)
            .map_err(|error| format!("não consegui abrir o banco: {error}"))?,
    );
    let org = crate::support::resolve_org(&store, text)?;
    Ok((store, org, configuration))
}

fn create(
    configuration: &HarnessConfig,
    store: &Arc<Store>,
    org: OrgId,
    args: CreateProduct,
) -> ExitCode {
    let name = match validate_text("nome", &args.name, 160) {
        Ok(name) => name,
        Err(error) => return failure(error),
    };
    let category = match resolve_category(store, org, &args.category) {
        Ok(category) => category,
        Err(error) => return failure(error),
    };
    let new = NewProduct {
        category_id: Some(category),
        name,
        slug: args.slug,
        aliases: args.alias,
        description: args.description,
        price_text: args.price_text,
        price_cents: args.price,
        unit: args.unit,
        includes: args.includes,
        exclui: args.excludes,
        conditions: args.conditions,
        tags: args.tag,
    };
    match store.products().create(org, new) {
        Ok(product) => {
            print_index_notice(reindex_org(configuration, store, org));
            output(&product, args.json, "produto criado")
        }
        Err(error) => failure(describe(&error)),
    }
}

fn list(store: &Arc<Store>, org: OrgId, args: ListProducts) -> ExitCode {
    let page = match validate_limit(args.limit) {
        Ok(page) => page,
        Err(error) => return failure(error),
    };
    let result = if args.all {
        store.products().list(org, page)
    } else {
        store.products().list_active(org, page)
    };
    match result {
        Ok(products) => {
            if args.json {
                let items: Vec<Value> = products.iter().map(product_json).collect();
                print_json(&json!({
                    "versao_relatorio": 1,
                    "org_id": org.to_string(),
                    "total": items.len(),
                    "products": items,
                }));
            } else if products.is_empty() {
                println!("Nenhum produto na org.");
            } else {
                for product in &products {
                    println!(
                        "{}  {:<9} {:>12}  {}",
                        product.id,
                        status_in_portuguese(product.status),
                        price_text(product),
                        product.name
                    );
                }
                println!("\n{} produto(s).", products.len());
            }
            ExitCode::SUCCESS
        }
        Err(error) => failure(describe(&error)),
    }
}

fn show(store: &Arc<Store>, org: OrgId, args: ProductReference) -> ExitCode {
    let product = match get(store, org, &args.product) {
        Ok(product) => product,
        Err(error) => return failure(error),
    };
    output(&product, args.json, "")
}

fn update(
    configuration: &HarnessConfig,
    store: &Arc<Store>,
    org: OrgId,
    args: UpdateProduct,
) -> ExitCode {
    let current = match get(store, org, &args.product) {
        Ok(product) => product,
        Err(error) => return failure(error),
    };
    let category = match args.category.as_deref() {
        Some(text) => match resolve_category(store, org, text) {
            Ok(category) => Some(category),
            Err(error) => return failure(error),
        },
        None => None,
    };
    let status = match args.status.as_deref() {
        Some(text) => match parse_status(text) {
            Ok(status) => Some(status),
            Err(error) => return failure(error),
        },
        None => None,
    };
    let change = ProductChange {
        category_id: category,
        name: args.name,
        slug: args.slug,
        aliases: (!args.alias.is_empty()).then_some(args.alias),
        description: args.description,
        price_text: args.price_text,
        price_cents: args.price,
        unit: args.unit,
        includes: args.includes,
        exclui: args.excludes,
        conditions: args.conditions,
        tags: (!args.tag.is_empty()).then_some(args.tag),
        status,
    };
    match store
        .products()
        .update(org, current.id, args.if_match, change)
    {
        Ok(product) => {
            print_index_notice(reindex_org(configuration, store, org));
            output(&product, args.json, "produto alterado")
        }
        Err(error) => failure(describe(&error)),
    }
}

fn delete(
    configuration: &HarnessConfig,
    store: &Arc<Store>,
    org: OrgId,
    args: DeleteProduct,
) -> ExitCode {
    let product = match get(store, org, &args.product) {
        Ok(product) => product,
        Err(error) => return failure(error),
    };
    let refs = match store.products().references(org, product.id) {
        Ok(refs) => refs,
        Err(error) => return failure(describe(&error)),
    };
    if !args.force {
        return failure(if refs.cart_items > 0 {
            format!(
                "a exclusão apaga o produto do catálogo e deixa {} item(ns) de {} carrinho(s) como \"produto removido do catálogo\"; confirme com --force",
                refs.cart_items, refs.carts
            )
        } else {
            "a exclusão apaga o produto do catálogo; confirme com --force".to_owned()
        });
    }
    match store.products().delete(org, product.id) {
        Ok(()) => {
            print_index_notice(reindex_org(configuration, store, org));
            if args.json {
                print_json(&json!({
                    "versao_relatorio": 1,
                    "org_id": org.to_string(),
                    "deleted": product.id.to_string(),
                    "name": product.name,
                }));
            } else {
                println!("produto excluído: {} ({})", product.name, product.id);
                if refs.cart_items > 0 {
                    println!(
                        "{} item(ns) de {} carrinho(s) ficaram como \"produto removido do catálogo\" (o preço gravado continua)",
                        refs.cart_items, refs.carts
                    );
                }
            }
            ExitCode::SUCCESS
        }
        Err(error) => failure(describe(&error)),
    }
}

fn reindex(
    configuration: &HarnessConfig,
    store: &Arc<Store>,
    org: OrgId,
    args: ReindexProducts,
) -> ExitCode {
    let catalog = match open_catalog(configuration, store) {
        Ok(catalog) => catalog,
        Err(error) => return failure(error),
    };
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => return failure(format!("não consegui iniciar o runtime: {error}")),
    };
    let result = runtime.block_on(async {
        match args.agent.as_deref() {
            Some(text) => {
                let agent = resolve_agent(store, org, text)?;
                let report = catalog
                    .synchronise_agent(org, agent)
                    .await
                    .map_err(|error| format!("{error}"))?;
                Ok(vec![(agent, report)])
            }
            None => catalog
                .synchronise_org(org)
                .await
                .map_err(|error| format!("{error}")),
        }
    });
    match result {
        Ok(reports) => {
            if args.json {
                let items: Vec<Value> = reports
                    .iter()
                    .map(|(agent, report)| {
                        json!({
                            "agent_id": agent.to_string(),
                            "new": report.new_items,
                            "changed": report.changed_items,
                            "unchanged": report.unchanged_items,
                            "removed": report.removed_items,
                            "chunks_written": report.stored_chunks,
                        })
                    })
                    .collect();
                print_json(&json!({
                    "versao_relatorio": 1,
                    "org_id": org.to_string(),
                    "agents": items,
                }));
            } else if reports.is_empty() {
                println!("Nenhum agente na org para indexar.");
            } else {
                for (agent, report) in &reports {
                    println!(
                        "{}: novos {}, alterados {}, inalterados {}, removidos {}",
                        agent,
                        report.new_items,
                        report.changed_items,
                        report.unchanged_items,
                        report.removed_items
                    );
                }
            }
            ExitCode::SUCCESS
        }
        Err(error) => failure(error),
    }
}

fn reindex_org(configuration: &HarnessConfig, store: &Arc<Store>, org: OrgId) -> Option<String> {
    let catalog = match open_catalog(configuration, store) {
        Ok(catalog) => catalog,
        Err(error) => return Some(error),
    };
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => return Some(format!("não consegui iniciar o runtime: {error}")),
    };
    runtime
        .block_on(catalog.synchronise_org(org))
        .err()
        .map(|error| format!("{error}"))
}

fn print_index_notice(error: Option<String>) {
    if let Some(reason) = error {
        eprintln!(
            "[aviso] o índice de produtos não foi atualizado ({reason}); a escrita valeu. \
             Rode `poph product reindex` quando o modelo estiver disponível."
        );
    }
}

fn open_catalog(
    configuration: &HarnessConfig,
    store: &Arc<Store>,
) -> Result<Arc<ProductCatalog>, String> {
    crate::rag::open_catalog(configuration, Arc::clone(store))
}

fn get(store: &Store, org: OrgId, text: &str) -> Result<Product, String> {
    if uuid::Uuid::parse_str(text).is_ok() {
        let id: ProductId = parse_id(text)?;
        return store
            .products()
            .get(org, id)
            .map_err(|error| describe(&error));
    }
    store
        .products()
        .by_slug(org, text)
        .map_err(|error| describe(&error))
}

fn resolve_category(store: &Store, org: OrgId, text: &str) -> Result<ProductCategoryId, String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err("informe --category com o id (UUID v7) ou o nome da categoria".to_owned());
    }
    if uuid::Uuid::parse_str(trimmed).is_ok() {
        let id: ProductCategoryId = parse_id(trimmed)?;
        return store
            .categories()
            .get(org, id)
            .map(|category| category.id)
            .map_err(|error| describe(&error));
    }
    let mut page = Page::with_limit(200);
    loop {
        let batch = store
            .categories()
            .list(org, page)
            .map_err(|error| describe(&error))?;
        if batch.is_empty() {
            return Err(format!("não existe categoria `{trimmed}` na org"));
        }
        if let Some(category) = batch
            .iter()
            .find(|category| category.name.eq_ignore_ascii_case(trimmed))
        {
            return Ok(category.id);
        }
        page.after = batch.last().map(|category| category.id.as_uuid());
    }
}

fn resolve_agent(store: &Store, org: OrgId, text: &str) -> Result<AgentId, String> {
    if uuid::Uuid::parse_str(text).is_ok() {
        let id: AgentId = parse_id(text)?;
        return store
            .agents()
            .get(org, id)
            .map(|agent| agent.id)
            .map_err(|error| describe(&error));
    }
    store
        .agents()
        .by_slug(org, text)
        .map(|agent| agent.id)
        .map_err(|error| describe(&error))
}

fn output(product: &Product, json_output: bool, notice: &str) -> ExitCode {
    if json_output {
        print_json(&json!({
            "versao_relatorio": 1,
            "product": product_json(product),
        }));
    } else {
        if !notice.is_empty() {
            println!("{notice}: {}", product.name);
        }
        println!("{}  {}", product.id, product.name);
        println!("  slug: {}", product.slug);
        println!("  categoria: {}", product.category_id);
        println!(
            "  preço: {} ({} centavos)",
            price_text(product),
            product.price_cents
        );
        if !product.unit.is_empty() {
            println!("  unidade: {}", product.unit);
        }
        if !product.conditions.is_empty() {
            println!("  condições: {}", product.conditions);
        }
        println!(
            "  situação: {} · versão {}",
            status_in_portuguese(product.status),
            product.version
        );
    }
    ExitCode::SUCCESS
}

fn product_json(product: &Product) -> Value {
    json!({
        "id": product.id.to_string(),
        "category_id": product.category_id.to_string(),
        "name": product.name,
        "slug": product.slug,
        "aliases": product.aliases,
        "description": product.description,
        "price_text": price_text(product),
        "price_cents": product.price_cents,
        "unit": product.unit,
        "includes": product.includes,
        "excludes": product.exclui,
        "conditions": product.conditions,
        "tags": product.tags,
        "status": product.status.as_text(),
        "version": product.version,
    })
}
