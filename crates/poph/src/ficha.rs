use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Subcommand};
use poph_server::case_service::{case_json, detail};
use poph_store::{RecordFilter, Store};
use poph_types::{OrgId, RecordId, RecordStatus};
use serde_json::{Value, json};

use crate::config;
use crate::support::{
    describe, failure, open_with_configuration, parse_id, parse_page, print_json, resolve_agent,
    resolve_org,
};

const REPORT_VERSION: u64 = 1;

#[derive(Debug, Subcommand)]
pub enum CaseAction {
    #[command(about = "Lista as Fichas da org, com filtros (primeira página, em ordem de id)")]
    Listar(ListArgs),
    #[command(about = "Mostra uma Ficha: as execuções de fluxo, as etapas e os campos respondidos")]
    Mostrar(ShowArgs),
}

#[derive(Debug, Args)]
pub struct ListArgs {
    #[arg(long, help = "Situação: `open` ou `closed`")]
    pub status: Option<String>,

    #[arg(long, help = "Agente da Ficha: id (UUID v7) ou slug")]
    pub agente: Option<String>,

    #[arg(long, help = "Canal da conversa, por exemplo `whatsapp`")]
    pub canal: Option<String>,

    #[arg(
        long,
        help = "Só Fichas abertas a partir deste instante, em milissegundos desde 1970"
    )]
    pub desde: Option<i64>,

    #[arg(long, default_value_t = 50, help = "Quantas Fichas trazer (1 a 200)")]
    pub limit: u32,

    #[arg(long, help = "`next_cursor` da página anterior")]
    pub cursor: Option<String>,

    #[arg(long, help = "Saída em JSON (formato estável, com `versao_relatorio`)")]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct ShowArgs {
    #[arg(help = "Id (UUID v7) da Ficha")]
    pub ficha: String,

    #[arg(long, help = "Saída em JSON (formato estável, com `versao_relatorio`)")]
    pub json: bool,
}

pub fn run(
    flag: Option<PathBuf>,
    environment: &config::Environment,
    org: Option<&str>,
    acao: CaseAction,
) -> ExitCode {
    let resultado =
        open_with_configuration(flag, environment).and_then(|(store, _configuration)| {
            let org = org
                .ok_or_else(|| "informe --org com o id (UUID v7) ou o slug da org".to_owned())
                .and_then(|texto| resolve_org(&store, texto))?;
            match acao {
                CaseAction::Listar(args) => listar(&store, org, args),
                CaseAction::Mostrar(args) => mostrar(&store, org, args),
            }
        });
    match resultado {
        Ok(code) => code,
        Err(mensagem) => failure(mensagem),
    }
}

fn listar(store: &Store, org: OrgId, args: ListArgs) -> Result<ExitCode, String> {
    let page = parse_page(args.limit, args.cursor.as_deref())?;
    let filter = RecordFilter {
        status: args
            .status
            .as_deref()
            .map(RecordStatus::from_text)
            .transpose()
            .map_err(|error| error.to_string())?,
        agent: match args.agente.as_deref() {
            Some(texto) => Some(resolve_agent(store, org, texto)?.id),
            None => None,
        },
        canal: match args.canal.as_deref() {
            Some(texto) if texto.trim().is_empty() => {
                return Err("informe o canal, por exemplo whatsapp".to_owned());
            }
            Some(texto) => Some(texto.trim().to_owned()),
            None => None,
        },
        since: args.desde,
    };
    let fichas = store
        .cases()
        .list(org, &filter, page)
        .map_err(|error| describe(&error))?;
    let next_cursor = if fichas.len() < page.limit as usize {
        None
    } else {
        fichas.last().map(|ficha| ficha.id.to_string())
    };
    let items: Vec<Value> = fichas.iter().map(case_json).collect();
    if args.json {
        print_json(&json!({
            "versao_relatorio": REPORT_VERSION,
            "org_id": org.to_string(),
            "items": items,
            "next_cursor": next_cursor,
        }));
    } else if items.is_empty() {
        println!("Nenhuma Ficha com esses filtros.");
    } else {
        for item in &items {
            println!(
                "{}  {:<7} agente {}  canal {}  aberta em {} (ms desde 1970)",
                text_of(&item["id"]),
                text_of(&item["status"]),
                text_of(&item["agent_id"]),
                text_of(&item["canal"]),
                text_of(&item["aberta_em"]),
            );
        }
        if let Some(cursor) = &next_cursor {
            println!("\nPróxima página: --cursor {cursor}");
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn mostrar(store: &Store, org: OrgId, args: ShowArgs) -> Result<ExitCode, String> {
    let ficha_id: RecordId = parse_id(&args.ficha)?;
    let body = detail(store, org, ficha_id).map_err(|error| {
        if error.is_not_found() {
            format!("não existe Ficha `{}` nesta org", args.ficha)
        } else {
            describe(&error)
        }
    })?;
    if args.json {
        let mut saida = body;
        saida["versao_relatorio"] = json!(REPORT_VERSION);
        print_json(&saida);
        return Ok(ExitCode::SUCCESS);
    }
    println!("Ficha {}", text_of(&body["id"]));
    println!(
        "  situação: {}, agente {}, canal {}",
        text_of(&body["status"]),
        text_of(&body["agent_id"]),
        text_of(&body["canal"]),
    );
    let executions = body["execucoes"].as_array().cloned().unwrap_or_default();
    if executions.is_empty() {
        println!("  sem execuções de fluxo.");
    }
    for execution in &executions {
        println!(
            "\n  execução {} (fluxo {}): {}, no atual {}",
            text_of(&execution["id"]),
            text_of(&execution["fluxo_id"]),
            text_of(&execution["status"]),
            text_of(&execution["no_atual"]),
        );
        for etapa in execution["etapas"].as_array().cloned().unwrap_or_default() {
            let campos = etapa["dados"]
                .as_object()
                .map(|map| {
                    map.iter()
                        .map(|(nome, value)| format!("{nome}={}", text_of(value)))
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            println!(
                "    etapa {} ({}): campos [{campos}]",
                text_of(&etapa["no_id"]),
                text_of(&etapa["status"]),
            );
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn text_of(value: &Value) -> String {
    match value {
        Value::String(texto) => texto.clone(),
        Value::Null => "-".to_owned(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_of_strips_quotes_and_marks_empty() {
        assert_eq!(text_of(&json!("open")), "open");
        assert_eq!(text_of(&Value::Null), "-");
    }
}
