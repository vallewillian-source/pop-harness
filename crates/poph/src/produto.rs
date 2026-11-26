//! `poph product`: catálogo de produtos da org (HR-18).
//!
//! Criar, listar, mostrar, alterar e excluir produto, mais `reindex`. Toda escrita que muda o que o
//! agente encontra na busca reindexa os agentes da org na hora — se o modelo de embedding estiver
//! instalado. Sem o modelo, a escrita vale e o comando avisa que o índice ficou pendente.
//!
//! Preço é campo do registro (`--price` em centavos, `--price-text` para o texto que o cliente vê) e
//! **não** entra no índice: mudar preço não pede reindexação.

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use clap::{Args, Subcommand};
use poph_agent::ProductCatalog;
use poph_agent::price_text;
use poph_store::{NewProduct, Page, Product, ProductChange, Store};
use poph_types::{AgentId, HarnessConfig, OrgId, ProductCategoryId, ProductId};
use serde_json::{Value, json};

use crate::apoio::{
    descrever, falha, imprimir_json, parse_id, status_de_texto, status_em_portugues,
    validar_limite, validar_texto,
};
use crate::config;

/// Subcomandos de `poph product` (todos exigem `--org`).
#[derive(Debug, Subcommand)]
pub enum AcaoProduto {
    /// Cria um produto na org.
    Create(CriarProduto),
    /// Lista os produtos da org (primeira página, por id).
    List(ListarProdutos),
    /// Mostra um produto.
    Show(ReferenciaProduto),
    /// Altera um produto com `--if-match`.
    Update(AtualizarProduto),
    /// Exclui o produto. Exige `--force`.
    Delete(ExcluirProduto),
    /// Reconstrói o índice de produtos de um agente (ou de todos os agentes da org).
    Reindex(ReindexProdutos),
}

/// `poph product create`.
#[derive(Debug, Args)]
pub struct CriarProduto {
    /// Nome comercial (até 160 caracteres).
    #[arg(long)]
    pub name: String,

    /// Categoria dona: id (UUID v7) ou nome exato.
    #[arg(long)]
    pub category: String,

    /// Preço em centavos (ex.: 32000 = R$ 320,00).
    #[arg(long, default_value_t = 0)]
    pub price: i64,

    /// Preço como o cliente vê (ex.: "R$ 320,00"). Vazio = formatado a partir dos centavos.
    #[arg(long, default_value = "")]
    pub price_text: String,

    /// Slug (vazio deriva do nome).
    #[arg(long)]
    pub slug: Option<String>,

    /// Outro nome pelo qual o cliente chama o produto (pode repetir).
    #[arg(long = "alias")]
    pub alias: Vec<String>,

    /// Etiqueta de busca (pode repetir).
    #[arg(long = "tag")]
    pub tag: Vec<String>,

    /// Descrição.
    #[arg(long, default_value = "")]
    pub description: String,

    /// Unidade ("por m²", "por visita").
    #[arg(long, default_value = "")]
    pub unit: String,

    /// O que está incluso.
    #[arg(long, default_value = "")]
    pub includes: String,

    /// O que não está incluso.
    #[arg(long, default_value = "")]
    pub excludes: String,

    /// Condições (prazo, pagamento).
    #[arg(long, default_value = "")]
    pub conditions: String,

    /// Saída em JSON (formato estável).
    #[arg(long)]
    pub json: bool,
}

/// `poph product list`.
#[derive(Debug, Args)]
pub struct ListarProdutos {
    /// Quantos produtos trazer (1 a 200).
    #[arg(long, default_value_t = 50)]
    pub limit: u32,

    /// Inclui inativos e arquivados.
    #[arg(long)]
    pub all: bool,

    /// Saída em JSON (formato estável).
    #[arg(long)]
    pub json: bool,
}

/// Referência a um produto (id UUID v7 ou slug).
#[derive(Debug, Args)]
pub struct ReferenciaProduto {
    /// Id (UUID v7) ou slug do produto.
    pub produto: String,

    /// Saída em JSON (formato estável).
    #[arg(long)]
    pub json: bool,
}

/// `poph product update`.
#[derive(Debug, Args)]
pub struct AtualizarProduto {
    /// Id (UUID v7) ou slug do produto.
    pub produto: String,

    /// Versão lida com `show` (`If-Match`). Obrigatória.
    #[arg(long)]
    pub if_match: u64,

    /// Nova categoria (id ou nome exato).
    #[arg(long)]
    pub category: Option<String>,

    /// Novo nome.
    #[arg(long)]
    pub name: Option<String>,

    /// Novo slug.
    #[arg(long)]
    pub slug: Option<String>,

    /// Novo preço em centavos.
    #[arg(long)]
    pub price: Option<i64>,

    /// Novo texto de preço.
    #[arg(long)]
    pub price_text: Option<String>,

    /// Novos aliases (substitui a lista).
    #[arg(long = "alias")]
    pub alias: Vec<String>,

    /// Novas tags (substitui a lista).
    #[arg(long = "tag")]
    pub tag: Vec<String>,

    /// Nova descrição.
    #[arg(long)]
    pub description: Option<String>,

    /// Nova unidade.
    #[arg(long)]
    pub unit: Option<String>,

    /// Novo inclui.
    #[arg(long)]
    pub includes: Option<String>,

    /// Novo exclui.
    #[arg(long)]
    pub excludes: Option<String>,

    /// Novas condições.
    #[arg(long)]
    pub conditions: Option<String>,

    /// Nova situação: `active`, `inactive` ou `archived` (inativo sai da busca, D-05).
    #[arg(long)]
    pub status: Option<String>,

    /// Saída em JSON (formato estável).
    #[arg(long)]
    pub json: bool,
}

/// `poph product delete`.
#[derive(Debug, Args)]
pub struct ExcluirProduto {
    /// Id (UUID v7) ou slug do produto.
    pub produto: String,

    /// Confirma a exclusão.
    #[arg(long)]
    pub force: bool,

    /// Saída em JSON (formato estável).
    #[arg(long)]
    pub json: bool,
}

/// `poph product reindex`.
#[derive(Debug, Args)]
pub struct ReindexProdutos {
    /// Só este agente (id ou slug). Sem ele, todos os agentes da org.
    #[arg(long)]
    pub agent: Option<String>,

    /// Saída em JSON (formato estável).
    #[arg(long)]
    pub json: bool,
}

/// Executa um subcomando de `poph product`.
pub fn executar(
    flag: Option<PathBuf>,
    ambiente: &config::Ambiente,
    offline: bool,
    org: Option<&str>,
    acao: AcaoProduto,
) -> ExitCode {
    if let Err(erro) = crate::apoio::exigir_offline(offline) {
        return falha(erro);
    }
    let (store, org, configuracao) = match abrir(flag, ambiente, org) {
        Ok(par) => par,
        Err(erro) => return falha(erro),
    };
    match acao {
        AcaoProduto::Create(args) => criar(&configuracao, &store, org, args),
        AcaoProduto::List(args) => listar(&store, org, args),
        AcaoProduto::Show(args) => mostrar(&store, org, args),
        AcaoProduto::Update(args) => atualizar(&configuracao, &store, org, args),
        AcaoProduto::Delete(args) => excluir(&configuracao, &store, org, args),
        AcaoProduto::Reindex(args) => reindexar(&configuracao, &store, org, args),
    }
}

fn abrir(
    flag: Option<PathBuf>,
    ambiente: &config::Ambiente,
    org: Option<&str>,
) -> Result<(Arc<Store>, OrgId, HarnessConfig), String> {
    let texto =
        org.ok_or_else(|| "informe --org com o id (UUID v7) ou o slug da org".to_owned())?;
    let pasta = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let arquivo = config::resolver_arquivo(flag.as_deref(), ambiente, &pasta);
    let configuracao =
        config::carregar(arquivo.as_deref(), ambiente).map_err(|erro| format!("{erro}"))?;
    let store = Arc::new(
        Store::open(&configuracao).map_err(|erro| format!("não consegui abrir o banco: {erro}"))?,
    );
    let org = crate::apoio::resolver_org(&store, texto)?;
    Ok((store, org, configuracao))
}

fn criar(
    configuracao: &HarnessConfig,
    store: &Arc<Store>,
    org: OrgId,
    args: CriarProduto,
) -> ExitCode {
    let nome = match validar_texto("nome", &args.name, 160) {
        Ok(nome) => nome,
        Err(erro) => return falha(erro),
    };
    let categoria = match resolver_categoria(store, org, &args.category) {
        Ok(categoria) => categoria,
        Err(erro) => return falha(erro),
    };
    let novo = NewProduct {
        category_id: Some(categoria),
        name: nome,
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
    match store.products().create(org, novo) {
        Ok(produto) => {
            aviso_de_indice(reindexar_org(configuracao, store, org));
            saida(&produto, args.json, "produto criado")
        }
        Err(erro) => falha(descrever(&erro)),
    }
}

fn listar(store: &Arc<Store>, org: OrgId, args: ListarProdutos) -> ExitCode {
    let pagina = match validar_limite(args.limit) {
        Ok(pagina) => pagina,
        Err(erro) => return falha(erro),
    };
    let resultado = if args.all {
        store.products().list(org, pagina)
    } else {
        store.products().list_active(org, pagina)
    };
    match resultado {
        Ok(produtos) => {
            if args.json {
                let itens: Vec<Value> = produtos.iter().map(json_produto).collect();
                imprimir_json(&json!({
                    "versao_relatorio": 1,
                    "org_id": org.to_string(),
                    "total": itens.len(),
                    "products": itens,
                }));
            } else if produtos.is_empty() {
                println!("Nenhum produto na org.");
            } else {
                for produto in &produtos {
                    println!(
                        "{}  {:<9} {:>12}  {}",
                        produto.id,
                        status_em_portugues(produto.status),
                        price_text(produto),
                        produto.name
                    );
                }
                println!("\n{} produto(s).", produtos.len());
            }
            ExitCode::SUCCESS
        }
        Err(erro) => falha(descrever(&erro)),
    }
}

fn mostrar(store: &Arc<Store>, org: OrgId, args: ReferenciaProduto) -> ExitCode {
    let produto = match obter(store, org, &args.produto) {
        Ok(produto) => produto,
        Err(erro) => return falha(erro),
    };
    saida(&produto, args.json, "")
}

fn atualizar(
    configuracao: &HarnessConfig,
    store: &Arc<Store>,
    org: OrgId,
    args: AtualizarProduto,
) -> ExitCode {
    let atual = match obter(store, org, &args.produto) {
        Ok(produto) => produto,
        Err(erro) => return falha(erro),
    };
    let categoria = match args.category.as_deref() {
        Some(texto) => match resolver_categoria(store, org, texto) {
            Ok(categoria) => Some(categoria),
            Err(erro) => return falha(erro),
        },
        None => None,
    };
    let status = match args.status.as_deref() {
        Some(texto) => match status_de_texto(texto) {
            Ok(status) => Some(status),
            Err(erro) => return falha(erro),
        },
        None => None,
    };
    let mudanca = ProductChange {
        category_id: categoria,
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
        .update(org, atual.id, args.if_match, mudanca)
    {
        Ok(produto) => {
            aviso_de_indice(reindexar_org(configuracao, store, org));
            saida(&produto, args.json, "produto alterado")
        }
        Err(erro) => falha(descrever(&erro)),
    }
}

fn excluir(
    configuracao: &HarnessConfig,
    store: &Arc<Store>,
    org: OrgId,
    args: ExcluirProduto,
) -> ExitCode {
    let produto = match obter(store, org, &args.produto) {
        Ok(produto) => produto,
        Err(erro) => return falha(erro),
    };
    // Conta antes de apagar: a exclusão faz os itens de carrinho ficarem sem produto (com o preço
    // gravado), e a recusa e a saída precisam dizer quantos foram.
    let refs = match store.products().references(org, produto.id) {
        Ok(refs) => refs,
        Err(erro) => return falha(descrever(&erro)),
    };
    if !args.force {
        return falha(if refs.cart_items > 0 {
            format!(
                "a exclusão apaga o produto do catálogo e deixa {} item(ns) de {} carrinho(s) como \"produto removido do catálogo\"; confirme com --force",
                refs.cart_items, refs.carts
            )
        } else {
            "a exclusão apaga o produto do catálogo; confirme com --force".to_owned()
        });
    }
    match store.products().delete(org, produto.id) {
        Ok(()) => {
            aviso_de_indice(reindexar_org(configuracao, store, org));
            if args.json {
                imprimir_json(&json!({
                    "versao_relatorio": 1,
                    "org_id": org.to_string(),
                    "deleted": produto.id.to_string(),
                    "name": produto.name,
                }));
            } else {
                println!("produto excluído: {} ({})", produto.name, produto.id);
                if refs.cart_items > 0 {
                    println!(
                        "{} item(ns) de {} carrinho(s) ficaram como \"produto removido do catálogo\" (o preço gravado continua)",
                        refs.cart_items, refs.carts
                    );
                }
            }
            ExitCode::SUCCESS
        }
        Err(erro) => falha(descrever(&erro)),
    }
}

fn reindexar(
    configuracao: &HarnessConfig,
    store: &Arc<Store>,
    org: OrgId,
    args: ReindexProdutos,
) -> ExitCode {
    let catalogo = match abrir_catalogo(configuracao, store) {
        Ok(catalogo) => catalogo,
        Err(erro) => return falha(erro),
    };
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(erro) => return falha(format!("não consegui iniciar o runtime: {erro}")),
    };
    let resultado = runtime.block_on(async {
        match args.agent.as_deref() {
            Some(texto) => {
                let agente = resolver_agente(store, org, texto)?;
                let relatorio = catalogo
                    .synchronise_agent(org, agente)
                    .await
                    .map_err(|erro| format!("{erro}"))?;
                Ok(vec![(agente, relatorio)])
            }
            None => catalogo
                .synchronise_org(org)
                .await
                .map_err(|erro| format!("{erro}")),
        }
    });
    match resultado {
        Ok(relatorios) => {
            if args.json {
                let itens: Vec<Value> = relatorios
                    .iter()
                    .map(|(agente, relatorio)| {
                        json!({
                            "agent_id": agente.to_string(),
                            "new": relatorio.new_items,
                            "changed": relatorio.changed_items,
                            "unchanged": relatorio.unchanged_items,
                            "removed": relatorio.removed_items,
                            "chunks_written": relatorio.stored_chunks,
                        })
                    })
                    .collect();
                imprimir_json(&json!({
                    "versao_relatorio": 1,
                    "org_id": org.to_string(),
                    "agents": itens,
                }));
            } else if relatorios.is_empty() {
                println!("Nenhum agente na org para indexar.");
            } else {
                for (agente, relatorio) in &relatorios {
                    println!(
                        "{}: novos {}, alterados {}, inalterados {}, removidos {}",
                        agente,
                        relatorio.new_items,
                        relatorio.changed_items,
                        relatorio.unchanged_items,
                        relatorio.removed_items
                    );
                }
            }
            ExitCode::SUCCESS
        }
        Err(erro) => falha(erro),
    }
}

/// Reindexa os agentes da org depois de uma escrita. Sem o modelo (ou sem pasta de dados), devolve
/// o aviso para o comando mostrar — a escrita já valeu.
fn reindexar_org(configuracao: &HarnessConfig, store: &Arc<Store>, org: OrgId) -> Option<String> {
    let catalogo = match abrir_catalogo(configuracao, store) {
        Ok(catalogo) => catalogo,
        Err(erro) => return Some(erro),
    };
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(erro) => return Some(format!("não consegui iniciar o runtime: {erro}")),
    };
    runtime
        .block_on(catalogo.synchronise_org(org))
        .err()
        .map(|erro| format!("{erro}"))
}

fn aviso_de_indice(erro: Option<String>) {
    if let Some(motivo) = erro {
        eprintln!(
            "[aviso] o índice de produtos não foi atualizado ({motivo}); a escrita valeu. \
             Rode `poph product reindex` quando o modelo estiver disponível."
        );
    }
}

fn abrir_catalogo(
    configuracao: &HarnessConfig,
    store: &Arc<Store>,
) -> Result<Arc<ProductCatalog>, String> {
    crate::rag::abrir_catalogo(configuracao, Arc::clone(store))
}

/// Acha o produto por id (UUID v7) ou slug, dentro da org.
fn obter(store: &Store, org: OrgId, texto: &str) -> Result<Product, String> {
    if uuid::Uuid::parse_str(texto).is_ok() {
        let id: ProductId = parse_id(texto)?;
        return store
            .products()
            .get(org, id)
            .map_err(|erro| descrever(&erro));
    }
    store
        .products()
        .by_slug(org, texto)
        .map_err(|erro| descrever(&erro))
}

/// Categoria pelo id (UUID v7) ou pelo nome exato (sem diferenciar maiúsculas).
fn resolver_categoria(store: &Store, org: OrgId, texto: &str) -> Result<ProductCategoryId, String> {
    let limpo = texto.trim();
    if limpo.is_empty() {
        return Err("informe --category com o id (UUID v7) ou o nome da categoria".to_owned());
    }
    if uuid::Uuid::parse_str(limpo).is_ok() {
        let id: ProductCategoryId = parse_id(limpo)?;
        return store
            .categories()
            .get(org, id)
            .map(|categoria| categoria.id)
            .map_err(|erro| descrever(&erro));
    }
    let mut pagina = Page::with_limit(200);
    loop {
        let lote = store
            .categories()
            .list(org, pagina)
            .map_err(|erro| descrever(&erro))?;
        if lote.is_empty() {
            return Err(format!("não existe categoria `{limpo}` na org"));
        }
        if let Some(categoria) = lote
            .iter()
            .find(|categoria| categoria.name.eq_ignore_ascii_case(limpo))
        {
            return Ok(categoria.id);
        }
        pagina.after = lote.last().map(|categoria| categoria.id.as_uuid());
    }
}

/// Agente pelo id (UUID v7) ou slug, dentro da org.
fn resolver_agente(store: &Store, org: OrgId, texto: &str) -> Result<AgentId, String> {
    if uuid::Uuid::parse_str(texto).is_ok() {
        let id: AgentId = parse_id(texto)?;
        return store
            .agents()
            .get(org, id)
            .map(|agente| agente.id)
            .map_err(|erro| descrever(&erro));
    }
    store
        .agents()
        .by_slug(org, texto)
        .map(|agente| agente.id)
        .map_err(|erro| descrever(&erro))
}

fn saida(produto: &Product, json_saida: bool, aviso: &str) -> ExitCode {
    if json_saida {
        imprimir_json(&json!({
            "versao_relatorio": 1,
            "product": json_produto(produto),
        }));
    } else {
        if !aviso.is_empty() {
            println!("{aviso}: {}", produto.name);
        }
        println!("{}  {}", produto.id, produto.name);
        println!("  slug: {}", produto.slug);
        println!("  categoria: {}", produto.category_id);
        println!(
            "  preço: {} ({} centavos)",
            price_text(produto),
            produto.price_cents
        );
        if !produto.unit.is_empty() {
            println!("  unidade: {}", produto.unit);
        }
        if !produto.conditions.is_empty() {
            println!("  condições: {}", produto.conditions);
        }
        println!(
            "  situação: {} · versão {}",
            status_em_portugues(produto.status),
            produto.version
        );
    }
    ExitCode::SUCCESS
}

fn json_produto(produto: &Product) -> Value {
    json!({
        "id": produto.id.to_string(),
        "category_id": produto.category_id.to_string(),
        "name": produto.name,
        "slug": produto.slug,
        "aliases": produto.aliases,
        "description": produto.description,
        "price_text": price_text(produto),
        "price_cents": produto.price_cents,
        "unit": produto.unit,
        "includes": produto.includes,
        "excludes": produto.exclui,
        "conditions": produto.conditions,
        "tags": produto.tags,
        "status": produto.status.as_text(),
        "version": produto.version,
    })
}
