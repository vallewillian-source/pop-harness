use std::collections::BTreeMap;
use std::time::Duration;

use poph_types::HandoffConfig;

use crate::types::Question;

pub const RISK_QUESTION_ID: &str = "risco_imediato";
pub const NO_RISK: &str = "nenhum";
pub const INGESTION_RISK: &str = "ingestao_contato_produto";
pub const ANIMAL_RISK: &str = "animal_peconhento";
pub const OTHER_RISK: &str = "outro_risco_saude";

pub const RISK_TYPES: [&str; 3] = [INGESTION_RISK, ANIMAL_RISK, OTHER_RISK];

pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(4);

const INSTRUCTIONS: &str = "A conversa JÁ foi transferida para um atendente humano. Classifique, \
pela ÚLTIMA mensagem do cliente lida no contexto anterior, se existe RISCO IMEDIATO à saúde de \
alguém e de que tipo: a escolha define a orientação de primeiros cuidados que o cliente recebe \
enquanto espera. Julgue o que o cliente DESCREVE, não o que ele pede. Pedido de pessoa, \
reclamação, cobrança, orçamento e dúvida comum não são risco. Na ausência de sinal claro de risco \
à saúde, escolha 'nenhum'.";

#[derive(Debug, Clone, PartialEq)]
pub struct HandoffGuidance {
    pub ativo: bool,
    pub confidence: f64,
    pub timeout: Duration,
    pub mensagem_automatica: String,
    pub texts: BTreeMap<String, String>,
}

impl Default for HandoffGuidance {
    fn default() -> Self {
        Self::from_config(&HandoffConfig::default())
    }
}

impl HandoffGuidance {
    pub fn from_config(config: &HandoffConfig) -> Self {
        let mut texts = BTreeMap::new();
        texts.insert(
            INGESTION_RISK.to_owned(),
            config.guidance_product_ingestion.clone(),
        );
        texts.insert(
            ANIMAL_RISK.to_owned(),
            config.guidance_venomous_animal.clone(),
        );
        texts.insert(
            OTHER_RISK.to_owned(),
            config.guidance_other_health_risk.clone(),
        );
        Self {
            ativo: config.guidance_enabled,
            confidence: config.guidance_confidence.clamp(0.0, 1.0),
            timeout: Duration::from_secs(u64::from(config.guidance_timeout_seconds.max(1))),
            mensagem_automatica: config.automatic_message.clone(),
            texts,
        }
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn with_text(mut self, tipo: &str, texto: impl Into<String>) -> Self {
        self.texts.insert(tipo.to_owned(), texto.into());
        self
    }

    pub fn texto(&self, tipo: &str) -> &str {
        self.texts.get(tipo).map(String::as_str).unwrap_or("")
    }

    pub fn question(&self) -> Question {
        risk_question()
    }

    pub fn reliable_type(&self, resultado: &GuidanceResult) -> Option<String> {
        if !self.ativo || !resultado.queried || resultado.failure.is_some() {
            return None;
        }
        let tipo = resultado.tipo.as_deref()?;
        if !RISK_TYPES.contains(&tipo) {
            return None;
        }
        if resultado.confidence < self.confidence {
            return None;
        }
        if self.texto(tipo).trim().is_empty() {
            return None;
        }
        Some(tipo.to_owned())
    }

    pub fn compose(&self, tipo: Option<&str>) -> String {
        let automatic_message = self.mensagem_automatica.clone();
        match tipo {
            Some(tipo) if !self.texto(tipo).trim().is_empty() => {
                format!("{}\n\n{automatic_message}", self.texto(tipo))
            }
            _ => automatic_message,
        }
    }
}

pub fn risk_question() -> Question {
    Question::choice(INSTRUCTIONS, options())
}

fn options() -> BTreeMap<String, String> {
    let mut options = BTreeMap::new();
    options.insert(
        NO_RISK.to_owned(),
        "A conversa não indica risco imediato à saúde de ninguém: pedido de atendente/pessoa, \
         reclamação, cobrança, orçamento, agendamento, dúvida comum, problema com serviço já \
         feito. Falar de produto, praga, veneno ou isca SEM pessoa exposta ou animal peçonhento \
         presente também é 'nenhum'."
            .to_owned(),
    );
    options.insert(
        INGESTION_RISK.to_owned(),
        "Alguém engoliu, inalou, tocou ou de outra forma teve contato com produto químico, veneno, \
         isca ou inseticida — criança, adulto ou animal de estimação. Sintomas depois da exposição \
         (vômito, ardência, mal-estar) também entram aqui."
            .to_owned(),
    );
    options.insert(
        ANIMAL_RISK.to_owned(),
        "Escorpião, aranha, cobra, abelha/marimbondo ou outro animal peçonhento está presente no \
         local ou picou alguém. Vale mesmo que ninguém tenha sido picado ainda (animal no berço, \
         no quintal, dentro de casa)."
            .to_owned(),
    );
    options.insert(
        OTHER_RISK.to_owned(),
        "Risco imediato à saúde ou à vida que não é ingestão/contato com produto nem animal \
         peçonhento: falta de ar, convulsão, desmaio, dor no peito, sangramento, queimadura, \
         reação alérgica grave, mal-estar súbito."
            .to_owned(),
    );
    options
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct GuidanceResult {
    pub queried: bool,
    pub tipo: Option<String>,
    pub confidence: f64,
    pub ms: u64,
    pub tokens: u64,
    pub failure: Option<String>,
}

impl GuidanceResult {
    pub fn not_queried(motivo: impl Into<String>) -> Self {
        Self {
            queried: false,
            failure: Some(motivo.into()),
            ..Self::default()
        }
    }

    pub fn failed(&self) -> bool {
        self.failure.is_some()
    }

    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "consultado": self.queried,
            "tipo": self.tipo,
            "confianca": self.confidence,
            "ms": self.ms,
            "tokens": self.tokens,
            "falha": self.failure,
        })
    }

    pub fn summary(&self) -> String {
        if let Some(failure) = &self.failure {
            return format!("risco não classificado ({failure})");
        }
        format!(
            "risco={} (confiança {:.2})",
            self.tipo.as_deref().unwrap_or("—"),
            self.confidence
        )
    }
}
