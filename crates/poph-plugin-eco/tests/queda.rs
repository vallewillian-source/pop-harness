//! Queda do plugin no meio da conversa (HR-27): `kill -9` no processo do plugin não pode derrubar
//! o supervisor, o plugin tem de voltar sozinho e a mensagem não pode se perder nem duplicar.
//!
//! O binário do plugin é copiado com um nome único para que o `kill -9` atinja exatamente o
//! processo deste teste (e nenhum outro plugin rodando na máquina).

mod comum;

use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

use comum::{Cenario, pasta_temporaria};
use poph_agent::fila_integracao::EnvioPeloCanal;
use poph_types::IntegrationState;
use poph_types::TurnId;

fn eco() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_plugin-eco"))
}

/// Pasta da cópia do plugin, apagada no fim (inclusive quando o teste falha).
struct Limpeza(PathBuf);

impl Drop for Limpeza {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Copia o plugin para uma pasta temporária com um nome só deste teste.
fn copia_unica() -> (Limpeza, PathBuf, String) {
    let pasta = pasta_temporaria("queda-bin");
    let nome = format!("plugin-eco-{}-{}", std::process::id(), agora_ms());
    let destino = pasta.join(&nome);
    std::fs::copy(eco(), &destino).expect("copia o binário do plugin");
    (Limpeza(pasta), destino, nome)
}

fn agora_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duracao| duracao.as_millis())
        .unwrap_or(0)
}

/// `kill -9` no processo cujo comando contém o nome único. Devolve `true` se matou algo.
fn matar(nome: &str) -> bool {
    let saida = Command::new("pkill")
        .args(["-9", "-f", nome])
        .output()
        .expect("executa o pkill");
    saida.status.success()
}

#[tokio::test(flavor = "multi_thread")]
async fn kill_9_no_plugin_nao_derruba_o_supervisor_e_a_mensagem_volta() {
    let (_limpeza, programa, nome) = copia_unica();
    let cenario = Cenario::novo("queda", &programa, Some("5511999999999")).await;
    let transporte: EnvioPeloCanal = cenario.transporte();

    // 1. Antes da queda, o caminho funciona.
    cenario.enfileirar_texto(TurnId::generate(), "antes");
    let relato = cenario
        .fluxo
        .enviar_pendentes(cenario.org, &transporte)
        .await
        .expect("envia o primeiro texto");
    assert_eq!(relato.enviados, 1, "{relato:?}");
    cenario
        .esperar("a primeira mensagem chegar", || {
            cenario.entrada.contadores().gravados >= 1
        })
        .await;

    // 2. Mata o processo do plugin, como um `kill -9` de operação.
    let mut matou = false;
    for _ in 0..50 {
        if matar(&nome) {
            matou = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(matou, "não achei o processo do plugin para matar");

    // 3. O supervisor não cai: ele reinicia o plugin (o estado passa por reinício) e volta a
    //    receber. O item da fila é entregue no fim, sem sumir.
    let turno = TurnId::generate();
    let item = cenario.enfileirar_texto(turno, "depois da queda");
    let limite = Instant::now() + Duration::from_secs(10);
    let mut tentativas = 0;
    loop {
        tentativas += 1;
        cenario
            .fluxo
            .enviar_pendentes(cenario.org, &transporte)
            .await
            .expect("o ciclo de envio termina mesmo com o plugin caindo");
        if cenario.estado_do_item(item.id) == poph_store::EstadoSaida::Enviado {
            break;
        }
        assert!(
            !cenario.supervisor.estado().is_closed(),
            "o supervisor não pode desistir do plugin: {:?}",
            cenario.supervisor.estado()
        );
        assert!(
            Instant::now() < limite,
            "o item não foi entregue depois da queda (estado {:?}, tentativas {tentativas})",
            cenario.estado_do_item(item.id)
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    // 4. Voltou sozinho: estado pronto de novo e pelo menos um reinício contado.
    cenario
        .esperar("o plugin voltar a ficar pronto", || {
            cenario.supervisor.estado() == IntegrationState::Ready
        })
        .await;
    let contadores = cenario.supervisor.contadores();
    assert!(
        contadores.reinicios >= 1,
        "esperava pelo menos um reinício: {contadores:?}"
    );

    // 5. Nada duplicado: uma mensagem por entrega distinta, e a que voltou é a mesma do item.
    cenario
        .esperar("a segunda mensagem chegar", || {
            cenario.entrada.contadores().gravados >= 2
        })
        .await;
    let mensagens = cenario.mensagens();
    assert_eq!(mensagens.len(), 2, "{mensagens:#?}");
    assert_eq!(mensagens[0].texto, "setna");
    assert_eq!(mensagens[1].texto, "adeuq ad sioped");
    assert_eq!(cenario.entrada.contadores().duplicados, 0);

    // 6. E o canal continua respondendo depois da queda.
    let saude = cenario.canal.health().await.expect("consulta a saúde");
    assert!(saude.ok, "{saude:?}");

    cenario.encerrar().await;
}
