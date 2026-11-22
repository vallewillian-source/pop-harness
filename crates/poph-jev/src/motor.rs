//! O motor do Jev: um lote só de perguntas, cache, limiares e fallback.
//!
//! Desenho (PRD §8.5 e §10):
//! - **Um pedido por turno**, com as perguntas de todos os gates ligados. N chamadas viram 1.
//! - **Cache por hash do estado**: pergunta repetida sobre o mesmo estado não custa chamada.
//! - **Nunca derruba o turno**: timeout, erro, resposta ambígua ou flag desligada caem no fallback
//!   do gate, que é o comportamento sem o Jev.
//! - **Entrega decisões, não efeitos**: quem age é o turno, nas tarefas HR-29 a HR-36.
//!
//! O motor também é o que o `poph jev eval` usa para medir um conjunto rotulado.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::future::BoxFuture;
use poph_types::GatesJev;
use serde_json::{Value, json};

use crate::cache::{CacheDecisoes, OpcoesCache};
use crate::cliente::{Pedido, TransporteJev};
use crate::estado::EstadoJev;
use crate::flags::AjusteGates;
use crate::gate::{Decisao, Gate, Limiares, OrigemDecisao};
use crate::gates;
use crate::handoff::ResultadoOrientacao;
use crate::tipos::{IdGate, Resposta, UsageTokens};

/// Prazo do lote inteiro, contando as retentativas do cliente (8 s + folga da retentativa).
pub const TEMPO_MAXIMO_PADRAO: Duration = Duration::from_secs(12);

/// Ajustes do motor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpcoesJev {
    /// Cache de decisão.
    pub cache: OpcoesCache,
    /// Prazo do lote inteiro (a chamada do cliente tem prazo próprio por tentativa).
    pub tempo_maximo: Duration,
}

impl Default for OpcoesJev {
    fn default() -> Self {
        Self {
            cache: OpcoesCache::default(),
            tempo_maximo: TEMPO_MAXIMO_PADRAO,
        }
    }
}

/// O que aconteceu na chamada do lote (vai para o registro do turno).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RelatoChamada {
    /// Quantas perguntas foram enviadas nesta chamada.
    pub perguntas: usize,
    /// Quantas decisões vieram do cache (não custaram chamada).
    pub decididas_por_cache: usize,
    /// Duração do lote, em ms.
    pub ms: u64,
    /// Tokens consumidos no lote.
    pub tokens: u64,
    /// Consumo detalhado informado pelo Jev (entrada, saída, total); vazio quando ele não informa.
    pub usage: UsageTokens,
    /// Motivo da falha do lote, quando houve.
    pub falha: Option<String>,
}

/// Resultado de uma avaliação: as decisões por gate e o relato da chamada.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ResultadoJev {
    /// Decisões, na ordem dos gates.
    pub decisoes: Vec<Decisao>,
    /// Resposta crua do Jev por gate — inclusive quando ela **não** passou no limiar.
    ///
    /// É o que permite medir calibração: a decisão já é o que vale para o turno (com o padrão do
    /// gate no fallback), e a escolha do modelo fica aqui, sem alterar comportamento nenhum.
    pub brutas: BTreeMap<IdGate, Resposta>,
    /// O que a chamada custou.
    pub chamada: RelatoChamada,
}

impl ResultadoJev {
    /// Decisão de um gate, se ele rodou neste turno.
    pub fn decisao(&self, gate: IdGate) -> Option<&Decisao> {
        self.decisoes.iter().find(|decisao| decisao.gate == gate)
    }

    /// Resposta crua do Jev para um gate (existe mesmo quando a decisão caiu no fallback).
    pub fn resposta(&self, gate: IdGate) -> Option<&Resposta> {
        self.brutas.get(&gate)
    }

    /// Escolha de um gate, quando houve.
    pub fn escolha(&self, gate: IdGate) -> Option<&str> {
        self.decisao(gate)?.escolha.as_deref()
    }

    /// Escolha confiável de um gate (não é fallback).
    pub fn escolha_confiavel(&self, gate: IdGate) -> Option<&str> {
        let decisao = self.decisao(gate)?;
        if decisao.confiavel() {
            decisao.escolha.as_deref()
        } else {
            None
        }
    }

    /// `true` quando o lote falhou (timeout, erro do provedor).
    pub fn falhou(&self) -> bool {
        self.chamada.falha.is_some()
    }

    /// JSON gravado em `turns.jev_json`.
    pub fn para_json(&self) -> Value {
        json!({
            "chamada": {
                "perguntas": self.chamada.perguntas,
                "decididas_por_cache": self.chamada.decididas_por_cache,
                "ms": self.chamada.ms,
                "tokens": self.chamada.tokens,
                "usage": self.chamada.usage.para_json(),
                "falha": self.chamada.falha,
            },
            "decisoes": self
                .decisoes
                .iter()
                .map(|decisao| {
                    // A escolha crua do Jev vai junto: `escolha` é o que vale para o turno
                    // (inclui o padrão do gate no fallback) e `escolha_do_jev` é o que o modelo
                    // respondeu, mesmo quando a confiança não passou no limiar.
                    let mut valor = decisao.para_json();
                    if let Some(bruta) = self.brutas.get(&decisao.gate) {
                        // Só a escolha: as probabilidades dobrariam o tamanho do registro do turno
                        // e não são necessárias para decidir nada.
                        valor["escolha_do_jev"] = json!(bruta.escolha);
                    }
                    valor
                })
                .collect::<Vec<_>>(),
        })
    }

    /// Texto JSON do registro do turno.
    pub fn para_json_texto(&self) -> String {
        self.para_json().to_string()
    }

    /// Resumo curto para o observador (`EventoTurno::Jev`).
    pub fn resumo(&self) -> Vec<String> {
        self.decisoes
            .iter()
            .map(|decisao| {
                format!(
                    "{}={} ({}, confiança {:.2})",
                    decisao.gate.como_texto(),
                    decisao.escolha.as_deref().unwrap_or("—"),
                    decisao.origem.como_texto(),
                    decisao.confianca
                )
            })
            .collect()
    }
}

/// Quem o turno consulta para decidir os portões.
///
/// Existe como trait para o turno não depender do cliente HTTP: o teste injeta um Jev falso e o
/// turno de produção recebe o [`MotorJev`].
pub trait AvaliadorGates: Send + Sync {
    /// Gates ligados para este agente (global ajustado pelo agente).
    fn gates_efetivos(&self, ajuste: Option<AjusteGates>) -> GatesJev;

    /// Avalia os gates ligados num lote só. Nunca falha: erro vira fallback.
    fn avaliar<'a>(
        &'a self,
        estado: EstadoJev,
        ajuste: Option<AjusteGates>,
    ) -> BoxFuture<'a, ResultadoJev>;

    /// Segunda pergunta do G1 (HR-29): classifica o risco imediato à saúde **depois** de a
    /// transferência ser decidida, para escolher o texto de orientação ao cliente.
    ///
    /// Nunca falha para o turno: erro, prazo e resposta fora do contrato voltam em
    /// [`ResultadoOrientacao::falha`], e quem chama segue com a mensagem automática só.
    fn orientar<'a>(
        &'a self,
        estado: EstadoJev,
        prazo: Duration,
    ) -> BoxFuture<'a, ResultadoOrientacao> {
        let _ = (estado, prazo);
        Box::pin(async {
            ResultadoOrientacao::nao_consultado("este avaliador não faz a pergunta de risco")
        })
    }
}

/// Motor do Jev: cliente do lote, gates, limiares e cache.
pub struct MotorJev {
    transporte: Arc<dyn TransporteJev>,
    gates: Vec<Box<dyn Gate>>,
    globais: GatesJev,
    modelo: String,
    limiares: BTreeMap<IdGate, Limiares>,
    cache: CacheDecisoes,
    tempo_maximo: Duration,
}

impl std::fmt::Debug for MotorJev {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MotorJev")
            .field("globais", &self.globais)
            .field("modelo", &self.modelo)
            .field(
                "gates",
                &self.gates.iter().map(|g| g.id()).collect::<Vec<_>>(),
            )
            .finish_non_exhaustive()
    }
}

impl MotorJev {
    /// Motor com os quatro gates da V1 e os ajustes padrão.
    pub fn novo(
        transporte: Arc<dyn TransporteJev>,
        globais: GatesJev,
        modelo: impl Into<String>,
    ) -> Self {
        Self {
            transporte,
            gates: gates::padrao(),
            globais,
            modelo: modelo.into(),
            limiares: BTreeMap::new(),
            cache: CacheDecisoes::novo(OpcoesCache::default()),
            tempo_maximo: TEMPO_MAXIMO_PADRAO,
        }
    }

    /// Troca os ajustes (cache e prazo do lote).
    pub fn com_opcoes(mut self, opcoes: OpcoesJev) -> Self {
        self.cache = CacheDecisoes::novo(opcoes.cache);
        self.tempo_maximo = opcoes.tempo_maximo;
        self
    }

    /// Fixa os limiares de um gate (calibração por medição; ver `poph jev eval`).
    pub fn com_limiares(mut self, gate: IdGate, limiares: Limiares) -> Self {
        self.limiares.insert(gate, limiares);
        self
    }

    /// Troca o catálogo de gates (teste; G5–G8 entram pelas tarefas deles).
    pub fn com_gates(mut self, gates: Vec<Box<dyn Gate>>) -> Self {
        self.gates = gates;
        self
    }

    /// Cache em uso (teste e observação).
    pub fn cache(&self) -> &CacheDecisoes {
        &self.cache
    }

    /// Limiares em uso para um gate.
    pub fn limiares_de(&self, gate: IdGate) -> Limiares {
        self.limiares
            .get(&gate)
            .copied()
            .or_else(|| {
                self.gates
                    .iter()
                    .find(|item| item.id() == gate)
                    .map(|item| item.limiares())
            })
            .unwrap_or_default()
    }

    /// `true` quando o gate está ligado nos gates efetivos.
    pub fn ligado(gates: GatesJev, gate: IdGate) -> bool {
        match gate {
            IdGate::G1 => gates.g1,
            IdGate::G2 => gates.g2,
            IdGate::G3 => gates.g3,
            IdGate::G4 => gates.g4,
            IdGate::G5 => gates.g5,
            IdGate::G6 => gates.g6,
            IdGate::G7 => gates.g7,
            IdGate::G8 => gates.g8,
        }
    }

    async fn avaliar_agora(&self, estado: EstadoJev, gates: GatesJev) -> ResultadoJev {
        let hash = estado.hash();
        let aplicaveis: Vec<&Box<dyn Gate>> = self
            .gates
            .iter()
            .filter(|gate| Self::ligado(gates, gate.id()) && gate.aplicavel(&estado))
            .collect();

        let mut do_cache: BTreeMap<IdGate, Resposta> = BTreeMap::new();
        let mut pendentes: Vec<IdGate> = Vec::new();
        for gate in &aplicaveis {
            match self.cache.obter(crate::cache::chave(gate.id(), hash)) {
                Some(resposta) => {
                    do_cache.insert(gate.id(), resposta);
                }
                None => pendentes.push(gate.id()),
            }
        }

        let mut do_jev: BTreeMap<IdGate, Resposta> = BTreeMap::new();
        let mut relato = RelatoChamada {
            decididas_por_cache: do_cache.len(),
            ..RelatoChamada::default()
        };

        if !pendentes.is_empty() {
            let perguntas: BTreeMap<String, crate::tipos::Pergunta> = aplicaveis
                .iter()
                .filter(|gate| pendentes.contains(&gate.id()))
                .map(|gate| (gate.id().pergunta_id().to_owned(), gate.pergunta(&estado)))
                .collect();
            relato.perguntas = perguntas.len();
            let pedido = Pedido {
                estado: estado.para_json(),
                modelo: self.modelo.clone(),
                perguntas,
            };
            let inicio = Instant::now();
            match tokio::time::timeout(self.tempo_maximo, self.transporte.julgar(pedido)).await {
                Ok(Ok(lote)) => {
                    relato.ms = u64::try_from(inicio.elapsed().as_millis()).unwrap_or(u64::MAX);
                    relato.tokens = lote.tokens;
                    relato.usage = lote.usage;
                    for gate in &aplicaveis {
                        if !pendentes.contains(&gate.id()) {
                            continue;
                        }
                        if let Some(resposta) = lote.respostas.get(gate.id().pergunta_id()) {
                            self.cache
                                .guardar(crate::cache::chave(gate.id(), hash), resposta.clone());
                            do_jev.insert(gate.id(), resposta.clone());
                        }
                    }
                }
                Ok(Err(erro)) => {
                    relato.ms = u64::try_from(inicio.elapsed().as_millis()).unwrap_or(u64::MAX);
                    relato.falha = Some(erro.to_string());
                    tracing::warn!(erro = %erro, "Jev falhou; todos os gates caem no fallback");
                }
                Err(_) => {
                    relato.ms = u64::try_from(inicio.elapsed().as_millis()).unwrap_or(u64::MAX);
                    let ms = u64::try_from(self.tempo_maximo.as_millis()).unwrap_or(u64::MAX);
                    relato.falha = Some(format!("o Jev não respondeu em {ms} ms"));
                    tracing::warn!(
                        ms,
                        "lote do Jev estourou o prazo; fallback em todos os gates"
                    );
                }
            }
        }

        let mut brutas: BTreeMap<IdGate, crate::tipos::Resposta> = BTreeMap::new();
        let mut decisoes = Vec::with_capacity(aplicaveis.len());
        for gate in &aplicaveis {
            let id = gate.id();
            let limiares = self.limiares_de(id);
            let (resposta, origem) = match (do_jev.get(&id), do_cache.get(&id)) {
                (Some(resposta), _) => (Some(resposta), OrigemDecisao::Jev),
                (None, Some(resposta)) => (Some(resposta), OrigemDecisao::Cache),
                (None, None) => (None, OrigemDecisao::Fallback),
            };
            let mut decisao = gate.interpretar(&estado, resposta, limiares);
            // Decisão aceita veio desta chamada ou do cache; o gate não tem como saber a diferença.
            if decisao.confiavel() {
                decisao.origem = origem;
            }
            // O gate devolve `Fallback` quando a resposta existe mas não passa no limiar: guarda a
            // confiança e a margem mesmo assim, para o relatório dizer o quanto faltou.
            if let Some(resposta) = resposta {
                brutas.insert(id, resposta.clone());
                decisao.confianca = resposta.confianca;
                decisao.margem = resposta.margem();
                decisao.ms = if origem == OrigemDecisao::Jev {
                    relato.ms
                } else {
                    0
                };
                decisao.tokens = if origem == OrigemDecisao::Jev {
                    relato.tokens
                } else {
                    0
                };
            }
            decisoes.push(decisao);
        }

        ResultadoJev {
            decisoes,
            brutas,
            chamada: relato,
        }
    }
}

impl AvaliadorGates for MotorJev {
    fn gates_efetivos(&self, ajuste: Option<AjusteGates>) -> GatesJev {
        match ajuste {
            Some(ajuste) => ajuste.sobre(self.globais),
            None => self.globais,
        }
    }

    fn avaliar<'a>(
        &'a self,
        estado: EstadoJev,
        ajuste: Option<AjusteGates>,
    ) -> BoxFuture<'a, ResultadoJev> {
        let gates = self.gates_efetivos(ajuste);
        Box::pin(async move {
            if Self::nenhum(gates) {
                return ResultadoJev::default();
            }
            self.avaliar_agora(estado, gates).await
        })
    }

    /// Segunda pergunta do G1 (HR-29): o tipo de risco imediato, para a orientação ao cliente.
    ///
    /// Chamada **fora** do lote: só acontece quando a transferência já foi decidida, então não
    /// custa nada nos turnos normais. Sem cache de propósito (é uma pergunta só, uma vez por
    /// transferência) e sem retentativa própria — o prazo aqui é o da transferência, e o cliente
    /// não pode esperar por ela.
    fn orientar<'a>(
        &'a self,
        estado: EstadoJev,
        prazo: Duration,
    ) -> BoxFuture<'a, ResultadoOrientacao> {
        Box::pin(async move {
            let mut perguntas = BTreeMap::new();
            perguntas.insert(
                crate::handoff::RISCO_PERGUNTA_ID.to_owned(),
                crate::handoff::pergunta_risco(),
            );
            let pedido = Pedido {
                estado: estado.para_json(),
                modelo: self.modelo.clone(),
                perguntas,
            };
            let inicio = Instant::now();
            match tokio::time::timeout(prazo, self.transporte.julgar(pedido)).await {
                Ok(Ok(lote)) => {
                    let resposta = lote.respostas.get(crate::handoff::RISCO_PERGUNTA_ID);
                    ResultadoOrientacao {
                        consultado: true,
                        tipo: resposta.and_then(|item| item.escolha.clone()),
                        confianca: resposta.map(|item| item.confianca).unwrap_or(0.0),
                        ms: u64::try_from(inicio.elapsed().as_millis()).unwrap_or(u64::MAX),
                        tokens: lote.tokens,
                        falha: None,
                    }
                }
                Ok(Err(erro)) => ResultadoOrientacao {
                    consultado: true,
                    ms: u64::try_from(inicio.elapsed().as_millis()).unwrap_or(u64::MAX),
                    falha: Some(erro.to_string()),
                    ..ResultadoOrientacao::default()
                },
                Err(_) => ResultadoOrientacao {
                    consultado: true,
                    ms: u64::try_from(inicio.elapsed().as_millis()).unwrap_or(u64::MAX),
                    falha: Some(format!(
                        "a pergunta de risco não respondeu em {} ms",
                        prazo.as_millis()
                    )),
                    ..ResultadoOrientacao::default()
                },
            }
        })
    }
}

impl MotorJev {
    /// `true` quando nenhum gate está ligado.
    pub fn nenhum(gates: GatesJev) -> bool {
        IdGate::TODOS
            .into_iter()
            .all(|gate| !Self::ligado(gates, gate))
    }
}
