//! `Gate`: o molde comum dos portões do Jev (PRD §10).
//!
//! Cada gate declara quatro coisas e nada mais:
//! 1. **a pergunta** que manda ao Jev (instrução + critério de cada opção), montada a partir do
//!    estado do turno;
//! 2. **quando se aplica** (agente sem transferência habilitada não pergunta sobre transferir, e por
//!    aí vai);
//! 3. **os limiares** de confiança e margem (padrão do POP Atende: 0,6–0,75 de confiança e 0,1 de
//!    margem);
//! 4. **a interpretação**: resposta confiável vira decisão; resposta ausente, ambígua ou abaixo do
//!    limiar vira `Fallback` com o motivo — e o turno segue como se o Jev não existisse.
//!
//! O gate **não executa nada**. Ele decide; quem age é o turno (transferir, pular a busca, escolher
//! o departamento), e só nas tarefas seguintes (HR-29 a HR-36).

use crate::estado::EstadoJev;
use crate::tipos::{IdGate, Pergunta, Resposta};

/// Confiança mínima para aceitar uma decisão sem o ajuste do gate.
pub const CONFIANCA_PADRAO: f64 = 0.7;
/// Margem mínima sobre a segunda opção.
pub const MARGEM_PADRAO: f64 = 0.1;

/// Limiares de aceitação de uma decisão.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Limiares {
    /// Confiança mínima (0–1).
    pub confianca: f64,
    /// Margem mínima sobre a segunda opção (0 = sem exigência de margem).
    pub margem: f64,
}

impl Default for Limiares {
    fn default() -> Self {
        Self {
            confianca: CONFIANCA_PADRAO,
            margem: MARGEM_PADRAO,
        }
    }
}

impl Limiares {
    /// Limiares com os valores dados.
    pub fn novos(confianca: f64, margem: f64) -> Self {
        Self {
            confianca: confianca.clamp(0.0, 1.0),
            margem: margem.clamp(0.0, 1.0),
        }
    }
}

/// De onde veio a decisão.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrigemDecisao {
    /// Resposta do Jev nesta chamada.
    Jev,
    /// Resposta do Jev guardada no cache (mesmo estado).
    Cache,
    /// Não houve decisão confiável: vale o comportamento sem o Jev.
    Fallback,
}

impl OrigemDecisao {
    /// Texto usado no JSON do turno e no relatório.
    pub fn como_texto(self) -> &'static str {
        match self {
            OrigemDecisao::Jev => "jev",
            OrigemDecisao::Cache => "cache",
            OrigemDecisao::Fallback => "fallback",
        }
    }
}

/// O que um gate decidiu.
#[derive(Debug, Clone, PartialEq)]
pub struct Decisao {
    /// Gate que decidiu.
    pub gate: IdGate,
    /// Opção escolhida. Em `Fallback`, é o comportamento sem o Jev (pode ser `None`).
    pub escolha: Option<String>,
    /// De onde veio.
    pub origem: OrigemDecisao,
    /// Confiança declarada (0 no fallback).
    pub confianca: f64,
    /// Margem sobre a segunda opção (0 no fallback).
    pub margem: f64,
    /// Por que caiu no fallback.
    pub motivo: Option<String>,
    /// Tempo da chamada que produziu a decisão (0 no cache: não houve chamada).
    pub ms: u64,
    /// Tokens do lote que produziu a decisão.
    pub tokens: u64,
}

impl Decisao {
    /// Decisão confiável (veio do Jev ou do cache).
    pub fn confiavel(&self) -> bool {
        self.origem != OrigemDecisao::Fallback
    }

    /// Decisão sem o Jev: vale o comportamento padrão.
    pub fn fallback(gate: IdGate, escolha: Option<&str>, motivo: impl Into<String>) -> Self {
        Self {
            gate,
            escolha: escolha.map(str::to_owned),
            origem: OrigemDecisao::Fallback,
            confianca: 0.0,
            margem: 0.0,
            motivo: Some(motivo.into()),
            ms: 0,
            tokens: 0,
        }
    }

    /// A decisão como vai no `turns.jev_json`.
    pub fn para_json(&self) -> serde_json::Value {
        serde_json::json!({
            "gate": self.gate.como_texto(),
            "escolha": self.escolha,
            "origem": self.origem.como_texto(),
            "confianca": self.confianca,
            "margem": self.margem,
            "motivo": self.motivo,
            "ms": self.ms,
            "tokens": self.tokens,
        })
    }
}

/// Um portão do Jev.
pub trait Gate: Send + Sync + std::fmt::Debug {
    /// Qual gate é.
    fn id(&self) -> IdGate;

    /// Se este estado justifica a pergunta (agente com transferência desligada não pergunta, por ex.).
    fn aplicavel(&self, estado: &EstadoJev) -> bool;

    /// A pergunta que vai no lote.
    fn pergunta(&self, estado: &EstadoJev) -> Pergunta;

    /// Aviso para o operador quando o gate **deixa** de se aplicar por um motivo que ele precisa
    /// saber, e não por este turno não ter o que julgar.
    ///
    /// Hoje só o G9 usa (mais fluxos liberados do que opções cabem na pergunta); o motor registra o
    /// aviso quando o gate está ligado e não se aplica. `None` = nada a avisar.
    fn aviso(&self, _estado: &EstadoJev) -> Option<String> {
        None
    }

    /// Limiares deste gate.
    fn limiares(&self) -> Limiares {
        Limiares::default()
    }

    /// Interpreta a resposta (ou a falta dela) no estado dado.
    ///
    /// Deve devolver `Decisao` com `origem: Fallback` quando não houver resposta ou quando a
    /// resposta não passar nos limiares — nunca `panic`, nunca erro.
    fn interpretar(
        &self,
        estado: &EstadoJev,
        resposta: Option<&Resposta>,
        limiares: Limiares,
    ) -> Decisao;
}

/// Aplica a regra comum: escolha conhecida, acima dos limiares de confiança e margem.
///
/// Devolve `None` quando a resposta não serve, com o motivo preenchido em `motivo`.
pub(crate) fn escolha_confiavel<'a>(
    resposta: Option<&'a Resposta>,
    validas: &[String],
    limiares: Limiares,
    motivo: &mut Option<String>,
) -> Option<&'a str> {
    let Some(resposta) = resposta else {
        *motivo = Some("sem resposta do Jev".to_owned());
        return None;
    };
    let Some(escolha) = resposta.escolha.as_deref() else {
        *motivo = Some("resposta sem escolha (score/noul ou vazia)".to_owned());
        return None;
    };
    if !validas.iter().any(|valida| valida == escolha) {
        *motivo = Some(format!("escolha fora das opções: {escolha}"));
        return None;
    }
    if resposta.confianca < limiares.confianca {
        *motivo = Some(format!(
            "confiança {:.2} abaixo do limiar {:.2}",
            resposta.confianca, limiares.confianca
        ));
        return None;
    }
    if resposta.margem() < limiares.margem {
        *motivo = Some(format!(
            "margem {:.2} abaixo do limiar {:.2}",
            resposta.margem(),
            limiares.margem
        ));
        return None;
    }
    Some(escolha)
}
