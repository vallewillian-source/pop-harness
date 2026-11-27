use std::time::{SystemTime, UNIX_EPOCH};

use poph_matrix::client::{ClientConfig, CreateRoom, MatrixClient};
use poph_types::Secret;
use serde_json::json;

fn credential(name: &str) -> String {
    let path = std::env::var("HOME").expect("HOME") + "/.cache/poph-matrix/credenciais.env";
    let content = std::fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "não consegui ler {path} ({error}): suba o ambiente com \
             `bash apps/pop-harness/scripts/matrix-dev.sh subir`"
        )
    });
    content
        .lines()
        .filter_map(|line| line.split_once('='))
        .find(|(key, _)| key.trim() == name)
        .map(|(_, value)| value.trim().trim_matches('"').to_owned())
        .unwrap_or_else(|| panic!("{name} ausente em {path}"))
}

fn client() -> (MatrixClient, String) {
    let base = credential("POPH_MATRIX_DEV_BASE_URL");
    let server_name = credential("POPH_MATRIX_DEV_SERVER_NAME");
    let token = credential("POPH_MATRIX_DEV_HARNESS_AS_TOKEN");
    let config = ClientConfig::new(base, server_name.clone(), Secret::new(token))
        .expect("configuração do cliente");
    let client = MatrixClient::new(config).expect("cliente");
    (client, server_name)
}

#[tokio::test]
#[ignore = "precisa do Synapse de desenvolvimento da MX-01 no ar"]
async fn all_calls_against_development_synapse() {
    let (client, server_name) = client();
    let bot = format!("@ph_as_bot:{server_name}");
    let marker = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("relógio")
        .as_secs();
    println!("server_name: {server_name}");
    println!("bot do AS: {bot}");

    let room = client
        .create_room(
            &bot,
            &CreateRoom {
                room_alias_name: Some(format!("ph_c_teste_{marker}")),
                name: Some(format!("Teste MX-09 {marker}")),
                topic: Some("sala descartável do teste do cliente".to_owned()),
                creation_content: Some(json!({"type": "ph.teste"})),
                ..Default::default()
            },
        )
        .await
        .expect("createRoom");
    println!("createRoom -> {room}");

    let localpart = format!("ph_wa_teste{marker}");
    let user = client
        .register_user(&localpart)
        .await
        .expect("register de usuário virtual");
    println!("register -> {user}");
    client
        .set_display_name(&user, &user, "Cliente Teste (WhatsApp)")
        .await
        .expect("profile displayname");
    let profile = client.profile(&user).await.expect("profile GET");
    println!("profile -> {:?}", profile.displayname);
    assert_eq!(
        profile.displayname.as_deref(),
        Some("Cliente Teste (WhatsApp)")
    );

    client.invite(&bot, &room, &user).await.expect("invite");
    let entered = client.enter(&user, &room).await.expect("join do fantasma");
    assert_eq!(entered, room);
    let members = client.members(&bot, &room).await.expect("joined_members");
    println!("joined_members -> {:?}", members.keys().collect::<Vec<_>>());
    assert!(members.contains_key(&user), "o fantasma não entrou");
    assert!(members.contains_key(&bot), "o bot não está na sala");

    let body = json!({"msgtype": "m.text", "body": "olá do teste MX-09"});
    let txn = format!("ph-teste-{marker}");
    let event = client
        .send_event(&user, &room, "m.room.message", &txn, &body)
        .await
        .expect("send m.room.message");
    let repeated = client
        .send_event(&user, &room, "m.room.message", &txn, &body)
        .await
        .expect("send repetido com o mesmo txnId");
    println!("send -> {event} (repetido -> {repeated})");
    assert_eq!(event, repeated, "txnId repetido duplicou o evento");

    client
        .send_state(
            &bot,
            &room,
            "ph.teste.estado",
            None,
            &json!({"ok": true, "n": marker}),
        )
        .await
        .expect("state PUT");
    let state = client
        .read_state(&bot, &room, "ph.teste.estado", None)
        .await
        .expect("state GET");
    println!("state -> {state}");
    assert_eq!(state["ok"], true);

    let space = client
        .create_room(
            &bot,
            &CreateRoom {
                room_alias_name: Some(format!("ph_s_teste_{marker}")),
                name: Some(format!("Espaço de teste {marker}")),
                creation_content: Some(json!({"type": "m.space"})),
                ..Default::default()
            },
        )
        .await
        .expect("createRoom do espaço");
    client
        .space_child(&bot, &space, &room, Some("10"))
        .await
        .expect("m.space.child");
    let child = client
        .read_state(&bot, &space, "m.space.child", Some(&room))
        .await
        .expect("m.space.child GET");
    println!("m.space.child -> {child}");
    assert_eq!(child["via"], json!([server_name]));

    println!("todas as chamadas passaram no Synapse de desenvolvimento");
}
