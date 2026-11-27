mod support;

use std::io;
use std::sync::{Arc, Mutex};

use poph_agent::turno::MAX_BYTES_PER_IMAGE;
use poph_core::llm::{Message, Part};
use poph_store::TurnState;
use support::{Attachment, Scenario};

#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<u8>>>);

impl io::Write for Capture {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.lock().expect("trava").extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Capture {
    fn texto(&self) -> String {
        let bytes = self.0.lock().expect("trava").clone();
        String::from_utf8_lossy(&bytes).into_owned()
    }
}

#[tokio::test]
async fn image_client_not_goes_for_log() {
    let capture = Capture::default();
    let assinante = tracing_subscriber::fmt()
        .json()
        .with_max_level(tracing::Level::DEBUG)
        .with_writer({
            let capture = capture.clone();
            move || capture.clone()
        })
        .finish();
    let _guard = tracing::subscriber::set_default(assinante);
    tracing::callsite::rebuild_interest_cache();

    let scenario = Scenario::new("logs");
    let secret = b"BYTES-SECRETOS-DA-IMAGEM-DO-CLIENTE-4f1c9a".to_vec();
    let secret_base64 = poph_core::llm::base64::encode(&secret);

    let mut missing = Attachment::image("foto-sumida.png", b"bytes-da-foto-sumida");
    missing.without_blob = true;
    let mut large = Attachment::image("foto-grande.png", b"x");
    large.size_declared = Some(MAX_BYTES_PER_IMAGE + 1);
    let gatilho = scenario.speak_with_files(
        "olha as fotos e o contrato",
        vec![
            Attachment::image("foto-boa.png", &secret),
            Attachment::document("contrato.pdf", "cláusula primeira: prazo de 5 dias"),
            missing,
            large,
        ],
    );
    let (engine, fake) = scenario.engine(true);

    let summary = scenario.turno(&engine, gatilho).await;

    assert_eq!(summary.estado, TurnState::Completed, "{:?}", summary.error);

    let request = &fake.received()[0];
    let found: Vec<&Vec<u8>> = request
        .messages
        .iter()
        .filter_map(|mensagem| match mensagem {
            Message::User(parts) => Some(parts),
            _ => None,
        })
        .flatten()
        .filter_map(|part| match part {
            Part::Image { bytes, .. } => Some(bytes),
            _ => None,
        })
        .collect();
    assert_eq!(found, vec![&secret], "só a imagem boa vai ao modelo");

    let log = capture.texto();
    assert!(
        log.contains("imagem omitida do turno") && log.contains("não consegui ler a imagem"),
        "os avisos de degradação têm de estar no log; veio: {log}"
    );
    assert!(
        !log.contains("BYTES-SECRETOS-DA-IMAGEM"),
        "os bytes da imagem não podem aparecer no log"
    );
    assert!(
        !log.contains(&secret_base64),
        "o base64 da imagem não pode aparecer no log"
    );
    assert!(
        !log.contains("bytes-da-foto-sumida"),
        "nem os bytes de uma imagem que não foi lida"
    );
}
