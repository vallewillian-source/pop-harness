#[derive(Debug, Default)]
pub struct SseReader {
    pending: Vec<u8>,
}

impl SseReader {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, bytes: &[u8]) -> Vec<String> {
        self.pending.extend_from_slice(bytes);
        let mut payloads = Vec::new();
        while let Some(position) = self.pending.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = self.pending.drain(..=position).collect();
            if let Some(payload) = line_payload(&line) {
                payloads.push(payload);
            }
        }
        payloads
    }

    pub fn finish(&mut self) -> Vec<String> {
        let remainder = std::mem::take(&mut self.pending);
        line_payload(&remainder).into_iter().collect()
    }
}

fn line_payload(raw: &[u8]) -> Option<String> {
    let texto = String::from_utf8_lossy(raw);
    let line = texto.trim_end_matches(['\r', '\n']);
    let dados = line.strip_prefix("data:")?;
    Some(dados.trim_start().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delivers_only_complete_lines() {
        let mut reader = SseReader::new();
        assert!(reader.push(b"data: {\"a\"").is_empty());
        assert_eq!(
            reader.push(b":1}\n\ndata: [DONE]\n"),
            vec!["{\"a\":1}", "[DONE]"]
        );
        assert!(reader.finish().is_empty());
    }

    #[test]
    fn utf8_character_split_across_chunks_is_preserved() {
        let texto = "data: dedetização\n";
        let bytes = texto.as_bytes();
        let split = texto.find('ç').expect("tem ç") + 1;
        let mut reader = SseReader::new();
        assert!(reader.push(&bytes[..split]).is_empty());
        assert_eq!(reader.push(&bytes[split..]), vec!["dedetização"]);
    }

    #[test]
    fn skips_non_data_lines_and_accepts_crlf() {
        let mut reader = SseReader::new();
        let input: &[u8] = b": comentario\r\nevent: x\r\ndata: ok\r\n";
        assert_eq!(reader.push(input), vec!["ok"]);
    }

    #[test]
    fn finish_delivers_last_line_without_newline() {
        let mut reader = SseReader::new();
        assert!(reader.push(b"data: fim").is_empty());
        assert_eq!(reader.finish(), vec!["fim"]);
    }
}
