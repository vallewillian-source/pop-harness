use std::io::{IsTerminal, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use clap::Args;
use poph_agent::turno::{TransportConsole, TurnEvent, TurnInput, TurnObserver};
use poph_core::llm::Cancellation;
use poph_store::{Conversa, Direction, NewConversation, NewMessage, Role, Store};
use poph_types::{AgentId, BaseMetrics, ConversationId, OrgId, Registry};
use serde_json::json;

use crate::assembly;
use crate::config;
use crate::support;

#[derive(Debug, Args)]
pub struct ChatArgs {
    #[arg(
        value_name = "AGENTE",
        help = "Id (UUID v7) ou slug do agente que atende"
    )]
    pub agente: String,

    #[arg(
        long,
        help = "Saída em JSON: uma linha por evento (`usuario`, `inicio`, `texto`, `ferramenta`, `fim`, `erro`)"
    )]
    pub json: bool,

    #[arg(
        long = "conversa",
        value_name = "CONVERSA",
        help = "Retoma uma conversa existente (id UUID v7) em vez de abrir uma nova com memória zerada"
    )]
    pub conversa: Option<String>,
}

pub fn run(
    flag: Option<PathBuf>,
    environment: &config::Environment,
    org: Option<&str>,
    args: ChatArgs,
) -> ExitCode {
    match run_chat(flag, environment, org, args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(mensagem) => support::failure(mensagem),
    }
}

fn run_chat(
    flag: Option<PathBuf>,
    environment: &config::Environment,
    org: Option<&str>,
    args: ChatArgs,
) -> Result<(), String> {
    let org_text = org.ok_or_else(|| {
        "informe --org com o id (UUID v7) ou o slug da org para abrir a conversa".to_owned()
    })?;
    let folder = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let file = config::resolve_file(flag.as_deref(), environment, &folder);
    let configuracao =
        config::load(file.as_deref(), environment).map_err(|error| format!("{error}"))?;
    let store = Arc::new(
        Store::open(&configuracao)
            .map_err(|error| format!("não consegui abrir o banco: {error}"))?,
    );
    let org = support::resolve_org(&store, org_text)?;
    let agente = resolve_agent(&store, org, &args.agente)?;
    let conversa = resolve_conversation(&store, org, &agente.id, &args)?;

    let key_name = configuracao.llm.api_key_env.clone();
    let chave = config::llm_key(environment, &key_name).ok_or_else(|| {
        format!("defina {key_name} no ambiente desta sessão para conversar com o modelo")
    })?;
    let registro = Registry::new();
    let base_metrics =
        BaseMetrics::register(&registro).map_err(|error| format!("métricas base: {error}"))?;
    let assembly = assembly::assemble(
        &configuracao,
        Arc::clone(&store),
        assembly::Keys {
            llm: chave,
            jev: config::secrets(environment).typesafe,
        },
        assembly::Metrics::own(base_metrics),
        assembly::ProcessFlows::default(),
        environment,
    )?;
    let assembly::Assembly {
        engine,
        flow: fluxo,
        base,
        catalog,
        ..
    } = assembly;

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| format!("não consegui iniciar o runtime: {error}"))?;

    let transport = TransportConsole::silencioso();
    if args.json {
        println!(
            "{}",
            json!({
                "tipo": "conversa",
                "conversa": conversa.id.to_string(),
                "agente": agente.id.to_string(),
                "slug": agente.slug,
                "canal": conversa.canal,
                "modelo": agent_model(&agente, &configuracao.llm.model),
            })
        );
    } else {
        println!("POP Harness · {} · conversa {}", agente.name, conversa.id);
        println!("Digite /sair para encerrar.\n");
    }

    let interativo = std::io::stdin().is_terminal();
    runtime.block_on(async {
        if let Some(base) = &base
            && let Err(error) = base.synchronise_agent(org, agente.id).await
        {
            eprintln!("[aviso] não consegui sincronizar o índice da base: {error}");
        }
        if let Some(catalog) = &catalog
            && let Err(error) = catalog.synchronise_agent(org, agente.id).await
        {
            eprintln!("[aviso] não consegui sincronizar o índice de produtos: {error}");
        }
        loop {
            if interativo && !args.json {
                print!("você> ");
                let _ = std::io::stdout().flush();
            }
            let Some(line) = read_line().await else {
                break;
            };
            let texto = line.trim();
            if texto.is_empty() {
                continue;
            }
            if texto == "/sair" || texto == "/exit" {
                break;
            }

            let mensagem = store
                .messages()
                .write(
                    org,
                    NewMessage {
                        conversation_id: conversa.id,
                        direction: Direction::Inbound,
                        role: Role::User,
                        texto: texto.to_owned(),
                        attachments_json: "[]".to_owned(),
                        external_id: None,
                        turn_id: None,
                        origin: poph_store::Origin::Agente,
                        author_name: None,
                    },
                )
                .map_err(|error| format!("não consegui gravar a mensagem: {error}"))?;
            if args.json {
                println!(
                    "{}",
                    json!({
                        "tipo": "usuario",
                        "conversa": conversa.id.to_string(),
                        "mensagem": mensagem.id.to_string(),
                    })
                );
            }

            let mut input = TurnInput::new(org, conversa.id, mensagem.id);
            let cancellation = Cancellation::new();
            input.cancellation = cancellation.clone();
            let mut observer = ChatObserver::new(args.json);
            let turno = engine.run(input, &mut observer);
            tokio::pin!(turno);
            let summary = loop {
                tokio::select! {
                    resultado = &mut turno => break resultado,
                    _ = tokio::signal::ctrl_c() => {
                        if !args.json {
                            eprintln!("\n[cancelando o turno…]");
                        }
                        cancellation.cancel();
                    }
                }
            };

            match summary {
                Ok(summary) => {
                    if args.json {
                        println!(
                            "{}",
                            json!({
                                "tipo": "fim",
                                "turno": summary.turno.to_string(),
                                "estado": summary.estado.as_text(),
                                "motivo": summary.motivo_fim,
                                "erro": summary.error,
                                "tokens_entrada": summary.input_tokens,
                                "tokens_saida": summary.output_tokens,
                                "total_ms": summary.timings.total_ms,
                                "primeiro_texto_ms": summary.timings.first_text_ms,
                                "fila_ms": summary.timings.queue_ms,
                                "montagem_ms": summary.timings.assembly_ms,
                            })
                        );
                    } else {
                        println!();
                    }
                }
                Err(error) => {
                    if args.json {
                        println!("{}", json!({"tipo": "erro", "mensagem": error.to_string()}));
                    } else {
                        eprintln!("\n[erro] {error}");
                    }
                }
            }

            if let Err(error) = fluxo.send_pending_console(org, &transport).await {
                if args.json {
                    println!(
                        "{}",
                        json!({"tipo": "erro", "mensagem": format!("saída: {error}")})
                    );
                } else {
                    eprintln!("[aviso] a saída não foi confirmada: {error}");
                }
            }
        }
        Ok::<(), String>(())
    })
}

fn agent_model(agente: &poph_store::Agente, default: &str) -> String {
    let modelo = agente.modelo.trim();
    if modelo.is_empty() {
        default.to_owned()
    } else {
        modelo.to_owned()
    }
}

async fn read_line() -> Option<String> {
    tokio::task::spawn_blocking(|| {
        let mut line = String::new();
        match std::io::stdin().read_line(&mut line) {
            Ok(0) => None,
            Ok(_) => Some(line),
            Err(_) => None,
        }
    })
    .await
    .ok()
    .flatten()
}

fn resolve_agent(store: &Store, org: OrgId, texto: &str) -> Result<poph_store::Agente, String> {
    if uuid::Uuid::parse_str(texto).is_ok() {
        let id: AgentId = support::parse_id(texto)?;
        return store
            .agents()
            .get(org, id)
            .map_err(|error| support::describe(&error));
    }
    store
        .agents()
        .by_slug(org, texto)
        .map_err(|error| support::describe(&error))
}

fn resolve_conversation(
    store: &Store,
    org: OrgId,
    agente: &AgentId,
    args: &ChatArgs,
) -> Result<Conversa, String> {
    match &args.conversa {
        Some(texto) => {
            let id: ConversationId = support::parse_id(texto)?;
            let conversa = store
                .conversations()
                .get(org, id)
                .map_err(|error| support::describe(&error))?;
            if conversa.agent_id != *agente {
                return Err(format!(
                    "a conversa {id} é de outro agente; abra uma conversa nova ou passe o agente \
                     dono dela"
                ));
            }
            Ok(conversa)
        }
        None => store
            .conversations()
            .create(
                org,
                NewConversation {
                    agent_id: *agente,
                    integration_id: None,
                    canal: "cli".to_owned(),
                    peer_externo: Some("cli".to_owned()),
                    target_department: None,
                },
            )
            .map_err(|error| format!("não consegui abrir a conversa: {error}")),
    }
}

struct ChatObserver {
    json: bool,
    wrote_text: bool,
}

impl ChatObserver {
    fn new(json: bool) -> Self {
        Self {
            json,
            wrote_text: false,
        }
    }
}

impl TurnObserver for ChatObserver {
    fn evento(&mut self, evento: TurnEvent) {
        match evento {
            TurnEvent::Started { turno, .. } => {
                if self.json {
                    println!("{}", json!({"tipo": "inicio", "turno": turno.to_string()}));
                }
            }
            TurnEvent::Texto(piece) => {
                if self.json {
                    println!("{}", json!({"tipo": "texto", "delta": piece}));
                } else {
                    if !self.wrote_text {
                        println!();
                        self.wrote_text = true;
                    }
                    print!("{piece}");
                    let _ = std::io::stdout().flush();
                }
            }
            TurnEvent::Base { sources } => {
                if self.json {
                    println!(
                        "{}",
                        json!({"tipo": "base", "trechos": sources.len(), "fontes": sources})
                    );
                } else if !sources.is_empty() {
                    eprintln!("\n[base: {}]", sources.join("; "));
                }
            }
            TurnEvent::Tool { nome, round } => {
                if self.json {
                    println!(
                        "{}",
                        json!({"tipo": "ferramenta", "nome": nome, "rodada": round})
                    );
                } else {
                    eprintln!("\n[ferramenta: {nome}]");
                }
            }
            TurnEvent::Handoff {
                departamento,
                risk_kind,
                guidance,
            } => {
                if self.json {
                    println!(
                        "{}",
                        json!({
                            "tipo": "transferencia",
                            "departamento": departamento.map(|dep| dep.to_string()),
                            "tipo_risco": risk_kind,
                            "orientacao": guidance,
                        })
                    );
                } else {
                    eprintln!(
                        "\n[transferida para humano{}{}]",
                        departamento
                            .map(|dep| format!(" · departamento {dep}"))
                            .unwrap_or_default(),
                        if guidance {
                            " · com orientação ao cliente"
                        } else {
                            ""
                        }
                    );
                }
            }
            TurnEvent::Fim { .. } => {}
        }
    }
}
