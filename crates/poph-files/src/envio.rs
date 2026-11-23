//! Tipo real do arquivo que o **atendente** manda ao cliente (MX-48).
//!
//! O caminho de entrada ([`crate::tipo::classificar`]) é fechado de propósito: arquivo que chega do
//! cliente vira contexto para o agente, e áudio e vídeo não são lidos por ninguém — recusá-los ali
//! é o certo. O caminho de **saída** é outro problema: o arquivo que o atendente manda pelo Element
//! não é lido, é entregue. Por isso esta lista é separada e continua **fechada**: só os tipos que o
//! WhatsApp entrega, um por um, sem "qualquer coisa com `audio/` no nome".
//!
//! ## O que a lista aceita (e por que cada um)
//!
//! - **imagem**: JPEG, PNG, WebP, GIF — o que o WhatsApp mostra como foto;
//! - **documento**: PDF, Word (.docx), Excel (.xlsx e .xls) e PowerPoint (.pptx) — o que já saía
//!   desde a MX-44;
//! - **áudio** (novo na MX-48): MP3, M4A, OGG/Opus (o formato da gravação de voz do Element), WAV,
//!   AMR, AAC, FLAC e AIFF;
//! - **vídeo** (novo na MX-48): MP4, M4V, QuickTime (.mov), WebM, Matroska (.mkv), AVI, MPEG, WMV e
//!   FLV.
//!
//! Programa, script, compactado, Office antigo (.doc/.ppt), texto e o que o `infer` não reconhece
//! continuam recusados: o tipo é decidido **pelo conteúdo**, e o nome do arquivo só serve para
//! pegar nome enganoso (conteúdo de imagem chamado `.pdf`, por exemplo).
//!
//! ## Áudio e vídeo são grupos irmãos
//!
//! MP4 é contêiner dos dois: um `.m4a` costuma sair como `audio/m4a`, mas um MP4 sem marca de áudio
//! sai como `video/mp4`. Por isso, na conferência do nome, áudio e vídeo contam como o **mesmo**
//! grupo: um arquivo de áudio chamado `.mp4` não é nome enganoso, é contêiner. O que muda entre os
//! dois é só o rótulo do log e o nome padrão quando o evento não traz nome.
//!
//! Nos documentos o grupo é **fino**, como sempre foi na entrada (MX-44): PDF chamado `.docx` é nome
//! enganoso e é recusado. Nas imagens o grupo é o mesmo para os quatro formatos: `.jpg` com conteúdo
//! PNG sempre foi aceito, porque os dois abrem em qualquer visualizador.

use crate::erro::ErroArquivo;
use crate::tipo::{EXTENSOES_BLOQUEADAS, extensao_minuscula, office_pelo_pacote};

/// Grupo a que o arquivo pertence, do ponto de vista de quem entrega ao WhatsApp.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FamiliaEnvio {
    /// Imagem: o WhatsApp mostra como foto.
    Imagem,
    /// Documento: chega como arquivo para baixar.
    Documento,
    /// Áudio: chega como arquivo de áudio.
    Audio,
    /// Vídeo: chega como arquivo de vídeo.
    Video,
}

impl FamiliaEnvio {
    /// Código curto e estável, para log e métrica (sem dado de cliente).
    pub fn codigo(self) -> &'static str {
        match self {
            Self::Imagem => "imagem",
            Self::Documento => "documento",
            Self::Audio => "audio",
            Self::Video => "video",
        }
    }
}

/// Tipo real de um arquivo que vai para o cliente.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TipoEnvio {
    mime: &'static str,
    familia: FamiliaEnvio,
}

impl TipoEnvio {
    /// Tipo MIME real, como o conteúdo o identifica.
    pub fn mime(self) -> &'static str {
        self.mime
    }

    /// Grupo do arquivo.
    pub fn familia(self) -> FamiliaEnvio {
        self.familia
    }

    /// Se o WhatsApp mostra o arquivo como imagem (e não como documento).
    pub fn eh_imagem(self) -> bool {
        self.familia == FamiliaEnvio::Imagem
    }

    /// Código curto do grupo, para log e métrica.
    pub fn codigo(self) -> &'static str {
        self.familia.codigo()
    }

    /// Nome usado quando o evento não traz nome de arquivo: curto, sem acento e sem dado nenhum.
    pub fn nome_padrao(self) -> &'static str {
        match self.familia {
            FamiliaEnvio::Imagem => "imagem",
            FamiliaEnvio::Documento => "arquivo",
            FamiliaEnvio::Audio => "audio",
            FamiliaEnvio::Video => "video",
        }
    }
}

/// Lista fechada: o MIME que o `infer` devolve, um por um, e o grupo de cada um.
const MIMES_ACEITOS: &[(&str, FamiliaEnvio)] = &[
    // Imagem
    ("image/jpeg", FamiliaEnvio::Imagem),
    ("image/png", FamiliaEnvio::Imagem),
    ("image/webp", FamiliaEnvio::Imagem),
    ("image/gif", FamiliaEnvio::Imagem),
    // Documento
    ("application/pdf", FamiliaEnvio::Documento),
    (
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        FamiliaEnvio::Documento,
    ),
    (
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        FamiliaEnvio::Documento,
    ),
    (
        "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        FamiliaEnvio::Documento,
    ),
    ("application/vnd.ms-excel", FamiliaEnvio::Documento),
    // Áudio
    ("audio/mpeg", FamiliaEnvio::Audio),
    ("audio/m4a", FamiliaEnvio::Audio),
    ("audio/ogg", FamiliaEnvio::Audio),
    ("audio/opus", FamiliaEnvio::Audio),
    ("audio/x-wav", FamiliaEnvio::Audio),
    ("audio/amr", FamiliaEnvio::Audio),
    ("audio/aac", FamiliaEnvio::Audio),
    ("audio/x-flac", FamiliaEnvio::Audio),
    ("audio/x-aiff", FamiliaEnvio::Audio),
    // Vídeo
    ("video/mp4", FamiliaEnvio::Video),
    ("video/x-m4v", FamiliaEnvio::Video),
    ("video/quicktime", FamiliaEnvio::Video),
    ("video/webm", FamiliaEnvio::Video),
    ("video/x-matroska", FamiliaEnvio::Video),
    ("video/x-msvideo", FamiliaEnvio::Video),
    ("video/mpeg", FamiliaEnvio::Video),
    ("video/x-ms-wmv", FamiliaEnvio::Video),
    ("video/x-flv", FamiliaEnvio::Video),
];

/// Descobre o tipo real do arquivo que vai ao cliente e recusa o que não está na lista.
///
/// Erros: [`ErroArquivo::TipoNaoAceito`] para programa, script, compactado, formato antigo do
/// Office e o que o conteúdo não identifica; [`ErroArquivo::ExtensaoEnganosa`] quando o nome diz um
/// grupo e o conteúdo é de outro (áudio e vídeo não se acusam entre si: MP4 é contêiner dos dois).
pub fn classificar(bytes: &[u8], nome: &str) -> Result<TipoEnvio, ErroArquivo> {
    let extensao = extensao_minuscula(nome);

    // Nome de programa ou script é recusado mesmo que o conteúdo pareça outra coisa: o cliente
    // receberia um arquivo com nome que o sistema dele executa.
    if let Some(ext) = extensao.as_deref()
        && EXTENSOES_BLOQUEADAS.contains(&ext)
    {
        return Err(ErroArquivo::TipoNaoAceito(format!(
            "o nome termina em .{ext}, que é extensão de programa ou script"
        )));
    }

    let real = detectar(bytes)?;
    if let Some(ext) = extensao.as_deref()
        && let Some(grupo) = grupo_da_extensao(ext)
        && !mesmo_grupo(grupo, grupo_do_tipo(real))
    {
        return Err(ErroArquivo::ExtensaoEnganosa {
            declarada: ext.to_owned(),
            real: real.mime().to_owned(),
        });
    }
    Ok(real)
}

/// Tipo pelo conteúdo, na lista fechada. Sem assinatura conhecida, é recusa: arquivo que vai ao
/// cliente não é adivinhado.
fn detectar(bytes: &[u8]) -> Result<TipoEnvio, ErroArquivo> {
    let Some(inferido) = infer::get(bytes) else {
        return Err(ErroArquivo::TipoNaoAceito(
            "o conteúdo não é um dos tipos que o cliente recebe (imagem, PDF, Office, áudio ou vídeo)"
                .to_owned(),
        ));
    };
    let mut mime = inferido.mime_type();

    // Word, Excel e PowerPoint mínimos saem como ZIP genérico: o nome do pacote decide. ZIP que não
    // é Office continua recusado.
    if mime == "application/zip" {
        let Some(office) = office_pelo_pacote(bytes) else {
            return Err(ErroArquivo::TipoNaoAceito(
                "o conteúdo é um arquivo compactado, que não vai ao cliente".to_owned(),
            ));
        };
        mime = office.mime();
    }

    de_mime(mime).ok_or_else(|| {
        ErroArquivo::TipoNaoAceito(format!(
            "o conteúdo é {mime}, que não é um dos tipos que o cliente recebe"
        ))
    })
}

/// Traduz um MIME da lista fechada; `None` para qualquer outro.
fn de_mime(mime: &str) -> Option<TipoEnvio> {
    MIMES_ACEITOS
        .iter()
        .find(|(aceito, _)| *aceito == mime)
        .map(|(aceito, familia)| TipoEnvio {
            mime: aceito,
            familia: *familia,
        })
}

/// Grupo **fino** usado só na conferência do nome: os quatro formatos de imagem contam como um só
/// (sempre foi assim), e cada família de documento é a sua (PDF chamado de `.docx` é enganoso).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Grupo {
    Imagem,
    Pdf,
    Docx,
    Planilha,
    Pptx,
    Audio,
    Video,
}

/// Grupo que o tipo real declara.
fn grupo_do_tipo(tipo: TipoEnvio) -> Grupo {
    match tipo.familia() {
        FamiliaEnvio::Imagem => Grupo::Imagem,
        FamiliaEnvio::Audio => Grupo::Audio,
        FamiliaEnvio::Video => Grupo::Video,
        FamiliaEnvio::Documento => match tipo.mime() {
            "application/pdf" => Grupo::Pdf,
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document" => {
                Grupo::Docx
            }
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
            | "application/vnd.ms-excel" => Grupo::Planilha,
            _ => Grupo::Pptx,
        },
    }
}

/// Grupo declarado pela extensão do nome, quando ela é de um tipo desta lista.
fn grupo_da_extensao(ext: &str) -> Option<Grupo> {
    match ext {
        "jpg" | "jpeg" | "png" | "webp" | "gif" => Some(Grupo::Imagem),
        "pdf" => Some(Grupo::Pdf),
        "docx" => Some(Grupo::Docx),
        "xlsx" | "xls" => Some(Grupo::Planilha),
        "pptx" => Some(Grupo::Pptx),
        "mp3" | "m4a" | "ogg" | "oga" | "opus" | "wav" | "amr" | "aac" | "flac" | "aiff" => {
            Some(Grupo::Audio)
        }
        "mp4" | "m4v" | "mov" | "webm" | "mkv" | "avi" | "mpg" | "mpeg" | "wmv" | "flv" | "3gp" => {
            Some(Grupo::Video)
        }
        _ => None,
    }
}

/// Os dois grupos são o mesmo para efeito de nome enganoso? Áudio e vídeo são (MP4 é contêiner dos
/// dois); o resto só é igual a si mesmo.
fn mesmo_grupo(declarado: Grupo, real: Grupo) -> bool {
    if declarado == real {
        return true;
    }
    matches!(
        (declarado, real),
        (Grupo::Audio, Grupo::Video) | (Grupo::Video, Grupo::Audio)
    )
}

#[cfg(test)]
mod testes {
    use super::*;

    /// PNG 1×1: imagem de verdade, sem dado de ninguém.
    const PNG: &[u8] = &[
        0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f,
        0x15, 0xc4, 0x89, 0x00, 0x00, 0x00, 0x0a, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0x00,
        0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x0d, 0x0a, 0x2d, 0xb4, 0x00, 0x00, 0x00, 0x00, 0x49,
        0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
    ];

    /// PDF mínimo com a assinatura que todo leitor reconhece.
    const PDF: &[u8] =
        b"%PDF-1.4\n1 0 obj\n<< /Type /Catalog >>\nendobj\ntrailer\n<< /Root 1 0 R >>\n%%EOF\n";

    /// OGG/Opus mínimo: o cabeçalho `OpusHead` dentro do contêiner OGG, como a gravação de voz do
    /// Element manda.
    fn ogg_opus() -> Vec<u8> {
        let mut bytes = b"OggS".to_vec();
        bytes.extend_from_slice(&[0x00, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]);
        bytes.extend_from_slice(&[0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00]);
        bytes.extend_from_slice(b"OpusHead");
        bytes.extend_from_slice(&[0x01, 0x02, 0x38, 0x01, 0x80, 0xbb, 0x00, 0x00]);
        bytes
    }

    /// MP4 mínimo: caixa `ftyp` com a marca `isom`.
    fn mp4() -> Vec<u8> {
        let mut bytes = vec![0x00, 0x00, 0x00, 0x18];
        bytes.extend_from_slice(b"ftypisom");
        bytes.extend_from_slice(&[0x00, 0x00, 0x02, 0x00]);
        bytes.extend_from_slice(b"isomiso2");
        bytes
    }

    #[test]
    fn imagem_pdf_e_office_continuam_como_antes() {
        let png = classificar(PNG, "foto.png").expect("PNG é imagem");
        assert_eq!(png.mime(), "image/png");
        assert_eq!(png.familia(), FamiliaEnvio::Imagem);
        assert!(png.eh_imagem());
        assert_eq!(png.nome_padrao(), "imagem");

        let pdf = classificar(PDF, "laudo.pdf").expect("PDF é documento");
        assert_eq!(pdf.mime(), "application/pdf");
        assert_eq!(pdf.familia(), FamiliaEnvio::Documento);
        assert!(!pdf.eh_imagem());
        assert_eq!(pdf.nome_padrao(), "arquivo");
    }

    #[test]
    fn audio_e_video_entram_na_lista_com_o_mime_real() {
        let audio = classificar(&ogg_opus(), "voice-message.ogg").expect("OGG/Opus é áudio");
        assert_eq!(audio.familia(), FamiliaEnvio::Audio);
        assert_eq!(
            audio.mime(),
            "audio/ogg",
            "o áudio do Element é contêiner OGG"
        );
        assert_eq!(audio.codigo(), "audio");
        assert_eq!(audio.nome_padrao(), "audio");

        let video = classificar(&mp4(), "video.mp4").expect("MP4 é vídeo");
        assert_eq!(video.mime(), "video/mp4");
        assert_eq!(video.familia(), FamiliaEnvio::Video);
        assert_eq!(video.codigo(), "video");
        assert_eq!(video.nome_padrao(), "video");
    }

    #[test]
    fn audio_chamado_mp4_nao_e_nome_enganoso() {
        // MP4 é contêiner dos dois: um arquivo de áudio com nome `.mp4` não é enganoso.
        let audio = classificar(&ogg_opus(), "gravacao.mp4")
            .expect("áudio e vídeo são o mesmo grupo para nome enganoso");
        assert_eq!(audio.familia(), FamiliaEnvio::Audio);
    }

    #[test]
    fn conteudo_recusado_continua_recusado() {
        // ZIP de verdade (compactado, não Office).
        let zip = b"PK\x03\x04\x14\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00";
        assert!(matches!(
            classificar(zip, "pacote.zip"),
            Err(ErroArquivo::TipoNaoAceito(_))
        ));
        // Texto simples: o conteúdo não identifica tipo nenhum.
        assert!(matches!(
            classificar(b"apenas um texto qualquer\n", "notas.txt"),
            Err(ErroArquivo::TipoNaoAceito(_))
        ));
        // Nome de script é recusado mesmo que o conteúdo seja outra coisa.
        assert!(matches!(
            classificar(PNG, "abrir.sh"),
            Err(ErroArquivo::TipoNaoAceito(_))
        ));
    }

    #[test]
    fn nome_enganoso_e_recusado_quando_o_conteudo_e_de_outro_grupo() {
        let erro = classificar(PNG, "laudo.pdf").expect_err("imagem chamada de PDF");
        assert!(
            matches!(erro, ErroArquivo::ExtensaoEnganosa { .. }),
            "{erro:?}"
        );
        let erro = classificar(PDF, "foto.png").expect_err("PDF chamado de imagem");
        assert!(
            matches!(erro, ErroArquivo::ExtensaoEnganosa { .. }),
            "{erro:?}"
        );
        // Áudio com nome de documento também é enganoso.
        let erro = classificar(&ogg_opus(), "laudo.pdf").expect_err("áudio chamado de PDF");
        assert!(
            matches!(erro, ErroArquivo::ExtensaoEnganosa { .. }),
            "{erro:?}"
        );
        // Documento contra documento continua fino, como na MX-44: PDF chamado `.docx` é enganoso.
        let erro = classificar(PDF, "contrato.docx").expect_err("PDF chamado de Word");
        assert!(
            matches!(erro, ErroArquivo::ExtensaoEnganosa { .. }),
            "{erro:?}"
        );
    }

    #[test]
    fn a_lista_e_fechada_e_nao_aceita_familia_por_prefixo() {
        // Nenhum MIME fora da lista entra: o casamento é por igualdade, não por prefixo.
        assert!(de_mime("audio/midi").is_none());
        assert!(de_mime("video/x-ms-wmv-extra").is_none());
        assert!(de_mime("application/zip").is_none());
        assert!(de_mime("application/octet-stream").is_none());
        assert!(de_mime("audio/opus").is_some());
    }
}
