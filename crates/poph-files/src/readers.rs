use std::io::{Cursor, Read};

use quick_xml::escape::resolve_predefined_entity;
use quick_xml::events::Event;
use zip::ZipArchive;

pub const XML_BLOCK_LIMIT: u64 = 32 * 1024 * 1024;
pub const XML_TOTAL_LIMIT: u64 = 64 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Loaded {
    pub texto: String,
    pub characters: usize,
}

pub fn read(formato: &str, bytes: &[u8], character_limit: usize) -> Result<Loaded, String> {
    let texto = match formato {
        "pdf" => pdf_extract::extract_text_from_mem(bytes)
            .map_err(|_| "o PDF não pôde ser lido (pode estar corrompido)".to_owned())?,
        "docx" => docx(bytes)?,
        "pptx" => pptx(bytes)?,
        "xlsx" | "xls" => planilha(bytes)?,
        other => return Err(format!("formato de leitura desconhecido: {other}")),
    };
    let texto = texto.trim().to_owned();
    let characters = texto.chars().count();
    let texto: String = texto.chars().take(character_limit).collect();
    Ok(Loaded { texto, characters })
}

fn open_zip(bytes: &[u8]) -> Result<ZipArchive<Cursor<&[u8]>>, String> {
    ZipArchive::new(Cursor::new(bytes))
        .map_err(|_| "o arquivo não é um documento Office válido".to_owned())
}

fn read_block(
    file: &mut ZipArchive<Cursor<&[u8]>>,
    nome: &str,
    total: &mut u64,
) -> Result<Option<String>, String> {
    let mut input = match file.by_name(nome) {
        Ok(input) => input,
        Err(zip::result::ZipError::FileNotFound) => return Ok(None),
        Err(_) => return Err("o documento Office está danificado".to_owned()),
    };
    if input.size() > XML_BLOCK_LIMIT {
        return Err(
            "o conteúdo, descompactado, passa do limite de 32 MB; arquivo recusado".to_owned(),
        );
    }
    let mut buffer = Vec::new();
    (&mut input)
        .take(XML_BLOCK_LIMIT + 1)
        .read_to_end(&mut buffer)
        .map_err(|_| "o documento Office está danificado".to_owned())?;
    if buffer.len() as u64 > XML_BLOCK_LIMIT {
        return Err(
            "o conteúdo, descompactado, passa do limite de 32 MB; arquivo recusado".to_owned(),
        );
    }
    *total += buffer.len() as u64;
    if *total > XML_TOTAL_LIMIT {
        return Err(
            "o conteúdo, descompactado, passa do limite de 64 MB; arquivo recusado".to_owned(),
        );
    }
    Ok(Some(String::from_utf8_lossy(&buffer).into_owned()))
}

fn docx(bytes: &[u8]) -> Result<String, String> {
    let mut file = open_zip(bytes)?;
    let mut total = 0;
    match read_block(&mut file, "word/document.xml", &mut total)? {
        Some(xml) => Ok(xml_text(&xml)),
        None => Err("o documento Word não tem conteúdo de texto".to_owned()),
    }
}

fn pptx(bytes: &[u8]) -> Result<String, String> {
    let mut file = open_zip(bytes)?;
    let mut slides: Vec<(u32, String)> = file
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
    let mut parts = Vec::new();
    for (numero, nome) in slides {
        if let Some(xml) = read_block(&mut file, &nome, &mut total)? {
            let texto = xml_text(&xml);
            if !texto.trim().is_empty() {
                parts.push(format!("Slide {numero}:\n{}", texto.trim()));
            }
        }
    }
    Ok(parts.join("\n\n"))
}

fn xml_text(xml: &str) -> String {
    let mut reader = quick_xml::Reader::from_str(xml);
    let mut saida = String::new();
    let mut inside_text = false;
    let mut in_cell = false;
    while let Ok(evento) = reader.read_event() {
        match evento {
            Event::Eof => break,
            Event::Start(e) => match e.local_name().as_ref() {
                "t" => inside_text = true,
                "tc" => in_cell = true,
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
                "t" => inside_text = false,
                "p" if in_cell => saida.push(' '),
                "p" => saida.push('\n'),
                "tc" => {
                    in_cell = false;
                    trim_trailing_spaces(&mut saida);
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
            Event::Text(e) if inside_text => saida.push_str(&e.xml10_content()),
            Event::GeneralRef(e) if inside_text => {
                if let Ok(Some(character)) = e.resolve_char_ref() {
                    saida.push(character);
                } else if let Some(value) = resolve_predefined_entity(&e.xml10_content()) {
                    saida.push_str(value);
                }
            }
            _ => {}
        }
    }
    saida
}

fn trim_trailing_spaces(saida: &mut String) {
    while saida.ends_with(' ') {
        saida.pop();
    }
}

fn planilha(bytes: &[u8]) -> Result<String, String> {
    use calamine::{Data, Reader, open_workbook_auto_from_rs};

    let mut planilha = open_workbook_auto_from_rs(Cursor::new(bytes)).map_err(|_| {
        "a planilha não pôde ser aberta (pode estar corrompida ou ser de outro formato)".to_owned()
    })?;
    let names = planilha.sheet_names();
    let mut saida = String::new();
    for nome in names {
        let interval = planilha
            .worksheet_range(&nome)
            .map_err(|_| "uma das abas da planilha não pôde ser lida".to_owned())?;
        saida.push_str(&format!("Planilha: {nome}\n"));
        for line in interval.rows() {
            let cells: Vec<String> = line
                .iter()
                .map(|cell| match cell {
                    Data::Empty => String::new(),
                    other_value => other_value.to_string(),
                })
                .collect();
            if cells.iter().all(String::is_empty) {
                continue;
            }
            saida.push_str(&cells.join("\t"));
            saida.push('\n');
        }
        saida.push('\n');
    }
    Ok(saida)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xml_paragraphs_tables_entities() {
        let xml = r#"<w:document xmlns:w="x"><w:body>
            <w:p><w:r><w:t>Ingestão &amp; cuidado</w:t></w:r></w:p>
            <w:tbl><w:tr><w:tc><w:p><w:r><w:t>Sintoma</w:t></w:r></w:p></w:tc>
            <w:tc><w:p><w:r><w:t>Conduta</w:t></w:r></w:p></w:tc></w:tr></w:tbl>
            <w:p><w:r><w:t>a</w:t><w:tab/><w:t>b</w:t></w:r></w:p>
        </w:body></w:document>"#;
        let texto = xml_text(xml);
        assert!(texto.contains("Ingestão & cuidado"), "{texto:?}");
        assert!(texto.contains("Sintoma\tConduta\n"), "{texto:?}");
        assert!(texto.contains("a\tb"), "{texto:?}");
    }

    #[test]
    fn non_xml_text_keeps_reading_alive() {
        assert_eq!(xml_text("<w:p><w:t>ok"), "ok");
    }

    #[test]
    fn cut_respects_character_limit() {
        let loaded = read("pdf", b"nao e pdf", 10);
        assert!(loaded.is_err());
    }
}
