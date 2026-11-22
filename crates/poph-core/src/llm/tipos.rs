//! Tipos do contrato com o provedor de LLM: mensagens, ferramentas, eventos do stream e uso.
//!
//! Nada aqui sabe de HTTP. O formato do gateway (OpenAI-compatível) fica em `antigravity.rs`.

use std::fmt;
use std::sync::Arc;

use poph_types::OrgId;
use serde_json::Value;
use tokio::sync::watch;

/// Pedaço de uma mensagem do usuário: texto ou imagem.
#[derive(Clone, PartialEq, Eq)]
pub enum Parte {
    /// Texto (instrução do usuário, texto extraído de arquivo, trecho de base).
    Texto(String),
    /// Imagem em bytes. `mime` deve ser `image/jpeg`, `image/png`, `image/gif` ou `image/webp`.
    Imagem { mime: String, bytes: Vec<u8> },
}

// Debug nunca imprime o conteúdo: texto de cliente e bytes de imagem não vão para log por engano.
impl fmt::Debug for Parte {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Parte::Texto(texto) => write!(f, "Texto({} caracteres)", texto.chars().count()),
            Parte::Imagem { mime, bytes } => write!(f, "Imagem({mime}, {} bytes)", bytes.len()),
        }
    }
}

/// Ferramenta que o modelo pode chamar. `parametros` é um JSON Schema (objeto).
#[derive(Debug, Clone, PartialEq)]
pub struct Ferramenta {
    /// Nome único da ferramenta, como o modelo vai escrevê-lo na chamada.
    pub nome: String,
    /// Texto que explica ao modelo quando usar a ferramenta.
    pub descricao: String,
    /// JSON Schema dos argumentos.
    pub parametros: Value,
}

/// Chamada de ferramenta pedida pelo modelo, já com os argumentos lidos como JSON.
#[derive(Debug, Clone, PartialEq)]
pub struct ChamadaFerramenta {
    /// Id dado pelo provedor; a resposta da ferramenta precisa citar o mesmo id.
    pub id: String,
    /// Nome da ferramenta chamada.
    pub nome: String,
    /// Argumentos (objeto JSON).
    pub argumentos: Value,
}

/// Mensagem enviada ao modelo. A ordem da lista é a ordem da conversa.
#[derive(Debug, Clone, PartialEq)]
pub enum Mensagem {
    /// Instruções do agente (prefixo estável do prompt).
    Sistema(String),
    /// Mensagem do cliente, com texto e imagens.
    Usuario(Vec<Parte>),
    /// Resposta anterior do modelo, com as chamadas de ferramenta que ela pediu.
    Assistente {
        texto: String,
        chamadas: Vec<ChamadaFerramenta>,
    },
    /// Resultado de uma chamada de ferramenta (cita o id da chamada).
    Resultado { id: String, conteudo: String },
}

/// Pedido de uma resposta ao modelo.
#[derive(Debug, Clone)]
pub struct Pedido {
    /// Org dona do turno. Define o limite de simultaneidade e o isolamento entre orgs.
    pub org: OrgId,
    /// Conversa inteira enviada ao modelo. A última mensagem tem de ser do usuário ou um resultado.
    pub mensagens: Vec<Mensagem>,
    /// Ferramentas que o modelo pode chamar neste pedido (já filtradas pelo agente).
    pub ferramentas: Vec<Ferramenta>,
    /// Modelo deste pedido. `None` usa o modelo padrão da configuração.
    pub modelo: Option<String>,
    /// Temperatura deste pedido (configuração por agente). `None` usa a da configuração.
    pub temperatura: Option<f32>,
    /// Teto de tokens de saída deste pedido. `None` usa o da configuração.
    pub max_tokens: Option<u32>,
    /// Corta o pedido no meio: quem cancela recebe `ErroLlm::Cancelada` e a conexão é fechada.
    pub cancelamento: Cancelamento,
}

impl Pedido {
    /// Pedido sem ferramentas, com o modelo padrão e um cancelamento novo.
    pub fn new(org: OrgId, mensagens: Vec<Mensagem>) -> Self {
        Self {
            org,
            mensagens,
            ferramentas: Vec::new(),
            modelo: None,
            temperatura: None,
            max_tokens: None,
            cancelamento: Cancelamento::novo(),
        }
    }
}

/// Tokens gastos numa resposta.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Uso {
    /// Tokens de entrada (prompt, inclusive histórico e imagens).
    pub entrada: u64,
    /// Tokens de saída (resposta e chamadas de ferramenta).
    pub saida: u64,
}

impl Uso {
    /// Entrada somada à saída.
    pub fn total(&self) -> u64 {
        self.entrada + self.saida
    }
}

/// Preço por milhão de tokens, em micro-unidades monetárias (1 unidade = 1.000.000 micros).
///
/// A config ainda não traz tabela de preços: quem tiver o preço do modelo monta isto.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PrecoModelo {
    /// Custo de 1 milhão de tokens de entrada.
    pub entrada_por_milhao_micros: u64,
    /// Custo de 1 milhão de tokens de saída.
    pub saida_por_milhao_micros: u64,
}

/// Custo de um uso, em micros. Arredonda para baixo, por milhão de tokens.
pub fn custo_micros(uso: Uso, preco: PrecoModelo) -> u64 {
    let entrada = uso.entrada.saturating_mul(preco.entrada_por_milhao_micros) / 1_000_000;
    let saida = uso.saida.saturating_mul(preco.saida_por_milhao_micros) / 1_000_000;
    entrada.saturating_add(saida)
}

/// Motivo informado pelo provedor para encerrar a resposta.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MotivoFim {
    /// Resposta completa.
    Concluido,
    /// O modelo parou para pedir ferramentas.
    ChamadasFerramenta,
    /// Parou por limite de tokens de saída.
    LimiteDeTokens,
    /// Outro motivo, como vem do provedor.
    Outro(String),
}

/// Evento recebido durante o stream de uma resposta.
#[derive(Debug, Clone, PartialEq)]
pub enum EventoLlm {
    /// Pedaço de texto da resposta, na ordem em que chegou.
    Texto(String),
    /// Chamada de ferramenta completa (só sai depois que os argumentos chegaram inteiros).
    Ferramenta(ChamadaFerramenta),
    /// Tokens usados na resposta inteira. Sai uma vez, antes do `Fim`.
    Uso(Uso),
    /// Fim da resposta. Sempre o último evento de um stream bem-sucedido.
    Fim(MotivoFim),
}

/// Cancelamento compartilhado entre quem pede e quem lê o stream.
///
/// Clonar é barato: todos os clones falam do mesmo cancelamento.
#[derive(Debug, Clone)]
pub struct Cancelamento(Arc<watch::Sender<bool>>);

impl Cancelamento {
    /// Cancelamento que ainda não foi pedido.
    pub fn novo() -> Self {
        Self(Arc::new(watch::channel(false).0))
    }

    /// Pede o cancelamento. Repetir não faz mal.
    pub fn cancelar(&self) {
        self.0.send_replace(true);
    }

    /// Se o cancelamento já foi pedido.
    pub fn cancelado(&self) -> bool {
        *self.0.borrow()
    }

    /// Espera até alguém pedir o cancelamento (retorna na hora se já foi pedido).
    pub async fn aguardar(&self) {
        let mut recebedor = self.0.subscribe();
        // `wait_for` só falha se o emissor sumir, e ele vive enquanto este clone existir.
        let _ = recebedor.wait_for(|cancelado| *cancelado).await;
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn debug_de_parte_nao_mostra_conteudo() {
        let texto = Parte::Texto("dado do cliente 123".to_owned());
        let imagem = Parte::Imagem {
            mime: "image/png".to_owned(),
            bytes: vec![1, 2, 3],
        };
        let saida = format!("{texto:?} {imagem:?}");
        assert!(!saida.contains("dado do cliente"));
        assert_eq!(saida, "Texto(19 caracteres) Imagem(image/png, 3 bytes)");
    }

    #[test]
    fn custo_usa_o_preco_por_milhao() {
        let preco = PrecoModelo {
            entrada_por_milhao_micros: 300_000,
            saida_por_milhao_micros: 2_500_000,
        };
        let uso = Uso {
            entrada: 1_000_000,
            saida: 2_000_000,
        };
        // 1M entrada × 0,30 + 2M saída × 2,50 = 5,30 unidades = 5.300.000 micros.
        assert_eq!(custo_micros(uso, preco), 5_300_000);
        assert_eq!(uso.total(), 3_000_000);
    }

    #[test]
    fn cancelamento_e_visto_por_todos_os_clones() {
        let cancelamento = Cancelamento::novo();
        let outro = cancelamento.clone();
        assert!(!outro.cancelado());
        cancelamento.cancelar();
        assert!(outro.cancelado());
    }
}
