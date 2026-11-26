use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use poph_agent::atores::{Atores, Futuro, OpcoesAtores, Processador, Tarefa};
use poph_agent::entrada_saida::{Fluxo, OpcoesFluxo};
use poph_agent::fila_integracao::{ConversaDoCanal, EnvioPeloCanal, InboxDoCanal};
use poph_agent::metricas::MetricasMemoria;
use poph_plugins::{OpcoesPlugin, Supervisor};
use poph_store::{NewAgent, NewConversation, NewOrg, NewOutboxItem, OutboxState, Page, Store};
use poph_types::{EventSink, Integration, IntegrationId, SendText, TurnId};

struct NoTurn;

impl Processador for NoTurn {
    fn processar(&self, _task: Tarefa) -> Futuro {
        Box::pin(async { Ok(()) })
    }
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let directory = match std::env::args().nth(1) {
        Some(path) => PathBuf::from(path),
        None => std::env::temp_dir().join(format!("poph-eco-demo-{}", std::process::id())),
    };
    let temporary = std::env::args().nth(1).is_none();
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory)?;

    let program = echo_program()?;
    println!("0. plugin de exemplo: {}", program.display());

    let store = Arc::new(Store::open_at(
        &directory.join("harness.db"),
        &directory.join("rag.db"),
    )?);
    let org = store
        .orgs()
        .create(NewOrg {
            name: "Org do exemplo".to_owned(),
            slug: "org-do-exemplo".to_owned(),
            config_json: String::new(),
        })?
        .id;
    let agent = store
        .agents()
        .create(
            org,
            NewAgent {
                name: "Agente do eco".to_owned(),
                slug: "agente-do-eco".to_owned(),
                ..NewAgent::default()
            },
        )?
        .id;
    let conversation = store.conversations().create(
        org,
        NewConversation {
            agent_id: agent,
            integration_id: None,
            canal: "eco".to_owned(),
            peer_externo: Some("5511999999999".to_owned()),
            target_department: None,
        },
    )?;
    println!(
        "1. conversa aberta com o contato {} (canal eco)",
        conversation.peer_externo.as_deref().unwrap_or("-")
    );

    let actors = Atores::novo(
        Arc::new(NoTurn),
        OpcoesAtores::default(),
        MetricasMemoria::proprio(),
    );
    let flow = Fluxo::nova(Arc::clone(&store), actors, OpcoesFluxo::default());

    let resolver = Arc::new(ConversaDoCanal::novo(
        Arc::clone(&store),
        agent,
        "eco",
        None,
    ));
    let inbox = InboxDoCanal::iniciar(flow.clone(), org, resolver);

    let integration = IntegrationId::generate();
    let options = OpcoesPlugin::novo(&program, integration, org).com_kind("eco");
    let supervisor = Supervisor::iniciar(options, Arc::clone(&inbox) as Arc<dyn EventSink>).await?;
    let manifest = supervisor.manifesto().expect("manifesto do handshake");
    println!(
        "2. supervisor pronto: {} v{} (kind={}, protocolo={})",
        manifest.name, manifest.version, manifest.kind, manifest.protocol
    );
    let channel: Arc<dyn Integration> = Arc::new(supervisor.clone());

    let turn = TurnId::generate();
    let enqueued = store.outbox().enqueue(
        org,
        NewOutboxItem {
            conversation_id: conversation.id,
            turn_id: turn,
            seq: 0,
            tipo: "text".to_owned(),
            content_json: serde_json::json!({"texto": "olá, mundo"}).to_string(),
        },
    )?;
    println!(
        "3. item na fila de saída: `olá, mundo` (estado {:?})",
        enqueued.1.state
    );

    let transport = EnvioPeloCanal::novo(Arc::clone(&store), Arc::clone(&channel));
    let report = flow.enviar_pendentes(org, &transport).await?;
    let item = store.outbox().get(org, enqueued.1.id)?;
    println!(
        "4. envio pelo plugin: enviados={} falharam={} → estado {:?}",
        report.enviados, report.falharam, item.state
    );

    wait_until(|| inbox.contadores().gravados >= 1).await?;
    let messages = store
        .messages()
        .list(org, conversation.id, Page::with_limit(10))?;
    for message in &messages {
        println!(
            "5. inbox → mensagem ({}/{}): {:?}",
            message.direction.as_text(),
            message.role.as_text(),
            message.texto
        );
    }

    channel
        .send_text(SendText {
            idempotency: format!("{turn}:0"),
            peer: "5511999999999".to_owned(),
            text: "olá, mundo".to_owned(),
        })
        .await?;
    wait_until(|| inbox.contadores().duplicados >= 1).await?;
    let counters = inbox.contadores();
    println!(
        "6. reentrega do mesmo comando: gravados={} duplicados={} (nada duplicado na conversa)",
        counters.gravados, counters.duplicados
    );
    let messages = store
        .messages()
        .list(org, conversation.id, Page::with_limit(10))?;
    println!("7. mensagens na conversa no fim: {}", messages.len());

    let health = channel.health().await?;
    println!(
        "8. saúde do canal: ok={} ({})",
        health.ok,
        health.detail.unwrap_or_default()
    );
    println!(
        "9. fila de saída no fim: pendentes={} enviados={}",
        store.outbox().count_by_state(org, OutboxState::Pending)?,
        store.outbox().count_by_state(org, OutboxState::Sent)?
    );

    supervisor.encerrar().await;
    drop(inbox);
    if temporary {
        let _ = std::fs::remove_dir_all(&directory);
    }
    Ok(())
}

fn echo_program() -> Result<PathBuf, Box<dyn std::error::Error>> {
    let current = std::env::current_exe()?;
    let name = if cfg!(windows) {
        "plugin-eco.exe"
    } else {
        "plugin-eco"
    };
    let mut directory = current.parent();
    for _ in 0..3 {
        let Some(here) = directory else { break };
        let path = here.join(name);
        if path.exists() {
            return Ok(path);
        }
        directory = here.parent();
    }
    Err(format!(
        "não encontrei o `{}` perto de {}: rode `cargo build -p poph-plugin-eco --bins`",
        name,
        current.display()
    )
    .into())
}

async fn wait_until(condition: impl Fn() -> bool) -> Result<(), Box<dyn std::error::Error>> {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if condition() {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    Err("a condição não ficou verdadeira em 5 s".into())
}
