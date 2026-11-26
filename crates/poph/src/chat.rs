//! `poph chat <agente>`: a conversa funcionando de ponta a ponta pela CLI (HR-13, marco da fase 1).
//!
//! O comando abre (ou retoma) uma conversa do canal `cli`, lê a linha digitada, grava como mensagem
//! do cliente e roda o turno do motor com **streaming**: cada pedaço de texto do modelo aparece
//! conforme chega. Ctrl-C no meio do stream cancela o turno (o turno é gravado como falho e a
//! entrada é concluída). `--json` troca a saída por uma linha JSON por evento.
//!
//! Memória zerada: cada execução sem `--conversa` abre uma conversa nova; para continuar uma
//! anterior, passe o id dela.

use std::io::{IsTerminal, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use clap::Args;
use poph_agent::turno::{Entrada, EventoTurno, Observador, TransporteConsole};
use poph_core::llm::Cancelamento;
use poph_store::{Conversa, Direction, NewConversation, NewMessage, Role, Store};
use poph_types::{AgentId, BaseMetrics, ConversationId, OrgId, Registry};
use serde_json::json;

use crate::apoio;
use crate::config;
use crate::montagem;

/// Opções do `poph chat`.
#[derive(Debug, Args)]
pub struct ChatArgs {
    /// Id (UUID v7) ou slug do agente que atende.
    pub agente: String,

    /// Saída em JSON: uma linha por evento (`usuario`, `inicio`, `texto`, `ferramenta`, `fim`,
    /// `erro`).
    #[arg(long)]
    pub json: bool,

    /// Retoma uma conversa existente (id UUID v7) em vez de abrir uma nova com memória zerada.
    #[arg(long, value_name = "CONVERSA")]
    pub conversa: Option<String>,
}

/// Executa o `poph chat`.
pub fn executar(
    flag: Option<PathBuf>,
    ambiente: &config::Ambiente,
    org: Option<&str>,
    args: ChatArgs,
) -> ExitCode {
    match rodar(flag, ambiente, org, args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(mensagem) => apoio::falha(mensagem),
    }
}

fn rodar(
    flag: Option<PathBuf>,
    ambiente: &config::Ambiente,
    org: Option<&str>,
    args: ChatArgs,
) -> Result<(), String> {
    let texto_org = org.ok_or_else(|| {
        "informe --org com o id (UUID v7) ou o slug da org para abrir a conversa".to_owned()
    })?;
    let pasta = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let arquivo = config::resolver_arquivo(flag.as_deref(), ambiente, &pasta);
    let configuracao =
        config::carregar(arquivo.as_deref(), ambiente).map_err(|erro| format!("{erro}"))?;
    let store = Arc::new(
        Store::open(&configuracao).map_err(|erro| format!("não consegui abrir o banco: {erro}"))?,
    );
    let org = apoio::resolver_org(&store, texto_org)?;
    let agente = resolver_agente(&store, org, &args.agente)?;
    let conversa = resolver_conversa(&store, org, &agente.id, &args)?;

    let nome_da_chave = configuracao.llm.api_key_env.clone();
    let chave = config::chave_llm(ambiente, &nome_da_chave).ok_or_else(|| {
        format!("defina {nome_da_chave} no ambiente desta sessão para conversar com o modelo")
    })?;
    let registro = Registry::new();
    let metricas_base =
        BaseMetrics::register(&registro).map_err(|erro| format!("métricas base: {erro}"))?;
    // Mesma montagem do `poph serve` (HR-26h); o chat mantém as métricas dele, fora do `/metrics`.
    let montagem = montagem::montar(
        &configuracao,
        Arc::clone(&store),
        montagem::Chaves {
            llm: chave,
            jev: config::segredos(ambiente).typesafe,
        },
        montagem::Metricas::proprias(metricas_base),
        montagem::FluxosDoProcesso::default(),
        ambiente,
    )?;
    let montagem::Montagem {
        motor,
        fluxo,
        base,
        catalogo,
        ..
    } = montagem;

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|erro| format!("não consegui iniciar o runtime: {erro}"))?;

    // O canal do chat confirma os itens da `outbox` sem imprimir de novo: o texto já saiu no stream.
    let transporte = TransporteConsole::silencioso();
    if args.json {
        println!(
            "{}",
            json!({
                "tipo": "conversa",
                "conversa": conversa.id.to_string(),
                "agente": agente.id.to_string(),
                "slug": agente.slug,
                "canal": conversa.canal,
                "modelo": modelo_do_agente(&agente, &configuracao.llm.model),
            })
        );
    } else {
        println!("POP Harness · {} · conversa {}", agente.name, conversa.id);
        println!("Digite /sair para encerrar.\n");
    }

    let interativo = std::io::stdin().is_terminal();
    runtime.block_on(async {
        // Rede de segurança: a base pode ter sido escrita fora da CLI (importador, Console) ou o
        // índice pode ter sido limpo por troca de modelo. Sincronizar aqui só reembeda o que mudou.
        if let Some(base) = &base
            && let Err(erro) = base.sincronizar_agente(org, agente.id).await
        {
            eprintln!("[aviso] não consegui sincronizar o índice da base: {erro}");
        }
        if let Some(catalogo) = &catalogo
            && let Err(erro) = catalogo.sincronizar_agente(org, agente.id).await
        {
            eprintln!("[aviso] não consegui sincronizar o índice de produtos: {erro}");
        }
        loop {
            if interativo && !args.json {
                print!("você> ");
                let _ = std::io::stdout().flush();
            }
            let Some(linha) = ler_linha().await else {
                break;
            };
            let texto = linha.trim();
            if texto.is_empty() {
                continue;
            }
            if texto == "/sair" || texto == "/exit" {
                break;
            }

            let mensagem = store
                .messages()
                .gravar(
                    org,
                    NewMessage {
                        conversation_id: conversa.id,
                        direction: Direction::Inbound,
                        role: Role::User,
                        texto: texto.to_owned(),
                        attachments_json: "[]".to_owned(),
                        id_externo: None,
                        turn_id: None,
                        origin: poph_store::Origin::Agente,
                        author_name: None,
                    },
                )
                .map_err(|erro| format!("não consegui gravar a mensagem: {erro}"))?;
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

            let mut entrada = Entrada::nova(org, conversa.id, mensagem.id);
            let cancelamento = Cancelamento::novo();
            entrada.cancelamento = cancelamento.clone();
            let mut observador = ObservadorChat::novo(args.json);
            let turno = motor.executar(entrada, &mut observador);
            tokio::pin!(turno);
            let resumo = loop {
                tokio::select! {
                    resultado = &mut turno => break resultado,
                    _ = tokio::signal::ctrl_c() => {
                        if !args.json {
                            eprintln!("\n[cancelando o turno…]");
                        }
                        cancelamento.cancelar();
                    }
                }
            };

            match resumo {
                Ok(resumo) => {
                    if args.json {
                        println!(
                            "{}",
                            json!({
                                "tipo": "fim",
                                "turno": resumo.turno.to_string(),
                                "estado": resumo.estado.as_text(),
                                "motivo": resumo.motivo_fim,
                                "erro": resumo.erro,
                                "tokens_entrada": resumo.tokens_entrada,
                                "tokens_saida": resumo.tokens_saida,
                                "total_ms": resumo.tempos.total_ms,
                                "primeiro_texto_ms": resumo.tempos.primeiro_texto_ms,
                                "fila_ms": resumo.tempos.fila_ms,
                                "montagem_ms": resumo.tempos.montagem_ms,
                            })
                        );
                    } else {
                        println!();
                    }
                }
                Err(erro) => {
                    if args.json {
                        println!("{}", json!({"tipo": "erro", "mensagem": erro.to_string()}));
                    } else {
                        eprintln!("\n[erro] {erro}");
                    }
                }
            }

            // Confirma a saída no canal do chat (o texto já foi impresso pelo stream).
            // Só os itens do console: a fila da org também tem WhatsApp, que este processo não entrega (HR-26f).
            if let Err(erro) = fluxo.enviar_pendentes_do_console(org, &transporte).await {
                if args.json {
                    println!(
                        "{}",
                        json!({"tipo": "erro", "mensagem": format!("saída: {erro}")})
                    );
                } else {
                    eprintln!("[aviso] a saída não foi confirmada: {erro}");
                }
            }
        }
        Ok::<(), String>(())
    })
}

fn modelo_do_agente(agente: &poph_store::Agente, padrao: &str) -> String {
    let modelo = agente.modelo.trim();
    if modelo.is_empty() {
        padrao.to_owned()
    } else {
        modelo.to_owned()
    }
}

/// Lê uma linha da entrada padrão sem travar a thread do runtime.
async fn ler_linha() -> Option<String> {
    tokio::task::spawn_blocking(|| {
        let mut linha = String::new();
        match std::io::stdin().read_line(&mut linha) {
            Ok(0) => None,
            Ok(_) => Some(linha),
            Err(_) => None,
        }
    })
    .await
    .ok()
    .flatten()
}

/// Acha o agente pelo id (UUID v7) ou pelo slug, dentro da org.
fn resolver_agente(store: &Store, org: OrgId, texto: &str) -> Result<poph_store::Agente, String> {
    if uuid::Uuid::parse_str(texto).is_ok() {
        let id: AgentId = apoio::parse_id(texto)?;
        return store
            .agents()
            .get(org, id)
            .map_err(|erro| apoio::descrever(&erro));
    }
    store
        .agents()
        .por_slug(org, texto)
        .map_err(|erro| apoio::descrever(&erro))
}

/// Retoma a conversa pedida ou abre uma nova com memória zerada.
fn resolver_conversa(
    store: &Store,
    org: OrgId,
    agente: &AgentId,
    args: &ChatArgs,
) -> Result<Conversa, String> {
    match &args.conversa {
        Some(texto) => {
            let id: ConversationId = apoio::parse_id(texto)?;
            let conversa = store
                .conversas()
                .get(org, id)
                .map_err(|erro| apoio::descrever(&erro))?;
            if conversa.agent_id != *agente {
                return Err(format!(
                    "a conversa {id} é de outro agente; abra uma conversa nova ou passe o agente \
                     dono dela"
                ));
            }
            Ok(conversa)
        }
        None => store
            .conversas()
            .criar(
                org,
                NewConversation {
                    agent_id: *agente,
                    integration_id: None,
                    canal: "cli".to_owned(),
                    peer_externo: Some("cli".to_owned()),
                    target_department: None,
                },
            )
            .map_err(|erro| format!("não consegui abrir a conversa: {erro}")),
    }
}

/// Observador que imprime o turno conforme ele acontece.
struct ObservadorChat {
    json: bool,
    escreveu_texto: bool,
}

impl ObservadorChat {
    fn novo(json: bool) -> Self {
        Self {
            json,
            escreveu_texto: false,
        }
    }
}

impl Observador for ObservadorChat {
    fn evento(&mut self, evento: EventoTurno) {
        match evento {
            EventoTurno::Inicio { turno, .. } => {
                if self.json {
                    println!("{}", json!({"tipo": "inicio", "turno": turno.to_string()}));
                }
            }
            EventoTurno::Texto(pedaco) => {
                if self.json {
                    println!("{}", json!({"tipo": "texto", "delta": pedaco}));
                } else {
                    if !self.escreveu_texto {
                        println!();
                        self.escreveu_texto = true;
                    }
                    print!("{pedaco}");
                    let _ = std::io::stdout().flush();
                }
            }
            EventoTurno::Base { fontes } => {
                if self.json {
                    println!(
                        "{}",
                        json!({"tipo": "base", "trechos": fontes.len(), "fontes": fontes})
                    );
                } else if !fontes.is_empty() {
                    eprintln!("\n[base: {}]", fontes.join("; "));
                }
            }
            EventoTurno::Ferramenta { nome, rodada } => {
                if self.json {
                    println!(
                        "{}",
                        json!({"tipo": "ferramenta", "nome": nome, "rodada": rodada})
                    );
                } else {
                    eprintln!("\n[ferramenta: {nome}]");
                }
            }
            EventoTurno::Transferencia {
                departamento,
                tipo_risco,
                orientacao,
            } => {
                if self.json {
                    println!(
                        "{}",
                        json!({
                            "tipo": "transferencia",
                            "departamento": departamento.map(|dep| dep.to_string()),
                            "tipo_risco": tipo_risco,
                            "orientacao": orientacao,
                        })
                    );
                } else {
                    eprintln!(
                        "\n[transferida para humano{}{}]",
                        departamento
                            .map(|dep| format!(" · departamento {dep}"))
                            .unwrap_or_default(),
                        if orientacao {
                            " · com orientação ao cliente"
                        } else {
                            ""
                        }
                    );
                }
            }
            // O `fim` sai no laço principal, com tokens e tempos; aqui não há nada a imprimir.
            EventoTurno::Fim { .. } => {}
        }
    }
}
