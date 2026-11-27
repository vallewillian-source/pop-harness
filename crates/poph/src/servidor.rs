//! `poph serve`: servidor HTTP do Harness (HR-20) e `poph openapi` (documento para o Console).

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use clap::Args;
use poph_server::ServerState;
use poph_store::Store;
use poph_store::harness_target_version;

use crate::config;
use crate::montagem;

/// Opções do `poph serve`.
#[derive(Debug, Args)]
pub struct ServeArgs {
    /// Imprime o endereço e sai (útil para scripts de teste).
    #[arg(long)]
    pub somente_endereco: bool,
}

/// Opções do `poph openapi`.
#[derive(Debug, Args)]
pub struct OpenapiArgs {
    /// Grava o documento no arquivo (sem isso, imprime na saída padrão).
    #[arg(long, value_name = "ARQUIVO")]
    pub out: Option<PathBuf>,
}

/// Executa `poph serve`.
pub fn executar(
    flag: Option<PathBuf>,
    ambiente: &config::Environment,
    args: ServeArgs,
) -> ExitCode {
    let pasta = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let arquivo = config::resolve_file(flag.as_deref(), ambiente, &pasta);
    let configuracao = match config::load(arquivo.as_deref(), ambiente) {
        Ok(configuracao) => configuracao,
        Err(erro) => {
            eprintln!("{erro}");
            return ExitCode::from(1);
        }
    };
    // Logs do servidor (HR-26f): sem o assinante, a rota do WhatsApp e o laço de saída não deixam
    // rastro. Vai para o erro padrão; o nível vem de RUST_LOG ou de `log.nivel`.
    if let Err(erro) = crate::logs::iniciar(
        &configuracao.log,
        ambiente.get(crate::logs::VAR_RUST_LOG).map(String::as_str),
    ) {
        eprintln!("{erro}");
        return ExitCode::from(1);
    }
    if let Err(erro) = conferir_migracao(&configuracao) {
        eprintln!("{erro}");
        return ExitCode::from(1);
    }
    // Cluster (HR-50a): o mapa estático é lido na subida e pode ser relido por
    // `POST /admin/v1/cluster/reload`. Segredo só do ambiente; arquivo inválido impede subir — o
    // operador vê o problema agora, em vez de descobrir o roteamento errado no primeiro pedido.
    let caminho_cluster = config::cluster_path(ambiente, arquivo.as_deref(), &pasta);
    let segredo_cluster = config::cluster_secret(ambiente);
    let cluster = match poph_server::cluster::load(&caminho_cluster, segredo_cluster.as_ref()) {
        Ok(config) => poph_server::cluster::LiveCluster {
            path: caminho_cluster.clone(),
            config,
            secret: segredo_cluster,
        },
        Err(erro) => {
            eprintln!("cluster: {erro}");
            return ExitCode::from(1);
        }
    };
    tracing::info!(
        modo = cluster.mode(),
        arquivo = %cluster.path.display(),
        instancias = cluster.config.instances.len(),
        orgs_no_mapa = cluster.config.orgs.len(),
        fingerprint = %cluster.fingerprint(),
        "cluster"
    );
    // Instâncias da Evolution (HR-26b): segredos de webhook e a chave da Evolution vêm do ambiente,
    // nunca do arquivo. O valor de nenhum deles é impresso.
    let segredos = config::secrets(ambiente);
    let canais = match poph_server::evolution::channels_from_config(
        &configuracao,
        segredos.evolution.clone(),
        &|nome: &str| {
            ambiente
                .get(nome)
                .map(|valor| valor.trim().to_owned())
                .filter(|valor| !valor.is_empty())
        },
    ) {
        Ok(canais) => canais,
        Err(erro) => {
            eprintln!("instâncias da Evolution: {erro}");
            return ExitCode::from(1);
        }
    };

    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(erro) => {
            eprintln!("não consegui iniciar o runtime do servidor: {erro}");
            return ExitCode::from(1);
        }
    };

    runtime.block_on(async move {
        let store = match Store::open(&configuracao) {
            Ok(store) => Arc::new(store),
            Err(erro) => {
                eprintln!("não consegui abrir o banco: {erro}");
                return ExitCode::from(1);
            }
        };
        let estado = match ServerState::new(Arc::clone(&store), configuracao.clone()) {
            Ok(estado) => estado.with_evolution_channels(canais).with_cluster(cluster),
            Err(erro) => {
                eprintln!("{erro}");
                return ExitCode::from(1);
            }
        };
        // Turno no servidor (HR-26h): mensagem recebida vira resposta pela mesma montagem do chat.
        let estado = match ligar_turno(estado, &configuracao, ambiente, &store).await {
            Ok(estado) => estado,
            Err(erro) => {
                eprintln!("{erro}");
                return ExitCode::from(1);
            }
        };
        let endereco =
            std::net::SocketAddr::new(configuracao.server.address, configuracao.server.port);
        let listener = match tokio::net::TcpListener::bind(endereco).await {
            Ok(listener) => listener,
            Err(erro) => {
                eprintln!("não consegui abrir {endereco}: {erro}");
                return ExitCode::from(1);
            }
        };

        if args.somente_endereco {
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
            Err(erro) => {
                eprintln!("o servidor parou com erro: {erro}");
                ExitCode::from(1)
            }
        }
    })
}

/// Liga o turno no servidor (HR-26h).
///
/// Sem a chave do provedor (`llm.chave_env`, por padrão `ANTIGRAVITY_API_KEY`), o servidor segue só
/// recebendo: as mensagens ficam na fila e são respondidas quando ele subir com a chave. Com ela, a
/// base e o catálogo dos agentes das instâncias configuradas são conferidos antes de atender.
async fn ligar_turno(
    estado: ServerState,
    configuracao: &poph_types::HarnessConfig,
    ambiente: &config::Environment,
    store: &Arc<Store>,
) -> Result<ServerState, String> {
    let segredos = config::secrets(ambiente);
    let nome_da_chave = &configuracao.llm.api_key_env;
    let Some(chave) = config::llm_key(ambiente, nome_da_chave) else {
        if !estado.evolution_channels.is_empty() {
            tracing::warn!(
                target: "poph_server::turno",
                %nome_da_chave,
                "turno desligado: chave do provedor não definida. As mensagens recebidas ficam na fila e são respondidas quando o servidor subir com a chave"
            );
        }
        return Ok(estado);
    };
    let metricas = montagem::Metrics::registered(estado.metrics.clone(), &estado.registry)?;
    let montagem = montagem::assemble(
        configuracao,
        Arc::clone(store),
        montagem::Keys {
            llm: chave,
            jev: segredos.typesafe.clone(),
        },
        metricas,
        montagem::ProcessFlows::from_server(&estado),
        ambiente,
    )?;

    // Rede de segurança, como no chat: a base pode ter sido escrita fora do servidor. Sincronizar
    // só reembeda o que mudou.
    for canal in estado.evolution_channels.iter() {
        let agente = match store.channel_integration(canal.org, canal.integration) {
            Ok(Some(integracao)) => integracao.agent_id,
            Ok(None) => continue,
            Err(erro) => {
                eprintln!(
                    "[aviso] não consegui ler a integração de {}: {erro}",
                    canal.instance
                );
                continue;
            }
        };
        if let Some(base) = &montagem.base
            && let Err(erro) = base.synchronise_agent(canal.org, agente).await
        {
            eprintln!("[aviso] não consegui sincronizar o índice da base: {erro}");
        }
        if let Some(catalogo) = &montagem.catalog
            && let Err(erro) = catalogo.synchronise_agent(canal.org, agente).await
        {
            eprintln!("[aviso] não consegui sincronizar o índice de produtos: {erro}");
        }
    }

    let intervalo = std::time::Duration::from_millis(configuracao.turn.dispatch_interval_ms);
    // HR-22: o chat de teste do Console roda o mesmo motor do turno, com observador próprio.
    Ok(estado
        .with_engine(Arc::clone(&montagem.engine))
        .with_turn(montagem.flow, intervalo))
}

/// Executa `poph openapi`.
pub fn executar_openapi(args: OpenapiArgs) -> ExitCode {
    let documento = poph_server::openapi();
    let texto = match serde_json::to_string_pretty(&documento) {
        Ok(texto) => texto,
        Err(erro) => {
            eprintln!("não consegui serializar o OpenAPI: {erro}");
            return ExitCode::from(1);
        }
    };
    match args.out {
        Some(caminho) => {
            if let Some(pasta) = caminho.parent()
                && !pasta.as_os_str().is_empty()
                && let Err(erro) = std::fs::create_dir_all(pasta)
            {
                eprintln!("não consegui criar {}: {erro}", pasta.display());
                return ExitCode::from(1);
            }
            if let Err(erro) = std::fs::write(&caminho, format!("{texto}\n")) {
                eprintln!("não consegui gravar {}: {erro}", caminho.display());
                return ExitCode::from(1);
            }
            println!("OpenAPI gravado em {}", caminho.display());
        }
        None => println!("{texto}"),
    }
    ExitCode::SUCCESS
}

/// Recusa subir com banco existente que ficou para trás nas migrações.
///
/// Banco ausente é criado já migrado (instalação nova). Banco existente e desatualizado exige
/// `poph migrate` — subir assim faria a API servir dados num esquema que o código não espera.
fn conferir_migracao(configuracao: &poph_types::HarnessConfig) -> Result<(), String> {
    let alvo = harness_target_version();
    let caminho = configuracao.harness_db();
    if !caminho.exists() {
        return Ok(());
    }
    match poph_store::file_version(&caminho) {
        Ok(Some(versao)) if versao < alvo => Err(format!(
            "o banco {} está na versão {versao} e o código espera a {alvo}: rode `poph migrate` \
             com o servidor parado e suba de novo",
            caminho.display()
        )),
        Ok(_) => Ok(()),
        Err(erro) => Err(format!("não consegui ler a versão do banco: {erro}")),
    }
}
