//! Leitor de linhas `data:` do stream SSE (formato de `text/event-stream`).
//!
//! Os bytes chegam em pedaços que podem cortar uma linha, ou um caractere UTF-8, no meio.
//! O leitor guarda o que sobra e só entrega linhas completas.

/// Acumula bytes e devolve os payloads `data:` de cada linha completa.
#[derive(Debug, Default)]
pub struct LeitorSse {
    pendente: Vec<u8>,
}

impl LeitorSse {
    /// Novo leitor, sem bytes pendentes.
    pub fn new() -> Self {
        Self::default()
    }

    /// Recebe o próximo pedaço e devolve os payloads completos que ele fechou, na ordem.
    pub fn empurrar(&mut self, bytes: &[u8]) -> Vec<String> {
        self.pendente.extend_from_slice(bytes);
        let mut payloads = Vec::new();
        while let Some(posicao) = self.pendente.iter().position(|b| *b == b'\n') {
            let linha: Vec<u8> = self.pendente.drain(..=posicao).collect();
            if let Some(payload) = payload_da_linha(&linha) {
                payloads.push(payload);
            }
        }
        payloads
    }

    /// Fim do corpo: entrega o que ficou numa última linha sem `\n`, se for um `data:`.
    pub fn encerrar(&mut self) -> Vec<String> {
        let resto = std::mem::take(&mut self.pendente);
        payload_da_linha(&resto).into_iter().collect()
    }
}

fn payload_da_linha(bruta: &[u8]) -> Option<String> {
    let texto = String::from_utf8_lossy(bruta);
    let linha = texto.trim_end_matches(['\r', '\n']);
    let dados = linha.strip_prefix("data:")?;
    Some(dados.trim_start().to_owned())
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn entrega_so_linhas_completas() {
        let mut leitor = LeitorSse::new();
        assert!(leitor.empurrar(b"data: {\"a\"").is_empty());
        assert_eq!(
            leitor.empurrar(b":1}\n\ndata: [DONE]\n"),
            vec!["{\"a\":1}", "[DONE]"]
        );
        assert!(leitor.encerrar().is_empty());
    }

    #[test]
    fn caractere_utf8_cortado_entre_pedacos_nao_vira_lixo() {
        let texto = "data: dedetização\n";
        let bytes = texto.as_bytes();
        // Corta no meio do "ç" (dois bytes em UTF-8).
        let corte = texto.find('ç').expect("tem ç") + 1;
        let mut leitor = LeitorSse::new();
        assert!(leitor.empurrar(&bytes[..corte]).is_empty());
        assert_eq!(leitor.empurrar(&bytes[corte..]), vec!["dedetização"]);
    }

    #[test]
    fn ignora_linhas_que_nao_sao_dados_e_aceita_crlf() {
        let mut leitor = LeitorSse::new();
        let entrada: &[u8] = b": comentario\r\nevent: x\r\ndata: ok\r\n";
        assert_eq!(leitor.empurrar(entrada), vec!["ok"]);
    }

    #[test]
    fn encerrar_entrega_a_ultima_linha_sem_quebra() {
        let mut leitor = LeitorSse::new();
        assert!(leitor.empurrar(b"data: fim").is_empty());
        assert_eq!(leitor.encerrar(), vec!["fim"]);
    }
}
