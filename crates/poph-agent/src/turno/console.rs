//! Transporte "console": entrega a resposta na saída do processo (PRD §6, passo 8).
//!
//! É o canal interno usado pelo `poph chat` e pelos testes ponta a ponta. O canal real (Evolution,
//! plugin) entra na fase 4 com o mesmo [`TransporteSaida`](crate::entrada_saida::TransporteSaida).
//!
//! `poph chat` imprime o **stream** de cada pedaço de texto conforme ele chega; quando confirma o
//! item da `outbox`, usa o modo [silencioso](TransporteConsole::silencioso) para não imprimir o texto
//! duas vezes. Quem não faz streaming (servidor, teste, plugin) usa
//! [voz alta](TransporteConsole::voz_alta) e imprime a resposta inteira de uma vez.

use std::io::Write;
use std::sync::{Arc, Mutex};

use serde_json::Value;

use poph_store::ItemSaida;

use crate::entrada_saida::{FalhaEnvio, FuturoEnvio, TransporteSaida};

/// Transporte que escreve o texto dos itens na saída padrão.
#[derive(Clone)]
pub struct TransporteConsole {
    imprimir: bool,
    saida: Arc<Mutex<Box<dyn Write + Send>>>,
}

impl TransporteConsole {
    /// Imprime o texto do item na saída padrão (quem não faz streaming ao vivo).
    pub fn voz_alta() -> Self {
        Self {
            imprimir: true,
            saida: Arc::new(Mutex::new(Box::new(std::io::stdout()))),
        }
    }

    /// Não imprime nada: o texto já saiu no stream. Só confirma a entrega ao fluxo.
    pub fn silencioso() -> Self {
        Self {
            imprimir: false,
            saida: Arc::new(Mutex::new(Box::new(std::io::sink()))),
        }
    }

    /// Transporte que escreve num destino qualquer (teste, arquivo de sessão).
    pub fn para_saida(imprimir: bool, saida: Box<dyn Write + Send>) -> Self {
        Self {
            imprimir,
            saida: Arc::new(Mutex::new(saida)),
        }
    }
}

impl TransporteSaida for TransporteConsole {
    fn enviar(&self, item: ItemSaida) -> FuturoEnvio {
        // Item de saída que não é texto (arquivo, presença) não é trabalho deste transporte nesta
        // fase: confirmar sem imprimir evita repetir o envio para sempre.
        if self.imprimir
            && item.tipo == "text"
            && let Some(texto) = texto_do_item(&item.conteudo_json)
        {
            let mut saida = self
                .saida
                .lock()
                .unwrap_or_else(|veneno| veneno.into_inner());
            if writeln!(saida, "{texto}").is_err() {
                return Box::pin(async { Err(FalhaEnvio::transitoria("saída padrão fechada")) });
            }
            let _ = saida.flush();
        }
        Box::pin(async { Ok(()) })
    }
}

/// Lê o texto do `conteudo_json` de um item de saída (`{"texto": "..."}`, o contrato da fase 1).
pub fn texto_do_item(conteudo_json: &str) -> Option<String> {
    let valor: Value = serde_json::from_str(conteudo_json).ok()?;
    if let Some(texto) = valor.get("texto").and_then(Value::as_str) {
        return Some(texto.to_owned());
    }
    // Aceita também o nome em inglês, para o dia em que a API (HR-21) usar `text`.
    valor.get("text").and_then(Value::as_str).map(str::to_owned)
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn le_o_texto_nos_dois_formatos() {
        assert_eq!(texto_do_item(r#"{"texto":"oi"}"#).as_deref(), Some("oi"));
        assert_eq!(texto_do_item(r#"{"text":"hi"}"#).as_deref(), Some("hi"));
        assert_eq!(texto_do_item(r#"{"outro":1}"#), None);
        assert_eq!(texto_do_item("não é json"), None);
    }
}
