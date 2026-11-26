use sha2::{Digest, Sha256};

use crate::embedder::{CHUNK_SIZE_CHARS, ModelIdentity, OVERLAP_CHARS};

pub fn embedding_text(title: &str, chunk: &str) -> String {
    let title = title.trim();
    if title.is_empty() {
        chunk.to_owned()
    } else {
        format!("{title}\n{chunk}")
    }
}

pub fn split(text: &str) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut current: Vec<Unit> = Vec::new();
    for unit in units(text) {
        if !current.is_empty() {
            let fits = combined_length(&current) + 1 + char_count(&unit.text) <= CHUNK_SIZE_CHARS;
            let new_paragraph =
                unit.starts_paragraph && combined_length(&current) >= CHUNK_SIZE_CHARS / 2;
            if !fits || new_paragraph {
                chunks.push(join_units(&current));
                current = if new_paragraph {
                    Vec::new()
                } else {
                    overlap(&current, &unit)
                };
            }
        }
        current.push(unit);
    }
    if !current.is_empty() {
        chunks.push(join_units(&current));
    }
    chunks
}

pub fn source_hash(identity: &ModelIdentity, title: &str, text: &str) -> String {
    let mut hash = Sha256::new();
    let parts = [
        identity.chunker_version,
        identity.name,
        identity.quantization,
        identity.model_sha256.as_str(),
        identity.tokenizer_sha256.as_str(),
        title,
        text,
    ];
    for part in parts {
        hash.update(part.as_bytes());
        hash.update([0u8]);
    }
    hash.finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Unit {
    text: String,
    starts_paragraph: bool,
}

fn char_count(text: &str) -> usize {
    text.chars().count()
}

fn combined_length(units: &[Unit]) -> usize {
    if units.is_empty() {
        return 0;
    }
    units.iter().map(|u| char_count(&u.text)).sum::<usize>() + units.len() - 1
}

fn join_units(units: &[Unit]) -> String {
    let mut text = String::new();
    for (index, unit) in units.iter().enumerate() {
        if index > 0 {
            text.push(if unit.starts_paragraph { '\n' } else { ' ' });
        }
        text.push_str(&unit.text);
    }
    text
}

fn overlap(current: &[Unit], next: &Unit) -> Vec<Unit> {
    let mut keep = 0;
    let mut used = 0;
    for unit in current.iter().rev() {
        let including_this = used + if keep == 0 { 0 } else { 1 } + char_count(&unit.text);
        if including_this > OVERLAP_CHARS
            || including_this + 1 + char_count(&next.text) > CHUNK_SIZE_CHARS
        {
            break;
        }
        used = including_this;
        keep += 1;
    }
    current[current.len() - keep..].to_vec()
}

fn units(text: &str) -> Vec<Unit> {
    let mut paragraphs: Vec<String> = Vec::new();
    let mut current = String::new();
    for line in text.lines() {
        let normalized = line.split_whitespace().collect::<Vec<_>>().join(" ");
        if normalized.is_empty() {
            if !current.is_empty() {
                paragraphs.push(std::mem::take(&mut current));
            }
        } else if current.is_empty() {
            current = normalized;
        } else {
            current.push(' ');
            current.push_str(&normalized);
        }
    }
    if !current.is_empty() {
        paragraphs.push(current);
    }

    let mut output = Vec::new();
    for paragraph in paragraphs {
        let mut first = true;
        for sentence in sentences(&paragraph) {
            for piece in split_long_sentence(&sentence) {
                output.push(Unit {
                    text: piece,
                    starts_paragraph: first,
                });
                first = false;
            }
        }
    }
    output
}

fn sentences(paragraph: &str) -> Vec<String> {
    let chars: Vec<char> = paragraph.chars().collect();
    let mut sentences = Vec::new();
    let mut current = String::new();
    for (index, character) in chars.iter().enumerate() {
        current.push(*character);
        let ends_sentence = chars.get(index + 1).is_none_or(|next| next.is_whitespace());
        if matches!(character, '.' | '!' | '?' | ';') && ends_sentence {
            sentences.push(std::mem::take(&mut current));
        }
    }
    if !current.trim().is_empty() {
        sentences.push(current);
    }
    sentences
        .into_iter()
        .map(|sentence| sentence.trim().to_owned())
        .filter(|sentence| !sentence.is_empty())
        .collect()
}

fn split_long_sentence(sentence: &str) -> Vec<String> {
    if char_count(sentence) <= CHUNK_SIZE_CHARS {
        return vec![sentence.to_owned()];
    }
    let mut pieces = Vec::new();
    let mut current = String::new();
    for word in sentence.split_whitespace() {
        let chars: Vec<char> = word.chars().collect();
        if chars.len() > CHUNK_SIZE_CHARS {
            if !current.is_empty() {
                pieces.push(std::mem::take(&mut current));
            }
            for block in chars.chunks(CHUNK_SIZE_CHARS) {
                let text: String = block.iter().collect();
                if block.len() == CHUNK_SIZE_CHARS {
                    pieces.push(text);
                } else {
                    current = text;
                }
            }
            continue;
        }
        let including_word = if current.is_empty() {
            char_count(word)
        } else {
            char_count(&current) + 1 + char_count(word)
        };
        if including_word > CHUNK_SIZE_CHARS {
            pieces.push(std::mem::replace(&mut current, word.to_owned()));
        } else {
            if !current.is_empty() {
                current.push(' ');
            }
            current.push_str(word);
        }
    }
    if !current.is_empty() {
        pieces.push(current);
    }
    pieces
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sentence(n: usize) -> String {
        format!("Frase número {n} com um pouco de texto para o trecho.")
    }

    #[test]
    fn empty_or_whitespace_text_produces_no_chunks() {
        assert!(split("").is_empty());
        assert!(split("   \n\n \t ").is_empty());
    }

    #[test]
    fn short_text_becomes_one_normalized_chunk() {
        let chunks = split("Primeira   linha.\n\nSegundo parágrafo!");
        assert_eq!(
            chunks,
            vec!["Primeira linha.\nSegundo parágrafo!".to_owned()]
        );
    }

    #[test]
    fn all_text_appears_within_chunk_size_limit() {
        let text = (1..=120).map(sentence).collect::<Vec<_>>().join(" ");
        let chunks = split(&text);
        assert!(chunks.len() > 10, "deveria gerar vários trechos");
        for chunk in &chunks {
            assert!(
                char_count(chunk) <= CHUNK_SIZE_CHARS,
                "trecho grande demais: {chunk}"
            );
        }
        for n in 1..=120 {
            assert!(
                chunks.iter().any(|t| t.contains(&format!("número {n} "))),
                "a frase {n} sumiu"
            );
        }
    }

    #[test]
    fn adjacent_chunks_overlap_within_limit() {
        let text = (1..=60).map(sentence).collect::<Vec<_>>().join(" ");
        let chunks = split(&text);
        for pair in chunks.windows(2) {
            let (before, after) = (&pair[0], &pair[1]);
            let shared = (1..=OVERLAP_CHARS.min(char_count(before))).rev().find(|k| {
                let end: String = before.chars().skip(char_count(before) - k).collect();
                after.starts_with(&end)
            });
            assert!(
                shared.is_some(),
                "trechos vizinhos sem sobreposição: {before:?} / {after:?}"
            );
            assert!(shared.unwrap_or(0) <= OVERLAP_CHARS);
        }
    }

    #[test]
    fn oversized_word_is_split_without_losing_chars() {
        let word = "x".repeat(1000);
        let chunks = split(&word);
        assert!(chunks.iter().all(|t| char_count(t) <= CHUNK_SIZE_CHARS));
        assert_eq!(chunks.concat(), word);
    }

    #[test]
    fn splitting_is_deterministic() {
        let text = (1..=40).map(sentence).collect::<Vec<_>>().join("\n\n");
        assert_eq!(split(&text), split(&text));
    }

    #[test]
    fn hash_changes_with_title_text_or_model() {
        use crate::embedder::{CHUNKER_VERSION, MODEL_NAME};
        let base = ModelIdentity {
            name: MODEL_NAME,
            quantization: "int8",
            dimension: 384,
            token_window: 128,
            model_sha256: "aa".to_owned(),
            tokenizer_sha256: "bb".to_owned(),
            chunker_version: CHUNKER_VERSION,
        };
        let h = source_hash(&base, "Titulo", "Texto");
        assert_eq!(h, source_hash(&base, "Titulo", "Texto"));
        assert_ne!(h, source_hash(&base, "Outro", "Texto"));
        assert_ne!(h, source_hash(&base, "Titulo", "Outro"));
        let other_model = ModelIdentity {
            model_sha256: "cc".to_owned(),
            ..base.clone()
        };
        assert_ne!(h, source_hash(&other_model, "Titulo", "Texto"));
    }

    #[test]
    fn embedding_text_starts_with_title() {
        assert_eq!(
            embedding_text("Prazo", "Entrega em 3 dias."),
            "Prazo\nEntrega em 3 dias."
        );
        assert_eq!(embedding_text("  ", "Só o texto."), "Só o texto.");
    }
}
