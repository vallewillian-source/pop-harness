//! HR-46f: sem o modelo de embedding instalado, o turno continua oferecendo as ferramentas que só
//! usam o banco — `enviar_arquivo` inclusive — e `consultar_base`/`buscar_produto` ficam de fora.
//!
//! O defeito corrigido: o registro de ferramentas era montado **dentro** do bloco da base de
//! conhecimento. Numa instalação sem `poph model install` o pedido ao provedor saía com
//! `tools: []`, o modelo não recebia `enviar_arquivo` e o turno terminava `done` sem enviar nada.
//!
//! Aqui sobe o binário `poph serve` de verdade, com um provedor OpenAI-compatível falso e **sem**
//! a pasta de modelos. O que se prova:
//! (a) o pedido ao provedor leva `tools` com `enviar_arquivo` e sem `consultar_base`/`buscar_produto`
//!     (as três estavam habilitadas no agente, então a ausência é do registro, não do filtro);
//! (b) o modelo chama `enviar_arquivo` e o histórico do chat de teste devolve `files_out`;
//! (c) o download da biblioteca devolve os mesmos bytes do arquivo semeado (sha256 igual);
//! (d) a resposta do histórico não leva caminho de blob (`blob_ref`/`blobs/`).

use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use argon2::Argon2;
use argon2::password_hash::{PasswordHasher, SaltString};
use poph_store::{NewAgent, NewOrg, Store};
use poph_types::config::GlobalTokenConfig;
use poph_types::{HarnessConfig, StructuredInstructions, ToolConfig};
use rand_core::OsRng;
use serde_json::{Value, json};

const TOKEN_GLOBAL: &str = "pophg_teste-hr46f";
const CHAVE_FALSA: &str = "chave-de-teste-hr46f";

fn hash_argon2(valor: &str) -> String {
    let sal = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(valor.as_bytes(), &sal)
        .expect("hash argon2")
        .to_string()
}

fn fixture(nome: &str) -> Vec<u8> {
    let caminho = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../poph-files/tests/fixtures")
        .join(nome);
    fs::read(&caminho).unwrap_or_else(|erro| panic!("fixture {nome}: {erro}"))
}

/// Pasta descartável por teste.
fn pasta_de_teste(rotulo: &str) -> PathBuf {
    let carimbo = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let pasta = std::env::temp_dir().join(format!(
        "poph-sem-base-{rotulo}-{}-{carimbo}",
        std::process::id()
    ));
    fs::create_dir_all(pasta.join("dados")).expect("pasta de dados");
    pasta
}

/// Servidor encerrado no fim do teste, mesmo quando uma asserção falha.
struct Servidor(Child);

impl Drop for Servidor {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Lê um pedido HTTP inteiro (cabeçalhos + corpo pelo `content-length`).
fn ler_pedido(fluxo: &mut TcpStream) -> Option<String> {
    let mut recebido = Vec::new();
    let mut pedaco = [0u8; 4096];
    let fim_dos_cabecalhos = loop {
        let lido = fluxo.read(&mut pedaco).ok()?;
        if lido == 0 {
            return None;
        }
        recebido.extend_from_slice(&pedaco[..lido]);
        if let Some(posicao) = recebido.windows(4).position(|janela| janela == b"\r\n\r\n") {
            break posicao + 4;
        }
    };
    let cabecalhos = String::from_utf8_lossy(&recebido[..fim_dos_cabecalhos]).to_lowercase();
    let tamanho: usize = cabecalhos
        .lines()
        .find_map(|linha| linha.strip_prefix("content-length:"))
        .and_then(|valor| valor.trim().parse().ok())
        .unwrap_or(0);
    while recebido.len() < fim_dos_cabecalhos + tamanho {
        let lido = fluxo.read(&mut pedaco).ok()?;
        if lido == 0 {
            break;
        }
        recebido.extend_from_slice(&pedaco[..lido]);
    }
    Some(String::from_utf8_lossy(&recebido[fim_dos_cabecalhos..]).into_owned())
}

/// Escreve um evento SSE em *chunked encoding*.
fn evento(fluxo: &mut TcpStream, objeto: Value) {
    let dados = format!("data: {objeto}\n\n");
    let _ = write!(fluxo, "{:x}\r\n{dados}\r\n", dados.len());
    let _ = fluxo.flush();
}

/// Provedor OpenAI-compatível falso: 1º pedido pede `enviar_arquivo`; do 2º em diante responde
/// texto. Guarda o corpo de cada pedido para o teste conferir o que o turno enviou.
fn provedor_falso(arquivo_id: String) -> (u16, Arc<Mutex<Vec<Value>>>) {
    let ouvinte = TcpListener::bind("127.0.0.1:0").expect("porta do provedor falso");
    let porta = ouvinte.local_addr().expect("endereço do provedor").port();
    let pedidos = Arc::new(Mutex::new(Vec::new()));
    let guardados = Arc::clone(&pedidos);
    thread::spawn(move || {
        for conexao in ouvinte.incoming() {
            let Ok(mut fluxo) = conexao else { continue };
            let Some(corpo) = ler_pedido(&mut fluxo) else {
                continue;
            };
            let pedido: Value = serde_json::from_str(&corpo).unwrap_or(Value::Null);
            let numero = {
                let mut lista = guardados.lock().expect("lista de pedidos");
                lista.push(pedido);
                lista.len()
            };
            let _ = write!(
                fluxo,
                "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\n\
                 cache-control: no-cache\r\ntransfer-encoding: chunked\r\nconnection: close\r\n\r\n"
            );
            if numero == 1 {
                evento(
                    &mut fluxo,
                    json!({"choices": [{"delta": {"tool_calls": [{
                        "index": 0,
                        "id": "call-tabela",
                        "type": "function",
                        "function": {
                            "name": "enviar_arquivo",
                            "arguments": json!({"id": arquivo_id}).to_string(),
                        },
                    }]}}]}),
                );
                evento(
                    &mut fluxo,
                    json!({"choices": [{"delta": {}, "finish_reason": "tool_calls"}]}),
                );
            } else {
                for pedaco in ["Enviei a ", "tabela de preços", " agora."] {
                    evento(
                        &mut fluxo,
                        json!({"choices": [{"delta": {"content": pedaco}}]}),
                    );
                }
                evento(
                    &mut fluxo,
                    json!({"choices": [{"delta": {}, "finish_reason": "stop"}],
                           "usage": {"prompt_tokens": 120, "completion_tokens": 16}}),
                );
            }
            let fim = "data: [DONE]\n\n";
            let _ = write!(fluxo, "{:x}\r\n{fim}\r\n0\r\n\r\n", fim.len());
            let _ = fluxo.flush();
        }
    });
    (porta, pedidos)
}

/// Corpo multipart com só o campo `text`.
fn multipart(texto: &str) -> (String, Vec<u8>) {
    let limite = "hr46f-boundary";
    let corpo = format!(
        "--{limite}\r\nContent-Disposition: form-data; name=\"text\"\r\n\r\n{texto}\r\n\
         --{limite}--\r\n"
    )
    .into_bytes();
    (format!("multipart/form-data; boundary={limite}"), corpo)
}

struct Cenario {
    pasta: PathBuf,
    base: String,
    org: String,
    agente: String,
    arquivo_id: String,
    bytes: Vec<u8>,
    pedidos: Arc<Mutex<Vec<Value>>>,
    cliente: reqwest::Client,
    _servidor: Servidor,
}

impl Cenario {
    /// Semeia banco (org, agente com as três ferramentas habilitadas, arquivo na biblioteca),
    /// sobe o provedor falso e o `poph serve` **sem** a pasta de modelos.
    async fn novo(rotulo: &str) -> Self {
        let pasta = pasta_de_teste(rotulo);
        let dados = pasta.join("dados");
        let bytes = fixture("tabela.pdf");

        let (org, agente, arquivo_id) = {
            let store = Store::open_at(&dados.join("harness.db"), &dados.join("rag.db"))
                .expect("abre o banco de teste");
            let org = store
                .orgs()
                .create(NewOrg {
                    name: "Org HR-46f".to_owned(),
                    slug: "org-hr46f".to_owned(),
                    config_json: "{}".to_owned(),
                })
                .expect("org")
                .id;
            let agente = store
                .agents()
                .create(
                    org,
                    NewAgent {
                        name: "Bia".to_owned(),
                        slug: "bia".to_owned(),
                        // Sem janela de agrupamento: o teste não espera 2 s por turno.
                        grouping_window_ms: 1,
                        ..Default::default()
                    },
                )
                .expect("agente")
                .id;
            // As três ferramentas habilitadas: o que sumir do pedido é decisão do registro, não do
            // filtro por agente (D-04).
            let instrucoes = StructuredInstructions {
                tools: vec![
                    ToolConfig::new("enviar_arquivo"),
                    ToolConfig::new("consultar_base"),
                    ToolConfig::new("buscar_produto"),
                ],
                ..StructuredInstructions::default()
            };
            store
                .instructions()
                .save(
                    org,
                    agente,
                    0,
                    &instrucoes.to_json(),
                    &poph_agent::compor_instrucoes(&instrucoes),
                    true,
                )
                .expect("instruções");
            let gravado =
                poph_files::blob::gravar(&dados.join("blobs"), org, &bytes).expect("blob");
            let arquivo = store
                .library()
                .create(
                    org,
                    poph_store::NewLibraryFile {
                        agent_id: agente,
                        name: "Tabela de precos".to_owned(),
                        description: "tabela de preços".to_owned(),
                        mime: "application/pdf".to_owned(),
                        size_bytes: bytes.len() as u64,
                        sha256: gravado.sha256.clone(),
                        blob_path: format!("{org}/{}", gravado.sha256),
                    },
                )
                .expect("arquivo na biblioteca");
            (org, agente, arquivo.id.to_string())
        };

        let (porta_llm, pedidos) = provedor_falso(arquivo_id.clone());
        let porta = {
            let ouvinte = TcpListener::bind("127.0.0.1:0").expect("porta do servidor");
            ouvinte.local_addr().expect("endereço do servidor").port()
        };

        let mut config = HarnessConfig::default();
        config.data.directory = dados.clone();
        config.server.port = porta;
        config.llm.url_base = format!("http://127.0.0.1:{porta_llm}/v1");
        config.llm.retries = 0;
        config.llm.timeout_seconds = 10;
        config.turn.dispatch_interval_ms = 100;
        config.admin.tokens_globais = vec![GlobalTokenConfig {
            label: "teste-hr46f".to_owned(),
            hash: hash_argon2(TOKEN_GLOBAL),
        }];
        fs::write(
            pasta.join("poph.toml"),
            toml::to_string(&config).expect("configuração em TOML"),
        )
        .expect("grava a configuração");

        let servidor = Servidor(
            Command::new(env!("CARGO_BIN_EXE_poph"))
                .args(["serve", "--config", "poph.toml"])
                .current_dir(&pasta)
                .env_clear()
                .env("ANTIGRAVITY_API_KEY", CHAVE_FALSA)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .expect("sobe o poph serve"),
        );

        let cenario = Self {
            pasta,
            base: format!("http://127.0.0.1:{porta}"),
            org: org.to_string(),
            agente: agente.to_string(),
            arquivo_id,
            bytes,
            pedidos,
            cliente: reqwest::Client::builder()
                .timeout(Duration::from_secs(20))
                .build()
                .expect("cliente HTTP"),
            _servidor: servidor,
        };
        cenario.esperar_subir().await;
        cenario
    }

    async fn esperar_subir(&self) {
        let inicio = Instant::now();
        loop {
            if let Ok(resposta) = self
                .cliente
                .get(format!("{}/healthz", self.base))
                .send()
                .await
                && resposta.status().is_success()
            {
                return;
            }
            assert!(
                inicio.elapsed() < Duration::from_secs(20),
                "o servidor não subiu: {}",
                self.pasta.display()
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    fn raiz(&self) -> String {
        format!(
            "{}/admin/v1/orgs/{}/agents/{}/test-chat",
            self.base, self.org, self.agente
        )
    }

    async fn criar_conversa(&self) -> String {
        let resposta = self
            .cliente
            .post(self.raiz())
            .bearer_auth(TOKEN_GLOBAL)
            .send()
            .await
            .expect("criar conversa");
        assert_eq!(resposta.status(), 201, "criar conversa");
        let corpo: Value = resposta.json().await.expect("json da conversa");
        corpo["id"].as_str().expect("id da conversa").to_owned()
    }

    async fn enviar(&self, conversa: &str) -> String {
        let (tipo, corpo) = multipart("manda a tabela de preços por favor");
        let resposta = self
            .cliente
            .post(format!("{}/{conversa}/messages", self.raiz()))
            .bearer_auth(TOKEN_GLOBAL)
            .header("content-type", tipo)
            .body(corpo)
            .send()
            .await
            .expect("envia a mensagem");
        assert_eq!(resposta.status(), 200, "envia a mensagem");
        resposta.text().await.expect("corpo do SSE")
    }

    async fn historico(&self, conversa: &str) -> (Value, String) {
        let resposta = self
            .cliente
            .get(format!("{}/{conversa}", self.raiz()))
            .bearer_auth(TOKEN_GLOBAL)
            .send()
            .await
            .expect("histórico");
        assert_eq!(resposta.status(), 200, "histórico");
        let texto = resposta.text().await.expect("corpo do histórico");
        let json: Value = serde_json::from_str(&texto).expect("histórico em JSON");
        (json, texto)
    }

    async fn baixar(&self, arquivo: &str) -> reqwest::Response {
        self.cliente
            .get(format!(
                "{}/admin/v1/orgs/{}/agents/{}/files/{arquivo}/content",
                self.base, self.org, self.agente
            ))
            .bearer_auth(TOKEN_GLOBAL)
            .send()
            .await
            .expect("download do arquivo")
    }
}

#[tokio::test]
async fn sem_modelo_de_embedding_o_turno_oferece_enviar_arquivo_e_o_arquivo_sai() {
    let cenario = Cenario::novo("enviar-arquivo").await;
    let conversa = cenario.criar_conversa().await;
    let sse = cenario.enviar(&conversa).await;

    // Os eventos do turno chegam em ordem, com a ferramenta e o fim.
    let nomes: Vec<&str> = sse
        .lines()
        .filter_map(|linha| linha.strip_prefix("event: "))
        .collect();
    assert_eq!(
        nomes,
        vec![
            "mensagem",
            "inicio",
            "ferramenta",
            "texto",
            "texto",
            "texto",
            "fim"
        ],
        "eventos: {sse}"
    );
    assert!(sse.contains("\"state\":\"done\""), "fim do turno: {sse}");

    // (a) O pedido ao provedor leva `enviar_arquivo` e NÃO leva as ferramentas da base, mesmo
    //     estando as três habilitadas no agente.
    let pedidos = cenario.pedidos.lock().expect("pedidos").clone();
    assert_eq!(pedidos.len(), 2, "o turno faz duas chamadas ao modelo");
    let ferramentas: Vec<String> = pedidos[0]["tools"]
        .as_array()
        .unwrap_or(&Vec::new())
        .iter()
        .filter_map(|ferramenta| ferramenta["function"]["name"].as_str().map(str::to_owned))
        .collect();
    assert_eq!(
        ferramentas,
        vec!["enviar_arquivo".to_owned()],
        "ferramentas oferecidas sem base: {ferramentas:?}"
    );
    // O segundo pedido leva o resultado da ferramenta para o modelo.
    assert!(
        pedidos[1]["messages"].to_string().contains("\"tool\""),
        "o resultado da ferramenta não voltou ao modelo"
    );

    // (b) O histórico traz `files_out` com o arquivo enviado.
    let (historico, texto) = cenario.historico(&conversa).await;
    let turno = &historico["turns"][0];
    assert_eq!(turno["state"], "done");
    let arquivos = turno["files_out"].as_array().expect("files_out");
    assert_eq!(arquivos.len(), 1, "files_out: {arquivos:?}");
    assert_eq!(arquivos[0]["arquivo_id"], cenario.arquivo_id.as_str());
    assert_eq!(arquivos[0]["mime"], "application/pdf");

    // (d) Nenhum caminho de blob na resposta.
    for proibido in ["blob_ref", "blobs/", "conteudo_json"] {
        assert!(
            !texto.contains(proibido),
            "{proibido} apareceu no histórico"
        );
    }

    // (c) O download devolve os mesmos bytes do arquivo semeado.
    let resposta = cenario.baixar(&cenario.arquivo_id).await;
    assert_eq!(resposta.status(), 200, "download");
    let baixado = resposta.bytes().await.expect("bytes do download");
    assert_eq!(
        baixado.as_ref(),
        cenario.bytes.as_slice(),
        "bytes idênticos"
    );
}

#[tokio::test]
async fn sem_modelo_de_embedding_a_ferramenta_nao_habilitada_continua_fora() {
    // Controle: o filtro por agente (D-04) continua valendo depois da HR-46f. Sem
    // `enviar_arquivo` nas instruções, a ferramenta existe no registro, mas não vai ao modelo.
    let pasta = pasta_de_teste("filtro");
    let dados = pasta.join("dados");
    let (org, agente) = {
        let store = Store::open_at(&dados.join("harness.db"), &dados.join("rag.db"))
            .expect("abre o banco de teste");
        let org = store
            .orgs()
            .create(NewOrg {
                name: "Org HR-46f filtro".to_owned(),
                slug: "org-hr46f-filtro".to_owned(),
                config_json: "{}".to_owned(),
            })
            .expect("org")
            .id;
        let agente = store
            .agents()
            .create(
                org,
                NewAgent {
                    name: "Bia".to_owned(),
                    slug: "bia".to_owned(),
                    grouping_window_ms: 1,
                    ..Default::default()
                },
            )
            .expect("agente")
            .id;
        let instrucoes = StructuredInstructions {
            tools: vec![ToolConfig {
                name: "enviar_arquivo".to_owned(),
                enabled: false,
                when_to_use: String::new(),
            }],
            ..StructuredInstructions::default()
        };
        store
            .instructions()
            .save(
                org,
                agente,
                0,
                &instrucoes.to_json(),
                &poph_agent::compor_instrucoes(&instrucoes),
                true,
            )
            .expect("instruções");
        (org, agente)
    };

    let (porta_llm, pedidos) = provedor_falso("nao-usado".to_owned());
    let porta = {
        let ouvinte = TcpListener::bind("127.0.0.1:0").expect("porta do servidor");
        ouvinte.local_addr().expect("endereço").port()
    };
    let mut config = HarnessConfig::default();
    config.data.directory = dados;
    config.server.port = porta;
    config.llm.url_base = format!("http://127.0.0.1:{porta_llm}/v1");
    config.llm.retries = 0;
    config.turn.dispatch_interval_ms = 100;
    config.admin.tokens_globais = vec![GlobalTokenConfig {
        label: "teste-hr46f".to_owned(),
        hash: hash_argon2(TOKEN_GLOBAL),
    }];
    fs::write(
        pasta.join("poph.toml"),
        toml::to_string(&config).expect("TOML"),
    )
    .expect("configuração");
    let _servidor = Servidor(
        Command::new(env!("CARGO_BIN_EXE_poph"))
            .args(["serve", "--config", "poph.toml"])
            .current_dir(&pasta)
            .env_clear()
            .env("ANTIGRAVITY_API_KEY", CHAVE_FALSA)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("sobe o poph serve"),
    );

    let base = format!("http://127.0.0.1:{porta}");
    let cliente = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .expect("cliente HTTP");
    let inicio = Instant::now();
    loop {
        if let Ok(resposta) = cliente.get(format!("{base}/healthz")).send().await
            && resposta.status().is_success()
        {
            break;
        }
        assert!(
            inicio.elapsed() < Duration::from_secs(20),
            "servidor não subiu"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    let raiz = format!("{base}/admin/v1/orgs/{org}/agents/{agente}/test-chat");
    let conversa: Value = cliente
        .post(&raiz)
        .bearer_auth(TOKEN_GLOBAL)
        .send()
        .await
        .expect("criar conversa")
        .json()
        .await
        .expect("json da conversa");
    let conversa = conversa["id"].as_str().expect("id").to_owned();
    let (tipo, corpo) = multipart("olá");
    let resposta = cliente
        .post(format!("{raiz}/{conversa}/messages"))
        .bearer_auth(TOKEN_GLOBAL)
        .header("content-type", tipo)
        .body(corpo)
        .send()
        .await
        .expect("envia a mensagem");
    assert_eq!(resposta.status(), 200);
    let _ = resposta.text().await.expect("SSE");

    let pedidos = pedidos.lock().expect("pedidos").clone();
    let ferramentas: Vec<String> = pedidos[0]["tools"]
        .as_array()
        .unwrap_or(&Vec::new())
        .iter()
        .filter_map(|ferramenta| ferramenta["function"]["name"].as_str().map(str::to_owned))
        .collect();
    assert!(
        ferramentas.is_empty(),
        "ferramenta desabilitada foi oferecida: {ferramentas:?}"
    );
}
