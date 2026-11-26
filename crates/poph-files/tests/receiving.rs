use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use poph_files::envelope::{UNTRUSTED_CLOSE, UNTRUSTED_NOTICE, UNTRUSTED_OPEN};
use poph_files::{Content, FileConfig, FileError, FilePart, Tipo, receive};
use poph_types::OrgId;
use tempfile::TempDir;

struct Environment {
    _folder: TempDir,
    blobs: PathBuf,
    config: FileConfig,
}

fn environment() -> Environment {
    let folder = tempfile::tempdir().expect("pasta temporária");
    let blobs = folder.path().join("blobs");
    let mut config = FileConfig::default(&blobs);
    config.extractor_path = Some(PathBuf::from(env!("CARGO_BIN_EXE_poph-extrator")));
    Environment {
        _folder: folder,
        blobs,
        config,
    }
}

fn fixture(nome: &str) -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(nome);
    std::fs::read(&path).unwrap_or_else(|error| panic!("fixture {nome}: {error}"))
}

fn text_of_parts(parts: &[FilePart]) -> String {
    parts
        .iter()
        .filter_map(|part| match part {
            FilePart::Texto(texto) => Some(texto.as_str()),
            FilePart::Image { .. } => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn docx_with_text(texto: &str) -> Vec<u8> {
    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>{texto}</w:t></w:r></w:p></w:body></w:document>"#
    );
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default();
    writer.start_file("word/document.xml", options).unwrap();
    writer.write_all(xml.as_bytes()).unwrap();
    writer.finish().unwrap().into_inner()
}

#[test]
fn supported_types_become_text_or_image() {
    let amb = environment();
    let org = OrgId::generate();
    let cases: &[(&str, &str, &str)] = &[
        ("texto.pdf", "Exemplo", "pdf"),
        ("tabela.pdf", "Fipronil", "pdf"),
        ("procedimento.docx", "Sintoma", "docx"),
        ("treinamento.pptx", "Treinamento de atendimento", "pptx"),
        ("produtos.xlsx", "Exemplo 200 SC", "xlsx"),
        ("produtos.xls", "Exemplo 200 SC", "xls"),
    ];
    for (nome, expected, _) in cases {
        let received = receive(&amb.config, org, nome, &fixture(nome))
            .unwrap_or_else(|error| panic!("{nome} deveria ser aceito: {error}"));
        let parts = received.for_llm();
        let texto = text_of_parts(&parts);
        assert!(
            texto.contains(expected),
            "{nome}: não achei {expected:?} em {texto:?}"
        );
    }
    let procedimento = receive(
        &amb.config,
        org,
        "procedimento.docx",
        &fixture("procedimento.docx"),
    )
    .unwrap();
    let texto = text_of_parts(&procedimento.for_llm());
    assert!(texto.contains("pronto-socorro & informe"), "{texto:?}");
    assert!(texto.contains("Náusea\tHidratação"), "{texto:?}");
}

#[test]
fn images_become_multimodal_parts() {
    let amb = environment();
    let org = OrgId::generate();
    let jpeg = [&[0xFF, 0xD8, 0xFF, 0xE0][..], &[0u8; 64][..]].concat();
    let png = [&b"\x89PNG\r\n\x1a\n"[..], &[0u8; 64][..]].concat();
    let gif = [&b"GIF89a"[..], &[0u8; 64][..]].concat();
    let webp = [&b"RIFF"[..], &[0u8; 4][..], &b"WEBP"[..], &[0u8; 64][..]].concat();
    let cases = [
        ("foto.jpg", jpeg, "image/jpeg"),
        ("print.png", png, "image/png"),
        ("anim.gif", gif, "image/gif"),
        ("foto.webp", webp, "image/webp"),
    ];
    for (nome, bytes, expected_mime) in cases {
        let received = receive(&amb.config, org, nome, &bytes).unwrap();
        assert_eq!(received.metadados.mime_real, expected_mime);
        let parts = received.for_llm();
        assert_eq!(parts.len(), 1);
        match &parts[0] {
            FilePart::Image { mime, bytes: sent } => {
                assert_eq!(mime, expected_mime);
                assert_eq!(sent, &bytes);
            }
            other_part => panic!("{nome}: esperava imagem, veio {other_part:?}"),
        }
    }
}

#[test]
fn spreadsheet_with_swapped_xls_xlsx_name_is_accepted() {
    let amb = environment();
    let org = OrgId::generate();
    let texto = text_of_parts(
        &receive(
            &amb.config,
            org,
            "tabela-antiga.xlsx",
            &fixture("produtos.xls"),
        )
        .unwrap()
        .for_llm(),
    );
    assert!(texto.contains("Exemplo 200 SC"), "{texto:?}");
}

#[test]
fn old_doc_and_ppt_are_refused_with_polite_message() {
    let amb = environment();
    let org = OrgId::generate();
    for nome in ["injecao.doc", "apresentacao.ppt"] {
        let error = receive(&amb.config, org, nome, &fixture(nome)).unwrap_err();
        match error {
            FileError::UnsupportedType(mensagem) => {
                assert!(
                    mensagem.contains(".docx") || mensagem.contains(".pptx"),
                    "{mensagem}"
                );
            }
            other => panic!("{nome}: esperava recusa, veio {other:?}"),
        }
    }
}

#[test]
fn audio_is_refused() {
    let amb = environment();
    let org = OrgId::generate();
    let mut audio = b"ID3\x03\x00\x00\x00\x00\x00\x00".to_vec();
    audio.extend_from_slice(&[0u8; 256]);
    let error = receive(&amb.config, org, "nota-de-voz.mp3", &audio).unwrap_err();
    assert!(
        matches!(&error, FileError::UnsupportedType(m) if m.contains("áudio")),
        "{error:?}"
    );
}

#[test]
fn executable_named_pdf_is_misleading_extension() {
    let amb = environment();
    let org = OrgId::generate();
    let mut elf = b"\x7fELF\x02\x01\x01\x00".to_vec();
    elf.extend_from_slice(&[0u8; 256]);
    let error = receive(&amb.config, org, "contrato.pdf", &elf).unwrap_err();
    match error {
        FileError::MisleadingExtension {
            declared: declared_ext,
            real,
        } => {
            assert_eq!(declared_ext, "pdf");
            assert!(real.contains("executável"), "{real}");
        }
        other => panic!("esperava extensão enganosa, veio {other:?}"),
    }
}

#[test]
fn pdf_named_docx_is_misleading_extension() {
    let amb = environment();
    let org = OrgId::generate();
    let error = receive(&amb.config, org, "relatorio.docx", &fixture("texto.pdf")).unwrap_err();
    assert!(
        matches!(error, FileError::MisleadingExtension { ref declared, .. } if declared == "docx"),
        "{error:?}"
    );
}

#[test]
fn program_extension_is_refused_even_with_pdf_content() {
    let amb = environment();
    let org = OrgId::generate();
    let error = receive(&amb.config, org, "instalar.exe", &fixture("texto.pdf")).unwrap_err();
    assert!(
        matches!(&error, FileError::UnsupportedType(m) if m.contains(".exe")),
        "{error:?}"
    );
}

#[test]
fn script_disguised_as_text_is_refused() {
    let amb = environment();
    let org = OrgId::generate();
    let error = receive(&amb.config, org, "nota.txt", b"#!/bin/sh\necho oi\n").unwrap_err();
    assert!(
        matches!(&error, FileError::UnsupportedType(m) if m.contains("script")),
        "{error:?}"
    );
}

#[test]
fn file_over_16_mb_is_refused_without_writing() {
    let amb = environment();
    let org = OrgId::generate();
    let large = vec![0u8; 16 * 1024 * 1024 + 1];
    let error = receive(&amb.config, org, "grande.pdf", &large).unwrap_err();
    assert!(matches!(error, FileError::Large { .. }), "{error:?}");
    assert!(!amb.blobs.exists(), "nada pode ter sido gravado");
}

#[test]
fn file_at_exact_limit_passes_size_check() {
    let amb = environment();
    let org = OrgId::generate();
    let at_limit = vec![b'a'; 16 * 1024 * 1024];
    let error = receive(&amb.config, org, "nota.txt", &at_limit).unwrap_err();
    assert!(matches!(error, FileError::UnsupportedType(_)), "{error:?}");
}

#[test]
fn empty_file_is_refused() {
    let amb = environment();
    let org = OrgId::generate();
    assert_eq!(
        receive(&amb.config, org, "vazio.pdf", &[]).unwrap_err(),
        FileError::Vazio
    );
}

#[test]
fn zip_bomb_is_refused_fast_and_writes_nothing() {
    let amb = environment();
    let org = OrgId::generate();
    let started = Instant::now();
    let error = receive(&amb.config, org, "bomba.docx", &fixture("bomba.docx")).unwrap_err();
    let elapsed = started.elapsed();
    assert!(
        matches!(&error, FileError::Reading(m) if m.contains("32 MB")),
        "{error:?}"
    );
    assert!(elapsed < Duration::from_secs(3), "demorou {elapsed:?}");
    assert!(!amb.blobs.exists(), "nada pode ter sido gravado");
}

#[test]
fn malicious_pdf_does_not_crash_process_and_respects_limit() {
    let amb = environment();
    let org = OrgId::generate();
    let started = Instant::now();
    let error = receive(&amb.config, org, "bomba.pdf", &fixture("bomba.pdf")).unwrap_err();
    let elapsed = started.elapsed();
    assert!(
        matches!(error, FileError::Reading(_) | FileError::Time { .. }),
        "{error:?}"
    );
    assert!(elapsed < Duration::from_secs(12), "demorou {elapsed:?}");
    assert!(!amb.blobs.exists(), "nada pode ter sido gravado");
}

#[test]
fn reader_over_memory_ceiling_is_interrupted() {
    let mut amb = environment();
    amb.config.extraction_time = Duration::from_secs(60);
    let org = OrgId::generate();
    let error = receive(&amb.config, org, "bomba.pdf", &fixture("bomba.pdf")).unwrap_err();
    assert!(
        matches!(&error, FileError::Reading(m) if m.contains("parou")),
        "{error:?}"
    );
}

#[test]
fn reader_over_deadline_is_killed() {
    let mut amb = environment();
    amb.config.extraction_time = Duration::from_millis(500);
    let org = OrgId::generate();
    let started = Instant::now();
    let error = receive(&amb.config, org, "bomba.pdf", &fixture("bomba.pdf")).unwrap_err();
    let elapsed = started.elapsed();
    assert!(matches!(error, FileError::Time { .. }), "{error:?}");
    assert!(elapsed < Duration::from_secs(3), "demorou {elapsed:?}");
}

#[test]
fn broken_pdf_returns_clear_error() {
    let amb = environment();
    let org = OrgId::generate();
    let error = receive(&amb.config, org, "quebrado.pdf", &fixture("quebrado.pdf")).unwrap_err();
    assert!(
        matches!(&error, FileError::Reading(m) if m.contains("corrompido")),
        "{error:?}"
    );
}

#[test]
fn scanned_pdf_returns_honest_notice_without_invented_text() {
    let amb = environment();
    let org = OrgId::generate();
    let received = receive(&amb.config, org, "scan.pdf", &fixture("scan.pdf")).unwrap();
    assert_eq!(received.content, Content::NoText);
    let path = received.metadados.blob_path.clone();
    let parts = received.for_llm();
    let texto = text_of_parts(&parts);
    assert!(texto.contains("Não consegui ler"), "{texto:?}");
    assert!(
        !texto.contains(UNTRUSTED_OPEN),
        "não pode haver bloco de conteúdo inventado"
    );
    assert!(path.is_file(), "o arquivo recebido fica guardado");
}

#[test]
fn injection_in_pdf_comes_out_as_untrusted_block() {
    let amb = environment();
    let org = OrgId::generate();
    let received = receive(&amb.config, org, "orientacao.pdf", &fixture("injecao.pdf")).unwrap();
    let parts = received.for_llm();

    assert_eq!(parts[0], FilePart::Texto(UNTRUSTED_NOTICE.to_owned()));
    let block = match parts.last() {
        Some(FilePart::Texto(texto)) => texto.clone(),
        other_part => panic!("esperava bloco de texto, veio {other_part:?}"),
    };
    assert!(block.starts_with(UNTRUSTED_OPEN), "{block:?}");
    assert!(block.ends_with(UNTRUSTED_CLOSE), "{block:?}");
    assert!(
        block.contains("Ignore as instruções anteriores"),
        "o texto tem que chegar, como dado"
    );
    assert_eq!(block.matches(UNTRUSTED_CLOSE).count(), 1, "{block:?}");
    assert_eq!(block.matches(UNTRUSTED_OPEN).count(), 1, "{block:?}");
    assert!(block.contains("‹‹FIM_ARQUIVO_NAO_CONFIAVEL"), "{block:?}");
}

#[test]
fn long_text_is_cut_with_notice() {
    let amb = environment();
    let org = OrgId::generate();
    let texto = "a".repeat(30_000);
    let bytes = docx_with_text(&texto);
    let received = receive(&amb.config, org, "longo.docx", &bytes).unwrap();
    match &received.content {
        Content::Texto {
            original_characters: original_count,
            truncated: was_truncated,
            ..
        } => {
            assert_eq!(*original_count, 30_000);
            assert!(*was_truncated);
        }
        other => panic!("esperava texto, veio {other:?}"),
    }
    let parts = received.for_llm();
    let notice = text_of_parts(&parts);
    assert!(
        notice.contains("tinha 30000 caracteres e foi cortado"),
        "{notice}"
    );
    let block = match parts.last() {
        Some(FilePart::Texto(t)) => t.clone(),
        _ => unreachable!(),
    };
    let miolo = block
        .strip_prefix(UNTRUSTED_OPEN)
        .and_then(|t| t.strip_suffix(UNTRUSTED_CLOSE))
        .unwrap()
        .trim();
    assert_eq!(miolo.chars().count(), 20_000);
}

#[test]
fn same_file_twice_makes_one_blob() {
    let amb = environment();
    let org = OrgId::generate();
    let bytes = fixture("texto.pdf");
    let first = receive(&amb.config, org, "a.pdf", &bytes).unwrap();
    let second = receive(&amb.config, org, "b.pdf", &bytes).unwrap();
    assert!(!first.metadados.blob_already_existed);
    assert!(second.metadados.blob_already_existed);
    assert_eq!(first.metadados.blob_path, second.metadados.blob_path);
    assert_eq!(first.metadados.sha256, second.metadados.sha256);

    let org_folder = amb.blobs.join(org.to_string());
    let files: Vec<_> = std::fs::read_dir(&org_folder)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(files.len(), 1, "{files:?}");
    assert_eq!(std::fs::read(&first.metadados.blob_path).unwrap(), bytes);
    assert_eq!(first.metadados.blob_path.file_name().unwrap().len(), 64);
}

#[test]
fn different_orgs_store_blobs_in_separate_folders() {
    let amb = environment();
    let org_a = OrgId::generate();
    let org_b = OrgId::generate();
    let bytes = fixture("texto.pdf");
    let a = receive(&amb.config, org_a, "x.pdf", &bytes).unwrap();
    let b = receive(&amb.config, org_b, "x.pdf", &bytes).unwrap();
    assert!(
        a.metadados
            .blob_path
            .starts_with(amb.blobs.join(org_a.to_string()))
    );
    assert!(
        b.metadados
            .blob_path
            .starts_with(amb.blobs.join(org_b.to_string()))
    );
    assert!(!b.metadados.blob_already_existed);
}

#[test]
fn name_with_folder_does_not_escape_org_directory() {
    let amb = environment();
    let org = OrgId::generate();
    let received = receive(
        &amb.config,
        org,
        "../../../etc/contrato.pdf",
        &fixture("texto.pdf"),
    )
    .unwrap();
    assert_eq!(received.metadados.original_name, "contrato.pdf");
    assert!(
        received
            .metadados
            .blob_path
            .starts_with(amb.blobs.join(org.to_string()))
    );
}

#[test]
fn invalid_config_is_refused() {
    let mut amb = environment();
    amb.config.byte_limit = 0;
    let org = OrgId::generate();
    let error = receive(&amb.config, org, "a.pdf", b"x").unwrap_err();
    assert!(matches!(error, FileError::Configuracao(_)), "{error:?}");
}

#[test]
fn missing_reader_returns_clear_error() {
    let mut amb = environment();
    amb.config.extractor_path = Some(amb.blobs.join("nao-existe"));
    let org = OrgId::generate();
    let error = receive(&amb.config, org, "a.pdf", &fixture("texto.pdf")).unwrap_err();
    assert!(matches!(error, FileError::Extrator(_)), "{error:?}");
    assert!(!amb.blobs.exists());
}

#[test]
fn real_type_is_what_the_content_says() {
    assert_eq!(
        poph_files::classify(&fixture("texto.pdf"), "qualquer.bin").unwrap(),
        Tipo::Pdf
    );
}
