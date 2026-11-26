//! `poph retention` (HR-51): roda a retenção do histórico e mostra a configuração de cada org.
//!
//! - `poph retention run [--dry-run] [--json]`: apaga o que venceu em todas as orgs, ou só na de
//!   `--org`. Com `--dry-run`, nada é apagado: a saída diz o que sairia (e a auditoria fica
//!   marcada como simulação).
//! - `poph retention show [--json]`: mostra os dias de retenção de cada org (ou de `--org`).
//!
//! Saída: 0 quando todas as orgs terminaram bem (varridas ou puladas de propósito); 1 quando
//! alguma org deu erro; 2 uso incorreto (o clap decide).

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Subcommand};
use poph_retention::config::dias_da_org;
use poph_retention::{
    Armazem, BancoHarness, Opcoes, RelatorioExecucao, RelatorioOrg, Situacao, executar_org,
    executar_todas,
};
use poph_store::Store;
use poph_types::HarnessConfig;
use serde_json::{Value, json};

use crate::apoio::{falha, imprimir_json, resolver_org};
use crate::config;

/// Versão do formato JSON desta saída (muda só se a forma mudar).
const VERSAO_RELATORIO: u64 = 1;

/// Ações de `poph retention`.
#[derive(Debug, Subcommand)]
pub enum AcaoRetencao {
    /// Apaga o histórico vencido. Use `--dry-run` para ver o que sairia, sem apagar.
    Run(RunArgs),
    /// Mostra quantos dias de histórico cada org guarda (`--org` para uma só).
    Show(ShowArgs),
}

/// Opções de `run`.
#[derive(Debug, Args)]
pub struct RunArgs {
    /// Só conta e lista o que sairia. Não apaga nada.
    #[arg(long)]
    dry_run: bool,
    /// Saída em JSON (formato estável, com `versao_relatorio`).
    #[arg(long)]
    json: bool,
}

/// Opções de `show`.
#[derive(Debug, Args)]
pub struct ShowArgs {
    /// Saída em JSON (formato estável, com `versao_relatorio`).
    #[arg(long)]
    json: bool,
}

/// Executa `poph retention`.
pub fn executar(
    flag: Option<PathBuf>,
    ambiente: &config::Ambiente,
    org: Option<&str>,
    acao: AcaoRetencao,
) -> ExitCode {
    let (store, configuracao) = match abrir(flag, ambiente) {
        Ok(par) => par,
        Err(erro) => return falha(erro),
    };
    match acao {
        AcaoRetencao::Run(args) => executar_run(&store, &configuracao, org, args),
        AcaoRetencao::Show(args) => executar_show(&store, &configuracao, org, args),
    }
}

/// Abre a configuração e o banco (migrando se preciso), como os outros comandos.
fn abrir(
    flag: Option<PathBuf>,
    ambiente: &config::Ambiente,
) -> Result<(Store, HarnessConfig), String> {
    let pasta = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let arquivo = config::resolver_arquivo(flag.as_deref(), ambiente, &pasta);
    let configuracao = config::carregar(arquivo.as_deref(), ambiente)
        .map_err(|erro| format!("configuração inválida: {erro}"))?;
    let store = Store::abrir(&configuracao)
        .map_err(|erro| format!("não consegui abrir o banco: {erro}"))?;
    Ok((store, configuracao))
}

fn executar_run(
    store: &Store,
    configuracao: &HarnessConfig,
    org: Option<&str>,
    args: RunArgs,
) -> ExitCode {
    let armazem = BancoHarness::novo(store, configuracao.blobs());
    let agora = poph_store::agora_do_sistema();
    let opcoes = Opcoes::novas(agora, configuracao.retention.days, args.dry_run, "cli");

    let resultado = match org {
        Some(texto) => resolver_org(store, texto)
            .and_then(|id| executar_org(&armazem, id, &opcoes).map_err(|erro| erro.to_string()))
            .map(|relatorio| RelatorioExecucao {
                agora_ms: agora,
                simulacao: args.dry_run,
                orgs: vec![relatorio],
            }),
        None => executar_todas(&armazem, &opcoes).map_err(|erro| erro.to_string()),
    };
    let relatorio = match resultado {
        Ok(relatorio) => relatorio,
        Err(erro) => return falha(erro),
    };

    if args.json {
        imprimir_json(&json_run(&relatorio));
    } else {
        imprimir_run(&relatorio);
    }
    let com_erro = relatorio
        .orgs
        .iter()
        .any(|org| org.situacao == Situacao::Erro);
    if com_erro {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

fn executar_show(
    store: &Store,
    configuracao: &HarnessConfig,
    org: Option<&str>,
    args: ShowArgs,
) -> ExitCode {
    let armazem = BancoHarness::novo(store, configuracao.blobs());
    let orgs = match org {
        Some(texto) => match resolver_org(store, texto).and_then(|id| {
            armazem
                .org(id)
                .map(|org| vec![org])
                .map_err(|erro| erro.to_string())
        }) {
            Ok(orgs) => orgs,
            Err(erro) => return falha(erro),
        },
        None => match armazem.orgs() {
            Ok(orgs) => orgs,
            Err(erro) => return falha(erro.to_string()),
        },
    };

    let padrao = configuracao.retention.days;
    let linhas: Vec<Value> = orgs
        .iter()
        .map(|org| {
            let definido = configurada(&org.config_json);
            match dias_da_org(&org.slug, &org.config_json, padrao) {
                Ok(dias) => json!({
                    "org_id": org.id.to_string(),
                    "slug": org.slug,
                    "dias": dias,
                    "origem": if definido { "org" } else { "padrao" },
                    "erro": Value::Null,
                }),
                Err(erro) => json!({
                    "org_id": org.id.to_string(),
                    "slug": org.slug,
                    "dias": Value::Null,
                    "origem": if definido { "org" } else { "padrao" },
                    "erro": erro.to_string(),
                }),
            }
        })
        .collect();
    let com_erro = linhas.iter().any(|linha| !linha["erro"].is_null());

    if args.json {
        imprimir_json(&json!({
            "versao_relatorio": VERSAO_RELATORIO,
            "padrao_dias": padrao,
            "orgs": linhas,
        }));
    } else {
        imprimir_show(padrao, &linhas);
    }
    if com_erro {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

/// `true` quando a org define `retencao.dias` na própria configuração.
fn configurada(config_json: &str) -> bool {
    serde_json::from_str::<Value>(config_json)
        .ok()
        .and_then(|valor| valor.get("retencao")?.get("dias").cloned())
        .is_some()
}

fn json_run(relatorio: &RelatorioExecucao) -> Value {
    let mut valor = serde_json::to_value(relatorio).unwrap_or_else(|_| json!({}));
    if let Some(mapa) = valor.as_object_mut() {
        mapa.insert("versao_relatorio".to_owned(), json!(VERSAO_RELATORIO));
    }
    valor
}

fn imprimir_run(relatorio: &RelatorioExecucao) {
    if relatorio.simulacao {
        println!("POP Harness: retenção do histórico (simulação: nada será apagado)\n");
    } else {
        println!("POP Harness: retenção do histórico\n");
    }
    if relatorio.orgs.is_empty() {
        println!("Nenhuma org para varrer.");
        return;
    }
    for org in &relatorio.orgs {
        imprimir_org(org, relatorio.simulacao);
    }
}

fn imprimir_org(org: &RelatorioOrg, simulacao: bool) {
    let rotulo = match org.situacao {
        Situacao::Varrida if simulacao => "simulada",
        Situacao::Varrida => "varrida",
        Situacao::Ignorada => "pulada",
        Situacao::Erro => "erro",
    };
    println!("Org {}: {rotulo}", org.slug);
    if let Some(dias) = org.dias {
        if dias == 0 {
            println!("  Retenção: nunca apagar");
        } else {
            println!("  Retenção: {dias} dias");
        }
    }
    if let Some(banco) = &org.banco {
        let rotulo = if simulacao {
            "Histórico que sairia"
        } else {
            "Histórico apagado"
        };
        println!(
            "  {rotulo}: {} mensagens, {} anexos, {} turnos, {} entradas, {} saídas, {} conversas",
            banco.mensagens, banco.anexos, banco.turnos, banco.inbox, banco.outbox, banco.conversas
        );
        if !banco.ids_conversas.is_empty() {
            println!("  Conversas afetadas: {}", banco.ids_conversas.join(", "));
        }
        let arquivos = if simulacao {
            format!("{} sairiam", org.arquivos_sem_dono.len())
        } else {
            format!(
                "{} apagados de {} sem dono",
                org.arquivos_apagados,
                org.arquivos_sem_dono.len()
            )
        };
        println!("  Arquivos sem dono: {arquivos}");
    }
    if let Some(motivo) = &org.motivo {
        println!("  Motivo: {motivo}");
    }
    println!();
}

fn imprimir_show(padrao: u32, linhas: &[Value]) {
    println!("POP Harness: retenção por org (padrão da instância: {padrao} dias)\n");
    if linhas.is_empty() {
        println!("Nenhuma org cadastrada.");
        return;
    }
    for linha in linhas {
        let slug = linha["slug"].as_str().unwrap_or("?");
        if let Some(erro) = linha["erro"].as_str() {
            println!("Org {slug}: configuração inválida. {erro}");
            continue;
        }
        let dias = linha["dias"].as_u64().unwrap_or(0);
        let texto = if dias == 0 {
            "nunca apagar".to_owned()
        } else {
            format!("{dias} dias")
        };
        let origem = if linha["origem"] == "org" {
            "definida na org"
        } else {
            "padrão da instância"
        };
        println!("Org {slug}: {texto} ({origem})");
    }
}
