//! `poph department`: departamentos de uma org (destino das conversas e das transferências). HR-08.
//!
//! Excluir departamento não apaga conversas nem agentes: os campos que apontavam para ele ficam
//! vazios (o agente perde o departamento padrão e o departamento de transferência).

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Subcommand};
use poph_store::{Departamento, DepartmentChange, NewDepartment, Store};
use poph_types::{DepartmentId, OrgId};
use serde_json::{Value, json};

use crate::config;
use crate::support::{
    abrir_com_org, describe, failure, parse_id, parse_status, print_json, status_in_portuguese,
    validate_limit, validate_text,
};

/// Subcomandos de `poph department` (todos exigem `--org`).
#[derive(Debug, Subcommand)]
pub enum AcaoDepartamento {
    /// Cria um departamento na org.
    Create(CriarDepartamento),
    /// Lista os departamentos da org (primeira página, por id).
    List(ListarDepartamentos),
    /// Mostra um departamento.
    Show(ReferenciaDepartamento),
    /// Altera um departamento com `--if-match`.
    Update(AtualizarDepartamento),
    /// Exclui o departamento (agentes e conversas que o usavam ficam sem ele).
    Delete(ReferenciaDepartamento),
}

/// `poph department create`.
#[derive(Debug, Args)]
pub struct CriarDepartamento {
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

/// `poph department list`.
#[derive(Debug, Args)]
pub struct ListarDepartamentos {
    /// Quantos departamentos trazer (1 a 200).
    #[arg(long, default_value_t = 50)]
    pub limit: u32,

    /// Saída em JSON (formato estável).
    #[arg(long)]
    pub json: bool,
}

/// Referência a um departamento pelo id (UUID v7).
#[derive(Debug, Args)]
pub struct ReferenciaDepartamento {
    /// Id (UUID v7) do departamento.
    pub departamento: String,

    /// Saída em JSON (formato estável).
    #[arg(long)]
    pub json: bool,
}

/// `poph department update`.
#[derive(Debug, Args)]
pub struct AtualizarDepartamento {
    /// Id (UUID v7) do departamento.
    pub departamento: String,

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

/// Executa um subcomando de `poph department`.
pub fn executar(
    flag: Option<PathBuf>,
    ambiente: &config::Environment,
    offline: bool,
    org: Option<&str>,
    acao: AcaoDepartamento,
) -> ExitCode {
    let (store, org) = match abrir_com_org(flag, ambiente, offline, org) {
        Ok(par) => par,
        Err(erro) => return failure(erro),
    };
    match acao {
        AcaoDepartamento::Create(args) => criar(&store, org, args),
        AcaoDepartamento::List(args) => listar(&store, org, args),
        AcaoDepartamento::Show(args) => mostrar(&store, org, args),
        AcaoDepartamento::Update(args) => atualizar(&store, org, args),
        AcaoDepartamento::Delete(args) => excluir(&store, org, args),
    }
}

fn criar(store: &Store, org: OrgId, args: CriarDepartamento) -> ExitCode {
    let nome = match validate_text("nome", &args.name, 120) {
        Ok(nome) => nome,
        Err(erro) => return failure(erro),
    };
    let novo = NewDepartment {
        name: nome,
        description: args.description.trim().to_owned(),
    };
    match store.departments().create(org, novo) {
        Ok(departamento) => saida(&departamento, args.json, "departamento criado"),
        Err(erro) => failure(describe(&erro)),
    }
}

fn listar(store: &Store, org: OrgId, args: ListarDepartamentos) -> ExitCode {
    let pagina = match validate_limit(args.limit) {
        Ok(pagina) => pagina,
        Err(erro) => return failure(erro),
    };
    match store.departments().list(org, pagina) {
        Ok(departamentos) => {
            if args.json {
                let itens: Vec<Value> = departamentos.iter().map(json_departamento).collect();
                print_json(&json!({
                    "versao_relatorio": 1,
                    "org_id": org.to_string(),
                    "total": itens.len(),
                    "departments": itens,
                }));
            } else if departamentos.is_empty() {
                println!("Nenhum departamento na org.");
            } else {
                for departamento in &departamentos {
                    println!(
                        "{}  {:<9} {}",
                        departamento.id,
                        status_in_portuguese(departamento.status),
                        departamento.name
                    );
                }
                println!("\n{} departamento(s).", departamentos.len());
            }
            ExitCode::SUCCESS
        }
        Err(erro) => failure(describe(&erro)),
    }
}

fn mostrar(store: &Store, org: OrgId, args: ReferenciaDepartamento) -> ExitCode {
    let departamento = match parse_id::<DepartmentId>(&args.departamento).and_then(|id| {
        store
            .departments()
            .get(org, id)
            .map_err(|erro| mensagem_de_ausencia(erro, &args.departamento))
    }) {
        Ok(departamento) => departamento,
        Err(erro) => return failure(erro),
    };
    saida(&departamento, args.json, "")
}

fn atualizar(store: &Store, org: OrgId, args: AtualizarDepartamento) -> ExitCode {
    let id: DepartmentId = match parse_id(&args.departamento) {
        Ok(id) => id,
        Err(erro) => return failure(erro),
    };
    let nome = match args
        .name
        .as_deref()
        .map(|nome| validate_text("nome", nome, 120))
        .transpose()
    {
        Ok(nome) => nome,
        Err(erro) => return failure(erro),
    };
    let status = match args.status.as_deref().map(parse_status).transpose() {
        Ok(status) => status,
        Err(erro) => return failure(erro),
    };
    let descricao = args.description.map(|texto| texto.trim().to_owned());
    if nome.is_none() && descricao.is_none() && status.is_none() {
        return failure("nada a alterar: informe --name, --description ou --status");
    }
    let mudanca = DepartmentChange {
        name: nome,
        description: descricao,
        status,
    };
    match store.departments().update(org, id, args.if_match, mudanca) {
        Ok(departamento) => saida(&departamento, args.json, "departamento alterado"),
        Err(erro) => failure(describe(&erro)),
    }
}

fn excluir(store: &Store, org: OrgId, args: ReferenciaDepartamento) -> ExitCode {
    let id: DepartmentId = match parse_id(&args.departamento) {
        Ok(id) => id,
        Err(erro) => return failure(erro),
    };
    // Confere que existe nesta org antes de apagar, para a mensagem sair certa.
    if let Err(erro) = store.departments().get(org, id) {
        return failure(mensagem_de_ausencia(erro, &args.departamento));
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
        Err(erro) => failure(describe(&erro)),
    }
}

fn mensagem_de_ausencia(erro: poph_store::StoreError, texto: &str) -> String {
    if erro.is_not_found() {
        format!("não existe departamento `{texto}` nesta org")
    } else {
        describe(&erro)
    }
}

/// Saída de um departamento: JSON ou texto.
fn saida(departamento: &Departamento, json: bool, mensagem: &str) -> ExitCode {
    if json {
        print_json(&json!({
            "versao_relatorio": 1,
            "org_id": departamento.org_id.to_string(),
            "department": json_departamento(departamento),
        }));
    } else {
        if !mensagem.is_empty() {
            println!("{mensagem}\n");
        }
        print!("{}", texto_departamento(departamento));
    }
    ExitCode::SUCCESS
}

fn json_departamento(departamento: &Departamento) -> Value {
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

fn texto_departamento(departamento: &Departamento) -> String {
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
