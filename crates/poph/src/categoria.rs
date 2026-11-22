//! `poph category`: categorias de produto de uma org. HR-08.
//!
//! Excluir categoria apaga os produtos dela e os vínculos com agentes (cascata no banco). Como a
//! CLI ainda não tem como contar os produtos, a exclusão exige `--force` sempre.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Subcommand};
use poph_store::{CategoriaProduto, MudancaCategoria, NovaCategoria, Store};
use poph_types::{OrgId, ProductCategoryId};
use serde_json::{Value, json};

use crate::apoio::{
    abrir_com_org, descrever, falha, imprimir_json, parse_id, status_de_texto, status_em_portugues,
    validar_limite, validar_texto,
};
use crate::config;

/// Subcomandos de `poph category` (todos exigem `--org`).
#[derive(Debug, Subcommand)]
pub enum AcaoCategoria {
    /// Cria uma categoria de produto na org.
    Create(CriarCategoria),
    /// Lista as categorias da org (primeira página, por id).
    List(ListarCategorias),
    /// Mostra uma categoria.
    Show(ReferenciaCategoria),
    /// Altera uma categoria com `--if-match`.
    Update(AtualizarCategoria),
    /// Exclui a categoria (e os produtos dela). Exige `--force`.
    Delete(ExcluirCategoria),
}

/// `poph category create`.
#[derive(Debug, Args)]
pub struct CriarCategoria {
    /// Nome (até 120 caracteres; não pode repetir na org, sem diferenciar maiúsculas).
    #[arg(long)]
    pub name: String,

    /// Descrição curta.
    #[arg(long, default_value = "")]
    pub description: String,

    /// Saída em JSON (formato estável).
    #[arg(long)]
    pub json: bool,
}

/// `poph category list`.
#[derive(Debug, Args)]
pub struct ListarCategorias {
    /// Quantas categorias trazer (1 a 200).
    #[arg(long, default_value_t = 50)]
    pub limit: u32,

    /// Saída em JSON (formato estável).
    #[arg(long)]
    pub json: bool,
}

/// Referência a uma categoria pelo id (UUID v7).
#[derive(Debug, Args)]
pub struct ReferenciaCategoria {
    /// Id (UUID v7) da categoria.
    pub categoria: String,

    /// Saída em JSON (formato estável).
    #[arg(long)]
    pub json: bool,
}

/// `poph category update`.
#[derive(Debug, Args)]
pub struct AtualizarCategoria {
    /// Id (UUID v7) da categoria.
    pub categoria: String,

    /// Versão lida com `show` (`If-Match`). Obrigatória.
    #[arg(long)]
    pub if_match: u64,

    /// Novo nome.
    #[arg(long)]
    pub name: Option<String>,

    /// Nova descrição.
    #[arg(long)]
    pub description: Option<String>,

    /// Nova situação: `active`, `inactive` ou `archived`.
    #[arg(long)]
    pub status: Option<String>,

    /// Saída em JSON (formato estável).
    #[arg(long)]
    pub json: bool,
}

/// `poph category delete`.
#[derive(Debug, Args)]
pub struct ExcluirCategoria {
    /// Id (UUID v7) da categoria.
    pub categoria: String,

    /// Confirma: os produtos da categoria saem junto.
    #[arg(long)]
    pub force: bool,

    /// Saída em JSON (formato estável).
    #[arg(long)]
    pub json: bool,
}

/// Executa um subcomando de `poph category`.
pub fn executar(
    flag: Option<PathBuf>,
    ambiente: &config::Ambiente,
    offline: bool,
    org: Option<&str>,
    acao: AcaoCategoria,
) -> ExitCode {
    let (store, org) = match abrir_com_org(flag, ambiente, offline, org) {
        Ok(par) => par,
        Err(erro) => return falha(erro),
    };
    match acao {
        AcaoCategoria::Create(args) => criar(&store, org, args),
        AcaoCategoria::List(args) => listar(&store, org, args),
        AcaoCategoria::Show(args) => mostrar(&store, org, args),
        AcaoCategoria::Update(args) => atualizar(&store, org, args),
        AcaoCategoria::Delete(args) => excluir(&store, org, args),
    }
}

fn criar(store: &Store, org: OrgId, args: CriarCategoria) -> ExitCode {
    let nome = match validar_texto("nome", &args.name, 120) {
        Ok(nome) => nome,
        Err(erro) => return falha(erro),
    };
    let nova = NovaCategoria {
        nome,
        descricao: args.description.trim().to_owned(),
    };
    match store.categorias().criar(org, nova) {
        Ok(categoria) => saida(&categoria, args.json, "categoria criada"),
        Err(erro) => falha(descrever(&erro)),
    }
}

fn listar(store: &Store, org: OrgId, args: ListarCategorias) -> ExitCode {
    let pagina = match validar_limite(args.limit) {
        Ok(pagina) => pagina,
        Err(erro) => return falha(erro),
    };
    match store.categorias().listar(org, pagina) {
        Ok(categorias) => {
            if args.json {
                let itens: Vec<Value> = categorias.iter().map(json_categoria).collect();
                imprimir_json(&json!({
                    "versao_relatorio": 1,
                    "org_id": org.to_string(),
                    "total": itens.len(),
                    "categories": itens,
                }));
            } else if categorias.is_empty() {
                println!("Nenhuma categoria na org.");
            } else {
                for categoria in &categorias {
                    println!(
                        "{}  {:<9} {}",
                        categoria.id,
                        status_em_portugues(categoria.status),
                        categoria.nome
                    );
                }
                println!("\n{} categoria(s).", categorias.len());
            }
            ExitCode::SUCCESS
        }
        Err(erro) => falha(descrever(&erro)),
    }
}

fn mostrar(store: &Store, org: OrgId, args: ReferenciaCategoria) -> ExitCode {
    let categoria = match obter(store, org, &args.categoria) {
        Ok(categoria) => categoria,
        Err(erro) => return falha(erro),
    };
    saida(&categoria, args.json, "")
}

fn atualizar(store: &Store, org: OrgId, args: AtualizarCategoria) -> ExitCode {
    let id: ProductCategoryId = match parse_id(&args.categoria) {
        Ok(id) => id,
        Err(erro) => return falha(erro),
    };
    let nome = match args
        .name
        .as_deref()
        .map(|nome| validar_texto("nome", nome, 120))
        .transpose()
    {
        Ok(nome) => nome,
        Err(erro) => return falha(erro),
    };
    let status = match args.status.as_deref().map(status_de_texto).transpose() {
        Ok(status) => status,
        Err(erro) => return falha(erro),
    };
    let descricao = args.description.map(|texto| texto.trim().to_owned());
    if nome.is_none() && descricao.is_none() && status.is_none() {
        return falha("nada a alterar: informe --name, --description ou --status");
    }
    let mudanca = MudancaCategoria {
        nome,
        descricao,
        status,
    };
    match store
        .categorias()
        .atualizar(org, id, args.if_match, mudanca)
    {
        Ok(categoria) => saida(&categoria, args.json, "categoria alterada"),
        Err(erro) => falha(descrever(&erro)),
    }
}

fn excluir(store: &Store, org: OrgId, args: ExcluirCategoria) -> ExitCode {
    let id: ProductCategoryId = match parse_id(&args.categoria) {
        Ok(id) => id,
        Err(erro) => return falha(erro),
    };
    if !args.force {
        return falha(
            "excluir a categoria apaga também os produtos dela e os vínculos com agentes. \
             Para confirmar, repita o comando com --force",
        );
    }
    // Confere que existe nesta org antes de apagar, para a mensagem sair certa.
    if let Err(erro) = store.categorias().obter(org, id) {
        return falha(mensagem_de_ausencia(erro, &args.categoria));
    }
    match store.categorias().excluir(org, id) {
        Ok(()) => {
            if args.json {
                imprimir_json(&json!({
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
        Err(erro) => falha(descrever(&erro)),
    }
}

fn obter(store: &Store, org: OrgId, texto: &str) -> Result<CategoriaProduto, String> {
    let id: ProductCategoryId = parse_id(texto)?;
    store
        .categorias()
        .obter(org, id)
        .map_err(|erro| mensagem_de_ausencia(erro, texto))
}

fn mensagem_de_ausencia(erro: poph_store::ErroStore, texto: &str) -> String {
    if erro.eh_nao_encontrado() {
        format!("não existe categoria `{texto}` nesta org")
    } else {
        descrever(&erro)
    }
}

/// Saída de uma categoria: JSON ou texto.
fn saida(categoria: &CategoriaProduto, json: bool, mensagem: &str) -> ExitCode {
    if json {
        imprimir_json(&json!({
            "versao_relatorio": 1,
            "org_id": categoria.org_id.to_string(),
            "category": json_categoria(categoria),
        }));
    } else {
        if !mensagem.is_empty() {
            println!("{mensagem}\n");
        }
        print!("{}", texto_categoria(categoria));
    }
    ExitCode::SUCCESS
}

fn json_categoria(categoria: &CategoriaProduto) -> Value {
    json!({
        "id": categoria.id.to_string(),
        "org_id": categoria.org_id.to_string(),
        "name": categoria.nome,
        "description": categoria.descricao,
        "status": categoria.status.como_texto(),
        "version": categoria.versao,
        "created_at": categoria.criado_em,
        "updated_at": categoria.atualizado_em,
    })
}

fn texto_categoria(categoria: &CategoriaProduto) -> String {
    format!(
        "categoria {}\n  nome: {}\n  descrição: {}\n  situação: {}\n  versão: {}\n",
        categoria.id,
        categoria.nome,
        if categoria.descricao.is_empty() {
            "-"
        } else {
            &categoria.descricao
        },
        status_em_portugues(categoria.status),
        categoria.versao
    )
}
