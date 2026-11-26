//! Demonstração de ponta a ponta do plugin de exemplo (HR-27).
//!
//! Sobe o **`plugin-eco`** de verdade como processo filho, enfileira uma mensagem na `outbox` e
//! mostra o caminho completo que o WhatsApp vai usar:
//!
//! ```text
//! outbox (texto) → supervisor → plugin eco → evento de entrada → inbox → mensagem na conversa
//! ```
//!
//! Uso: `plugin-eco-demo [pasta]` (pasta padrão: temporária, apagada no fim).
//! Normalmente chamado por `scripts/plugin-eco.sh`.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use poph_agent::atores::{Atores, Futuro, OpcoesAtores, Processador, Tarefa};
use poph_agent::entrada_saida::{Fluxo, OpcoesFluxo};
use poph_agent::fila_integracao::{ConversaDoCanal, EnvioPeloCanal, InboxDoCanal};
use poph_agent::metricas::MetricasMemoria;
use poph_plugins::{OpcoesPlugin, Supervisor};
use poph_store::{EstadoSaida, NovaConversa, NovaOrg, NovaSaida, NovoAgente, Pagina, Store};
use poph_types::{EventSink, Integration, IntegrationId, SendText, TurnId};

/// Nenhum turno é processado nesta demonstração: só a ida e a volta das filas.
struct SemTurno;

impl Processador for SemTurno {
    fn processar(&self, _tarefa: Tarefa) -> Futuro {
        Box::pin(async { Ok(()) })
    }
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let pasta = match std::env::args().nth(1) {
        Some(caminho) => PathBuf::from(caminho),
        None => std::env::temp_dir().join(format!("poph-eco-demo-{}", std::process::id())),
    };
    let temporaria = std::env::args().nth(1).is_none();
    let _ = std::fs::remove_dir_all(&pasta);
    std::fs::create_dir_all(&pasta)?;

    let programa = programa_eco()?;
    println!("0. plugin de exemplo: {}", programa.display());

    // Banco temporário com org, agente e a conversa do contato.
    let store = Arc::new(Store::abrir_em(
        &pasta.join("harness.db"),
        &pasta.join("rag.db"),
    )?);
    let org = store
        .orgaos()
        .criar(NovaOrg {
            nome: "Org do exemplo".to_owned(),
            slug: "org-do-exemplo".to_owned(),
            config_json: String::new(),
        })?
        .id;
    let agente = store
        .agentes()
        .criar(
            org,
            NovoAgente {
                nome: "Agente do eco".to_owned(),
                slug: "agente-do-eco".to_owned(),
                ..NovoAgente::default()
            },
        )?
        .id;
    let conversa = store.conversas().criar(
        org,
        NovaConversa {
            agent_id: agente,
            integration_id: None,
            canal: "eco".to_owned(),
            peer_externo: Some("5511999999999".to_owned()),
            departamento_alvo: None,
        },
    )?;
    println!(
        "1. conversa aberta com o contato {} (canal eco)",
        conversa.peer_externo.as_deref().unwrap_or("-")
    );

    let atores = Atores::novo(
        Arc::new(SemTurno),
        OpcoesAtores::default(),
        MetricasMemoria::proprio(),
    );
    let fluxo = Fluxo::nova(Arc::clone(&store), atores, OpcoesFluxo::default());

    // Entrada: o evento do plugin cai na inbox da conversa do contato.
    let resolvedor = Arc::new(ConversaDoCanal::novo(
        Arc::clone(&store),
        agente,
        "eco",
        None,
    ));
    let entrada = InboxDoCanal::iniciar(fluxo.clone(), org, resolvedor);

    // O canal: o supervisor do plugin em processo externo.
    let integracao = IntegrationId::generate();
    let opcoes = OpcoesPlugin::novo(&programa, integracao, org).com_kind("eco");
    let supervisor =
        Supervisor::iniciar(opcoes, Arc::clone(&entrada) as Arc<dyn EventSink>).await?;
    let manifesto = supervisor.manifesto().expect("manifesto do handshake");
    println!(
        "2. supervisor pronto: {} v{} (kind={}, protocolo={})",
        manifesto.name, manifesto.version, manifesto.kind, manifesto.protocol
    );
    let canal: Arc<dyn Integration> = Arc::new(supervisor.clone());

    // Saída: um item de texto na fila, entregue ao plugin pelo mesmo transporte do WhatsApp.
    let turno = TurnId::generate();
    let enfileirado = store.caixa_saida().enfileirar(
        org,
        NovaSaida {
            conversation_id: conversa.id,
            turn_id: turno,
            seq: 0,
            tipo: "text".to_owned(),
            conteudo_json: serde_json::json!({"texto": "olá, mundo"}).to_string(),
        },
    )?;
    println!(
        "3. item na fila de saída: `olá, mundo` (estado {:?})",
        enfileirado.1.estado
    );

    let transporte = EnvioPeloCanal::novo(Arc::clone(&store), Arc::clone(&canal));
    let relato = fluxo.enviar_pendentes(org, &transporte).await?;
    let item = store.caixa_saida().obter(org, enfileirado.1.id)?;
    println!(
        "4. envio pelo plugin: enviados={} falharam={} → estado {:?}",
        relato.enviados, relato.falharam, item.estado
    );

    esperar(|| entrada.contadores().gravados >= 1).await?;
    let mensagens = store
        .mensagens()
        .listar(org, conversa.id, Pagina::com_limite(10))?;
    for mensagem in &mensagens {
        println!(
            "5. inbox → mensagem ({}/{}): {:?}",
            mensagem.direcao.como_texto(),
            mensagem.papel.como_texto(),
            mensagem.texto
        );
    }

    // Reentrega do mesmo comando: o `id_externo` estável faz a inbox descartar a repetição.
    canal
        .send_text(SendText {
            idempotency: format!("{turno}:0"),
            peer: "5511999999999".to_owned(),
            text: "olá, mundo".to_owned(),
        })
        .await?;
    esperar(|| entrada.contadores().duplicados >= 1).await?;
    let contadores = entrada.contadores();
    println!(
        "6. reentrega do mesmo comando: gravados={} duplicados={} (nada duplicado na conversa)",
        contadores.gravados, contadores.duplicados
    );
    let mensagens = store
        .mensagens()
        .listar(org, conversa.id, Pagina::com_limite(10))?;
    println!("7. mensagens na conversa no fim: {}", mensagens.len());

    let saude = canal.health().await?;
    println!(
        "8. saúde do canal: ok={} ({})",
        saude.ok,
        saude.detail.unwrap_or_default()
    );
    println!(
        "9. fila de saída no fim: pendentes={} enviados={}",
        store
            .caixa_saida()
            .contar_por_estado(org, EstadoSaida::Pendente)?,
        store
            .caixa_saida()
            .contar_por_estado(org, EstadoSaida::Enviado)?
    );

    supervisor.encerrar().await;
    drop(entrada);
    if temporaria {
        let _ = std::fs::remove_dir_all(&pasta);
    }
    Ok(())
}

/// Binário do `plugin-eco` perto deste executável (os dois saem do mesmo `cargo build`).
///
/// O `example` fica em `target/debug/examples/` e o binário em `target/debug/`, então a busca
/// sobe as pastas até achar.
fn programa_eco() -> Result<PathBuf, Box<dyn std::error::Error>> {
    let atual = std::env::current_exe()?;
    let nome = if cfg!(windows) {
        "plugin-eco.exe"
    } else {
        "plugin-eco"
    };
    let mut pasta = atual.parent();
    for _ in 0..3 {
        let Some(diretorio) = pasta else { break };
        let caminho = diretorio.join(nome);
        if caminho.exists() {
            return Ok(caminho);
        }
        pasta = diretorio.parent();
    }
    Err(format!(
        "não encontrei o `{}` perto de {}: rode `cargo build -p poph-plugin-eco --bins`",
        nome,
        atual.display()
    )
    .into())
}

/// Espera a condição virar verdadeira, conferindo a cada 10 ms (até 5 s).
async fn esperar(condicao: impl Fn() -> bool) -> Result<(), Box<dyn std::error::Error>> {
    let limite = Instant::now() + Duration::from_secs(5);
    while Instant::now() < limite {
        if condicao() {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    Err("a condição não ficou verdadeira em 5 s".into())
}
