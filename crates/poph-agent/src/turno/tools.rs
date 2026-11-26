use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use poph_core::llm::Tool;
use poph_types::{AgentId, ConversationId, OrgId, ProductId, TurnId};
use serde::Serialize;

pub type ToolFuture = Pin<Box<dyn Future<Output = Result<String, String>> + Send + 'static>>;

#[derive(Debug, Clone)]
pub struct ToolContext {
    pub org: OrgId,
    pub conversa: ConversationId,
    pub agente: AgentId,
    pub turno: TurnId,
    pub usage: Arc<UseTools>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProductUsed {
    #[serde(rename = "produto")]
    pub product: ProductId,
    pub nome: String,
    #[serde(rename = "preco_centavos")]
    pub price_cents: i64,
    #[serde(rename = "preco_texto")]
    pub price_text: String,
    #[serde(rename = "unidade")]
    pub unit: String,
    #[serde(rename = "condicoes")]
    pub conditions: String,
}

#[derive(Debug, Default)]
pub struct UseTools {
    products: Mutex<Vec<ProductUsed>>,
}

impl UseTools {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn register_product(&self, product: ProductUsed) {
        let mut list = self
            .products
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if list
            .iter()
            .any(|current| current.product == product.product)
        {
            return;
        }
        list.push(product);
    }

    pub fn products(&self) -> Vec<ProductUsed> {
        self.products
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    pub fn vazio(&self) -> bool {
        self.products
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .is_empty()
    }
}

pub const CAP_SUMMARY_TOOL: usize = 1024;

pub const MARK_OF_CUTOFF: &str = "…[cortado]";

pub const REASON_NOT_ENABLED: &str = "nao_habilitada";
pub const REASON_UNKNOWN: &str = "desconhecida";
pub const REASON_ARGUMENT_INVALID: &str = "argumento_invalido";
pub const REASON_ERROR_OF_EXECUTION: &str = "erro_de_execucao";
pub const REASON_LIMIT_OF_CALLS: &str = "limite_de_chamadas";
pub const REASON_OUTSIDE_DO_TURN: &str = "fora_do_turno";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Desfecho {
    Executed,
    Refused(&'static str),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CallRecorded {
    pub nome: String,
    #[serde(rename = "rodada")]
    pub round: u32,
    pub ordem: u32,
    pub estado: &'static str,
    pub motivo: Option<&'static str>,
    #[serde(rename = "duracao_ms")]
    pub duration_ms: u64,
    #[serde(rename = "argumentos")]
    pub arguments: String,
    pub resultado: String,
}

#[derive(Debug, Default)]
pub struct RecordCalls {
    calls: Mutex<Vec<CallRecorded>>,
}

impl RecordCalls {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn register(
        &self,
        nome: &str,
        round: u32,
        desfecho: Desfecho,
        duration: Duration,
        arguments: &serde_json::Value,
        resultado: &str,
    ) {
        let mut list = self.calls.lock().unwrap_or_else(|error| error.into_inner());
        let ordem = u32::try_from(list.len())
            .unwrap_or(u32::MAX)
            .saturating_add(1);
        let (estado, motivo) = match desfecho {
            Desfecho::Executed => ("executada", None),
            Desfecho::Refused(motivo) => ("recusada", Some(motivo)),
        };
        list.push(CallRecorded {
            nome: nome.to_owned(),
            round,
            ordem,
            estado,
            motivo,
            duration_ms: u64::try_from(duration.as_millis()).unwrap_or(u64::MAX),
            arguments: summary_of_arguments(arguments),
            resultado: cut(resultado),
        });
    }

    pub fn calls(&self) -> Vec<CallRecorded> {
        self.calls
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    pub fn for_json(&self) -> serde_json::Value {
        serde_json::to_value(self.calls()).unwrap_or_else(|_| serde_json::Value::Array(Vec::new()))
    }

    pub fn vazio(&self) -> bool {
        self.calls
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .is_empty()
    }
}

pub fn record_turn(names: &[String], calls: &RecordCalls) -> String {
    serde_json::json!({
        "nomes": names,
        "chamadas": calls.for_json(),
    })
    .to_string()
}

pub fn summary_of_arguments(arguments: &serde_json::Value) -> String {
    cut(&draft(arguments).to_string())
}

fn draft(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => serde_json::Value::Object(
            map.iter()
                .map(|(chave, value)| {
                    let value = if key_sensitive(chave) {
                        serde_json::Value::String("[redigido]".to_owned())
                    } else {
                        draft(value)
                    };
                    (chave.clone(), value)
                })
                .collect(),
        ),
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.iter().map(draft).collect())
        }
        other => other.clone(),
    }
}

fn key_sensitive(chave: &str) -> bool {
    const SENSIVEIS: &[&str] = &[
        "senha",
        "token",
        "secret",
        "segredo",
        "chave",
        "password",
        "api_key",
        "apikey",
        "authorization",
        "credencial",
    ];
    let lowercase = chave.to_lowercase();
    SENSIVEIS.iter().any(|word| lowercase.contains(word))
}

pub fn cut(texto: &str) -> String {
    if texto.chars().count() <= CAP_SUMMARY_TOOL {
        return texto.to_owned();
    }
    let mut saida: String = texto.chars().take(CAP_SUMMARY_TOOL).collect();
    saida.push_str(MARK_OF_CUTOFF);
    saida
}

pub trait InternalTool: Send + Sync {
    fn nome(&self) -> &'static str;
    fn descricao(&self) -> &'static str;
    fn parameters(&self) -> serde_json::Value;
    fn run(&self, context: ToolContext, arguments: serde_json::Value) -> ToolFuture;
}

pub trait ToolRegistry: Send + Sync {
    fn tool(&self, nome: &str) -> Option<Arc<dyn InternalTool>>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct NoTools;

impl ToolRegistry for NoTools {
    fn tool(&self, _name: &str) -> Option<Arc<dyn InternalTool>> {
        None
    }
}

#[derive(Default, Clone)]
pub struct DefaultRegistry {
    tools: HashMap<String, Arc<dyn InternalTool>>,
}

impl DefaultRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, tool: Arc<dyn InternalTool>) {
        self.tools.insert(tool.nome().to_owned(), tool);
    }

    pub fn com(mut self, tool: Arc<dyn InternalTool>) -> Self {
        self.register(tool);
        self
    }

    pub fn names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.tools.keys().cloned().collect();
        names.sort();
        names
    }
}

impl ToolRegistry for DefaultRegistry {
    fn tool(&self, nome: &str) -> Option<Arc<dyn InternalTool>> {
        self.tools.get(nome).cloned()
    }
}

pub fn offered(registro: &dyn ToolRegistry, enabled: &[String]) -> Vec<Tool> {
    enabled
        .iter()
        .filter_map(|nome| registro.tool(nome))
        .map(|tool| Tool {
            nome: tool.nome().to_owned(),
            descricao: tool.descricao().to_owned(),
            parameters: tool.parameters(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Sempre;

    impl InternalTool for Sempre {
        fn nome(&self) -> &'static str {
            "consultar_base"
        }

        fn descricao(&self) -> &'static str {
            "consulta a base"
        }

        fn parameters(&self) -> serde_json::Value {
            serde_json::json!({"type": "object"})
        }

        fn run(&self, _context: ToolContext, _arguments: serde_json::Value) -> ToolFuture {
            Box::pin(async { Ok("ok".to_owned()) })
        }
    }

    struct Record;

    impl ToolRegistry for Record {
        fn tool(&self, nome: &str) -> Option<Arc<dyn InternalTool>> {
            (nome == "consultar_base").then(|| Arc::new(Sempre) as Arc<dyn InternalTool>)
        }
    }

    #[test]
    fn without_record_not_oferece_nothing_same_enabled() {
        let list = offered(&NoTools, &["consultar_base".to_owned()]);
        assert!(list.is_empty());
    }

    #[test]
    fn oferece_so_o_that_this_enabled_exists() {
        let registro = Record;
        let list = offered(
            &registro,
            &[
                "consultar_base".to_owned(),
                "buscar_produto".to_owned(),
                "transferir".to_owned(),
            ],
        );
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].nome, "consultar_base");
    }

    #[test]
    fn record_guard_execution_with_order_round() {
        let registro = RecordCalls::new();
        registro.register(
            "consultar_base",
            0,
            Desfecho::Executed,
            Duration::from_millis(7),
            &serde_json::json!({"consulta": "prazo"}),
            "Prazo de 5 dias úteis.",
        );
        let calls = registro.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].nome, "consultar_base");
        assert_eq!(calls[0].round, 0);
        assert_eq!(calls[0].ordem, 1);
        assert_eq!(calls[0].estado, "executada");
        assert_eq!(calls[0].motivo, None);
        assert_eq!(calls[0].duration_ms, 7);
        assert!(calls[0].arguments.contains("prazo"));
        assert_eq!(calls[0].resultado, "Prazo de 5 dias úteis.");
    }

    #[test]
    fn record_guard_refusal_with_reason() {
        let registro = RecordCalls::new();
        registro.register(
            "cart_add_item",
            1,
            Desfecho::Refused(REASON_NOT_ENABLED),
            Duration::ZERO,
            &serde_json::Value::Null,
            "erro: a ferramenta `cart_add_item` não está habilitada para este agente",
        );
        let calls = registro.calls();
        assert_eq!(calls[0].estado, "recusada");
        assert_eq!(calls[0].motivo, Some(REASON_NOT_ENABLED));
        assert_eq!(calls[0].duration_ms, 0);
    }

    #[test]
    fn argument_with_name_of_secret_draft() {
        let summary = summary_of_arguments(&serde_json::json!({
            "termo": "baratas",
            "token": "segredo-abc",
            "api_key": "chave-xyz",
            "password": "senha-123",
            "aninhado": {"chave_de_acesso": "outro-segredo", "ok": "visivel"},
        }));
        assert!(summary.contains("baratas"));
        assert!(summary.contains("visivel"));
        assert!(summary.contains("[redigido]"));
        for forbidden in ["segredo-abc", "chave-xyz", "senha-123", "outro-segredo"] {
            assert!(
                !summary.contains(forbidden),
                "vazou `{forbidden}`: {summary}"
            );
        }
    }

    #[test]
    fn summary_cut_cap_with_mark() {
        let large = "a".repeat(CAP_SUMMARY_TOOL * 3);
        let summary = cut(&large);
        assert_eq!(
            summary.chars().count(),
            CAP_SUMMARY_TOOL + MARK_OF_CUTOFF.chars().count()
        );
        assert!(summary.ends_with(MARK_OF_CUTOFF));
        assert_eq!(cut("curto"), "curto");
    }

    #[test]
    fn record_turn_traz_names_calls() {
        let registro = RecordCalls::new();
        registro.register(
            "consultar_base",
            0,
            Desfecho::Executed,
            Duration::ZERO,
            &serde_json::json!({}),
            "ok",
        );
        let json: serde_json::Value =
            serde_json::from_str(&record_turn(&["consultar_base".to_owned()], &registro))
                .expect("JSON do registro");
        assert_eq!(json["nomes"], serde_json::json!(["consultar_base"]));
        assert_eq!(json["chamadas"].as_array().expect("lista").len(), 1);
    }
}
