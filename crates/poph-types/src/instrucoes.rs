//! Instruções estruturadas do agente (`StructuredInstructions`).
//!
//! Espelho exato do modelo do POP Atende (`core/agent/instructions_models.py`, HR-03 §9.1): os
//! mesmos nomes de campo, os mesmos padrões e a mesma tolerância a variação. Só tipos e regras
//! de validação — a composição do texto fica em `poph-core::compositor`.
//!
//! Notas de contrato:
//! - `linked_template_id` (Fichas) **não** faz parte do modelo: é fora da V1 e campo desconhecido
//!   no JSON é ignorado na leitura, então importar um agente do POP Atende não falha por isso.
//! - `flow_steps` trafega e é preservado, mas não tem efeito no motor (paridade com o POP Atende).
//! - `scope.handoff.reasons` é lido pelo Jev; `scope.handoff.department_id` é espelho do
//!   departamento padrão do agente. O compositor não emite nenhum dos dois.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::DomainError;

/// Ferramentas que o motor da V1 oferece ao LLM (lista fechada, decisão D-04).
///
/// O POP Atende tinha nomes próprios (`search_knowledge_base`, `lookup_product`) que não filtravam
/// nada; no Harness a lista de ferramentas habilitadas **filtra** o toolset. Nome fora desta lista
/// é recusado na escrita, para o agente nunca ficar com uma ferramenta que não existe.
pub const FERRAMENTAS_V1: &[&str] = &[
    "consultar_base",
    "buscar_produto",
    "enviar_arquivo",
    "transferir",
    "cart_add_item",
    "cart_remove_item",
    "cart_summary",
    "cart_clear",
    // Fluxos e Fichas (FX-24): ação, abandono e o início da reserva do Jev. Sem elas aqui, o
    // agente não conseguiria habilitá-las (a escrita recusa nome fora da lista) e a ferramenta
    // existiria sem nunca ser oferecida.
    "fluxo_executar_acao",
    "fluxo_abandonar",
    "fluxo_iniciar",
];

/// Identidade do agente.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Identidade {
    /// Nome do assistente (entra no cabeçalho do markdown e é o nome no Jev).
    pub name: String,
    /// Papel/função (entra no cabeçalho, depois de uma vírgula).
    pub role: String,
    /// Empresa/negócio (entra no cabeçalho, depois de ` da `).
    pub company: String,
    /// Descrição da empresa (entra como bloco logo abaixo do cabeçalho).
    pub company_description: String,
}

/// Princípio inegociável, com a ação de contingência.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Principio {
    /// Regra, em uma frase.
    pub rule: String,
    /// Ação quando a regra não pode ser cumprida: `say_dont_know` | `transfer` | `escalate`.
    ///
    /// É só texto no markdown: `transfer` e `escalate` não viram ação de sistema.
    #[serde(default = "fallback_padrao")]
    pub fallback_action: String,
}

fn fallback_padrao() -> String {
    "say_dont_know".to_owned()
}

impl Default for Principio {
    fn default() -> Self {
        Self {
            rule: String::new(),
            fallback_action: fallback_padrao(),
        }
    }
}

/// Configuração de encaminhamento dentro do escopo.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct HandoffEscopo {
    /// Departamento de destino da transferência (espelho do departamento padrão do agente).
    pub department_id: String,
    /// Motivos de transferência lidos pelo Jev. O LLM nunca os vê.
    pub reasons: Vec<String>,
}

/// O que o agente faz e o que não faz.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Escopo {
    /// O que o agente faz.
    pub does: String,
    /// O que o agente não faz.
    pub does_not: String,
    /// Encaminhamento (não vai para o markdown).
    pub handoff: HandoffEscopo,
}

/// Passo de fluxo. Trafega no JSON e não tem efeito (paridade com o POP Atende).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PassoFluxo {
    /// Nome do passo.
    pub name: String,
    /// Descrição do passo.
    pub description: String,
    /// Ferramentas usadas no passo.
    #[serde(default)]
    pub tools_used: Vec<String>,
    /// Passo veio de um template (Fichas).
    #[serde(default)]
    pub auto_from_template: bool,
}

/// Tom e formato da comunicação.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Tom {
    /// Idioma: `pt-BR` | `en` | `es` (valor fora do mapa passa cru para o markdown).
    pub language: String,
    /// Estilo: `professional` | `casual` | `technical` | `friendly`.
    pub style: String,
    /// Política de emojis: `moderate` | `none` | `free`.
    pub emoji_policy: String,
    /// Formato: `simple_lists` | `free_text` | `tables`.
    pub format: String,
    /// Restrições adicionais (uma linha de lista).
    pub extra_restrictions: String,
}

impl Default for Tom {
    fn default() -> Self {
        Self {
            language: "pt-BR".to_owned(),
            style: "professional".to_owned(),
            emoji_policy: "moderate".to_owned(),
            format: "simple_lists".to_owned(),
            extra_restrictions: String::new(),
        }
    }
}

/// Ferramenta habilitada (ou não) para o agente.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FerramentaConfig {
    /// Nome da ferramenta (na V1, um de [`FERRAMENTAS_V1`]).
    pub name: String,
    /// `true` = o motor oferece a ferramenta ao LLM e ela entra no markdown.
    /// No contrato do POP Atende o padrão é `true` quando o campo não vem.
    #[serde(default = "enabled_padrao")]
    pub enabled: bool,
    /// Quando usar (texto livre, entra após o nome no markdown).
    #[serde(default)]
    pub when_to_use: String,
}

fn enabled_padrao() -> bool {
    true
}

impl Default for FerramentaConfig {
    fn default() -> Self {
        Self {
            name: String::new(),
            enabled: true,
            when_to_use: String::new(),
        }
    }
}

impl FerramentaConfig {
    /// Item com o padrão do contrato (`enabled = true`).
    pub fn nova(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            enabled: true,
            when_to_use: String::new(),
        }
    }
}

/// Instruções estruturadas do agente.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct StructuredInstructions {
    /// Identidade.
    pub identity: Identidade,
    /// Princípios inegociáveis.
    pub principles: Vec<Principio>,
    /// Escopo.
    pub scope: Escopo,
    /// Passos de fluxo (sem efeito no motor).
    pub flow_steps: Vec<PassoFluxo>,
    /// Tom e formato.
    pub tone: Tom,
    /// Ferramentas habilitadas (D-04: filtram o toolset).
    pub tools: Vec<FerramentaConfig>,
    /// Instruções extras.
    pub extra_instructions: String,
}

impl StructuredInstructions {
    /// Lê o JSON tolerando as formas que existem no POP Atende: objeto, texto JSON e o
    /// duplo-encoding legado (string dentro de string). É o que o importador (HR-52) precisa.
    pub fn de_json_tolerante(texto: &str) -> Result<Self, DomainError> {
        let valor: Value = serde_json::from_str(texto).map_err(|erro| DomainError::Validacao {
            campo: "structured_json",
            motivo: format!("JSON inválido: {erro}"),
        })?;
        let objeto = match valor {
            Value::String(interno) => {
                serde_json::from_str::<Value>(&interno).map_err(|erro| DomainError::Validacao {
                    campo: "structured_json",
                    motivo: format!("JSON duplamente codificado e inválido: {erro}"),
                })?
            }
            outro => outro,
        };
        if !objeto.is_object() {
            return Err(DomainError::Validacao {
                campo: "structured_json",
                motivo: "o valor precisa ser um objeto JSON".to_owned(),
            });
        }
        serde_json::from_value(objeto).map_err(|erro| DomainError::Validacao {
            campo: "structured_json",
            motivo: erro.to_string(),
        })
    }

    /// JSON canônico do modelo (para gravar em `agent_instructions.structured_json`).
    pub fn para_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{}".to_owned())
    }

    /// Regras que o motor exige na escrita (D-04: lista de ferramentas fechada).
    ///
    /// Não roda na leitura: um JSON legado do POP Atende com nomes antigos precisa ser lido e
    /// mapeado pelo importador, não recusado.
    pub fn validar(&self) -> Result<(), DomainError> {
        for ferramenta in &self.tools {
            if !FERRAMENTAS_V1.contains(&ferramenta.name.as_str()) {
                return Err(DomainError::Validacao {
                    campo: "tools[].name",
                    motivo: format!(
                        "`{}` não é ferramenta da V1 (use uma de: {})",
                        ferramenta.name,
                        FERRAMENTAS_V1.join(", ")
                    ),
                });
            }
        }
        Ok(())
    }

    /// Ferramentas marcadas como habilitadas, na ordem do formulário (D-04: é o toolset do agente).
    pub fn ferramentas_habilitadas(&self) -> Vec<&FerramentaConfig> {
        self.tools
            .iter()
            .filter(|ferramenta| ferramenta.enabled)
            .collect()
    }
}
