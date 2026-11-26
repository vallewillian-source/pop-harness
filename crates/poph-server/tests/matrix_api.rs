//! Testes da API administrativa do módulo Matrix (MX-22).
//!
//! O que se prova aqui, com banco de verdade, servidor HTTP de verdade e um **Synapse falso** que
//! responde como o Synapse:
//! - o estado da org (`GET …/matrix`) traz ligado, corte do backfill, "conectado", atraso do cursor,
//!   falhas pendentes e a última falha, com `ETag`;
//! - `PUT …/matrix` exige `If-Match` (`428` sem, `412` com valor velho), liga gravando o corte do
//!   backfill, desliga preservando o corte e repete sem quebrar;
//! - token de org só opera a própria org (`403`), sem token é `401` e org inexistente é `404`;
//! - `PUT …/matrix/spaces` cria o espaço uma vez só (repetir não cria segundo) e liga um espaço que
//!   já existe no Element, recusando com `409` e instrução quando o bot não tem permissão;
//! - `DELETE …/matrix/spaces/{department}` desliga o espaço sem apagar nada no Synapse;
//! - módulo desligado nesta instância: `connected` é `null` e as rotas de espaço respondem `409`.
//!
//! Os testes de espaço montam só o serviço de espaços sobre a API (sem subir os laços do módulo):
//! assim a asserção é determinística, porque nenhum laço de fundo mexe em espaço no meio do teste.
//! O teste de estado usa a ligação real (`matrix_inicio::iniciar_com`), que é a mesma do `poph serve`.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use argon2::Argon2;
use argon2::password_hash::{PasswordHasher, SaltString};
use axum::Router;
use axum::extract::State;
use axum::http::header::AUTHORIZATION;
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use poph_matrix::config::ModuloMatrix;
use poph_matrix::config::SegredosMatrix;
use poph_matrix::espacos::Espacos;
use poph_server::Estado;
use poph_server::matrix_inicio::{self, AjustesDeInicio, EspacosDoModulo, PortaEspacosStore};
use poph_store::{
    Direcao, NovaConversa, NovaMensagem, NovaOrg, NovoAgente, NovoDepartamento, Origem, Papel,
    Store,
};
use poph_types::config::GlobalTokenConfig;
use poph_types::{AgentId, DepartmentId, HarnessConfig, MatrixConfig, OrgId, Secret};
use rand_core::OsRng;
use reqwest::{Client, Method as MetodoHttp};
use serde_json::{Value, json};
use tokio::sync::oneshot;

/// Token que o Harness apresenta ao Synapse.
const AS_TOKEN: &str = "CANARIO-AS-mx22-nao-pode-vazar";
/// Token que o Synapse apresenta ao Harness.
const HS_TOKEN: &str = "CANARIO-HS-mx22-nao-pode-vazar";
/// Domínio dos ids no teste.
const SERVIDOR: &str = "matrix-dev.local";
/// Prefixo dos usuários virtuais.
const PREFIXO: &str = "ph";
/// Token global (cria e lê qualquer org, inclusive org que não existe).
const TOKEN_GLOBAL: &str = "pophg_teste-global-mx22";

// ───────────────────────────── Synapse falso ─────────────────────────────

#[derive(Default, Clone)]
struct SalaFalsa {
    nome: Option<String>,
    tipo: Option<String>,
    membros: Vec<String>,
    convidados: Vec<String>,
    estado: BTreeMap<(String, String), Value>,
    mensagens: Vec<(String, String, Value)>,
}

#[derive(Default)]
struct EstadoFalso {
    usuarios: BTreeMap<String, Option<String>>,
    salas: BTreeMap<String, SalaFalsa>,
    aliases: BTreeMap<String, String>,
    proxima_sala: u32,
    versoes: u32,
}

fn responder(status: u16, corpo: Value) -> Response {
    (
        StatusCode::from_u16(status).expect("status válido"),
        [("content-type", "application/json")],
        corpo.to_string(),
    )
        .into_response()
}

fn decodificar(segmento: &str) -> String {
    let bytes = segmento.as_bytes();
    let mut saida = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
            if let Ok(byte) = u8::from_str_radix(hex, 16) {
                saida.push(byte);
                i += 3;
                continue;
            }
        }
        saida.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&saida).into_owned()
}

/// Percent-encoding mínimo de um id de usuário no parâmetro `user_id`.
fn urlencoding(valor: &str) -> String {
    valor
        .chars()
        .map(|caractere| match caractere {
            '@' => "%40".to_owned(),
            ':' => "%3A".to_owned(),
            outro => outro.to_string(),
        })
        .collect()
}

fn consulta(bruta: &str, nome: &str) -> Option<String> {
    bruta.split('&').find_map(|par| {
        let (chave, valor) = par.split_once('=')?;
        (chave == nome).then(|| decodificar(valor))
    })
}

async fn atender(
    State(estado): State<Arc<Mutex<EstadoFalso>>>,
    metodo: Method,
    uri: Uri,
    cabecalhos: HeaderMap,
    corpo: String,
) -> Response {
    let corpo: Value = serde_json::from_str(&corpo).unwrap_or(Value::Null);
    let mut guarda = estado.lock().expect("trava do Synapse falso");

    let autorizacao = cabecalhos
        .get(AUTHORIZATION)
        .and_then(|valor| valor.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    if autorizacao != format!("Bearer {AS_TOKEN}") {
        return responder(
            401,
            json!({ "errcode": "M_UNKNOWN_TOKEN", "error": "Token inválido" }),
        );
    }
    if uri.path() == "/_matrix/client/versions" {
        guarda.versoes += 1;
        return responder(200, json!({ "versions": ["v1.11"] }));
    }

    let caminho = uri.path().trim_start_matches("/_matrix/client/v3");
    let partes: Vec<String> = caminho
        .split('/')
        .filter(|pedaco| !pedaco.is_empty())
        .map(decodificar)
        .collect();
    let consulta_bruta = uri.query().unwrap_or_default().to_owned();
    let quem = consulta(&consulta_bruta, "user_id").unwrap_or_default();

    match (metodo.as_str(), partes.as_slice()) {
        ("POST", [registro]) if registro == "register" => {
            let localpart = corpo["username"].as_str().unwrap_or_default();
            let id = format!("@{localpart}:{SERVIDOR}");
            if guarda.usuarios.contains_key(&id) {
                return responder(
                    400,
                    json!({ "errcode": "M_USER_IN_USE", "error": "User ID already taken." }),
                );
            }
            guarda.usuarios.insert(id.clone(), None);
            responder(200, json!({ "user_id": id }))
        }
        ("GET", [perfil, id]) if perfil == "profile" => match guarda.usuarios.get(id) {
            Some(nome) => {
                let mut resposta = json!({});
                if let Some(nome) = nome {
                    resposta["displayname"] = Value::String(nome.clone());
                }
                responder(200, resposta)
            }
            None => responder(
                404,
                json!({ "errcode": "M_NOT_FOUND", "error": "Usuário não existe" }),
            ),
        },
        ("PUT", [perfil, id, campo]) if perfil == "profile" && campo == "displayname" => {
            guarda
                .usuarios
                .insert(id.clone(), corpo["displayname"].as_str().map(str::to_owned));
            responder(200, json!({}))
        }
        ("POST", [criacao]) if criacao == "createRoom" => {
            let alias = corpo["room_alias_name"]
                .as_str()
                .map(|localpart| format!("#{localpart}:{SERVIDOR}"));
            if let Some(alias) = &alias
                && guarda.aliases.contains_key(alias)
            {
                return responder(
                    400,
                    json!({ "errcode": "M_ROOM_IN_USE", "error": "Room alias already taken." }),
                );
            }
            guarda.proxima_sala += 1;
            let sala = format!("!{}:{SERVIDOR}", guarda.proxima_sala);
            let mut membros = Vec::new();
            if !quem.is_empty() {
                membros.push(quem.clone());
            }
            guarda.salas.insert(
                sala.clone(),
                SalaFalsa {
                    nome: corpo["name"].as_str().map(str::to_owned),
                    tipo: corpo["creation_content"]["type"]
                        .as_str()
                        .map(str::to_owned),
                    membros,
                    ..SalaFalsa::default()
                },
            );
            if let Some(alias) = alias {
                guarda.aliases.insert(alias, sala.clone());
            }
            responder(200, json!({ "room_id": sala }))
        }
        ("GET", [diretorio, sala, alias]) if diretorio == "directory" && sala == "room" => {
            match guarda.aliases.get(alias) {
                Some(sala) => responder(200, json!({ "room_id": sala })),
                None => responder(
                    404,
                    json!({ "errcode": "M_NOT_FOUND", "error": "Alias não encontrado" }),
                ),
            }
        }
        ("POST", [salas, sala, convite]) if salas == "rooms" && convite == "invite" => {
            match guarda.salas.get_mut(sala) {
                Some(alvo) => {
                    let convidado = corpo["user_id"].as_str().unwrap_or_default().to_owned();
                    if !alvo.convidados.contains(&convidado) {
                        alvo.convidados.push(convidado);
                    }
                    responder(200, json!({}))
                }
                None => responder(
                    404,
                    json!({ "errcode": "M_NOT_FOUND", "error": "Sala não existe" }),
                ),
            }
        }
        ("POST", [entrar, alvo]) if entrar == "join" => {
            let sala = if alvo.starts_with('#') {
                guarda.aliases.get(alvo).cloned()
            } else {
                Some(alvo.clone())
            };
            match sala.and_then(|sala| guarda.salas.get_mut(&sala).map(|alvo| (sala, alvo))) {
                Some((sala, alvo)) => {
                    if !alvo.membros.contains(&quem) {
                        alvo.membros.push(quem.clone());
                    }
                    responder(200, json!({ "room_id": sala }))
                }
                None => responder(
                    404,
                    json!({ "errcode": "M_NOT_FOUND", "error": "Sala não existe" }),
                ),
            }
        }
        ("GET", [salas, sala, membros]) if salas == "rooms" && membros == "joined_members" => {
            match guarda.salas.get(sala) {
                Some(alvo) => {
                    let juntos: BTreeMap<String, Value> = alvo
                        .membros
                        .iter()
                        .map(|id| (id.clone(), json!({})))
                        .collect();
                    responder(200, json!({ "joined": juntos }))
                }
                None => responder(
                    404,
                    json!({ "errcode": "M_NOT_FOUND", "error": "Sala não existe" }),
                ),
            }
        }
        ("PUT", [salas, sala, estado, tipo, chave @ ..])
            if salas == "rooms" && estado == "state" =>
        {
            let chave = chave.join("/");
            match guarda.salas.get_mut(sala) {
                Some(alvo) => {
                    alvo.estado.insert((tipo.clone(), chave), corpo);
                    responder(200, json!({ "event_id": "$estado" }))
                }
                None => responder(
                    404,
                    json!({ "errcode": "M_NOT_FOUND", "error": "Sala não existe" }),
                ),
            }
        }
        ("GET", [salas, sala, estado, tipo, chave @ ..])
            if salas == "rooms" && estado == "state" =>
        {
            let chave = chave.join("/");
            match guarda.salas.get(sala) {
                Some(alvo) => match alvo.estado.get(&(tipo.clone(), chave)) {
                    Some(corpo) => responder(200, corpo.clone()),
                    None => responder(
                        404,
                        json!({ "errcode": "M_NOT_FOUND", "error": "Evento não existe" }),
                    ),
                },
                None => responder(
                    404,
                    json!({ "errcode": "M_NOT_FOUND", "error": "Sala não existe" }),
                ),
            }
        }
        ("PUT", [salas, sala, envio, tipo, txn]) if salas == "rooms" && envio == "send" => {
            match guarda.salas.get_mut(sala) {
                Some(alvo) => {
                    if !alvo.mensagens.iter().any(|(ja, _, _)| ja == txn) {
                        alvo.mensagens.push((txn.clone(), tipo.clone(), corpo));
                    }
                    responder(200, json!({ "event_id": format!("$msg_{txn}") }))
                }
                None => responder(
                    404,
                    json!({ "errcode": "M_NOT_FOUND", "error": "Sala não existe" }),
                ),
            }
        }
        _ => responder(
            404,
            json!({ "errcode": "M_UNRECOGNIZED", "error": "Rota não existe no Synapse falso" }),
        ),
    }
}

/// Synapse falso no ar, numa porta efêmera. O `Drop` desliga.
struct Synapse {
    endereco: String,
    estado: Arc<Mutex<EstadoFalso>>,
    desligar: Option<oneshot::Sender<()>>,
}

impl Synapse {
    async fn subir() -> Self {
        let estado = Arc::new(Mutex::new(EstadoFalso::default()));
        let app = Router::new()
            .fallback(atender)
            .with_state(Arc::clone(&estado));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("porta efêmera");
        let endereco = format!("http://{}", listener.local_addr().expect("endereço"));
        let (desligar, sinal) = oneshot::channel();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = sinal.await;
                })
                .await;
        });
        Self {
            endereco,
            estado,
            desligar: Some(desligar),
        }
    }

    fn salas(&self) -> Vec<(String, SalaFalsa)> {
        self.estado
            .lock()
            .expect("trava")
            .salas
            .iter()
            .map(|(id, sala)| (id.clone(), sala.clone()))
            .collect()
    }

    /// Cria uma sala no Synapse falso como outro usuário (um espaço que já existe no Element, com
    /// dono e sem o bot dentro).
    async fn criar_sala(&self, como: &str) -> String {
        let cliente = Client::new();
        let resposta = cliente
            .post(format!(
                "{}/_matrix/client/v3/createRoom?user_id={}",
                self.endereco,
                urlencoding(como)
            ))
            .bearer_auth(AS_TOKEN)
            .json(&json!({"name": "Espaço do operador"}))
            .send()
            .await
            .expect("createRoom no Synapse falso");
        let corpo: Value = resposta.json().await.expect("corpo do createRoom");
        corpo["room_id"].as_str().expect("room_id").to_owned()
    }

    fn quantidade_de_espacos(&self) -> usize {
        self.salas()
            .iter()
            .filter(|(_, sala)| sala.tipo.as_deref() == Some("m.space"))
            .count()
    }
}

impl Drop for Synapse {
    fn drop(&mut self) {
        if let Some(desligar) = self.desligar.take() {
            let _ = desligar.send(());
        }
    }
}

// ───────────────────────────── cenário ─────────────────────────────

fn hash_argon2(valor: &str) -> String {
    let sal = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(valor.as_bytes(), &sal)
        .expect("hash argon2")
        .to_string()
}

struct Cenario {
    pasta: PathBuf,
    store: Arc<Store>,
    config: HarnessConfig,
    relogio: Arc<AtomicI64>,
}

impl Cenario {
    fn novo(rotulo: &str, homeserver_url: String) -> Self {
        let carimbo = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duracao| duracao.as_nanos())
            .unwrap_or(0);
        let pasta = std::env::temp_dir().join(format!(
            "poph-matrix-api-{rotulo}-{}-{carimbo}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&pasta);
        fs::create_dir_all(pasta.join("dados")).expect("pasta de dados");

        let mut config = HarnessConfig::default();
        config.data.directory = pasta.join("dados");
        config.admin.tokens_globais = vec![GlobalTokenConfig {
            label: "teste".to_owned(),
            hash: hash_argon2(TOKEN_GLOBAL),
        }];
        config.matrix = Some(MatrixConfig {
            enabled: true,
            homeserver_url,
            server_name: SERVIDOR.to_owned(),
            prefix: PREFIXO.to_owned(),
            concurrency: 4,
            interval_ms: 50,
            ..MatrixConfig::default()
        });
        config.validar().expect("configuração válida");

        let relogio = Arc::new(AtomicI64::new(1_000_000));
        let proprio = Arc::clone(&relogio);
        let store = Arc::new(
            Store::abrir(&config)
                .expect("abre o banco")
                .com_relogio(Arc::new(move || proprio.fetch_add(10, Ordering::SeqCst))),
        );
        Self {
            pasta,
            store,
            config,
            relogio,
        }
    }

    fn estado(&self) -> Estado {
        Estado::novo(Arc::clone(&self.store), self.config.clone()).expect("estado do servidor")
    }

    fn modulo(&self) -> ModuloMatrix {
        ModuloMatrix {
            config: self.config.matrix.clone().expect("seção [matrix]"),
            segredos: SegredosMatrix {
                as_token: Secret::new(AS_TOKEN.to_owned()),
                hs_token: Secret::new(HS_TOKEN.to_owned()),
            },
        }
    }

    fn org(&self, slug: &str) -> OrgId {
        self.store
            .orgaos()
            .criar(NovaOrg {
                nome: format!("Org {slug}"),
                slug: slug.to_owned(),
                config_json: String::new(),
            })
            .expect("cria a org")
            .id
    }

    /// Token de org no banco, no formato que a API autentica: `poph_<id>_<segredo>`.
    fn token(&self, org: OrgId, rotulo: &str) -> String {
        let segredo = format!("segredo-{rotulo}");
        let criado = self
            .store
            .tokens()
            .criar(Some(org), rotulo, &hash_argon2(&segredo))
            .expect("cria o token");
        format!("poph_{}_{segredo}", criado.id)
    }

    fn agente(&self, org: OrgId) -> AgentId {
        self.store
            .agentes()
            .criar(
                org,
                NovoAgente {
                    nome: "Bia".to_owned(),
                    slug: "bia".to_owned(),
                    ..NovoAgente::default()
                },
            )
            .expect("cria o agente")
            .id
    }

    fn conversa(&self, org: OrgId, agente: AgentId) -> poph_types::ConversationId {
        self.store
            .conversas()
            .criar(
                org,
                NovaConversa {
                    agent_id: agente,
                    integration_id: None,
                    canal: "whatsapp".to_owned(),
                    peer_externo: Some("5511999999999".to_owned()),
                    departamento_alvo: None,
                },
            )
            .expect("cria a conversa")
            .id
    }

    fn mensagem(&self, org: OrgId, conversa: poph_types::ConversationId, texto: &str) -> u64 {
        let mensagem = self
            .store
            .mensagens()
            .gravar(
                org,
                NovaMensagem {
                    conversation_id: conversa,
                    direcao: Direcao::Entrada,
                    papel: Papel::Usuario,
                    origem: Origem::Agente,
                    autor_nome: None,
                    texto: texto.to_owned(),
                    anexos_json: "[]".to_owned(),
                    id_externo: None,
                    turn_id: None,
                },
            )
            .expect("grava a mensagem");
        self.store
            .mensagens()
            .desde(0, 1000)
            .expect("lê as mensagens")
            .into_iter()
            .find(|lida| lida.id == mensagem.id)
            .expect("mensagem recém-gravada")
            .rowid
    }

    fn departamento(&self, org: OrgId, nome: &str) -> DepartmentId {
        self.store
            .departamentos()
            .criar(
                org,
                NovoDepartamento {
                    nome: nome.to_owned(),
                    descricao: String::new(),
                },
            )
            .expect("cria o departamento")
            .id
    }

    /// Serviço de espaços com a porta real sobre o banco, como a raiz de composição monta.
    fn espacos(&self) -> EspacosDoModulo {
        Espacos::da_config(
            poph_matrix::ClienteMatrix::das_partes(&self.modulo().config, &self.modulo().segredos)
                .expect("cliente do Matrix"),
            PortaEspacosStore(Arc::clone(&self.store)),
            &self.modulo().config,
        )
        .expect("espaços")
    }
}

impl Drop for Cenario {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.pasta);
    }
}

// ───────────────────────────── HTTP ─────────────────────────────

/// Sobe o router do servidor numa porta efêmera.
async fn servir(estado: Estado) -> (String, oneshot::Sender<()>) {
    let app = poph_server::router(estado);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("porta efêmera");
    let endereco = format!("http://{}", listener.local_addr().expect("endereço"));
    let (desligar, sinal) = oneshot::channel();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = sinal.await;
            })
            .await;
    });
    (endereco, desligar)
}

struct Resposta {
    status: StatusCode,
    json: Value,
    texto: String,
    etag: Option<String>,
}

async fn pedir(
    base: &str,
    metodo: MetodoHttp,
    caminho: &str,
    token: Option<&str>,
    corpo: Option<Value>,
    if_match: Option<&str>,
) -> Resposta {
    let cliente = Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .expect("cliente HTTP");
    let mut pedido = cliente.request(metodo, format!("{base}{caminho}"));
    if let Some(token) = token {
        pedido = pedido.bearer_auth(token);
    }
    if let Some(valor) = if_match {
        pedido = pedido.header("If-Match", valor);
    }
    if let Some(corpo) = corpo {
        pedido = pedido.json(&corpo);
    }
    let resposta = pedido.send().await.expect("pedido HTTP");
    let status = resposta.status();
    let etag = resposta
        .headers()
        .get("etag")
        .and_then(|valor| valor.to_str().ok())
        .map(str::to_owned);
    let texto = resposta.text().await.unwrap_or_default();
    let json = serde_json::from_str(&texto).unwrap_or(Value::Null);
    Resposta {
        status,
        json,
        texto,
        etag,
    }
}

async fn esperar_ate(descricao: &str, mut condicao: impl FnMut() -> bool) {
    let inicio = Instant::now();
    while inicio.elapsed() < Duration::from_secs(20) {
        if condicao() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("tempo esgotado esperando: {descricao}");
}

// ───────────────────────────── estado e liga/desliga ─────────────────────────────

#[tokio::test]
async fn estado_da_org_traz_atraso_falhas_e_etag() {
    let cenario = Cenario::novo("estado", "http://127.0.0.1:9".to_owned());
    let org = cenario.org("estado");
    let token = cenario.token(org, "estado");
    let agente = cenario.agente(org);
    let conversa = cenario.conversa(org, agente);
    let primeira = cenario.mensagem(org, conversa, "primeira");
    cenario.mensagem(org, conversa, "segunda");
    // Uma das duas já foi espelhada: o atraso tem de ser 1.
    cenario
        .store
        .matrix()
        .avancar_mensagens(org, primeira)
        .expect("avança o cursor");
    // A mensagem pendente é mais velha que o relógio atual em 40 s.
    let pendente = cenario
        .store
        .mensagens()
        .desde(primeira, 10)
        .expect("lê a leitura incremental")
        .first()
        .map(|mensagem| mensagem.criado_em)
        .expect("mensagem pendente");
    // Relógio 65 s à frente da mensagem pendente: a idade tem de ser 65 s.
    cenario.relogio.store(pendente + 65_000, Ordering::SeqCst);
    // Uma falha desta org e uma da outra, para o isolamento valer também aqui.
    cenario
        .store
        .matrix()
        .registrar_falha(
            org,
            poph_types::MessageId::generate(),
            "formato_invalido",
            1,
        )
        .expect("registra a falha");
    let outra = cenario.org("outra");
    cenario
        .store
        .matrix()
        .registrar_falha(
            outra,
            poph_types::MessageId::generate(),
            "formato_invalido",
            1,
        )
        .expect("falha da outra org");

    // Sem o módulo montado nesta instância: `connected` é nulo, e o resto é lido do banco.
    let estado = cenario.estado();
    let (base, desligar) = servir(estado).await;
    let rota = format!("/admin/v1/orgs/{org}/matrix");
    let resposta = pedir(&base, MetodoHttp::GET, &rota, Some(&token), None, None).await;
    assert_eq!(resposta.status, StatusCode::OK, "{}", resposta.texto);
    assert_eq!(resposta.json["org_id"], json!(org.to_string()));
    assert_eq!(resposta.json["enabled"], json!(false));
    assert_eq!(resposta.json["enabled_at"], Value::Null);
    assert_eq!(
        resposta.json["connected"],
        Value::Null,
        "sem módulo nesta instância, `connected` é nulo"
    );
    assert_eq!(resposta.json["backlog"]["messages"], json!(1));
    assert_eq!(
        resposta.json["backlog"]["oldest_seconds"],
        json!(65),
        "a idade é a diferença entre agora e a mensagem mais antiga pendente"
    );
    assert_eq!(resposta.json["pending_failures"], json!(1));
    assert_eq!(resposta.json["last_failure"], json!("formato_invalido"));
    let updated_at = resposta.json["updated_at"].as_i64().expect("updated_at");
    assert_eq!(
        resposta.etag.as_deref(),
        Some(format!("\"{updated_at}\"").as_str()),
        "o ETag é o updated_at da linha"
    );
    // Nada de dado de cliente na resposta.
    assert!(
        !resposta.texto.contains("5511999999999"),
        "o estado não pode trazer telefone nem texto de cliente"
    );

    let _ = desligar.send(());
}

#[tokio::test]
async fn estado_da_org_diz_conectado_quando_a_sonda_fala_com_o_synapse() {
    let synapse = Synapse::subir().await;
    let cenario = Cenario::novo("conectado", synapse.endereco.clone());
    let org = cenario.org("conectado");
    let token = cenario.token(org, "conectado");

    let mut estado = cenario.estado();
    let rodando = matrix_inicio::iniciar_com_ajustes(
        &mut estado,
        cenario.modulo(),
        AjustesDeInicio {
            periodo_da_sonda: Duration::from_millis(20),
            teto_da_sonda: Duration::from_millis(50),
            prazo_de_parada: Duration::from_secs(2),
        },
    )
    .expect("módulo ligado");
    let (base, desligar) = servir(estado.clone()).await;
    esperar_ate("sonda falar com o Synapse falso", || {
        estado.matrix_conectado() == Some(true)
    })
    .await;

    let rota = format!("/admin/v1/orgs/{org}/matrix");
    let resposta = pedir(&base, MetodoHttp::GET, &rota, Some(&token), None, None).await;
    assert_eq!(resposta.status, StatusCode::OK, "{}", resposta.texto);
    assert_eq!(
        resposta.json["connected"],
        json!(true),
        "com o Synapse respondendo, a API diz conectado"
    );
    assert!(
        resposta.json["backlog"]["messages"].as_u64().is_some(),
        "o atraso é número, não texto"
    );

    rodando.parar().await;
    let _ = desligar.send(());
}

#[tokio::test]
async fn ligar_e_desligar_exigem_if_match_e_preservam_o_corte() {
    let cenario = Cenario::novo("liga", "http://127.0.0.1:9".to_owned());
    let org = cenario.org("liga");
    let token = cenario.token(org, "liga");
    let estado = cenario.estado();
    let (base, desligar) = servir(estado).await;
    let rota = format!("/admin/v1/orgs/{org}/matrix");

    // Sem o cabeçalho: 428.
    let sem = pedir(
        &base,
        MetodoHttp::PUT,
        &rota,
        Some(&token),
        Some(json!({"enabled": true})),
        None,
    )
    .await;
    assert_eq!(
        sem.status,
        StatusCode::PRECONDITION_REQUIRED,
        "{}",
        sem.texto
    );
    assert_eq!(sem.json["code"], json!("precondition_required"));

    // Leitura inicial: nunca ligou, `updated_at` é 0, sem `enabled_at`.
    let inicial = pedir(&base, MetodoHttp::GET, &rota, Some(&token), None, None).await;
    assert_eq!(inicial.status, StatusCode::OK);
    assert_eq!(inicial.json["updated_at"], json!(0));

    // Liga com o valor lido: 200 e `enabled_at` gravado pelo repositório.
    let ligada = pedir(
        &base,
        MetodoHttp::PUT,
        &rota,
        Some(&token),
        Some(json!({"enabled": true})),
        Some("0"),
    )
    .await;
    assert_eq!(ligada.status, StatusCode::OK, "{}", ligada.texto);
    assert_eq!(ligada.json["enabled"], json!(true));
    let corte = ligada.json["enabled_at"]
        .as_i64()
        .expect("corte do backfill");
    let etag = ligada.json["updated_at"].as_i64().expect("updated_at");
    assert!(etag > 0);

    // Repetir com valor velho: 412, e nada muda no banco.
    let velha = pedir(
        &base,
        MetodoHttp::PUT,
        &rota,
        Some(&token),
        Some(json!({"enabled": false})),
        Some("0"),
    )
    .await;
    assert_eq!(
        velha.status,
        StatusCode::PRECONDITION_FAILED,
        "{}",
        velha.texto
    );
    assert_eq!(velha.json["code"], json!("version_conflict"));
    assert!(
        cenario.store.matrix().estado(org).expect("estado").ligado,
        "o 412 não pode desligar a org"
    );

    // Repetir o mesmo pedido com o valor atual: aceito (é idempotente, não é conflito).
    let de_novo = pedir(
        &base,
        MetodoHttp::PUT,
        &rota,
        Some(&token),
        Some(json!({"enabled": true})),
        Some(&etag.to_string()),
    )
    .await;
    assert_eq!(de_novo.status, StatusCode::OK, "{}", de_novo.texto);
    assert_eq!(
        de_novo.json["enabled_at"],
        json!(corte),
        "religar não move o corte"
    );

    // Desliga: `enabled_at` é preservado e a sala/espaço não são apagados.
    let etag_atual = de_novo.json["updated_at"].as_i64().expect("updated_at");
    let desligada = pedir(
        &base,
        MetodoHttp::PUT,
        &rota,
        Some(&token),
        Some(json!({"enabled": false})),
        Some(&etag_atual.to_string()),
    )
    .await;
    assert_eq!(desligada.status, StatusCode::OK, "{}", desligada.texto);
    assert_eq!(desligada.json["enabled"], json!(false));
    assert_eq!(
        desligada.json["enabled_at"],
        json!(corte),
        "o corte fica gravado"
    );
    assert!(
        !desligada.texto.contains(TOKEN_GLOBAL),
        "token global não vaza"
    );
    assert!(!desligada.texto.contains(&token), "token de org não vaza");

    let _ = desligar.send(());
}

#[tokio::test]
async fn token_de_outra_org_e_org_inexistente() {
    let cenario = Cenario::novo("isolamento", "http://127.0.0.1:9".to_owned());
    let org_a = cenario.org("a");
    let org_b = cenario.org("b");
    let token_a = cenario.token(org_a, "a");
    let estado = cenario.estado();
    let (base, desligar) = servir(estado).await;

    let da_b = format!("/admin/v1/orgs/{org_b}/matrix");
    let sem_token = pedir(&base, MetodoHttp::GET, &da_b, None, None, None).await;
    assert_eq!(sem_token.status, StatusCode::UNAUTHORIZED);

    let cruzado = pedir(&base, MetodoHttp::GET, &da_b, Some(&token_a), None, None).await;
    assert_eq!(cruzado.status, StatusCode::FORBIDDEN, "{}", cruzado.texto);

    let cruzado_put = pedir(
        &base,
        MetodoHttp::PUT,
        &da_b,
        Some(&token_a),
        Some(json!({"enabled": true})),
        Some("0"),
    )
    .await;
    assert_eq!(cruzado_put.status, StatusCode::FORBIDDEN);
    assert!(
        !cenario.store.matrix().estado(org_b).expect("estado").ligado,
        "o ataque não liga o módulo na outra org"
    );

    // Org que não existe: `404` (o token global alcança qualquer org, mas não inventa uma).
    let inexistente = poph_types::OrgId::generate();
    let fantasma = format!("/admin/v1/orgs/{inexistente}/matrix");
    let nao_existe = pedir(
        &base,
        MetodoHttp::GET,
        &fantasma,
        Some(TOKEN_GLOBAL),
        None,
        None,
    )
    .await;
    assert_eq!(
        nao_existe.status,
        StatusCode::NOT_FOUND,
        "{}",
        nao_existe.texto
    );

    let espaco_fantasma = format!("/admin/v1/orgs/{inexistente}/matrix/spaces");
    let sem_org = pedir(
        &base,
        MetodoHttp::GET,
        &espaco_fantasma,
        Some(TOKEN_GLOBAL),
        None,
        None,
    )
    .await;
    assert_eq!(sem_org.status, StatusCode::NOT_FOUND);

    // O dono continua lendo a própria org.
    let propria = format!("/admin/v1/orgs/{org_a}/matrix");
    let dono = pedir(&base, MetodoHttp::GET, &propria, Some(&token_a), None, None).await;
    assert_eq!(dono.status, StatusCode::OK);

    let _ = desligar.send(());
}

// ───────────────────────────── espaços ─────────────────────────────

#[tokio::test]
async fn espaco_do_departamento_e_criado_uma_vez_ligado_e_desligado() {
    let synapse = Synapse::subir().await;
    let cenario = Cenario::novo("espacos", synapse.endereco.clone());
    let org = cenario.org("espacos");
    let token = cenario.token(org, "espacos");
    let comercial = cenario.departamento(org, "Comercial");
    let outro = cenario.departamento(org, "Outro");

    // Só o serviço de espaços na API (sem os laços do módulo: a asserção fica determinística).
    let mut estado = cenario.estado();
    estado.ligar_espacos_do_matrix(Arc::new(cenario.espacos()));
    let (base, desligar) = servir(estado).await;
    let rota = format!("/admin/v1/orgs/{org}/matrix/spaces");

    // Criar: cria o espaço automático do departamento.
    let criado = pedir(
        &base,
        MetodoHttp::PUT,
        &rota,
        Some(&token),
        Some(json!({"department_id": comercial.to_string()})),
        None,
    )
    .await;
    assert_eq!(criado.status, StatusCode::OK, "{}", criado.texto);
    assert_eq!(criado.json["department_id"], json!(comercial.to_string()));
    assert_eq!(criado.json["department_name"], json!("Comercial"));
    assert_eq!(criado.json["origin"], json!("auto"));
    let espaco = criado.json["space_room_id"]
        .as_str()
        .expect("espaço")
        .to_owned();
    assert_eq!(synapse.quantidade_de_espacos(), 1);
    let no_synapse = synapse
        .salas()
        .into_iter()
        .find(|(id, _)| id == &espaco)
        .expect("o espaço existe no Synapse")
        .1;
    assert_eq!(
        no_synapse.nome.as_deref(),
        Some("Comercial"),
        "o espaço criado leva o nome do departamento"
    );
    assert_eq!(no_synapse.tipo.as_deref(), Some("m.space"));

    // Repetir não cria segundo espaço (o alias é a trava).
    let de_novo = pedir(
        &base,
        MetodoHttp::PUT,
        &rota,
        Some(&token),
        Some(json!({"department_id": comercial.to_string()})),
        None,
    )
    .await;
    assert_eq!(de_novo.status, StatusCode::OK, "{}", de_novo.texto);
    assert_eq!(de_novo.json["space_room_id"], json!(espaco));
    assert_eq!(synapse.quantidade_de_espacos(), 1, "um espaço só");

    // Ligar um espaço que já existe no Element, mas sem o bot dentro: 409 com o que fazer, e nada
    // gravado. A sala é criada no Synapse falso como outro usuário (o dono do espaço).
    let do_operador = synapse.criar_sala("@operador:matrix-dev.local").await;
    let sem_permissoo = pedir(
        &base,
        MetodoHttp::PUT,
        &rota,
        Some(&token),
        Some(json!({"department_id": outro.to_string(), "space": do_operador})),
        None,
    )
    .await;
    assert_eq!(
        sem_permissoo.status,
        StatusCode::CONFLICT,
        "{}",
        sem_permissoo.texto
    );
    assert_eq!(sem_permissoo.json["code"], json!("conflict"));
    let mensagem = sem_permissoo.json["message"].as_str().unwrap_or_default();
    assert!(
        mensagem.contains("convite") || mensagem.contains("convite") || mensagem.contains("poder"),
        "o erro diz o que o operador faz no Element: {mensagem}"
    );
    assert!(
        cenario
            .store
            .matrix()
            .espaco(org, outro)
            .expect("lê")
            .is_none(),
        "a falha de permissão não grava espaço"
    );

    // Espaço informado inválido: 400 de validação.
    let invalido = pedir(
        &base,
        MetodoHttp::PUT,
        &rota,
        Some(&token),
        Some(json!({"department_id": outro.to_string(), "space": "nem-id-nem-alias"})),
        None,
    )
    .await;
    assert_eq!(
        invalido.status,
        StatusCode::BAD_REQUEST,
        "{}",
        invalido.texto
    );

    // Lista: um item, com o nome do departamento.
    let lista = pedir(&base, MetodoHttp::GET, &rota, Some(&token), None, None).await;
    assert_eq!(lista.status, StatusCode::OK, "{}", lista.texto);
    let itens = lista.json["items"].as_array().expect("lista").clone();
    assert_eq!(itens.len(), 1);
    assert_eq!(itens[0]["department_id"], json!(comercial.to_string()));
    assert_eq!(itens[0]["department_name"], json!("Comercial"));

    // Uma sala dentro do espaço, para o `DELETE` ter o que esquecer.
    let agente = cenario.agente(org);
    let conversa = cenario.conversa(org, agente);
    cenario
        .store
        .matrix()
        .garantir_sala(org, conversa, "!sala:matrix-dev.local", Some(&espaco))
        .expect("sala no espaço");

    // Desligar: apaga a linha e esquece o vínculo da sala; nada é apagado no Synapse.
    let desligado = pedir(
        &base,
        MetodoHttp::DELETE,
        &format!("{rota}/{comercial}"),
        Some(&token),
        None,
        None,
    )
    .await;
    assert_eq!(desligado.status, StatusCode::OK, "{}", desligado.texto);
    assert_eq!(desligado.json["removed"], json!(true));
    assert_eq!(desligado.json["rooms_forgotten"], json!(1));
    assert_eq!(
        synapse.quantidade_de_espacos(),
        1,
        "o espaço continua no Synapse (desligar não apaga sala)"
    );
    let depois = pedir(&base, MetodoHttp::GET, &rota, Some(&token), None, None).await;
    assert_eq!(depois.json["items"].as_array().expect("lista").len(), 0);
    assert_eq!(
        cenario
            .store
            .matrix()
            .sala(org, conversa)
            .expect("sala")
            .expect("existe")
            .espaco,
        None,
        "a sala continua existindo, só sai do registro do módulo"
    );

    // Departamento inexistente (ou de outra org): 404.
    let outra_org = cenario.org("outra-espacos");
    let departamento_da_outra = cenario.departamento(outra_org, "Dela");
    let alheio = pedir(
        &base,
        MetodoHttp::DELETE,
        &format!("{rota}/{departamento_da_outra}"),
        Some(&token),
        None,
        None,
    )
    .await;
    assert_eq!(alheio.status, StatusCode::NOT_FOUND, "{}", alheio.texto);

    let _ = desligar.send(());
}

#[tokio::test]
async fn modulo_desligado_recusa_espaco_e_mostra_conectado_nulo() {
    let cenario = Cenario::novo("sem-modulo", "http://127.0.0.1:9".to_owned());
    let org = cenario.org("sem-modulo");
    let token = cenario.token(org, "sem-modulo");
    let departamento = cenario.departamento(org, "Comercial");

    let mut config = cenario.config.clone();
    config.matrix = None;
    let estado = Estado::novo(Arc::clone(&cenario.store), config).expect("estado sem módulo");
    let (base, desligar) = servir(estado).await;

    let rota = format!("/admin/v1/orgs/{org}/matrix");
    let resposta = pedir(&base, MetodoHttp::GET, &rota, Some(&token), None, None).await;
    assert_eq!(resposta.status, StatusCode::OK, "{}", resposta.texto);
    assert_eq!(
        resposta.json["connected"],
        Value::Null,
        "sem módulo nesta instância, `connected` é nulo"
    );

    let espaco = pedir(
        &base,
        MetodoHttp::PUT,
        &format!("{rota}/spaces"),
        Some(&token),
        Some(json!({"department_id": departamento.to_string()})),
        None,
    )
    .await;
    assert_eq!(espaco.status, StatusCode::CONFLICT, "{}", espaco.texto);
    assert_eq!(espaco.json["code"], json!("matrix_desligado"));

    let _ = desligar.send(());
}
