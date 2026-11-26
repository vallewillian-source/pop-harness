mod common;

use std::path::PathBuf;

use common::Scenario;
use poph_agent::queue_integration::SendByChannel;
use poph_store::{NewOutboxItem, OutboxState};
use poph_types::{OutboxId, SendText, TurnId};

fn plugin_binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_plugin-eco"))
}

#[tokio::test(flavor = "multi_thread")]
async fn text_goes_out_through_the_plugin_and_comes_back_inverted_through_the_inbox() {
    let scenario = Scenario::new("conversa", &plugin_binary(), Some("5511999999999")).await;

    let turn = TurnId::generate();
    let item = scenario.enqueue_text(turn, "olá, mundo");
    assert_eq!(scenario.item_state(item.id), OutboxState::Pending);

    let transport = scenario.transport();
    let report = scenario
        .flow
        .send_pending(scenario.org, &transport)
        .await
        .expect("envia o pendente");
    assert_eq!(report.sent, 1, "{report:?}");
    assert_eq!(scenario.item_state(item.id), OutboxState::Sent);

    scenario
        .wait_until("o evento chegar na inbox", || {
            scenario.inbox.counters().stored >= 1
        })
        .await;
    let messages = scenario.messages();
    assert_eq!(messages.len(), 1, "{messages:#?}");
    assert_eq!(messages[0].texto, "odnum ,álo");
    assert_eq!(messages[0].direction.as_text(), "in");
    assert_eq!(
        messages[0].external_id.as_deref(),
        Some(&*format!("eco-{turn}:0"))
    );

    scenario
        .channel
        .send_text(SendText {
            idempotency: format!("{turn}:0"),
            peer: "5511999999999".to_owned(),
            text: "olá, mundo".to_owned(),
        })
        .await
        .expect("o plugin aceita a repetição");
    scenario
        .wait_until("a repetição ser descartada", || {
            scenario.inbox.counters().duplicated >= 1
        })
        .await;

    let counters = scenario.inbox.counters();
    assert_eq!(counters.stored, 1, "{counters:?}");
    assert_eq!(counters.duplicated, 1, "{counters:?}");
    assert_eq!(counters.failures, 0, "{counters:?}");
    assert_eq!(
        scenario.messages().len(),
        1,
        "a conversa não pode ganhar repetição"
    );

    let health = scenario.channel.health().await.expect("consulta a saúde");
    assert!(health.ok, "{health:?}");
    assert_eq!(
        scenario
            .store
            .outbox()
            .count_by_state(scenario.org, OutboxState::Pending)
            .expect("conta pendentes"),
        0
    );

    scenario.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn library_file_becomes_a_send_command() {
    let scenario = Scenario::new("arquivo", &plugin_binary(), Some("5511999999999")).await;

    let turn = TurnId::generate();
    let sha256 = "b".repeat(64);
    let item = scenario
        .store
        .outbox()
        .enqueue(
            scenario.org,
            NewOutboxItem {
                conversation_id: scenario.conversation_id(),
                turn_id: turn,
                seq: 4,
                tipo: "file".to_owned(),
                content_json: serde_json::json!({
                    "blob_ref": format!("{}/{sha256}", scenario.org),
                    "arquivo_id": "01a1f6a5-cd9b-7000-8000-000000000000",
                    "nome": "manual.pdf",
                    "mime": "application/pdf",
                    "tamanho_bytes": 1234,
                    "sha256": sha256,
                })
                .to_string(),
            },
        )
        .expect("enfileira o arquivo")
        .1;

    let transport = scenario.transport();
    scenario
        .flow
        .send_pending(scenario.org, &transport)
        .await
        .expect("envia o arquivo");
    assert_eq!(scenario.item_state(item.id), OutboxState::Sent);

    assert_eq!(scenario.inbox.counters().stored, 0);
    assert!(scenario.messages().is_empty());

    scenario.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn conversation_without_a_contact_fails_without_blocking_the_queue() {
    let scenario = Scenario::new("sem-peer", &plugin_binary(), None).await;

    let item = scenario.enqueue_text(TurnId::generate(), "oi");
    let transport = scenario.transport();
    let report = scenario
        .flow
        .send_pending(scenario.org, &transport)
        .await
        .expect("o ciclo termina");

    assert_eq!(report.failed, 1, "{report:?}");
    let stored = scenario
        .store
        .outbox()
        .get(scenario.org, item.id)
        .expect("lê o item");
    assert_eq!(stored.state, OutboxState::Failed);
    let reason = stored.ultimo_erro.unwrap_or_default();
    assert!(reason.contains("peer"), "{reason}");

    assert_eq!(scenario.inbox.counters().stored, 0);
    assert_eq!(scenario.supervisor.counters().commands, 0);

    scenario.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn agent_without_a_contact_uses_the_one_from_the_event() {
    let scenario = Scenario::new("dois-contatos", &plugin_binary(), Some("5511911111111")).await;

    scenario
        .channel
        .send_text(SendText {
            idempotency: "turno-a:0".to_owned(),
            peer: "5511911111111".to_owned(),
            text: "abc".to_owned(),
        })
        .await
        .expect("envia para o contato da conversa");
    scenario
        .wait_until("gravar o primeiro contato", || {
            scenario.inbox.counters().stored >= 1
        })
        .await;

    scenario
        .channel
        .send_text(SendText {
            idempotency: "turno-b:0".to_owned(),
            peer: "5511922222222".to_owned(),
            text: "xyz".to_owned(),
        })
        .await
        .expect("envia para outro contato");
    scenario
        .wait_until("gravar o segundo contato", || {
            scenario.inbox.counters().stored >= 2
        })
        .await;

    let from_the_first = scenario.messages();
    assert_eq!(from_the_first.len(), 1);
    assert_eq!(from_the_first[0].texto, "cba");

    let conversations = scenario
        .store
        .conversations()
        .list(scenario.org, None, poph_store::Page::with_limit(10))
        .expect("lista conversas");
    assert_eq!(conversations.len(), 2, "cada contato tem a sua conversa");

    scenario.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn transport_rejects_item_of_unknown_type_without_calling_the_plugin() {
    let scenario = Scenario::new("tipo-estranho", &plugin_binary(), Some("5511999999999")).await;

    let item = scenario
        .store
        .outbox()
        .enqueue(
            scenario.org,
            NewOutboxItem {
                conversation_id: scenario.conversation_id(),
                turn_id: TurnId::generate(),
                seq: 1,
                tipo: "text".to_owned(),
                content_json: serde_json::json!({"outro": 1}).to_string(),
            },
        )
        .expect("enfileira")
        .1;

    let transport: SendByChannel = scenario.transport();
    let report = scenario
        .flow
        .send_pending(scenario.org, &transport)
        .await
        .expect("o ciclo termina");
    assert_eq!(report.failed, 1, "{report:?}");
    let stored: OutboxId = item.id;
    assert_eq!(
        scenario.item_state(stored),
        OutboxState::Failed,
        "payload sem `texto` é definitivo"
    );

    scenario.shutdown().await;
}
