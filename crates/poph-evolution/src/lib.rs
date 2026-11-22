//! Integração nativa do WhatsApp (EvolutionAPI) no POP Harness — HR-26.
//!
//! É o primeiro canal **nativo** (em Rust, no mesmo processo) do contrato da HR-25, ao lado do
//! plugin em processo externo. O que ele cobre, com as rotas e as regras do POP Atende:
//!
//! - [`cliente::ClienteEvolution`] — as chamadas da EvolutionAPI v2: criar instância, parear (QR),
//!   estado, desconectar, apagar, definir webhook, enviar texto, enviar mídia e baixar mídia.
//! - [`webhook`] — a entrada: `messages.upsert` vira [`poph_types::InboundEvent`] (texto, imagem,
//!   PDF/Office), `connection.update`/`qrcode.updated` viram [`poph_types::EventoEstado`], e o
//!   segredo do webhook é conferido em tempo constante.
//! - [`texto`] — a regra do canal reproduzida do POP Atende: Markdown convertido para o formato do
//!   WhatsApp e corte em 4000 caracteres, porque a Evolution trunca sozinha e trunca pior.
//! - [`integracao::WhatsAppEvolution`] — o canal como [`poph_types::Integration`], pronto para a
//!   `outbox` (`EnvioPeloCanal`) e para a `inbox` (`InboxDoCanal`) da HR-12.
//!
//! O relatório das três verificações que a tarefa pede — reenvio do webhook em 5xx, autenticação do
//! webhook e a regra de mensagem longa — está em `docs/EVOLUTION_NOTAS.md`, com a linha de código do
//! POP Atende que serve de evidência em cada uma.
//!
//! ```no_run
//! # async fn exemplo() -> Result<(), Box<dyn std::error::Error>> {
//! use std::sync::Arc;
//! use poph_evolution::{ConfigCanal, ClienteEvolution, ConfigEvolution, WhatsAppEvolution, BlobsEmDisco};
//! use poph_types::{IntegrationId, Segredo};
//!
//! let cliente = ClienteEvolution::novo(ConfigEvolution::novo(
//!     "http://127.0.0.1:8080",
//!     "pop-atende",
//!     Segredo::new("chave-da-evolution".to_owned()),
//! )?)?;
//! let config = ConfigCanal::novo(
//!     IntegrationId::generate(),
//!     Segredo::new("segredo-do-webhook".to_owned()),
//! )?
//! .com_webhook("https://harness.exemplo/v1/webhooks/whatsapp");
//! let canal = WhatsAppEvolution::novo(cliente, config, Arc::new(BlobsEmDisco::nova("data/blobs")));
//! canal.iniciar().await?;
//! # Ok(())
//! # }
//! ```

pub mod cliente;
pub mod erro;
pub mod integracao;
pub mod texto;
pub mod webhook;

pub use cliente::{
    ClienteEvolution, ConfigEvolution, MidiaBaixada, Pareamento, TETO_CORPO_PADRAO, TETO_MIDIA,
};
pub use erro::ErroEvolution;
pub use integracao::{BlobsEmDisco, ConfigCanal, FonteDeMidia, WhatsAppEvolution};
pub use texto::{LIMITE_TEXTO, cortar, para_whatsapp};
pub use webhook::{
    CABECALHO_SEGREDO, Decisao, EntradaRecebida, ErroWebhook, MidiaParaBaixar, MotivoIgnorado,
    Politica, anexo_embutido, conferir_segredo, interpretar,
};
