//! `poph cluster show|check` de ponta a ponta: binário de verdade, arquivos de verdade.
//!
//! O que se prova:
//! - sem `cluster.toml`, os dois comandos dizem que a instalação roda em modo instância única;
//! - com o arquivo, `show` traz instância local, orgs por instância e impressão digital;
//! - `show --json` tem formato estável (`versao_relatorio: 1`) e nenhum segredo;
//! - mapa inválido (`check`) sai com código 1 e aponta a chave; arquivo com TOML quebrado também;
//! - cluster sem `POPH_CLUSTER_SECRET` é recusado, sem imprimir o valor de segredo nenhum;
//! - `POPH_CLUSTER` aponta o arquivo fora do diretório do `poph.toml`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const SEGREDO: &str = "SENTINELA-SEGREDO-CLUSTER-cli";
const MAPA: &str = r#"
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

fn pasta_de_teste(rotulo: &str) -> PathBuf {
    let pasta =
        std::env::temp_dir().join(format!("poph-cli-cluster-{rotulo}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&pasta);
    fs::create_dir_all(&pasta).expect("cria pasta de teste");
    pasta
}

fn poph(pasta_atual: &Path, args: &[&str], variaveis: &[(&str, &str)]) -> Output {
    let mut comando = Command::new(env!("CARGO_BIN_EXE_poph"));
    comando.args(args).current_dir(pasta_atual).env_clear();
    for (chave, valor) in variaveis {
        comando.env(chave, valor);
    }
    comando.output().expect("executa o poph")
}

fn texto(saida: &[u8]) -> String {
    String::from_utf8_lossy(saida).into_owned()
}

#[test]
fn sem_cluster_toml_e_modo_instancia_unica() {
    let pasta = pasta_de_teste("unica");
    let show = poph(&pasta, &["cluster", "show"], &[]);
    let stdout = texto(&show.stdout);
    assert_eq!(show.status.code(), Some(0), "stdout: {stdout}");
    assert!(stdout.contains("modo: instância única"), "stdout: {stdout}");
    assert!(!stdout.contains(SEGREDO));

    let check = poph(&pasta, &["cluster", "check"], &[]);
    let stdout = texto(&check.stdout);
    assert_eq!(check.status.code(), Some(0), "stdout: {stdout}");
    assert!(stdout.contains("Nada a corrigir"), "stdout: {stdout}");
}

#[test]
fn show_mostra_o_mapa_com_instancia_local_e_orgs_por_instancia() {
    let pasta = pasta_de_teste("mapa");
    fs::write(pasta.join("cluster.toml"), MAPA).expect("cluster.toml");
    let variaveis = [("POPH_CLUSTER_SECRET", SEGREDO)];
    let saida = poph(&pasta, &["cluster", "show"], &variaveis);
    let stdout = texto(&saida.stdout);
    assert_eq!(saida.status.code(), Some(0), "stdout: {stdout}");
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
    assert!(
        !stdout.contains(SEGREDO),
        "o valor do segredo não pode sair"
    );

    let check = poph(&pasta, &["cluster", "check"], &variaveis);
    let stdout = texto(&check.stdout);
    assert_eq!(check.status.code(), Some(0), "stdout: {stdout}");
    assert!(
        stdout.contains("Mapa do cluster válido"),
        "stdout: {stdout}"
    );
}

#[test]
fn show_json_tem_formato_estavel_e_nao_traz_segredo() {
    let pasta = pasta_de_teste("json");
    fs::write(pasta.join("cluster.toml"), MAPA).expect("cluster.toml");
    let saida = poph(
        &pasta,
        &["cluster", "show", "--json"],
        &[("POPH_CLUSTER_SECRET", SEGREDO)],
    );
    let stdout = texto(&saida.stdout);
    assert_eq!(saida.status.code(), Some(0), "stdout: {stdout}");
    assert!(!stdout.contains(SEGREDO));
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("stdout é JSON");
    assert_eq!(json["versao_relatorio"], 1);
    assert_eq!(json["mapa"]["modo"], "cluster");
    assert_eq!(json["mapa"]["instance_id"], "a");
    assert_eq!(json["mapa"]["segredo_configurado"], true);
    assert_eq!(json["mapa"]["orgs"]["org_01"], "a");
    assert_eq!(json["fingerprint"].as_str().map(str::len), Some(32));
}

#[test]
fn mapa_invalido_reprova_o_check_com_a_chave() {
    let pasta = pasta_de_teste("invalido");
    let invalido = MAPA.replace("org_01 = \"a\"", "org_01 = \"c\"");
    fs::write(pasta.join("cluster.toml"), invalido).expect("cluster.toml");
    let saida = poph(
        &pasta,
        &["cluster", "check"],
        &[("POPH_CLUSTER_SECRET", SEGREDO)],
    );
    let stdout = texto(&saida.stdout);
    assert_eq!(saida.status.code(), Some(1), "stdout: {stdout}");
    assert!(stdout.contains("orgs.org_01"), "stdout: {stdout}");
    assert!(!stdout.contains(SEGREDO));

    let show = poph(
        &pasta,
        &["cluster", "show"],
        &[("POPH_CLUSTER_SECRET", SEGREDO)],
    );
    assert_eq!(
        show.status.code(),
        Some(1),
        "mapa inválido não é mostrado como bom"
    );
}

#[test]
fn toml_quebrado_reprova_com_a_origem() {
    let pasta = pasta_de_teste("toml");
    fs::write(pasta.join("cluster.toml"), "[orgs\norg_01 = \"a\"\n").expect("cluster.toml");
    let saida = poph(
        &pasta,
        &["cluster", "check"],
        &[("POPH_CLUSTER_SECRET", SEGREDO)],
    );
    let stderr = texto(&saida.stderr);
    assert_eq!(saida.status.code(), Some(1));
    assert!(stderr.contains("cluster.toml"), "stderr: {stderr}");
    assert!(stderr.contains("sintaxe"), "stderr: {stderr}");
}

#[test]
fn cluster_sem_segredo_no_ambiente_e_recusado() {
    let pasta = pasta_de_teste("sem-segredo");
    fs::write(pasta.join("cluster.toml"), MAPA).expect("cluster.toml");
    let saida = poph(&pasta, &["cluster", "check"], &[]);
    let stderr = texto(&saida.stderr);
    assert_eq!(saida.status.code(), Some(1), "stderr: {stderr}");
    assert!(stderr.contains("POPH_CLUSTER_SECRET"), "stderr: {stderr}");
}

#[test]
fn poph_cluster_aponta_o_arquivo_fora_da_pasta_do_poph_toml() {
    let pasta = pasta_de_teste("override");
    // `poph.toml` na pasta, mas o mapa num arquivo separado indicado por POPH_CLUSTER.
    fs::write(pasta.join("poph.toml"), "[servidor]\nporta = 7499\n").expect("poph.toml");
    let outro = pasta.join("outro-lugar.toml");
    fs::write(&outro, MAPA).expect("cluster alternativo");
    let variaveis = [
        ("POPH_CLUSTER_SECRET", SEGREDO),
        ("POPH_CLUSTER", outro.to_str().expect("caminho UTF-8")),
    ];
    let saida = poph(&pasta, &["cluster", "show"], &variaveis);
    let stdout = texto(&saida.stdout);
    assert_eq!(saida.status.code(), Some(0), "stdout: {stdout}");
    assert!(
        stdout.contains("outro-lugar.toml"),
        "o caminho de POPH_CLUSTER aparece: {stdout}"
    );
    assert!(stdout.contains("modo: cluster"), "stdout: {stdout}");
}

#[test]
fn fingerprint_do_mapa_e_igual_entre_instancias_do_mesmo_cluster() {
    // HR-50b: o `x-poph-mapa` compara o conteúdo compartilhado, não o nome local. Duas instâncias
    // com o mesmo cluster.toml (só o `instance_id` muda) têm a mesma impressão do mapa e
    // impressões locais diferentes.
    let pasta_a = pasta_de_teste("mapa-a");
    let pasta_b = pasta_de_teste("mapa-b");
    fs::write(pasta_a.join("cluster.toml"), MAPA).expect("cluster.toml de a");
    fs::write(
        pasta_b.join("cluster.toml"),
        MAPA.replace("instance_id = \"a\"", "instance_id = \"b\""),
    )
    .expect("cluster.toml de b");

    let de_a = poph(
        &pasta_a,
        &["cluster", "show", "--json"],
        &[("POPH_CLUSTER_SECRET", SEGREDO)],
    );
    let de_b = poph(
        &pasta_b,
        &["cluster", "show", "--json"],
        &[("POPH_CLUSTER_SECRET", SEGREDO)],
    );
    let a: serde_json::Value = serde_json::from_str(&texto(&de_a.stdout)).expect("json de a");
    let b: serde_json::Value = serde_json::from_str(&texto(&de_b.stdout)).expect("json de b");
    assert_eq!(a["fingerprint_mapa"].as_str().map(str::len), Some(32));
    assert_eq!(a["fingerprint_mapa"], b["fingerprint_mapa"]);
    assert_ne!(a["fingerprint"], b["fingerprint"]);

    let texto_de_a = poph(
        &pasta_a,
        &["cluster", "show"],
        &[("POPH_CLUSTER_SECRET", SEGREDO)],
    );
    let stdout = texto(&texto_de_a.stdout);
    assert!(
        stdout.contains("fingerprint do mapa (sem a instância local): "),
        "stdout: {stdout}"
    );
    assert!(!stdout.contains(SEGREDO), "segredo no stdout");
}

// ───────────── `cluster check --remoto` (HR-50b-2): mapa entre instâncias ─────────────

/// Serve uma única resposta JSON em cada conexão, num `/healthz` falso, e devolve o endereço.
///
/// O listener já vem aberto para o endereço entrar no mapa **antes** de o teste calcular a
/// impressão digital (a URL faz parte do texto canônico).
fn servir_healthz(listener: std::net::TcpListener, corpo: String) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        use std::io::{Read, Write};
        for fluxo in listener.incoming().take(8) {
            let Ok(mut fluxo) = fluxo else { continue };
            let mut buffer = [0u8; 4096];
            let _ = fluxo.read(&mut buffer);
            let resposta = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\
                 connection: close\r\n\r\n{corpo}",
                corpo.len()
            );
            let _ = fluxo.write_all(resposta.as_bytes());
        }
    })
}

fn mapa_com_vizinha(url_b: &str) -> String {
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

/// `--remoto` com o mapa igual nas duas pontas: sai 0 e diz que está tudo certo.
#[test]
fn check_remoto_aprova_mapa_igual_entre_instancias() {
    let pasta = pasta_de_teste("remoto-ok");
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("porta livre");
    let url_b = format!("http://{}", listener.local_addr().expect("endereço"));
    fs::write(pasta.join("cluster.toml"), mapa_com_vizinha(&url_b)).expect("mapa");

    // A impressão digital do mapa é calculada antes de o servidor falso responder.
    let show = poph(
        &pasta,
        &["cluster", "show", "--json"],
        &[("POPH_CLUSTER_SECRET", SEGREDO)],
    );
    assert_eq!(show.status.code(), Some(0), "{}", texto(&show.stderr));
    let relatorio: serde_json::Value =
        serde_json::from_str(&texto(&show.stdout)).expect("json do show");
    let fingerprint = relatorio["fingerprint_mapa"]
        .as_str()
        .expect("fingerprint do mapa")
        .to_owned();

    let corpo = format!(
        r#"{{"status":"ok","version":"0.1.0","uptime_seconds":1,"databases":{{"harness":1,"rag":0}},"instance_id":"b","cluster_map_fingerprint":"{fingerprint}"}}"#
    );
    // O servidor falso fica de pé até o fim do processo de teste (não se junta à thread).
    let _servidor = servir_healthz(listener, corpo);

    let check = poph(
        &pasta,
        &["cluster", "check", "--remoto"],
        &[("POPH_CLUSTER_SECRET", SEGREDO)],
    );
    let stdout = texto(&check.stdout);
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
    assert!(!stdout.contains(SEGREDO), "segredo na saída: {stdout}");
}

/// `--remoto` com mapa divergente numa ponta e a outra fora do ar: sai 1 e aponta as duas.
#[test]
fn check_remoto_reprova_mapa_divergente_e_instancia_fora_do_ar() {
    let pasta = pasta_de_teste("remoto-divergente");
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("porta livre");
    let url_b = format!("http://{}", listener.local_addr().expect("endereço"));
    // Porta que ninguém escuta (abre e fecha em seguida).
    let fora = std::net::TcpListener::bind("127.0.0.1:0").expect("porta livre");
    let url_c = format!("http://{}", fora.local_addr().expect("endereço"));
    drop(fora);

    let mapa = format!(
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
    fs::write(pasta.join("cluster.toml"), mapa).expect("mapa");

    let corpo = r#"{"status":"ok","version":"0.1.0","uptime_seconds":1,"databases":{"harness":1,"rag":0},"instance_id":"b","cluster_map_fingerprint":"outro-mapa-qualquer"}"#.to_owned();
    // O servidor falso fica de pé até o fim do processo de teste (não se junta à thread).
    let _servidor = servir_healthz(listener, corpo);

    let check = poph(
        &pasta,
        &["cluster", "check", "--remoto"],
        &[("POPH_CLUSTER_SECRET", SEGREDO)],
    );
    let stdout = texto(&check.stdout);
    assert_eq!(check.status.code(), Some(1), "stdout: {stdout}");
    assert!(stdout.contains("mapa divergente"), "stdout: {stdout}");
    assert!(stdout.contains("inacessível"), "stdout: {stdout}");
    assert!(
        stdout.contains("2 instância(s) com problema"),
        "stdout: {stdout}"
    );

    // Em JSON: `ok: false`, um problema remoto por instância e nenhum segredo.
    let json = poph(
        &pasta,
        &["cluster", "check", "--remoto", "--json"],
        &[("POPH_CLUSTER_SECRET", SEGREDO)],
    );
    let saida = texto(&json.stdout);
    let relatorio: serde_json::Value = serde_json::from_str(&saida).expect("json do check");
    assert_eq!(relatorio["ok"], serde_json::json!(false), "{saida}");
    assert_eq!(
        relatorio["problemas_remotos"].as_array().map(Vec::len),
        Some(2),
        "{saida}"
    );
    assert_eq!(
        relatorio["remoto"].as_array().map(Vec::len),
        Some(2),
        "{saida}"
    );
    assert!(!saida.contains(SEGREDO), "segredo na saída: {saida}");
}

/// `--remoto` numa instalação sem cluster: nada a consultar, sai 0 e diz isso.
#[test]
fn check_remoto_sem_cluster_nao_consulta_nada() {
    let pasta = pasta_de_teste("remoto-sem-cluster");
    let check = poph(&pasta, &["cluster", "check", "--remoto"], &[]);
    let stdout = texto(&check.stdout);
    assert_eq!(check.status.code(), Some(0), "stdout: {stdout}");
    assert!(stdout.contains("modo instância única"), "stdout: {stdout}");
    assert!(!stdout.contains("consulta remota"), "stdout: {stdout}");
}
