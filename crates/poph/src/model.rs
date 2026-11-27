use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Subcommand};
use poph_rag::installation::{InstallReport, install};

use crate::config;

#[derive(Debug, Subcommand)]
pub enum ModelAction {
    #[command(about = "Baixa o modelo e o tokenizador para a pasta de modelos e confere o sha256")]
    Install(InstallArgs),
}

#[derive(Debug, Args)]
pub struct InstallArgs {
    #[arg(long, help = "Saída em JSON (formato estável, `versao_relatorio` = 1)")]
    json: bool,
}

pub fn run(
    flag: Option<PathBuf>,
    environment: &config::Environment,
    action: ModelAction,
) -> ExitCode {
    match action {
        ModelAction::Install(args) => run_install(flag, environment, args),
    }
}

fn run_install(
    flag: Option<PathBuf>,
    environment: &config::Environment,
    args: InstallArgs,
) -> ExitCode {
    let current_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let config_path = config::resolve_file(flag.as_deref(), environment, &current_dir);
    let config = match config::load(config_path.as_deref(), environment) {
        Ok(config) => config,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::from(1);
        }
    };

    let models_dir = config.models();
    match install(&models_dir) {
        Ok(report) => {
            if args.json {
                println!("{}", installation_json(&report));
            } else {
                print!("{}", installation_text(&report));
            }
            ExitCode::SUCCESS
        }
        Err(error) => {
            if args.json {
                let value = serde_json::json!({
                    "versao_relatorio": 1,
                    "erro": error.to_string(),
                });
                println!("{value}");
            }
            eprintln!("não consegui instalar o modelo: {error}");
            ExitCode::from(1)
        }
    }
}

fn installation_text(report: &InstallReport) -> String {
    let mut text = String::from("POP Harness: instalação do modelo de embedding\n\n");
    text.push_str(&format!("Pasta: {}\n", report.directory.display()));
    for file in &report.files {
        let status = match file.status {
            poph_rag::installation::InstallStatus::Downloaded => "baixado",
            poph_rag::installation::InstallStatus::AlreadyInstalled => "já estava instalado",
        };
        text.push_str(&format!(
            "  {}: {} ({:.1} MB, sha256 conferido)\n",
            file.name,
            status,
            file.bytes as f64 / 1_000_000.0
        ));
    }
    text.push_str("\nModelo pronto para uso.\n");
    text
}

fn installation_json(report: &InstallReport) -> String {
    let value = serde_json::json!({
        "versao_relatorio": 1,
        "diretorio": report.directory.display().to_string(),
        "arquivos": report.files.iter().map(|file| serde_json::json!({
            "nome": file.name,
            "bytes": file.bytes,
            "sha256": file.sha256,
            "situacao": file.status.as_str(),
        })).collect::<Vec<_>>(),
    });
    serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string())
}
