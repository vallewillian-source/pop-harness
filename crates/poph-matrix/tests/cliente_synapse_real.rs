//! Teste do cliente contra o Synapse **real de desenvolvimento** da MX-01 (MX-09).
//!
//! Fica `#[ignore]`: não roda no CI (que não tem Synapse) e precisa do ambiente de pé em
//! `~/.cache/poph-matrix/credenciais.env`. Rodar com:
//!
//! ```text
//! bash apps/pop-harness/scripts/matrix-dev.sh subir
//! cargo test -p poph-matrix --test cliente_synapse_real -- --ignored --nocapture
//! ```
//!
//! O teste cria salas e um usuário virtual com marcação de tempo no nome; é ambiente descartável.

use std::time::{SystemTime, UNIX_EPOCH};

use poph_matrix::cliente::{ClienteMatrix, ConfigCliente, CriarSala};
use poph_types::Segredo;
use serde_json::json;

/// Lê uma chave do `credenciais.env` sem imprimir valor nenhum.
fn credencial(nome: &str) -> String {
    let caminho = std::env::var("HOME").expect("HOME") + "/.cache/poph-matrix/credenciais.env";
    let conteudo = std::fs::read_to_string(&caminho).unwrap_or_else(|erro| {
        panic!(
            "não consegui ler {caminho} ({erro}): suba o ambiente com \
             `bash apps/pop-harness/scripts/matrix-dev.sh subir`"
        )
    });
    conteudo
        .lines()
        .filter_map(|linha| linha.split_once('='))
        .find(|(chave, _)| chave.trim() == nome)
        .map(|(_, valor)| valor.trim().trim_matches('"').to_owned())
        .unwrap_or_else(|| panic!("{nome} ausente em {caminho}"))
}

fn cliente() -> (ClienteMatrix, String) {
    let base = credencial("POPH_MATRIX_DEV_BASE_URL");
    let server_name = credencial("POPH_MATRIX_DEV_SERVER_NAME");
    let token = credencial("POPH_MATRIX_DEV_HARNESS_AS_TOKEN");
    let config = ConfigCliente::novo(base, server_name.clone(), Segredo::new(token))
        .expect("configuração do cliente");
    let cliente = ClienteMatrix::novo(config).expect("cliente");
    (cliente, server_name)
}

#[tokio::test]
#[ignore = "precisa do Synapse de desenvolvimento da MX-01 no ar"]
async fn todas_as_chamadas_contra_o_synapse_de_desenvolvimento() {
    let (cliente, server_name) = cliente();
    let bot = format!("@ph_as_bot:{server_name}");
    let marcador = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("relógio")
        .as_secs();
    println!("server_name: {server_name}");
    println!("bot do AS: {bot}");

    // 1. Sala da conversa.
    let sala = cliente
        .criar_sala(
            &bot,
            &CriarSala {
                room_alias_name: Some(format!("ph_c_teste_{marcador}")),
                name: Some(format!("Teste MX-09 {marcador}")),
                topic: Some("sala descartável do teste do cliente".to_owned()),
                creation_content: Some(json!({"type": "ph.teste"})),
                ..Default::default()
            },
        )
        .await
        .expect("createRoom");
    println!("createRoom -> {sala}");

    // 2. Usuário virtual (cliente do WhatsApp) e nome de exibição.
    let localpart = format!("ph_wa_teste{marcador}");
    let usuario = cliente
        .registrar_usuario(&localpart)
        .await
        .expect("register de usuário virtual");
    println!("register -> {usuario}");
    cliente
        .definir_nome_exibicao(&usuario, &usuario, "Cliente Teste (WhatsApp)")
        .await
        .expect("profile displayname");
    let perfil = cliente.perfil(&usuario).await.expect("profile GET");
    println!("profile -> {:?}", perfil.displayname);
    assert_eq!(
        perfil.displayname.as_deref(),
        Some("Cliente Teste (WhatsApp)")
    );

    // 3. Convite e entrada do fantasma; membros da sala.
    cliente
        .convidar(&bot, &sala, &usuario)
        .await
        .expect("invite");
    let entrou = cliente
        .entrar(&usuario, &sala)
        .await
        .expect("join do fantasma");
    assert_eq!(entrou, sala);
    let membros = cliente.membros(&bot, &sala).await.expect("joined_members");
    println!("joined_members -> {:?}", membros.keys().collect::<Vec<_>>());
    assert!(membros.contains_key(&usuario), "o fantasma não entrou");
    assert!(membros.contains_key(&bot), "o bot não está na sala");

    // 4. Mensagem do fantasma do cliente (com txnId) e repetição segura do mesmo txnId.
    let corpo = json!({"msgtype": "m.text", "body": "olá do teste MX-09"});
    let txn = format!("ph-teste-{marcador}");
    let evento = cliente
        .enviar_evento(&usuario, &sala, "m.room.message", &txn, &corpo)
        .await
        .expect("send m.room.message");
    let repetido = cliente
        .enviar_evento(&usuario, &sala, "m.room.message", &txn, &corpo)
        .await
        .expect("send repetido com o mesmo txnId");
    println!("send -> {evento} (repetido -> {repetido})");
    assert_eq!(evento, repetido, "txnId repetido duplicou o evento");

    // 5. Estado sem chave: escreve e lê de volta.
    cliente
        .enviar_estado(
            &bot,
            &sala,
            "ph.teste.estado",
            None,
            &json!({"ok": true, "n": marcador}),
        )
        .await
        .expect("state PUT");
    let estado = cliente
        .ler_estado(&bot, &sala, "ph.teste.estado", None)
        .await
        .expect("state GET");
    println!("state -> {estado}");
    assert_eq!(estado["ok"], true);

    // 6. Espaço do departamento e sala filha (m.space.child).
    let espaco = cliente
        .criar_sala(
            &bot,
            &CriarSala {
                room_alias_name: Some(format!("ph_s_teste_{marcador}")),
                name: Some(format!("Espaço de teste {marcador}")),
                creation_content: Some(json!({"type": "m.space"})),
                ..Default::default()
            },
        )
        .await
        .expect("createRoom do espaço");
    cliente
        .espaco_filho(&bot, &espaco, &sala, Some("10"))
        .await
        .expect("m.space.child");
    let filho = cliente
        .ler_estado(&bot, &espaco, "m.space.child", Some(&sala))
        .await
        .expect("m.space.child GET");
    println!("m.space.child -> {filho}");
    assert_eq!(filho["via"], json!([server_name]));

    println!("todas as chamadas passaram no Synapse de desenvolvimento");
}
