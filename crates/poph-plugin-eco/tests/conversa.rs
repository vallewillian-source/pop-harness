//! Conversa de teste pelo caminho que o WhatsApp vai usar (HR-27), sem WhatsApp:
//!
//! ```text
//! outbox (item de texto) → supervisor → plugin eco → evento de entrada → inbox → mensagem
//! ```
//!
//! O plugin é um processo filho de verdade (`plugin-eco`), então o que se prova aqui é o protocolo
//! e a ligação com as filas — não um dublê em memória.

mod comum;

use std::path::PathBuf;

use comum::Cenario;
use poph_agent::fila_integracao::EnvioPeloCanal;
use poph_store::{NewOutboxItem, OutboxState};
use poph_types::{OutboxId, SendText, TurnId};

/// Binário do plugin que o cargo compila antes de rodar este teste.
fn eco() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_plugin-eco"))
}

#[tokio::test(flavor = "multi_thread")]
async fn texto_sai_pelo_plugin_e_volta_invertido_pela_inbox() {
    let cenario = Cenario::novo("conversa", &eco(), Some("5511999999999")).await;

    // 1. Um item de texto na fila de saída, como o turno deixaria.
    let turno = TurnId::generate();
    let item = cenario.enfileirar_texto(turno, "olá, mundo");
    assert_eq!(cenario.estado_do_item(item.id), OutboxState::Pending);

    // 2. O plugin recebe, confirma e devolve o evento.
    let transporte = cenario.transporte();
    let relato = cenario
        .fluxo
        .enviar_pendentes(cenario.org, &transporte)
        .await
        .expect("envia o pendente");
    assert_eq!(relato.enviados, 1, "{relato:?}");
    assert_eq!(cenario.estado_do_item(item.id), OutboxState::Sent);

    // 3. O evento do plugin virou mensagem na conversa do contato, com o texto invertido.
    cenario
        .esperar("o evento chegar na inbox", || {
            cenario.entrada.contadores().gravados >= 1
        })
        .await;
    let mensagens = cenario.mensagens();
    assert_eq!(mensagens.len(), 1, "{mensagens:#?}");
    assert_eq!(mensagens[0].texto, "odnum ,álo");
    assert_eq!(mensagens[0].direction.as_text(), "in");
    assert_eq!(
        mensagens[0].id_externo.as_deref(),
        Some(&*format!("eco-{turno}:0"))
    );

    // 4. A mesma entrega de novo (o `id_externo` sai da chave de idempotência) não duplica.
    cenario
        .canal
        .send_text(SendText {
            idempotency: format!("{turno}:0"),
            peer: "5511999999999".to_owned(),
            text: "olá, mundo".to_owned(),
        })
        .await
        .expect("o plugin aceita a repetição");
    cenario
        .esperar("a repetição ser descartada", || {
            cenario.entrada.contadores().duplicados >= 1
        })
        .await;

    let contadores = cenario.entrada.contadores();
    assert_eq!(contadores.gravados, 1, "{contadores:?}");
    assert_eq!(contadores.duplicados, 1, "{contadores:?}");
    assert_eq!(contadores.falhas, 0, "{contadores:?}");
    assert_eq!(
        cenario.mensagens().len(),
        1,
        "a conversa não pode ganhar repetição"
    );

    // 5. O canal continua saudável e a fila de saída está vazia.
    let saude = cenario.canal.health().await.expect("consulta a saúde");
    assert!(saude.ok, "{saude:?}");
    assert_eq!(
        cenario
            .store
            .outbox()
            .contar_por_estado(cenario.org, OutboxState::Pending)
            .expect("conta pendentes"),
        0
    );

    cenario.encerrar().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn arquivo_da_biblioteca_vira_comando_de_envio() {
    let cenario = Cenario::novo("arquivo", &eco(), Some("5511999999999")).await;

    // O item `file` tem a forma que a ferramenta `enviar_arquivo` grava (HR-24).
    let turno = TurnId::generate();
    let sha256 = "b".repeat(64);
    let item = cenario
        .store
        .outbox()
        .enfileirar(
            cenario.org,
            NewOutboxItem {
                conversation_id: cenario.conversa_id(),
                turn_id: turno,
                seq: 4,
                tipo: "file".to_owned(),
                content_json: serde_json::json!({
                    "blob_ref": format!("{}/{sha256}", cenario.org),
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

    let transporte = cenario.transporte();
    cenario
        .fluxo
        .enviar_pendentes(cenario.org, &transporte)
        .await
        .expect("envia o arquivo");
    assert_eq!(cenario.estado_do_item(item.id), OutboxState::Sent);

    // O eco não devolve evento para arquivo: só a confirmação. Nada entra na inbox.
    assert_eq!(cenario.entrada.contadores().gravados, 0);
    assert!(cenario.mensagens().is_empty());

    cenario.encerrar().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn conversa_sem_contato_falha_sem_travar_a_fila() {
    let cenario = Cenario::novo("sem-peer", &eco(), None).await;

    let item = cenario.enfileirar_texto(TurnId::generate(), "oi");
    let transporte = cenario.transporte();
    let relato = cenario
        .fluxo
        .enviar_pendentes(cenario.org, &transporte)
        .await
        .expect("o ciclo termina");

    // Falha definitiva (não adianta retentar sem destino), com motivo legível, e o item sai da fila.
    assert_eq!(relato.falharam, 1, "{relato:?}");
    let gravado = cenario
        .store
        .outbox()
        .obter(cenario.org, item.id)
        .expect("lê o item");
    assert_eq!(gravado.estado, OutboxState::Failed);
    let motivo = gravado.ultimo_erro.unwrap_or_default();
    assert!(motivo.contains("peer"), "{motivo}");

    // O canal não foi incomodado.
    assert_eq!(cenario.entrada.contadores().gravados, 0);
    assert_eq!(cenario.supervisor.contadores().comandos, 0);

    cenario.encerrar().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn agente_sem_contato_usa_o_do_evento() {
    // O `peer` do evento é o que decide a conversa: dois contatos diferentes não se misturam.
    let cenario = Cenario::novo("dois-contatos", &eco(), Some("5511911111111")).await;

    cenario
        .canal
        .send_text(SendText {
            idempotency: "turno-a:0".to_owned(),
            peer: "5511911111111".to_owned(),
            text: "abc".to_owned(),
        })
        .await
        .expect("envia para o contato da conversa");
    cenario
        .esperar("gravar o primeiro contato", || {
            cenario.entrada.contadores().gravados >= 1
        })
        .await;

    // O segundo contato abre conversa nova pelo resolvedor, sem tocar na primeira.
    cenario
        .canal
        .send_text(SendText {
            idempotency: "turno-b:0".to_owned(),
            peer: "5511922222222".to_owned(),
            text: "xyz".to_owned(),
        })
        .await
        .expect("envia para outro contato");
    cenario
        .esperar("gravar o segundo contato", || {
            cenario.entrada.contadores().gravados >= 2
        })
        .await;

    let da_primeira = cenario.mensagens();
    assert_eq!(da_primeira.len(), 1);
    assert_eq!(da_primeira[0].texto, "cba");

    let conversas = cenario
        .store
        .conversas()
        .listar(cenario.org, None, poph_store::Page::with_limit(10))
        .expect("lista conversas");
    assert_eq!(conversas.len(), 2, "cada contato tem a sua conversa");

    cenario.encerrar().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn transporte_recusa_item_de_tipo_desconhecido_sem_chamar_o_plugin() {
    let cenario = Cenario::novo("tipo-estranho", &eco(), Some("5511999999999")).await;

    let item = cenario
        .store
        .outbox()
        .enfileirar(
            cenario.org,
            NewOutboxItem {
                conversation_id: cenario.conversa_id(),
                turn_id: TurnId::generate(),
                seq: 1,
                tipo: "text".to_owned(),
                content_json: serde_json::json!({"outro": 1}).to_string(),
            },
        )
        .expect("enfileira")
        .1;

    let transporte: EnvioPeloCanal = cenario.transporte();
    let relato = cenario
        .fluxo
        .enviar_pendentes(cenario.org, &transporte)
        .await
        .expect("o ciclo termina");
    assert_eq!(relato.falharam, 1, "{relato:?}");
    let gravado: OutboxId = item.id;
    assert_eq!(
        cenario.estado_do_item(gravado),
        OutboxState::Failed,
        "payload sem `texto` é definitivo"
    );

    cenario.encerrar().await;
}
