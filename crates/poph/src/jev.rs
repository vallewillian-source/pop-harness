//! `poph jev eval <gate> <arquivo>`: mede um gate do Jev sobre um conjunto rotulado (HR-28).
//!
//! Uso: `poph jev eval g1 tests/jev/g1.jsonl` (e `--json` para o relatório estável).
//!
//! O comando é a ferramenta de calibração de cada gate: imprime acerto, abstenção e latência, que
//! são os três números que o PRD §10 exige antes de ligar um gate por padrão. Sem `TYPESAFE_API_KEY`
//! no ambiente ele não inventa medição: falha com a instrução do que falta.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Subcommand};
use poph_jev::{ClienteJev, ConfigJev, IdGate, MotorJev, avaliar_conjunto, ler_conjunto};
use poph_types::BaseMetrics;

use crate::apoio;
use crate::config;

/// Subcomandos do `poph jev`.
#[derive(Debug, Subcommand)]
pub enum AcaoJev {
    /// Mede um gate sobre um conjunto rotulado (JSON Lines com `estado` e `esperado`).
    Eval(EvalArgs),
}

/// Opções do `poph jev eval`.
#[derive(Debug, Args)]
pub struct EvalArgs {
    /// Gate a medir: `g1` a `g9`.
    pub gate: String,

    /// Arquivo do conjunto rotulado (JSON Lines).
    pub arquivo: PathBuf,

    /// Saída em JSON (formato estável, com o detalhe por caso).
    #[arg(long)]
    pub json: bool,
}

/// Executa o `poph jev`.
pub fn executar(flag: Option<PathBuf>, ambiente: &config::Ambiente, acao: AcaoJev) -> ExitCode {
    match acao {
        AcaoJev::Eval(args) => match rodar_eval(flag, ambiente, args) {
            Ok(()) => ExitCode::SUCCESS,
            Err(mensagem) => apoio::falha(mensagem),
        },
    }
}

fn rodar_eval(
    flag: Option<PathBuf>,
    ambiente: &config::Ambiente,
    args: EvalArgs,
) -> Result<(), String> {
    let gate = IdGate::from_text(&args.gate).ok_or_else(|| {
        format!(
            "gate `{}` desconhecido: use g1, g2, g3, g4, g5, g6, g7, g8 ou g9",
            args.gate
        )
    })?;
    let casos = ler_conjunto(&args.arquivo).map_err(|erro| erro.to_string())?;

    let pasta = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let arquivo = config::resolver_arquivo(flag.as_deref(), ambiente, &pasta);
    let configuracao = config::carregar(arquivo.as_deref(), ambiente)
        .map_err(|erro| format!("configuração inválida: {erro}"))?;
    let chave = config::segredos(ambiente).typesafe.ok_or_else(|| {
        "defina TYPESAFE_API_KEY no ambiente desta sessão para medir o gate no Jev de verdade"
            .to_owned()
    })?;
    let cliente =
        ConfigJev::da_config(&configuracao.jev, chave).map_err(|erro| erro.to_string())?;
    let registro = poph_types::Registry::new();
    let metricas =
        BaseMetrics::register(&registro).map_err(|erro| format!("métricas base: {erro}"))?;
    let transporte = ClienteJev::novo(cliente, metricas).map_err(|erro| erro.to_string())?;
    let motor = MotorJev::novo(
        std::sync::Arc::new(transporte),
        configuracao.jev.gates,
        configuracao.jev.model.clone(),
    );

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|erro| format!("não consegui iniciar o runtime: {erro}"))?;
    let relatorio = runtime.block_on(avaliar_conjunto(&motor, gate, &casos));

    if args.json {
        let valor = serde_json::json!({
            "versao_relatorio": 1,
            "arquivo": args.arquivo.display().to_string(),
            "relatorio": relatorio,
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&valor).unwrap_or_default()
        );
    } else {
        print!("{}", relatorio.texto(&args.arquivo.display().to_string()));
    }
    Ok(())
}
