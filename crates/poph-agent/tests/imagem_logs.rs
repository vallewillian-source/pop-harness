mod apoio;

use std::io;
use std::sync::{Arc, Mutex};

use apoio::{Anexo, Scenario};
use poph_agent::turno::MAX_BYTES_PER_IMAGE;
use poph_core::llm::{Message, Part};
use poph_store::TurnState;

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
    let _guard = tracing::subscriber::set_default(assinante);
    tracing::callsite::rebuild_interest_cache();

    let scenario = Scenario::new("logs");
    let segredo = b"BYTES-SECRETOS-DA-IMAGEM-DO-CLIENTE-4f1c9a".to_vec();
    let segredo_base64 = poph_core::llm::base64::encode(&segredo);

    let mut ausente = Anexo::imagem("foto-sumida.png", b"bytes-da-foto-sumida");
    ausente.sem_blob = true;
    let mut large = Anexo::imagem("foto-grande.png", b"x");
    large.tamanho_declarado = Some(MAX_BYTES_PER_IMAGE + 1);
    let gatilho = scenario.falar_com_arquivos(
        "olha as fotos e o contrato",
        vec![
            Anexo::imagem("foto-boa.png", &segredo),
            Anexo::documento("contrato.pdf", "cláusula primeira: prazo de 5 dias"),
            ausente,
            large,
        ],
    );
    let (engine, fake) = scenario.engine(true);

    let summary = scenario.turno(&engine, gatilho).await;

    assert_eq!(summary.estado, TurnState::Completed, "{:?}", summary.error);

    let request = &fake.received()[0];
    let achadas: Vec<&Vec<u8>> = request
        .messages
        .iter()
        .filter_map(|mensagem| match mensagem {
            Message::User(parts) => Some(parts),
            _ => None,
        })
        .flatten()
        .filter_map(|parte| match parte {
            Part::Image { bytes, .. } => Some(bytes),
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
