use std::sync::LazyLock;

use regex::Regex;

pub const TEXT_LIMIT: usize = 4000;

static TABLE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^(?:\|[^\n]*\|[ \t]*(?:\n|$)){2,}").expect("regex de tabela")
});

static FENCE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?s)```(?:[A-Za-z0-9_+-]+)?\n?(.*?)```").expect("regex de cerca")
});

static INLINE_CODE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"`([^`]+?)`").expect("regex de código em linha"));

static BOLD_ITALIC: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\*\*\*(.+?)\*\*\*|___(.+?)___").expect("regex de negrito e itálico")
});

static BOLD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\*\*(.+?)\*\*|__(.+?)__").expect("regex de negrito"));

static ITALIC: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\*([^*\s](?:[^*]*[^*\s])?)\*").expect("regex de itálico"));

static STRIKE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"~~(.+?)~~").expect("regex de riscado"));

static HEADING: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^(?:#+)[ \t]+(.+)$").expect("regex de título"));

static LINK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\[([^\]]+)\]\(([^)]+)\)").expect("regex de link"));

static ITEM: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^([ \t]*)[-*][ \t]+").expect("regex de item"));

static NUMBERED_ITEM: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^([ \t]*)(\d+)[.)][ \t]+").expect("regex de item numerado"));

pub fn to_whatsapp(text: &str) -> String {
    if text.trim().is_empty() {
        return String::new();
    }
    let mut text = flatten_tables(text);
    let mut stashed: Vec<(String, String)> = Vec::new();

    text = FENCE
        .replace_all(&text, |captures: &regex::Captures<'_>| {
            stash(&mut stashed, format!("`{}`", captures[1].trim()))
        })
        .into_owned();
    text = INLINE_CODE
        .replace_all(&text, |captures: &regex::Captures<'_>| {
            stash(&mut stashed, format!("`{}`", &captures[1]))
        })
        .into_owned();

    text = BOLD_ITALIC
        .replace_all(&text, |c: &regex::Captures<'_>| {
            let inner = c
                .get(1)
                .or_else(|| c.get(2))
                .map(|m| m.as_str())
                .unwrap_or("");
            stash(&mut stashed, format!("*_{inner}_*"))
        })
        .into_owned();
    text = BOLD
        .replace_all(&text, |c: &regex::Captures<'_>| {
            let inner = c
                .get(1)
                .or_else(|| c.get(2))
                .map(|m| m.as_str())
                .unwrap_or("");
            stash(&mut stashed, format!("*{inner}*"))
        })
        .into_owned();
    text = ITALIC
        .replace_all(&text, |c: &regex::Captures<'_>| {
            stash(&mut stashed, format!("_{}_", &c[1]))
        })
        .into_owned();
    text = STRIKE.replace_all(&text, "~${1}~").into_owned();
    text = HEADING.replace_all(&text, "*${1}*").into_owned();
    text = LINK.replace_all(&text, "${1} (${2})").into_owned();
    text = ITEM.replace_all(&text, "${1}• ").into_owned();
    text = NUMBERED_ITEM.replace_all(&text, "${1}${2}) ").into_owned();

    for (key, value) in stashed {
        text = text.replace(&key, &value);
    }

    truncate(&text)
}

fn stash(stashed: &mut Vec<(String, String)>, value: String) -> String {
    let key = format!("WAGUARDADO{}WAFIM", stashed.len());
    stashed.push((key.clone(), value));
    key
}

pub fn truncate(text: &str) -> String {
    if text.chars().count() <= TEXT_LIMIT {
        return text.to_owned();
    }
    let mut truncated: String = text.chars().take(TEXT_LIMIT - 1).collect();
    while truncated.ends_with(char::is_whitespace) {
        truncated.pop();
    }
    truncated.push('…');
    truncated
}

fn flatten_tables(text: &str) -> String {
    TABLE
        .replace_all(text, |captures: &regex::Captures<'_>| {
            let block = captures[0].trim_end();
            let lines: Vec<&str> = block.lines().collect();
            if lines.len() < 2 {
                return captures[0].to_owned();
            }
            let cells = |line: &str| -> Vec<String> {
                line.trim()
                    .trim_matches('|')
                    .split('|')
                    .map(|cell| cell.trim().to_owned())
                    .collect()
            };
            let header = cells(lines[0]);
            let mut saida: Vec<String> = Vec::new();
            for line in &lines[1..] {
                let values = cells(line);
                let separator = values.iter().all(|value| {
                    !value.is_empty() && value.chars().all(|c| matches!(c, '-' | ':' | ' '))
                });
                if separator {
                    continue;
                }
                let pairs: Vec<String> = values
                    .iter()
                    .enumerate()
                    .filter(|(_, value)| !value.is_empty())
                    .map(|(index, value)| match header.get(index) {
                        Some(title) if !title.is_empty() => format!("{title}: {value}"),
                        _ => value.clone(),
                    })
                    .collect();
                if !pairs.is_empty() {
                    saida.push(pairs.join(" · "));
                }
            }
            if saida.is_empty() {
                captures[0].to_owned()
            } else {
                saida.join("\n")
            }
        })
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bold_and_italic_become_the_channel_format() {
        assert_eq!(to_whatsapp("**oi** e _tchau_"), "*oi* e _tchau_");
        assert_eq!(to_whatsapp("***forte***"), "*_forte_*");
        assert_eq!(to_whatsapp("~~riscado~~"), "~riscado~");
    }

    #[test]
    fn heading_link_and_list() {
        assert_eq!(
            to_whatsapp("## Resumo\n- um\n- dois"),
            "*Resumo*\n• um\n• dois"
        );
        assert_eq!(
            to_whatsapp("veja [aqui](https://exemplo.test/x)"),
            "veja aqui (https://exemplo.test/x)"
        );
        assert_eq!(to_whatsapp("1. passo"), "1) passo");
    }

    #[test]
    fn code_is_not_reformatted() {
        assert_eq!(
            to_whatsapp("use `**nao** vira negrito`"),
            "use `**nao** vira negrito`"
        );
    }

    #[test]
    fn table_becomes_key_value_lines() {
        let table = "| Plano | Preco |\n|---|---|\n| Basico | 100 |\n| Pro | 200 |";
        assert_eq!(
            to_whatsapp(table),
            "Plano: Basico · Preco: 100\nPlano: Pro · Preco: 200"
        );
    }

    #[test]
    fn long_text_is_truncated_at_the_limit() {
        let long = "a".repeat(TEXT_LIMIT + 50);
        let truncated = to_whatsapp(&long);
        assert_eq!(truncated.chars().count(), TEXT_LIMIT);
        assert!(truncated.ends_with('…'));
    }

    #[test]
    fn text_at_the_limit_is_not_truncated() {
        let exact = "b".repeat(TEXT_LIMIT);
        assert_eq!(to_whatsapp(&exact), exact);
        assert!(!to_whatsapp(&exact).ends_with('…'));
    }
}
