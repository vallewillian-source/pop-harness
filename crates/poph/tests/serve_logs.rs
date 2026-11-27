use std::fs;
use std::io::{Read, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use poph_store::{NewAgent, NewOrg, Store};
use poph_types::{HarnessConfig, IntegrationId, OrgId};
use serde_json::json;

const SECRET_VAR: &str = "POPH_TESTE_SEGREDO_HR26F";
const SECRET: &str = "segredo-webhook-hr26f-certo";
const WRONG_SECRET: &str = "segredo-webhook-hr26f-errado";
const EVOLUTION_KEY: &str = "chave-evolution-hr26f-nao-pode-sair";
const INSTANCE_TOKEN: &str = "token-instancia-hr26f-nao-pode-sair";
const PHONE: &str = "5511987650000";
const JID: &str = "5511987650000@s.whatsapp.net";

struct Setup {
    dir: PathBuf,
    org: OrgId,
    porta: u16,
}

fn free_port() -> u16 {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("porta livre");
    listener.local_addr().expect("endereço").port()
}

fn prepare(label: &str) -> Setup {
    let dir = std::env::temp_dir().join(format!("poph-hr26f-log-{label}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("dados")).expect("pasta de dados");

    let mut config = HarnessConfig::default();
    config.data.directory = dir.join("dados");
    let store = Store::open(&config).expect("cria o banco");
    let org = store
        .orgs()
        .create(NewOrg {
            name: "Loja HR-26f".to_owned(),
            slug: format!("loja-hr26f-{label}"),
            config_json: String::new(),
        })
        .expect("cria a org")
        .id;
    let agente = store
        .agents()
        .create(
            org,
            NewAgent {
                name: "Atendente HR-26f".to_owned(),
                slug: format!("atendente-hr26f-{label}"),
                ..NewAgent::default()
            },
        )
        .expect("cria o agente")
        .id;
    let integracao: IntegrationId = store
        .create_integration(org, agente, "whatsapp".to_owned(), "active".to_owned())
        .expect("cria a integração")
        .id
        .parse()
        .expect("id da integração");
    drop(store);

    let porta = free_port();
    let texto = format!(
        "[dados]\ndiretorio = \"{dados}\"\n\n\
         [servidor]\nporta = {porta}\n\n\
         [log]\nnivel = \"info\"\n\n\
         [evolution]\nurl_base = \"http://127.0.0.1:18081\"\n\n\
         [[evolution.instancias]]\norg_id = \"{org}\"\ninstancia = \"loja\"\n\
         integracao_id = \"{integracao}\"\nsegredo_env = \"{SECRET_VAR}\"\n",
        dados = dir.join("dados").display(),
    );
    fs::write(dir.join("poph.toml"), texto).expect("grava a configuração");
    Setup { dir, org, porta }
}

struct Servidor {
    child: Child,
    error: Arc<Mutex<Vec<u8>>>,
    reader: Option<thread::JoinHandle<()>>,
}

fn start(setup: &Setup) -> Servidor {
    let mut child = Command::new(env!("CARGO_BIN_EXE_poph"))
        .args(["serve", "--config", "poph.toml"])
        .current_dir(&setup.dir)
        .env_clear()
        .env(SECRET_VAR, SECRET)
        .env("EVOLUTION_API_KEY", EVOLUTION_KEY)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("sobe o poph serve");

    let error = Arc::new(Mutex::new(Vec::new()));
    let mut saida = child.stderr.take().expect("erro padrão do processo");
    let stored = Arc::clone(&error);
    let reader = thread::spawn(move || {
        let mut buffer = [0u8; 4096];
        loop {
            match saida.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(n) => stored
                    .lock()
                    .expect("sem veneno")
                    .extend_from_slice(&buffer[..n]),
            }
        }
    });
    Servidor {
        child,
        error,
        reader: Some(reader),
    }
}

impl Servidor {
    fn log(&self) -> String {
        String::from_utf8_lossy(&self.error.lock().expect("sem veneno")).into_owned()
    }

    fn stop(mut self) -> String {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
        self.log()
    }
}

impl Drop for Servidor {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn http(
    porta: u16,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: &str,
) -> (u16, String) {
    let mut connection =
        TcpStream::connect((Ipv4Addr::LOCALHOST, porta)).expect("conecta ao servidor");
    connection
        .set_read_timeout(Some(Duration::from_secs(10)))
        .expect("tempo limite de leitura");
    let mut request =
        format!("{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n");
    for (nome, value) in headers {
        request.push_str(&format!("{nome}: {value}\r\n"));
    }
    request.push_str(&format!(
        "Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    ));
    connection
        .write_all(request.as_bytes())
        .expect("envia o pedido");
    let mut response = String::new();
    connection
        .read_to_string(&mut response)
        .expect("lê a resposta");
    let status = response
        .split_whitespace()
        .nth(1)
        .and_then(|texto| texto.parse::<u16>().ok())
        .expect("status da resposta");
    let body = response
        .split_once("\r\n\r\n")
        .map(|(_, c)| c.to_owned())
        .unwrap_or_default();
    (status, format!("{response}\n{body}"))
}

fn wait_until_up(porta: u16) {
    for _ in 0..200 {
        if let Ok(connection) = TcpStream::connect((Ipv4Addr::LOCALHOST, porta)) {
            drop(connection);
            let (status, _) = http(porta, "GET", "/healthz", &[], "");
            if status == 200 {
                return;
            }
        }
        thread::sleep(Duration::from_millis(50));
    }
    panic!("o servidor não subiu na porta {porta}");
}

fn text_event() -> String {
    json!({
        "event": "messages.upsert",
        "instance": "loja",
        "apikey": INSTANCE_TOKEN,
        "destination": "http://harness.teste/webhook/evolution/x/loja",
        "data": {
            "key": {"id": "MSG-HR26F-1", "remoteJid": JID, "fromMe": false},
            "pushName": "Cliente de Teste",
            "messageType": "conversation",
            "message": {"conversation": "oi, quero um orçamento"},
            "messageTimestamp": 1795177000
        }
    })
    .to_string()
}

#[test]
fn server_logs_webhook_operation_without_sensitive_data() {
    let setup = prepare("ok");
    let servidor = start(&setup);
    wait_until_up(setup.porta);

    let path = format!("/webhook/evolution/{}/loja", setup.org);
    let body = text_event();
    let (ok, ok_response) = http(
        setup.porta,
        "POST",
        &path,
        &[("X-Evolution-Webhook-Secret", SECRET)],
        &body,
    );
    assert_eq!(ok, 200, "entrega com segredo certo: {ok_response}");

    let (wrong, wrong_response) = http(
        setup.porta,
        "POST",
        &path,
        &[("X-Evolution-Webhook-Secret", WRONG_SECRET)],
        &body,
    );
    assert_eq!(wrong, 401, "entrega com segredo errado: {wrong_response}");

    let registro = servidor.stop();
    let _ = fs::remove_dir_all(&setup.dir);

    assert!(
        registro.contains("servidor no ar"),
        "a linha de subida não apareceu: o log não está sendo capturado\n{registro}"
    );
    assert!(
        registro.contains("mensagem do WhatsApp registrada"),
        "a entrada do webhook não deixou rastro\n{registro}"
    );
    let row = registro
        .lines()
        .find(|row| row.contains("mensagem do WhatsApp registrada"))
        .unwrap_or_default();
    assert!(
        row.contains("nova"),
        "o rastro não diz que a mensagem entrou como nova: {row}"
    );

    let forbidden = [
        PHONE,
        JID,
        "@s.whatsapp.net",
        EVOLUTION_KEY,
        INSTANCE_TOKEN,
        SECRET,
        WRONG_SECRET,
        "oi, quero um orçamento",
    ];
    for forbidden_item in forbidden {
        assert!(
            !registro.contains(forbidden_item),
            "o log vazou `{forbidden_item}`:\n{registro}"
        );
        assert!(
            !ok_response.contains(forbidden_item) && !wrong_response.contains(forbidden_item),
            "uma resposta devolveu `{forbidden_item}`"
        );
    }
}

fn state_event() -> String {
    json!({
        "event": "connection.update",
        "instance": "loja",
        "apikey": INSTANCE_TOKEN,
        "destination": "http://harness.teste/webhook/evolution/x/loja",
        "data": {"state": "open"}
    })
    .to_string()
}

#[test]
fn server_logs_connection_state_without_sensitive_data() {
    let setup = prepare("estado");
    let servidor = start(&setup);
    wait_until_up(setup.porta);

    let path = format!("/webhook/evolution/{}/loja", setup.org);
    let (status, response) = http(
        setup.porta,
        "POST",
        &path,
        &[("X-Evolution-Webhook-Secret", SECRET)],
        &state_event(),
    );
    assert_eq!(
        status, 200,
        "entrega de estado com segredo certo: {response}"
    );

    let registro = servidor.stop();
    let _ = fs::remove_dir_all(&setup.dir);

    assert!(
        registro.contains("servidor no ar"),
        "a linha de subida não apareceu: o log não está sendo capturado\n{registro}"
    );
    let row = registro
        .lines()
        .find(|row| row.contains("estado de conexão do WhatsApp atualizado"))
        .unwrap_or_default();
    assert!(
        !row.is_empty(),
        "a atualização de estado não deixou rastro\n{registro}"
    );
    assert!(
        row.contains("connected"),
        "a linha não diz o estado novo: {row}"
    );
    assert!(row.contains("loja"), "a linha não diz a instância: {row}");

    for forbidden_item in [SECRET, EVOLUTION_KEY, INSTANCE_TOKEN] {
        assert!(
            !registro.contains(forbidden_item),
            "o log vazou `{forbidden_item}`:\n{registro}"
        );
    }
}
