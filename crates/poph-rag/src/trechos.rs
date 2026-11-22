//! Divisão do texto de um item em trechos (HR-16) e a identidade de cada item (hash de origem).
//!
//! Regras (D-09 e RELATORIO_HR02 §1): trechos de até [`TAMANHO_TRECHO_CARACTERES`] (350), com
//! sobreposição de até [`SOBREPOSICAO_CARACTERES`] (80) entre vizinhos. A divisão respeita
//! parágrafo (linha em branco) e frase. Só quebra palavra quando uma palavra sozinha é maior que
//! o trecho. O título do item entra no texto embutido e no índice de texto.

use sha2::{Digest, Sha256};

use crate::embedder::{
    IdentidadeModelo, SOBREPOSICAO_CARACTERES as SOBREPOSICAO, TAMANHO_TRECHO_CARACTERES as TAMANHO,
};

/// Texto que vai para o modelo e para o índice de texto: o título na primeira linha, depois o trecho.
pub fn texto_embutido(titulo: &str, trecho: &str) -> String {
    let titulo = titulo.trim();
    if titulo.is_empty() {
        trecho.to_owned()
    } else {
        format!("{titulo}\n{trecho}")
    }
}

/// Divide `texto` em trechos. Texto vazio, ou só de espaços, dá lista vazia.
pub fn dividir(texto: &str) -> Vec<String> {
    let mut trechos = Vec::new();
    let mut atual: Vec<Unidade> = Vec::new();
    for unidade in unidades(texto) {
        if !atual.is_empty() {
            let cabe = medida(&atual) + 1 + tamanho(&unidade.texto) <= TAMANHO;
            let novo_paragrafo = unidade.inicia_paragrafo && medida(&atual) >= TAMANHO / 2;
            if !cabe || novo_paragrafo {
                trechos.push(juntar(&atual));
                // Entre parágrafos não há sobreposição: cada parágrafo começa limpo.
                atual = if novo_paragrafo {
                    Vec::new()
                } else {
                    sobreposicao(&atual, &unidade)
                };
            }
        }
        atual.push(unidade);
    }
    if !atual.is_empty() {
        trechos.push(juntar(&atual));
    }
    trechos
}

/// Identidade do conteúdo de um item: muda com o texto, o título, a versão do divisor e o modelo.
/// Igual entrada, igual hash. Usada para pular item sem mudança na reindexação.
pub fn hash_origem(identidade: &IdentidadeModelo, titulo: &str, texto: &str) -> String {
    let mut hash = Sha256::new();
    let partes = [
        identidade.versao_chunker,
        identidade.nome,
        identidade.quantizacao,
        identidade.sha256_modelo.as_str(),
        identidade.sha256_tokenizador.as_str(),
        titulo,
        texto,
    ];
    for parte in partes {
        hash.update(parte.as_bytes());
        hash.update([0u8]);
    }
    hash.finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Unidade {
    texto: String,
    /// Primeira unidade de um parágrafo: quando o trecho chega perto do meio, quebra aqui.
    inicia_paragrafo: bool,
}

fn tamanho(texto: &str) -> usize {
    texto.chars().count()
}

/// Caracteres ocupados pelas unidades juntas, com um separador (1 caractere) entre elas.
fn medida(unidades: &[Unidade]) -> usize {
    if unidades.is_empty() {
        return 0;
    }
    unidades.iter().map(|u| tamanho(&u.texto)).sum::<usize>() + unidades.len() - 1
}

fn juntar(unidades: &[Unidade]) -> String {
    let mut texto = String::new();
    for (indice, unidade) in unidades.iter().enumerate() {
        if indice > 0 {
            texto.push(if unidade.inicia_paragrafo { '\n' } else { ' ' });
        }
        texto.push_str(&unidade.texto);
    }
    texto
}

/// As últimas unidades inteiras do trecho que fecha, até [`SOBREPOSICAO`] caracteres, e que ainda
/// deixam espaço para a unidade seguinte dentro de [`TAMANHO`].
fn sobreposicao(atual: &[Unidade], proxima: &Unidade) -> Vec<Unidade> {
    let mut manter = 0;
    let mut usado = 0;
    for unidade in atual.iter().rev() {
        let com_esta = usado + if manter == 0 { 0 } else { 1 } + tamanho(&unidade.texto);
        if com_esta > SOBREPOSICAO || com_esta + 1 + tamanho(&proxima.texto) > TAMANHO {
            break;
        }
        usado = com_esta;
        manter += 1;
    }
    atual[atual.len() - manter..].to_vec()
}

/// Parágrafos (separados por linha em branco) viram frases, e frases longas viram pedaços.
fn unidades(texto: &str) -> Vec<Unidade> {
    let mut paragrafos: Vec<String> = Vec::new();
    let mut atual = String::new();
    for linha in texto.lines() {
        let limpa = linha.split_whitespace().collect::<Vec<_>>().join(" ");
        if limpa.is_empty() {
            if !atual.is_empty() {
                paragrafos.push(std::mem::take(&mut atual));
            }
        } else if atual.is_empty() {
            atual = limpa;
        } else {
            atual.push(' ');
            atual.push_str(&limpa);
        }
    }
    if !atual.is_empty() {
        paragrafos.push(atual);
    }

    let mut saida = Vec::new();
    for paragrafo in paragrafos {
        let mut primeira = true;
        for frase in frases(&paragrafo) {
            for pedaco in quebra_longa(&frase) {
                saida.push(Unidade {
                    texto: pedaco,
                    inicia_paragrafo: primeira,
                });
                primeira = false;
            }
        }
    }
    saida
}

/// Corta em ponto, exclamação, interrogação ou ponto e vírgula seguidos de espaço ou do fim.
fn frases(paragrafo: &str) -> Vec<String> {
    let caracteres: Vec<char> = paragrafo.chars().collect();
    let mut frases = Vec::new();
    let mut atual = String::new();
    for (indice, caractere) in caracteres.iter().enumerate() {
        atual.push(*caractere);
        let fim_de_frase = caracteres
            .get(indice + 1)
            .is_none_or(|proximo| proximo.is_whitespace());
        if matches!(caractere, '.' | '!' | '?' | ';') && fim_de_frase {
            frases.push(std::mem::take(&mut atual));
        }
    }
    if !atual.trim().is_empty() {
        frases.push(atual);
    }
    frases
        .into_iter()
        .map(|frase| frase.trim().to_owned())
        .filter(|frase| !frase.is_empty())
        .collect()
}

/// Frase que cabe em um trecho fica como está. Frase maior quebra em palavras; palavra maior que o
/// trecho quebra em blocos de caracteres.
fn quebra_longa(frase: &str) -> Vec<String> {
    if tamanho(frase) <= TAMANHO {
        return vec![frase.to_owned()];
    }
    let mut pedacos = Vec::new();
    let mut atual = String::new();
    for palavra in frase.split_whitespace() {
        let caracteres: Vec<char> = palavra.chars().collect();
        if caracteres.len() > TAMANHO {
            if !atual.is_empty() {
                pedacos.push(std::mem::take(&mut atual));
            }
            for bloco in caracteres.chunks(TAMANHO) {
                let texto: String = bloco.iter().collect();
                if bloco.len() == TAMANHO {
                    pedacos.push(texto);
                } else {
                    atual = texto;
                }
            }
            continue;
        }
        let com_palavra = if atual.is_empty() {
            tamanho(palavra)
        } else {
            tamanho(&atual) + 1 + tamanho(palavra)
        };
        if com_palavra > TAMANHO {
            pedacos.push(std::mem::replace(&mut atual, palavra.to_owned()));
        } else {
            if !atual.is_empty() {
                atual.push(' ');
            }
            atual.push_str(palavra);
        }
    }
    if !atual.is_empty() {
        pedacos.push(atual);
    }
    pedacos
}

#[cfg(test)]
mod testes {
    use super::*;

    fn frase(n: usize) -> String {
        format!("Frase número {n} com um pouco de texto para o trecho.")
    }

    #[test]
    fn texto_vazio_ou_so_espacos_nao_gera_trecho() {
        assert!(dividir("").is_empty());
        assert!(dividir("   \n\n \t ").is_empty());
    }

    #[test]
    fn texto_curto_vira_um_trecho_normalizado() {
        let trechos = dividir("Primeira   linha.\n\nSegundo parágrafo!");
        assert_eq!(
            trechos,
            vec!["Primeira linha.\nSegundo parágrafo!".to_owned()]
        );
    }

    #[test]
    fn nenhum_trecho_passa_do_tamanho_e_o_texto_inteiro_aparece() {
        let texto = (1..=120).map(frase).collect::<Vec<_>>().join(" ");
        let trechos = dividir(&texto);
        assert!(trechos.len() > 10, "deveria gerar vários trechos");
        for trecho in &trechos {
            assert!(tamanho(trecho) <= TAMANHO, "trecho grande demais: {trecho}");
        }
        for n in 1..=120 {
            assert!(
                trechos.iter().any(|t| t.contains(&format!("número {n} "))),
                "a frase {n} sumiu"
            );
        }
    }

    #[test]
    fn vizinhos_se_sobrepoem_no_maximo_o_tamanho_da_sobreposicao() {
        let texto = (1..=60).map(frase).collect::<Vec<_>>().join(" ");
        let trechos = dividir(&texto);
        for par in trechos.windows(2) {
            let (antes, depois) = (&par[0], &par[1]);
            let comum = (1..=SOBREPOSICAO.min(tamanho(antes))).rev().find(|k| {
                let fim: String = antes.chars().skip(tamanho(antes) - k).collect();
                depois.starts_with(&fim)
            });
            assert!(
                comum.is_some(),
                "trechos vizinhos sem sobreposição: {antes:?} / {depois:?}"
            );
            assert!(comum.unwrap_or(0) <= SOBREPOSICAO);
        }
    }

    #[test]
    fn palavra_maior_que_o_trecho_e_quebrada_sem_perder_caracteres() {
        let palavra = "x".repeat(1000);
        let trechos = dividir(&palavra);
        assert!(trechos.iter().all(|t| tamanho(t) <= TAMANHO));
        assert_eq!(trechos.concat(), palavra);
    }

    #[test]
    fn divisao_e_determinística() {
        let texto = (1..=40).map(frase).collect::<Vec<_>>().join("\n\n");
        assert_eq!(dividir(&texto), dividir(&texto));
    }

    #[test]
    fn hash_muda_com_titulo_texto_ou_modelo_e_fica_igual_sem_mudanca() {
        use crate::embedder::{NOME_MODELO, VERSAO_CHUNKER};
        let base = IdentidadeModelo {
            nome: NOME_MODELO,
            quantizacao: "int8",
            dimensao: 384,
            janela_tokens: 128,
            sha256_modelo: "aa".to_owned(),
            sha256_tokenizador: "bb".to_owned(),
            versao_chunker: VERSAO_CHUNKER,
        };
        let h = hash_origem(&base, "Titulo", "Texto");
        assert_eq!(h, hash_origem(&base, "Titulo", "Texto"));
        assert_ne!(h, hash_origem(&base, "Outro", "Texto"));
        assert_ne!(h, hash_origem(&base, "Titulo", "Outro"));
        let outro_modelo = IdentidadeModelo {
            sha256_modelo: "cc".to_owned(),
            ..base.clone()
        };
        assert_ne!(h, hash_origem(&outro_modelo, "Titulo", "Texto"));
    }

    #[test]
    fn texto_embutido_leva_o_titulo_na_frente() {
        assert_eq!(
            texto_embutido("Prazo", "Entrega em 3 dias."),
            "Prazo\nEntrega em 3 dias."
        );
        assert_eq!(texto_embutido("  ", "Só o texto."), "Só o texto.");
    }
}
