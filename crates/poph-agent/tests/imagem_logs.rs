//! A imagem do cliente nunca vai para o log (HR-23b).
//!
//! Fica num binário próprio de propósito: o cache de interesse do `tracing` é global, e os outros
//! testes de imagem já disparam os mesmos pontos de log sem assinante. Aqui só há este teste.
//!
//! O que se prova: os bytes da imagem (e o base64 deles) não aparecem no log, nem no caminho em que
//! a imagem entra no pedido, nem nos caminhos de degradação (blob ausente e imagem acima do teto) —
//! que registram o fato com o id do anexo e o tamanho, nunca o conteúdo.

mod apoio;

use std::io;
use std::sync::{Arc, Mutex};

use apoio::{Anexo, Cenario};
use poph_agent::turno::MAX_BYTES_POR_IMAGEM;
use poph_core::llm::{Mensagem, Parte};
use poph_store::EstadoTurno;

/// Escritor que guarda tudo em memória, para ler o que o tracing escreveu.
#[derive(Clone, Default)]
struct Captura(Arc<Mutex<Vec<u8>>>);

impl io::Write for Captura {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.lock().expect("trava").extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Captura {
    fn texto(&self) -> String {
        let bytes = self.0.lock().expect("trava").clone();
        String::from_utf8_lossy(&bytes).into_owned()
    }
}

#[tokio::test]
async fn imagem_do_cliente_nao_vai_para_o_log() {
    let captura = Captura::default();
    let assinante = tracing_subscriber::fmt()
        .json()
        .with_max_level(tracing::Level::DEBUG)
        .with_writer({
            let captura = captura.clone();
            move || captura.clone()
        })
        .finish();
    let _guarda = tracing::subscriber::set_default(assinante);
    // Outros testes deste binário já dispararam os mesmos pontos de log sem assinante; sem isto o
    // cache de interesse do tracing os marca como "ignorar" e o log não aparece.
    tracing::callsite::rebuild_interest_cache();

    let cenario = Cenario::novo("logs");
    let segredo = b"BYTES-SECRETOS-DA-IMAGEM-DO-CLIENTE-4f1c9a".to_vec();
    let segredo_base64 = poph_core::llm::base64::codificar(&segredo);

    // Três anexos num turno: a imagem que entra no pedido, uma cujo blob não está no disco e uma
    // acima do teto de bytes. Os dois últimos exercitam os avisos do caminho de degradação.
    let mut ausente = Anexo::imagem("foto-sumida.png", b"bytes-da-foto-sumida");
    ausente.sem_blob = true;
    let mut grande = Anexo::imagem("foto-grande.png", b"x");
    grande.tamanho_declarado = Some(MAX_BYTES_POR_IMAGEM + 1);
    let gatilho = cenario.falar_com_arquivos(
        "olha as fotos e o contrato",
        vec![
            Anexo::imagem("foto-boa.png", &segredo),
            Anexo::documento("contrato.pdf", "cláusula primeira: prazo de 5 dias"),
            ausente,
            grande,
        ],
    );
    let (motor, fake) = cenario.motor(true);

    let resumo = cenario.turno(&motor, gatilho).await;

    assert_eq!(resumo.estado, EstadoTurno::Concluido, "{:?}", resumo.erro);

    // A imagem boa chegou ao modelo: o log é o do caminho real, não de um turno vazio.
    let pedido = &fake.recebidos()[0];
    let achadas: Vec<&Vec<u8>> = pedido
        .mensagens
        .iter()
        .filter_map(|mensagem| match mensagem {
            Mensagem::Usuario(partes) => Some(partes),
            _ => None,
        })
        .flatten()
        .filter_map(|parte| match parte {
            Parte::Imagem { bytes, .. } => Some(bytes),
            _ => None,
        })
        .collect();
    assert_eq!(achadas, vec![&segredo], "só a imagem boa vai ao modelo");

    let log = captura.texto();
    assert!(
        log.contains("imagem omitida do turno") && log.contains("não consegui ler a imagem"),
        "os avisos de degradação têm de estar no log; veio: {log}"
    );
    assert!(
        !log.contains("BYTES-SECRETOS-DA-IMAGEM"),
        "os bytes da imagem não podem aparecer no log"
    );
    assert!(
        !log.contains(&segredo_base64),
        "o base64 da imagem não pode aparecer no log"
    );
    assert!(
        !log.contains("bytes-da-foto-sumida"),
        "nem os bytes de uma imagem que não foi lida"
    );
}
