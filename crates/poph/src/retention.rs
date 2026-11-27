use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Subcommand};
use poph_retention::config::days_of_org;
use poph_retention::{
    ExecutionReport, HarnessDatabase, Options, OrgReport, Situacao, Warehouse, run_all, run_org,
};
use poph_store::Store;
use poph_types::HarnessConfig;
use serde_json::{Value, json};

use crate::config;
use crate::support::{failure, print_json, resolve_org};

const REPORT_VERSION: u64 = 1;

#[derive(Debug, Subcommand)]
pub enum RetentionAction {
    #[command(
        about = "Apaga o histórico vencido. Use `--dry-run` para ver o que sairia, sem apagar"
    )]
    Run(RunArgs),
    #[command(about = "Mostra quantos dias de histórico cada org guarda (`--org` para uma só)")]
    Show(ShowArgs),
}

#[derive(Debug, Args)]
pub struct RunArgs {
    #[arg(long, help = "Só conta e lista o que sairia. Não apaga nada")]
    dry_run: bool,
    #[arg(long, help = "Saída em JSON (formato estável, com `versao_relatorio`)")]
    json: bool,
}

#[derive(Debug, Args)]
pub struct ShowArgs {
    #[arg(long, help = "Saída em JSON (formato estável, com `versao_relatorio`)")]
    json: bool,
}

pub fn run(
    flag: Option<PathBuf>,
    environment: &config::Environment,
    org: Option<&str>,
    action: RetentionAction,
) -> ExitCode {
    let (store, harness_config) = match open_store(flag, environment) {
        Ok(pair) => pair,
        Err(error) => return failure(error),
    };
    match action {
        RetentionAction::Run(args) => run_sweep(&store, &harness_config, org, args),
        RetentionAction::Show(args) => show_settings(&store, &harness_config, org, args),
    }
}

fn open_store(
    flag: Option<PathBuf>,
    environment: &config::Environment,
) -> Result<(Store, HarnessConfig), String> {
    let current_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let config_path = config::resolve_file(flag.as_deref(), environment, &current_dir);
    let harness_config = config::load(config_path.as_deref(), environment)
        .map_err(|error| format!("configuração inválida: {error}"))?;
    let store = Store::open(&harness_config)
        .map_err(|error| format!("não consegui abrir o banco: {error}"))?;
    Ok((store, harness_config))
}

fn run_sweep(
    store: &Store,
    harness_config: &HarnessConfig,
    org: Option<&str>,
    args: RunArgs,
) -> ExitCode {
    let warehouse = HarnessDatabase::new(store, harness_config.blobs());
    let now = poph_store::system_now();
    let options = Options::new(now, harness_config.retention.days, args.dry_run, "cli");

    let result = match org {
        Some(org_ref) => resolve_org(store, org_ref)
            .and_then(|id| run_org(&warehouse, id, &options).map_err(|error| error.to_string()))
            .map(|report| ExecutionReport {
                now_ms: now,
                simulation: args.dry_run,
                orgs: vec![report],
            }),
        None => run_all(&warehouse, &options).map_err(|error| error.to_string()),
    };
    let report = match result {
        Ok(report) => report,
        Err(error) => return failure(error),
    };

    if args.json {
        print_json(&json_run(&report));
    } else {
        print_run(&report);
    }
    let has_failure = report
        .orgs
        .iter()
        .any(|org| org.situacao == Situacao::Failed);
    if has_failure {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

fn show_settings(
    store: &Store,
    harness_config: &HarnessConfig,
    org: Option<&str>,
    args: ShowArgs,
) -> ExitCode {
    let warehouse = HarnessDatabase::new(store, harness_config.blobs());
    let orgs = match org {
        Some(org_ref) => match resolve_org(store, org_ref).and_then(|id| {
            warehouse
                .org(id)
                .map(|org| vec![org])
                .map_err(|error| error.to_string())
        }) {
            Ok(orgs) => orgs,
            Err(error) => return failure(error),
        },
        None => match warehouse.orgs() {
            Ok(orgs) => orgs,
            Err(error) => return failure(error.to_string()),
        },
    };

    let default_days = harness_config.retention.days;
    let rows: Vec<Value> = orgs
        .iter()
        .map(|org| {
            let has_org_override = has_retention_override(&org.config_json);
            match days_of_org(&org.slug, &org.config_json, default_days) {
                Ok(days) => json!({
                    "org_id": org.id.to_string(),
                    "slug": org.slug,
                    "dias": days,
                    "origem": if has_org_override { "org" } else { "padrao" },
                    "erro": Value::Null,
                }),
                Err(error) => json!({
                    "org_id": org.id.to_string(),
                    "slug": org.slug,
                    "dias": Value::Null,
                    "origem": if has_org_override { "org" } else { "padrao" },
                    "erro": error.to_string(),
                }),
            }
        })
        .collect();
    let has_error = rows.iter().any(|row| !row["erro"].is_null());

    if args.json {
        print_json(&json!({
            "versao_relatorio": REPORT_VERSION,
            "padrao_dias": default_days,
            "orgs": rows,
        }));
    } else {
        print_show(default_days, &rows);
    }
    if has_error {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

fn has_retention_override(config_json: &str) -> bool {
    serde_json::from_str::<Value>(config_json)
        .ok()
        .and_then(|value| value.get("retencao")?.get("dias").cloned())
        .is_some()
}

fn json_run(report: &ExecutionReport) -> Value {
    let mut value = serde_json::to_value(report).unwrap_or_else(|_| json!({}));
    if let Some(object) = value.as_object_mut() {
        object.insert("versao_relatorio".to_owned(), json!(REPORT_VERSION));
    }
    value
}

fn print_run(report: &ExecutionReport) {
    if report.simulation {
        println!("POP Harness: retenção do histórico (simulação: nada será apagado)\n");
    } else {
        println!("POP Harness: retenção do histórico\n");
    }
    if report.orgs.is_empty() {
        println!("Nenhuma org para varrer.");
        return;
    }
    for org in &report.orgs {
        print_org(org, report.simulation);
    }
}

fn print_org(org: &OrgReport, simulation: bool) {
    let status_label = match org.situacao {
        Situacao::Swept if simulation => "simulada",
        Situacao::Swept => "varrida",
        Situacao::Skipped => "pulada",
        Situacao::Failed => "erro",
    };
    println!("Org {}: {status_label}", org.slug);
    if let Some(days) = org.dias {
        if days == 0 {
            println!("  Retenção: nunca apagar");
        } else {
            println!("  Retenção: {days} dias");
        }
    }
    if let Some(database) = &org.database {
        let history_label = if simulation {
            "Histórico que sairia"
        } else {
            "Histórico apagado"
        };
        println!(
            "  {history_label}: {} mensagens, {} anexos, {} turnos, {} entradas, {} saídas, {} conversas",
            database.messages,
            database.anexos,
            database.turns,
            database.inbox,
            database.outbox,
            database.conversas
        );
        if !database.conversation_ids.is_empty() {
            println!(
                "  Conversas afetadas: {}",
                database.conversation_ids.join(", ")
            );
        }
        let files = if simulation {
            format!("{} sairiam", org.orphan_files.len())
        } else {
            format!(
                "{} apagados de {} sem dono",
                org.deleted_files,
                org.orphan_files.len()
            )
        };
        println!("  Arquivos sem dono: {files}");
    }
    if let Some(reason) = &org.motivo {
        println!("  Motivo: {reason}");
    }
    println!();
}

fn print_show(default_days: u32, rows: &[Value]) {
    println!("POP Harness: retenção por org (padrão da instância: {default_days} dias)\n");
    if rows.is_empty() {
        println!("Nenhuma org cadastrada.");
        return;
    }
    for row in rows {
        let slug = row["slug"].as_str().unwrap_or("?");
        if let Some(error) = row["erro"].as_str() {
            println!("Org {slug}: configuração inválida. {error}");
            continue;
        }
        let days = row["dias"].as_u64().unwrap_or(0);
        let text = if days == 0 {
            "nunca apagar".to_owned()
        } else {
            format!("{days} dias")
        };
        let source = if row["origem"] == "org" {
            "definida na org"
        } else {
            "padrão da instância"
        };
        println!("Org {slug}: {text} ({source})");
    }
}
