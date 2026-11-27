use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn test_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("poph-serve-{label}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("dados")).expect("cria pasta de teste");
    dir
}

fn write_config(dir: &Path, porta: u16) {
    let texto = format!(
        "[dados]\ndiretorio = \"{}\"\n\n[servidor]\nporta = {porta}\n",
        dir.join("dados").display()
    );
    fs::write(dir.join("poph.toml"), texto).expect("configuração");
}

fn poph(current_dir: &Path, args: &[&str]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_poph"));
    command.args(args).current_dir(current_dir).env_clear();
    command.output().expect("executa o poph")
}

fn serve(current_dir: &Path) -> Output {
    poph(current_dir, &["serve", "--config", "poph.toml"])
}

fn texto(saida: &[u8]) -> String {
    String::from_utf8_lossy(saida).into_owned()
}

#[test]
fn serve_refuses_database_not_from_harness() {
    let dir = test_dir("estragado");
    write_config(&dir, 7487);
    fs::write(dir.join("dados").join("harness.db"), b"nao sou sqlite").expect("arquivo");

    let saida = serve(&dir);
    assert_eq!(saida.status.code(), Some(1), "saída: {saida:?}");
    let error = texto(&saida.stderr);
    assert!(
        error.contains("não consegui ler a versão do banco"),
        "mensagem inesperada: {error}"
    );
}

#[test]
fn serve_refuses_database_behind_migrations() {
    let dir = test_dir("atrasado");
    write_config(&dir, 7488);

    let migrated = poph(&dir, &["migrate", "--config", "poph.toml"]);
    assert!(migrated.status.success(), "migrate: {migrated:?}");

    let path = dir.join("dados").join("harness.db");
    let connection = rusqlite::Connection::open(&path).expect("abre o harness.db");
    connection
        .execute("DELETE FROM schema_migrations", [])
        .expect("esvazia o registro de versão");
    drop(connection);

    let saida = serve(&dir);
    assert_eq!(saida.status.code(), Some(1), "saída: {saida:?}");
    let error = texto(&saida.stderr);
    let expected = format!(
        "está na versão 0 e o código espera a {}",
        poph_store::harness_target_version()
    );
    assert!(error.contains(&expected), "mensagem inesperada: {error}");
    assert!(
        error.contains("poph migrate"),
        "a mensagem precisa dizer o que fazer: {error}"
    );
}
