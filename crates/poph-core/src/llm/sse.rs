//! Leitor de linhas `data:` do stream SSE (formato de `text/event-stream`).
//!
//! Os bytes chegam em pedaços que podem cortar uma linha, ou um caractere UTF-8, no meio.
//! O leitor guarda o que sobra e só entrega linhas completas.

/// Acumula bytes e devolve os payloads `data:` de cada linha completa.
#[derive(Debug, Default)]
pub struct SseReader {
    pendente: Vec<u8>,
}

impl SseReader {
    /// Novo leitor, sem bytes pendentes.
    pub fn new() -> Self {
        Self::default()
    }

    /// Recebe o próximo pedaço e devolve os payloads completos que ele fechou, na ordem.
    pub fn push(&mut self, bytes: &[u8]) -> Vec<String> {
        self.pendente.extend_from_slice(bytes);
        let mut payloads = Vec::new();
        while let Some(posicao) = self.pendente.iter().position(|b| *b == b'\n') {
            let linha: Vec<u8> = self.pendente.drain(..=posicao).collect();
            if let Some(payload) = line_payload(&linha) {
                payloads.push(payload);
            }
        }
        payloads
    }

    /// Fim do corpo: entrega o que ficou numa última linha sem `\n`, se for um `data:`.
    pub fn encerrar(&mut self) -> Vec<String> {
        let resto = std::mem::take(&mut self.pendente);
        line_payload(&resto).into_iter().collect()
    }
}

fn line_payload(bruta: &[u8]) -> Option<String> {
    let texto = String::from_utf8_lossy(bruta);
    let linha = texto.trim_end_matches(['\r', '\n']);
    let dados = linha.strip_prefix("data:")?;
    Some(dados.trim_start().to_owned())
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn delivers_only_complete_lines() {
        let mut leitor = SseReader::new();
        assert!(leitor.push(b"data: {\"a\"").is_empty());
        assert_eq!(
            leitor.push(b":1}\n\ndata: [DONE]\n"),
            vec!["{\"a\":1}", "[DONE]"]
        );
        assert!(leitor.encerrar().is_empty());
    }

    #[test]
    fn utf8_character_split_across_chunks_is_preserved() {
        let texto = "data: dedetização\n";
        let bytes = texto.as_bytes();
        // Corta no meio do "ç" (dois bytes em UTF-8).
        let corte = texto.find('ç').expect("tem ç") + 1;
        let mut leitor = SseReader::new();
        assert!(leitor.push(&bytes[..corte]).is_empty());
        assert_eq!(leitor.push(&bytes[corte..]), vec!["dedetização"]);
    }

    #[test]
    fn ignora_linhas_que_nao_sao_dados_e_aceita_crlf() {
        let mut leitor = SseReader::new();
        let entrada: &[u8] = b": comentario\r\nevent: x\r\ndata: ok\r\n";
        assert_eq!(leitor.push(entrada), vec!["ok"]);
    }

    #[test]
    fn finish_delivers_last_line_without_newline() {
        let mut leitor = SseReader::new();
        assert!(leitor.push(b"data: fim").is_empty());
        assert_eq!(leitor.encerrar(), vec!["fim"]);
    }
}
