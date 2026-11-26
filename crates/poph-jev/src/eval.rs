//! Conjunto rotulado e medição de um gate (`poph jev eval <gate> <arquivo>`).
//!
//! O PRD §10 pede, para cada gate, um conjunto rotulado com casos difíceis e de abstenção, e um
//! relatório de acerto, abstenção e latência. Formato do arquivo (JSON Lines, uma linha por caso):
//!
//! ```json
//! {"estado": {"canal": "cli", "mensagens": [{"papel": "cliente", "texto": "quero falar com alguém"}]},
//!  "esperado": "transferir_agora"}
//! ```
//!
//! Regras de contagem:
//! - **abstenção** = o gate não deu decisão confiável (fallback) — é o que mede o quanto ele cala;
//! - **acerto** = decisão confiável igual ao esperado;
//! - **erro** = decisão confiável diferente do esperado;
//! - latência por caso: do pedido ao Jev até a decisão (0 quando veio do cache).

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::gate::DecisionOrigin;
use crate::motor::{AvaliadorGates, ResultadoJev};
use crate::state::JevState;
use crate::types::{IdGate, JevError, UsageTokens};

/// Um caso do conjunto rotulado.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CasoRotulado {
    /// Estado que o gate vai julgar.
    pub estado: JevState,
    /// Resposta certa para o caso (`transferir_agora`, `nada`, nome do departamento…).
    pub esperado: String,
    /// Agrupamento do caso no relatório (`insatisfacao_urgencia`, `risco_saude`…).
    ///
    /// Opcional: conjunto antigo sem categoria continua valendo e medindo igual.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub categoria: Option<String>,
    /// Por que este rótulo é o certo, em uma linha.
    ///
    /// Fica no conjunto, não só no relatório: quem revisa um caso discutível lê o motivo ao lado
    /// dele, e a próxima medição herda a explicação.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub justificativa: Option<String>,
}

/// Erro ao ler o conjunto.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ErroConjunto {
    /// Arquivo não encontrado ou sem permissão.
    #[error("não consegui ler {caminho}: {motivo}")]
    Leitura {
        /// Caminho pedido.
        caminho: String,
        /// Motivo do sistema.
        motivo: String,
    },
    /// Linha fora do formato.
    #[error("{caminho}:{linha}: caso inválido ({motivo})")]
    Caso {
        /// Caminho do conjunto.
        caminho: String,
        /// Número da linha (1-based).
        linha: usize,
        /// O que o parser reclamou.
        motivo: String,
    },
    /// Conjunto vazio.
    #[error("{caminho}: o conjunto está vazio")]
    Vazio {
        /// Caminho do conjunto.
        caminho: String,
    },
}

/// Lê um conjunto rotulado em JSON Lines.
pub fn ler_conjunto(caminho: &Path) -> Result<Vec<CasoRotulado>, ErroConjunto> {
    let texto = std::fs::read_to_string(caminho).map_err(|erro| ErroConjunto::Leitura {
        caminho: caminho.display().to_string(),
        motivo: erro.to_string(),
    })?;
    let mut casos = Vec::new();
    for (indice, linha) in texto.lines().enumerate() {
        let linha = linha.trim();
        if linha.is_empty() || linha.starts_with('#') {
            continue;
        }
        let caso: CasoRotulado =
            serde_json::from_str(linha).map_err(|erro| ErroConjunto::Caso {
                caminho: caminho.display().to_string(),
                linha: indice + 1,
                motivo: erro.to_string(),
            })?;
        casos.push(caso);
    }
    if casos.is_empty() {
        return Err(ErroConjunto::Vazio {
            caminho: caminho.display().to_string(),
        });
    }
    Ok(casos)
}

/// O que aconteceu em um caso.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DetalheCaso {
    /// Esperado (do conjunto).
    pub esperado: String,
    /// O que o gate decidiu (`None` quando absteve).
    pub obtido: Option<String>,
    /// Opção que o **Jev respondeu**, mesmo quando a decisão caiu no fallback.
    ///
    /// É o dado que faltava para calibrar limiar: `obtido` diz o que vale para o turno, `escolha`
    /// diz o que o modelo queria — quando os dois divergem, a resposta existia e não passou no
    /// limiar (ou veio fora das opções).
    pub escolha: Option<String>,
    /// Confiança da resposta, como o Jev declarou (0 quando não houve resposta).
    pub confianca: String,
    /// Motivo do fallback (limiar, margem, escolha inválida, falta de resposta).
    pub motivo: Option<String>,
    /// Por que a chamada ao Jev falhou neste caso (prazo, erro do provedor).
    pub falha: Option<String>,
    /// Categoria do caso no conjunto (quando declarada).
    pub categoria: Option<String>,
    /// Acertou, errou ou absteve.
    pub resultado: String,
}

/// Relatório de um gate sobre um conjunto.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RelatorioEval {
    /// Gate medido.
    pub gate: String,
    /// Quantos casos.
    pub casos: usize,
    /// Decisões confiáveis iguais ao esperado.
    pub acertos: usize,
    /// Decisões confiáveis diferentes do esperado.
    pub erros: usize,
    /// Casos em que o gate não decidiu (fallback).
    pub abstencoes: usize,
    /// Casos em que o lote do Jev falhou.
    pub falhas_de_chamada: usize,
    /// Latências por caso, em ms.
    pub latencias_ms: Vec<u64>,
    /// Confiança média das decisões confiáveis.
    pub confianca_media: f64,
    /// Tokens consumidos na execução, somando o `usage` de cada chamada (o cache não conta).
    pub usage: UsageTokens,
    /// Detalhe por caso.
    pub detalhes: Vec<DetalheCaso>,
}

impl RelatorioEval {
    /// Fração de acertos sobre o total.
    pub fn taxa_acerto(&self) -> f64 {
        if self.casos == 0 {
            return 0.0;
        }
        self.acertos as f64 / self.casos as f64
    }

    /// Uma linha por categoria: quantos casos, acertos, abstenções e erros.
    ///
    /// `None` quando o conjunto não declara categoria (compatível com o formato antigo).
    pub fn resumo_por_categoria(&self) -> Option<String> {
        let mut por_categoria: std::collections::BTreeMap<&str, (usize, usize, usize, usize)> =
            std::collections::BTreeMap::new();
        for detalhe in &self.detalhes {
            let Some(categoria) = detalhe.categoria.as_deref() else {
                continue;
            };
            let item = por_categoria.entry(categoria).or_insert((0, 0, 0, 0));
            item.0 += 1;
            match detalhe.resultado.as_str() {
                "acerto" => item.1 += 1,
                "erro" => item.3 += 1,
                _ => item.2 += 1,
            }
        }
        if por_categoria.is_empty() {
            return None;
        }
        let mut saida = String::new();
        for (categoria, (casos, acertos, abstencoes, erros)) in por_categoria {
            saida.push_str(&format!(
                "  {categoria}: {casos} casos · {acertos} acerto · {abstencoes} abstenção · {erros} erro\n"
            ));
        }
        Some(saida)
    }

    /// Fração de abstenções sobre o total.
    pub fn taxa_abstencao(&self) -> f64 {
        if self.casos == 0 {
            return 0.0;
        }
        self.abstencoes as f64 / self.casos as f64
    }

    /// Latência média, em ms.
    pub fn latencia_media(&self) -> f64 {
        if self.latencias_ms.is_empty() {
            return 0.0;
        }
        let soma: u64 = self.latencias_ms.iter().sum();
        soma as f64 / self.latencias_ms.len() as f64
    }

    /// Percentil da latência (0–100), em ms.
    pub fn latencia_percentil(&self, percentil: f64) -> u64 {
        if self.latencias_ms.is_empty() {
            return 0;
        }
        let mut ordenadas = self.latencias_ms.clone();
        ordenadas.sort_unstable();
        let posicao = ((percentil / 100.0) * (ordenadas.len() as f64 - 1.0)).round() as usize;
        ordenadas[posicao.min(ordenadas.len() - 1)]
    }

    /// Texto para o terminal.
    pub fn texto(&self, titulo: &str) -> String {
        let mut saida = String::new();
        saida.push_str(&format!("Jev · {} — {titulo}\n", self.gate));
        saida.push_str(&format!("casos: {}\n", self.casos));
        saida.push_str(&format!(
            "acerto: {}/{} ({:.1}%)\n",
            self.acertos,
            self.casos,
            self.taxa_acerto() * 100.0
        ));
        saida.push_str(&format!(
            "abstenção: {}/{} ({:.1}%)\n",
            self.abstencoes,
            self.casos,
            self.taxa_abstencao() * 100.0
        ));
        saida.push_str(&format!("erros: {}\n", self.erros));
        saida.push_str(&format!(
            "latência: média {:.0} ms · p50 {} ms · p95 {} ms\n",
            self.latencia_media(),
            self.latencia_percentil(50.0),
            self.latencia_percentil(95.0)
        ));
        saida.push_str(&format!(
            "confiança média (decisões confiáveis): {:.2}\n",
            self.confianca_media
        ));
        saida.push_str(&format!("tokens: {}\n", self.usage.as_text()));
        if self.falhas_de_chamada > 0 {
            saida.push_str(&format!(
                "falhas da chamada ao Jev: {} (contam como abstenção)\n",
                self.falhas_de_chamada
            ));
        }
        if let Some(resumo) = self.resumo_por_categoria() {
            saida.push_str("\npor categoria (acerto/abstenção/erro):\n");
            saida.push_str(&resumo);
        }
        saida.push_str("\ndetalhe por caso (escolha do Jev · confiança · motivo do fallback):\n");
        for (indice, detalhe) in self.detalhes.iter().enumerate() {
            saida.push_str(&format!(
                "  #{:02} {} · esperado {} · jev {}{}\n",
                indice + 1,
                detalhe.resultado,
                detalhe.esperado,
                detalhe.escolha.as_deref().unwrap_or("—"),
                detalhe
                    .categoria
                    .as_deref()
                    .map(|categoria| format!(" · {categoria}"))
                    .unwrap_or_default()
            ));
            let mut extras = vec![format!("confiança {}", detalhe.confianca)];
            if let Some(motivo) = &detalhe.motivo {
                extras.push(format!("motivo: {motivo}"));
            }
            if let Some(falha) = &detalhe.falha {
                extras.push(format!("falha: {falha}"));
            }
            saida.push_str(&format!("      {}\n", extras.join(" · ")));
        }
        saida
    }
}

/// Roda o conjunto por um avaliador, com **só** o gate medido ligado.
pub async fn avaliar_conjunto(
    avaliador: &dyn AvaliadorGates,
    gate: IdGate,
    casos: &[CasoRotulado],
) -> RelatorioEval {
    let mut relatorio = RelatorioEval {
        gate: gate.as_text().to_owned(),
        casos: casos.len(),
        acertos: 0,
        erros: 0,
        abstencoes: 0,
        falhas_de_chamada: 0,
        latencias_ms: Vec::with_capacity(casos.len()),
        confianca_media: 0.0,
        usage: UsageTokens::default(),
        detalhes: Vec::with_capacity(casos.len()),
    };
    let mut soma_confianca = 0.0_f64;
    let mut confiaveis = 0_usize;

    for caso in casos {
        // O conjunto pede um gate só: o avaliador é chamado com esse ajuste explícito.
        let ajuste = ajuste_de_gate(gate);
        let resultado: ResultadoJev = avaliador.avaliar(caso.estado.clone(), Some(ajuste)).await;
        if resultado.falhou() {
            relatorio.falhas_de_chamada += 1;
        }
        // O consumo é somado por execução: chamada que veio do cache não custa nada.
        relatorio.usage = relatorio.usage.plus(resultado.chamada.usage);
        // O que o modelo respondeu, independentemente de a decisão ter passado no limiar.
        let escolha_do_jev = resultado
            .resposta(gate)
            .and_then(|resposta| resposta.choice.clone());
        let motivo = resultado
            .decisao(gate)
            .and_then(|decisao| decisao.motivo.clone());
        let falha = resultado.chamada.falha.clone();
        let decisao = resultado.decisao(gate);
        let (obtido, confianca, resultado_texto) = match decisao {
            Some(decisao) if decisao.origin != DecisionOrigin::Fallback => {
                soma_confianca += decisao.confidence;
                confiaveis += 1;
                let obtido = decisao.choice.clone();
                let acertou = obtido.as_deref() == Some(caso.esperado.as_str());
                if acertou {
                    relatorio.acertos += 1;
                } else {
                    relatorio.erros += 1;
                }
                (
                    obtido,
                    format!("{:.2}", decisao.confidence),
                    if acertou { "acerto" } else { "erro" }.to_owned(),
                )
            }
            Some(decisao) => {
                relatorio.abstencoes += 1;
                (
                    None,
                    format!("{:.2}", decisao.confidence),
                    "abstencao".to_owned(),
                )
            }
            None => {
                // Gate não se aplicou a este estado: conta como abstenção, não como erro.
                relatorio.abstencoes += 1;
                (None, "0.00".to_owned(), "nao_aplicavel".to_owned())
            }
        };
        relatorio
            .latencias_ms
            .push(resultado.chamada.ms.max(decisao.map(|d| d.ms).unwrap_or(0)));
        relatorio.detalhes.push(DetalheCaso {
            esperado: caso.esperado.clone(),
            obtido,
            escolha: escolha_do_jev,
            confianca,
            motivo,
            falha,
            categoria: caso.categoria.clone(),
            resultado: resultado_texto,
        });
    }

    relatorio.confianca_media = if confiaveis == 0 {
        0.0
    } else {
        soma_confianca / confiaveis as f64
    };
    relatorio
}

/// Ajuste que liga só o gate medido.
pub fn ajuste_de_gate(gate: IdGate) -> crate::flags::GateOverrides {
    crate::flags::GateOverrides::only(gate)
}

/// Erro de Jev usado pelo conjunto quando a chave não está configurada.
pub fn erro_sem_chave() -> JevError {
    JevError::MissingKey
}
