use crate::error::FileError;
use crate::file_type::{BLOCKED_EXTENSIONS, lowercase_extension, office_by_package};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutboundFamily {
    Image,
    Document,
    Audio,
    Video,
}

impl OutboundFamily {
    pub fn code(self) -> &'static str {
        match self {
            Self::Image => "imagem",
            Self::Document => "documento",
            Self::Audio => "audio",
            Self::Video => "video",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutboundType {
    mime: &'static str,
    family: OutboundFamily,
}

impl OutboundType {
    pub fn mime(self) -> &'static str {
        self.mime
    }

    pub fn family(self) -> OutboundFamily {
        self.family
    }

    pub fn is_image(self) -> bool {
        self.family == OutboundFamily::Image
    }

    pub fn code(self) -> &'static str {
        self.family.code()
    }

    pub fn default_name(self) -> &'static str {
        match self.family {
            OutboundFamily::Image => "imagem",
            OutboundFamily::Document => "arquivo",
            OutboundFamily::Audio => "audio",
            OutboundFamily::Video => "video",
        }
    }
}

const ACCEPTED_MIMES: &[(&str, OutboundFamily)] = &[
    ("image/jpeg", OutboundFamily::Image),
    ("image/png", OutboundFamily::Image),
    ("image/webp", OutboundFamily::Image),
    ("image/gif", OutboundFamily::Image),
    ("application/pdf", OutboundFamily::Document),
    (
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        OutboundFamily::Document,
    ),
    (
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        OutboundFamily::Document,
    ),
    (
        "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        OutboundFamily::Document,
    ),
    ("application/vnd.ms-excel", OutboundFamily::Document),
    ("audio/mpeg", OutboundFamily::Audio),
    ("audio/m4a", OutboundFamily::Audio),
    ("audio/ogg", OutboundFamily::Audio),
    ("audio/opus", OutboundFamily::Audio),
    ("audio/x-wav", OutboundFamily::Audio),
    ("audio/amr", OutboundFamily::Audio),
    ("audio/aac", OutboundFamily::Audio),
    ("audio/x-flac", OutboundFamily::Audio),
    ("audio/x-aiff", OutboundFamily::Audio),
    ("video/mp4", OutboundFamily::Video),
    ("video/x-m4v", OutboundFamily::Video),
    ("video/quicktime", OutboundFamily::Video),
    ("video/webm", OutboundFamily::Video),
    ("video/x-matroska", OutboundFamily::Video),
    ("video/x-msvideo", OutboundFamily::Video),
    ("video/mpeg", OutboundFamily::Video),
    ("video/x-ms-wmv", OutboundFamily::Video),
    ("video/x-flv", OutboundFamily::Video),
];

pub fn classify(bytes: &[u8], nome: &str) -> Result<OutboundType, FileError> {
    let extension = lowercase_extension(nome);

    if let Some(ext) = extension.as_deref()
        && BLOCKED_EXTENSIONS.contains(&ext)
    {
        return Err(FileError::UnsupportedType(format!(
            "o nome termina em .{ext}, que é extensão de programa ou script"
        )));
    }

    let real = detectar(bytes)?;
    if let Some(ext) = extension.as_deref()
        && let Some(group) = group_of_extension(ext)
        && !same_group(group, group_of_type(real))
    {
        return Err(FileError::MisleadingExtension {
            declared: ext.to_owned(),
            real: real.mime().to_owned(),
        });
    }
    Ok(real)
}

fn detectar(bytes: &[u8]) -> Result<OutboundType, FileError> {
    let Some(inferido) = infer::get(bytes) else {
        return Err(FileError::UnsupportedType(
            "o conteúdo não é um dos tipos que o cliente recebe (imagem, PDF, Office, áudio ou vídeo)"
                .to_owned(),
        ));
    };
    let mut mime = inferido.mime_type();

    if mime == "application/zip" {
        let Some(office) = office_by_package(bytes) else {
            return Err(FileError::UnsupportedType(
                "o conteúdo é um arquivo compactado, que não vai ao cliente".to_owned(),
            ));
        };
        mime = office.mime();
    }

    from_mime(mime).ok_or_else(|| {
        FileError::UnsupportedType(format!(
            "o conteúdo é {mime}, que não é um dos tipos que o cliente recebe"
        ))
    })
}

fn from_mime(mime: &str) -> Option<OutboundType> {
    ACCEPTED_MIMES
        .iter()
        .find(|(accepted, _)| *accepted == mime)
        .map(|(accepted, family)| OutboundType {
            mime: accepted,
            family: *family,
        })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Group {
    Image,
    Pdf,
    Docx,
    Planilha,
    Pptx,
    Audio,
    Video,
}

fn group_of_type(tipo: OutboundType) -> Group {
    match tipo.family() {
        OutboundFamily::Image => Group::Image,
        OutboundFamily::Audio => Group::Audio,
        OutboundFamily::Video => Group::Video,
        OutboundFamily::Document => match tipo.mime() {
            "application/pdf" => Group::Pdf,
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document" => {
                Group::Docx
            }
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
            | "application/vnd.ms-excel" => Group::Planilha,
            _ => Group::Pptx,
        },
    }
}

fn group_of_extension(ext: &str) -> Option<Group> {
    match ext {
        "jpg" | "jpeg" | "png" | "webp" | "gif" => Some(Group::Image),
        "pdf" => Some(Group::Pdf),
        "docx" => Some(Group::Docx),
        "xlsx" | "xls" => Some(Group::Planilha),
        "pptx" => Some(Group::Pptx),
        "mp3" | "m4a" | "ogg" | "oga" | "opus" | "wav" | "amr" | "aac" | "flac" | "aiff" => {
            Some(Group::Audio)
        }
        "mp4" | "m4v" | "mov" | "webm" | "mkv" | "avi" | "mpg" | "mpeg" | "wmv" | "flv" | "3gp" => {
            Some(Group::Video)
        }
        _ => None,
    }
}

fn same_group(declared_group: Group, real: Group) -> bool {
    if declared_group == real {
        return true;
    }
    matches!(
        (declared_group, real),
        (Group::Audio, Group::Video) | (Group::Video, Group::Audio)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const PNG: &[u8] = &[
        0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f,
        0x15, 0xc4, 0x89, 0x00, 0x00, 0x00, 0x0a, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0x00,
        0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x0d, 0x0a, 0x2d, 0xb4, 0x00, 0x00, 0x00, 0x00, 0x49,
        0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
    ];

    const PDF: &[u8] =
        b"%PDF-1.4\n1 0 obj\n<< /Type /Catalog >>\nendobj\ntrailer\n<< /Root 1 0 R >>\n%%EOF\n";

    fn ogg_opus() -> Vec<u8> {
        let mut bytes = b"OggS".to_vec();
        bytes.extend_from_slice(&[0x00, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]);
        bytes.extend_from_slice(&[0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00]);
        bytes.extend_from_slice(b"OpusHead");
        bytes.extend_from_slice(&[0x01, 0x02, 0x38, 0x01, 0x80, 0xbb, 0x00, 0x00]);
        bytes
    }

    fn mp4() -> Vec<u8> {
        let mut bytes = vec![0x00, 0x00, 0x00, 0x18];
        bytes.extend_from_slice(b"ftypisom");
        bytes.extend_from_slice(&[0x00, 0x00, 0x02, 0x00]);
        bytes.extend_from_slice(b"isomiso2");
        bytes
    }

    #[test]
    fn image_pdf_and_office_unchanged() {
        let png = classify(PNG, "foto.png").expect("PNG é imagem");
        assert_eq!(png.mime(), "image/png");
        assert_eq!(png.family(), OutboundFamily::Image);
        assert!(png.is_image());
        assert_eq!(png.default_name(), "imagem");

        let pdf = classify(PDF, "laudo.pdf").expect("PDF é documento");
        assert_eq!(pdf.mime(), "application/pdf");
        assert_eq!(pdf.family(), OutboundFamily::Document);
        assert!(!pdf.is_image());
        assert_eq!(pdf.default_name(), "arquivo");
    }

    #[test]
    fn audio_and_video_listed_with_real_mime() {
        let audio = classify(&ogg_opus(), "voice-message.ogg").expect("OGG/Opus é áudio");
        assert_eq!(audio.family(), OutboundFamily::Audio);
        assert_eq!(
            audio.mime(),
            "audio/ogg",
            "o áudio do Element é contêiner OGG"
        );
        assert_eq!(audio.code(), "audio");
        assert_eq!(audio.default_name(), "audio");

        let video = classify(&mp4(), "video.mp4").expect("MP4 é vídeo");
        assert_eq!(video.mime(), "video/mp4");
        assert_eq!(video.family(), OutboundFamily::Video);
        assert_eq!(video.code(), "video");
        assert_eq!(video.default_name(), "video");
    }

    #[test]
    fn audio_named_mp4_is_not_misleading() {
        let audio = classify(&ogg_opus(), "gravacao.mp4")
            .expect("áudio e vídeo são o mesmo grupo para nome enganoso");
        assert_eq!(audio.family(), OutboundFamily::Audio);
    }

    #[test]
    fn refused_content_stays_refused() {
        let zip = b"PK\x03\x04\x14\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00";
        assert!(matches!(
            classify(zip, "pacote.zip"),
            Err(FileError::UnsupportedType(_))
        ));
        assert!(matches!(
            classify(b"apenas um texto qualquer\n", "notas.txt"),
            Err(FileError::UnsupportedType(_))
        ));
        assert!(matches!(
            classify(PNG, "abrir.sh"),
            Err(FileError::UnsupportedType(_))
        ));
    }

    #[test]
    fn misleading_name_refused_for_other_group() {
        let error = classify(PNG, "laudo.pdf").expect_err("imagem chamada de PDF");
        assert!(
            matches!(error, FileError::MisleadingExtension { .. }),
            "{error:?}"
        );
        let error = classify(PDF, "foto.png").expect_err("PDF chamado de imagem");
        assert!(
            matches!(error, FileError::MisleadingExtension { .. }),
            "{error:?}"
        );
        let error = classify(&ogg_opus(), "laudo.pdf").expect_err("áudio chamado de PDF");
        assert!(
            matches!(error, FileError::MisleadingExtension { .. }),
            "{error:?}"
        );
        let error = classify(PDF, "contrato.docx").expect_err("PDF chamado de Word");
        assert!(
            matches!(error, FileError::MisleadingExtension { .. }),
            "{error:?}"
        );
    }

    #[test]
    fn list_is_closed_no_prefix_family() {
        assert!(from_mime("audio/midi").is_none());
        assert!(from_mime("video/x-ms-wmv-extra").is_none());
        assert!(from_mime("application/zip").is_none());
        assert!(from_mime("application/octet-stream").is_none());
        assert!(from_mime("audio/opus").is_some());
    }
}
