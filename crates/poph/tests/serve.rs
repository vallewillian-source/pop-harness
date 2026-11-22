//! `poph serve` recusa subir com banco que não está pronto (HR-20).
//!
//! A regra: banco ausente é criado já migrado (instalação nova); banco existente que o código não
//! entende — arquivo estragado ou esquema atrás das migrações — exige `poph migrate` antes.
//! Aqui se prova a recusa: o processo sai com código 1, explica o que fazer e não abre porta.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn pasta_de_teste(rotulo: &str) -> PathBuf {
    let pasta = std::env::temp_dir().join(format!("poph-serve-{rotulo}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&pasta);
    fs::create_dir_all(pasta.join("dados")).expect("cria pasta de teste");
    pasta
}

fn escrever_config(pasta: &Path, porta: u16) {
    let texto = format!(
        "[dados]\ndiretorio = \"{}\"\n\n[servidor]\nporta = {porta}\n",
        pasta.join("dados").display()
    );
    fs::write(pasta.join("poph.toml"), texto).expect("configuração");
}

fn poph(pasta_atual: &Path, args: &[&str]) -> Output {
    let mut comando = Command::new(env!("CARGO_BIN_EXE_poph"));
    comando.args(args).current_dir(pasta_atual).env_clear();
    comando.output().expect("executa o poph")
}

fn serve(pasta_atual: &Path) -> Output {
    poph(pasta_atual, &["serve", "--config", "poph.toml"])
}

fn texto(saida: &[u8]) -> String {
    String::from_utf8_lossy(saida).into_owned()
}

#[test]
fn serve_recusa_banco_que_nao_e_do_harness() {
    let pasta = pasta_de_teste("estragado");
    escrever_config(&pasta, 7487);
    // Banco existente mas ilegível: não é um SQLite do Harness.
    fs::write(pasta.join("dados").join("harness.db"), b"nao sou sqlite").expect("arquivo");

    let saida = serve(&pasta);
    assert_eq!(saida.status.code(), Some(1), "saída: {saida:?}");
    let erro = texto(&saida.stderr);
    assert!(
        erro.contains("não consegui ler a versão do banco"),
        "mensagem inesperada: {erro}"
    );
}

#[test]
fn serve_recusa_banco_atras_das_migracoes() {
    let pasta = pasta_de_teste("atrasado");
    escrever_config(&pasta, 7488);

    let migrou = poph(&pasta, &["migrate", "--config", "poph.toml"]);
    assert!(migrou.status.success(), "migrate: {migrou:?}");

    // Simula o banco de uma versão anterior: o esquema existe, mas sem o registro da migração.
    let caminho = pasta.join("dados").join("harness.db");
    let conexao = rusqlite::Connection::open(&caminho).expect("abre o harness.db");
    conexao
        .execute("DELETE FROM schema_migrations", [])
        .expect("esvazia o registro de versão");
    drop(conexao);

    let saida = serve(&pasta);
    assert_eq!(saida.status.code(), Some(1), "saída: {saida:?}");
    let erro = texto(&saida.stderr);
    // A versão esperada vem da fonte única do `poph-store`: a próxima migração não deixa esta
    // asserção para trás.
    let esperado = format!(
        "está na versão 0 e o código espera a {}",
        poph_store::versao_alvo_harness()
    );
    assert!(erro.contains(&esperado), "mensagem inesperada: {erro}");
    assert!(
        erro.contains("poph migrate"),
        "a mensagem precisa dizer o que fazer: {erro}"
    );
}
