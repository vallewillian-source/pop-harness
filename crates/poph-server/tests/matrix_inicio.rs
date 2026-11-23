//! Testes da ligação do módulo Matrix no servidor (MX-20).
//!
//! O que se prova aqui, com o banco de verdade e um Synapse **falso** que responde como o Synapse:
//! - módulo desligado → nenhuma tarefa do módulo e as rotas do Application Service em `404` (A8);
//! - Synapse fora na subida → o servidor sobe, atende e o módulo fica degradado (sem derrubar
//!   nada), e a sonda marca "conectado" quando ele responde;
//! - desligar uma org em `matrix_orgs` para o espelho dela até o ciclo seguinte, sem reiniciar;
//! - término ordenado: o que estava pendente não se perde e sai na subida seguinte;
//! - a sala criada pelo módulo usa o nome do cliente (`conversations.client_name`, MX-32).
//!
//! O Synapse falso é o mesmo tipo usado nos testes da MX-11: guarda usuários, salas, aliases,
//! membros, eventos de estado e mensagens, e deduplica envio por `txnId` como o Synapse faz.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::Router;
use axum::extract::State;
use axum::http::header::AUTHORIZATION;
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use poph_matrix::config::{ModuloMatrix, SegredosMatrix};
use poph_matrix::salas::TIPO_METADATA;
use poph_server::matrix_inicio::AjustesDeInicio;
use poph_server::{Estado, matrix_inicio};
use poph_store::{
    Direcao, EstadoSaida, FiltroConversas, NovaConversa, NovaMensagem, NovaOrg, NovoAgente,
    NovoDepartamento, Origem, Pagina, Papel, StatusConversa, Store,
};
use poph_types::{
    AgentId, ConversationId, DepartmentId, HarnessConfig, MatrixConfig, OrgId, Segredo,
};
use serde_json::{Value, json};

/// Token que o Harness apresenta ao Synapse.
const AS_TOKEN: &str = "CANARIO-AS-mx20-nao-pode-vazar";
/// Token que o Synapse apresenta ao Harness.
const HS_TOKEN: &str = "CANARIO-HS-mx20-nao-pode-vazar";
/// Domínio dos ids no teste.
const SERVIDOR: &str = "matrix-dev.local";
/// Prefixo dos usuários virtuais.
const PREFIXO: &str = "ph";

// ───────────────────────────── Synapse falso ─────────────────────────────

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
    /// Escritas de `m.space.child` com conteúdo (o vínculo da sala com o espaço).
    filhos_escritos: u32,
    salas: BTreeMap<String, SalaFalsa>,
    aliases: BTreeMap<String, String>,
    proxima_sala: u32,
    /// Recusa todo envio com 503 (Synapse "fora" para o envio, mas respondendo à sonda).
    falhar_envio: bool,
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
            if tipo == "m.space.child" && !corpo.as_object().is_none_or(|corpo| corpo.is_empty()) {
                guarda.filhos_escritos += 1;
            }
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
        // Estado inteiro da sala (`GET /rooms/{id}/state`): a leitura que acha os espaços de onde a
        // sala é filha quando o registro da conversa está vazio (MX-51).
        ("GET", [salas, sala, estado]) if salas == "rooms" && estado == "state" => {
            match guarda.salas.get(sala) {
                Some(alvo) => {
                    let eventos: Vec<Value> = alvo
                    .estado
                    .iter()
                    .map(|((tipo, chave), corpo)| json!({ "type": tipo, "state_key": chave, "content": corpo }))
                    .collect();
                    responder(200, Value::Array(eventos))
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
            if guarda.falhar_envio {
                return responder(
                    503,
                    json!({ "errcode": "M_UNKNOWN", "error": "indisponível" }),
                );
            }
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
    desligar: Option<tokio::sync::oneshot::Sender<()>>,
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
        let (desligar, sinal) = tokio::sync::oneshot::channel();
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

    fn mensagens(&self, sala: &str) -> Vec<String> {
        self.estado
            .lock()
            .expect("trava")
            .salas
            .get(sala)
            .map(|sala| {
                sala.mensagens
                    .iter()
                    .map(|(txn, _, _)| txn.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Corpo de cada mensagem escrita nas salas, para conferir o que o Harness publicou.
    fn corpos_das_mensagens(&self) -> Vec<Value> {
        self.estado
            .lock()
            .expect("trava")
            .salas
            .values()
            .flat_map(|sala| sala.mensagens.iter().map(|(_, _, corpo)| corpo.clone()))
            .collect()
    }

    fn total_de_mensagens(&self) -> usize {
        self.estado
            .lock()
            .expect("trava")
            .salas
            .values()
            .map(|sala| sala.mensagens.len())
            .sum()
    }

    fn versoes(&self) -> u32 {
        self.estado.lock().expect("trava").versoes
    }

    fn filhos_escritos(&self) -> u32 {
        self.estado.lock().expect("trava").filhos_escritos
    }

    fn falhar_envio(&self, falhar: bool) {
        self.estado.lock().expect("trava").falhar_envio = falhar;
    }
}

impl Drop for Synapse {
    fn drop(&mut self) {
        if let Some(desligar) = self.desligar.take() {
            let _ = desligar.send(());
        }
    }
}

// ───────────────────────────── cenário de banco ─────────────────────────────

/// Banco de verdade numa pasta temporária, com a configuração do servidor pronta.
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
            "poph-matrix-inicio-{rotulo}-{}-{carimbo}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&pasta);
        fs::create_dir_all(pasta.join("dados")).expect("pasta de dados");

        let mut config = HarnessConfig::default();
        config.dados.diretorio = pasta.join("dados");
        config.matrix = Some(MatrixConfig {
            habilitado: true,
            homeserver_url,
            server_name: SERVIDOR.to_owned(),
            prefixo: PREFIXO.to_owned(),
            concorrencia: 4,
            intervalo_ms: 50,
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

    /// O módulo já preparado, com os tokens de teste (sem tocar no ambiente do processo).
    fn modulo(&self) -> ModuloMatrix {
        ModuloMatrix {
            config: self.config.matrix.clone().expect("seção [matrix]"),
            segredos: SegredosMatrix {
                as_token: Segredo::new(AS_TOKEN.to_owned()),
                hs_token: Segredo::new(HS_TOKEN.to_owned()),
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

    /// Agente já com departamento padrão (é ele que manda a sala para o espaço).
    fn agente_com_departamento(&self, org: OrgId, departamento: DepartmentId) -> AgentId {
        self.store
            .agentes()
            .criar(
                org,
                NovoAgente {
                    nome: "Bia".to_owned(),
                    slug: "bia".to_owned(),
                    departamento_padrao: Some(departamento),
                    ..NovoAgente::default()
                },
            )
            .expect("cria o agente")
            .id
    }

    fn conversa(&self, org: OrgId, agente: AgentId, peer: &str) -> ConversationId {
        self.store
            .conversas()
            .criar(
                org,
                NovaConversa {
                    agent_id: agente,
                    integration_id: None,
                    canal: "whatsapp".to_owned(),
                    peer_externo: Some(peer.to_owned()),
                    departamento_alvo: None,
                },
            )
            .expect("cria a conversa")
            .id
    }

    fn mensagem(&self, org: OrgId, conversa: ConversationId, texto: &str) -> String {
        self.relogio.fetch_add(10, Ordering::SeqCst);
        self.store
            .mensagens()
            .gravar(
                org,
                NovaMensagem {
                    conversation_id: conversa,
                    direcao: Direcao::Saida,
                    papel: Papel::Assistente,
                    origem: Origem::Agente,
                    autor_nome: None,
                    texto: texto.to_owned(),
                    anexos_json: "[]".to_owned(),
                    id_externo: None,
                    turn_id: None,
                },
            )
            .expect("grava a mensagem")
            .id
            .to_string()
    }
}

impl Drop for Cenario {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.pasta);
    }
}

/// Espera até a condição valer. Falha o teste (com o motivo) quando o prazo acaba.
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

/// Sobe o router do servidor numa porta efêmera (sem a raiz de composição, para o teste controlar
/// quando o módulo é ligado).
async fn servir(estado: Estado) -> (String, tokio::sync::oneshot::Sender<()>) {
    let app = poph_server::router(estado);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("porta efêmera");
    let endereco = format!("http://{}", listener.local_addr().expect("endereço"));
    let (desligar, sinal) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = sinal.await;
            })
            .await;
    });
    (endereco, desligar)
}

/// Cliente HTTP dos testes.
fn cliente() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .expect("cliente HTTP")
}

/// `GET` simples, devolvendo `(status, corpo)`.
async fn buscar(url: &str, token: Option<&str>) -> (StatusCode, String) {
    let mut pedido = cliente().get(url);
    if let Some(token) = token {
        pedido = pedido.bearer_auth(token);
    }
    let resposta = pedido.send().await.expect("pedido HTTP");
    let status = resposta.status();
    (status, resposta.text().await.unwrap_or_default())
}

// ───────────────────────────── módulo desligado (A8) ─────────────────────────────

#[tokio::test]
async fn modulo_desligado_nao_sobe_tarefas_nem_publica_metricas() {
    let cenario = Cenario::novo("desligado", "http://127.0.0.1:9".to_owned());
    for secao in [None, Some(MatrixConfig::default())] {
        let mut config = cenario.config.clone();
        config.matrix = secao;
        let mut estado = Estado::novo(Arc::clone(&cenario.store), config).expect("estado");

        let rodando = matrix_inicio::iniciar(&mut estado);
        assert!(
            rodando.is_none(),
            "módulo desligado não pode render tarefa do módulo"
        );
        assert!(
            estado.matrix_as.is_none(),
            "sem receptor injetado, as rotas do Synapse não existem"
        );
        assert!(
            estado.matrix_conectado().is_none(),
            "módulo desligado não tem estado de conexão para a API ler"
        );
    }
}

#[tokio::test]
async fn modulo_desligado_responde_404_no_application_service() {
    let cenario = Cenario::novo("desligado-http", "http://127.0.0.1:9".to_owned());
    let mut config = cenario.config.clone();
    config.matrix = None;
    let estado = Estado::novo(Arc::clone(&cenario.store), config).expect("estado");
    let (base, desligar) = servir(estado).await;

    let alvo = format!("{base}/_matrix/app/v1/users/@ph_wa_5511999999999:{SERVIDOR}");
    let (status, _) = buscar(&alvo, Some(HS_TOKEN)).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "módulo desligado: nenhuma rota do Application Service"
    );

    // E nenhuma métrica do módulo no `/metrics`.
    let (status, corpo) = buscar(&format!("{base}/metrics"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        !corpo.contains("poph_matrix_"),
        "módulo desligado não pode publicar métrica do módulo"
    );
    let _ = desligar.send(());
}

// ───────────────────────────── Synapse fora ─────────────────────────────

#[tokio::test]
async fn synapse_fora_na_subida_nao_impede_o_serve() {
    // Porta fechada: qualquer chamada falha na conexão.
    let cenario = Cenario::novo("fora", "http://127.0.0.1:9".to_owned());
    let mut estado = cenario.estado();
    let rodando = matrix_inicio::iniciar_com(&mut estado, cenario.modulo())
        .expect("módulo ligado sobe mesmo com o Synapse fora");
    assert!(estado.matrix_as.is_some(), "receptor injetado");

    let (base, desligar) = servir(estado.clone()).await;
    let (status, _) = buscar(&format!("{base}/healthz"), None).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "o servidor atende com o Synapse fora"
    );

    // As rotas do Synapse também respondem: elas não dependem da conexão com o Synapse.
    let alvo = format!("{base}/_matrix/app/v1/users/@ph_wa_5511999999999:{SERVIDOR}");
    let (status, _) = buscar(&alvo, Some(HS_TOKEN)).await;
    assert_eq!(status, StatusCode::OK);

    assert!(
        !rodando.conectado(),
        "sem Synapse do outro lado, o módulo está degradado"
    );
    assert_eq!(
        estado.matrix_conectado(),
        Some(false),
        "o estado do módulo diz 'degradado' para a API administrativa"
    );
    rodando
        .com_prazo_de_parada(Duration::from_secs(2))
        .parar()
        .await;
    let _ = desligar.send(());
}

#[tokio::test]
async fn sonda_marca_conectado_e_volta_a_degradar() {
    let synapse = Synapse::subir().await;
    let cenario = Cenario::novo("sonda", synapse.endereco.clone());
    let mut estado = cenario.estado();
    // Sonda rápida: em produção ela é de 30 s, aqui de 50 ms.
    let ajustes = AjustesDeInicio {
        periodo_da_sonda: Duration::from_millis(50),
        teto_da_sonda: Duration::from_millis(200),
        prazo_de_parada: Duration::from_secs(2),
    };
    let rodando = matrix_inicio::iniciar_com_ajustes(&mut estado, cenario.modulo(), ajustes)
        .expect("módulo ligado");

    esperar_ate("a sonda marcar conectado", || rodando.conectado()).await;
    assert!(synapse.versoes() >= 1, "a sonda fala com o Synapse");
    assert_eq!(estado.matrix_conectado(), Some(true));

    drop(synapse);
    esperar_ate("a sonda marcar degradado depois da queda", || {
        !rodando.conectado()
    })
    .await;

    rodando
        .com_prazo_de_parada(Duration::from_secs(2))
        .parar()
        .await;
}

// ───────────────────────────── espelho: sala, nome e org ─────────────────────────────

/// Cena comum: org ligada, agente, conversa com nome de cliente conhecido e uma mensagem.
struct Cena {
    cenario: Cenario,
    org: OrgId,
    conversa: ConversationId,
    primeiro: String,
}

fn cena(rotulo: &str, endereco: String) -> Cena {
    let cenario = Cenario::novo(rotulo, endereco);
    let org = cenario.org("a");
    let agente = cenario.agente(org);
    let conversa = cenario.conversa(org, agente, "5511999999999");
    cenario
        .store
        .conversas()
        .definir_nome_cliente(org, conversa, "Ana Souza")
        .expect("grava o nome do cliente");
    cenario.store.matrix().ligar(org).expect("liga a org");
    let primeiro = cenario.mensagem(org, conversa, "primeira resposta do agente");
    Cena {
        cenario,
        org,
        conversa,
        primeiro,
    }
}

#[tokio::test]
async fn sala_criada_usa_o_nome_do_cliente_e_espelha_a_mensagem() {
    let synapse = Synapse::subir().await;
    let cena = cena("nome", synapse.endereco.clone());
    let mut estado = cena.cenario.estado();
    let rodando =
        matrix_inicio::iniciar_com(&mut estado, cena.cenario.modulo()).expect("módulo ligado");

    esperar_ate("o Synapse receber a mensagem", || {
        synapse.total_de_mensagens() >= 1
    })
    .await;

    let (sala, falsa) = synapse.salas().pop().expect("a sala foi criada");
    assert_eq!(
        falsa.nome.as_deref(),
        Some("Ana Souza"),
        "a sala nasce com o nome do cliente, não com o telefone"
    );
    assert_eq!(
        falsa.tipo.as_deref(),
        Some(poph_matrix::salas::TIPO_SALA),
        "a sala nasce com o tipo de atendimento que o Element reconhece"
    );
    assert!(
        synapse
            .mensagens(&sala)
            .contains(&format!("poph-msg-{}", cena.primeiro)),
        "a mensagem do Harness chegou à sala"
    );
    let metadata = synapse
        .estado_da_sala(&sala, TIPO_METADATA, "")
        .expect("metadata escrito");
    assert_eq!(metadata["client_name"], json!("Ana Souza"));
    assert_eq!(
        metadata["conversation_id"],
        json!(cena.conversa.to_string())
    );

    let gravada = cena
        .cenario
        .store
        .matrix()
        .sala(cena.org, cena.conversa)
        .expect("lê a sala")
        .expect("linha da sala");
    assert_eq!(
        gravada.estado,
        poph_store::EstadoSala::Pronta,
        "a linha fica pronta depois da criação"
    );
    assert_eq!(gravada.room_id, sala);

    rodando
        .com_prazo_de_parada(Duration::from_secs(2))
        .parar()
        .await;
}

#[tokio::test]
async fn sala_criada_depois_da_passagem_do_estado_ja_entra_no_espaco_do_departamento() {
    let synapse = Synapse::subir().await;
    let cenario = Cenario::novo("espaco", synapse.endereco.clone());
    let org = cenario.org("a");
    let departamento = cenario.departamento(org, "Comercial");
    let agente = cenario.agente_com_departamento(org, departamento);
    let conversa = cenario.conversa(org, agente, "5511988887777");
    cenario.store.matrix().ligar(org).expect("liga a org");

    let mut estado = cenario.estado();
    let rodando = matrix_inicio::iniciar_com(&mut estado, cenario.modulo()).expect("módulo ligado");

    // O laço de estado vê a conversa **antes** de a sala existir (não há mensagem ainda): é o caso
    // em que antes a sala nascia fora do espaço e só entrava na próxima alteração da conversa.
    esperar_ate("o laço de estado passar pela conversa sem sala", || {
        let cursores = cenario.store.matrix().ler_cursores(org).expect("cursores");
        cursores.conversas.id == conversa.to_string()
    })
    .await;
    assert_eq!(
        cenario
            .store
            .matrix()
            .sala(org, conversa)
            .expect("lê a sala"),
        None,
        "o laço de estado não cria sala: quem cria é o espelho"
    );

    // A mensagem chega depois: o espelho cria a sala e ela já entra no espaço, sem que a conversa
    // mude de novo.
    let primeira = cenario.mensagem(org, conversa, "primeira resposta do agente");
    esperar_ate("o Synapse receber a mensagem", || {
        synapse.total_de_mensagens() >= 1
    })
    .await;

    // Duas salas no Synapse: o espaço do departamento e a sala da conversa. A da conversa é a que
    // recebeu a mensagem espelhada.
    let (sala, _) = synapse
        .salas()
        .into_iter()
        .find(|(id, _)| !synapse.mensagens(id).is_empty())
        .expect("a sala da conversa foi criada");
    assert!(
        synapse
            .mensagens(&sala)
            .contains(&format!("poph-msg-{primeira}"))
    );

    // O espaço do departamento existe, tem a sala como filha e o vínculo está gravado no banco.
    let espaco = cenario
        .store
        .matrix()
        .espaco(org, departamento)
        .expect("lê o espaço")
        .expect("o espaço do departamento foi criado");
    let filho = synapse
        .estado_da_sala(&espaco.room_id, "m.space.child", &sala)
        .expect("m.space.child no espaço do departamento");
    assert!(
        filho["via"].is_array(),
        "o filho leva `via`, que é o que o Element usa para desenhar a sala sob o espaço: {filho}"
    );
    let gravada = cenario
        .store
        .matrix()
        .sala(org, conversa)
        .expect("lê a sala")
        .expect("linha da sala");
    assert_eq!(
        gravada.espaco.as_deref(),
        Some(espaco.room_id.as_str()),
        "o vínculo da sala com o espaço fica gravado"
    );
    let escritas = synapse.filhos_escritos();
    assert_eq!(escritas, 1, "o `m.space.child` foi escrito uma vez");

    // Repetir não escreve de novo: nova mensagem na mesma conversa não reescreve o `m.space.child`.
    let segunda = cenario.mensagem(org, conversa, "segunda resposta do agente");
    esperar_ate("a segunda mensagem ser espelhada", || {
        synapse
            .mensagens(&sala)
            .contains(&format!("poph-msg-{segunda}"))
    })
    .await;
    assert_eq!(
        synapse.filhos_escritos(),
        escritas,
        "sala já posicionada não escreve `m.space.child` de novo"
    );

    rodando
        .com_prazo_de_parada(Duration::from_secs(2))
        .parar()
        .await;
}

#[tokio::test]
async fn desligar_org_para_o_espelho_dela_sem_reiniciar() {
    let synapse = Synapse::subir().await;
    let cena = cena("org", synapse.endereco.clone());
    let mut estado = cena.cenario.estado();
    let rodando =
        matrix_inicio::iniciar_com(&mut estado, cena.cenario.modulo()).expect("módulo ligado");

    esperar_ate("a primeira mensagem ser espelhada", || {
        synapse.total_de_mensagens() >= 1
    })
    .await;
    let antes = synapse.total_de_mensagens();

    // Desligar a org: o laço relê `matrix_orgs` a cada ciclo, sem reiniciar o servidor.
    cena.cenario
        .store
        .matrix()
        .desligar(cena.org)
        .expect("desliga");
    cena.cenario
        .mensagem(cena.org, cena.conversa, "mensagem com a org desligada");
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(
        synapse.total_de_mensagens(),
        antes,
        "org desligada não espelha mais nada"
    );

    // Religar volta a espelhar: a org reaparece para o laço sem reiniciar o servidor. O que foi
    // escrito **enquanto** ela estava desligada fica de fora de propósito — religar move o corte do
    // backfill (P8), e só o que vem depois entra. Por isso a mensagem que prova a volta é nova.
    cena.cenario.store.matrix().ligar(cena.org).expect("religa");
    cena.cenario
        .mensagem(cena.org, cena.conversa, "mensagem depois de religar");
    esperar_ate("a mensagem nova sair depois de religar", || {
        synapse.total_de_mensagens() > antes
    })
    .await;

    rodando
        .com_prazo_de_parada(Duration::from_secs(2))
        .parar()
        .await;
}

// ───────────────────────────── término ordenado ─────────────────────────────

#[tokio::test]
async fn termino_ordenado_nao_perde_mensagem_pendente() {
    let synapse = Synapse::subir().await;
    let cena = cena("parada", synapse.endereco.clone());
    // O Synapse aceita tudo, menos o envio: a mensagem fica pendente.
    synapse.falhar_envio(true);

    let mut estado = cena.cenario.estado();
    let rodando =
        matrix_inicio::iniciar_com(&mut estado, cena.cenario.modulo()).expect("módulo ligado");
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        synapse.total_de_mensagens(),
        0,
        "nada entregue enquanto o envio falha"
    );
    let cursor = cena
        .cenario
        .store
        .matrix()
        .ler_cursores(cena.org)
        .expect("cursor")
        .mensagens;
    assert_eq!(cursor, 0, "o cursor não anda sem sucesso");

    rodando
        .com_prazo_de_parada(Duration::from_secs(5))
        .parar()
        .await;

    // Subida seguinte, com o Synapse de volta: a mensagem pendente sai, uma vez só.
    synapse.falhar_envio(false);
    let mut estado = cena.cenario.estado();
    let rodando =
        matrix_inicio::iniciar_com(&mut estado, cena.cenario.modulo()).expect("módulo ligado");
    esperar_ate("a mensagem pendente sair na subida seguinte", || {
        synapse.total_de_mensagens() >= 1
    })
    .await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(
        synapse.total_de_mensagens(),
        1,
        "uma entrega só, sem duplicar"
    );
    rodando
        .com_prazo_de_parada(Duration::from_secs(2))
        .parar()
        .await;
}

// ───────────────────────────── o caminho do atendente está ligado ─────────────────────────────

#[tokio::test]
async fn resposta_do_atendente_esta_ligada_no_servidor() {
    let synapse = Synapse::subir().await;
    let cena = cena("resposta", synapse.endereco.clone());
    let mut estado = cena.cenario.estado();
    let rodando =
        matrix_inicio::iniciar_com(&mut estado, cena.cenario.modulo()).expect("módulo ligado");
    esperar_ate("a sala da conversa existir", || !synapse.salas().is_empty()).await;
    let (sala, _) = synapse.salas().pop().expect("sala criada");

    // O Synapse entrega a mensagem que o atendente escreveu no Element.
    let (base, desligar) = servir(estado).await;
    let corpo = json!({
        "events": [{
            "type": "m.room.message",
            "room_id": sala,
            "sender": "@ana:matrix-dev.local",
            "event_id": "$humano-1",
            "content": { "msgtype": "m.text", "body": "oi, aqui é a Ana do atendimento" }
        }]
    })
    .to_string();
    let resposta = cliente()
        .put(format!("{base}/_matrix/app/v1/transactions/9"))
        .bearer_auth(HS_TOKEN)
        .header("content-type", "application/json")
        .body(corpo)
        .send()
        .await
        .expect("pedido HTTP");
    assert_eq!(resposta.status(), StatusCode::OK);

    // A entrega é a de verdade (MX-18): a resposta entrou na fila e a conversa foi pausada.
    esperar_ate("a resposta entrar na fila de saída", || {
        cena.cenario
            .store
            .caixa_saida()
            .contar_por_estado(cena.org, EstadoSaida::Pendente)
            .map(|total| total >= 1)
            .unwrap_or(false)
    })
    .await;
    let conversa = cena
        .cenario
        .store
        .conversas()
        .obter(cena.org, cena.conversa)
        .expect("lê a conversa");
    assert_eq!(
        conversa.status,
        StatusConversa::Transferida,
        "a resposta do atendente pausa o agente"
    );

    let _ = desligar.send(());
    rodando
        .com_prazo_de_parada(Duration::from_secs(2))
        .parar()
        .await;
}

#[tokio::test]
async fn arquivo_do_atendente_esta_ligado_no_servidor() {
    // MX-44: o receptor montado pelo `iniciar_com` conhece a pasta de blobs do servidor. Sem essa
    // ligação, todo arquivo do atendente cairia no aviso "não consegui guardar o arquivo" — aqui a
    // mesma entrega responde o motivo certo para o evento que não trouxe `url`.
    let synapse = Synapse::subir().await;
    let cena = cena("arquivo", synapse.endereco.clone());
    let mut estado = cena.cenario.estado();
    let rodando =
        matrix_inicio::iniciar_com(&mut estado, cena.cenario.modulo()).expect("módulo ligado");
    esperar_ate("a sala da conversa existir", || !synapse.salas().is_empty()).await;
    let (sala, _) = synapse.salas().pop().expect("sala criada");

    let (base, desligar) = servir(estado).await;
    let corpo = json!({
        "events": [{
            "type": "m.room.message",
            "room_id": sala,
            "sender": "@ana:matrix-dev.local",
            "event_id": "$midia-1",
            "content": { "msgtype": "m.image", "body": "foto.png" }
        }]
    })
    .to_string();
    let resposta = cliente()
        .put(format!("{base}/_matrix/app/v1/transactions/10"))
        .bearer_auth(HS_TOKEN)
        .header("content-type", "application/json")
        .body(corpo)
        .send()
        .await
        .expect("pedido HTTP");
    assert_eq!(resposta.status(), StatusCode::OK);

    esperar_ate("o aviso da mídia aparecer na sala", || {
        synapse
            .corpos_das_mensagens()
            .iter()
            .any(|corpo| corpo["msgtype"] == json!("m.notice"))
    })
    .await;
    let aviso = synapse
        .corpos_das_mensagens()
        .into_iter()
        .find(|corpo| corpo["msgtype"] == json!("m.notice"))
        .expect("o aviso está na sala");
    assert_eq!(
        aviso["body"],
        json!("Não consegui enviar o arquivo ao cliente: o evento não trouxe o arquivo"),
        "a pasta de blobs do servidor está ligada no receptor"
    );

    let _ = desligar.send(());
    rodando
        .com_prazo_de_parada(Duration::from_secs(2))
        .parar()
        .await;
}

// ───────────── Identificador do cliente: metadata e listagem usam a mesma regra (RP-02) ─────────────

/// A sala criada pelo espelho e o metadata dela, quando já estão no Synapse falso.
fn metadata_da_sala_criada(synapse: &Synapse) -> Option<(String, Value)> {
    let (sala, _) = synapse.salas().into_iter().next()?;
    let metadata = synapse.estado_da_sala(&sala, TIPO_METADATA, "")?;
    Some((sala, metadata))
}

#[tokio::test]
async fn metadata_leva_o_external_peer_sem_espacos_na_criacao_e_no_fechamento() {
    let synapse = Synapse::subir().await;
    let cenario = Cenario::novo("peer-espacos", synapse.endereco.clone());
    let org = cenario.org("a");
    let agente = cenario.agente(org);
    // O espaço nas pontas fica gravado no banco de propósito: o identificador tem de sair sem ele.
    let conversa = cenario.conversa(org, agente, "  5511999999999 ");
    cenario.store.matrix().ligar(org).expect("liga a org");
    cenario.mensagem(org, conversa, "primeira resposta do agente");

    let mut estado = cenario.estado();
    let rodando = matrix_inicio::iniciar_com(&mut estado, cenario.modulo()).expect("módulo ligado");

    esperar_ate("a sala criada com metadata", || {
        metadata_da_sala_criada(&synapse).is_some()
    })
    .await;
    let (sala, criado) = metadata_da_sala_criada(&synapse).expect("sala e metadata");
    assert_eq!(
        criado["client_identifier"],
        json!("5511999999999"),
        "na criação, o identificador sai sem espaços nas pontas"
    );

    // Fechar a conversa faz o laço de estado reescrever o metadata: a regra tem de ser a mesma.
    let atual = cenario
        .store
        .conversas()
        .obter(org, conversa)
        .expect("conversa");
    cenario
        .store
        .conversas()
        .fechar_auditado(org, conversa, atual.versao, "teste".to_owned())
        .expect("fecha a conversa");
    esperar_ate("o metadata reescrito no fechamento", || {
        synapse
            .estado_da_sala(&sala, TIPO_METADATA, "")
            .is_some_and(|metadata| metadata["status"] == json!("closed"))
    })
    .await;
    let fechado = synapse
        .estado_da_sala(&sala, TIPO_METADATA, "")
        .expect("metadata do fechamento");
    assert_eq!(
        fechado["client_identifier"],
        json!("5511999999999"),
        "no fechamento, o identificador continua o mesmo"
    );

    rodando
        .com_prazo_de_parada(Duration::from_secs(2))
        .parar()
        .await;
}

#[tokio::test]
async fn conversa_sem_peer_usa_o_id_da_conversa_no_metadata() {
    let synapse = Synapse::subir().await;
    let cenario = Cenario::novo("sem-peer", synapse.endereco.clone());
    let org = cenario.org("a");
    let agente = cenario.agente(org);
    let conversa = cenario
        .store
        .conversas()
        .criar(
            org,
            NovaConversa {
                agent_id: agente,
                integration_id: None,
                canal: "whatsapp".to_owned(),
                peer_externo: None,
                departamento_alvo: None,
            },
        )
        .expect("cria a conversa sem cliente")
        .id;
    cenario.store.matrix().ligar(org).expect("liga a org");
    cenario.mensagem(org, conversa, "primeira resposta do agente");

    let mut estado = cenario.estado();
    let rodando = matrix_inicio::iniciar_com(&mut estado, cenario.modulo()).expect("módulo ligado");

    esperar_ate("a sala criada com metadata", || {
        metadata_da_sala_criada(&synapse).is_some()
    })
    .await;
    let (_, criado) = metadata_da_sala_criada(&synapse).expect("sala e metadata");
    assert_eq!(
        criado["client_identifier"],
        json!(conversa.to_string()),
        "sem external_peer, o identificador é o id da conversa (a listagem usa o mesmo valor)"
    );

    rodando
        .com_prazo_de_parada(Duration::from_secs(2))
        .parar()
        .await;
}

#[tokio::test]
async fn listagem_por_cliente_acha_so_a_conversa_do_identificador_do_metadata() {
    let synapse = Synapse::subir().await;
    let cenario = Cenario::novo("listagem", synapse.endereco.clone());
    let org_a = cenario.org("a");
    let agente_a = cenario.agente(org_a);
    let conversa_a = cenario.conversa(org_a, agente_a, "5511888888888");
    // Outra org com o mesmo telefone: a listagem não pode trazê-la.
    let org_b = cenario.org("b");
    let agente_b = cenario.agente(org_b);
    let conversa_b = cenario.conversa(org_b, agente_b, "5511888888888");
    cenario.store.matrix().ligar(org_a).expect("liga a org");
    cenario.mensagem(org_a, conversa_a, "primeira resposta do agente");

    let mut estado = cenario.estado();
    let rodando = matrix_inicio::iniciar_com(&mut estado, cenario.modulo()).expect("módulo ligado");

    esperar_ate("a sala criada com metadata", || {
        metadata_da_sala_criada(&synapse).is_some()
    })
    .await;
    let (_, criado) = metadata_da_sala_criada(&synapse).expect("sala e metadata");
    let identificador = criado["client_identifier"]
        .as_str()
        .expect("identificador em texto")
        .to_owned();
    assert_eq!(identificador, "5511888888888");

    // O que o Element pede ao Harness para achar o histórico: a lista filtrada pelo mesmo valor.
    let filtro = FiltroConversas {
        status: None,
        external_peer: Some(identificador),
    };
    let da_a: Vec<String> = cenario
        .store
        .conversas()
        .listar_filtrado(org_a, &filtro, Pagina::default())
        .expect("lista a org A")
        .iter()
        .map(|conversa| conversa.id.to_string())
        .collect();
    assert_eq!(
        da_a,
        vec![conversa_a.to_string()],
        "a listagem da org A acha exatamente a conversa da sala"
    );
    let da_b: Vec<String> = cenario
        .store
        .conversas()
        .listar_filtrado(org_b, &filtro, Pagina::default())
        .expect("lista a org B")
        .iter()
        .map(|conversa| conversa.id.to_string())
        .collect();
    assert_eq!(
        da_b,
        vec![conversa_b.to_string()],
        "a listagem da org B acha só a conversa dela, com o mesmo telefone"
    );

    rodando
        .com_prazo_de_parada(Duration::from_secs(2))
        .parar()
        .await;
}
