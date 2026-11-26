use std::fmt;
use std::sync::Arc;

use poph_types::OrgId;
use serde_json::Value;
use tokio::sync::watch;

#[derive(Clone, PartialEq, Eq)]
pub enum Part {
    Texto(String),
    Image { mime: String, bytes: Vec<u8> },
}

impl fmt::Debug for Part {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Part::Texto(texto) => write!(f, "Texto({} caracteres)", texto.chars().count()),
            Part::Image { mime, bytes } => write!(f, "Imagem({mime}, {} bytes)", bytes.len()),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Tool {
    pub nome: String,
    pub descricao: String,
    pub parameters: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolCall {
    pub id: String,
    pub nome: String,
    pub arguments: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Message {
    System(String),
    User(Vec<Part>),
    Assistant { texto: String, calls: Vec<ToolCall> },
    Resultado { id: String, content: String },
}

#[derive(Debug, Clone)]
pub struct Request {
    pub org: OrgId,
    pub messages: Vec<Message>,
    pub tools: Vec<Tool>,
    pub modelo: Option<String>,
    pub temperatura: Option<f32>,
    pub max_tokens: Option<u32>,
    pub cancellation: Cancellation,
}

impl Request {
    pub fn new(org: OrgId, messages: Vec<Message>) -> Self {
        Self {
            org,
            messages,
            tools: Vec::new(),
            modelo: None,
            temperatura: None,
            max_tokens: None,
            cancellation: Cancellation::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Usage {
    pub input: u64,
    pub saida: u64,
}

impl Usage {
    pub fn total(&self) -> u64 {
        self.input + self.saida
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelPrice {
    pub input_per_million_micros: u64,
    pub output_per_million_micros: u64,
}

pub fn cost_micros(usage: Usage, price: ModelPrice) -> u64 {
    let input = usage.input.saturating_mul(price.input_per_million_micros) / 1_000_000;
    let saida = usage.saida.saturating_mul(price.output_per_million_micros) / 1_000_000;
    input.saturating_add(saida)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FinishReason {
    Completed,
    ToolCalls,
    TokenLimit,
    Other(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum LlmEvent {
    Texto(String),
    Tool(ToolCall),
    Usage(Usage),
    Fim(FinishReason),
}

#[derive(Debug, Clone)]
pub struct Cancellation(Arc<watch::Sender<bool>>);

impl Default for Cancellation {
    fn default() -> Self {
        Self::new()
    }
}

impl Cancellation {
    pub fn new() -> Self {
        Self(Arc::new(watch::channel(false).0))
    }

    pub fn cancel(&self) {
        self.0.send_replace(true);
    }

    pub fn cancelled(&self) -> bool {
        *self.0.borrow()
    }

    pub async fn wait(&self) {
        let mut receiver = self.0.subscribe();
        let _ = receiver.wait_for(|cancelled| *cancelled).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn part_debug_does_not_show_content() {
        let texto = Part::Texto("dado do cliente 123".to_owned());
        let image = Part::Image {
            mime: "image/png".to_owned(),
            bytes: vec![1, 2, 3],
        };
        let saida = format!("{texto:?} {image:?}");
        assert!(!saida.contains("dado do cliente"));
        assert_eq!(saida, "Texto(19 caracteres) Imagem(image/png, 3 bytes)");
    }

    #[test]
    fn cost_uses_price_per_million() {
        let price = ModelPrice {
            input_per_million_micros: 300_000,
            output_per_million_micros: 2_500_000,
        };
        let usage = Usage {
            input: 1_000_000,
            saida: 2_000_000,
        };
        assert_eq!(cost_micros(usage, price), 5_300_000);
        assert_eq!(usage.total(), 3_000_000);
    }

    #[test]
    fn cancellation_is_seen_by_all_clones() {
        let cancellation = Cancellation::new();
        let other = cancellation.clone();
        assert!(!other.cancelled());
        cancellation.cancel();
        assert!(other.cancelled());
    }
}
