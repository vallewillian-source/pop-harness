//! O que o WhatsApp mostra de verdade e quanto texto cabe (HR-26).
//!
//! O POP Atende já resolveu duas coisas na prática e a regra é **reproduzida**, não reinventada
//! (`apps/backend/core/utils/markdown_to_whatsapp.py`):
//!
//! - **Formatação.** O WhatsApp não renderiza Markdown: negrito é `*texto*`, itálico é `_texto_`,
//!   riscado é `~texto~`, título é negrito na própria linha, link é `texto (url)` e lista é `• item`.
//!   Tabela não existe no canal: vira linha `coluna: valor`.
//! - **Limite.** [`LIMITE_TEXTO`] é 4000 caracteres. A Evolution/WhatsApp **trunca** o que passa
//!   disso, então o texto é cortado aqui e termina em `…` — não é dividido em duas mensagens (é o
//!   que o POP Atende faz hoje: `WHATSAPP_TEXT_LIMIT = 4000` e `text[: limit - 1].rstrip() + "…"`).
//!
//! Blocos de código e código em linha ficam **protegidos**: o conteúdo deles não é reformatado, só
//! convertido para o formato de código do WhatsApp.

use std::sync::LazyLock;

use regex::Regex;

/// Teto de texto do canal, em caracteres (o mesmo do POP Atende).
pub const LIMITE_TEXTO: usize = 4000;

/// Linha de tabela Markdown: começa e termina em `|`, com pelo menos duas seguidas.
static TABELA: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^(?:\|[^\n]*\|[ \t]*(?:\n|$)){2,}").expect("regex de tabela")
});

/// Bloco de código cercado por três crases, com linguagem opcional.
static CERCA: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?s)```(?:[A-Za-z0-9_+-]+)?\n?(.*?)```").expect("regex de cerca")
});

/// Código em linha (uma crase de cada lado).
static LINHA: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"`([^`]+?)`").expect("regex de código em linha"));

/// Negrito e itálico juntos.
static NEGRITO_ITALICO: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\*\*\*(.+?)\*\*\*|___(.+?)___").expect("regex de negrito e itálico")
});

/// Negrito.
static NEGRITO: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\*\*(.+?)\*\*|__(.+?)__").expect("regex de negrito"));

/// Itálico com asterisco simples (sem colar em negrito nem em lista).
static ITALICO: LazyLock<Regex> = LazyLock::new(|| {
    // Uma estrela de cada lado, sem começar nem terminar em espaço. O `regex` do Rust não tem
    // look-around, então a proteção contra `**negrito**` já foi feita antes (o negrito virou
    // marcador) e `PREGAO` não é tocado porque o casamento exige estrela dos dois lados.
    Regex::new(r"\*([^*\s](?:[^*]*[^*\s])?)\*").expect("regex de itálico")
});

/// Riscado.
static RISCADO: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"~~(.+?)~~").expect("regex de riscado"));

/// Título (`#`, `##`…) no começo da linha.
static TITULO: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^(?:#+)[ \t]+(.+)$").expect("regex de título"));

/// Link Markdown `[texto](url)`.
static LINK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\[([^\]]+)\]\(([^)]+)\)").expect("regex de link"));

/// Item de lista com marcador.
static ITEM: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^([ \t]*)[-*][ \t]+").expect("regex de item"));

/// Item de lista numerado.
static ITEM_NUMERADO: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^([ \t]*)(\d+)[.)][ \t]+").expect("regex de item numerado"));

/// Converte Markdown no que o WhatsApp renderiza e corta no limite do canal.
///
/// Os trechos já convertidos viram marcadores (`WAGUARDADO{n}WAFIM`) enquanto o resto é tratado,
/// senão o `*negrito*` recém-criado seria lido como itálico na regra seguinte. É o mesmo truque de
/// marcadores do POP Atende, com um sufixo no marcador para um `1` não casar dentro de um `10`.
pub fn para_whatsapp(texto: &str) -> String {
    if texto.trim().is_empty() {
        return String::new();
    }
    let mut texto = achatar_tabelas(texto);
    let mut guardados: Vec<(String, String)> = Vec::new();

    // 1. Código fica de fora da formatação.
    texto = CERCA
        .replace_all(&texto, |capturas: &regex::Captures<'_>| {
            guardar(&mut guardados, format!("`{}`", capturas[1].trim()))
        })
        .into_owned();
    texto = LINHA
        .replace_all(&texto, |capturas: &regex::Captures<'_>| {
            guardar(&mut guardados, format!("`{}`", &capturas[1]))
        })
        .into_owned();

    // 2. Formatação, guardando cada resultado.
    texto = NEGRITO_ITALICO
        .replace_all(&texto, |c: &regex::Captures<'_>| {
            let dentro = c
                .get(1)
                .or_else(|| c.get(2))
                .map(|m| m.as_str())
                .unwrap_or("");
            guardar(&mut guardados, format!("*_{dentro}_*"))
        })
        .into_owned();
    texto = NEGRITO
        .replace_all(&texto, |c: &regex::Captures<'_>| {
            let dentro = c
                .get(1)
                .or_else(|| c.get(2))
                .map(|m| m.as_str())
                .unwrap_or("");
            guardar(&mut guardados, format!("*{dentro}*"))
        })
        .into_owned();
    texto = ITALICO
        .replace_all(&texto, |c: &regex::Captures<'_>| {
            guardar(&mut guardados, format!("_{}_", &c[1]))
        })
        .into_owned();
    texto = RISCADO.replace_all(&texto, "~${1}~").into_owned();
    texto = TITULO.replace_all(&texto, "*${1}*").into_owned();
    texto = LINK.replace_all(&texto, "${1} (${2})").into_owned();
    texto = ITEM.replace_all(&texto, "${1}• ").into_owned();
    texto = ITEM_NUMERADO.replace_all(&texto, "${1}${2}) ").into_owned();

    // 3. Devolve o que foi guardado.
    for (chave, valor) in guardados {
        texto = texto.replace(&chave, &valor);
    }

    cortar(&texto)
}

/// Guarda um trecho convertido e devolve o marcador que ocupa o lugar dele.
fn guardar(guardados: &mut Vec<(String, String)>, valor: String) -> String {
    let chave = format!("WAGUARDADO{}WAFIM", guardados.len());
    guardados.push((chave.clone(), valor));
    chave
}

/// Corta no teto do canal, terminando em `…` (o WhatsApp trunca sozinho, e truncar no meio da
/// palavra é pior do que avisar que cortou).
pub fn cortar(texto: &str) -> String {
    if texto.chars().count() <= LIMITE_TEXTO {
        return texto.to_owned();
    }
    let mut cortado: String = texto.chars().take(LIMITE_TEXTO - 1).collect();
    while cortado.ends_with(char::is_whitespace) {
        cortado.pop();
    }
    cortado.push('…');
    cortado
}

/// Tabela Markdown vira uma linha `coluna: valor` por linha de dados (o WhatsApp não desenha
/// tabela, e uma tabela crua vira `|` solto no meio do texto).
fn achatar_tabelas(texto: &str) -> String {
    TABELA
        .replace_all(texto, |capturas: &regex::Captures<'_>| {
            let bloco = capturas[0].trim_end();
            let linhas: Vec<&str> = bloco.lines().collect();
            if linhas.len() < 2 {
                return capturas[0].to_owned();
            }
            let celulas = |linha: &str| -> Vec<String> {
                linha
                    .trim()
                    .trim_matches('|')
                    .split('|')
                    .map(|celula| celula.trim().to_owned())
                    .collect()
            };
            let cabecalho = celulas(linhas[0]);
            let mut saida: Vec<String> = Vec::new();
            for linha in &linhas[1..] {
                let valores = celulas(linha);
                // Linha separadora (`|---|---|`) não tem dado.
                let separadora = valores.iter().all(|valor| {
                    !valor.is_empty() && valor.chars().all(|c| matches!(c, '-' | ':' | ' '))
                });
                if separadora {
                    continue;
                }
                let pares: Vec<String> = valores
                    .iter()
                    .enumerate()
                    .filter(|(_, valor)| !valor.is_empty())
                    .map(|(indice, valor)| match cabecalho.get(indice) {
                        Some(titulo) if !titulo.is_empty() => format!("{titulo}: {valor}"),
                        _ => valor.clone(),
                    })
                    .collect();
                if !pares.is_empty() {
                    saida.push(pares.join(" · "));
                }
            }
            if saida.is_empty() {
                capturas[0].to_owned()
            } else {
                saida.join("\n")
            }
        })
        .into_owned()
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn negrito_e_italico_viram_o_formato_do_canal() {
        assert_eq!(para_whatsapp("**oi** e _tchau_"), "*oi* e _tchau_");
        assert_eq!(para_whatsapp("***forte***"), "*_forte_*");
        assert_eq!(para_whatsapp("~~riscado~~"), "~riscado~");
    }

    #[test]
    fn titulo_link_e_lista() {
        assert_eq!(
            para_whatsapp("## Resumo\n- um\n- dois"),
            "*Resumo*\n• um\n• dois"
        );
        assert_eq!(
            para_whatsapp("veja [aqui](https://exemplo.test/x)"),
            "veja aqui (https://exemplo.test/x)"
        );
        assert_eq!(para_whatsapp("1. passo"), "1) passo");
    }

    #[test]
    fn codigo_nao_e_reformatado() {
        assert_eq!(
            para_whatsapp("use `**nao** vira negrito`"),
            "use `**nao** vira negrito`"
        );
    }

    #[test]
    fn tabela_vira_linhas_de_chave_e_valor() {
        let tabela = "| Plano | Preco |\n|---|---|\n| Basico | 100 |\n| Pro | 200 |";
        assert_eq!(
            para_whatsapp(tabela),
            "Plano: Basico · Preco: 100\nPlano: Pro · Preco: 200"
        );
    }

    #[test]
    fn texto_longo_e_cortado_no_limite() {
        let longo = "a".repeat(LIMITE_TEXTO + 50);
        let cortado = para_whatsapp(&longo);
        assert_eq!(cortado.chars().count(), LIMITE_TEXTO);
        assert!(cortado.ends_with('…'));
    }

    #[test]
    fn texto_no_limite_nao_e_cortado() {
        let exato = "b".repeat(LIMITE_TEXTO);
        assert_eq!(para_whatsapp(&exato), exato);
        assert!(!para_whatsapp(&exato).ends_with('…'));
    }
}
