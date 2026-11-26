//! `poph model install`: baixa o modelo de embedding para a pasta de modelos e confere o sha256.
//!
//! É o único comando que baixa da rede. O turno nunca baixa em silêncio: se o modelo falta, o
//! erro manda rodar este comando. Arquivo que já confere não é baixado de novo.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Subcommand};
use poph_rag::instalacao::{RelatorioInstalacao, instalar};

use crate::config;

/// Ações sobre o modelo de embedding.
#[derive(Debug, Subcommand)]
pub enum AcaoModelo {
    /// Baixa o modelo e o tokenizador para a pasta de modelos e confere o sha256.
    Install(InstallArgs),
}

/// Opções do `model install`.
#[derive(Debug, Args)]
pub struct InstallArgs {
    /// Saída em JSON (formato estável, `versao_relatorio` = 1).
    #[arg(long)]
    json: bool,
}

/// Executa a ação escolhida. Saída: 0 quando o modelo está instalado; 1 com erro.
pub fn executar(flag: Option<PathBuf>, ambiente: &config::Ambiente, acao: AcaoModelo) -> ExitCode {
    match acao {
        AcaoModelo::Install(args) => executar_install(flag, ambiente, args),
    }
}

fn executar_install(
    flag: Option<PathBuf>,
    ambiente: &config::Ambiente,
    args: InstallArgs,
) -> ExitCode {
    let pasta = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let arquivo = config::resolver_arquivo(flag.as_deref(), ambiente, &pasta);
    let config = match config::carregar(arquivo.as_deref(), ambiente) {
        Ok(config) => config,
        Err(erro) => {
            eprintln!("{erro}");
            return ExitCode::from(1);
        }
    };

    let diretorio = config.models();
    match instalar(&diretorio) {
        Ok(relatorio) => {
            if args.json {
                println!("{}", json_instalacao(&relatorio));
            } else {
                print!("{}", texto_instalacao(&relatorio));
            }
            ExitCode::SUCCESS
        }
        Err(erro) => {
            if args.json {
                let valor = serde_json::json!({
                    "versao_relatorio": 1,
                    "erro": erro.to_string(),
                });
                println!("{valor}");
            }
            eprintln!("não consegui instalar o modelo: {erro}");
            ExitCode::from(1)
        }
    }
}

fn texto_instalacao(relatorio: &RelatorioInstalacao) -> String {
    let mut texto = String::from("POP Harness: instalação do modelo de embedding\n\n");
    texto.push_str(&format!("Pasta: {}\n", relatorio.diretorio.display()));
    for arquivo in &relatorio.arquivos {
        let situacao = match arquivo.situacao {
            poph_rag::instalacao::Situacao::Baixado => "baixado",
            poph_rag::instalacao::Situacao::JaEstava => "já estava instalado",
        };
        texto.push_str(&format!(
            "  {}: {} ({:.1} MB, sha256 conferido)\n",
            arquivo.nome,
            situacao,
            arquivo.bytes as f64 / 1_000_000.0
        ));
    }
    texto.push_str("\nModelo pronto para uso.\n");
    texto
}

fn json_instalacao(relatorio: &RelatorioInstalacao) -> String {
    let valor = serde_json::json!({
        "versao_relatorio": 1,
        "diretorio": relatorio.diretorio.display().to_string(),
        "arquivos": relatorio.arquivos.iter().map(|arquivo| serde_json::json!({
            "nome": arquivo.nome,
            "bytes": arquivo.bytes,
            "sha256": arquivo.sha256,
            "situacao": arquivo.situacao.como_texto(),
        })).collect::<Vec<_>>(),
    });
    serde_json::to_string_pretty(&valor).unwrap_or_else(|_| valor.to_string())
}
