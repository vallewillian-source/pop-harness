//! `poph cluster`: mostra e confere o mapa de roteamento entre instâncias (HR-50a).
//!
//! O mapa é o `cluster.toml` (PRD §12), separado do `poph.toml` e igual em todas as instâncias.
//! Sem o arquivo, a instalação roda em **modo instância única** e os dois comandos dizem isso.
//!
//! - `poph cluster show` mostra o mapa efetivo: instância local, orgs por instância, impressão
//!   digital e se o segredo compartilhado está configurado (o valor nunca sai).
//! - `poph cluster check` valida e lista **todos** os problemas; código de saída 1 quando há algum.
//!
//! A recarga do processo vivo é HTTP (`POST /admin/v1/cluster/reload`, token global), não CLI: o
//! arquivo pode ser corrigido à mão, mas quem relê sem reiniciar é o servidor.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Args, Subcommand};
use poph_server::cluster::{ClusterVivo, ErroCluster, ResumoCluster};
use poph_types::Segredo;
use serde_json::json;

use crate::config;

/// Subcomandos de `poph cluster`.
#[derive(Debug, Subcommand)]
pub enum AcaoCluster {
    /// Mostra o mapa efetivo (instância local, orgs por instância e impressão digital).
    Show(MostrarCluster),
    /// Valida o `cluster.toml` e lista os problemas. Código de saída 1 quando há algum.
    Check(ConferirCluster),
}

/// `poph cluster show`.
#[derive(Debug, Args)]
pub struct MostrarCluster {
    /// Saída em JSON (formato estável, sem segredo).
    #[arg(long)]
    pub json: bool,
}

/// `poph cluster check`.
#[derive(Debug, Args)]
pub struct ConferirCluster {
    /// Saída em JSON (formato estável, sem segredo).
    #[arg(long)]
    pub json: bool,
    /// Consulta o `/healthz` de cada instância declarada e compara o mapa entre elas: aponta mapa
    /// divergente, instância inacessível e instância que se diz outra (HR-50b-2).
    #[arg(long)]
    pub remoto: bool,
}

/// Executa `poph cluster …`.
pub fn executar(flag: Option<PathBuf>, ambiente: &config::Ambiente, acao: AcaoCluster) -> ExitCode {
    let pasta = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let arquivo = config::resolver_arquivo(flag.as_deref(), ambiente, &pasta);
    let caminho = config::caminho_cluster(ambiente, arquivo.as_deref(), &pasta);
    let segredo = config::segredo_cluster(ambiente);
    match acao {
        AcaoCluster::Show(args) => mostrar(&caminho, segredo.as_ref(), args),
        AcaoCluster::Check(args) => conferir(&caminho, segredo.as_ref(), args),
    }
}

/// Lê o mapa e devolve o estado vivo; erro continua sendo erro para os dois comandos.
fn carregar_vivo(caminho: &Path, segredo: Option<&Segredo>) -> Result<ClusterVivo, ErroCluster> {
    let config = poph_server::cluster::carregar(caminho, segredo)?;
    Ok(ClusterVivo {
        caminho: caminho.to_path_buf(),
        config,
        segredo: segredo.cloned(),
    })
}

/// `poph cluster show`.
fn mostrar(caminho: &Path, segredo: Option<&Segredo>, args: MostrarCluster) -> ExitCode {
    match carregar_vivo(caminho, segredo) {
        Ok(vivo) => {
            if args.json {
                println!("{}", json_show(&vivo));
            } else {
                print!("{}", texto_show(&vivo));
            }
            ExitCode::SUCCESS
        }
        Err(erro) => {
            if args.json {
                println!("{}", json_check(caminho, None, &erro.problemas(), None));
            }
            eprintln!("{erro}");
            ExitCode::from(1)
        }
    }
}

/// `poph cluster check`.
fn conferir(caminho: &Path, segredo: Option<&Segredo>, args: ConferirCluster) -> ExitCode {
    match carregar_vivo(caminho, segredo) {
        Ok(vivo) => {
            let remoto = if args.remoto {
                match consultar_remoto(&vivo) {
                    Ok(diagnosticos) => diagnosticos,
                    Err(motivo) => {
                        eprintln!("{motivo}");
                        return ExitCode::from(1);
                    }
                }
            } else {
                Vec::new()
            };
            let problemas_remotos = problemas_do_remoto(&remoto);
            if args.json {
                println!(
                    "{}",
                    json_check(
                        caminho,
                        Some(&vivo),
                        &[],
                        args.remoto.then_some(remoto.as_slice()),
                    )
                );
            } else if vivo.declara_cluster() {
                println!("Mapa do cluster válido: {}", caminho.display());
                println!("  {} instância(s)", vivo.config.instances.len());
                println!("  fingerprint: {}", vivo.fingerprint());
                println!(
                    "  segredo compartilhado: {}",
                    if vivo.segredo.is_some() {
                        format!("definido ({})", config::VAR_CLUSTER_SECRET)
                    } else {
                        "não definido".to_owned()
                    }
                );
                if args.remoto {
                    print!("{}", texto_remoto(&vivo, &remoto));
                }
            } else {
                println!(
                    "Sem cluster.toml em {}: modo instância única (todas as orgs vivem nesta \
                     instância).",
                    caminho.display()
                );
                println!("Nada a corrigir.");
            }
            if problemas_remotos.is_empty() {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            }
        }
        Err(erro) => {
            if args.json {
                println!("{}", json_check(caminho, None, &erro.problemas(), None));
            } else {
                println!("Mapa do cluster com problema em {}:", caminho.display());
                for problema in erro.problemas() {
                    println!("  - {problema}");
                }
            }
            eprintln!("{erro}");
            ExitCode::from(1)
        }
    }
}

fn orgs_por_instancia(resumo: &ResumoCluster) -> BTreeMap<String, Vec<String>> {
    let mut por_instancia: BTreeMap<String, Vec<String>> = resumo
        .instancias
        .iter()
        .map(|instancia| (instancia.id.clone(), Vec::new()))
        .collect();
    for (org, destino) in &resumo.orgs {
        por_instancia
            .entry(destino.clone())
            .or_default()
            .push(org.clone());
    }
    for orgs in por_instancia.values_mut() {
        orgs.sort();
    }
    por_instancia
}

fn texto_show(vivo: &ClusterVivo) -> String {
    let resumo = vivo.resumo();
    let mut saida = String::from("POP Harness: cluster\n");
    if resumo.modo == "instancia_unica" {
        saida.push_str(
            "modo: instância única (sem cluster.toml: todas as orgs vivem nesta instância)\n",
        );
        saida.push_str(&format!(
            "arquivo: {} (não existe)\n",
            vivo.caminho.display()
        ));
    } else {
        saida.push_str(&format!(
            "modo: cluster ({} instância(s))\n",
            resumo.instancias.len()
        ));
        saida.push_str(&format!("arquivo: {}\n", vivo.caminho.display()));
        saida.push_str(&format!(
            "instância local: {}\n",
            if resumo.instance_id.is_empty() {
                "(não definida)"
            } else {
                &resumo.instance_id
            }
        ));
        match resumo.default_instance.as_deref() {
            Some(padrao) => saida.push_str(&format!("instância padrão: {padrao}\n")),
            None => saida.push_str(
                "instância padrão: a única instância declarada (não há default_instance)\n",
            ),
        }
        saida.push_str("instâncias:\n");
        for instancia in &resumo.instancias {
            saida.push_str(&format!(
                "  {}{}  {}\n",
                if instancia.local { "* " } else { "  " },
                instancia.id,
                instancia.url
            ));
        }
        saida.push_str("orgs por instância:\n");
        for (instancia, orgs) in orgs_por_instancia(&resumo) {
            if orgs.is_empty() {
                saida.push_str(&format!("  {instancia}: (nenhuma org no mapa)\n"));
            } else {
                saida.push_str(&format!("  {instancia}: {}\n", orgs.join(", ")));
            }
        }
        if !resumo.orgs.is_empty() && resumo.default_instance.is_some() {
            saida.push_str("  (orgs fora do mapa vão para a instância padrão)\n");
        }
    }
    saida.push_str(&format!(
        "segredo compartilhado: {}\n",
        if resumo.segredo_configurado {
            format!("definido ({})", config::VAR_CLUSTER_SECRET)
        } else {
            "não definido".to_owned()
        }
    ));
    saida.push_str(&format!("fingerprint: {}\n", vivo.fingerprint()));
    saida.push_str(&format!(
        "fingerprint do mapa (sem a instância local): {}\n",
        vivo.fingerprint_do_mapa()
    ));
    saida
}

fn json_show(vivo: &ClusterVivo) -> String {
    let valor = json!({
        "versao_relatorio": 1,
        "arquivo": vivo.caminho.display().to_string(),
        "fingerprint": vivo.fingerprint(),
        "fingerprint_mapa": vivo.fingerprint_do_mapa(),
        "mapa": vivo.resumo(),
    });
    serde_json::to_string_pretty(&valor).unwrap_or_else(|_| valor.to_string())
}

fn json_check(
    caminho: &Path,
    vivo: Option<&ClusterVivo>,
    problemas: &[String],
    remoto: Option<&[poph_server::cluster::DiagnosticoRemoto]>,
) -> String {
    let problemas_remotos = remoto.map(problemas_do_remoto).unwrap_or_default();
    let (ok, fingerprint, instancias, orgs) = match vivo {
        Some(vivo) if problemas.is_empty() => (
            problemas_remotos.is_empty(),
            Some(vivo.fingerprint()),
            vivo.config.instances.len(),
            vivo.config.orgs.len(),
        ),
        _ => (false, None, 0, 0),
    };
    let valor = json!({
        "versao_relatorio": 1,
        "ok": ok,
        "arquivo": caminho.display().to_string(),
        "fingerprint": fingerprint,
        "fingerprint_mapa": vivo.map(|vivo| vivo.fingerprint_do_mapa()),
        "instancias": instancias,
        "orgs_no_mapa": orgs,
        "problemas": problemas,
        "problemas_remotos": problemas_remotos,
        "remoto": remoto,
    });
    serde_json::to_string_pretty(&valor).unwrap_or_else(|_| valor.to_string())
}

/// Consulta o mapa das instâncias vizinhas (`--remoto`). Sem cluster, não há o que consultar.
fn consultar_remoto(
    vivo: &ClusterVivo,
) -> Result<Vec<poph_server::cluster::DiagnosticoRemoto>, String> {
    if !vivo.declara_cluster() || vivo.config.instances.len() <= 1 {
        return Ok(Vec::new());
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|erro| format!("não consegui iniciar a consulta remota: {erro}"))?;
    Ok(runtime.block_on(poph_server::cluster::conferir_remoto(&vivo.config)))
}

/// Problemas encontrados na consulta remota, um por linha (o que faz o código de saída ser 1).
fn problemas_do_remoto(remoto: &[poph_server::cluster::DiagnosticoRemoto]) -> Vec<String> {
    remoto
        .iter()
        .filter(|diagnostico| !diagnostico.ok())
        .map(|diagnostico| {
            let motivo = diagnostico
                .mensagem
                .clone()
                .unwrap_or_else(|| diagnostico.situacao.to_owned());
            format!("{} ({}): {motivo}", diagnostico.instancia, diagnostico.url)
        })
        .collect()
}

/// Relatório em texto da consulta remota.
fn texto_remoto(vivo: &ClusterVivo, remoto: &[poph_server::cluster::DiagnosticoRemoto]) -> String {
    let mut saida = format!(
        "consulta remota (mapa local: {}):\n",
        vivo.fingerprint_do_mapa()
    );
    if remoto.is_empty() {
        saida.push_str("  (nenhuma instância vizinha declarada)\n");
        return saida;
    }
    for diagnostico in remoto {
        saida.push_str(&diagnostico.linha());
        saida.push('\n');
    }
    let problemas = remoto.iter().filter(|d| !d.ok()).count();
    if problemas == 0 {
        saida.push_str("  todas as instâncias respondem com o mesmo mapa\n");
    } else {
        saida.push_str(&format!("  {problemas} instância(s) com problema\n"));
    }
    saida
}
