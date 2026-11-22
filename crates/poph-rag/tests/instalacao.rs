//! Instalação do modelo (`poph model install`) contra um servidor HTTP local de teste.
//! Não usa a internet: os arquivos e os hashes são controlados pelo teste.

use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::thread;

use poph_rag::ErroRag;
use poph_rag::instalacao::{ArquivoModelo, Situacao, instalar_arquivos};
use sha2::{Digest, Sha256};

struct Pasta(PathBuf);

impl Pasta {
    fn nova(nome: &str) -> Self {
        let agora = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let caminho = std::env::temp_dir().join(format!(
            "poph-instalacao-{nome}-{}-{agora}",
            std::process::id()
        ));
        Self(caminho)
    }

    fn caminho(&self) -> &Path {
        &self.0
    }
}

impl Drop for Pasta {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn sha_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Servidor que atende uma conexão por resposta, na ordem dada, e depois se encerra.
/// Devolve a origem (`http://127.0.0.1:porta`).
fn servir(respostas: Vec<(u16, Vec<u8>)>) -> String {
    let ouvinte = TcpListener::bind("127.0.0.1:0").expect("porta local");
    let origem = format!("http://{}", ouvinte.local_addr().expect("endereço"));
    thread::spawn(move || {
        for (status, corpo) in respostas {
            let Ok((mut conexao, _)) = ouvinte.accept() else {
                return;
            };
            let mut pedido = [0u8; 4096];
            let _ = conexao.read(&mut pedido);
            let cabecalho = format!(
                "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                corpo.len()
            );
            let _ = conexao.write_all(cabecalho.as_bytes());
            let _ = conexao.write_all(&corpo);
        }
    });
    origem
}

fn arquivo(nome: &str, origem: &str, sha: &str) -> ArquivoModelo {
    ArquivoModelo {
        nome: nome.to_owned(),
        url: format!("{origem}/{nome}"),
        sha256: sha.to_owned(),
    }
}

#[test]
fn baixa_confere_publica_e_na_segunda_vez_nao_baixa() {
    let pasta = Pasta::nova("ok");
    let corpo = b"modelo de teste, conteudo fixo".to_vec();
    // Só uma resposta: se a segunda chamada tentasse baixar, a conexão falharia.
    let origem = servir(vec![(200, corpo.clone())]);
    let itens = [arquivo("modelo.bin", &origem, &sha_hex(&corpo))];

    let primeira = instalar_arquivos(pasta.caminho(), &itens).expect("instala");
    assert_eq!(primeira.arquivos.len(), 1);
    assert_eq!(primeira.arquivos[0].situacao, Situacao::Baixado);
    assert_eq!(primeira.arquivos[0].bytes, corpo.len() as u64);
    assert_eq!(
        fs::read(pasta.caminho().join("modelo.bin")).expect("lê"),
        corpo
    );
    assert!(
        !pasta.caminho().join(".modelo.bin.parcial").exists(),
        "temporário deve sumir"
    );

    let segunda = instalar_arquivos(pasta.caminho(), &itens).expect("segunda vez");
    assert_eq!(segunda.arquivos[0].situacao, Situacao::JaEstava);
}

#[test]
fn sha_errado_nao_publica_o_arquivo() {
    let pasta = Pasta::nova("sha-errado");
    let corpo = b"conteudo que nao confere".to_vec();
    let origem = servir(vec![(200, corpo.clone())]);
    let esperado = sha_hex(b"outro conteudo");
    let itens = [arquivo("modelo.bin", &origem, &esperado)];

    let erro =
        instalar_arquivos(pasta.caminho(), &itens).expect_err("hash errado não pode ser aceito");

    match &erro {
        ErroRag::ChecksumNaoConfere {
            arquivo,
            esperado: e,
            obtido,
        } => {
            assert_eq!(arquivo, "modelo.bin");
            assert_eq!(e, &esperado);
            assert_eq!(obtido, &sha_hex(&corpo));
        }
        outro => panic!("erro inesperado: {outro:?}"),
    }
    assert!(erro.to_string().contains("não confere"), "{erro}");
    assert!(
        !pasta.caminho().join("modelo.bin").exists(),
        "arquivo com hash errado não pode ser publicado"
    );
    assert!(
        !pasta.caminho().join(".modelo.bin.parcial").exists(),
        "temporário deve ser removido"
    );
}

#[test]
fn arquivo_existente_com_conteudo_errado_e_substituido() {
    let pasta = Pasta::nova("substitui");
    fs::create_dir_all(pasta.caminho()).expect("pasta");
    fs::write(pasta.caminho().join("modelo.bin"), b"lixo antigo").expect("escreve");
    let corpo = b"versao correta".to_vec();
    let origem = servir(vec![(200, corpo.clone())]);
    let itens = [arquivo("modelo.bin", &origem, &sha_hex(&corpo))];

    let relatorio = instalar_arquivos(pasta.caminho(), &itens).expect("substitui");

    assert_eq!(relatorio.arquivos[0].situacao, Situacao::Baixado);
    assert_eq!(
        fs::read(pasta.caminho().join("modelo.bin")).expect("lê"),
        corpo
    );
}

#[test]
fn resposta_de_erro_do_servidor_da_mensagem_clara() {
    let pasta = Pasta::nova("404");
    let origem = servir(vec![(404, b"nao achei".to_vec())]);
    let itens = [arquivo("modelo.bin", &origem, &sha_hex(b"qualquer"))];

    let erro = instalar_arquivos(pasta.caminho(), &itens).expect_err("404 é erro");

    match &erro {
        ErroRag::Download { arquivo, motivo } => {
            assert_eq!(arquivo, "modelo.bin");
            assert!(motivo.contains("404"), "{motivo}");
        }
        outro => panic!("erro inesperado: {outro:?}"),
    }
    assert!(!pasta.caminho().join(".modelo.bin.parcial").exists());
}
