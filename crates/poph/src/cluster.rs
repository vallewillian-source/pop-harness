use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Args, Subcommand};
use poph_server::cluster::{ClusterError, ClusterSummary, LiveCluster};
use poph_types::Secret;
use serde_json::json;

use crate::config;

#[derive(Debug, Subcommand)]
pub enum ClusterAction {
    #[command(
        about = "Mostra o mapa efetivo (instância local, orgs por instância e impressão digital)"
    )]
    Show(ShowCluster),
    #[command(
        about = "Valida o `cluster.toml` e lista os problemas. Código de saída 1 quando há algum"
    )]
    Check(CheckCluster),
}

#[derive(Debug, Args)]
pub struct ShowCluster {
    #[arg(long, help = "Saída em JSON (formato estável, sem segredo)")]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct CheckCluster {
    #[arg(long, help = "Saída em JSON (formato estável, sem segredo)")]
    pub json: bool,
    #[arg(
        long = "remoto",
        help = "Consulta o `/healthz` de cada instância declarada e compara o mapa entre elas: aponta mapa divergente, instância inacessível e instância que se diz outra (HR-50b-2)"
    )]
    pub remote: bool,
}

pub fn run(
    flag: Option<PathBuf>,
    environment: &config::Environment,
    action: ClusterAction,
) -> ExitCode {
    let folder = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let file = config::resolve_file(flag.as_deref(), environment, &folder);
    let path = config::cluster_path(environment, file.as_deref(), &folder);
    let secret = config::cluster_secret(environment);
    match action {
        ClusterAction::Show(args) => show(&path, secret.as_ref(), args),
        ClusterAction::Check(args) => check(&path, secret.as_ref(), args),
    }
}

fn load_live(path: &Path, secret: Option<&Secret>) -> Result<LiveCluster, ClusterError> {
    let config = poph_server::cluster::load(path, secret)?;
    Ok(LiveCluster {
        path: path.to_path_buf(),
        config,
        secret: secret.cloned(),
    })
}

fn show(path: &Path, secret: Option<&Secret>, args: ShowCluster) -> ExitCode {
    match load_live(path, secret) {
        Ok(live) => {
            if args.json {
                println!("{}", show_json(&live));
            } else {
                print!("{}", show_text(&live));
            }
            ExitCode::SUCCESS
        }
        Err(error) => {
            if args.json {
                println!("{}", check_json(path, None, &error.problems(), None));
            }
            eprintln!("{error}");
            ExitCode::from(1)
        }
    }
}

fn check(path: &Path, secret: Option<&Secret>, args: CheckCluster) -> ExitCode {
    match load_live(path, secret) {
        Ok(live) => {
            let remoto = if args.remote {
                match query_remote(&live) {
                    Ok(diagnostics) => diagnostics,
                    Err(reason) => {
                        eprintln!("{reason}");
                        return ExitCode::from(1);
                    }
                }
            } else {
                Vec::new()
            };
            let remote_problems = collect_remote_problems(&remoto);
            if args.json {
                println!(
                    "{}",
                    check_json(
                        path,
                        Some(&live),
                        &[],
                        args.remote.then_some(remoto.as_slice()),
                    )
                );
            } else if live.declares_cluster() {
                println!("Mapa do cluster válido: {}", path.display());
                println!("  {} instância(s)", live.config.instances.len());
                println!("  fingerprint: {}", live.fingerprint());
                println!(
                    "  segredo compartilhado: {}",
                    if live.secret.is_some() {
                        format!("definido ({})", config::VAR_CLUSTER_SECRET)
                    } else {
                        "não definido".to_owned()
                    }
                );
                if args.remote {
                    print!("{}", remote_text(&live, &remoto));
                }
            } else {
                println!(
                    "Sem cluster.toml em {}: modo instância única (todas as orgs vivem nesta \
                     instância).",
                    path.display()
                );
                println!("Nada a corrigir.");
            }
            if remote_problems.is_empty() {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            }
        }
        Err(error) => {
            if args.json {
                println!("{}", check_json(path, None, &error.problems(), None));
            } else {
                println!("Mapa do cluster com problema em {}:", path.display());
                for problem in error.problems() {
                    println!("  - {problem}");
                }
            }
            eprintln!("{error}");
            ExitCode::from(1)
        }
    }
}

fn orgs_per_instance(summary: &ClusterSummary) -> BTreeMap<String, Vec<String>> {
    let mut per_instance: BTreeMap<String, Vec<String>> = summary
        .instances
        .iter()
        .map(|instance| (instance.id.clone(), Vec::new()))
        .collect();
    for (org, destination) in &summary.orgs {
        per_instance
            .entry(destination.clone())
            .or_default()
            .push(org.clone());
    }
    for orgs in per_instance.values_mut() {
        orgs.sort();
    }
    per_instance
}

fn show_text(live: &LiveCluster) -> String {
    let summary = live.summary();
    let mut output = String::from("POP Harness: cluster\n");
    if summary.mode == "instancia_unica" {
        output.push_str(
            "modo: instância única (sem cluster.toml: todas as orgs vivem nesta instância)\n",
        );
        output.push_str(&format!("arquivo: {} (não existe)\n", live.path.display()));
    } else {
        output.push_str(&format!(
            "modo: cluster ({} instância(s))\n",
            summary.instances.len()
        ));
        output.push_str(&format!("arquivo: {}\n", live.path.display()));
        output.push_str(&format!(
            "instância local: {}\n",
            if summary.instance_id.is_empty() {
                "(não definida)"
            } else {
                &summary.instance_id
            }
        ));
        match summary.default_instance.as_deref() {
            Some(default) => output.push_str(&format!("instância padrão: {default}\n")),
            None => output.push_str(
                "instância padrão: a única instância declarada (não há default_instance)\n",
            ),
        }
        output.push_str("instâncias:\n");
        for instance in &summary.instances {
            output.push_str(&format!(
                "  {}{}  {}\n",
                if instance.local { "* " } else { "  " },
                instance.id,
                instance.url
            ));
        }
        output.push_str("orgs por instância:\n");
        for (instance, orgs) in orgs_per_instance(&summary) {
            if orgs.is_empty() {
                output.push_str(&format!("  {instance}: (nenhuma org no mapa)\n"));
            } else {
                output.push_str(&format!("  {instance}: {}\n", orgs.join(", ")));
            }
        }
        if !summary.orgs.is_empty() && summary.default_instance.is_some() {
            output.push_str("  (orgs fora do mapa vão para a instância padrão)\n");
        }
    }
    output.push_str(&format!(
        "segredo compartilhado: {}\n",
        if summary.secret_configured {
            format!("definido ({})", config::VAR_CLUSTER_SECRET)
        } else {
            "não definido".to_owned()
        }
    ));
    output.push_str(&format!("fingerprint: {}\n", live.fingerprint()));
    output.push_str(&format!(
        "fingerprint do mapa (sem a instância local): {}\n",
        live.map_fingerprint()
    ));
    output
}

fn show_json(live: &LiveCluster) -> String {
    let value = json!({
        "versao_relatorio": 1,
        "arquivo": live.path.display().to_string(),
        "fingerprint": live.fingerprint(),
        "fingerprint_mapa": live.map_fingerprint(),
        "mapa": live.summary(),
    });
    serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string())
}

fn check_json(
    path: &Path,
    live: Option<&LiveCluster>,
    problems: &[String],
    remoto: Option<&[poph_server::cluster::RemoteDiagnostic]>,
) -> String {
    let remote_problems = remoto.map(collect_remote_problems).unwrap_or_default();
    let (ok, fingerprint, instances, orgs) = match live {
        Some(current) if problems.is_empty() => (
            remote_problems.is_empty(),
            Some(current.fingerprint()),
            current.config.instances.len(),
            current.config.orgs.len(),
        ),
        _ => (false, None, 0, 0),
    };
    let value = json!({
        "versao_relatorio": 1,
        "ok": ok,
        "arquivo": path.display().to_string(),
        "fingerprint": fingerprint,
        "fingerprint_mapa": live.map(|current| current.map_fingerprint()),
        "instancias": instances,
        "orgs_no_mapa": orgs,
        "problemas": problems,
        "problemas_remotos": remote_problems,
        "remoto": remoto,
    });
    serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string())
}

fn query_remote(live: &LiveCluster) -> Result<Vec<poph_server::cluster::RemoteDiagnostic>, String> {
    if !live.declares_cluster() || live.config.instances.len() <= 1 {
        return Ok(Vec::new());
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| format!("não consegui iniciar a consulta remota: {error}"))?;
    Ok(runtime.block_on(poph_server::cluster::check_remote_instances(&live.config)))
}

fn collect_remote_problems(remote: &[poph_server::cluster::RemoteDiagnostic]) -> Vec<String> {
    remote
        .iter()
        .filter(|diagnostic| !diagnostic.ok())
        .map(|diagnostic| {
            let reason = diagnostic
                .message
                .clone()
                .unwrap_or_else(|| diagnostic.status.to_owned());
            format!("{} ({}): {reason}", diagnostic.instance, diagnostic.url)
        })
        .collect()
}

fn remote_text(live: &LiveCluster, remote: &[poph_server::cluster::RemoteDiagnostic]) -> String {
    let mut output = format!(
        "consulta remota (mapa local: {}):\n",
        live.map_fingerprint()
    );
    if remote.is_empty() {
        output.push_str("  (nenhuma instância vizinha declarada)\n");
        return output;
    }
    for diagnostic in remote {
        output.push_str(&diagnostic.line());
        output.push('\n');
    }
    let problems = remote.iter().filter(|d| !d.ok()).count();
    if problems == 0 {
        output.push_str("  todas as instâncias respondem com o mesmo mapa\n");
    } else {
        output.push_str(&format!("  {problems} instância(s) com problema\n"));
    }
    output
}
