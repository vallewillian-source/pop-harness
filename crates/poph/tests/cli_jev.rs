use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const CHAVE: &str = "chave-de-teste-nao-vaza";

fn jev_server(choice: &'static str, confidence: f64, usage: Option<(u64, u64)>) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("porta efêmera");
    let porta = listener.local_addr().expect("endereço").port();
    std::thread::spawn(move || {
        for fluxo in listener.incoming().flatten() {
            let _ = handle(&fluxo, choice, confidence, usage);
        }
    });
    porta
}

fn handle(
    fluxo: &TcpStream,
    choice: &str,
    confidence: f64,
    usage: Option<(u64, u64)>,
) -> std::io::Result<()> {
    let mut reader = BufReader::new(fluxo);
    let mut headers = String::new();
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            return Ok(());
        }
        if line == "\r\n" || line == "\n" {
            break;
        }
        headers.push_str(&line);
    }
    let length = headers
        .lines()
        .find_map(|line| {
            line.to_ascii_lowercase()
                .strip_prefix("content-length:")
                .map(|value| value.trim().parse::<usize>().unwrap_or(0))
        })
        .unwrap_or(0);
    let mut body = vec![0_u8; length];
    reader.read_exact(&mut body)?;

    let mut payload = serde_json::json!({
        "answers": {
            "g1_transferir": {
                "type": "choice",
                "choice": choice,
                "probabilities": {choice: confidence},
                "confidence": confidence
            }
        }
    });
    if let Some((input, saida)) = usage {
        payload["usage"] = serde_json::json!({
            "input_tokens": input,
            "output_tokens": saida,
            "total_tokens": input + saida
        });
    }
    let json = payload.to_string();
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{json}",
        json.len()
    );
    let mut fluxo = fluxo;
    fluxo.write_all(response.as_bytes())?;
    fluxo.flush()
}

struct Scenario {
    dir: PathBuf,
    porta: u16,
}

impl Scenario {
    fn new(label: &str, choice: &'static str) -> Self {
        Self::detailed(label, choice, 0.93, Some((30, 4)))
    }

    fn detailed(
        label: &str,
        choice: &'static str,
        confidence: f64,
        usage: Option<(u64, u64)>,
    ) -> Self {
        let dir = std::env::temp_dir().join(format!("poph-cli-jev-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("dados")).expect("pasta de dados");
        let porta = jev_server(choice, confidence, usage);
        let config = format!(
            "[dados]\ndiretorio = \"{}\"\n\n[jev]\nativo = true\nurl_base = \"http://127.0.0.1:{porta}\"\n",
            dir.join("dados").display()
        );
        fs::write(dir.join("poph.toml"), config).expect("configuração");
        Self { dir, porta }
    }

    fn write_dataset(&self, content: &str) -> PathBuf {
        let path = self.dir.join("g1.jsonl");
        fs::write(&path, content).expect("conjunto");
        path
    }

    fn poph(&self, args: &[&str], with_key: bool) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_poph"));
        command
            .args(args)
            .current_dir(&self.dir)
            .env_clear()
            .env("POPH_CONFIG", self.dir.join("poph.toml"));
        if with_key {
            command.env("TYPESAFE_API_KEY", CHAVE);
        }
        command.output().expect("executa o poph")
    }

    fn jev_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.porta)
    }
}

impl Drop for Scenario {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn exit_code(saida: &Output) -> i32 {
    saida.status.code().unwrap_or(-1)
}

fn texto(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

const DATASET: &str = "\
{\"estado\": {\"canal\": \"cli\", \"mensagens\": [{\"papel\": \"cliente\", \"texto\": \"quero falar com um atendente\"}], \"transferencia_habilitada\": true}, \"esperado\": \"transferir_agora\"}
{\"estado\": {\"canal\": \"cli\", \"mensagens\": [{\"papel\": \"cliente\", \"texto\": \"qual o prazo?\"}], \"transferencia_habilitada\": true}, \"esperado\": \"continuar_agente\"}
";

#[test]
fn measures_the_dataset_and_prints_accuracy_abstention_and_latency() {
    let scenario = Scenario::new("mede", "transferir_agora");
    let file = scenario.write_dataset(DATASET);

    let saida = scenario.poph(
        &["jev", "eval", "g1", file.to_str().expect("caminho")],
        true,
    );

    assert_eq!(exit_code(&saida), 0, "stderr: {}", texto(&saida.stderr));
    let printed = texto(&saida.stdout);
    assert!(printed.contains("Jev · g1"), "saída: {printed}");
    assert!(printed.contains("casos: 2"), "saída: {printed}");
    assert!(printed.contains("acerto: 1/2"), "saída: {printed}");
    assert!(printed.contains("erros: 1"), "saída: {printed}");
    assert!(printed.contains("latência:"), "saída: {printed}");
    assert!(!printed.contains(CHAVE), "a chave não pode aparecer");
    let _ = scenario.jev_url();
}

#[test]
fn json_report_is_stable() {
    let scenario = Scenario::new("json", "transferir_agora");
    let file = scenario.write_dataset(DATASET);

    let saida = scenario.poph(
        &[
            "jev",
            "eval",
            "g1",
            file.to_str().expect("caminho"),
            "--json",
        ],
        true,
    );

    assert_eq!(exit_code(&saida), 0, "stderr: {}", texto(&saida.stderr));
    let value: serde_json::Value =
        serde_json::from_str(&texto(&saida.stdout)).expect("saída é JSON");
    assert_eq!(value["versao_relatorio"], 1);
    assert_eq!(value["relatorio"]["gate"], "g1");
    assert_eq!(value["relatorio"]["casos"], 2);
    assert_eq!(value["relatorio"]["acertos"], 1);
    assert_eq!(value["relatorio"]["erros"], 1);
    assert_eq!(value["relatorio"]["abstencoes"], 0);
    assert!(
        value["relatorio"]["detalhes"]
            .as_array()
            .is_some_and(|d| d.len() == 2)
    );
    assert!(!texto(&saida.stdout).contains(CHAVE));
}

#[test]
fn without_key_refuses_and_says_what_is_missing() {
    let scenario = Scenario::new("sem-chave", "transferir_agora");
    let file = scenario.write_dataset(DATASET);

    let saida = scenario.poph(
        &["jev", "eval", "g1", file.to_str().expect("caminho")],
        false,
    );

    assert_eq!(exit_code(&saida), 1);
    assert!(
        texto(&saida.stderr).contains("TYPESAFE_API_KEY"),
        "stderr: {}",
        texto(&saida.stderr)
    );
}

#[test]
fn unknown_gate_and_missing_file_fail_with_a_message() {
    let scenario = Scenario::new("erros", "transferir_agora");
    let file = scenario.write_dataset(DATASET);

    let gate = scenario.poph(
        &["jev", "eval", "g10", file.to_str().expect("caminho")],
        true,
    );
    assert_eq!(exit_code(&gate), 1);
    assert!(texto(&gate.stderr).contains("desconhecido"));

    let missing = scenario.poph(
        &["jev", "eval", "g1", "/caminho/que/nao/existe.jsonl"],
        true,
    );
    assert_eq!(exit_code(&missing), 1);
    assert!(texto(&missing.stderr).contains("não consegui ler"));

    let bad_json = scenario.write_dataset("{ isto não é json }\n");
    let bad = scenario.poph(
        &["jev", "eval", "g1", bad_json.to_str().expect("caminho")],
        true,
    );
    assert_eq!(exit_code(&bad), 1);
    assert!(texto(&bad.stderr).contains("caso inválido"));
}

#[test]
fn help_lists_the_subcommand() {
    let scenario = Scenario::new("ajuda", "transferir_agora");
    let saida = scenario.poph(&["--help"], false);
    let printed = texto(&saida.stdout);
    assert!(printed.contains("jev"), "saída: {printed}");
    assert!(Path::new(&scenario.dir.join("poph.toml")).exists());
}

#[test]
fn abstention_shows_choice_and_reason() {
    let scenario = Scenario::detailed("abstencao", "transferir_agora", 0.5, Some((30, 4)));
    let file = scenario.write_dataset(DATASET);

    let saida = scenario.poph(
        &["jev", "eval", "g1", file.to_str().expect("caminho")],
        true,
    );
    assert_eq!(exit_code(&saida), 0, "stderr: {}", texto(&saida.stderr));
    let printed = texto(&saida.stdout);
    assert!(printed.contains("detalhe por caso"), "saída: {printed}");
    assert!(
        printed.contains("jev transferir_agora"),
        "a escolha do modelo aparece no fallback: {printed}"
    );
    assert!(
        printed.contains("motivo: confiança 0.50 abaixo do limiar 0.70"),
        "saída: {printed}"
    );
    assert!(
        printed.contains("tokens: entrada 60 · saída 8 · total 68"),
        "usage somado das duas chamadas: {printed}"
    );

    let saida = scenario.poph(
        &[
            "jev",
            "eval",
            "g1",
            file.to_str().expect("caminho"),
            "--json",
        ],
        true,
    );
    assert_eq!(exit_code(&saida), 0, "stderr: {}", texto(&saida.stderr));
    let value: serde_json::Value =
        serde_json::from_str(&texto(&saida.stdout)).expect("saída é JSON");
    assert_eq!(value["versao_relatorio"], 1);
    assert_eq!(value["relatorio"]["abstencoes"], 2);
    assert_eq!(value["relatorio"]["erros"], 0);
    assert_eq!(
        value["relatorio"]["detalhes"][0]["escolha"],
        "transferir_agora"
    );
    assert_eq!(
        value["relatorio"]["detalhes"][0]["obtido"],
        serde_json::Value::Null
    );
    assert!(
        value["relatorio"]["detalhes"][0]["motivo"]
            .as_str()
            .unwrap_or_default()
            .contains("abaixo do limiar")
    );
    assert_eq!(value["relatorio"]["usage"]["entrada"], 60);
    assert_eq!(value["relatorio"]["usage"]["saida"], 8);
    assert_eq!(value["relatorio"]["usage"]["total"], 68);
    assert_eq!(value["relatorio"]["usage"]["informado"], true);
    assert!(!texto(&saida.stdout).contains(CHAVE));
}

#[test]
fn without_usage_says_not_reported() {
    let scenario = Scenario::detailed("sem-usage", "continuar_agente", 0.9, None);
    let file = scenario.write_dataset(DATASET);

    let saida = scenario.poph(
        &["jev", "eval", "g1", file.to_str().expect("caminho")],
        true,
    );
    assert_eq!(exit_code(&saida), 0, "stderr: {}", texto(&saida.stderr));
    assert!(
        texto(&saida.stdout).contains("tokens: não informado"),
        "saída: {}",
        texto(&saida.stdout)
    );

    let saida = scenario.poph(
        &[
            "jev",
            "eval",
            "g1",
            file.to_str().expect("caminho"),
            "--json",
        ],
        true,
    );
    let value: serde_json::Value =
        serde_json::from_str(&texto(&saida.stdout)).expect("saída é JSON");
    assert_eq!(value["relatorio"]["usage"]["informado"], false);
    assert_eq!(
        value["relatorio"]["usage"]["entrada"],
        serde_json::Value::Null
    );
}
