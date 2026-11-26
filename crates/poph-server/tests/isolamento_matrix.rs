//! Isolamento entre organizações e segredos no módulo Matrix (MX-28; critérios A9 e A10).
//!
//! Segue o molde de `isolamento_orgs.rs`: servidor HTTP de verdade, banco de verdade em pasta
//! temporária e **Synapse falso** em `127.0.0.1`, com duas organizações que têm agente, integração
//! de WhatsApp, conversa, sala do módulo, departamento e token — cada uma atacando a outra.
//!
//! O que se prova, cenário por cenário:
//!
//! 1. **Evento do Application Service** na sala da org A trazendo ids da org B no corpo (id de
//!    conversa, org, agente) → a resposta é gravada na conversa de A, **nada** é gravado em B e a
//!    resposta HTTP continua `200` (erro de conteúdo nunca vira `4xx`). A conversa é achada pela
//!    sala, nunca por um id que veio no corpo: a operação atômica da MX-04 também recusa
//!    `(org A, conversa B)`.
//! 2. **Sala que ninguém conhece** com o id da conversa de B no corpo → `200`, nada gravado em
//!    nenhuma org e o contador de salas desconhecidas registra o caso.
//! 3. **Token da org A** nas cinco rotas `/admin/v1/orgs/{B}/matrix*` → `403` e nenhuma escrita;
//!    o dono continua lendo a própria org (controle).
//! 4. **Departamento da outra org** no espaço (`PUT` e `DELETE`) → `404`, nada gravado e nenhum
//!    espaço criado no Synapse; com o departamento da própria org, o `PUT` cria (controle).
//! 5. **Nome do cliente e nome espelhado** ficam na org: `pushName` no webhook da org A não muda a
//!    conversa, a sala nem `client_name_espelhado` da org B, e a passagem do laço de estado
//!    (MX-33) renomeia só a sala de A.
//! 6. **Varredura de valor-canário**: nem o `hs_token` nem o `as_token` aparecem em resposta HTTP,
//!    log capturado, mensagem de erro, arquivo criado na pasta do teste ou item da fila de saída.
//!
//! Cada cenário tem a prova de que morde: a mutação aplicada (a conferência de org removida) e o
//! teste que cai com ela estão no relatório da tarefa.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, SystemTime};

use argon2::Argon2;
use argon2::password_hash::{PasswordHasher, SaltString};
use axum::Router;
use axum::extract::State;
use axum::http::header::AUTHORIZATION;
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use poph_evolution::webhook::CABECALHO_SEGREDO;
use poph_matrix::{ClienteMatrix, ConfigCliente, Espacos, IdsFantasmas, UsuariosVirtuais};
use poph_server::evolution::CanalEvolution;
use poph_server::matrix_estado::{DestinoDoSynapse, EstadoDasConversas, PortaEstadoStore};
use poph_server::matrix_inicio::PortaEspacosStore;
use poph_server::{Estado, PortaAtendimento, PortaDoHarness, ReceptorAs, RespostaDoAtendente};
use poph_store::{
    NovaConversa, NovaOrg, NovoAgente, NovoDepartamento, Origem, Pagina, RespostaHumana,
    RespostaHumanaNova, StatusConversa, Store,
};
use poph_types::config::GlobalTokenConfig;
use poph_types::{
    AgentId, ConversationId, DepartmentId, HarnessConfig, IntegrationId, MatrixConfig, OrgId,
    Secret,
};
use rand_core::OsRng;
use reqwest::{Client, Method as MetodoHttp};
use serde_json::{Value, json};
use tokio::sync::oneshot;
use tracing_subscriber::fmt::MakeWriter;

// ───────────────────────────────── constantes ─────────────────────────────────

const PREFIXO: &str = "ph";
const SERVIDOR: &str = "matrix-dev.local";
const INSTANCIA: &str = "loja";

/// Token que o Harness apresenta ao Synapse. Valor-canário: não pode aparecer em lugar nenhum.
const AS_TOKEN: &str = "CANARIO-AS-mx28-3c1d-TAO-SECRETO";
/// Token que o Synapse apresenta ao Harness. Valor-canário.
const HS_TOKEN: &str = "CANARIO-HS-mx28-77a0-TAO-SECRETO";
/// Token global da instância. Valor-canário.
const TOKEN_GLOBAL: &str = "pophg_teste-global-mx28";

const CHAVE_EVO: &str = "chave-evo-mx28-teste";
const SEGREDO_WEBHOOK: &str = "segredo-webhook-mx28";

/// Número do cliente: **o mesmo** nas duas orgs, de propósito — é o caso em que um cruzamento de
/// org apareceria primeiro.
const TELEFONE: &str = "5511999990028";
const JID: &str = "5511999990028@s.whatsapp.net";

const ATENDENTE: &str = "@ana:matrix-dev.local";
const NOME_ATENDENTE: &str = "Ana Atendente";

const SALA_A: &str = "!sala-mx28-a:matrix-dev.local";
const SALA_B: &str = "!sala-mx28-b:matrix-dev.local";

/// Texto do atendente. Valor-canário: não pode vazar para a conversa da outra org.
const TEXTO_ATENDENTE: &str = "CANARIO-TEXTO-mx28-resposta-do-atendente";

/// Nome que o cliente da org A escolheu no WhatsApp.
const NOME_CLIENTE_A: &str = "Cliente da Org A";

// ───────────────────────────────── captura de logs ─────────────────────────────────

#[derive(Clone)]
struct Captura(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for Captura {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for Captura {
    type Writer = Captura;
    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

/// Um coletor de logs para o processo de testes inteiro (o subscriber é global).
fn logs_capturados() -> Arc<Mutex<Vec<u8>>> {
    static LOGS: OnceLock<Arc<Mutex<Vec<u8>>>> = OnceLock::new();
    LOGS.get_or_init(|| {
        let buffer = Arc::new(Mutex::new(Vec::new()));
        let _ = tracing_subscriber::fmt()
            .with_writer(Captura(Arc::clone(&buffer)))
            .with_ansi(false)
            .with_max_level(tracing::Level::DEBUG)
            .try_init();
        buffer
    })
    .clone()
}

fn logs_como_texto() -> String {
    String::from_utf8_lossy(&logs_capturados().lock().unwrap()).into_owned()
}

// ───────────────────────────────── Synapse falso ─────────────────────────────────

#[derive(Default, Clone)]
struct SalaFalsa {
    nome: Option<String>,
    tipo: Option<String>,
    membros: Vec<String>,
    convidados: Vec<String>,
    estado: BTreeMap<(String, String), Value>,
    /// `(txnId, tipo, corpo)` enviados por `send`.
    mensagens: Vec<(String, String, Value)>,
}

#[derive(Default)]
struct EstadoFalso {
    usuarios: BTreeMap<String, Option<String>>,
    salas: BTreeMap<String, SalaFalsa>,
    aliases: BTreeMap<String, String>,
    proxima_sala: u32,
    versoes: u32,
    /// URLs (caminho + consulta) que chegaram com token dentro: o token só pode ir no cabeçalho.
    urls_com_token: Vec<String>,
}

fn responder(status: u16, corpo: Value) -> Response {
    (
        StatusCode::from_u16(status).expect("status válido"),
        [("content-type", "application/json")],
        corpo.to_string(),
    )
        .into_response()
}

/// Percent-decoding de um segmento de caminho (o alias chega com `%23` no lugar do `#`).
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

/// Valor de um parâmetro da consulta (`?user_id=@…`).
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
    let url = uri.to_string();
    if url.contains(AS_TOKEN) || url.contains(HS_TOKEN) {
        guarda.urls_com_token.push(url);
    }

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

    // A sonda de conexão do módulo fala com `/_matrix/client/versions` (sem `/v3`).
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
            let sala = format!("!criada-{}:{SERVIDOR}", guarda.proxima_sala);
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
    async fn subir(nome_do_atendente: &str) -> Self {
        let estado = Arc::new(Mutex::new(EstadoFalso::default()));
        {
            let mut guarda = estado.lock().expect("trava");
            // O atendente existe no Synapse com nome: é o prefixo da resposta que vai ao cliente.
            guarda
                .usuarios
                .insert(ATENDENTE.to_owned(), Some(nome_do_atendente.to_owned()));
        }
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

    /// Registra a sala e quem está dentro dela (o fantasma do agente, na operação).
    fn registrar_sala(&self, sala: &str, dentro: &[&str]) {
        let mut guarda = self.estado.lock().expect("trava");
        guarda.salas.insert(
            sala.to_owned(),
            SalaFalsa {
                membros: dentro.iter().map(|id| (*id).to_owned()).collect(),
                ..SalaFalsa::default()
            },
        );
    }

    fn espacos(&self) -> Vec<(String, SalaFalsa)> {
        self.estado
            .lock()
            .expect("trava")
            .salas
            .iter()
            .filter(|(_, sala)| sala.tipo.as_deref() == Some("m.space"))
            .map(|(id, sala)| (id.clone(), sala.clone()))
            .collect()
    }

    /// Eventos de estado `tipo`/`chave` escritos na sala (o nome da sala é `m.room.name`).
    fn estado_da_sala(&self, sala: &str, tipo: &str, chave: &str) -> Option<Value> {
        self.estado
            .lock()
            .expect("trava")
            .salas
            .get(sala)
            .and_then(|sala| {
                sala.estado
                    .get(&(tipo.to_owned(), chave.to_owned()))
                    .cloned()
            })
    }
}

impl Drop for Synapse {
    fn drop(&mut self) {
        if let Some(desligar) = self.desligar.take() {
            let _ = desligar.send(());
        }
    }
}

// ───────────────────────────────── cenário ─────────────────────────────────

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
    org_a: OrgId,
    conversa_a: ConversationId,
    dept_a: DepartmentId,
    org_b: OrgId,
    conversa_b: ConversationId,
    dept_b: DepartmentId,
    token_a: String,
    token_b: String,
    synapse: Synapse,
}

impl Cenario {
    async fn novo(rotulo: &str) -> Self {
        // O coletor de log é global e só vale do instante em que é instalado: instalar antes de
        // qualquer chamada do cenário, senão o evento que o teste quer observar se perde.
        logs_capturados();
        let carimbo = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|duracao| duracao.as_nanos())
            .unwrap_or(0);
        let pasta = std::env::temp_dir().join(format!(
            "poph-isolamento-matrix-{rotulo}-{}-{carimbo}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&pasta);
        fs::create_dir_all(pasta.join("dados")).expect("pasta de dados");

        let synapse = Synapse::subir(NOME_ATENDENTE).await;

        let mut config = HarnessConfig::default();
        config.data.directory = pasta.join("dados");
        config.admin.tokens_globais = vec![GlobalTokenConfig {
            label: "teste".to_owned(),
            hash: hash_argon2(TOKEN_GLOBAL),
        }];
        config.matrix = Some(MatrixConfig {
            enabled: true,
            homeserver_url: synapse.endereco.clone(),
            server_name: SERVIDOR.to_owned(),
            prefix: PREFIXO.to_owned(),
            concurrency: 4,
            interval_ms: 50,
            ..MatrixConfig::default()
        });
        config.validar().expect("configuração válida");

        let store = Arc::new(Store::abrir(&config).expect("abre o banco"));

        let a = Self::org_com_conversa(&store, "org-a", "Sala A", "bia-a", SALA_A);
        let b = Self::org_com_conversa(&store, "org-b", "Sala B", "bia-b", SALA_B);
        let dept_a = Self::departamento(&store, a.0, "Comercial");
        let dept_b = Self::departamento(&store, b.0, "Financeiro");

        synapse.registrar_sala(SALA_A, &["@ph_ag_agente-a:matrix-dev.local"]);
        synapse.registrar_sala(SALA_B, &["@ph_ag_agente-b:matrix-dev.local"]);

        let token_a = Self::token(&store, a.0, "mx28-a");
        let token_b = Self::token(&store, b.0, "mx28-b");

        Self {
            pasta,
            store,
            config,
            org_a: a.0,
            conversa_a: a.1,
            dept_a,
            org_b: b.0,
            conversa_b: b.1,
            dept_b,
            token_a,
            token_b,
            synapse,
        }
    }

    /// Org + agente + integração de WhatsApp + conversa + sala `ready` do módulo.
    fn org_com_conversa(
        store: &Store,
        slug: &str,
        nome_agente: &str,
        slug_agente: &str,
        sala: &str,
    ) -> (OrgId, ConversationId, AgentId) {
        let org = store
            .orgaos()
            .criar(NovaOrg {
                nome: format!("Org {slug}"),
                slug: slug.to_owned(),
                config_json: String::new(),
            })
            .expect("cria a org")
            .id;
        let agente = store
            .agentes()
            .criar(
                org,
                NovoAgente {
                    nome: nome_agente.to_owned(),
                    slug: slug_agente.to_owned(),
                    ..NovoAgente::default()
                },
            )
            .expect("cria o agente")
            .id;
        let integracao: IntegrationId = store
            .integracao_criar(org, agente, "whatsapp".to_owned(), "active".to_owned())
            .expect("cria a integração")
            .id
            .parse()
            .expect("id da integração");
        let conversa = store
            .conversas()
            .criar(
                org,
                NovaConversa {
                    agent_id: agente,
                    integration_id: Some(integracao),
                    canal: "whatsapp".to_owned(),
                    // O mesmo contato nas duas orgs: é onde um cruzamento apareceria primeiro.
                    peer_externo: Some(TELEFONE.to_owned()),
                    departamento_alvo: None,
                },
            )
            .expect("cria a conversa")
            .id;
        store.matrix().ligar(org).expect("liga o módulo na org");
        store
            .matrix()
            .garantir_sala(org, conversa, sala, None)
            .expect("grava a sala");
        store
            .matrix()
            .marcar_sala_pronta(org, conversa)
            .expect("sala pronta");
        (org, conversa, agente)
    }

    fn departamento(store: &Store, org: OrgId, nome: &str) -> DepartmentId {
        store
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

    /// Token de org no formato que a API autentica: `poph_<id>_<segredo>`.
    fn token(store: &Store, org: OrgId, rotulo: &str) -> String {
        let segredo = format!("segredo-{rotulo}");
        let criado = store
            .tokens()
            .criar(Some(org), rotulo, &hash_argon2(&segredo))
            .expect("cria o token");
        format!("poph_{}_{segredo}", criado.id)
    }

    fn estado(&self) -> Estado {
        Estado::novo(Arc::clone(&self.store), self.config.clone()).expect("estado do servidor")
    }

    fn cliente_matrix(&self) -> ClienteMatrix {
        let config = self.config.matrix.clone().expect("seção [matrix]");
        ClienteMatrix::novo(
            ConfigCliente::novo(
                config.homeserver_url.clone(),
                config.server_name.clone(),
                Secret::new(AS_TOKEN.to_owned()),
            )
            .expect("config do cliente"),
        )
        .expect("cliente do Matrix")
    }

    fn ids(&self) -> IdsFantasmas {
        IdsFantasmas::novo(PREFIXO, SERVIDOR).expect("ids dos fantasmas")
    }

    /// A entrega da resposta do atendente (MX-18), como a raiz de composição monta.
    fn entrega(&self) -> Arc<RespostaDoAtendente> {
        Arc::new(RespostaDoAtendente::novo(
            Arc::new(PortaDoHarness::nova(Arc::clone(&self.store))),
            Some(self.cliente_matrix()),
            self.ids(),
        ))
    }

    /// O canal da Evolution de uma org, com o segredo do webhook (sem cliente: o teste não usa
    /// mídia).
    fn canal(&self, org: OrgId, conversa: ConversationId) -> CanalEvolution {
        let integracao = self
            .store
            .conversas()
            .obter(org, conversa)
            .expect("lê a conversa")
            .integration_id
            .expect("a conversa tem integração");
        CanalEvolution {
            org,
            instancia: INSTANCIA.to_owned(),
            integracao,
            segredo: Some(Secret::new(SEGREDO_WEBHOOK.to_owned())),
            cliente: None,
            teto_midia: 0,
        }
    }

    fn conversa(&self, org: OrgId) -> ConversationId {
        if org == self.org_a {
            self.conversa_a
        } else {
            self.conversa_b
        }
    }

    fn mensagens(&self, org: OrgId) -> Vec<poph_store::Mensagem> {
        self.store
            .mensagens()
            .listar(org, self.conversa(org), Pagina::com_limite(50))
            .expect("lista as mensagens")
    }

    fn status(&self, org: OrgId) -> StatusConversa {
        self.store
            .conversas()
            .obter(org, self.conversa(org))
            .expect("lê a conversa")
            .status
    }

    fn nome_cliente(&self, org: OrgId) -> Option<String> {
        self.store
            .conversas()
            .obter(org, self.conversa(org))
            .expect("lê a conversa")
            .nome_cliente
    }

    fn nome_espelhado(&self, org: OrgId) -> Option<String> {
        self.store
            .matrix()
            .sala(org, self.conversa(org))
            .expect("lê a sala")
            .expect("a sala existe")
            .nome_cliente_espelhado
    }

    fn pendentes(&self, org: OrgId) -> u64 {
        self.store
            .caixa_saida()
            .contar_por_estado(org, poph_store::EstadoSaida::Pendente)
            .expect("conta a fila de saída")
    }

    fn espacos_gravados(&self, org: OrgId) -> usize {
        self.store
            .matrix()
            .listar_espacos(org)
            .expect("lista os espaços")
            .len()
    }

    /// Uma passagem do laço de estado (MX-15), com as portas de verdade, como a MX-20 monta.
    async fn passagem_do_estado(&self) {
        let config = self.config.matrix.clone().expect("seção [matrix]");
        let cliente = self.cliente_matrix();
        let ids = self.ids();
        let usuarios = Arc::new(UsuariosVirtuais::novo(
            cliente.clone(),
            ids,
            poph_matrix::TETO_PADRAO_CACHE,
        ));
        let espacos = Espacos::da_config(
            cliente.clone(),
            PortaEspacosStore(Arc::clone(&self.store)),
            &config,
        )
        .expect("espaços");
        let destino = DestinoDoSynapse::novo(cliente, usuarios, espacos);
        let laco = EstadoDasConversas::novo(
            PortaEstadoStore::nova(Arc::clone(&self.store)),
            destino,
            &config,
        )
        .expect("laço de estado");
        laco.ciclo().await.expect("passagem do estado");
    }
}

impl Drop for Cenario {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.pasta);
    }
}

// ───────────────────────────────── HTTP ─────────────────────────────────

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
    let texto = resposta.text().await.unwrap_or_default();
    let json = serde_json::from_str(&texto).unwrap_or(Value::Null);
    Resposta {
        status,
        json,
        texto,
    }
}

/// Uma transação do Application Service com os eventos dados.
async fn transacao(base: &str, txn: &str, eventos: Value) -> Resposta {
    pedir_com_cabecalho(
        base,
        MetodoHttp::PUT,
        &format!("/_matrix/app/v1/transactions/{txn}"),
        Some(("authorization", &format!("Bearer {HS_TOKEN}"))),
        Some(json!({ "events": eventos })),
    )
    .await
}

async fn pedir_com_cabecalho(
    base: &str,
    metodo: MetodoHttp,
    caminho: &str,
    cabecalho: Option<(&str, &str)>,
    corpo: Option<Value>,
) -> Resposta {
    let cliente = Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .expect("cliente HTTP");
    let mut pedido = cliente.request(metodo, format!("{base}{caminho}"));
    if let Some((nome, valor)) = cabecalho {
        pedido = pedido.header(nome, valor);
    }
    if let Some(corpo) = corpo {
        pedido = pedido.json(&corpo);
    }
    let resposta = pedido.send().await.expect("pedido HTTP");
    let status = resposta.status();
    let texto = resposta.text().await.unwrap_or_default();
    let json = serde_json::from_str(&texto).unwrap_or(Value::Null);
    Resposta {
        status,
        json,
        texto,
    }
}

/// Webhook da Evolution com o segredo certo.
async fn webhook(base: &str, org: OrgId, corpo: Value) -> Resposta {
    pedir_com_cabecalho(
        base,
        MetodoHttp::POST,
        &format!("/webhook/evolution/{org}/{INSTANCIA}"),
        Some((CABECALHO_SEGREDO, SEGREDO_WEBHOOK)),
        Some(corpo),
    )
    .await
}

// ───────────────────────────────── 1 e 2 · Application Service ─────────────────────────────────

#[tokio::test]
async fn evento_da_sala_a_com_ids_da_org_b_nao_toca_a_conversa_da_b() {
    let cenario = Cenario::novo("as-cruzado").await;
    let estado = cenario
        .estado()
        .com_canais_evolution(vec![
            cenario.canal(cenario.org_a, cenario.conversa_a),
            cenario.canal(cenario.org_b, cenario.conversa_b),
        ])
        .com_matrix_as(Arc::new(ReceptorAs::novo(
            PREFIXO,
            SERVIDOR,
            Secret::new(HS_TOKEN.to_owned()),
            cenario.entrega(),
        )));
    let (base, desligar) = servir(estado).await;

    // O evento chega pela sala de A, mas o corpo traz os ids da org B em todos os lugares em que
    // um leitor apressado os procuraria: id de conversa, org, agente e o próprio texto.
    let corpo_do_evento = json!({
        "type": "m.room.message",
        "room_id": SALA_A,
        "sender": ATENDENTE,
        "event_id": "$evento-cruzado:matrix-dev.local",
        "origin_server_ts": 1_795_300_000_000i64,
        "content": {
            "msgtype": "m.text",
            "body": TEXTO_ATENDENTE,
            "conversation_id": cenario.conversa_b.to_string(),
            "org_id": cenario.org_b.to_string(),
            "com.sofacoding.conversation_id": cenario.conversa_b.to_string()
        },
        "conversation_id": cenario.conversa_b.to_string(),
        "org_id": cenario.org_b.to_string()
    });
    let resposta = transacao(&base, "txn-cruzada", json!([corpo_do_evento])).await;
    assert_eq!(
        resposta.status,
        StatusCode::OK,
        "erro de conteúdo nunca vira 4xx: {}",
        resposta.texto
    );
    assert_eq!(resposta.json, json!({}));

    // A conversa de A recebeu a resposta do atendente, e ela foi para a fila dela.
    let mensagens_a = cenario.mensagens(cenario.org_a);
    assert_eq!(mensagens_a.len(), 1, "a sala de A grava a resposta em A");
    assert_eq!(mensagens_a[0].origem, Origem::Humano);
    assert_eq!(mensagens_a[0].texto, TEXTO_ATENDENTE);
    assert_eq!(
        mensagens_a[0].id_externo.as_deref(),
        Some("$evento-cruzado:matrix-dev.local")
    );
    assert_eq!(
        cenario.pendentes(cenario.org_a),
        1,
        "a fila de A tem o item"
    );
    assert_eq!(cenario.status(cenario.org_a), StatusConversa::Transferida);

    // A org B não foi tocada: sem mensagem, sem fila e com a conversa ainda aberta.
    assert!(
        cenario.mensagens(cenario.org_b).is_empty(),
        "o id da conversa de B no corpo não pode gravar em B"
    );
    assert_eq!(
        cenario.pendentes(cenario.org_b),
        0,
        "a fila de B segue vazia"
    );
    assert_eq!(cenario.status(cenario.org_b), StatusConversa::Aberta);

    // A operação atômica da MX-04 também recusa o par cruzado (org de A com conversa de B): a
    // conferência de org é do banco, não do chamador.
    let porta = PortaDoHarness::nova(Arc::clone(&cenario.store));
    let cruzada = porta.registrar_resposta(
        poph_server::matrix_resposta::ConversaDaSala {
            org: cenario.org_a,
            conversa: cenario.conversa_b,
        },
        RespostaHumanaNova {
            evento_id: "$evento-cruzado-na-porta:matrix-dev.local",
            texto_bruto: TEXTO_ATENDENTE,
            texto_enviado: TEXTO_ATENDENTE,
            autor_id: ATENDENTE,
            autor_nome: NOME_ATENDENTE,
        },
    );
    assert!(
        matches!(cruzada, Ok(RespostaHumana::ConversaInexistente)),
        "conversa de outra org é inexistente para esta org: {cruzada:?}"
    );
    assert!(
        cenario.mensagens(cenario.org_b).is_empty(),
        "a tentativa cruzada não grava em B"
    );

    let _ = desligar.send(());
}

#[tokio::test]
async fn sala_desconhecida_com_id_da_org_b_nao_grava_nada_e_conta_o_caso() {
    let cenario = Cenario::novo("sala-desconhecida").await;
    let entrega = cenario.entrega();
    let estado = cenario.estado().com_matrix_as(Arc::new(ReceptorAs::novo(
        PREFIXO,
        SERVIDOR,
        Secret::new(HS_TOKEN.to_owned()),
        entrega.clone(),
    )));
    let (base, desligar) = servir(estado).await;

    // Sala que nenhuma org conhece, com o id da conversa de B no corpo: nada é gravado em lugar
    // nenhum, porque a conversa é achada pela sala e não pelo corpo.
    let resposta = transacao(
        &base,
        "txn-desconhecida",
        json!([{
            "type": "m.room.message",
            "room_id": "!sala-que-ninguem-conhece:matrix-dev.local",
            "sender": ATENDENTE,
            "event_id": "$evento-orfao:matrix-dev.local",
            "origin_server_ts": 1_795_300_000_000i64,
            "content": {
                "msgtype": "m.text",
                "body": TEXTO_ATENDENTE,
                "conversation_id": cenario.conversa_b.to_string()
            },
            "conversation_id": cenario.conversa_b.to_string()
        }]),
    )
    .await;
    assert_eq!(
        resposta.status,
        StatusCode::OK,
        "sala desconhecida é 200 sem processar: {}",
        resposta.texto
    );
    assert!(cenario.mensagens(cenario.org_a).is_empty());
    assert!(cenario.mensagens(cenario.org_b).is_empty());
    assert_eq!(cenario.pendentes(cenario.org_a), 0);
    assert_eq!(cenario.pendentes(cenario.org_b), 0);
    assert_eq!(
        entrega
            .contadores()
            .salas_desconhecidas
            .load(std::sync::atomic::Ordering::Relaxed),
        1,
        "o caso fica contado"
    );
    let logs = logs_como_texto();
    assert!(
        logs.contains("nenhuma org ligada conhece"),
        "o caso fica no log; capturado até agora: {logs}"
    );

    let _ = desligar.send(());
}

// ───────────────────────────────── 3 · token de org na API ─────────────────────────────────

#[tokio::test]
async fn token_da_org_a_nao_opera_as_rotas_da_org_b() {
    let cenario = Cenario::novo("api-cruzada").await;
    // A org B começa com o módulo desligado: assim "o ataque não ligou" é observável.
    cenario
        .store
        .matrix()
        .desligar(cenario.org_b)
        .expect("desliga o módulo na org B");
    let mut estado = cenario.estado();
    let config = cenario.config.matrix.clone().expect("seção [matrix]");
    estado.ligar_espacos_do_matrix(Arc::new(
        Espacos::da_config(
            cenario.cliente_matrix(),
            PortaEspacosStore(Arc::clone(&cenario.store)),
            &config,
        )
        .expect("espaços"),
    ));
    let (base, desligar) = servir(estado).await;

    let da_b = format!("/admin/v1/orgs/{}/matrix", cenario.org_b);
    let espacos_da_b = format!("{da_b}/spaces");
    let departamento_da_b = format!("{espacos_da_b}/{}", cenario.dept_b);

    // As cinco rotas da MX-22, com o token de A no caminho de B: todas `403`.
    let negados = [
        pedir(
            &base,
            MetodoHttp::GET,
            &da_b,
            Some(&cenario.token_a),
            None,
            None,
        )
        .await,
        pedir(
            &base,
            MetodoHttp::PUT,
            &da_b,
            Some(&cenario.token_a),
            Some(json!({"enabled": true})),
            Some("0"),
        )
        .await,
        pedir(
            &base,
            MetodoHttp::GET,
            &espacos_da_b,
            Some(&cenario.token_a),
            None,
            None,
        )
        .await,
        pedir(
            &base,
            MetodoHttp::PUT,
            &espacos_da_b,
            Some(&cenario.token_a),
            Some(json!({"department_id": cenario.dept_b.to_string()})),
            None,
        )
        .await,
        pedir(
            &base,
            MetodoHttp::DELETE,
            &departamento_da_b,
            Some(&cenario.token_a),
            None,
            None,
        )
        .await,
    ];
    for (posicao, resposta) in negados.iter().enumerate() {
        assert_eq!(
            resposta.status,
            StatusCode::FORBIDDEN,
            "rota {posicao} da org B aceitou o token da org A: {}",
            resposta.texto
        );
        assert!(
            !resposta.texto.contains(&cenario.token_a)
                && !resposta.texto.contains(&cenario.token_b),
            "token não aparece na negativa"
        );
    }

    // Nenhuma escrita passou: B não foi ligada, não ganhou espaço e a conversa dela segue aberta.
    assert!(
        !cenario
            .store
            .matrix()
            .estado(cenario.org_b)
            .expect("estado")
            .ligado,
        "o ataque não liga o módulo na outra org"
    );
    assert_eq!(cenario.espacos_gravados(cenario.org_b), 0);
    assert_eq!(cenario.espacos_gravados(cenario.org_a), 0);
    assert_eq!(cenario.status(cenario.org_b), StatusConversa::Aberta);

    // A direção contrária e o controle do dono: B também não alcança A, e B lê a própria org.
    let da_a = format!("/admin/v1/orgs/{}/matrix", cenario.org_a);
    let b_em_a = pedir(
        &base,
        MetodoHttp::GET,
        &da_a,
        Some(&cenario.token_b),
        None,
        None,
    )
    .await;
    assert_eq!(b_em_a.status, StatusCode::FORBIDDEN);
    let dono = pedir(
        &base,
        MetodoHttp::GET,
        &da_b,
        Some(&cenario.token_b),
        None,
        None,
    )
    .await;
    assert_eq!(dono.status, StatusCode::OK, "{}", dono.texto);

    // Sem token continua `401` (a negativa não veio de falta de autenticação).
    let sem_token = pedir(&base, MetodoHttp::GET, &da_b, None, None, None).await;
    assert_eq!(sem_token.status, StatusCode::UNAUTHORIZED);

    let _ = desligar.send(());
}

// ───────────────────────────────── 4 · espaço de departamento alheio ─────────────────────────────────

#[tokio::test]
async fn espaco_com_departamento_da_outra_org_e_recusado() {
    let cenario = Cenario::novo("espaco-cruzado").await;
    let mut estado = cenario.estado();
    let config = cenario.config.matrix.clone().expect("seção [matrix]");
    estado.ligar_espacos_do_matrix(Arc::new(
        Espacos::da_config(
            cenario.cliente_matrix(),
            PortaEspacosStore(Arc::clone(&cenario.store)),
            &config,
        )
        .expect("espaços"),
    ));
    let (base, desligar) = servir(estado).await;

    let rota_a = format!("/admin/v1/orgs/{}/matrix/spaces", cenario.org_a);

    // Criar espaço do departamento de B com o token e o caminho de A: `404`, nada gravado.
    let cruzado = pedir(
        &base,
        MetodoHttp::PUT,
        &rota_a,
        Some(&cenario.token_a),
        Some(json!({"department_id": cenario.dept_b.to_string()})),
        None,
    )
    .await;
    assert_eq!(
        cruzado.status,
        StatusCode::NOT_FOUND,
        "departamento de outra org não existe nesta: {}",
        cruzado.texto
    );
    assert_eq!(
        cenario.espacos_gravados(cenario.org_a),
        0,
        "nada gravado na org do token"
    );
    assert_eq!(
        cenario.espacos_gravados(cenario.org_b),
        0,
        "nada gravado na org dona do departamento"
    );
    assert!(
        cenario.synapse.espacos().is_empty(),
        "nenhum espaço criado no Synapse"
    );

    // Ligar um espaço existente para o departamento de B também é recusado, e nada vai ao Synapse.
    let ligado = pedir(
        &base,
        MetodoHttp::PUT,
        &rota_a,
        Some(&cenario.token_a),
        Some(json!({
            "department_id": cenario.dept_b.to_string(),
            "space": "#sala-do-operador:matrix-dev.local"
        })),
        None,
    )
    .await;
    assert_eq!(ligado.status, StatusCode::NOT_FOUND, "{}", ligado.texto);

    // Desligar o espaço do departamento de B pelo caminho de A: `404`, e o departamento continua.
    let desligado = pedir(
        &base,
        MetodoHttp::DELETE,
        &format!("{rota_a}/{}", cenario.dept_b),
        Some(&cenario.token_a),
        None,
        None,
    )
    .await;
    assert_eq!(
        desligado.status,
        StatusCode::NOT_FOUND,
        "{}",
        desligado.texto
    );
    assert!(
        cenario
            .store
            .departamentos()
            .obter(cenario.org_b, cenario.dept_b)
            .is_ok(),
        "o departamento da org B continua existindo"
    );

    // Controle: o departamento da própria org cria o espaço (o `404` não é genérico).
    let proprio = pedir(
        &base,
        MetodoHttp::PUT,
        &rota_a,
        Some(&cenario.token_a),
        Some(json!({"department_id": cenario.dept_a.to_string()})),
        None,
    )
    .await;
    assert_eq!(proprio.status, StatusCode::OK, "{}", proprio.texto);
    assert_eq!(
        proprio.json["department_id"],
        json!(cenario.dept_a.to_string())
    );
    assert_eq!(cenario.espacos_gravados(cenario.org_a), 1);
    assert_eq!(
        cenario.espacos_gravados(cenario.org_b),
        0,
        "o espaço criado é de A, não de B"
    );
    let espacos = cenario.synapse.espacos();
    assert_eq!(espacos.len(), 1, "um espaço no Synapse, o da org A");
    assert_eq!(
        espacos[0].1.nome.as_deref(),
        Some("Comercial"),
        "o espaço criado leva o nome do departamento da org A"
    );

    let _ = desligar.send(());
}

// ───────────────────────────────── 5 · nome do cliente por org ─────────────────────────────────

#[tokio::test]
async fn nome_do_cliente_fica_na_org_do_webhook_e_nao_renomeia_a_sala_da_outra() {
    let cenario = Cenario::novo("nome").await;
    let estado = cenario.estado().com_canais_evolution(vec![
        cenario.canal(cenario.org_a, cenario.conversa_a),
        cenario.canal(cenario.org_b, cenario.conversa_b),
    ]);
    let (base, desligar) = servir(estado).await;

    // Mensagem do cliente da org A com `pushName`: só a conversa de A ganha nome.
    let recebido = webhook(
        &base,
        cenario.org_a,
        json!({
            "event": "messages.upsert",
            "instance": INSTANCIA,
            "apikey": CHAVE_EVO,
            "data": {
                "key": { "id": "MSG-MX28-1", "remoteJid": JID, "fromMe": false },
                "pushName": NOME_CLIENTE_A,
                "messageType": "conversation",
                "message": { "conversation": "bom dia" },
                "messageTimestamp": 1_795_300_000i64
            }
        }),
    )
    .await;
    assert_eq!(recebido.status, StatusCode::OK, "{}", recebido.texto);

    assert_eq!(
        cenario.nome_cliente(cenario.org_a).as_deref(),
        Some(NOME_CLIENTE_A),
        "o nome do cliente entra na conversa da org A"
    );
    assert_eq!(
        cenario.nome_cliente(cenario.org_b),
        None,
        "a conversa da org B não recebe o nome da org A"
    );
    assert_eq!(
        cenario.nome_espelhado(cenario.org_b),
        None,
        "a sala da org B não recebe o nome da org A"
    );
    assert_eq!(
        cenario
            .store
            .conversas()
            .obter(cenario.org_b, cenario.conversa_b)
            .expect("lê a conversa de B")
            .status,
        StatusConversa::Aberta,
        "a mensagem do cliente de A não muda o estado da conversa de B"
    );

    // A passagem do laço de estado renomeia só a sala de A (MX-33).
    cenario.passagem_do_estado().await;
    assert_eq!(
        cenario.synapse.estado_da_sala(SALA_A, "m.room.name", ""),
        Some(json!({ "name": NOME_CLIENTE_A })),
        "a sala de A recebe o nome do cliente dela"
    );
    assert_eq!(
        cenario.synapse.estado_da_sala(SALA_B, "m.room.name", ""),
        None,
        "a sala de B não é renomeada pela mudança de A"
    );
    assert_eq!(
        cenario.nome_espelhado(cenario.org_a).as_deref(),
        Some(NOME_CLIENTE_A),
        "o nome espelhado fica na org A"
    );
    assert_eq!(
        cenario.nome_espelhado(cenario.org_b),
        None,
        "o nome espelhado da org B continua vazio"
    );

    let _ = desligar.send(());
}

// ───────────────────────────────── 6 · varredura de segredo-canário ─────────────────────────────────

/// Lê todos os arquivos da pasta do teste (o banco e o que o servidor criou) como bytes.
fn arquivos_da_pasta(pasta: &Path) -> Vec<(String, Vec<u8>)> {
    let mut encontrados = Vec::new();
    let mut pilha = vec![pasta.to_path_buf()];
    while let Some(diretorio) = pilha.pop() {
        let Ok(entradas) = fs::read_dir(&diretorio) else {
            continue;
        };
        for entrada in entradas.flatten() {
            let caminho = entrada.path();
            if caminho.is_dir() {
                pilha.push(caminho);
            } else if let Ok(bytes) = fs::read(&caminho) {
                encontrados.push((caminho.display().to_string(), bytes));
            }
        }
    }
    encontrados
}

#[tokio::test]
async fn tokens_canario_nao_aparecem_em_resposta_log_arquivo_nem_fila() {
    let cenario = Cenario::novo("canario").await;
    let mut estado = cenario.estado().com_canais_evolution(vec![
        cenario.canal(cenario.org_a, cenario.conversa_a),
        cenario.canal(cenario.org_b, cenario.conversa_b),
    ]);
    let config = cenario.config.matrix.clone().expect("seção [matrix]");
    estado.ligar_espacos_do_matrix(Arc::new(
        Espacos::da_config(
            cenario.cliente_matrix(),
            PortaEspacosStore(Arc::clone(&cenario.store)),
            &config,
        )
        .expect("espaços"),
    ));
    let estado = estado.com_matrix_as(Arc::new(ReceptorAs::novo(
        PREFIXO,
        SERVIDOR,
        Secret::new(HS_TOKEN.to_owned()),
        cenario.entrega(),
    )));
    let (base, desligar) = servir(estado).await;

    // Exercita os caminhos que têm token: administrativo (200/403/412), Application Service
    // (200/401/403), webhook (200/401) e o aviso na sala (que fala com o Synapse).
    let da_a = format!("/admin/v1/orgs/{}/matrix", cenario.org_a);
    let da_b = format!("/admin/v1/orgs/{}/matrix", cenario.org_b);
    let mut respostas: Vec<Resposta> = Vec::new();
    respostas.push(
        pedir(
            &base,
            MetodoHttp::GET,
            &da_a,
            Some(&cenario.token_a),
            None,
            None,
        )
        .await,
    );
    respostas.push(
        pedir(
            &base,
            MetodoHttp::PUT,
            &da_a,
            Some(&cenario.token_a),
            Some(json!({"enabled": true})),
            Some("999"),
        )
        .await,
    );
    respostas.push(
        pedir(
            &base,
            MetodoHttp::GET,
            &da_b,
            Some(&cenario.token_a),
            None,
            None,
        )
        .await,
    );
    respostas.push(pedir(&base, MetodoHttp::GET, &da_b, None, None, None).await);
    respostas.push(
        transacao(
            &base,
            "txn-canario",
            json!([{
                "type": "m.room.message",
                "room_id": SALA_A,
                "sender": ATENDENTE,
                "event_id": "$evento-canario:matrix-dev.local",
                "origin_server_ts": 1_795_300_000_000i64,
                "content": { "msgtype": "m.text", "body": TEXTO_ATENDENTE }
            }]),
        )
        .await,
    );
    respostas.push(
        pedir_com_cabecalho(
            &base,
            MetodoHttp::PUT,
            "/_matrix/app/v1/transactions/txn-sem-token",
            Some(("authorization", "Bearer token-errado-de-proposito")),
            Some(json!({"events": []})),
        )
        .await,
    );
    respostas.push(
        pedir_com_cabecalho(
            &base,
            MetodoHttp::GET,
            &format!("/_matrix/app/v1/users/@{PREFIXO}_wa_{TELEFONE}:{SERVIDOR}"),
            Some(("authorization", &format!("Bearer {HS_TOKEN}"))),
            None,
        )
        .await,
    );
    respostas.push(
        pedir_com_cabecalho(
            &base,
            MetodoHttp::POST,
            &format!("/webhook/evolution/{}/{INSTANCIA}", cenario.org_a),
            Some((CABECALHO_SEGREDO, "segredo-errado")),
            Some(json!({"event": "messages.upsert"})),
        )
        .await,
    );
    respostas.push(
        webhook(
            &base,
            cenario.org_a,
            json!({
                "event": "messages.upsert",
                "instance": INSTANCIA,
                "apikey": CHAVE_EVO,
                "data": {
                    "key": { "id": "MSG-MX28-CANARIO", "remoteJid": JID, "fromMe": false },
                    "pushName": "Cliente Canário",
                    "messageType": "conversation",
                    "message": { "conversation": "oi" },
                    "messageTimestamp": 1_795_300_000i64
                }
            }),
        )
        .await,
    );

    // Alguns caminhos precisam ter respondido, senão a varredura não vale.
    assert_eq!(respostas[0].status, StatusCode::OK);
    assert_eq!(respostas[2].status, StatusCode::FORBIDDEN);
    assert_eq!(respostas[3].status, StatusCode::UNAUTHORIZED);
    assert_eq!(respostas[4].status, StatusCode::OK);
    assert_eq!(respostas[5].status, StatusCode::FORBIDDEN);
    assert_eq!(respostas[6].status, StatusCode::OK);
    assert_eq!(respostas[7].status, StatusCode::UNAUTHORIZED);

    let proibidos = [
        ("as_token", AS_TOKEN),
        ("hs_token", HS_TOKEN),
        ("token global", TOKEN_GLOBAL),
    ];

    // 1. Respostas HTTP (corpo inteiro, inclusive as de erro).
    let respostas_juntas: String = respostas
        .iter()
        .map(|resposta| resposta.texto.clone())
        .collect::<Vec<_>>()
        .join("\n");
    for (rotulo, valor) in proibidos {
        assert!(
            !respostas_juntas.contains(valor),
            "o {rotulo} apareceu numa resposta HTTP"
        );
    }

    // 2. Nenhuma URL que chegou ao Synapse carregou token: ele vai só no cabeçalho.
    let urls = cenario
        .synapse
        .estado
        .lock()
        .unwrap()
        .urls_com_token
        .clone();
    assert!(
        urls.is_empty(),
        "token na URL de uma chamada ao Synapse: {urls:?}"
    );
    assert!(
        cenario.synapse.estado.lock().unwrap().versoes > 0
            || !cenario.synapse.estado.lock().unwrap().salas.is_empty(),
        "o Synapse falso recebeu chamadas (senão a varredura de URL não prova nada)"
    );

    // A fila de saída tem o item da resposta do atendente: a varredura abaixo não é vazia.
    assert!(
        cenario.pendentes(cenario.org_a) >= 1,
        "a resposta do atendente entrou na fila de A"
    );

    // 3. Itens da fila de saída (texto, arquivo e qualquer campo textual).
    for org in [cenario.org_a, cenario.org_b] {
        for item in cenario
            .store
            .caixa_saida()
            .listar_pendentes(org, 50)
            .expect("lista a fila")
        {
            let texto = format!(
                "{} {} {}",
                item.tipo,
                item.conteudo_json,
                item.ultimo_erro.as_deref().unwrap_or("")
            );
            for (rotulo, valor) in proibidos {
                assert!(
                    !texto.contains(valor),
                    "o {rotulo} apareceu num item da fila de saída"
                );
            }
        }
    }

    // 4. Log capturado do processo inteiro (o módulo logou transação, sala, erro e aviso).
    let logs = logs_como_texto();
    for (rotulo, valor) in proibidos {
        assert!(
            !logs.contains(valor),
            "o {rotulo} apareceu no log do módulo"
        );
    }
    assert!(
        logs.contains("matrix_as") || logs.contains("matrix_resposta"),
        "o log tem linha do módulo (senão a varredura não prova nada)"
    );

    // 5. Arquivos criados na pasta do teste (banco, WAL, blobs e o que mais aparecer).
    let arquivos = arquivos_da_pasta(&cenario.pasta);
    assert!(
        !arquivos.is_empty(),
        "a varredura de arquivo precisa ter arquivos para ler"
    );
    for (caminho, bytes) in arquivos {
        let texto = String::from_utf8_lossy(&bytes);
        for (rotulo, valor) in proibidos {
            assert!(
                !texto.contains(valor),
                "o {rotulo} apareceu no arquivo {caminho}"
            );
        }
    }

    let _ = desligar.send(());
}
