use std::fmt;
use std::sync::Arc;

use poph_types::OrgId;
use serde_json::Value;
use tokio::sync::watch;

#[derive(Clone, PartialEq, Eq)]
pub enum Part {
    Texto(String),
    Imagem { mime: String, bytes: Vec<u8> },
}

impl fmt::Debug for Part {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Part::Texto(texto) => write!(f, "Texto({} caracteres)", texto.chars().count()),
            Part::Imagem { mime, bytes } => write!(f, "Imagem({mime}, {} bytes)", bytes.len()),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Tool {
    pub nome: String,
    pub descricao: String,
    pub parametros: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolCall {
    pub id: String,
    pub nome: String,
    pub arguments: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Message {
    Sistema(String),
    Usuario(Vec<Part>),
    Assistente {
        texto: String,
        chamadas: Vec<ToolCall>,
    },
    Resultado {
        id: String,
        conteudo: String,
    },
}

#[derive(Debug, Clone)]
pub struct Request {
    pub org: OrgId,
    pub mensagens: Vec<Message>,
    pub tools: Vec<Tool>,
    pub modelo: Option<String>,
    pub temperatura: Option<f32>,
    pub max_tokens: Option<u32>,
    pub cancelamento: Cancellation,
}

impl Request {
    pub fn new(org: OrgId, mensagens: Vec<Message>) -> Self {
        Self {
            org,
            mensagens,
            tools: Vec::new(),
            modelo: None,
            temperatura: None,
            max_tokens: None,
            cancelamento: Cancellation::novo(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Usage {
    pub entrada: u64,
    pub saida: u64,
}

impl Usage {
    pub fn total(&self) -> u64 {
        self.entrada + self.saida
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelPrice {
    pub input_per_million_micros: u64,
    pub output_per_million_micros: u64,
}

pub fn cost_micros(uso: Usage, price: ModelPrice) -> u64 {
    let entrada = uso.entrada.saturating_mul(price.input_per_million_micros) / 1_000_000;
    let saida = uso.saida.saturating_mul(price.output_per_million_micros) / 1_000_000;
    entrada.saturating_add(saida)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FinishReason {
    Concluido,
    ChamadasFerramenta,
    LimiteDeTokens,
    Outro(String),
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

impl Cancellation {
    pub fn novo() -> Self {
        Self(Arc::new(watch::channel(false).0))
    }

    pub fn cancel(&self) {
        self.0.send_replace(true);
    }

    pub fn cancelled(&self) -> bool {
        *self.0.borrow()
    }

    pub async fn wait(&self) {
        let mut recebedor = self.0.subscribe();
        let _ = recebedor.wait_for(|cancelado| *cancelado).await;
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn part_debug_does_not_show_content() {
        let texto = Part::Texto("dado do cliente 123".to_owned());
        let imagem = Part::Imagem {
            mime: "image/png".to_owned(),
            bytes: vec![1, 2, 3],
        };
        let saida = format!("{texto:?} {imagem:?}");
        assert!(!saida.contains("dado do cliente"));
        assert_eq!(saida, "Texto(19 caracteres) Imagem(image/png, 3 bytes)");
    }

    #[test]
    fn custo_usa_o_preco_por_milhao() {
        let preco = ModelPrice {
            input_per_million_micros: 300_000,
            output_per_million_micros: 2_500_000,
        };
        let uso = Usage {
            entrada: 1_000_000,
            saida: 2_000_000,
        };
        assert_eq!(cost_micros(uso, preco), 5_300_000);
        assert_eq!(uso.total(), 3_000_000);
    }

    #[test]
    fn cancellation_is_seen_by_all_clones() {
        let cancelamento = Cancellation::novo();
        let outro = cancelamento.clone();
        assert!(!outro.cancelled());
        cancelamento.cancel();
        assert!(outro.cancelled());
    }
}
