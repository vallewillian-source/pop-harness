//! Segunda pergunta do G1: orientação mínima ao cliente antes da transferência (HR-29).
//!
//! Porta `resolve_handoff_guidance` de `core/jev/human_handoff.py` (`HUMAN_HANDOFF_GUIDANCE_PRD.md`
//! §5). Decidida a transferência, **uma** pergunta de escolha classifica se a conversa traz risco
//! imediato à saúde e de que tipo — e o tipo escolhe o texto fixo que o cliente lê antes de esperar
//! o humano. Nenhum modelo escreve esse texto: ele vem da configuração
//! (`HandoffConfig`, `HUMAN_HANDOFF_GUIDANCE_PRD.md` §5.3, aprovado em 19/11/2026).
//!
//! Três regras que valem para tudo aqui:
//! - **a transferência nunca é segurada pela orientação**: erro, prazo, escolha inesperada ou
//!   confiança abaixo do limiar só suprimem o texto — a transferência e a mensagem automática
//!   seguem;
//! - **`nenhum` não orienta**: só os três tipos de risco têm texto;
//! - **texto vazio conta como vazio**: tipo sem texto configurado não orienta (§5.3).
//!
//! A pergunta não entra no lote único do turno de propósito: ela só existe quando a transferência
//! já foi decidida, e mandá-la em todo turno pagaria tokens por uma pergunta que quase nunca muda a
//! resposta. É o mesmo desenho do POP Atende (segunda chamada, depois da decisão).

use std::collections::BTreeMap;
use std::time::Duration;

use poph_types::HandoffConfig;

use crate::types::Question;

/// Id da pergunta de risco no pedido ao Jev.
pub const RISCO_PERGUNTA_ID: &str = "risco_imediato";
/// Não há risco imediato à saúde (a conversa transfere sem orientação).
pub const RISCO_NENHUM: &str = "nenhum";
/// Alguém engoliu, inalou ou tocou em produto químico/veneno/isca.
pub const RISCO_INGESTAO: &str = "ingestao_contato_produto";
/// Animal peçonhento no local ou picou alguém.
pub const RISCO_ANIMAL: &str = "animal_peconhento";
/// Outro risco imediato à saúde (falta de ar, convulsão, dor no peito…).
pub const RISCO_OUTRO: &str = "outro_risco_saude";

/// Os três tipos de risco que orientam o cliente, na ordem do PRD.
pub const TIPOS_DE_RISCO: [&str; 3] = [RISCO_INGESTAO, RISCO_ANIMAL, RISCO_OUTRO];

/// Prazo padrão da segunda pergunta (o POP Atende usa 4 s).
pub const PRAZO_PADRAO: Duration = Duration::from_secs(4);

const INSTRUCOES: &str = "A conversa JÁ foi transferida para um atendente humano. Classifique, \
pela ÚLTIMA mensagem do cliente lida no contexto anterior, se existe RISCO IMEDIATO à saúde de \
alguém e de que tipo: a escolha define a orientação de primeiros cuidados que o cliente recebe \
enquanto espera. Julgue o que o cliente DESCREVE, não o que ele pede. Pedido de pessoa, \
reclamação, cobrança, orçamento e dúvida comum não são risco. Na ausência de sinal claro de risco \
à saúde, escolha 'nenhum'.";

/// Texto da orientação: o que o cliente lê antes da mensagem automática.
///
/// É **dado de configuração** (`[handoff]` do `poph.toml`), com os padrões aprovados em §5.3.
#[derive(Debug, Clone, PartialEq)]
pub struct Orientacao {
    /// Faz a pergunta de risco. Desligada, toda transferência sai só com a mensagem automática.
    pub ativo: bool,
    /// Confiança mínima para aceitar o tipo de risco.
    pub confianca: f64,
    /// Prazo só desta pergunta.
    pub prazo: Duration,
    /// Mensagem automática da transferência (sempre sai).
    pub mensagem_automatica: String,
    /// Texto fixo por tipo de risco (`ingestao_contato_produto`, `animal_peconhento`,
    /// `outro_risco_saude`). Ausente ou vazio = o tipo não orienta.
    pub textos: BTreeMap<String, String>,
}

impl Default for Orientacao {
    fn default() -> Self {
        // Os padrões vivem em `HandoffConfig` (uma fonte só para o arquivo e para o código).
        Self::da_config(&HandoffConfig::default())
    }
}

impl Orientacao {
    /// Monta a orientação a partir do bloco `[handoff]` da configuração.
    pub fn da_config(config: &HandoffConfig) -> Self {
        let mut textos = BTreeMap::new();
        textos.insert(
            RISCO_INGESTAO.to_owned(),
            config.guidance_product_ingestion.clone(),
        );
        textos.insert(
            RISCO_ANIMAL.to_owned(),
            config.guidance_venomous_animal.clone(),
        );
        textos.insert(
            RISCO_OUTRO.to_owned(),
            config.guidance_other_health_risk.clone(),
        );
        Self {
            ativo: config.guidance_enabled,
            confianca: config.guidance_confidence.clamp(0.0, 1.0),
            prazo: Duration::from_secs(u64::from(config.guidance_timeout_seconds.max(1))),
            mensagem_automatica: config.automatic_message.clone(),
            textos,
        }
    }

    /// Troca o prazo da pergunta (teste e configuração programática).
    pub fn com_prazo(mut self, prazo: Duration) -> Self {
        self.prazo = prazo;
        self
    }

    /// Troca o texto de um tipo (teste: texto vazio, texto próprio).
    pub fn com_texto(mut self, tipo: &str, texto: impl Into<String>) -> Self {
        self.textos.insert(tipo.to_owned(), texto.into());
        self
    }

    /// Texto configurado para o tipo (`""` quando não há).
    pub fn texto(&self, tipo: &str) -> &str {
        self.textos.get(tipo).map(String::as_str).unwrap_or("")
    }

    /// A pergunta que vai ao Jev (uma chamada só, depois da decisão de transferir).
    pub fn pergunta(&self) -> Question {
        pergunta_risco()
    }

    /// Tipo de risco que orienta o cliente, ou `None` quando não há orientação.
    ///
    /// `None` em todos os casos de fail-safe do PRD §5.4: orientação desligada, pergunta não feita
    /// ou falha, `nenhum`, escolha fora do contrato, confiança abaixo do limiar e texto vazio.
    pub fn tipo_confiavel(&self, resultado: &ResultadoOrientacao) -> Option<String> {
        if !self.ativo || !resultado.consultado || resultado.falha.is_some() {
            return None;
        }
        let tipo = resultado.tipo.as_deref()?;
        if !TIPOS_DE_RISCO.contains(&tipo) {
            return None;
        }
        if resultado.confianca < self.confianca {
            return None;
        }
        if self.texto(tipo).trim().is_empty() {
            return None;
        }
        Some(tipo.to_owned())
    }

    /// Mensagem única ao cliente: orientação + linha em branco + mensagem automática.
    ///
    /// Sem orientação (`None`, tipo desconhecido ou texto vazio) devolve a mensagem automática
    /// sozinha, byte a byte — é o comportamento do POP Atende (`compose_handoff_message`).
    pub fn compor(&self, tipo: Option<&str>) -> String {
        let automatica = self.mensagem_automatica.clone();
        match tipo {
            Some(tipo) if !self.texto(tipo).trim().is_empty() => {
                format!("{}\n\n{automatica}", self.texto(tipo))
            }
            _ => automatica,
        }
    }
}

/// A pergunta de risco, pronta para o pedido ao Jev.
pub fn pergunta_risco() -> Question {
    Question::choice(INSTRUCOES, criterios())
}

/// Critério de cada opção, como o POP Atende escreve (`_GUIDANCE_CRITERIA`).
fn criterios() -> BTreeMap<String, String> {
    let mut criterios = BTreeMap::new();
    criterios.insert(
        RISCO_NENHUM.to_owned(),
        "A conversa não indica risco imediato à saúde de ninguém: pedido de atendente/pessoa, \
         reclamação, cobrança, orçamento, agendamento, dúvida comum, problema com serviço já \
         feito. Falar de produto, praga, veneno ou isca SEM pessoa exposta ou animal peçonhento \
         presente também é 'nenhum'."
            .to_owned(),
    );
    criterios.insert(
        RISCO_INGESTAO.to_owned(),
        "Alguém engoliu, inalou, tocou ou de outra forma teve contato com produto químico, veneno, \
         isca ou inseticida — criança, adulto ou animal de estimação. Sintomas depois da exposição \
         (vômito, ardência, mal-estar) também entram aqui."
            .to_owned(),
    );
    criterios.insert(
        RISCO_ANIMAL.to_owned(),
        "Escorpião, aranha, cobra, abelha/marimbondo ou outro animal peçonhento está presente no \
         local ou picou alguém. Vale mesmo que ninguém tenha sido picado ainda (animal no berço, \
         no quintal, dentro de casa)."
            .to_owned(),
    );
    criterios.insert(
        RISCO_OUTRO.to_owned(),
        "Risco imediato à saúde ou à vida que não é ingestão/contato com produto nem animal \
         peçonhento: falta de ar, convulsão, desmaio, dor no peito, sangramento, queimadura, \
         reação alérgica grave, mal-estar súbito."
            .to_owned(),
    );
    criterios
}

/// O que a pergunta de risco devolveu (ou por que não foi feita).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ResultadoOrientacao {
    /// `true` quando a pergunta foi efetivamente enviada ao Jev.
    pub consultado: bool,
    /// Escolha crua do Jev (`nenhum`, um dos três tipos ou algo fora do contrato).
    pub tipo: Option<String>,
    /// Confiança declarada (0 quando ausente).
    pub confianca: f64,
    /// Duração da chamada, em ms.
    pub ms: u64,
    /// Tokens consumidos.
    pub tokens: u64,
    /// Motivo da falha (prazo, erro do provedor), quando houve. Sempre fail-safe.
    pub falha: Option<String>,
}

impl ResultadoOrientacao {
    /// A pergunta não foi feita (Jev desligado, orientação desligada, avaliador sem transporte).
    pub fn nao_consultado(motivo: impl Into<String>) -> Self {
        Self {
            consultado: false,
            falha: Some(motivo.into()),
            ..Self::default()
        }
    }

    /// `true` quando houve falha na chamada (conta como "sem orientação").
    pub fn falhou(&self) -> bool {
        self.falha.is_some()
    }

    /// Registro em `turns.jev_json` (campo `risco`).
    pub fn para_json(&self) -> serde_json::Value {
        serde_json::json!({
            "consultado": self.consultado,
            "tipo": self.tipo,
            "confianca": self.confianca,
            "ms": self.ms,
            "tokens": self.tokens,
            "falha": self.falha,
        })
    }

    /// Resumo curto para o observador e para o log.
    pub fn resumo(&self) -> String {
        if let Some(falha) = &self.falha {
            return format!("risco não classificado ({falha})");
        }
        format!(
            "risco={} (confiança {:.2})",
            self.tipo.as_deref().unwrap_or("—"),
            self.confianca
        )
    }
}
