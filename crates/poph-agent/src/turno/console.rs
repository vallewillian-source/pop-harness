use std::io::Write;
use std::sync::{Arc, Mutex};

use serde_json::Value;

use poph_store::OutboxItem;

use crate::entry_output::{OutputTransport, SendFailure, SendFuture};

#[derive(Clone)]
pub struct TransportConsole {
    print: bool,
    saida: Arc<Mutex<Box<dyn Write + Send>>>,
}

impl TransportConsole {
    pub fn voz_alta() -> Self {
        Self {
            print: true,
            saida: Arc::new(Mutex::new(Box::new(std::io::stdout()))),
        }
    }

    pub fn silencioso() -> Self {
        Self {
            print: false,
            saida: Arc::new(Mutex::new(Box::new(std::io::sink()))),
        }
    }

    pub fn for_output(print: bool, saida: Box<dyn Write + Send>) -> Self {
        Self {
            print,
            saida: Arc::new(Mutex::new(saida)),
        }
    }
}

impl OutputTransport for TransportConsole {
    fn send(&self, item: OutboxItem) -> SendFuture {
        if self.print
            && item.tipo == "text"
            && let Some(texto) = text_item(&item.content_json)
        {
            let mut saida = self
                .saida
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            if writeln!(saida, "{texto}").is_err() {
                return Box::pin(async { Err(SendFailure::transient("saída padrão fechada")) });
            }
            let _ = saida.flush();
        }
        Box::pin(async { Ok(()) })
    }
}

pub fn text_item(content_json: &str) -> Option<String> {
    let value: Value = serde_json::from_str(content_json).ok()?;
    if let Some(texto) = value.get("texto").and_then(Value::as_str) {
        return Some(texto.to_owned());
    }
    value.get("text").and_then(Value::as_str).map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn le_o_text_us_two_format() {
        assert_eq!(text_item(r#"{"texto":"oi"}"#).as_deref(), Some("oi"));
        assert_eq!(text_item(r#"{"text":"hi"}"#).as_deref(), Some("hi"));
        assert_eq!(text_item(r#"{"outro":1}"#), None);
        assert_eq!(text_item("não é json"), None);
    }
}
