use crate::error::FileError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tipo {
    Jpeg,
    Png,
    Webp,
    Gif,
    Pdf,
    Docx,
    Xlsx,
    Pptx,
    Xls,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Family {
    Image,
    Pdf,
    Docx,
    Planilha,
    Pptx,
}

impl Tipo {
    pub fn mime(self) -> &'static str {
        match self {
            Tipo::Jpeg => "image/jpeg",
            Tipo::Png => "image/png",
            Tipo::Webp => "image/webp",
            Tipo::Gif => "image/gif",
            Tipo::Pdf => "application/pdf",
            Tipo::Docx => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            Tipo::Xlsx => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            Tipo::Pptx => {
                "application/vnd.openxmlformats-officedocument.presentationml.presentation"
            }
            Tipo::Xls => "application/vnd.ms-excel",
        }
    }

    pub fn nome(self) -> &'static str {
        match self {
            Tipo::Jpeg => "uma imagem JPEG",
            Tipo::Png => "uma imagem PNG",
            Tipo::Webp => "uma imagem WebP",
            Tipo::Gif => "uma imagem GIF",
            Tipo::Pdf => "um PDF",
            Tipo::Docx => "um documento Word (.docx)",
            Tipo::Xlsx => "uma planilha Excel (.xlsx)",
            Tipo::Pptx => "uma apresentação PowerPoint (.pptx)",
            Tipo::Xls => "uma planilha Excel antiga (.xls)",
        }
    }

    pub fn is_image(self) -> bool {
        self.family() == Family::Image
    }

    pub fn reader_format(self) -> Option<&'static str> {
        match self {
            Tipo::Pdf => Some("pdf"),
            Tipo::Docx => Some("docx"),
            Tipo::Xlsx => Some("xlsx"),
            Tipo::Pptx => Some("pptx"),
            Tipo::Xls => Some("xls"),
            Tipo::Jpeg | Tipo::Png | Tipo::Webp | Tipo::Gif => None,
        }
    }

    fn family(self) -> Family {
        match self {
            Tipo::Jpeg | Tipo::Png | Tipo::Webp | Tipo::Gif => Family::Image,
            Tipo::Pdf => Family::Pdf,
            Tipo::Docx => Family::Docx,
            Tipo::Xlsx | Tipo::Xls => Family::Planilha,
            Tipo::Pptx => Family::Pptx,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Refusal {
    Executavel,
    Script,
    Audio,
    Video,
    LegacyOffice,
    Compactado,
    PlainText,
    Unknown,
    Other,
}

impl Refusal {
    fn descricao(&self) -> String {
        match self {
            Refusal::Executavel => "um programa executável".to_owned(),
            Refusal::Script => "um script".to_owned(),
            Refusal::Audio => "um áudio".to_owned(),
            Refusal::Video => "um vídeo".to_owned(),
            Refusal::LegacyOffice => {
                "um arquivo do Word ou PowerPoint antigo (.doc ou .ppt)".to_owned()
            }
            Refusal::Compactado => "um arquivo compactado (zip, rar ou similar)".to_owned(),
            Refusal::PlainText => "um texto simples".to_owned(),
            Refusal::Unknown => "um formato que não reconheço".to_owned(),
            Refusal::Other => "um arquivo de outro formato".to_owned(),
        }
    }

    fn mensagem(&self) -> String {
        let accepted_list = "PDF, Word (.docx), Excel (.xlsx ou .xls), PowerPoint (.pptx) ou imagem (JPEG, PNG, WebP ou GIF)";
        match self {
            Refusal::LegacyOffice => {
                "Word e PowerPoint no formato antigo (.doc e .ppt) não são aceitos. Salve como .docx ou .pptx, ou envie em PDF.".to_owned()
            }
            Refusal::Audio | Refusal::Video => format!(
                "{} não é aceito nesta versão. Envie {accepted_list}.",
                capitalizar(&self.descricao())
            ),
            _ => format!(
                "{} não é aceito. Envie {accepted_list}.",
                capitalizar(&self.descricao())
            ),
        }
    }
}

pub(crate) const BLOCKED_EXTENSIONS: &[&str] = &[
    "exe", "dll", "so", "com", "scr", "msi", "bat", "cmd", "ps1", "vbs", "vbe", "js", "jse", "wsf",
    "sh", "bash", "py", "pl", "rb", "php", "jar", "apk", "app", "deb", "rpm", "dmg", "elf",
];

pub fn classify(bytes: &[u8], nome: &str) -> Result<Tipo, FileError> {
    let extension = lowercase_extension(nome);
    let real = detectar(bytes);

    if let Some(ext) = extension.as_deref()
        && BLOCKED_EXTENSIONS.contains(&ext)
    {
        let descricao = match &real {
            Ok(tipo) => tipo.nome().to_owned(),
            Err(refusal) => refusal.descricao(),
        };
        return Err(FileError::UnsupportedType(format!(
            "o nome termina em .{ext}, que é extensão de programa ou script, e o conteúdo é {descricao}"
        )));
    }

    match real {
        Err(refusal) => {
            if let Some(ext) = extension.as_deref()
                && family_of_extension(ext).is_some()
            {
                return Err(FileError::MisleadingExtension {
                    declared: ext.to_owned(),
                    real: refusal.descricao(),
                });
            }
            Err(FileError::UnsupportedType(refusal.mensagem()))
        }
        Ok(tipo) => {
            if let Some(ext) = extension.as_deref()
                && let Some(family) = family_of_extension(ext)
                && family != tipo.family()
            {
                return Err(FileError::MisleadingExtension {
                    declared: ext.to_owned(),
                    real: tipo.nome().to_owned(),
                });
            }
            Ok(tipo)
        }
    }
}

fn detectar(bytes: &[u8]) -> Result<Tipo, Refusal> {
    if let Some(tipo) = infer::get(bytes) {
        let mime = tipo.mime_type();
        if mime == "application/zip"
            && let Some(office) = office_by_package(bytes)
        {
            return Ok(office);
        }
        return match mime {
            "image/jpeg" => Ok(Tipo::Jpeg),
            "image/png" => Ok(Tipo::Png),
            "image/webp" => Ok(Tipo::Webp),
            "image/gif" => Ok(Tipo::Gif),
            "application/pdf" => Ok(Tipo::Pdf),
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document" => {
                Ok(Tipo::Docx)
            }
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet" => Ok(Tipo::Xlsx),
            "application/vnd.openxmlformats-officedocument.presentationml.presentation" => {
                Ok(Tipo::Pptx)
            }
            "application/vnd.ms-excel" => Ok(Tipo::Xls),
            "application/msword" | "application/vnd.ms-powerpoint" => Err(Refusal::LegacyOffice),
            m if m.starts_with("audio/") => Err(Refusal::Audio),
            m if m.starts_with("video/") => Err(Refusal::Video),
            "application/x-executable"
            | "application/x-elf"
            | "application/x-sharedlib"
            | "application/x-mach-binary"
            | "application/vnd.microsoft.portable-executable"
            | "application/x-msdownload"
            | "application/x-dosexec"
            | "application/x-msi"
            | "application/java-archive" => Err(Refusal::Executavel),
            "application/zip"
            | "application/x-rar-compressed"
            | "application/vnd.rar"
            | "application/gzip"
            | "application/x-bzip2"
            | "application/x-xz"
            | "application/zstd"
            | "application/x-7z-compressed"
            | "application/x-tar" => Err(Refusal::Compactado),
            _ if std::str::from_utf8(bytes).is_ok() && looks_like_script(bytes) => {
                Err(Refusal::Script)
            }
            _ => Err(Refusal::Other),
        };
    }

    if std::str::from_utf8(bytes).is_ok() {
        if looks_like_script(bytes) {
            Err(Refusal::Script)
        } else {
            Err(Refusal::PlainText)
        }
    } else {
        Err(Refusal::Unknown)
    }
}

pub(crate) fn office_by_package(bytes: &[u8]) -> Option<Tipo> {
    let contem = |nome: &[u8]| bytes.windows(nome.len()).any(|window| window == nome);
    if contem(b"word/document.xml") {
        Some(Tipo::Docx)
    } else if contem(b"xl/workbook.xml") || contem(b"xl/worksheets/") {
        Some(Tipo::Xlsx)
    } else if contem(b"ppt/presentation.xml") || contem(b"ppt/slides/") {
        Some(Tipo::Pptx)
    } else {
        None
    }
}

fn looks_like_script(bytes: &[u8]) -> bool {
    let start = &bytes[..bytes.len().min(256)];
    let texto = String::from_utf8_lossy(start).trim_start().to_lowercase();
    texto.starts_with("#!")
        || texto.starts_with("<?php")
        || texto.starts_with("<script")
        || texto.starts_with("@echo")
        || texto.starts_with("set-executionpolicy")
        || texto.starts_with("powershell")
}

fn family_of_extension(ext: &str) -> Option<Family> {
    match ext {
        "jpg" | "jpeg" | "png" | "webp" | "gif" => Some(Family::Image),
        "pdf" => Some(Family::Pdf),
        "docx" => Some(Family::Docx),
        "xlsx" | "xls" => Some(Family::Planilha),
        "pptx" => Some(Family::Pptx),
        _ => None,
    }
}

pub(crate) fn lowercase_extension(nome: &str) -> Option<String> {
    let clean_name = nome.rsplit(['/', '\\']).next().unwrap_or(nome);
    let (_, ext) = clean_name.rsplit_once('.')?;
    if ext.is_empty() || ext.len() > 16 {
        return None;
    }
    Some(ext.to_lowercase())
}

fn capitalizar(texto: &str) -> String {
    let mut characters = texto.chars();
    match characters.next() {
        Some(first) => first.to_uppercase().collect::<String>() + characters.as_str(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_from_path_lowercased() {
        assert_eq!(
            lowercase_extension("pasta/Contrato.PDF").as_deref(),
            Some("pdf")
        );
        assert_eq!(
            lowercase_extension("..\\x\\foto.JPeG").as_deref(),
            Some("jpeg")
        );
        assert_eq!(lowercase_extension("sem_extensao"), None);
        assert_eq!(lowercase_extension("termina_com_ponto."), None);
    }

    #[test]
    fn script_with_shebang_recognized() {
        assert!(looks_like_script(b"#!/bin/sh\necho oi\n"));
        assert!(looks_like_script(b"  @echo off\r\n"));
        assert!(!looks_like_script(b"Relatorio do mes de novembro"));
    }

    #[test]
    fn plain_text_without_signature_refused() {
        let error = classify("oi, tudo bem?".as_bytes(), "nota.txt").unwrap_err();
        assert!(matches!(error, FileError::UnsupportedType(ref m) if m.contains("texto simples")));
    }

    #[test]
    fn pdf_signature_with_pdf_name_accepted() {
        let pdf = b"%PDF-1.4\n%fake\n";
        assert_eq!(classify(pdf, "contrato.pdf").unwrap(), Tipo::Pdf);
    }

    #[test]
    fn zip_with_word_document_xml_is_docx() {
        let mut zip = b"PK\x03\x04".to_vec();
        zip.extend_from_slice(&[0u8; 26]);
        zip.extend_from_slice(b"word/document.xml");
        zip.extend_from_slice(&[0u8; 32]);
        assert_eq!(classify(&zip, "a.docx").unwrap(), Tipo::Docx);
    }

    #[test]
    fn pdf_named_docx_misleading_extension() {
        let pdf = b"%PDF-1.4\n%fake\n";
        let error = classify(pdf, "contrato.docx").unwrap_err();
        assert!(matches!(error, FileError::MisleadingExtension { .. }));
    }
}
