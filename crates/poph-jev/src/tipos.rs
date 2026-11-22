//! Tipos do contrato do Jev (PRD §8.5): gate, pergunta, resposta e erro.
//!
//! Contrato: `POST {base}/v1/systemone` com `{state, model, questions:{id:{type,instructions,
//! criteria}}}`, resposta `answers[id]` com `choice|score|noul`, `probabilities`, `confidence` e
//! `usage`. Várias perguntas viajam num **pedido só** — é o que mantém o custo (~255–315 ms por
//! chamada no POP Atende) e a latência baixos.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;

/// Portão do Jev (PRD §10). G1–G5 existem; G6–G8 entram nas tarefas deles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum IdGate {
    /// G1: transferir para humano.
    G1,
    /// G2: esperar mais ou processar agora.
    G2,
    /// G3: precisa de base, produto ou nada.
    G3,
    /// G4: qual departamento recebe a transferência.
    G4,
    /// G5: qual produto o cliente quer.
    G5,
    /// G6: revisão da resposta antes do envio.
    G6,
    /// G7: injeção de prompt ou abuso.
    G7,
    /// G8: conversa encerrada.
    G8,
}

impl IdGate {
    /// Todos os portões, na ordem do PRD.
    pub const TODOS: [IdGate; 8] = [
        IdGate::G1,
        IdGate::G2,
        IdGate::G3,
        IdGate::G4,
        IdGate::G5,
        IdGate::G6,
        IdGate::G7,
        IdGate::G8,
    ];

    /// Texto usado na configuração, no JSON do turno e na CLI (`g1` … `g8`).
    pub fn como_texto(self) -> &'static str {
        match self {
            IdGate::G1 => "g1",
            IdGate::G2 => "g2",
            IdGate::G3 => "g3",
            IdGate::G4 => "g4",
            IdGate::G5 => "g5",
            IdGate::G6 => "g6",
            IdGate::G7 => "g7",
            IdGate::G8 => "g8",
        }
    }

    /// Nome curto para relatório e documentação.
    pub fn titulo(self) -> &'static str {
        match self {
            IdGate::G1 => "transferir para humano",
            IdGate::G2 => "esperar mais ou processar agora",
            IdGate::G3 => "precisa de base, produto ou nada",
            IdGate::G4 => "departamento da transferência",
            IdGate::G5 => "qual produto o cliente quer",
            IdGate::G6 => "revisão da resposta antes do envio",
            IdGate::G7 => "injeção de prompt ou abuso",
            IdGate::G8 => "conversa encerrada",
        }
    }

    /// Lê o texto do gate (`g1`, `G1`, `1`).
    pub fn de_texto(texto: &str) -> Option<Self> {
        let limpo = texto.trim().to_ascii_lowercase();
        let limpo = limpo.strip_prefix('g').unwrap_or(&limpo);
        let numero: u8 = limpo.parse().ok()?;
        let alvo = format!("g{numero}");
        Self::TODOS
            .into_iter()
            .find(|gate| gate.como_texto() == alvo.as_str())
    }

    /// Id da pergunta enviada ao Jev. Estável: o cache e a distribuição das respostas dependem dele.
    pub fn pergunta_id(self) -> &'static str {
        match self {
            IdGate::G1 => "g1_transferir",
            IdGate::G2 => "g2_esperar",
            IdGate::G3 => "g3_precisa",
            IdGate::G4 => "g4_departamento",
            IdGate::G5 => "g5_produto",
            IdGate::G6 => "g6_revisao",
            IdGate::G7 => "g7_injecao",
            IdGate::G8 => "g8_encerrada",
        }
    }
}

impl Serialize for IdGate {
    fn serialize<S: serde::Serializer>(&self, serializador: S) -> Result<S::Ok, S::Error> {
        serializador.serialize_str(self.como_texto())
    }
}

impl std::fmt::Display for IdGate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.como_texto())
    }
}

/// Tipo da pergunta enviada. Só `choice` é usado por G1–G4; `score`/`noul` já vêm na leitura da
/// resposta, porque a API devolve os três formatos.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TipoPergunta {
    /// Escolha entre opções conhecidas.
    Choice,
}

/// Uma pergunta do lote: o tipo, a instrução e o critério de cada opção.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pergunta {
    /// Tipo da primitiva.
    #[serde(rename = "type")]
    pub tipo: TipoPergunta,
    /// O que o Jev deve julgar.
    pub instructions: String,
    /// Critério de cada opção, por nome da opção.
    pub criteria: BTreeMap<String, String>,
}

impl Pergunta {
    /// Pergunta de escolha.
    pub fn escolha(instructions: impl Into<String>, criteria: BTreeMap<String, String>) -> Self {
        Self {
            tipo: TipoPergunta::Choice,
            instructions: instructions.into(),
            criteria,
        }
    }
}

/// Resposta do Jev para uma pergunta, já normalizada (o formato cru varia por primitiva).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Resposta {
    /// Opção escolhida (`choice`). `None` em `score`/`noul` ou quando o item não trouxe escolha.
    pub escolha: Option<String>,
    /// Probabilidade de cada opção, como o Jev devolveu.
    pub probabilidades: BTreeMap<String, f64>,
    /// Confiança declarada pelo Jev (0 quando ausente).
    pub confianca: f64,
    /// Valor de `score`, quando a primitiva for de nota.
    pub score: Option<f64>,
    /// Valor de `noul`, quando a primitiva for de probabilidade de "nenhuma das anteriores".
    pub noul: Option<f64>,
}

impl Resposta {
    /// Margem da escolha: probabilidade dela menos a da segunda colocada.
    ///
    /// Sem `probabilities` na resposta, a margem não pode ser medida e equivale à confiança: é o
    /// caso em que o Jev respondeu só com `choice` + `confidence`, e inventar margem zero barraria
    /// toda decisão.
    pub fn margem(&self) -> f64 {
        let Some(escolha) = &self.escolha else {
            return 0.0;
        };
        let Some(da_escolha) = self.probabilidades.get(escolha) else {
            return self.confianca;
        };
        let segunda = self
            .probabilidades
            .iter()
            .filter(|(opcao, _)| *opcao != escolha)
            .map(|(_, valor)| *valor)
            .fold(0.0_f64, f64::max);
        (da_escolha - segunda).max(0.0)
    }

    /// Lê um item de `answers` em qualquer um dos três formatos aceitos.
    pub fn de_json(item: &Value) -> Option<Self> {
        let objeto = item.as_object()?;
        let escolha = objeto
            .get("choice")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let score = objeto.get("score").and_then(Value::as_f64);
        let noul = objeto.get("noul").and_then(Value::as_f64);
        if escolha.is_none() && score.is_none() && noul.is_none() {
            return None;
        }
        let probabilidades = objeto
            .get("probabilities")
            .and_then(Value::as_object)
            .map(|mapa| {
                mapa.iter()
                    .filter_map(|(opcao, valor)| valor.as_f64().map(|valor| (opcao.clone(), valor)))
                    .collect()
            })
            .unwrap_or_default();
        Some(Self {
            escolha,
            probabilidades,
            confianca: objeto
                .get("confidence")
                .and_then(Value::as_f64)
                .unwrap_or(0.0),
            score,
            noul,
        })
    }
}

/// Consumo de tokens que o Jev informa no `usage` do lote.
///
/// Cada número é opcional porque o provedor pode devolver só uma parte — ou nada. **Nada aqui é
/// estimado**: quando a API não manda, o campo fica `None` e o relatório diz "não informado".
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UsageTokens {
    /// Tokens de entrada (`input_tokens` ou `prompt_tokens`).
    pub entrada: Option<u64>,
    /// Tokens de saída (`output_tokens` ou `completion_tokens`).
    pub saida: Option<u64>,
    /// Total informado (`total_tokens`); sem ele, entrada + saída quando as duas vierem.
    pub total: Option<u64>,
}

impl UsageTokens {
    /// Lê o objeto `usage` de uma resposta do Jev.
    pub fn de_json(uso: Option<&Value>) -> Self {
        let Some(mapa) = uso.and_then(Value::as_object) else {
            return Self::default();
        };
        let ler = |nomes: &[&str]| -> Option<u64> {
            nomes
                .iter()
                .find_map(|nome| mapa.get(*nome).and_then(Value::as_u64))
        };
        let entrada = ler(&["input_tokens", "prompt_tokens"]);
        let saida = ler(&["output_tokens", "completion_tokens"]);
        let total = ler(&["total_tokens"]).or_else(|| match (entrada, saida) {
            (Some(entrada), Some(saida)) => Some(entrada.saturating_add(saida)),
            _ => None,
        });
        Self {
            entrada,
            saida,
            total,
        }
    }

    /// `true` quando o provedor informou pelo menos um número.
    pub fn informado(&self) -> bool {
        self.entrada.is_some() || self.saida.is_some() || self.total.is_some()
    }

    /// Soma dois consumos (o relatório de medição soma o de cada chamada).
    pub fn somar(self, outro: Self) -> Self {
        fn soma(a: Option<u64>, b: Option<u64>) -> Option<u64> {
            match (a, b) {
                (None, None) => None,
                (a, b) => Some(a.unwrap_or(0).saturating_add(b.unwrap_or(0))),
            }
        }
        Self {
            entrada: soma(self.entrada, outro.entrada),
            saida: soma(self.saida, outro.saida),
            total: soma(self.total, outro.total),
        }
    }

    /// Texto curto para o relatório; "não informado" quando o provedor não mandou nada.
    pub fn como_texto(&self) -> String {
        if !self.informado() {
            return "não informado".to_owned();
        }
        fn numero(valor: Option<u64>) -> String {
            valor.map_or_else(|| "não informado".to_owned(), |valor| valor.to_string())
        }
        format!(
            "entrada {} · saída {} · total {}",
            numero(self.entrada),
            numero(self.saida),
            numero(self.total)
        )
    }

    /// Forma JSON usada no relatório e no `turns.jev_json`.
    pub fn para_json(&self) -> Value {
        json!({
            "entrada": self.entrada,
            "saida": self.saida,
            "total": self.total,
            "informado": self.informado(),
        })
    }
}

impl Serialize for UsageTokens {
    fn serialize<S: serde::Serializer>(&self, serializador: S) -> Result<S::Ok, S::Error> {
        self.para_json().serialize(serializador)
    }
}

/// Resposta de um lote: as respostas por id de pergunta e o consumo do pedido.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LoteResposta {
    /// Respostas por id de pergunta.
    pub respostas: BTreeMap<String, Resposta>,
    /// Tokens consumidos no pedido (`usage`), somados quando o Jev separa entrada e saída.
    pub tokens: u64,
    /// Consumo detalhado (entrada, saída, total), quando o provedor informa.
    pub usage: UsageTokens,
}

impl LoteResposta {
    /// Lê o corpo de `POST /v1/systemone`.
    pub fn de_json(corpo: &Value) -> Result<Self, ErroJev> {
        let objeto = corpo
            .as_object()
            .ok_or_else(|| ErroJev::Resposta("o corpo não é um objeto JSON".to_owned()))?;
        let respostas = match objeto.get("answers") {
            None => BTreeMap::new(),
            Some(Value::Object(mapa)) => mapa
                .iter()
                .filter_map(|(id, item)| {
                    Resposta::de_json(item).map(|resposta| (id.clone(), resposta))
                })
                .collect(),
            Some(_) => {
                return Err(ErroJev::Resposta(
                    "`answers` não é um objeto JSON".to_owned(),
                ));
            }
        };
        let usage = UsageTokens::de_json(objeto.get("usage"));
        // `tokens` preserva o comportamento anterior: o total informado, ou a soma de tudo o que
        // veio em `usage` (o provedor às vezes manda só um dos lados).
        let tokens = usage.total.unwrap_or_else(|| {
            objeto
                .get("usage")
                .and_then(Value::as_object)
                .map(|uso| uso.values().filter_map(Value::as_u64).sum::<u64>())
                .unwrap_or(0)
        });
        Ok(Self {
            respostas,
            tokens,
            usage,
        })
    }
}

/// Tudo o que pode dar errado numa chamada ao Jev.
///
/// Nenhuma variante carrega a chave nem o corpo da resposta do provedor: corpo de erro pode ecoar
/// o `Authorization` e não vale o risco de aparecer em log.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum ErroJev {
    /// Configuração inválida (URL, modelo, chave ausente).
    #[error("configuração do Jev: {0}")]
    Configuracao(String),

    /// Prazo esgotado (por tentativa).
    #[error("o Jev não respondeu em {ms} ms")]
    TempoEsgotado {
        /// Prazo que estourou.
        ms: u64,
    },

    /// Falha de rede/conexão antes de a resposta começar.
    #[error("falha de rede ao chamar o Jev: {0}")]
    Rede(String),

    /// Falha de TLS: certificado não confiável, nome errado ou handshake recusado. Não é retentada.
    #[error("falha de TLS ao chamar o Jev: {0}")]
    Tls(String),

    /// O Jev respondeu com status de erro.
    #[error("o Jev respondeu {codigo} ({motivo})")]
    Status {
        /// Código HTTP.
        codigo: u16,
        /// Motivo curto (`Too Many Requests`, `Internal Server Error`…), sem corpo.
        motivo: String,
    },

    /// Corpo fora do contrato.
    #[error("resposta do Jev fora do contrato: {0}")]
    Resposta(String),

    /// Sem chave configurada para chamar o Jev.
    #[error("chave do Jev ausente: defina TYPESAFE_API_KEY no ambiente")]
    SemChave,
}

impl ErroJev {
    /// `true` quando vale repetir a chamada (429, 5xx, queda de conexão ou prazo).
    pub fn eh_transitorio(&self) -> bool {
        match self {
            ErroJev::Status { codigo, .. } => *codigo == 429 || *codigo >= 500,
            ErroJev::Rede(_) | ErroJev::TempoEsgotado { .. } => true,
            ErroJev::Configuracao(_) | ErroJev::Resposta(_) | ErroJev::SemChave => false,
            // Repetir não conserta certificado nem nome do servidor.
            ErroJev::Tls(_) => false,
        }
    }
}
