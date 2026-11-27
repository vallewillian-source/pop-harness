use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const SECRET: &str = "SENTINELA-SEGREDO-CLUSTER-cli";
const MAP: &str = r#"
instance_id = "a"
default_instance = "b"

[[instances]]
id = "a"
url = "http://127.0.0.1:7481"

[[instances]]
id = "b"
url = "http://127.0.0.1:7482"

[orgs]
org_01 = "a"
org_02 = "b"
"#;

fn test_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("poph-cli-cluster-{label}-{}", std::process::id()));
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

fn text(output: &[u8]) -> String {
    String::from_utf8_lossy(output).into_owned()
}

#[test]
fn no_cluster_toml_means_single_instance_mode() {
    let dir = test_dir("unica");
    let show = poph(&dir, &["cluster", "show"], &[]);
    let stdout = text(&show.stdout);
    assert_eq!(show.status.code(), Some(0), "stdout: {stdout}");
    assert!(stdout.contains("modo: instância única"), "stdout: {stdout}");
    assert!(!stdout.contains(SECRET));

    let check = poph(&dir, &["cluster", "check"], &[]);
    let stdout = text(&check.stdout);
    assert_eq!(check.status.code(), Some(0), "stdout: {stdout}");
    assert!(stdout.contains("Nada a corrigir"), "stdout: {stdout}");
}

#[test]
fn show_prints_map_with_local_instance_and_orgs_per_instance() {
    let dir = test_dir("mapa");
    fs::write(dir.join("cluster.toml"), MAP).expect("cluster.toml");
    let variables = [("POPH_CLUSTER_SECRET", SECRET)];
    let output = poph(&dir, &["cluster", "show"], &variables);
    let stdout = text(&output.stdout);
    assert_eq!(output.status.code(), Some(0), "stdout: {stdout}");
    assert!(
        stdout.contains("modo: cluster (2 instância(s))"),
        "stdout: {stdout}"
    );
    assert!(stdout.contains("instância local: a"), "stdout: {stdout}");
    assert!(stdout.contains("instância padrão: b"), "stdout: {stdout}");
    assert!(stdout.contains("org_01"), "stdout: {stdout}");
    assert!(
        stdout.contains("segredo compartilhado: definido"),
        "stdout: {stdout}"
    );
    assert!(stdout.contains("fingerprint: "), "stdout: {stdout}");
    assert!(!stdout.contains(SECRET), "o valor do segredo não pode sair");

    let check = poph(&dir, &["cluster", "check"], &variables);
    let stdout = text(&check.stdout);
    assert_eq!(check.status.code(), Some(0), "stdout: {stdout}");
    assert!(
        stdout.contains("Mapa do cluster válido"),
        "stdout: {stdout}"
    );
}

#[test]
fn show_json_has_stable_format_and_no_secret() {
    let dir = test_dir("json");
    fs::write(dir.join("cluster.toml"), MAP).expect("cluster.toml");
    let output = poph(
        &dir,
        &["cluster", "show", "--json"],
        &[("POPH_CLUSTER_SECRET", SECRET)],
    );
    let stdout = text(&output.stdout);
    assert_eq!(output.status.code(), Some(0), "stdout: {stdout}");
    assert!(!stdout.contains(SECRET));
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("stdout é JSON");
    assert_eq!(json["versao_relatorio"], 1);
    assert_eq!(json["mapa"]["modo"], "cluster");
    assert_eq!(json["mapa"]["instance_id"], "a");
    assert_eq!(json["mapa"]["segredo_configurado"], true);
    assert_eq!(json["mapa"]["orgs"]["org_01"], "a");
    assert_eq!(json["fingerprint"].as_str().map(str::len), Some(32));
}

#[test]
fn invalid_map_fails_check_pointing_at_key() {
    let dir = test_dir("invalido");
    let invalid = MAP.replace("org_01 = \"a\"", "org_01 = \"c\"");
    fs::write(dir.join("cluster.toml"), invalid).expect("cluster.toml");
    let output = poph(
        &dir,
        &["cluster", "check"],
        &[("POPH_CLUSTER_SECRET", SECRET)],
    );
    let stdout = text(&output.stdout);
    assert_eq!(output.status.code(), Some(1), "stdout: {stdout}");
    assert!(stdout.contains("orgs.org_01"), "stdout: {stdout}");
    assert!(!stdout.contains(SECRET));

    let show = poph(
        &dir,
        &["cluster", "show"],
        &[("POPH_CLUSTER_SECRET", SECRET)],
    );
    assert_eq!(
        show.status.code(),
        Some(1),
        "mapa inválido não é mostrado como bom"
    );
}

#[test]
fn broken_toml_fails_with_source() {
    let dir = test_dir("toml");
    fs::write(dir.join("cluster.toml"), "[orgs\norg_01 = \"a\"\n").expect("cluster.toml");
    let output = poph(
        &dir,
        &["cluster", "check"],
        &[("POPH_CLUSTER_SECRET", SECRET)],
    );
    let stderr = text(&output.stderr);
    assert_eq!(output.status.code(), Some(1));
    assert!(stderr.contains("cluster.toml"), "stderr: {stderr}");
    assert!(stderr.contains("sintaxe"), "stderr: {stderr}");
}

#[test]
fn cluster_without_secret_in_environment_is_refused() {
    let dir = test_dir("sem-segredo");
    fs::write(dir.join("cluster.toml"), MAP).expect("cluster.toml");
    let output = poph(&dir, &["cluster", "check"], &[]);
    let stderr = text(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "stderr: {stderr}");
    assert!(stderr.contains("POPH_CLUSTER_SECRET"), "stderr: {stderr}");
}

#[test]
fn poph_cluster_points_at_file_outside_poph_toml_dir() {
    let dir = test_dir("override");
    fs::write(dir.join("poph.toml"), "[servidor]\nporta = 7499\n").expect("poph.toml");
    let other = dir.join("outro-lugar.toml");
    fs::write(&other, MAP).expect("cluster alternativo");
    let variables = [
        ("POPH_CLUSTER_SECRET", SECRET),
        ("POPH_CLUSTER", other.to_str().expect("caminho UTF-8")),
    ];
    let output = poph(&dir, &["cluster", "show"], &variables);
    let stdout = text(&output.stdout);
    assert_eq!(output.status.code(), Some(0), "stdout: {stdout}");
    assert!(
        stdout.contains("outro-lugar.toml"),
        "o caminho de POPH_CLUSTER aparece: {stdout}"
    );
    assert!(stdout.contains("modo: cluster"), "stdout: {stdout}");
}

#[test]
fn map_fingerprint_is_equal_across_instances_of_same_cluster() {
    let dir_a = test_dir("mapa-a");
    let dir_b = test_dir("mapa-b");
    fs::write(dir_a.join("cluster.toml"), MAP).expect("cluster.toml de a");
    fs::write(
        dir_b.join("cluster.toml"),
        MAP.replace("instance_id = \"a\"", "instance_id = \"b\""),
    )
    .expect("cluster.toml de b");

    let from_a = poph(
        &dir_a,
        &["cluster", "show", "--json"],
        &[("POPH_CLUSTER_SECRET", SECRET)],
    );
    let from_b = poph(
        &dir_b,
        &["cluster", "show", "--json"],
        &[("POPH_CLUSTER_SECRET", SECRET)],
    );
    let a: serde_json::Value = serde_json::from_str(&text(&from_a.stdout)).expect("json de a");
    let b: serde_json::Value = serde_json::from_str(&text(&from_b.stdout)).expect("json de b");
    assert_eq!(a["fingerprint_mapa"].as_str().map(str::len), Some(32));
    assert_eq!(a["fingerprint_mapa"], b["fingerprint_mapa"]);
    assert_ne!(a["fingerprint"], b["fingerprint"]);

    let text_from_a = poph(
        &dir_a,
        &["cluster", "show"],
        &[("POPH_CLUSTER_SECRET", SECRET)],
    );
    let stdout = text(&text_from_a.stdout);
    assert!(
        stdout.contains("fingerprint do mapa (sem a instância local): "),
        "stdout: {stdout}"
    );
    assert!(!stdout.contains(SECRET), "segredo no stdout");
}

fn serve_healthz(listener: std::net::TcpListener, body: String) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        use std::io::{Read, Write};
        for stream in listener.incoming().take(8) {
            let Ok(mut stream) = stream else { continue };
            let mut buffer = [0u8; 4096];
            let _ = stream.read(&mut buffer);
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\
                 connection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes());
        }
    })
}

fn map_with_neighbor(url_b: &str) -> String {
    format!(
        r#"
instance_id = "a"
default_instance = "b"

[[instances]]
id = "a"
url = "http://127.0.0.1:1"

[[instances]]
id = "b"
url = "{url_b}"

[orgs]
org_01 = "b"
"#
    )
}

#[test]
fn remote_check_approves_equal_map_across_instances() {
    let dir = test_dir("remoto-ok");
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("porta livre");
    let url_b = format!("http://{}", listener.local_addr().expect("endereço"));
    fs::write(dir.join("cluster.toml"), map_with_neighbor(&url_b)).expect("mapa");

    let show = poph(
        &dir,
        &["cluster", "show", "--json"],
        &[("POPH_CLUSTER_SECRET", SECRET)],
    );
    assert_eq!(show.status.code(), Some(0), "{}", text(&show.stderr));
    let report: serde_json::Value =
        serde_json::from_str(&text(&show.stdout)).expect("json do show");
    let fingerprint = report["fingerprint_mapa"]
        .as_str()
        .expect("fingerprint do mapa")
        .to_owned();

    let body = format!(
        r#"{{"status":"ok","version":"0.1.0","uptime_seconds":1,"databases":{{"harness":1,"rag":0}},"instance_id":"b","cluster_map_fingerprint":"{fingerprint}"}}"#
    );
    let _server = serve_healthz(listener, body);

    let check = poph(
        &dir,
        &["cluster", "check", "--remoto"],
        &[("POPH_CLUSTER_SECRET", SECRET)],
    );
    let stdout = text(&check.stdout);
    assert_eq!(check.status.code(), Some(0), "stdout: {stdout}");
    assert!(
        stdout.contains("consulta remota (mapa local:"),
        "stdout: {stdout}"
    );
    assert!(stdout.contains("mapa "), "stdout: {stdout}");
    assert!(
        stdout.contains("respondem com o mesmo mapa"),
        "stdout: {stdout}"
    );
    assert!(!stdout.contains(SECRET), "segredo na saída: {stdout}");
}

#[test]
fn remote_check_fails_divergent_map_and_instance_down() {
    let dir = test_dir("remoto-divergente");
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("porta livre");
    let url_b = format!("http://{}", listener.local_addr().expect("endereço"));
    let down = std::net::TcpListener::bind("127.0.0.1:0").expect("porta livre");
    let url_c = format!("http://{}", down.local_addr().expect("endereço"));
    drop(down);

    let map = format!(
        r#"
instance_id = "a"
default_instance = "b"

[[instances]]
id = "a"
url = "http://127.0.0.1:1"

[[instances]]
id = "b"
url = "{url_b}"

[[instances]]
id = "c"
url = "{url_c}"

[orgs]
org_01 = "b"
"#
    );
    fs::write(dir.join("cluster.toml"), map).expect("mapa");

    let body = r#"{"status":"ok","version":"0.1.0","uptime_seconds":1,"databases":{"harness":1,"rag":0},"instance_id":"b","cluster_map_fingerprint":"outro-mapa-qualquer"}"#.to_owned();
    let _server = serve_healthz(listener, body);

    let check = poph(
        &dir,
        &["cluster", "check", "--remoto"],
        &[("POPH_CLUSTER_SECRET", SECRET)],
    );
    let stdout = text(&check.stdout);
    assert_eq!(check.status.code(), Some(1), "stdout: {stdout}");
    assert!(stdout.contains("mapa divergente"), "stdout: {stdout}");
    assert!(stdout.contains("inacessível"), "stdout: {stdout}");
    assert!(
        stdout.contains("2 instância(s) com problema"),
        "stdout: {stdout}"
    );

    let json = poph(
        &dir,
        &["cluster", "check", "--remoto", "--json"],
        &[("POPH_CLUSTER_SECRET", SECRET)],
    );
    let output = text(&json.stdout);
    let report: serde_json::Value = serde_json::from_str(&output).expect("json do check");
    assert_eq!(report["ok"], serde_json::json!(false), "{output}");
    assert_eq!(
        report["problemas_remotos"].as_array().map(Vec::len),
        Some(2),
        "{output}"
    );
    assert_eq!(
        report["remoto"].as_array().map(Vec::len),
        Some(2),
        "{output}"
    );
    assert!(!output.contains(SECRET), "segredo na saída: {output}");
}

#[test]
fn remote_check_without_cluster_consults_nothing() {
    let dir = test_dir("remoto-sem-cluster");
    let check = poph(&dir, &["cluster", "check", "--remoto"], &[]);
    let stdout = text(&check.stdout);
    assert_eq!(check.status.code(), Some(0), "stdout: {stdout}");
    assert!(stdout.contains("modo instância única"), "stdout: {stdout}");
    assert!(!stdout.contains("consulta remota"), "stdout: {stdout}");
}
