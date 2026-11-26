//! Testes de ponta a ponta do recebimento de arquivos (HR-23), com fixturas próprias.
//!
//! Usam o programa `poph-extrator` de verdade (processo filho), como em produção.

use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use poph_files::envelope::{UNTRUSTED_CLOSE, UNTRUSTED_NOTICE, UNTRUSTED_OPEN};
use poph_files::{Content, FileConfig, FileError, FilePart, Tipo, receive};
use poph_types::OrgId;
use tempfile::TempDir;

struct Ambiente {
    _pasta: TempDir,
    blobs: PathBuf,
    config: FileConfig,
}

fn ambiente() -> Ambiente {
    let pasta = tempfile::tempdir().expect("pasta temporária");
    let blobs = pasta.path().join("blobs");
    let mut config = FileConfig::default(&blobs);
    config.extractor_path = Some(PathBuf::from(env!("CARGO_BIN_EXE_poph-extrator")));
    Ambiente {
        _pasta: pasta,
        blobs,
        config,
    }
}

fn fixture(nome: &str) -> Vec<u8> {
    let caminho = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(nome);
    std::fs::read(&caminho).unwrap_or_else(|erro| panic!("fixture {nome}: {erro}"))
}

fn texto_das_partes(partes: &[FilePart]) -> String {
    partes
        .iter()
        .filter_map(|parte| match parte {
            FilePart::Texto(texto) => Some(texto.as_str()),
            FilePart::Image { .. } => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn docx_com_texto(texto: &str) -> Vec<u8> {
    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>{texto}</w:t></w:r></w:p></w:body></w:document>"#
    );
    let mut escritor = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let opcoes = zip::write::SimpleFileOptions::default();
    escritor.start_file("word/document.xml", opcoes).unwrap();
    escritor.write_all(xml.as_bytes()).unwrap();
    escritor.finish().unwrap().into_inner()
}

// ---------- tipos suportados ----------

#[test]
fn tipos_suportados_viram_texto_ou_imagem() {
    let amb = ambiente();
    let org = OrgId::generate();
    let casos: &[(&str, &str, &str)] = &[
        ("texto.pdf", "Exemplo", "pdf"),
        ("tabela.pdf", "Fipronil", "pdf"),
        ("procedimento.docx", "Sintoma", "docx"),
        ("treinamento.pptx", "Treinamento de atendimento", "pptx"),
        ("produtos.xlsx", "Exemplo 200 SC", "xlsx"),
        ("produtos.xls", "Exemplo 200 SC", "xls"),
    ];
    for (nome, esperado, _) in casos {
        let recebido = receive(&amb.config, org, nome, &fixture(nome))
            .unwrap_or_else(|erro| panic!("{nome} deveria ser aceito: {erro}"));
        let partes = recebido.for_llm();
        let texto = texto_das_partes(&partes);
        assert!(
            texto.contains(esperado),
            "{nome}: não achei {esperado:?} em {texto:?}"
        );
    }
    // Procedimento tem entidade XML (&amp;) e tabela: tem que sair como texto legível.
    let procedimento = receive(
        &amb.config,
        org,
        "procedimento.docx",
        &fixture("procedimento.docx"),
    )
    .unwrap();
    let texto = texto_das_partes(&procedimento.for_llm());
    assert!(texto.contains("pronto-socorro & informe"), "{texto:?}");
    assert!(texto.contains("Náusea\tHidratação"), "{texto:?}");
}

#[test]
fn imagens_viram_partes_multimodais() {
    let amb = ambiente();
    let org = OrgId::generate();
    let jpeg = [&[0xFF, 0xD8, 0xFF, 0xE0][..], &[0u8; 64][..]].concat();
    let png = [&b"\x89PNG\r\n\x1a\n"[..], &[0u8; 64][..]].concat();
    let gif = [&b"GIF89a"[..], &[0u8; 64][..]].concat();
    let webp = [&b"RIFF"[..], &[0u8; 4][..], &b"WEBP"[..], &[0u8; 64][..]].concat();
    let casos = [
        ("foto.jpg", jpeg, "image/jpeg"),
        ("print.png", png, "image/png"),
        ("anim.gif", gif, "image/gif"),
        ("foto.webp", webp, "image/webp"),
    ];
    for (nome, bytes, mime_esperado) in casos {
        let recebido = receive(&amb.config, org, nome, &bytes).unwrap();
        assert_eq!(recebido.metadados.mime_real, mime_esperado);
        let partes = recebido.for_llm();
        assert_eq!(partes.len(), 1);
        match &partes[0] {
            FilePart::Image {
                mime,
                bytes: enviados,
            } => {
                assert_eq!(mime, mime_esperado);
                assert_eq!(enviados, &bytes);
            }
            outra => panic!("{nome}: esperava imagem, veio {outra:?}"),
        }
    }
}

// ---------- recusas ----------

#[test]
fn planilha_com_nome_trocado_entre_xls_e_xlsx_e_aceita() {
    let amb = ambiente();
    let org = OrgId::generate();
    let texto = texto_das_partes(
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
fn doc_e_ppt_antigos_sao_recusados_com_mensagem_educada() {
    let amb = ambiente();
    let org = OrgId::generate();
    for nome in ["injecao.doc", "apresentacao.ppt"] {
        let erro = receive(&amb.config, org, nome, &fixture(nome)).unwrap_err();
        match erro {
            FileError::UnsupportedType(mensagem) => {
                assert!(
                    mensagem.contains(".docx") || mensagem.contains(".pptx"),
                    "{mensagem}"
                );
            }
            outro => panic!("{nome}: esperava recusa, veio {outro:?}"),
        }
    }
}

#[test]
fn audio_e_recusado() {
    let amb = ambiente();
    let org = OrgId::generate();
    let mut audio = b"ID3\x03\x00\x00\x00\x00\x00\x00".to_vec();
    audio.extend_from_slice(&[0u8; 256]);
    let erro = receive(&amb.config, org, "nota-de-voz.mp3", &audio).unwrap_err();
    assert!(
        matches!(&erro, FileError::UnsupportedType(m) if m.contains("áudio")),
        "{erro:?}"
    );
}

#[test]
fn executavel_com_nome_de_pdf_e_extensao_enganosa() {
    let amb = ambiente();
    let org = OrgId::generate();
    let mut elf = b"\x7fELF\x02\x01\x01\x00".to_vec();
    elf.extend_from_slice(&[0u8; 256]);
    let erro = receive(&amb.config, org, "contrato.pdf", &elf).unwrap_err();
    match erro {
        FileError::MisleadingExtension {
            declared: declarada,
            real,
        } => {
            assert_eq!(declarada, "pdf");
            assert!(real.contains("executável"), "{real}");
        }
        outro => panic!("esperava extensão enganosa, veio {outro:?}"),
    }
}

#[test]
fn pdf_com_nome_de_docx_e_extensao_enganosa() {
    let amb = ambiente();
    let org = OrgId::generate();
    let erro = receive(&amb.config, org, "relatorio.docx", &fixture("texto.pdf")).unwrap_err();
    assert!(
        matches!(erro, FileError::MisleadingExtension { ref declared, .. } if declared == "docx"),
        "{erro:?}"
    );
}

#[test]
fn extensao_de_programa_e_recusada_mesmo_com_conteudo_de_pdf() {
    let amb = ambiente();
    let org = OrgId::generate();
    let erro = receive(&amb.config, org, "instalar.exe", &fixture("texto.pdf")).unwrap_err();
    assert!(
        matches!(&erro, FileError::UnsupportedType(m) if m.contains(".exe")),
        "{erro:?}"
    );
}

#[test]
fn script_disfarcado_de_texto_e_recusado() {
    let amb = ambiente();
    let org = OrgId::generate();
    let erro = receive(&amb.config, org, "nota.txt", b"#!/bin/sh\necho oi\n").unwrap_err();
    assert!(
        matches!(&erro, FileError::UnsupportedType(m) if m.contains("script")),
        "{erro:?}"
    );
}

#[test]
fn arquivo_acima_de_16_mb_e_recusado_sem_gravar() {
    let amb = ambiente();
    let org = OrgId::generate();
    let grande = vec![0u8; 16 * 1024 * 1024 + 1];
    let erro = receive(&amb.config, org, "grande.pdf", &grande).unwrap_err();
    assert!(matches!(erro, FileError::Large { .. }), "{erro:?}");
    assert!(!amb.blobs.exists(), "nada pode ter sido gravado");
}

#[test]
fn arquivo_no_limite_exato_e_aceito_pelo_tamanho() {
    let amb = ambiente();
    let org = OrgId::generate();
    // Exatamente 16 MB: passa do tamanho; o tipo (texto puro) é que recusa depois.
    let no_limite = vec![b'a'; 16 * 1024 * 1024];
    let erro = receive(&amb.config, org, "nota.txt", &no_limite).unwrap_err();
    assert!(matches!(erro, FileError::UnsupportedType(_)), "{erro:?}");
}

#[test]
fn arquivo_vazio_e_recusado() {
    let amb = ambiente();
    let org = OrgId::generate();
    assert_eq!(
        receive(&amb.config, org, "vazio.pdf", &[]).unwrap_err(),
        FileError::Vazio
    );
}

// ---------- leitura isolada: bombas e falhas ----------

#[test]
fn zip_bomba_e_recusada_rapido_e_sem_gravar() {
    let amb = ambiente();
    let org = OrgId::generate();
    let inicio = Instant::now();
    let erro = receive(&amb.config, org, "bomba.docx", &fixture("bomba.docx")).unwrap_err();
    let decorrido = inicio.elapsed();
    assert!(
        matches!(&erro, FileError::Reading(m) if m.contains("32 MB")),
        "{erro:?}"
    );
    assert!(decorrido < Duration::from_secs(3), "demorou {decorrido:?}");
    assert!(!amb.blobs.exists(), "nada pode ter sido gravado");
}

#[test]
fn pdf_malicioso_nao_derruba_o_processo_e_respeita_o_limite() {
    let amb = ambiente();
    let org = OrgId::generate();
    let inicio = Instant::now();
    let erro = receive(&amb.config, org, "bomba.pdf", &fixture("bomba.pdf")).unwrap_err();
    let decorrido = inicio.elapsed();
    // Ou o sistema corta por memória (leitor parou) ou o prazo de tempo corta primeiro.
    assert!(
        matches!(erro, FileError::Reading(_) | FileError::Time { .. }),
        "{erro:?}"
    );
    assert!(decorrido < Duration::from_secs(12), "demorou {decorrido:?}");
    assert!(!amb.blobs.exists(), "nada pode ter sido gravado");
}

#[test]
fn leitor_que_passa_do_teto_de_memoria_e_interrompido() {
    let mut amb = ambiente();
    amb.config.extraction_time = Duration::from_secs(60);
    let org = OrgId::generate();
    let erro = receive(&amb.config, org, "bomba.pdf", &fixture("bomba.pdf")).unwrap_err();
    assert!(
        matches!(&erro, FileError::Reading(m) if m.contains("parou")),
        "{erro:?}"
    );
}

#[test]
fn leitor_que_passa_do_prazo_e_morto() {
    let mut amb = ambiente();
    amb.config.extraction_time = Duration::from_millis(500);
    let org = OrgId::generate();
    let inicio = Instant::now();
    let erro = receive(&amb.config, org, "bomba.pdf", &fixture("bomba.pdf")).unwrap_err();
    let decorrido = inicio.elapsed();
    assert!(matches!(erro, FileError::Time { .. }), "{erro:?}");
    assert!(decorrido < Duration::from_secs(3), "demorou {decorrido:?}");
}

#[test]
fn pdf_quebrado_devolve_erro_claro() {
    let amb = ambiente();
    let org = OrgId::generate();
    let erro = receive(&amb.config, org, "quebrado.pdf", &fixture("quebrado.pdf")).unwrap_err();
    assert!(
        matches!(&erro, FileError::Reading(m) if m.contains("corrompido")),
        "{erro:?}"
    );
}

#[test]
fn pdf_escaneado_devolve_aviso_honesto_sem_texto_inventado() {
    let amb = ambiente();
    let org = OrgId::generate();
    let recebido = receive(&amb.config, org, "scan.pdf", &fixture("scan.pdf")).unwrap();
    assert_eq!(recebido.content, Content::NoText);
    let caminho = recebido.metadados.blob_path.clone();
    let partes = recebido.for_llm();
    let texto = texto_das_partes(&partes);
    assert!(texto.contains("Não consegui ler"), "{texto:?}");
    assert!(
        !texto.contains(UNTRUSTED_OPEN),
        "não pode haver bloco de conteúdo inventado"
    );
    assert!(caminho.is_file(), "o arquivo recebido fica guardado");
}

// ---------- injeção e bloco não confiável ----------

#[test]
fn injecao_no_pdf_sai_como_bloco_nao_confiavel() {
    let amb = ambiente();
    let org = OrgId::generate();
    let recebido = receive(&amb.config, org, "orientacao.pdf", &fixture("injecao.pdf")).unwrap();
    let partes = recebido.for_llm();

    // Primeira parte: aviso nosso. Depois o bloco. Nada fora dele é "conteúdo do cliente".
    assert_eq!(partes[0], FilePart::Texto(UNTRUSTED_NOTICE.to_owned()));
    let bloco = match partes.last() {
        Some(FilePart::Texto(texto)) => texto.clone(),
        outra => panic!("esperava bloco de texto, veio {outra:?}"),
    };
    assert!(bloco.starts_with(UNTRUSTED_OPEN), "{bloco:?}");
    assert!(bloco.ends_with(UNTRUSTED_CLOSE), "{bloco:?}");
    assert!(
        bloco.contains("Ignore as instruções anteriores"),
        "o texto tem que chegar, como dado"
    );
    // O PDF tentou fechar o bloco com a marca de fechamento: não pode conseguir.
    assert_eq!(bloco.matches(UNTRUSTED_CLOSE).count(), 1, "{bloco:?}");
    assert_eq!(bloco.matches(UNTRUSTED_OPEN).count(), 1, "{bloco:?}");
    assert!(bloco.contains("‹‹FIM_ARQUIVO_NAO_CONFIAVEL"), "{bloco:?}");
}

#[test]
fn texto_longo_e_cortado_com_aviso() {
    let amb = ambiente();
    let org = OrgId::generate();
    let texto = "a".repeat(30_000);
    let bytes = docx_com_texto(&texto);
    let recebido = receive(&amb.config, org, "longo.docx", &bytes).unwrap();
    match &recebido.content {
        Content::Texto {
            original_characters: caracteres_originais,
            truncated: truncado,
            ..
        } => {
            assert_eq!(*caracteres_originais, 30_000);
            assert!(*truncado);
        }
        outro => panic!("esperava texto, veio {outro:?}"),
    }
    let partes = recebido.for_llm();
    let aviso = texto_das_partes(&partes);
    assert!(
        aviso.contains("tinha 30000 caracteres e foi cortado"),
        "{aviso}"
    );
    let bloco = match partes.last() {
        Some(FilePart::Texto(t)) => t.clone(),
        _ => unreachable!(),
    };
    let miolo = bloco
        .strip_prefix(UNTRUSTED_OPEN)
        .and_then(|t| t.strip_suffix(UNTRUSTED_CLOSE))
        .unwrap()
        .trim();
    assert_eq!(miolo.chars().count(), 20_000);
}

// ---------- blobs e isolamento por org ----------

#[test]
fn mesmo_arquivo_duas_vezes_gera_um_blob_so() {
    let amb = ambiente();
    let org = OrgId::generate();
    let bytes = fixture("texto.pdf");
    let primeiro = receive(&amb.config, org, "a.pdf", &bytes).unwrap();
    let segundo = receive(&amb.config, org, "b.pdf", &bytes).unwrap();
    assert!(!primeiro.metadados.blob_already_existed);
    assert!(segundo.metadados.blob_already_existed);
    assert_eq!(primeiro.metadados.blob_path, segundo.metadados.blob_path);
    assert_eq!(primeiro.metadados.sha256, segundo.metadados.sha256);

    let pasta_org = amb.blobs.join(org.to_string());
    let arquivos: Vec<_> = std::fs::read_dir(&pasta_org)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(arquivos.len(), 1, "{arquivos:?}");
    assert_eq!(std::fs::read(&primeiro.metadados.blob_path).unwrap(), bytes);
    assert_eq!(primeiro.metadados.blob_path.file_name().unwrap().len(), 64);
}

#[test]
fn orgs_diferentes_guardam_o_blob_em_pastas_separadas() {
    let amb = ambiente();
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
    // A org B nunca reaproveita o blob da org A: ele não aparece como "já existia" para ela.
    assert!(!b.metadados.blob_already_existed);
}

#[test]
fn nome_com_pasta_nao_escapa_do_diretorio_da_org() {
    let amb = ambiente();
    let org = OrgId::generate();
    let recebido = receive(
        &amb.config,
        org,
        "../../../etc/contrato.pdf",
        &fixture("texto.pdf"),
    )
    .unwrap();
    assert_eq!(recebido.metadados.original_name, "contrato.pdf");
    assert!(
        recebido
            .metadados
            .blob_path
            .starts_with(amb.blobs.join(org.to_string()))
    );
}

// ---------- configuração ----------

#[test]
fn configuracao_invalida_e_recusada() {
    let mut amb = ambiente();
    amb.config.byte_limit = 0;
    let org = OrgId::generate();
    let erro = receive(&amb.config, org, "a.pdf", b"x").unwrap_err();
    assert!(matches!(erro, FileError::Configuracao(_)), "{erro:?}");
}

#[test]
fn leitor_ausente_devolve_erro_claro() {
    let mut amb = ambiente();
    amb.config.extractor_path = Some(amb.blobs.join("nao-existe"));
    let org = OrgId::generate();
    let erro = receive(&amb.config, org, "a.pdf", &fixture("texto.pdf")).unwrap_err();
    assert!(matches!(erro, FileError::Extrator(_)), "{erro:?}");
    assert!(!amb.blobs.exists());
}

#[test]
fn tipo_real_e_o_que_o_conteudo_diz() {
    assert_eq!(
        poph_files::classify(&fixture("texto.pdf"), "qualquer.bin").unwrap(),
        Tipo::Pdf
    );
}
