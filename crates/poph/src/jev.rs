use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Subcommand};
use poph_jev::{IdGate, JevClient, JevClientConfig, JevEngine, evaluate_dataset, read_dataset};
use poph_types::BaseMetrics;

use crate::config;
use crate::support;

#[derive(Debug, Subcommand)]
pub enum JevAction {
    #[command(
        about = "Mede um gate sobre um conjunto rotulado (JSON Lines com `estado` e `esperado`)"
    )]
    Eval(EvalArgs),
}

#[derive(Debug, Args)]
pub struct EvalArgs {
    #[arg(help = "Gate a medir: `g1` a `g9`")]
    pub gate: String,

    #[arg(
        value_name = "ARQUIVO",
        help = "Arquivo do conjunto rotulado (JSON Lines)"
    )]
    pub dataset: PathBuf,

    #[arg(long, help = "Saída em JSON (formato estável, com o detalhe por caso)")]
    pub json: bool,
}

pub fn run(
    flag: Option<PathBuf>,
    environment: &config::Environment,
    action: JevAction,
) -> ExitCode {
    match action {
        JevAction::Eval(args) => match run_eval(flag, environment, args) {
            Ok(()) => ExitCode::SUCCESS,
            Err(message) => support::failure(message),
        },
    }
}

fn run_eval(
    flag: Option<PathBuf>,
    environment: &config::Environment,
    args: EvalArgs,
) -> Result<(), String> {
    let gate = IdGate::from_text(&args.gate).ok_or_else(|| {
        format!(
            "gate `{}` desconhecido: use g1, g2, g3, g4, g5, g6, g7, g8 ou g9",
            args.gate
        )
    })?;
    let cases = read_dataset(&args.dataset).map_err(|error| error.to_string())?;

    let current_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let config_path = config::resolve_file(flag.as_deref(), environment, &current_dir);
    let harness_config = config::load(config_path.as_deref(), environment)
        .map_err(|error| format!("configuração inválida: {error}"))?;
    let api_key = config::secrets(environment).typesafe.ok_or_else(|| {
        "defina TYPESAFE_API_KEY no ambiente desta sessão para medir o gate no Jev de verdade"
            .to_owned()
    })?;
    let client_config = JevClientConfig::from_config(&harness_config.jev, api_key)
        .map_err(|error| error.to_string())?;
    let registry = poph_types::Registry::new();
    let metrics =
        BaseMetrics::register(&registry).map_err(|error| format!("métricas base: {error}"))?;
    let client = JevClient::new(client_config, metrics).map_err(|error| error.to_string())?;
    let engine = JevEngine::new(
        std::sync::Arc::new(client),
        harness_config.jev.gates,
        harness_config.jev.model.clone(),
    );

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| format!("não consegui iniciar o runtime: {error}"))?;
    let report = runtime.block_on(evaluate_dataset(&engine, gate, &cases));

    if args.json {
        let value = serde_json::json!({
            "versao_relatorio": 1,
            "arquivo": args.dataset.display().to_string(),
            "relatorio": report,
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&value).unwrap_or_default()
        );
    } else {
        print!("{}", report.texto(&args.dataset.display().to_string()));
    }
    Ok(())
}
