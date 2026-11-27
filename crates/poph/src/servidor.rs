use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use clap::Args;
use poph_server::ServerState;
use poph_store::Store;
use poph_store::harness_target_version;

use crate::assembly;
use crate::config;

#[derive(Debug, Args)]
pub struct ServeArgs {
    #[arg(
        long = "somente-endereco",
        help = "Imprime o endereço e sai (útil para scripts de teste)"
    )]
    pub address_only: bool,
}

#[derive(Debug, Args)]
pub struct OpenapiArgs {
    #[arg(
        long,
        value_name = "ARQUIVO",
        help = "Grava o documento no arquivo (sem isso, imprime na saída padrão)"
    )]
    pub out: Option<PathBuf>,
}

pub fn run(flag: Option<PathBuf>, environment: &config::Environment, args: ServeArgs) -> ExitCode {
    let folder = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let file = config::resolve_file(flag.as_deref(), environment, &folder);
    let configuracao = match config::load(file.as_deref(), environment) {
        Ok(configuracao) => configuracao,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::from(1);
        }
    };
    if let Err(error) = crate::logs::iniciar(
        &configuracao.log,
        environment
            .get(crate::logs::VAR_RUST_LOG)
            .map(String::as_str),
    ) {
        eprintln!("{error}");
        return ExitCode::from(1);
    }
    if let Err(error) = check_migration(&configuracao) {
        eprintln!("{error}");
        return ExitCode::from(1);
    }
    let cluster_path = config::cluster_path(environment, file.as_deref(), &folder);
    let cluster_secret = config::cluster_secret(environment);
    let cluster = match poph_server::cluster::load(&cluster_path, cluster_secret.as_ref()) {
        Ok(config) => poph_server::cluster::LiveCluster {
            path: cluster_path.clone(),
            config,
            secret: cluster_secret,
        },
        Err(error) => {
            eprintln!("cluster: {error}");
            return ExitCode::from(1);
        }
    };
    tracing::info!(
        mode = cluster.mode(),
        file = %cluster.path.display(),
        instances = cluster.config.instances.len(),
        orgs_in_map = cluster.config.orgs.len(),
        fingerprint = %cluster.fingerprint(),
        "cluster"
    );
    let segredos = config::secrets(environment);
    let channels = match poph_server::evolution::channels_from_config(
        &configuracao,
        segredos.evolution.clone(),
        &|nome: &str| {
            environment
                .get(nome)
                .map(|value| value.trim().to_owned())
                .filter(|value| !value.is_empty())
        },
    ) {
        Ok(channels) => channels,
        Err(error) => {
            eprintln!("instâncias da Evolution: {error}");
            return ExitCode::from(1);
        }
    };

    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("não consegui iniciar o runtime do servidor: {error}");
            return ExitCode::from(1);
        }
    };

    runtime.block_on(async move {
        let store = match Store::open(&configuracao) {
            Ok(store) => Arc::new(store),
            Err(error) => {
                eprintln!("não consegui abrir o banco: {error}");
                return ExitCode::from(1);
            }
        };
        let estado = match ServerState::new(Arc::clone(&store), configuracao.clone()) {
            Ok(estado) => estado
                .with_evolution_channels(channels)
                .with_cluster(cluster),
            Err(error) => {
                eprintln!("{error}");
                return ExitCode::from(1);
            }
        };
        let estado = match wire_turn(estado, &configuracao, environment, &store).await {
            Ok(estado) => estado,
            Err(error) => {
                eprintln!("{error}");
                return ExitCode::from(1);
            }
        };
        let endereco =
            std::net::SocketAddr::new(configuracao.server.address, configuracao.server.port);
        let listener = match tokio::net::TcpListener::bind(endereco).await {
            Ok(listener) => listener,
            Err(error) => {
                eprintln!("não consegui abrir {endereco}: {error}");
                return ExitCode::from(1);
            }
        };

        if args.address_only {
            println!("{endereco}");
        } else {
            println!("POP Harness: servidor em http://{endereco}");
            println!("  API:      /admin/v1  (Bearer; OpenAPI em /admin/v1/openapi.json)");
            println!("  Saúde:    /healthz");
            println!("  Métricas: /metrics");
            println!("\nCtrl-C encerra.");
        }
        tracing::info!(%endereco, "servidor no ar");

        let desligamento = async {
            let _ = tokio::signal::ctrl_c().await;
            println!("\nencerrando o servidor…");
        };
        match poph_server::serve_at(listener, estado, desligamento).await {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("o servidor parou com erro: {error}");
                ExitCode::from(1)
            }
        }
    })
}

async fn wire_turn(
    estado: ServerState,
    configuracao: &poph_types::HarnessConfig,
    environment: &config::Environment,
    store: &Arc<Store>,
) -> Result<ServerState, String> {
    let segredos = config::secrets(environment);
    let key_name = &configuracao.llm.api_key_env;
    let Some(chave) = config::llm_key(environment, key_name) else {
        if !estado.evolution_channels.is_empty() {
            tracing::warn!(
                target: "poph_server::turno",
                %key_name,
                "turno desligado: chave do provedor não definida. As mensagens recebidas ficam na fila e são respondidas quando o servidor subir com a chave"
            );
        }
        return Ok(estado);
    };
    let metrics = assembly::Metrics::registered(estado.metrics.clone(), &estado.registry)?;
    let assembly = assembly::assemble(
        configuracao,
        Arc::clone(store),
        assembly::Keys {
            llm: chave,
            jev: segredos.typesafe.clone(),
        },
        metrics,
        assembly::ProcessFlows::from_server(&estado),
        environment,
    )?;

    for canal in estado.evolution_channels.iter() {
        let agente = match store.channel_integration(canal.org, canal.integration) {
            Ok(Some(integracao)) => integracao.agent_id,
            Ok(None) => continue,
            Err(error) => {
                eprintln!(
                    "[aviso] não consegui ler a integração de {}: {error}",
                    canal.instance
                );
                continue;
            }
        };
        if let Some(base) = &assembly.base
            && let Err(error) = base.synchronise_agent(canal.org, agente).await
        {
            eprintln!("[aviso] não consegui sincronizar o índice da base: {error}");
        }
        if let Some(catalog) = &assembly.catalog
            && let Err(error) = catalog.synchronise_agent(canal.org, agente).await
        {
            eprintln!("[aviso] não consegui sincronizar o índice de produtos: {error}");
        }
    }

    let interval = std::time::Duration::from_millis(configuracao.turn.dispatch_interval_ms);
    Ok(estado
        .with_engine(Arc::clone(&assembly.engine))
        .with_turn(assembly.flow, interval))
}

pub fn run_openapi(args: OpenapiArgs) -> ExitCode {
    let document = poph_server::openapi();
    let text = match serde_json::to_string_pretty(&document) {
        Ok(text) => text,
        Err(error) => {
            eprintln!("não consegui serializar o OpenAPI: {error}");
            return ExitCode::from(1);
        }
    };
    match args.out {
        Some(path) => {
            if let Some(folder) = path.parent()
                && !folder.as_os_str().is_empty()
                && let Err(error) = std::fs::create_dir_all(folder)
            {
                eprintln!("não consegui criar {}: {error}", folder.display());
                return ExitCode::from(1);
            }
            if let Err(error) = std::fs::write(&path, format!("{text}\n")) {
                eprintln!("não consegui gravar {}: {error}", path.display());
                return ExitCode::from(1);
            }
            println!("OpenAPI gravado em {}", path.display());
        }
        None => println!("{text}"),
    }
    ExitCode::SUCCESS
}

fn check_migration(configuracao: &poph_types::HarnessConfig) -> Result<(), String> {
    let target = harness_target_version();
    let path = configuracao.harness_db();
    if !path.exists() {
        return Ok(());
    }
    match poph_store::file_version(&path) {
        Ok(Some(versao)) if versao < target => Err(format!(
            "o banco {} está na versão {versao} e o código espera a {target}: rode `poph migrate` \
             com o servidor parado e suba de novo",
            path.display()
        )),
        Ok(_) => Ok(()),
        Err(error) => Err(format!("não consegui ler a versão do banco: {error}")),
    }
}
