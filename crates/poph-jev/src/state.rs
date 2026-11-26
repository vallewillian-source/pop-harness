use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub const MAX_MESSAGES: usize = 10;
pub const MAX_MESSAGE_CHARS: usize = 800;
pub const MAX_HANDOFF_SITUATIONS: usize = 15;
pub const MAX_SITUATION_CHARS: usize = 300;
pub const MAX_PRODUCT_CANDIDATES: usize = 10;
pub const MAX_CANDIDATE_CHARS: usize = 300;
pub const MAX_CANDIDATE_RESPONSE_CHARS: usize = 4000;
pub const MAX_FILE_TEXT_CHARS: usize = 20_000;
pub const MAX_RELEASED_FLOWS: usize = 20;
pub const MAX_TRIGGER_CHARS: usize = 400;
pub const MAX_FLOW_EXAMPLES: usize = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JevRole {
    #[serde(rename = "cliente")]
    Customer,
    #[serde(rename = "agente")]
    Agent,
}

impl JevRole {
    pub fn as_text(self) -> &'static str {
        match self {
            JevRole::Customer => "cliente",
            JevRole::Agent => "agente",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProductCandidate {
    pub id: String,
    pub nome: String,
    #[serde(default, rename = "detalhe")]
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JevFlow {
    pub id: String,
    pub nome: String,
    #[serde(default, rename = "quando")]
    pub when: String,
    #[serde(default, rename = "nao_quando")]
    pub not_when: String,
    #[serde(default, rename = "exemplos")]
    pub examples: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JevMessage {
    #[serde(rename = "papel")]
    pub role: JevRole,
    pub texto: String,
}

impl JevMessage {
    pub fn from_customer(texto: impl Into<String>) -> Self {
        Self::with_role(JevRole::Customer, texto)
    }

    pub fn from_agent(texto: impl Into<String>) -> Self {
        Self::with_role(JevRole::Agent, texto)
    }

    fn with_role(role: JevRole, texto: impl Into<String>) -> Self {
        Self {
            role,
            texto: truncate(&texto.into(), MAX_MESSAGE_CHARS),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct JevState {
    #[serde(default)]
    pub canal: String,
    #[serde(default, rename = "mensagens")]
    pub messages: Vec<JevMessage>,
    #[serde(default, rename = "situacoes_de_handoff")]
    pub handoff_situations: Vec<String>,
    #[serde(default, rename = "departamentos")]
    pub departments: Vec<String>,
    #[serde(default, rename = "transferencia_habilitada")]
    pub transfer_enabled: bool,
    #[serde(default, rename = "base_disponivel")]
    pub knowledge_base_available: bool,
    #[serde(default, rename = "produtos_disponiveis")]
    pub products_available: bool,
    #[serde(default, rename = "produtos_candidatos")]
    pub product_candidates: Vec<ProductCandidate>,
    #[serde(
        default,
        rename = "resposta_candidata",
        skip_serializing_if = "Option::is_none"
    )]
    pub candidate_response: Option<String>,
    #[serde(default, rename = "fluxos_liberados")]
    pub released_flows: Vec<JevFlow>,
    #[serde(
        default,
        rename = "texto_de_arquivo",
        skip_serializing_if = "Option::is_none"
    )]
    pub file_text: Option<String>,
}

impl JevState {
    pub fn new(canal: impl Into<String>) -> Self {
        Self {
            canal: canal.into(),
            ..Self::default()
        }
    }

    pub fn with_messages(mut self, messages: impl IntoIterator<Item = JevMessage>) -> Self {
        let mut all: Vec<JevMessage> = messages.into_iter().collect();
        if all.len() > MAX_MESSAGES {
            all.drain(..all.len() - MAX_MESSAGES);
        }
        self.messages = all;
        self
    }

    pub fn with_handoff(mut self, situations: Vec<String>, enabled: bool) -> Self {
        self.handoff_situations = situations
            .into_iter()
            .take(MAX_HANDOFF_SITUATIONS)
            .map(|situacao| truncate(&situacao, MAX_SITUATION_CHARS))
            .collect();
        self.transfer_enabled = enabled;
        self
    }

    pub fn with_departments(mut self, departments: Vec<String>) -> Self {
        self.departments = departments;
        self
    }

    pub fn with_capabilities(
        mut self,
        knowledge_base_available: bool,
        products_available: bool,
    ) -> Self {
        self.knowledge_base_available = knowledge_base_available;
        self.products_available = products_available;
        self
    }

    pub fn with_product_candidates(mut self, candidates: Vec<ProductCandidate>) -> Self {
        self.product_candidates = candidates
            .into_iter()
            .take(MAX_PRODUCT_CANDIDATES)
            .map(|candidate| ProductCandidate {
                id: truncate(&candidate.id, MAX_CANDIDATE_CHARS),
                nome: truncate(&candidate.nome, MAX_CANDIDATE_CHARS),
                detail: truncate(&candidate.detail, MAX_CANDIDATE_CHARS),
            })
            .collect();
        self
    }

    pub fn with_candidate_response(mut self, texto: impl Into<String>) -> Self {
        self.candidate_response = Some(truncate(&texto.into(), MAX_CANDIDATE_RESPONSE_CHARS));
        self
    }

    pub fn with_file_text(mut self, texto: impl Into<String>) -> Self {
        self.file_text = Some(truncate(&texto.into(), MAX_FILE_TEXT_CHARS));
        self
    }

    pub fn with_released_flows(mut self, flows: impl IntoIterator<Item = JevFlow>) -> Self {
        self.released_flows = flows
            .into_iter()
            .map(|fluxo| JevFlow {
                id: truncate(&fluxo.id, MAX_CANDIDATE_CHARS),
                nome: truncate(&fluxo.nome, MAX_CANDIDATE_CHARS),
                when: truncate(&fluxo.when, MAX_TRIGGER_CHARS),
                not_when: truncate(&fluxo.not_when, MAX_TRIGGER_CHARS),
                examples: fluxo
                    .examples
                    .into_iter()
                    .take(MAX_FLOW_EXAMPLES)
                    .map(|example| truncate(&example, MAX_TRIGGER_CHARS))
                    .collect(),
            })
            .collect();
        self
    }

    pub fn last_message(&self) -> Option<&JevMessage> {
        self.messages.last()
    }
    pub fn last_from_customer(&self) -> Option<&JevMessage> {
        self.messages
            .iter()
            .rev()
            .find(|mensagem| mensagem.role == JevRole::Customer)
    }

    pub fn to_json(&self) -> Value {
        let mut value = json!({
            "canal": self.canal,
            "conversa": {
                "mensagens": self
                    .messages
                    .iter()
                    .map(|mensagem| json!({
                        "papel": mensagem.role.as_text(),
                        "texto": mensagem.texto,
                    }))
                    .collect::<Vec<_>>(),
            },
            "politica": {
                "situacoes_de_handoff": self.handoff_situations,
                "transferencia_habilitada": self.transfer_enabled,
                "departamentos": self.departments,
            },
            "capacidades": {
                "base_de_conhecimento": self.knowledge_base_available,
                "produtos": self.products_available,
            },
            "catalogo": {
                "candidatos": self
                    .product_candidates
                    .iter()
                    .map(|candidate| json!({
                        "id": candidate.id,
                        "nome": candidate.nome,
                        "detalhe": candidate.detail,
                    }))
                    .collect::<Vec<_>>(),
            },
        });
        if let Some(candidate) = &self.candidate_response {
            value["resposta_candidata"] = json!(candidate);
        }
        if !self.released_flows.is_empty() {
            value["fluxos"] = json!({
                "liberados": self
                    .released_flows
                    .iter()
                    .map(|fluxo| json!({
                        "id": fluxo.id,
                        "nome": fluxo.nome,
                        "quando": fluxo.when,
                        "nao_quando": fluxo.not_when,
                        "exemplos": fluxo.examples,
                    }))
                    .collect::<Vec<_>>(),
            });
        }
        if let Some(texto) = &self.file_text {
            value["arquivo"] = json!({ "texto": texto });
        }
        value
    }

    pub fn hash(&self) -> u64 {
        let texto = serde_json::to_string(&self.to_json()).unwrap_or_default();
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for byte in texto.as_bytes() {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        hash
    }
}

fn truncate(texto: &str, max: usize) -> String {
    if texto.chars().count() <= max {
        return texto.to_owned();
    }
    let mut cut: String = texto.chars().take(max).collect();
    cut.push('…');
    cut
}
