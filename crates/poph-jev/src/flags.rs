//! Flags do Jev por agente (`agents.jev_json`), sobre a configuração global `[jev].gates`.
//!
//! Regra (PRD §10): cada gate tem uma flag **global** e uma **por agente**. O agente só ajusta o
//! que declarar; o que ele não declarar herda a global. `{"ativo": false}` desliga todos os gates
//! do agente de uma vez (é o que o Console usa como interruptor).
//!
//! Formatos aceitos (JSON no `--jev-json` da CLI / Console):
//! ```json
//! {"ativo": true, "gates": {"g1": true, "g4": false}}
//! {"g1": true}
//! ```
//! Campo desconhecido é ignorado: o JSON é de operador e não pode derrubar o turno.

use poph_types::GatesJev;

use crate::tipos::IdGate;
use serde_json::Value;

/// Ajuste de gates de um agente: cada campo `None` herda a configuração global.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AjusteGates {
    /// Interruptor geral do agente (`false` desliga todos).
    pub ativo: Option<bool>,
    /// G1.
    pub g1: Option<bool>,
    /// G2.
    pub g2: Option<bool>,
    /// G3.
    pub g3: Option<bool>,
    /// G4.
    pub g4: Option<bool>,
    /// G5.
    pub g5: Option<bool>,
    /// G6.
    pub g6: Option<bool>,
    /// G7.
    pub g7: Option<bool>,
    /// G8.
    pub g8: Option<bool>,
}

impl AjusteGates {
    /// `true` quando o agente não ajustou nada (herda tudo).
    pub fn vazio(&self) -> bool {
        *self == Self::default()
    }

    /// Ajuste que liga **só** o gate dado (o resto fica desligado, inclusive pela global).
    ///
    /// É como o turno pede uma pergunta fora do lote (o G5, cujos candidatos só existem depois da
    /// busca de produtos) e como o `poph jev eval` mede um gate isolado.
    pub fn apenas(gate: IdGate) -> Self {
        let mut ajuste = Self {
            ativo: Some(true),
            g1: Some(false),
            g2: Some(false),
            g3: Some(false),
            g4: Some(false),
            g5: Some(false),
            g6: Some(false),
            g7: Some(false),
            g8: Some(false),
        };
        match gate {
            IdGate::G1 => ajuste.g1 = Some(true),
            IdGate::G2 => ajuste.g2 = Some(true),
            IdGate::G3 => ajuste.g3 = Some(true),
            IdGate::G4 => ajuste.g4 = Some(true),
            IdGate::G5 => ajuste.g5 = Some(true),
            IdGate::G6 => ajuste.g6 = Some(true),
            IdGate::G7 => ajuste.g7 = Some(true),
            IdGate::G8 => ajuste.g8 = Some(true),
        }
        ajuste
    }

    /// Lê o ajuste do JSON do agente. `None` = nada declarado (herda a configuração global).
    pub fn de_json(texto: &str) -> Option<Self> {
        let valor: Value = serde_json::from_str(texto).ok()?;
        let objeto = valor.as_object()?;
        let mut ajuste = Self {
            ativo: objeto.get("ativo").and_then(Value::as_bool),
            ..Self::default()
        };
        // Campos na raiz e dentro de `gates`: os dois formatos valem.
        let aninhado = objeto.get("gates").and_then(Value::as_object);
        for (campo, valor) in objeto {
            atribuir(&mut ajuste, campo, valor.as_bool());
        }
        if let Some(gates) = aninhado {
            for (campo, valor) in gates {
                atribuir(&mut ajuste, campo, valor.as_bool());
            }
        }
        if ajuste.vazio() { None } else { Some(ajuste) }
    }

    /// Aplica o ajuste sobre a configuração global, devolvendo os gates efetivos.
    pub fn sobre(&self, base: GatesJev) -> GatesJev {
        if self.ativo == Some(false) {
            return GatesJev::default();
        }
        GatesJev {
            g1: self.g1.unwrap_or(base.g1),
            g2: self.g2.unwrap_or(base.g2),
            g3: self.g3.unwrap_or(base.g3),
            g4: self.g4.unwrap_or(base.g4),
            g5: self.g5.unwrap_or(base.g5),
            g6: self.g6.unwrap_or(base.g6),
            g7: self.g7.unwrap_or(base.g7),
            g8: self.g8.unwrap_or(base.g8),
        }
    }
}

fn atribuir(ajuste: &mut AjusteGates, campo: &str, valor: Option<bool>) {
    let Some(valor) = valor else { return };
    match campo {
        "g1" => ajuste.g1 = Some(valor),
        "g2" => ajuste.g2 = Some(valor),
        "g3" => ajuste.g3 = Some(valor),
        "g4" => ajuste.g4 = Some(valor),
        "g5" => ajuste.g5 = Some(valor),
        "g6" => ajuste.g6 = Some(valor),
        "g7" => ajuste.g7 = Some(valor),
        "g8" => ajuste.g8 = Some(valor),
        _ => {}
    }
}

/// Lê o ajuste de gates do agente a partir do `jev_json` gravado.
pub fn do_agente(jev_json: &str) -> Option<AjusteGates> {
    let texto = jev_json.trim();
    if texto.is_empty() || texto == "{}" {
        return None;
    }
    AjusteGates::de_json(texto)
}

/// Situações de transferência declaradas no `jev_json` do agente (`situacoes_de_handoff`).
///
/// Enquanto a HR-29 não dá um campo próprio para a política de transferência, ela vive no JSON do
/// agente: `{"situacoes_de_handoff": ["cliente pede segunda via", ...]}`. JSON inválido ou lista
/// ausente devolve lista vazia — o G1 então aplica a política padrão da instrução.
pub fn situacoes_do_agente(jev_json: &str) -> Vec<String> {
    let Ok(valor) = serde_json::from_str::<Value>(jev_json) else {
        return Vec::new();
    };
    valor
        .get("situacoes_de_handoff")
        .and_then(Value::as_array)
        .map(|itens| {
            itens
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// Limite de caracteres da despedida configurada pelo agente.
pub const MAX_CHARS_DESPEDIDA: usize = 500;

/// Despedida do agente quando a conversa é encerrada pelo cliente (G8, HR-36).
///
/// Vem do `jev_json` do agente: `{"despedida": "…"}`. Ausente = `None` (o turno usa o padrão do
/// gate). Texto vazio = `Some("")`: o agente quer encerrar **sem** mandar mensagem. JSON inválido
/// ou campo com outro tipo = `None`, para o JSON de operador nunca derrubar o turno.
pub fn despedida_do_agente(jev_json: &str) -> Option<String> {
    let valor: Value = serde_json::from_str(jev_json).ok()?;
    let texto = valor.get("despedida")?.as_str()?;
    Some(texto.trim().chars().take(MAX_CHARS_DESPEDIDA).collect())
}

/// Limite de caracteres da resposta segura configurada pelo agente.
pub const MAX_CHARS_RESPOSTA_SEGURA: usize = 1000;

/// Resposta segura do agente, usada quando a revisão do G6 reprova a resposta duas vezes (HR-34).
///
/// Vem do `jev_json` do agente: `{"resposta_segura": "…"}`. Ausente, vazia ou de outro tipo =
/// `None`, e o turno envia a resposta regenerada — nunca deixa o cliente sem resposta. O texto é
/// truncado em [`MAX_CHARS_RESPOSTA_SEGURA`] e o JSON inválido é ignorado, como no `despedida`.
pub fn resposta_segura_do_agente(jev_json: &str) -> Option<String> {
    let valor: Value = serde_json::from_str(jev_json).ok()?;
    let texto = valor.get("resposta_segura")?.as_str()?.trim();
    if texto.is_empty() {
        return None;
    }
    Some(texto.chars().take(MAX_CHARS_RESPOSTA_SEGURA).collect())
}

/// Ação do G7 quando a mensagem (ou o texto do arquivo) é classificada como injeção ou abuso.
///
/// Vem do `jev_json` do agente: `{"g7_acao": "registrar" | "responder_padrao" | "transferir"}`.
/// Ausente, desconhecida ou de outro tipo = [`AcaoG7::Registrar`], que é o comportamento de hoje
/// (a classificação é registrada e a resposta segue para o modelo).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AcaoG7 {
    /// Só registra a decisão; a resposta do turno não muda.
    #[default]
    Registrar,
    /// Responde o texto neutro configurado, sem chamar o modelo de texto neste turno.
    ResponderPadrao,
    /// Transfere para humano pelo mesmo caminho do G1.
    Transferir,
}

impl AcaoG7 {
    /// Texto usado no registro do turno.
    pub fn como_texto(self) -> &'static str {
        match self {
            AcaoG7::Registrar => "registrar",
            AcaoG7::ResponderPadrao => "responder_padrao",
            AcaoG7::Transferir => "transferir",
        }
    }
}

/// Ação configurada para o G7 no `jev_json` do agente.
///
/// JSON inválido, campo ausente ou valor desconhecido devolvem o padrão: o JSON é de operador e não
/// pode derrubar o turno.
pub fn acao_g7_do_agente(jev_json: &str) -> AcaoG7 {
    let Ok(valor) = serde_json::from_str::<Value>(jev_json) else {
        return AcaoG7::Registrar;
    };
    match valor.get("g7_acao").and_then(Value::as_str) {
        Some("responder_padrao") => AcaoG7::ResponderPadrao,
        Some("transferir") => AcaoG7::Transferir,
        _ => AcaoG7::Registrar,
    }
}

/// Limite de caracteres da resposta neutra do G7 configurada pelo agente.
pub const MAX_CHARS_RESPOSTA_G7: usize = 1000;

/// Resposta neutra do G7, usada quando a ação é `responder_padrao`.
///
/// Vem do `jev_json` do agente: `{"g7_resposta": "…"}`. Ausente, vazia ou de outro tipo devolve
/// `None`, e o turno usa [`crate::gates::G7_RESPOSTA_PADRAO`] — nunca deixa o cliente sem
/// resposta.
pub fn resposta_g7_do_agente(jev_json: &str) -> Option<String> {
    let valor: Value = serde_json::from_str(jev_json).ok()?;
    let texto = valor.get("g7_resposta")?.as_str()?.trim();
    if texto.is_empty() {
        return None;
    }
    Some(texto.chars().take(MAX_CHARS_RESPOSTA_G7).collect())
}
