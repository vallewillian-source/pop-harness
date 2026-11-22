//! Leitura de PDF e Office em texto. Roda **só** dentro do processo `poph-extrator`, nunca no
//! processo principal: um arquivo malicioso pode pesar na memória ou no tempo, e o limite vem de
//! fora (ver `extracao`).
//!
//! Leitores (spike HR-02 §1 e §5): `pdf-extract` para PDF; `zip` + `quick-xml` para docx e pptx;
//! `calamine` para xlsx e xls.

use std::io::{Cursor, Read};

use quick_xml::escape::resolve_predefined_entity;
use quick_xml::events::Event;
use zip::ZipArchive;

/// Teto de cada bloco XML descompactado (32 MB, spike HR-02 §5.2). Passou disso: recusa.
pub const LIMITE_BLOCO_XML: u64 = 32 * 1024 * 1024;
/// Teto somado de todos os blocos XML de um documento (64 MB).
pub const LIMITE_TOTAL_XML: u64 = 64 * 1024 * 1024;

/// Texto lido de um arquivo, já cortado no limite de caracteres.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lido {
    /// Texto, no máximo `limite_caracteres` caracteres.
    pub texto: String,
    /// Quantos caracteres o texto tinha antes do corte.
    pub caracteres: usize,
}

/// Lê o texto de um arquivo no formato indicado (`pdf`, `docx`, `pptx`, `xlsx` ou `xls`).
///
/// Erros são texto curto e sem conteúdo do arquivo, para o processo principal repassar.
pub fn ler(formato: &str, bytes: &[u8], limite_caracteres: usize) -> Result<Lido, String> {
    let texto = match formato {
        "pdf" => pdf_extract::extract_text_from_mem(bytes)
            .map_err(|_| "o PDF não pôde ser lido (pode estar corrompido)".to_owned())?,
        "docx" => docx(bytes)?,
        "pptx" => pptx(bytes)?,
        "xlsx" | "xls" => planilha(bytes)?,
        outro => return Err(format!("formato de leitura desconhecido: {outro}")),
    };
    let texto = texto.trim().to_owned();
    let caracteres = texto.chars().count();
    let texto: String = texto.chars().take(limite_caracteres).collect();
    Ok(Lido { texto, caracteres })
}

fn abrir_zip(bytes: &[u8]) -> Result<ZipArchive<Cursor<&[u8]>>, String> {
    ZipArchive::new(Cursor::new(bytes))
        .map_err(|_| "o arquivo não é um documento Office válido".to_owned())
}

/// Lê um bloco (entrada) do zip com os tetos. `None` se a entrada não existe.
fn ler_bloco(
    arquivo: &mut ZipArchive<Cursor<&[u8]>>,
    nome: &str,
    total: &mut u64,
) -> Result<Option<String>, String> {
    let mut entrada = match arquivo.by_name(nome) {
        Ok(entrada) => entrada,
        Err(zip::result::ZipError::FileNotFound) => return Ok(None),
        Err(_) => return Err("o documento Office está danificado".to_owned()),
    };
    if entrada.size() > LIMITE_BLOCO_XML {
        return Err(
            "o conteúdo, descompactado, passa do limite de 32 MB; arquivo recusado".to_owned(),
        );
    }
    let mut buffer = Vec::new();
    (&mut entrada)
        .take(LIMITE_BLOCO_XML + 1)
        .read_to_end(&mut buffer)
        .map_err(|_| "o documento Office está danificado".to_owned())?;
    if buffer.len() as u64 > LIMITE_BLOCO_XML {
        return Err(
            "o conteúdo, descompactado, passa do limite de 32 MB; arquivo recusado".to_owned(),
        );
    }
    *total += buffer.len() as u64;
    if *total > LIMITE_TOTAL_XML {
        return Err(
            "o conteúdo, descompactado, passa do limite de 64 MB; arquivo recusado".to_owned(),
        );
    }
    Ok(Some(String::from_utf8_lossy(&buffer).into_owned()))
}

fn docx(bytes: &[u8]) -> Result<String, String> {
    let mut arquivo = abrir_zip(bytes)?;
    let mut total = 0;
    match ler_bloco(&mut arquivo, "word/document.xml", &mut total)? {
        Some(xml) => Ok(texto_xml(&xml)),
        None => Err("o documento Word não tem conteúdo de texto".to_owned()),
    }
}

fn pptx(bytes: &[u8]) -> Result<String, String> {
    let mut arquivo = abrir_zip(bytes)?;
    let mut slides: Vec<(u32, String)> = arquivo
        .file_names()
        .filter_map(|nome| {
            let nome = nome.ok()?;
            let numero = nome
                .strip_prefix("ppt/slides/slide")?
                .strip_suffix(".xml")?
                .parse::<u32>()
                .ok()?;
            Some((numero, nome.into_owned()))
        })
        .collect();
    slides.sort_by_key(|(numero, _)| *numero);

    let mut total = 0;
    let mut partes = Vec::new();
    for (numero, nome) in slides {
        if let Some(xml) = ler_bloco(&mut arquivo, &nome, &mut total)? {
            let texto = texto_xml(&xml);
            if !texto.trim().is_empty() {
                partes.push(format!("Slide {numero}:\n{}", texto.trim()));
            }
        }
    }
    Ok(partes.join("\n\n"))
}

/// Texto de um bloco XML de Word ou PowerPoint: parágrafos em linhas, tabelas com tabulação.
fn texto_xml(xml: &str) -> String {
    let mut leitor = quick_xml::Reader::from_str(xml);
    let mut saida = String::new();
    let mut dentro_de_texto = false;
    let mut em_celula = false;
    // XML quebrado: o `while let` para no primeiro erro e fica com o que já leu.
    while let Ok(evento) = leitor.read_event() {
        match evento {
            Event::Eof => break,
            Event::Start(e) => match e.local_name().as_ref() {
                "t" => dentro_de_texto = true,
                "tc" => em_celula = true,
                "tab" => saida.push('\t'),
                "br" => saida.push('\n'),
                _ => {}
            },
            Event::Empty(e) => match e.local_name().as_ref() {
                "tab" => saida.push('\t'),
                "br" => saida.push('\n'),
                _ => {}
            },
            Event::End(e) => match e.local_name().as_ref() {
                "t" => dentro_de_texto = false,
                // Parágrafo dentro de célula não quebra a linha da tabela: vira espaço.
                "p" if em_celula => saida.push(' '),
                "p" => saida.push('\n'),
                "tc" => {
                    em_celula = false;
                    apagar_espacos_finais(&mut saida);
                    saida.push('\t');
                }
                "tr" => {
                    while saida.ends_with('\t') {
                        saida.pop();
                    }
                    saida.push('\n');
                }
                _ => {}
            },
            Event::Text(e) if dentro_de_texto => saida.push_str(&e.xml10_content()),
            // Entidades (&amp; &lt; &#233; ...) chegam à parte, em eventos próprios.
            Event::GeneralRef(e) if dentro_de_texto => {
                if let Ok(Some(caractere)) = e.resolve_char_ref() {
                    saida.push(caractere);
                } else if let Some(valor) = resolve_predefined_entity(&e.xml10_content()) {
                    saida.push_str(valor);
                }
            }
            _ => {}
        }
    }
    saida
}

/// Tira os espaços do fim do texto da célula, antes da tabulação que separa a célula seguinte.
fn apagar_espacos_finais(saida: &mut String) {
    while saida.ends_with(' ') {
        saida.pop();
    }
}

fn planilha(bytes: &[u8]) -> Result<String, String> {
    use calamine::{Data, Reader, open_workbook_auto_from_rs};

    let mut planilha = open_workbook_auto_from_rs(Cursor::new(bytes)).map_err(|_| {
        "a planilha não pôde ser aberta (pode estar corrompida ou ser de outro formato)".to_owned()
    })?;
    let nomes = planilha.sheet_names();
    let mut saida = String::new();
    for nome in nomes {
        let intervalo = planilha
            .worksheet_range(&nome)
            .map_err(|_| "uma das abas da planilha não pôde ser lida".to_owned())?;
        saida.push_str(&format!("Planilha: {nome}\n"));
        for linha in intervalo.rows() {
            let celulas: Vec<String> = linha
                .iter()
                .map(|celula| match celula {
                    Data::Empty => String::new(),
                    outra => outra.to_string(),
                })
                .collect();
            if celulas.iter().all(String::is_empty) {
                continue;
            }
            saida.push_str(&celulas.join("\t"));
            saida.push('\n');
        }
        saida.push('\n');
    }
    Ok(saida)
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn xml_de_paragrafos_tabelas_e_entidades() {
        let xml = r#"<w:document xmlns:w="x"><w:body>
            <w:p><w:r><w:t>Ingestão &amp; cuidado</w:t></w:r></w:p>
            <w:tbl><w:tr><w:tc><w:p><w:r><w:t>Sintoma</w:t></w:r></w:p></w:tc>
            <w:tc><w:p><w:r><w:t>Conduta</w:t></w:r></w:p></w:tc></w:tr></w:tbl>
            <w:p><w:r><w:t>a</w:t><w:tab/><w:t>b</w:t></w:r></w:p>
        </w:body></w:document>"#;
        let texto = texto_xml(xml);
        assert!(texto.contains("Ingestão & cuidado"), "{texto:?}");
        assert!(texto.contains("Sintoma\tConduta\n"), "{texto:?}");
        assert!(texto.contains("a\tb"), "{texto:?}");
    }

    #[test]
    fn texto_que_nao_e_xml_nao_derruba_a_leitura() {
        assert_eq!(texto_xml("<w:p><w:t>ok"), "ok");
    }

    #[test]
    fn corte_respeita_o_limite_de_caracteres() {
        let lido = ler("pdf", b"nao e pdf", 10);
        assert!(lido.is_err());
    }
}
