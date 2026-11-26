mod common;

use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

use common::{Scenario, temporary_dir};
use poph_agent::fila_integracao::EnvioPeloCanal;
use poph_types::IntegrationState;
use poph_types::TurnId;

fn plugin_binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_plugin-eco"))
}

struct Cleanup(PathBuf);

impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn unique_copy() -> (Cleanup, PathBuf, String) {
    let directory = temporary_dir("queda-bin");
    let name = format!("plugin-eco-{}-{}", std::process::id(), now_ms());
    let destination = directory.join(&name);
    std::fs::copy(plugin_binary(), &destination).expect("copia o binário do plugin");
    (Cleanup(directory), destination, name)
}

fn now_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis())
        .unwrap_or(0)
}

fn kill(name: &str) -> bool {
    let output = Command::new("pkill")
        .args(["-9", "-f", name])
        .output()
        .expect("executa o pkill");
    output.status.success()
}

#[tokio::test(flavor = "multi_thread")]
async fn kill_9_on_the_plugin_does_not_drop_the_supervisor_and_the_message_comes_back() {
    let (_cleanup, program, name) = unique_copy();
    let scenario = Scenario::new("queda", &program, Some("5511999999999")).await;
    let transport: EnvioPeloCanal = scenario.transport();

    scenario.enqueue_text(TurnId::generate(), "antes");
    let report = scenario
        .flow
        .enviar_pendentes(scenario.org, &transport)
        .await
        .expect("envia o primeiro texto");
    assert_eq!(report.enviados, 1, "{report:?}");
    scenario
        .wait_until("a primeira mensagem chegar", || {
            scenario.inbox.contadores().gravados >= 1
        })
        .await;

    let mut killed = false;
    for _ in 0..50 {
        if kill(&name) {
            killed = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(killed, "não achei o processo do plugin para matar");

    let turn = TurnId::generate();
    let item = scenario.enqueue_text(turn, "depois da queda");
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut attempts = 0;
    loop {
        attempts += 1;
        scenario
            .flow
            .enviar_pendentes(scenario.org, &transport)
            .await
            .expect("o ciclo de envio termina mesmo com o plugin caindo");
        if scenario.item_state(item.id) == poph_store::OutboxState::Sent {
            break;
        }
        assert!(
            !scenario.supervisor.state().is_closed(),
            "o supervisor não pode desistir do plugin: {:?}",
            scenario.supervisor.state()
        );
        assert!(
            Instant::now() < deadline,
            "o item não foi entregue depois da queda (estado {:?}, tentativas {attempts})",
            scenario.item_state(item.id)
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    scenario
        .wait_until("o plugin voltar a ficar pronto", || {
            scenario.supervisor.state() == IntegrationState::Ready
        })
        .await;
    let counters = scenario.supervisor.counters();
    assert!(
        counters.restarts >= 1,
        "esperava pelo menos um reinício: {counters:?}"
    );

    scenario
        .wait_until("a segunda mensagem chegar", || {
            scenario.inbox.contadores().gravados >= 2
        })
        .await;
    let messages = scenario.messages();
    assert_eq!(messages.len(), 2, "{messages:#?}");
    assert_eq!(messages[0].texto, "setna");
    assert_eq!(messages[1].texto, "adeuq ad sioped");
    assert_eq!(scenario.inbox.contadores().duplicados, 0);

    let health = scenario.channel.health().await.expect("consulta a saúde");
    assert!(health.ok, "{health:?}");

    scenario.shutdown().await;
}
