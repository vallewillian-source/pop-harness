use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Subcommand};
use poph_store::{Departamento, DepartmentChange, NewDepartment, Store};
use poph_types::{DepartmentId, OrgId};
use serde_json::{Value, json};

use crate::config;
use crate::support::{
    describe, failure, open_with_org, parse_id, parse_status, print_json, status_in_portuguese,
    validate_limit, validate_text,
};

#[derive(Debug, Subcommand)]
pub enum DepartmentAction {
    #[command(about = "Cria um departamento na org")]
    Create(CriarDepartamento),
    #[command(about = "Lista os departamentos da org (primeira página, por id)")]
    List(ListDepartments),
    #[command(about = "Mostra um departamento")]
    Show(DepartmentReference),
    #[command(about = "Altera um departamento com `--if-match`")]
    Update(AtualizarDepartamento),
    #[command(about = "Exclui o departamento (agentes e conversas que o usavam ficam sem ele)")]
    Delete(DepartmentReference),
}

#[derive(Debug, Args)]
pub struct CriarDepartamento {
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
pub struct ListDepartments {
    #[arg(
        long,
        default_value_t = 50,
        help = "Quantos departamentos trazer (1 a 200)"
    )]
    pub limit: u32,

    #[arg(long, help = "Saída em JSON (formato estável)")]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct DepartmentReference {
    #[arg(help = "Id (UUID v7) do departamento")]
    pub departamento: String,

    #[arg(long, help = "Saída em JSON (formato estável)")]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct AtualizarDepartamento {
    #[arg(help = "Id (UUID v7) do departamento")]
    pub departamento: String,

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

pub fn run(
    flag: Option<PathBuf>,
    environment: &config::Environment,
    offline: bool,
    org: Option<&str>,
    acao: DepartmentAction,
) -> ExitCode {
    let (store, org) = match open_with_org(flag, environment, offline, org) {
        Ok(pair) => pair,
        Err(error) => return failure(error),
    };
    match acao {
        DepartmentAction::Create(args) => criar(&store, org, args),
        DepartmentAction::List(args) => listar(&store, org, args),
        DepartmentAction::Show(args) => mostrar(&store, org, args),
        DepartmentAction::Update(args) => atualizar(&store, org, args),
        DepartmentAction::Delete(args) => excluir(&store, org, args),
    }
}

fn criar(store: &Store, org: OrgId, args: CriarDepartamento) -> ExitCode {
    let nome = match validate_text("nome", &args.name, 120) {
        Ok(nome) => nome,
        Err(error) => return failure(error),
    };
    let new_department = NewDepartment {
        name: nome,
        description: args.description.trim().to_owned(),
    };
    match store.departments().create(org, new_department) {
        Ok(departamento) => saida(&departamento, args.json, "departamento criado"),
        Err(error) => failure(describe(&error)),
    }
}

fn listar(store: &Store, org: OrgId, args: ListDepartments) -> ExitCode {
    let page = match validate_limit(args.limit) {
        Ok(page) => page,
        Err(error) => return failure(error),
    };
    match store.departments().list(org, page) {
        Ok(departments) => {
            if args.json {
                let items: Vec<Value> = departments.iter().map(json_department).collect();
                print_json(&json!({
                    "versao_relatorio": 1,
                    "org_id": org.to_string(),
                    "total": items.len(),
                    "departments": items,
                }));
            } else if departments.is_empty() {
                println!("Nenhum departamento na org.");
            } else {
                for departamento in &departments {
                    println!(
                        "{}  {:<9} {}",
                        departamento.id,
                        status_in_portuguese(departamento.status),
                        departamento.name
                    );
                }
                println!("\n{} departamento(s).", departments.len());
            }
            ExitCode::SUCCESS
        }
        Err(error) => failure(describe(&error)),
    }
}

fn mostrar(store: &Store, org: OrgId, args: DepartmentReference) -> ExitCode {
    let departamento = match parse_id::<DepartmentId>(&args.departamento).and_then(|id| {
        store
            .departments()
            .get(org, id)
            .map_err(|error| absence_message(error, &args.departamento))
    }) {
        Ok(departamento) => departamento,
        Err(error) => return failure(error),
    };
    saida(&departamento, args.json, "")
}

fn atualizar(store: &Store, org: OrgId, args: AtualizarDepartamento) -> ExitCode {
    let id: DepartmentId = match parse_id(&args.departamento) {
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
    let change = DepartmentChange {
        name: nome,
        description: descricao,
        status,
    };
    match store.departments().update(org, id, args.if_match, change) {
        Ok(departamento) => saida(&departamento, args.json, "departamento alterado"),
        Err(error) => failure(describe(&error)),
    }
}

fn excluir(store: &Store, org: OrgId, args: DepartmentReference) -> ExitCode {
    let id: DepartmentId = match parse_id(&args.departamento) {
        Ok(id) => id,
        Err(error) => return failure(error),
    };
    if let Err(error) = store.departments().get(org, id) {
        return failure(absence_message(error, &args.departamento));
    }
    match store.departments().delete(org, id) {
        Ok(()) => {
            if args.json {
                print_json(&json!({
                    "versao_relatorio": 1,
                    "org_id": org.to_string(),
                    "removido": true,
                    "department_id": id.to_string(),
                }));
            } else {
                println!(
                    "departamento {id} excluído (agentes e conversas que o usavam ficam sem ele)"
                );
            }
            ExitCode::SUCCESS
        }
        Err(error) => failure(describe(&error)),
    }
}

fn absence_message(error: poph_store::StoreError, texto: &str) -> String {
    if error.is_not_found() {
        format!("não existe departamento `{texto}` nesta org")
    } else {
        describe(&error)
    }
}

fn saida(departamento: &Departamento, json: bool, mensagem: &str) -> ExitCode {
    if json {
        print_json(&json!({
            "versao_relatorio": 1,
            "org_id": departamento.org_id.to_string(),
            "department": json_department(departamento),
        }));
    } else {
        if !mensagem.is_empty() {
            println!("{mensagem}\n");
        }
        print!("{}", text_department(departamento));
    }
    ExitCode::SUCCESS
}

fn json_department(departamento: &Departamento) -> Value {
    json!({
        "id": departamento.id.to_string(),
        "org_id": departamento.org_id.to_string(),
        "name": departamento.name,
        "description": departamento.description,
        "status": departamento.status.as_text(),
        "version": departamento.version,
        "created_at": departamento.created_at,
        "updated_at": departamento.updated_at,
    })
}

fn text_department(departamento: &Departamento) -> String {
    format!(
        "departamento {}\n  nome: {}\n  descrição: {}\n  situação: {}\n  versão: {}\n",
        departamento.id,
        departamento.name,
        if departamento.description.is_empty() {
            "-"
        } else {
            &departamento.description
        },
        status_in_portuguese(departamento.status),
        departamento.version
    )
}
