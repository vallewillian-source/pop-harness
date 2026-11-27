//! Testes de `poph matrix registro` de ponta a ponta: processo de verdade, ambiente controlado.
//!
//! O que precisa ficar provado (MX-21): o arquivo sai com 0600, não sobrescreve sem `--forcar`,
//! token ausente nomeia a variável, nenhum token aparece em stdout/stderr, e `--verificar` aceita o
//! que este comando gerou e recusa (nomeando o campo) o que diverge da configuração.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const CANARIO_AS: &str = "CANARIO-MX21-AS-7f3a-TAO-SECRETO";
const CANARIO_HS: &str = "CANARIO-MX21-HS-91bc-TAO-SECRETO";
const URL_HARNESS: &str = "http://192.168.0.123:18010/matrix";

fn pasta_de_teste(rotulo: &str) -> PathBuf {
    let pasta =
        std::env::temp_dir().join(format!("poph-cli-matrix-{rotulo}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&pasta);
    fs::create_dir_all(&pasta).expect("cria pasta de teste");
    pasta
}

/// Configuração mínima com a seção `[matrix]` ligada e válida.
fn config(pasta: &Path, prefix: &str) -> PathBuf {
    let arquivo = pasta.join("poph.toml");
    fs::write(
        &arquivo,
        format!(
            "[dados]\ndiretorio = \"{}\"\n\n[matrix]\nhabilitado = true\n\
             homeserver_url = \"http://127.0.0.1:18008\"\nserver_name = \"matrix-dev.local\"\n\
             prefixo = \"{prefix}\"\n",
            pasta.join("dados").display()
        ),
    )
    .expect("escreve a configuração");
    arquivo
}

/// Ambiente limpo: só o necessário para o comando.
fn ambiente<'a>(config: &'a Path, tokens: &'a [(&'a str, &'a str)]) -> Vec<(&'a str, &'a str)> {
    let mut pares = vec![("POPH_CONFIG", config.to_str().expect("utf-8"))];
    pares.extend_from_slice(tokens);
    pares
}

fn poph(pasta: &Path, args: &[&str], variaveis: &[(&str, &str)]) -> Output {
    let mut comando = Command::new(env!("CARGO_BIN_EXE_poph"));
    comando.args(args).current_dir(pasta).env_clear();
    for (chave, valor) in variaveis {
        comando.env(chave, valor);
    }
    comando.output().expect("executa o poph")
}

fn text(saida: &[u8]) -> String {
    String::from_utf8_lossy(saida).into_owned()
}

fn sem_tokens(saida: &str) {
    for canario in [CANARIO_AS, CANARIO_HS] {
        assert!(!saida.contains(canario), "token vazou na saída: {saida}");
    }
}

fn modo(caminho: &Path) -> u32 {
    fs::metadata(caminho)
        .expect("arquivo existe")
        .permissions()
        .mode()
        & 0o777
}

#[test]
fn gera_com_0600_sem_token_na_saida_e_o_verificar_aceita() {
    let pasta = pasta_de_teste("gera");
    let config = config(&pasta, "ph");
    let ambiente = ambiente(
        &config,
        &[
            ("POPH_MATRIX_AS_TOKEN", CANARIO_AS),
            ("POPH_MATRIX_HS_TOKEN", CANARIO_HS),
        ],
    );
    let registro = pasta.join("registro.yaml");

    let saida = poph(
        &pasta,
        &[
            "matrix",
            "registro",
            "--saida",
            registro.to_str().expect("utf-8"),
            "--url",
            URL_HARNESS,
        ],
        &ambiente,
    );
    let stdout = text(&saida.stdout);
    assert_eq!(saida.status.code(), Some(0), "stdout: {stdout}");
    sem_tokens(&stdout);
    sem_tokens(&text(&saida.stderr));

    assert_eq!(modo(&registro), 0o600, "o registro tem que ser 0600");

    let conteudo = fs::read_to_string(&registro).expect("lê o registro");
    // O arquivo tem os tokens (é o uso dele); a saída do comando, não.
    assert!(conteudo.contains(CANARIO_AS));
    assert!(conteudo.contains(&format!("url: \"{URL_HARNESS}\"")));
    assert!(conteudo.contains("sender_localpart: \"ph_as_bot\""));

    let conferido = poph(
        &pasta,
        &[
            "matrix",
            "registro",
            "--verificar",
            registro.to_str().expect("utf-8"),
            "--url",
            URL_HARNESS,
        ],
        &ambiente,
    );
    let stdout = text(&conferido.stdout);
    assert_eq!(conferido.status.code(), Some(0), "stdout: {stdout}");
    assert!(stdout.contains("registro em dia"), "stdout: {stdout}");
    sem_tokens(&stdout);
    sem_tokens(&text(&conferido.stderr));
}

#[test]
fn recusa_sobrescrever_sem_forcar_e_sobrescreve_com_forcar() {
    let pasta = pasta_de_teste("sobrescrever");
    let config = config(&pasta, "ph");
    let ambiente = ambiente(
        &config,
        &[
            ("POPH_MATRIX_AS_TOKEN", CANARIO_AS),
            ("POPH_MATRIX_HS_TOKEN", CANARIO_HS),
        ],
    );
    let registro = pasta.join("registro.yaml");
    // Arquivo que já existe com outra permissão: prova que o --forcar volta para 0600.
    fs::write(&registro, "id: antigo\n").expect("escreve o arquivo antigo");
    fs::set_permissions(&registro, fs::Permissions::from_mode(0o644)).expect("permissão 0644");

    let args = |extra: &'static str| {
        let mut args = vec![
            "matrix",
            "registro",
            "--saida",
            registro.to_str().expect("utf-8"),
            "--url",
            URL_HARNESS,
        ];
        if !extra.is_empty() {
            args.push(extra);
        }
        args
    };

    let recusado = poph(&pasta, &args(""), &ambiente);
    assert_eq!(recusado.status.code(), Some(1));
    let stderr = text(&recusado.stderr);
    assert!(stderr.contains("já existe"), "stderr: {stderr}");
    assert!(stderr.contains("--forcar"), "stderr: {stderr}");
    assert_eq!(
        fs::read_to_string(&registro).expect("lê"),
        "id: antigo\n",
        "o arquivo não pode ter sido tocado"
    );

    let forçado = poph(&pasta, &args("--forcar"), &ambiente);
    assert_eq!(
        forçado.status.code(),
        Some(0),
        "stdout: {}",
        text(&forçado.stdout)
    );
    assert_eq!(modo(&registro), 0o600);
    assert!(
        fs::read_to_string(&registro)
            .expect("lê")
            .contains("id: \"poph-matrix\"")
    );
}

#[test]
fn token_ausente_nomeia_a_variavel_sem_mostrar_o_outro() {
    let pasta = pasta_de_teste("sem-token");
    let config = config(&pasta, "ph");
    let ambiente = ambiente(&config, &[("POPH_MATRIX_AS_TOKEN", CANARIO_AS)]);
    let registro = pasta.join("registro.yaml");

    let saida = poph(
        &pasta,
        &[
            "matrix",
            "registro",
            "--saida",
            registro.to_str().expect("utf-8"),
            "--url",
            URL_HARNESS,
        ],
        &ambiente,
    );
    assert_eq!(saida.status.code(), Some(1));
    let stderr = text(&saida.stderr);
    assert!(stderr.contains("POPH_MATRIX_HS_TOKEN"), "stderr: {stderr}");
    sem_tokens(&stderr);
    sem_tokens(&text(&saida.stdout));
    assert!(
        !registro.exists(),
        "nada pode ser gravado sem os dois tokens"
    );
}

#[test]
fn sem_url_o_erro_explica_o_que_falta() {
    let pasta = pasta_de_teste("sem-url");
    let config = config(&pasta, "ph");
    let ambiente = ambiente(
        &config,
        &[
            ("POPH_MATRIX_AS_TOKEN", CANARIO_AS),
            ("POPH_MATRIX_HS_TOKEN", CANARIO_HS),
        ],
    );
    let saida = poph(
        &pasta,
        &[
            "matrix",
            "registro",
            "--saida",
            pasta.join("registro.yaml").to_str().expect("utf-8"),
        ],
        &ambiente,
    );
    assert_eq!(saida.status.code(), Some(1));
    let stderr = text(&saida.stderr);
    assert!(stderr.contains("--url"), "stderr: {stderr}");
    assert!(stderr.contains("POPH_MATRIX_URL_AS"), "stderr: {stderr}");
    sem_tokens(&stderr);
}

#[test]
fn url_pela_variavel_de_ambiente_funciona() {
    let pasta = pasta_de_teste("url-variavel");
    let config = config(&pasta, "ph");
    let ambiente = ambiente(
        &config,
        &[
            ("POPH_MATRIX_AS_TOKEN", CANARIO_AS),
            ("POPH_MATRIX_HS_TOKEN", CANARIO_HS),
            ("POPH_MATRIX_URL_AS", URL_HARNESS),
        ],
    );
    let registro = pasta.join("registro.yaml");
    let saida = poph(
        &pasta,
        &[
            "matrix",
            "registro",
            "--saida",
            registro.to_str().expect("utf-8"),
        ],
        &ambiente,
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
            .contains(&format!("url: \"{URL_HARNESS}\""))
    );
}

#[test]
fn verificar_recusa_url_trocada_nomeando_o_campo() {
    let pasta = pasta_de_teste("url-trocada");
    let config = config(&pasta, "ph");
    let ambiente = ambiente(
        &config,
        &[
            ("POPH_MATRIX_AS_TOKEN", CANARIO_AS),
            ("POPH_MATRIX_HS_TOKEN", CANARIO_HS),
        ],
    );
    let registro = pasta.join("registro.yaml");
    let gerado = poph(
        &pasta,
        &[
            "matrix",
            "registro",
            "--saida",
            registro.to_str().expect("utf-8"),
            "--url",
            URL_HARNESS,
        ],
        &ambiente,
    );
    assert_eq!(gerado.status.code(), Some(0));

    let conferido = poph(
        &pasta,
        &[
            "matrix",
            "registro",
            "--verificar",
            registro.to_str().expect("utf-8"),
            "--url",
            "http://outro-endereco:9000",
        ],
        &ambiente,
    );
    assert_eq!(conferido.status.code(), Some(1));
    let stderr = text(&conferido.stderr);
    assert!(stderr.contains("campo url"), "stderr: {stderr}");
    assert!(
        stderr.contains("http://outro-endereco:9000"),
        "stderr: {stderr}"
    );
    sem_tokens(&stderr);
    sem_tokens(&text(&conferido.stdout));
}

#[test]
fn verificar_recusa_prefixo_trocado_nomeando_os_namespaces() {
    let pasta = pasta_de_teste("prefixo-trocado");
    let config_ph = config(&pasta, "ph");
    let ambiente_ph = ambiente(
        &config_ph,
        &[
            ("POPH_MATRIX_AS_TOKEN", CANARIO_AS),
            ("POPH_MATRIX_HS_TOKEN", CANARIO_HS),
        ],
    );
    let registro = pasta.join("registro.yaml");
    let gerado = poph(
        &pasta,
        &[
            "matrix",
            "registro",
            "--saida",
            registro.to_str().expect("utf-8"),
            "--url",
            URL_HARNESS,
        ],
        &ambiente_ph,
    );
    assert_eq!(gerado.status.code(), Some(0));

    // A configuração passa a dizer outro prefixo; o arquivo continua com o antigo.
    let config_wa = config(&pasta, "wa");
    let ambiente_wa = ambiente(
        &config_wa,
        &[
            ("POPH_MATRIX_AS_TOKEN", CANARIO_AS),
            ("POPH_MATRIX_HS_TOKEN", CANARIO_HS),
        ],
    );
    let conferido = poph(
        &pasta,
        &[
            "matrix",
            "registro",
            "--verificar",
            registro.to_str().expect("utf-8"),
            "--url",
            URL_HARNESS,
        ],
        &ambiente_wa,
    );
    assert_eq!(conferido.status.code(), Some(1));
    let stderr = text(&conferido.stderr);
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
    sem_tokens(&stderr);
}

#[test]
fn verificar_recusa_token_diferente_do_ambiente_sem_mostrar_valor() {
    let pasta = pasta_de_teste("token-trocado");
    let config = config(&pasta, "ph");
    let registro = pasta.join("registro.yaml");
    let gerado = poph(
        &pasta,
        &[
            "matrix",
            "registro",
            "--saida",
            registro.to_str().expect("utf-8"),
            "--url",
            URL_HARNESS,
        ],
        &ambiente(
            &config,
            &[
                ("POPH_MATRIX_AS_TOKEN", CANARIO_AS),
                ("POPH_MATRIX_HS_TOKEN", CANARIO_HS),
            ],
        ),
    );
    assert_eq!(gerado.status.code(), Some(0));

    let conferido = poph(
        &pasta,
        &[
            "matrix",
            "registro",
            "--verificar",
            registro.to_str().expect("utf-8"),
            "--url",
            URL_HARNESS,
        ],
        &ambiente(
            &config,
            &[
                ("POPH_MATRIX_AS_TOKEN", "outro-token-qualquer"),
                ("POPH_MATRIX_HS_TOKEN", CANARIO_HS),
            ],
        ),
    );
    assert_eq!(conferido.status.code(), Some(1));
    let stderr = text(&conferido.stderr);
    assert!(stderr.contains("campo as_token"), "stderr: {stderr}");
    assert!(stderr.contains("POPH_MATRIX_AS_TOKEN"), "stderr: {stderr}");
    sem_tokens(&stderr);
    assert!(
        !stderr.contains("outro-token-qualquer"),
        "o valor do token não pode aparecer: {stderr}"
    );
}

#[test]
fn sem_secao_matrix_o_erro_diz_o_que_falta() {
    let pasta = pasta_de_teste("sem-secao");
    let arquivo = pasta.join("poph.toml");
    fs::write(&arquivo, "[dados]\ndiretorio = \"dados\"\n").expect("escreve a configuração");
    let ambiente = ambiente(
        &arquivo,
        &[
            ("POPH_MATRIX_AS_TOKEN", CANARIO_AS),
            ("POPH_MATRIX_HS_TOKEN", CANARIO_HS),
        ],
    );
    let saida = poph(
        &pasta,
        &[
            "matrix",
            "registro",
            "--saida",
            pasta.join("registro.yaml").to_str().expect("utf-8"),
            "--url",
            URL_HARNESS,
        ],
        &ambiente,
    );
    assert_eq!(saida.status.code(), Some(1));
    let stderr = text(&saida.stderr);
    assert!(stderr.contains("[matrix]"), "stderr: {stderr}");
    sem_tokens(&stderr);
}

#[test]
fn verificar_arquivo_inexistente_nao_passa_em_silencio() {
    let pasta = pasta_de_teste("inexistente");
    let config = config(&pasta, "ph");
    let ambiente = ambiente(
        &config,
        &[
            ("POPH_MATRIX_AS_TOKEN", CANARIO_AS),
            ("POPH_MATRIX_HS_TOKEN", CANARIO_HS),
        ],
    );
    let saida = poph(
        &pasta,
        &[
            "matrix",
            "registro",
            "--verificar",
            pasta.join("nao-existe.yaml").to_str().expect("utf-8"),
            "--url",
            URL_HARNESS,
        ],
        &ambiente,
    );
    assert_eq!(saida.status.code(), Some(1));
    assert!(text(&saida.stderr).contains("não consegui ler"));
}

#[test]
fn saida_e_verificar_juntos_sao_recusados() {
    let pasta = pasta_de_teste("dois-modos");
    let config = config(&pasta, "ph");
    let ambiente = ambiente(&config, &[]);
    let saida = poph(
        &pasta,
        &[
            "matrix",
            "registro",
            "--saida",
            "a.yaml",
            "--verificar",
            "b.yaml",
        ],
        &ambiente,
    );
    assert_eq!(saida.status.code(), Some(1));
    assert!(text(&saida.stderr).contains("não os dois"));
}
