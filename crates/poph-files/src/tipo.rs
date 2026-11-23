//! Tipo real do arquivo, pelo conteúdo (assinatura), com a lista de aceitos e as recusas.
//!
//! A extensão do nome só serve para pegar arquivo enganoso: nunca decide o tipo sozinha.

use crate::erro::ErroArquivo;

/// Tipos aceitos. Imagens vão ao LLM como imagem; os demais viram texto extraído.
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

/// Grupo de extensão: `.jpg` e `.png` são o mesmo grupo (imagem), por exemplo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Familia {
    Imagem,
    Pdf,
    Docx,
    /// Xlsx e xls: o mesmo leitor de planilha. Nome trocado entre os dois não é enganoso.
    Planilha,
    Pptx,
}

impl Tipo {
    /// Tipo MIME real, como o `infer` o reconhece.
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

    /// Nome do formato para a mensagem ao cliente e para o LLM.
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

    /// Se o arquivo vai ao LLM como imagem (e não como texto).
    pub fn eh_imagem(self) -> bool {
        self.familia() == Familia::Imagem
    }

    /// Nome do leitor de texto (usado pelo programa `poph-extrator`). `None` para imagem.
    pub fn formato_leitor(self) -> Option<&'static str> {
        match self {
            Tipo::Pdf => Some("pdf"),
            Tipo::Docx => Some("docx"),
            Tipo::Xlsx => Some("xlsx"),
            Tipo::Pptx => Some("pptx"),
            Tipo::Xls => Some("xls"),
            Tipo::Jpeg | Tipo::Png | Tipo::Webp | Tipo::Gif => None,
        }
    }

    fn familia(self) -> Familia {
        match self {
            Tipo::Jpeg | Tipo::Png | Tipo::Webp | Tipo::Gif => Familia::Imagem,
            Tipo::Pdf => Familia::Pdf,
            Tipo::Docx => Familia::Docx,
            Tipo::Xlsx | Tipo::Xls => Familia::Planilha,
            Tipo::Pptx => Familia::Pptx,
        }
    }
}

/// Motivo de recusa de um conteúdo que não é um dos tipos aceitos.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Recusa {
    Executavel,
    Script,
    Audio,
    Video,
    OfficeAntigo,
    Compactado,
    TextoSimples,
    Desconhecido,
    Outro,
}

impl Recusa {
    /// Descrição curta do que o arquivo é, para encaixar em "o conteúdo é …".
    fn descricao(&self) -> String {
        match self {
            Recusa::Executavel => "um programa executável".to_owned(),
            Recusa::Script => "um script".to_owned(),
            Recusa::Audio => "um áudio".to_owned(),
            Recusa::Video => "um vídeo".to_owned(),
            Recusa::OfficeAntigo => {
                "um arquivo do Word ou PowerPoint antigo (.doc ou .ppt)".to_owned()
            }
            Recusa::Compactado => "um arquivo compactado (zip, rar ou similar)".to_owned(),
            Recusa::TextoSimples => "um texto simples".to_owned(),
            Recusa::Desconhecido => "um formato que não reconheço".to_owned(),
            Recusa::Outro => "um arquivo de outro formato".to_owned(),
        }
    }

    /// Mensagem educada para o cliente (sem jargão), com o que dá para enviar no lugar.
    fn mensagem(&self) -> String {
        let aceitos = "PDF, Word (.docx), Excel (.xlsx ou .xls), PowerPoint (.pptx) ou imagem (JPEG, PNG, WebP ou GIF)";
        match self {
            Recusa::OfficeAntigo => {
                "Word e PowerPoint no formato antigo (.doc e .ppt) não são aceitos. Salve como .docx ou .pptx, ou envie em PDF.".to_owned()
            }
            Recusa::Audio | Recusa::Video => format!(
                "{} não é aceito nesta versão. Envie {aceitos}.",
                capitalizar(&self.descricao())
            ),
            _ => format!(
                "{} não é aceito. Envie {aceitos}.",
                capitalizar(&self.descricao())
            ),
        }
    }
}

/// Extensões que são de programa ou script: recusadas mesmo que o conteúdo pareça outra coisa.
pub(crate) const EXTENSOES_BLOQUEADAS: &[&str] = &[
    "exe", "dll", "so", "com", "scr", "msi", "bat", "cmd", "ps1", "vbs", "vbe", "js", "jse", "wsf",
    "sh", "bash", "py", "pl", "rb", "php", "jar", "apk", "app", "deb", "rpm", "dmg", "elf",
];

/// Descobre o tipo real do arquivo e recusa o que não é aceito.
///
/// Erros: [`ErroArquivo::TipoNaoAceito`] para programa, script, áudio, formato antigo etc.;
/// [`ErroArquivo::ExtensaoEnganosa`] quando o nome diz um tipo aceito e o conteúdo é outro.
pub fn classificar(bytes: &[u8], nome: &str) -> Result<Tipo, ErroArquivo> {
    let extensao = extensao_minuscula(nome);
    let real = detectar(bytes);

    if let Some(ext) = extensao.as_deref()
        && EXTENSOES_BLOQUEADAS.contains(&ext)
    {
        let descricao = match &real {
            Ok(tipo) => tipo.nome().to_owned(),
            Err(recusa) => recusa.descricao(),
        };
        return Err(ErroArquivo::TipoNaoAceito(format!(
            "o nome termina em .{ext}, que é extensão de programa ou script, e o conteúdo é {descricao}"
        )));
    }

    match real {
        Err(recusa) => {
            // Nome de tipo aceito + conteúdo recusado: é o caso do "PDF que é executável".
            if let Some(ext) = extensao.as_deref()
                && familia_da_extensao(ext).is_some()
            {
                return Err(ErroArquivo::ExtensaoEnganosa {
                    declarada: ext.to_owned(),
                    real: recusa.descricao(),
                });
            }
            Err(ErroArquivo::TipoNaoAceito(recusa.mensagem()))
        }
        Ok(tipo) => {
            if let Some(ext) = extensao.as_deref()
                && let Some(familia) = familia_da_extensao(ext)
                && familia != tipo.familia()
            {
                return Err(ErroArquivo::ExtensaoEnganosa {
                    declarada: ext.to_owned(),
                    real: tipo.nome().to_owned(),
                });
            }
            Ok(tipo)
        }
    }
}

/// Tipo pelo conteúdo. Assinatura conhecida primeiro; sem assinatura, texto ou binário.
fn detectar(bytes: &[u8]) -> Result<Tipo, Recusa> {
    if let Some(tipo) = infer::get(bytes) {
        let mime = tipo.mime_type();
        // Word, Excel e PowerPoint mínimos saem como ZIP genérico: o nome interno do pacote decide.
        if mime == "application/zip"
            && let Some(office) = office_pelo_pacote(bytes)
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
            "application/msword" | "application/vnd.ms-powerpoint" => Err(Recusa::OfficeAntigo),
            m if m.starts_with("audio/") => Err(Recusa::Audio),
            m if m.starts_with("video/") => Err(Recusa::Video),
            "application/x-executable"
            | "application/x-elf"
            | "application/x-sharedlib"
            | "application/x-mach-binary"
            | "application/vnd.microsoft.portable-executable"
            | "application/x-msdownload"
            | "application/x-dosexec"
            | "application/x-msi"
            | "application/java-archive" => Err(Recusa::Executavel),
            "application/zip"
            | "application/x-rar-compressed"
            | "application/vnd.rar"
            | "application/gzip"
            | "application/x-bzip2"
            | "application/x-xz"
            | "application/zstd"
            | "application/x-7z-compressed"
            | "application/x-tar" => Err(Recusa::Compactado),
            _ if std::str::from_utf8(bytes).is_ok() && parece_script(bytes) => Err(Recusa::Script),
            _ => Err(Recusa::Outro),
        };
    }

    // Sem assinatura conhecida: texto simples (ou script disfarçado de texto) ou binário sem nome.
    if std::str::from_utf8(bytes).is_ok() {
        if parece_script(bytes) {
            Err(Recusa::Script)
        } else {
            Err(Recusa::TextoSimples)
        }
    } else {
        Err(Recusa::Desconhecido)
    }
}

/// Office dentro de um ZIP, pelo nome do arquivo interno (ficam em texto puro no cabeçalho do ZIP).
///
/// Não abre o ZIP aqui: a leitura de verdade é feita no processo isolado. Se o nome for falso,
/// o leitor isolado falha e o arquivo é recusado.
pub(crate) fn office_pelo_pacote(bytes: &[u8]) -> Option<Tipo> {
    let contem = |nome: &[u8]| bytes.windows(nome.len()).any(|janela| janela == nome);
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

/// Primeiros bytes de um script conhecido (shebang, marca de PHP, lote do Windows, HTML com script).
fn parece_script(bytes: &[u8]) -> bool {
    let inicio = &bytes[..bytes.len().min(256)];
    let texto = String::from_utf8_lossy(inicio).trim_start().to_lowercase();
    texto.starts_with("#!")
        || texto.starts_with("<?php")
        || texto.starts_with("<script")
        || texto.starts_with("@echo")
        || texto.starts_with("set-executionpolicy")
        || texto.starts_with("powershell")
}

/// Grupo da extensão declarada, se ela for de um tipo aceito.
fn familia_da_extensao(ext: &str) -> Option<Familia> {
    match ext {
        "jpg" | "jpeg" | "png" | "webp" | "gif" => Some(Familia::Imagem),
        "pdf" => Some(Familia::Pdf),
        "docx" => Some(Familia::Docx),
        "xlsx" | "xls" => Some(Familia::Planilha),
        "pptx" => Some(Familia::Pptx),
        _ => None,
    }
}

/// Extensão do nome em minúsculas, sem o ponto. `None` se não houver.
pub(crate) fn extensao_minuscula(nome: &str) -> Option<String> {
    let nome_limpo = nome.rsplit(['/', '\\']).next().unwrap_or(nome);
    let (_, ext) = nome_limpo.rsplit_once('.')?;
    if ext.is_empty() || ext.len() > 16 {
        return None;
    }
    Some(ext.to_lowercase())
}

/// Coloca a primeira letra em maiúscula, para começar uma frase.
fn capitalizar(texto: &str) -> String {
    let mut caracteres = texto.chars();
    match caracteres.next() {
        Some(primeiro) => primeiro.to_uppercase().collect::<String>() + caracteres.as_str(),
        None => String::new(),
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn extensao_sai_do_nome_com_caminho_e_em_minusculas() {
        assert_eq!(
            extensao_minuscula("pasta/Contrato.PDF").as_deref(),
            Some("pdf")
        );
        assert_eq!(
            extensao_minuscula("..\\x\\foto.JPeG").as_deref(),
            Some("jpeg")
        );
        assert_eq!(extensao_minuscula("sem_extensao"), None);
        assert_eq!(extensao_minuscula("termina_com_ponto."), None);
    }

    #[test]
    fn script_com_shebang_e_reconhecido() {
        assert!(parece_script(b"#!/bin/sh\necho oi\n"));
        assert!(parece_script(b"  @echo off\r\n"));
        assert!(!parece_script(b"Relatorio do mes de novembro"));
    }

    #[test]
    fn texto_simples_sem_assinatura_e_recusado() {
        let erro = classificar("oi, tudo bem?".as_bytes(), "nota.txt").unwrap_err();
        assert!(matches!(erro, ErroArquivo::TipoNaoAceito(ref m) if m.contains("texto simples")));
    }

    #[test]
    fn assinatura_de_pdf_com_nome_de_pdf_e_aceita() {
        let pdf = b"%PDF-1.4\n%fake\n";
        assert_eq!(classificar(pdf, "contrato.pdf").unwrap(), Tipo::Pdf);
    }

    #[test]
    fn zip_com_word_document_xml_e_docx() {
        let mut zip = b"PK\x03\x04".to_vec();
        zip.extend_from_slice(&[0u8; 26]);
        zip.extend_from_slice(b"word/document.xml");
        zip.extend_from_slice(&[0u8; 32]);
        assert_eq!(classificar(&zip, "a.docx").unwrap(), Tipo::Docx);
    }

    #[test]
    fn pdf_com_nome_de_docx_e_extensao_enganosa() {
        let pdf = b"%PDF-1.4\n%fake\n";
        let erro = classificar(pdf, "contrato.docx").unwrap_err();
        assert!(matches!(erro, ErroArquivo::ExtensaoEnganosa { .. }));
    }
}
