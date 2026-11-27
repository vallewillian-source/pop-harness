use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const CIPHER_SENTINEL: &str = "SENTINELA-CIFRA-7f3a";
const LLM_SENTINEL: &str = "SENTINELA-LLM-91bc";
const JEV_SENTINEL: &str = "SENTINELA-JEV-204d";

fn test_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("poph-cli-{label}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("cria pasta de teste");
    dir
}

fn poph(current_dir: &Path, args: &[&str], variables: &[(&str, &str)]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_poph"));
    command.args(args).current_dir(current_dir).env_clear();
    for (key, value) in variables {
        command.env(key, value);
    }
    command.output().expect("executa o poph")
}

fn full_environment(dir: &Path) -> Vec<(&'static str, String)> {
    vec![
        (
            "POPH_DADOS_DIRETORIO",
            dir.join("dados").display().to_string(),
        ),
        ("POPH_CHAVE_CIFRA", CIPHER_SENTINEL.to_owned()),
        ("ANTIGRAVITY_API_KEY", LLM_SENTINEL.to_owned()),
    ]
}

fn as_refs<'a>(pairs: &'a [(&'static str, String)]) -> Vec<(&'a str, &'a str)> {
    pairs.iter().map(|(k, v)| (*k, v.as_str())).collect()
}

fn text(output: &[u8]) -> String {
    String::from_utf8_lossy(output).into_owned()
}

fn without_sentinels(output: &str) {
    for sentinel in [CIPHER_SENTINEL, LLM_SENTINEL, JEV_SENTINEL] {
        assert!(
            !output.contains(sentinel),
            "segredo vazou na saída: {output}"
        );
    }
}

#[test]
fn version_and_help_keep_working() {
    let dir = test_dir("versao");
    let version = poph(&dir, &["--version"], &[]);
    assert!(version.status.success());
    assert_eq!(text(&version.stdout).trim(), "poph 0.1.0");

    let help = poph(&dir, &["--help"], &[]);
    assert!(help.status.success());
    assert!(text(&help.stdout).contains("doctor"));
}

#[test]
fn missing_command_shows_help_and_exits_two() {
    let dir = test_dir("sem-comando");
    let output = poph(&dir, &[], &[]);
    assert_eq!(output.status.code(), Some(2));
    assert!(text(&output.stderr).contains("doctor"));
}

#[test]
fn doctor_with_correct_environment_exits_zero_and_shows_no_secret() {
    let dir = test_dir("ok");
    let environment = full_environment(&dir);
    let output = poph(&dir, &["doctor"], &as_refs(&environment));
    let stdout = text(&output.stdout);
    assert_eq!(output.status.code(), Some(0), "stdout: {stdout}");
    assert!(stdout.contains("Ambiente pronto"), "stdout: {stdout}");
    assert!(stdout.contains("variável POPH_CHAVE_CIFRA: definida"));
    without_sentinels(&stdout);
    without_sentinels(&text(&output.stderr));
}

#[test]
fn doctor_json_is_valid_and_shows_no_secret() {
    let dir = test_dir("json");
    let environment = full_environment(&dir);
    let output = poph(&dir, &["doctor", "--json"], &as_refs(&environment));
    let stdout = text(&output.stdout);
    assert_eq!(output.status.code(), Some(0), "stdout: {stdout}");
    without_sentinels(&stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("stdout é JSON válido");
    assert_eq!(json["versao_relatorio"], 1);
    assert_eq!(json["status"], "ok");
}

#[test]
fn missing_required_variable_exits_nonzero_and_names_it() {
    let dir = test_dir("sem-cifra");
    let mut environment = full_environment(&dir);
    environment.retain(|(k, _)| *k != "POPH_CHAVE_CIFRA");
    let output = poph(&dir, &["doctor"], &as_refs(&environment));
    let stdout = text(&output.stdout);
    assert_ne!(output.status.code(), Some(0));
    assert!(stdout.contains("POPH_CHAVE_CIFRA"), "stdout: {stdout}");
    assert!(stdout.contains("erro(s) a corrigir"), "stdout: {stdout}");
    without_sentinels(&stdout);
}

#[test]
fn secrets_do_not_leak_in_log_even_at_trace_level_and_json() {
    let dir = test_dir("log");
    let mut environment = full_environment(&dir);
    environment.push(("TYPESAFE_API_KEY", JEV_SENTINEL.to_owned()));
    environment.push(("POPH_JEV_ATIVO", "true".to_owned()));
    environment.push(("POPH_LOG_NIVEL", "trace".to_owned()));
    environment.push(("POPH_LOG_FORMATO", "json".to_owned()));
    let output = poph(&dir, &["doctor", "--json"], &as_refs(&environment));
    let stderr = text(&output.stderr);
    assert!(
        stderr.contains("verificacao"),
        "o log em trace deveria ter eventos: {stderr}"
    );
    for line in stderr.lines().filter(|l| !l.trim().is_empty()) {
        serde_json::from_str::<serde_json::Value>(line)
            .unwrap_or_else(|error| panic!("linha de log não é JSON ({error}): {line}"));
    }
    without_sentinels(&stderr);
    without_sentinels(&text(&output.stdout));
}

#[test]
fn invalid_toml_names_file_and_key_and_exits_nonzero() {
    let dir = test_dir("toml");
    let file = dir.join("poph.toml");
    fs::write(&file, "[servidor]\nportaa = 2000\n").expect("escreve");
    let environment = full_environment(&dir);
    let path = file.display().to_string();
    let args = ["--config", path.as_str(), "doctor"];
    let output = poph(&dir, &args, &as_refs(&environment));
    let stdout = text(&output.stdout);
    assert_ne!(output.status.code(), Some(0));
    assert!(stdout.contains(&path), "faltou o arquivo: {stdout}");
    assert!(
        stdout.contains("servidor.portaa"),
        "faltou a chave: {stdout}"
    );
    without_sentinels(&stdout);
}

#[test]
fn file_and_environment_precedence_shows_in_doctor() {
    let dir = test_dir("precedencia");
    let file = dir.join("poph.toml");
    fs::write(&file, "[dados]\ndiretorio = \"do-arquivo\"\n").expect("escreve");
    let path = file.display().to_string();
    let args = ["--config", path.as_str(), "doctor", "--json"];
    let environment = full_environment(&dir);
    let output = poph(&dir, &args, &as_refs(&environment));
    let stdout = text(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("JSON");
    let data_dir = json["verificacoes"]
        .as_array()
        .expect("lista")
        .iter()
        .find(|v| v["nome"] == "pasta de dados")
        .expect("tem pasta de dados")["detalhe"]
        .as_str()
        .expect("texto")
        .to_owned();
    assert!(data_dir.contains("dados"), "detalhe: {data_dir}");
    assert!(
        !data_dir.contains("do-arquivo"),
        "arquivo venceu o ambiente: {data_dir}"
    );
}

#[test]
fn migrate_creates_databases_and_is_idempotent() {
    let dir = test_dir("migrate");
    let environment = full_environment(&dir);

    let first = poph(&dir, &["migrate"], &as_refs(&environment));
    let output = text(&first.stdout);
    assert_eq!(first.status.code(), Some(0), "saída: {output}");
    let target = poph_store::harness_target_version();
    assert!(
        output.contains(&format!("versão 0 → {target}")),
        "saída: {output}"
    );
    assert!(output.contains("0001_esquema_inicial"), "saída: {output}");
    assert!(
        output.contains("0003_origem_autor_mensagens"),
        "saída: {output}"
    );
    assert!(
        output.contains("0002_entrada_proxima_tentativa"),
        "saída: {output}"
    );
    assert!(
        dir.join("dados").join("harness.db").is_file(),
        "harness.db não foi criado"
    );
    without_sentinels(&output);

    let second = poph(&dir, &["migrate", "--json"], &as_refs(&environment));
    let json: serde_json::Value =
        serde_json::from_str(&text(&second.stdout)).expect("JSON do migrate");
    assert_eq!(second.status.code(), Some(0));
    assert_eq!(json["ja_estava_em_dia"], true);
    let schema_version = poph_store::harness_target_version();
    assert_eq!(
        json["harness"]["versao_depois"].as_u64(),
        Some(u64::from(schema_version))
    );
    assert_eq!(json["rag"]["versao_depois"], 0);
    assert_eq!(
        json["harness"]["aplicadas"].as_array().map(Vec::len),
        Some(0)
    );

    let doctor = poph(&dir, &["doctor"], &as_refs(&environment));
    let output = text(&doctor.stdout);
    assert_eq!(doctor.status.code(), Some(0), "saída: {output}");
    assert!(
        output.contains(&format!("esquema versão {schema_version}")),
        "saída: {output}"
    );
    without_sentinels(&output);
}
