//! `poph serve` escreve log das operações e não escreve dado sensível (HR-26f).
//!
//! Sobe o binário de verdade (`poph serve`) sobre banco temporário, com uma instância da Evolution
//! configurada, e entrega um webhook de texto pela rota pública. Depois lê o erro padrão do processo.
//!
//! Prova em duas partes:
//! (i) a captura funciona: a linha de subida (`servidor no ar`) e a da mensagem registrada aparecem;
//! (ii) nada sensível aparece no log, nem na resposta: telefone, JID, chave da Evolution, token da
//!      instância, segredo certo e o segredo errado que foi enviado.

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

const VAR_SEGREDO: &str = "POPH_TESTE_SEGREDO_HR26F";
const SEGREDO: &str = "segredo-webhook-hr26f-certo";
const SEGREDO_ERRADO: &str = "segredo-webhook-hr26f-errado";
const CHAVE_EVO: &str = "chave-evolution-hr26f-nao-pode-sair";
const TOKEN_INSTANCIA: &str = "token-instancia-hr26f-nao-pode-sair";
const TELEFONE: &str = "5511987650000";
const JID: &str = "5511987650000@s.whatsapp.net";

/// Pasta temporária com configuração e banco prontos (org, agente e integração de WhatsApp).
struct Preparo {
    pasta: PathBuf,
    org: OrgId,
    porta: u16,
}

fn porta_livre() -> u16 {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("porta livre");
    listener.local_addr().expect("endereço").port()
}

fn preparar(rotulo: &str) -> Preparo {
    let pasta =
        std::env::temp_dir().join(format!("poph-hr26f-log-{rotulo}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&pasta);
    fs::create_dir_all(pasta.join("dados")).expect("pasta de dados");

    let mut config = HarnessConfig::default();
    config.data.directory = pasta.join("dados");
    let store = Store::open(&config).expect("cria o banco");
    let org = store
        .orgs()
        .criar(NewOrg {
            name: "Loja HR-26f".to_owned(),
            slug: format!("loja-hr26f-{rotulo}"),
            config_json: String::new(),
        })
        .expect("cria a org")
        .id;
    let agente = store
        .agents()
        .criar(
            org,
            NewAgent {
                name: "Atendente HR-26f".to_owned(),
                slug: format!("atendente-hr26f-{rotulo}"),
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

    let porta = porta_livre();
    let texto = format!(
        "[dados]\ndiretorio = \"{dados}\"\n\n\
         [servidor]\nporta = {porta}\n\n\
         [log]\nnivel = \"info\"\n\n\
         [evolution]\nurl_base = \"http://127.0.0.1:18081\"\n\n\
         [[evolution.instancias]]\norg_id = \"{org}\"\ninstancia = \"loja\"\n\
         integracao_id = \"{integracao}\"\nsegredo_env = \"{VAR_SEGREDO}\"\n",
        dados = pasta.join("dados").display(),
    );
    fs::write(pasta.join("poph.toml"), texto).expect("grava a configuração");
    Preparo { pasta, org, porta }
}

/// O processo `poph serve`, com o erro padrão lido em segundo plano.
struct Servidor {
    filho: Child,
    erro: Arc<Mutex<Vec<u8>>>,
    leitor: Option<thread::JoinHandle<()>>,
}

fn subir(preparo: &Preparo) -> Servidor {
    let mut filho = Command::new(env!("CARGO_BIN_EXE_poph"))
        .args(["serve", "--config", "poph.toml"])
        .current_dir(&preparo.pasta)
        // Ambiente limpo: sem RUST_LOG, para provar que o nível vem da configuração.
        .env_clear()
        .env(VAR_SEGREDO, SEGREDO)
        .env("EVOLUTION_API_KEY", CHAVE_EVO)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("sobe o poph serve");

    let erro = Arc::new(Mutex::new(Vec::new()));
    let mut saida = filho.stderr.take().expect("erro padrão do processo");
    let guardado = Arc::clone(&erro);
    let leitor = thread::spawn(move || {
        let mut buffer = [0u8; 4096];
        loop {
            match saida.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(n) => guardado
                    .lock()
                    .expect("sem veneno")
                    .extend_from_slice(&buffer[..n]),
            }
        }
    });
    Servidor {
        filho,
        erro,
        leitor: Some(leitor),
    }
}

impl Servidor {
    fn log(&self) -> String {
        String::from_utf8_lossy(&self.erro.lock().expect("sem veneno")).into_owned()
    }

    /// Encerra o processo e espera o erro padrão terminar de ser lido.
    fn encerrar(mut self) -> String {
        let _ = self.filho.kill();
        let _ = self.filho.wait();
        if let Some(leitor) = self.leitor.take() {
            let _ = leitor.join();
        }
        self.log()
    }
}

impl Drop for Servidor {
    fn drop(&mut self) {
        let _ = self.filho.kill();
        let _ = self.filho.wait();
    }
}

/// Requisição HTTP mínima (sem biblioteca), para falar com o processo de verdade.
fn http(
    porta: u16,
    metodo: &str,
    caminho: &str,
    cabecalhos: &[(&str, &str)],
    corpo: &str,
) -> (u16, String) {
    let mut conexao =
        TcpStream::connect((Ipv4Addr::LOCALHOST, porta)).expect("conecta ao servidor");
    conexao
        .set_read_timeout(Some(Duration::from_secs(10)))
        .expect("tempo limite de leitura");
    let mut pedido =
        format!("{metodo} {caminho} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n");
    for (nome, valor) in cabecalhos {
        pedido.push_str(&format!("{nome}: {valor}\r\n"));
    }
    pedido.push_str(&format!(
        "Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{corpo}",
        corpo.len()
    ));
    conexao
        .write_all(pedido.as_bytes())
        .expect("envia o pedido");
    let mut resposta = String::new();
    conexao
        .read_to_string(&mut resposta)
        .expect("lê a resposta");
    let status = resposta
        .split_whitespace()
        .nth(1)
        .and_then(|texto| texto.parse::<u16>().ok())
        .expect("status da resposta");
    let corpo = resposta
        .split_once("\r\n\r\n")
        .map(|(_, c)| c.to_owned())
        .unwrap_or_default();
    (status, format!("{resposta}\n{corpo}"))
}

fn esperar_subir(porta: u16) {
    for _ in 0..200 {
        if let Ok(conexao) = TcpStream::connect((Ipv4Addr::LOCALHOST, porta)) {
            drop(conexao);
            let (status, _) = http(porta, "GET", "/healthz", &[], "");
            if status == 200 {
                return;
            }
        }
        thread::sleep(Duration::from_millis(50));
    }
    panic!("o servidor não subiu na porta {porta}");
}

fn evento_de_texto() -> String {
    json!({
        "event": "messages.upsert",
        "instance": "loja",
        "apikey": TOKEN_INSTANCIA,
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
fn servidor_registra_a_operacao_do_webhook_sem_dado_sensivel() {
    let preparo = preparar("ok");
    let servidor = subir(&preparo);
    esperar_subir(preparo.porta);

    let caminho = format!("/webhook/evolution/{}/loja", preparo.org);
    let corpo = evento_de_texto();
    let (certo, resposta_certa) = http(
        preparo.porta,
        "POST",
        &caminho,
        &[("X-Evolution-Webhook-Secret", SEGREDO)],
        &corpo,
    );
    assert_eq!(certo, 200, "entrega com segredo certo: {resposta_certa}");

    let (errado, resposta_errada) = http(
        preparo.porta,
        "POST",
        &caminho,
        &[("X-Evolution-Webhook-Secret", SEGREDO_ERRADO)],
        &corpo,
    );
    assert_eq!(errado, 401, "entrega com segredo errado: {resposta_errada}");

    let registro = servidor.encerrar();
    let _ = fs::remove_dir_all(&preparo.pasta);

    // (i) a captura funciona e a operação ficou registrada.
    assert!(
        registro.contains("servidor no ar"),
        "a linha de subida não apareceu: o log não está sendo capturado\n{registro}"
    );
    assert!(
        registro.contains("mensagem do WhatsApp registrada"),
        "a entrada do webhook não deixou rastro\n{registro}"
    );
    let linha = registro
        .lines()
        .find(|linha| linha.contains("mensagem do WhatsApp registrada"))
        .unwrap_or_default();
    assert!(
        linha.contains("nova"),
        "o rastro não diz que a mensagem entrou como nova: {linha}"
    );

    // (ii) nada sensível no log, nem nas respostas.
    let proibidos = [
        TELEFONE,
        JID,
        "@s.whatsapp.net",
        CHAVE_EVO,
        TOKEN_INSTANCIA,
        SEGREDO,
        SEGREDO_ERRADO,
        "oi, quero um orçamento",
    ];
    for proibido in proibidos {
        assert!(
            !registro.contains(proibido),
            "o log vazou `{proibido}`:\n{registro}"
        );
        assert!(
            !resposta_certa.contains(proibido) && !resposta_errada.contains(proibido),
            "uma resposta devolveu `{proibido}`"
        );
    }
}

fn evento_de_estado() -> String {
    json!({
        "event": "connection.update",
        "instance": "loja",
        "apikey": TOKEN_INSTANCIA,
        "destination": "http://harness.teste/webhook/evolution/x/loja",
        "data": {"state": "open"}
    })
    .to_string()
}

#[test]
fn servidor_registra_o_estado_de_conexao_sem_dado_sensivel() {
    let preparo = preparar("estado");
    let servidor = subir(&preparo);
    esperar_subir(preparo.porta);

    let caminho = format!("/webhook/evolution/{}/loja", preparo.org);
    let (status, resposta) = http(
        preparo.porta,
        "POST",
        &caminho,
        &[("X-Evolution-Webhook-Secret", SEGREDO)],
        &evento_de_estado(),
    );
    assert_eq!(
        status, 200,
        "entrega de estado com segredo certo: {resposta}"
    );

    let registro = servidor.encerrar();
    let _ = fs::remove_dir_all(&preparo.pasta);

    // (i) a captura funciona e a atualização de estado ficou registrada com o estado novo.
    assert!(
        registro.contains("servidor no ar"),
        "a linha de subida não apareceu: o log não está sendo capturado\n{registro}"
    );
    let linha = registro
        .lines()
        .find(|linha| linha.contains("estado de conexão do WhatsApp atualizado"))
        .unwrap_or_default();
    assert!(
        !linha.is_empty(),
        "a atualização de estado não deixou rastro\n{registro}"
    );
    assert!(
        linha.contains("connected"),
        "a linha não diz o estado novo: {linha}"
    );
    assert!(
        linha.contains("loja"),
        "a linha não diz a instância: {linha}"
    );

    // (ii) nada sensível no log.
    for proibido in [SEGREDO, CHAVE_EVO, TOKEN_INSTANCIA] {
        assert!(
            !registro.contains(proibido),
            "o log vazou `{proibido}`:\n{registro}"
        );
    }
}
