use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const AS_CANARY: &str = "CANARIO-MX21-AS-7f3a-TAO-SECRETO";
const HS_CANARY: &str = "CANARIO-MX21-HS-91bc-TAO-SECRETO";
const HARNESS_URL: &str = "http://192.168.0.123:18010/matrix";

fn test_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("poph-cli-matrix-{label}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("cria pasta de teste");
    dir
}

fn config(dir: &Path, prefix: &str) -> PathBuf {
    let file = dir.join("poph.toml");
    fs::write(
        &file,
        format!(
            "[dados]\ndiretorio = \"{}\"\n\n[matrix]\nhabilitado = true\n\
             homeserver_url = \"http://127.0.0.1:18008\"\nserver_name = \"matrix-dev.local\"\n\
             prefixo = \"{prefix}\"\n",
            dir.join("dados").display()
        ),
    )
    .expect("escreve a configuração");
    file
}

fn environment<'a>(config: &'a Path, tokens: &'a [(&'a str, &'a str)]) -> Vec<(&'a str, &'a str)> {
    let mut pairs = vec![("POPH_CONFIG", config.to_str().expect("utf-8"))];
    pairs.extend_from_slice(tokens);
    pairs
}

fn poph(dir: &Path, args: &[&str], variables: &[(&str, &str)]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_poph"));
    command.args(args).current_dir(dir).env_clear();
    for (chave, value) in variables {
        command.env(chave, value);
    }
    command.output().expect("executa o poph")
}

fn text(saida: &[u8]) -> String {
    String::from_utf8_lossy(saida).into_owned()
}

fn without_tokens(saida: &str) {
    for canary in [AS_CANARY, HS_CANARY] {
        assert!(!saida.contains(canary), "token vazou na saída: {saida}");
    }
}

fn mode(path: &Path) -> u32 {
    fs::metadata(path)
        .expect("arquivo existe")
        .permissions()
        .mode()
        & 0o777
}

#[test]
fn generates_with_0600_without_token_in_output_and_verify_accepts_it() {
    let dir = test_dir("gera");
    let config = config(&dir, "ph");
    let env_vars = environment(
        &config,
        &[
            ("POPH_MATRIX_AS_TOKEN", AS_CANARY),
            ("POPH_MATRIX_HS_TOKEN", HS_CANARY),
        ],
    );
    let registro = dir.join("registro.yaml");

    let saida = poph(
        &dir,
        &[
            "matrix",
            "registro",
            "--saida",
            registro.to_str().expect("utf-8"),
            "--url",
            HARNESS_URL,
        ],
        &env_vars,
    );
    let stdout = text(&saida.stdout);
    assert_eq!(saida.status.code(), Some(0), "stdout: {stdout}");
    without_tokens(&stdout);
    without_tokens(&text(&saida.stderr));

    assert_eq!(mode(&registro), 0o600, "o registro tem que ser 0600");

    let content = fs::read_to_string(&registro).expect("lê o registro");
    assert!(content.contains(AS_CANARY));
    assert!(content.contains(&format!("url: \"{HARNESS_URL}\"")));
    assert!(content.contains("sender_localpart: \"ph_as_bot\""));

    let verified = poph(
        &dir,
        &[
            "matrix",
            "registro",
            "--verificar",
            registro.to_str().expect("utf-8"),
            "--url",
            HARNESS_URL,
        ],
        &env_vars,
    );
    let stdout = text(&verified.stdout);
    assert_eq!(verified.status.code(), Some(0), "stdout: {stdout}");
    assert!(stdout.contains("registro em dia"), "stdout: {stdout}");
    without_tokens(&stdout);
    without_tokens(&text(&verified.stderr));
}

#[test]
fn refuses_overwrite_without_force_and_overwrites_with_force() {
    let dir = test_dir("sobrescrever");
    let config = config(&dir, "ph");
    let env_vars = environment(
        &config,
        &[
            ("POPH_MATRIX_AS_TOKEN", AS_CANARY),
            ("POPH_MATRIX_HS_TOKEN", HS_CANARY),
        ],
    );
    let registro = dir.join("registro.yaml");
    fs::write(&registro, "id: antigo\n").expect("escreve o arquivo antigo");
    fs::set_permissions(&registro, fs::Permissions::from_mode(0o644)).expect("permissão 0644");

    let args = |extra: &'static str| {
        let mut args = vec![
            "matrix",
            "registro",
            "--saida",
            registro.to_str().expect("utf-8"),
            "--url",
            HARNESS_URL,
        ];
        if !extra.is_empty() {
            args.push(extra);
        }
        args
    };

    let refused = poph(&dir, &args(""), &env_vars);
    assert_eq!(refused.status.code(), Some(1));
    let stderr = text(&refused.stderr);
    assert!(stderr.contains("já existe"), "stderr: {stderr}");
    assert!(stderr.contains("--forcar"), "stderr: {stderr}");
    assert_eq!(
        fs::read_to_string(&registro).expect("lê"),
        "id: antigo\n",
        "o arquivo não pode ter sido tocado"
    );

    let forced = poph(&dir, &args("--forcar"), &env_vars);
    assert_eq!(
        forced.status.code(),
        Some(0),
        "stdout: {}",
        text(&forced.stdout)
    );
    assert_eq!(mode(&registro), 0o600);
    assert!(
        fs::read_to_string(&registro)
            .expect("lê")
            .contains("id: \"poph-matrix\"")
    );
}

#[test]
fn missing_token_names_the_variable_without_showing_the_other() {
    let dir = test_dir("sem-token");
    let config = config(&dir, "ph");
    let env_vars = environment(&config, &[("POPH_MATRIX_AS_TOKEN", AS_CANARY)]);
    let registro = dir.join("registro.yaml");

    let saida = poph(
        &dir,
        &[
            "matrix",
            "registro",
            "--saida",
            registro.to_str().expect("utf-8"),
            "--url",
            HARNESS_URL,
        ],
        &env_vars,
    );
    assert_eq!(saida.status.code(), Some(1));
    let stderr = text(&saida.stderr);
    assert!(stderr.contains("POPH_MATRIX_HS_TOKEN"), "stderr: {stderr}");
    without_tokens(&stderr);
    without_tokens(&text(&saida.stdout));
    assert!(
        !registro.exists(),
        "nada pode ser gravado sem os dois tokens"
    );
}

#[test]
fn without_url_the_error_explains_what_is_missing() {
    let dir = test_dir("sem-url");
    let config = config(&dir, "ph");
    let env_vars = environment(
        &config,
        &[
            ("POPH_MATRIX_AS_TOKEN", AS_CANARY),
            ("POPH_MATRIX_HS_TOKEN", HS_CANARY),
        ],
    );
    let saida = poph(
        &dir,
        &[
            "matrix",
            "registro",
            "--saida",
            dir.join("registro.yaml").to_str().expect("utf-8"),
        ],
        &env_vars,
    );
    assert_eq!(saida.status.code(), Some(1));
    let stderr = text(&saida.stderr);
    assert!(stderr.contains("--url"), "stderr: {stderr}");
    assert!(stderr.contains("POPH_MATRIX_URL_AS"), "stderr: {stderr}");
    without_tokens(&stderr);
}

#[test]
fn url_from_environment_variable_works() {
    let dir = test_dir("url-variavel");
    let config = config(&dir, "ph");
    let env_vars = environment(
        &config,
        &[
            ("POPH_MATRIX_AS_TOKEN", AS_CANARY),
            ("POPH_MATRIX_HS_TOKEN", HS_CANARY),
            ("POPH_MATRIX_URL_AS", HARNESS_URL),
        ],
    );
    let registro = dir.join("registro.yaml");
    let saida = poph(
        &dir,
        &[
            "matrix",
            "registro",
            "--saida",
            registro.to_str().expect("utf-8"),
        ],
        &env_vars,
    );
    assert_eq!(
        saida.status.code(),
        Some(0),
        "stderr: {}",
        text(&saida.stderr)
    );
    assert!(
        fs::read_to_string(&registro)
            .expect("lê")
            .contains(&format!("url: \"{HARNESS_URL}\""))
    );
}

#[test]
fn verify_refuses_swapped_url_naming_the_field() {
    let dir = test_dir("url-trocada");
    let config = config(&dir, "ph");
    let env_vars = environment(
        &config,
        &[
            ("POPH_MATRIX_AS_TOKEN", AS_CANARY),
            ("POPH_MATRIX_HS_TOKEN", HS_CANARY),
        ],
    );
    let registro = dir.join("registro.yaml");
    let generated = poph(
        &dir,
        &[
            "matrix",
            "registro",
            "--saida",
            registro.to_str().expect("utf-8"),
            "--url",
            HARNESS_URL,
        ],
        &env_vars,
    );
    assert_eq!(generated.status.code(), Some(0));

    let verified = poph(
        &dir,
        &[
            "matrix",
            "registro",
            "--verificar",
            registro.to_str().expect("utf-8"),
            "--url",
            "http://outro-endereco:9000",
        ],
        &env_vars,
    );
    assert_eq!(verified.status.code(), Some(1));
    let stderr = text(&verified.stderr);
    assert!(stderr.contains("campo url"), "stderr: {stderr}");
    assert!(
        stderr.contains("http://outro-endereco:9000"),
        "stderr: {stderr}"
    );
    without_tokens(&stderr);
    without_tokens(&text(&verified.stdout));
}

#[test]
fn verify_refuses_swapped_prefix_naming_the_namespaces() {
    let dir = test_dir("prefixo-trocado");
    let ph_config = config(&dir, "ph");
    let ph_env = environment(
        &ph_config,
        &[
            ("POPH_MATRIX_AS_TOKEN", AS_CANARY),
            ("POPH_MATRIX_HS_TOKEN", HS_CANARY),
        ],
    );
    let registro = dir.join("registro.yaml");
    let generated = poph(
        &dir,
        &[
            "matrix",
            "registro",
            "--saida",
            registro.to_str().expect("utf-8"),
            "--url",
            HARNESS_URL,
        ],
        &ph_env,
    );
    assert_eq!(generated.status.code(), Some(0));

    let wa_config = config(&dir, "wa");
    let wa_env = environment(
        &wa_config,
        &[
            ("POPH_MATRIX_AS_TOKEN", AS_CANARY),
            ("POPH_MATRIX_HS_TOKEN", HS_CANARY),
        ],
    );
    let verified = poph(
        &dir,
        &[
            "matrix",
            "registro",
            "--verificar",
            registro.to_str().expect("utf-8"),
            "--url",
            HARNESS_URL,
        ],
        &wa_env,
    );
    assert_eq!(verified.status.code(), Some(1));
    let stderr = text(&verified.stderr);
    assert!(
        stderr.contains("campo sender_localpart"),
        "stderr: {stderr}"
    );
    assert!(
        stderr.contains("campo namespaces.users"),
        "stderr: {stderr}"
    );
    assert!(
        stderr.contains("campo namespaces.aliases"),
        "stderr: {stderr}"
    );
    without_tokens(&stderr);
}

#[test]
fn verify_refuses_token_different_from_environment_without_showing_value() {
    let dir = test_dir("token-trocado");
    let config = config(&dir, "ph");
    let registro = dir.join("registro.yaml");
    let generated = poph(
        &dir,
        &[
            "matrix",
            "registro",
            "--saida",
            registro.to_str().expect("utf-8"),
            "--url",
            HARNESS_URL,
        ],
        &environment(
            &config,
            &[
                ("POPH_MATRIX_AS_TOKEN", AS_CANARY),
                ("POPH_MATRIX_HS_TOKEN", HS_CANARY),
            ],
        ),
    );
    assert_eq!(generated.status.code(), Some(0));

    let verified = poph(
        &dir,
        &[
            "matrix",
            "registro",
            "--verificar",
            registro.to_str().expect("utf-8"),
            "--url",
            HARNESS_URL,
        ],
        &environment(
            &config,
            &[
                ("POPH_MATRIX_AS_TOKEN", "outro-token-qualquer"),
                ("POPH_MATRIX_HS_TOKEN", HS_CANARY),
            ],
        ),
    );
    assert_eq!(verified.status.code(), Some(1));
    let stderr = text(&verified.stderr);
    assert!(stderr.contains("campo as_token"), "stderr: {stderr}");
    assert!(stderr.contains("POPH_MATRIX_AS_TOKEN"), "stderr: {stderr}");
    without_tokens(&stderr);
    assert!(
        !stderr.contains("outro-token-qualquer"),
        "o valor do token não pode aparecer: {stderr}"
    );
}

#[test]
fn without_matrix_section_the_error_says_what_is_missing() {
    let dir = test_dir("sem-secao");
    let file = dir.join("poph.toml");
    fs::write(&file, "[dados]\ndiretorio = \"dados\"\n").expect("escreve a configuração");
    let env_vars = environment(
        &file,
        &[
            ("POPH_MATRIX_AS_TOKEN", AS_CANARY),
            ("POPH_MATRIX_HS_TOKEN", HS_CANARY),
        ],
    );
    let saida = poph(
        &dir,
        &[
            "matrix",
            "registro",
            "--saida",
            dir.join("registro.yaml").to_str().expect("utf-8"),
            "--url",
            HARNESS_URL,
        ],
        &env_vars,
    );
    assert_eq!(saida.status.code(), Some(1));
    let stderr = text(&saida.stderr);
    assert!(stderr.contains("[matrix]"), "stderr: {stderr}");
    without_tokens(&stderr);
}

#[test]
fn verify_missing_file_does_not_pass_in_silence() {
    let dir = test_dir("inexistente");
    let config = config(&dir, "ph");
    let env_vars = environment(
        &config,
        &[
            ("POPH_MATRIX_AS_TOKEN", AS_CANARY),
            ("POPH_MATRIX_HS_TOKEN", HS_CANARY),
        ],
    );
    let saida = poph(
        &dir,
        &[
            "matrix",
            "registro",
            "--verificar",
            dir.join("nao-existe.yaml").to_str().expect("utf-8"),
            "--url",
            HARNESS_URL,
        ],
        &env_vars,
    );
    assert_eq!(saida.status.code(), Some(1));
    assert!(text(&saida.stderr).contains("não consegui ler"));
}

#[test]
fn output_and_verify_together_are_refused() {
    let dir = test_dir("dois-modos");
    let config = config(&dir, "ph");
    let env_vars = environment(&config, &[]);
    let saida = poph(
        &dir,
        &[
            "matrix",
            "registro",
            "--saida",
            "a.yaml",
            "--verificar",
            "b.yaml",
        ],
        &env_vars,
    );
    assert_eq!(saida.status.code(), Some(1));
    assert!(text(&saida.stderr).contains("não os dois"));
}
