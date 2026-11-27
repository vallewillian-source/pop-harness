use serde::{Deserialize, Serialize};

use poph_types::{ActionActor, FieldKind};

pub const SCHEMA_VERSAO: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ir {
    pub schema_versao: u32,
    pub fluxo: Fluxo,
    pub gatilho: Gatilho,
    #[serde(rename = "inicio")]
    pub start: String,
    #[serde(rename = "etapas")]
    pub stages: Vec<Etapa>,
    #[serde(rename = "decisoes")]
    pub decisions: Vec<Decisao>,
    #[serde(rename = "fins")]
    pub ends: Vec<Fim>,
    pub defs: Vec<Def>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fluxo {
    pub id: String,
    pub nome: String,
    pub descricao: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Gatilho {
    #[serde(rename = "quando")]
    pub when: String,
    #[serde(rename = "nao_quando")]
    pub not_when: Option<String>,
    #[serde(rename = "exemplos")]
    pub examples: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Etapa {
    pub id: String,
    pub nome: String,
    #[serde(rename = "instrucao")]
    pub instruction: Option<String>,
    #[serde(rename = "atores_acao")]
    pub action_actors: Vec<ActionActor>,
    pub campos: Vec<Campo>,
    #[serde(rename = "acoes")]
    pub actions: Vec<Acao>,
    #[serde(rename = "ao_entrar")]
    pub on_enter: Vec<Effect>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Campo {
    pub id: String,
    #[serde(rename = "rotulo")]
    pub label: String,
    pub tipo: FieldKind,
    #[serde(rename = "obrigatorio")]
    pub required: bool,
    #[serde(rename = "opcoes")]
    pub options: Vec<String>,
    #[serde(rename = "ajuda")]
    pub help: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Acao {
    pub id: String,
    #[serde(rename = "rotulo")]
    pub label: String,
    #[serde(rename = "exige")]
    pub requires: Vec<String>,
    #[serde(rename = "ator")]
    pub actor: Option<ActionActor>,
    #[serde(rename = "vai_para")]
    pub goes_to: Destination,
    #[serde(rename = "efeitos")]
    pub effects: Vec<Effect>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Decisao {
    pub id: String,
    pub se: Rule,
    #[serde(rename = "entao")]
    pub then_branch: Destination,
    #[serde(rename = "senao")]
    pub else_branch: Destination,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fim {
    pub id: String,
    pub resultado: Option<String>,
    #[serde(rename = "encerra_atendimento")]
    pub ends_service: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Def {
    pub nome: String,
    pub fonte: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "tipo", rename_all = "snake_case")]
pub enum Destination {
    Etapa {
        id: String,
    },
    Decisao {
        id: String,
    },
    Fim {
        id: String,
        resultado: Option<String>,
        #[serde(rename = "encerra_atendimento")]
        ends_service: bool,
    },
}

impl Destination {
    pub fn id(&self) -> &str {
        match self {
            Self::Etapa { id } | Self::Decisao { id } | Self::Fim { id, .. } => id,
        }
    }

    pub fn is_open(&self) -> Option<&str> {
        match self {
            Self::Etapa { id } | Self::Decisao { id } => Some(id),
            Self::Fim { .. } => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "tipo", rename_all = "snake_case")]
pub enum Rule {
    Igual {
        campo: String,
        #[serde(rename = "valor")]
        value: String,
    },
    Diferente {
        campo: String,
        #[serde(rename = "valor")]
        value: String,
    },
    Maior {
        campo: String,
        #[serde(rename = "valor")]
        value: String,
    },
    Menor {
        campo: String,
        #[serde(rename = "valor")]
        value: String,
    },
    Contem {
        campo: String,
        #[serde(rename = "valor")]
        value: String,
    },
    Vazio {
        campo: String,
    },
    E {
        args: Vec<Rule>,
    },
    Ou {
        args: Vec<Rule>,
    },
    Nao {
        args: Vec<Rule>,
    },
    Funcao {
        nome: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "tipo", rename_all = "snake_case")]
pub enum Effect {
    #[serde(rename = "mensagem_ao_cliente")]
    MessageToCustomer {
        texto: String,
    },
    #[serde(rename = "notificar_whatsapp")]
    NotifyWhatsApp {
        para: String,
        mensagem: String,
    },
    Encaminhar {
        departamento: String,
    },
}

impl Ir {
    pub fn fluxo_id(&self) -> &str {
        &self.fluxo.id
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("a IR é serializável por construção")
    }

    pub fn from_json(texto: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(texto)
    }
}
