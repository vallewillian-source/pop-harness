//! Testes dos espaços por departamento contra um Synapse **falso** e uma porta em memória (MX-12).
//!
//! O Synapse falso é **com estado** e, diferente do da MX-11, respeita **poder**: escrever evento de
//! estado exige ser membro da sala e ter o poder que o `m.room.power_levels` da sala manda
//! (`state_default` = 50 no padrão do Matrix). É isso que deixa provar, do lado do servidor, o
//! caminho de "ligar a um espaço existente" — com e sem permissão — e o `M_ROOM_IN_USE` de duas
//! tentativas criando o mesmo espaço.
//!
//! A porta em memória imita `store.matrix()` (`garantir_espaco` idempotente por departamento,
//! `definir_espaco_da_sala`) e devolve o que o laço precisa para saber onde a sala está.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::extract::State;
use axum::http::header::AUTHORIZATION;
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use poph_matrix::cliente::{ClienteMatrix, ConfigCliente, CriarSala};
use poph_matrix::espacos::{
    DepartamentoEspaco, ErroEspaco, EspacoGravado, Espacos, LimitesConvite, OrigemEspaco,
    PortaEspacos, Posicao, SalaParaEspaco, TIPO_DEPARTAMENTO, TIPO_FILHO, TIPO_PAI,
    TIPO_REGRAS_ENTRADA, alias_do_departamento, resolver_departamento,
};
use poph_matrix::salas::ErroPorta;
use poph_matrix::usuarios::{Fantasma, IdsFantasmas};
use poph_types::{AgentId, ConversationId, DepartmentId, MatrixConfig, OrgId, Segredo};
use serde_json::{Value, json};

/// Valor-canário do token do Application Service.
const CANARIO: &str = "CANARIO-MX12-nao-pode-vazar";
/// Domínio dos ids no teste.
const SERVIDOR: &str = "matrix-dev.local";
/// Pessoa de verdade que o operador põe no espaço (como um atendente no Element).
const ATENDENTE: &str = "@ana:matrix-dev.local";

// ───────────────────────────── Synapse falso com poder ─────────────────────────────

#[derive(Default, Clone)]
struct SalaFalsa {
    tipo: Option<String>,
    nome: Option<String>,
    topico: Option<String>,
    criador: String,
    membros: BTreeSet<String>,
    convidados: BTreeSet<String>,
    estado: BTreeMap<(String, String), Value>,
    poder: Option<Value>,
}

#[derive(Default)]
struct Estado {
    token_esperado: String,
    salas: BTreeMap<String, SalaFalsa>,
    aliases: BTreeMap<String, String>,
    proxima_sala: u32,
    /// Caminho de cada requisição recebida (só o caminho: nada de corpo nem de token).
    requisicoes: Vec<String>,
    criacoes: u32,
    falhar_proxima_criacao: bool,
    /// Quantos convites (os próximos) respondem 503 antes de voltar ao normal.
    falhar_convites: usize,
}

impl Estado {
    /// Quantas requisições recebidas terminam com este sufixo de caminho.
    fn contagem(&self, sufixo: &str) -> usize {
        self.requisicoes
            .iter()
            .filter(|caminho| caminho.ends_with(sufixo))
            .count()
    }

    fn sala_do_alias(&self, alias: &str) -> Option<&String> {
        self.aliases.get(alias)
    }
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

/// `m.room.power_levels` que o Synapse cria junto da sala: os padrões com o
/// `power_level_content_override` da criação por cima.
fn poderes_iniciais(criador: &str, sobreposto: Option<&Value>) -> Value {
    let mut base = json!({
        "users": { criador: 100 },
        "users_default": 0,
        "events_default": 0,
        "state_default": 50,
        "ban": 50,
        "kick": 50,
        "redact": 50,
        "invite": 0,
    });
    if let Some(sobreposto) = sobreposto.filter(|valor| valor.is_object()) {
        for (chave, valor) in sobreposto.as_object().expect("objeto") {
            base[chave.clone()] = valor.clone();
        }
    }
    base
}

/// `m.room.power_levels` efetivo da sala: o evento de estado que o Synapse criou (ou o operador
/// reescreveu), senão os padrões.
fn poderes_da_sala(sala: &SalaFalsa) -> Value {
    match sala
        .estado
        .get(&("m.room.power_levels".to_owned(), String::new()))
    {
        Some(evento) => evento.clone(),
        None => poderes_iniciais(&sala.criador, sala.poder.as_ref()),
    }
}

fn poder_do_usuario(poderes: &Value, usuario: &str) -> i64 {
    poderes
        .get("users")
        .and_then(|usuarios| usuarios.get(usuario))
        .and_then(Value::as_i64)
        .or_else(|| poderes.get("users_default").and_then(Value::as_i64))
        .unwrap_or(0)
}

fn nivel_de_estado(poderes: &Value, tipo: &str) -> i64 {
    poderes
        .get("events")
        .and_then(|eventos| eventos.get(tipo))
        .and_then(Value::as_i64)
        .or_else(|| poderes.get("state_default").and_then(Value::as_i64))
        .unwrap_or(50)
}

async fn atender(
    State(estado): State<Arc<Mutex<Estado>>>,
    metodo: Method,
    uri: Uri,
    cabecalhos: HeaderMap,
    corpo: String,
) -> Response {
    let corpo: Value = serde_json::from_str(&corpo).unwrap_or(Value::Null);
    let mut guarda = estado.lock().expect("trava do Synapse falso");

    guarda.requisicoes.push(uri.path().to_owned());

    let autorizacao = cabecalhos
        .get(AUTHORIZATION)
        .and_then(|valor| valor.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    if autorizacao != format!("Bearer {}", guarda.token_esperado) {
        return responder(
            401,
            json!({ "errcode": "M_UNKNOWN_TOKEN", "error": "Token inválido" }),
        );
    }

    let caminho = uri
        .path()
        .trim_start_matches("/_matrix/client/v3")
        .trim_start_matches("/_matrix/client/v1");
    let partes: Vec<String> = caminho
        .split('/')
        .filter(|pedaco| !pedaco.is_empty())
        .map(decodificar)
        .collect();
    let consulta_bruta = uri.query().unwrap_or_default().to_owned();
    let quem = consulta(&consulta_bruta, "user_id").unwrap_or_default();

    match (metodo.as_str(), partes.as_slice()) {
        ("POST", [criacao]) if criacao == "createRoom" => {
            if guarda.falhar_proxima_criacao {
                guarda.falhar_proxima_criacao = false;
                return responder(
                    503,
                    json!({ "errcode": "M_UNKNOWN", "error": "Synapse fora do ar" }),
                );
            }
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
            let mut membros = BTreeSet::new();
            if !quem.is_empty() {
                membros.insert(quem.clone());
            }
            // O Synapse cria o `m.room.power_levels` junto da sala; o falso faz igual.
            let mut estado = BTreeMap::new();
            estado.insert(
                ("m.room.power_levels".to_owned(), String::new()),
                poderes_iniciais(&quem, corpo.get("power_level_content_override")),
            );
            guarda.salas.insert(
                sala.clone(),
                SalaFalsa {
                    tipo: corpo["creation_content"]["type"]
                        .as_str()
                        .map(str::to_owned),
                    nome: corpo["name"].as_str().map(str::to_owned),
                    topico: corpo["topic"].as_str().map(str::to_owned),
                    criador: quem.clone(),
                    membros,
                    poder: corpo.get("power_level_content_override").cloned(),
                    estado,
                    ..SalaFalsa::default()
                },
            );
            if let Some(alias) = alias {
                guarda.aliases.insert(alias, sala.clone());
            }
            guarda.criacoes += 1;
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
            if guarda.falhar_convites > 0 {
                guarda.falhar_convites -= 1;
                return responder(
                    503,
                    json!({ "errcode": "M_UNKNOWN", "error": "indisponível no momento" }),
                );
            }
            match guarda.salas.get_mut(sala) {
                Some(alvo) => {
                    alvo.convidados
                        .insert(corpo["user_id"].as_str().unwrap_or_default().to_owned());
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
                    alvo.membros.insert(quem.clone());
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
        ("GET", [salas, sala, hierarquia])
            if salas == "rooms" && hierarquia == "hierarchy" && caminho.starts_with("/rooms") =>
        {
            match guarda.salas.get(sala) {
                Some(alvo) => {
                    let filhas: Vec<Value> = alvo
                        .estado
                        .iter()
                        .filter(|((tipo, _), corpo)| {
                            tipo == TIPO_FILHO && corpo.as_object().is_some_and(|m| !m.is_empty())
                        })
                        .map(|((_, filha), _)| json!({ "room_id": filha }))
                        .collect();
                    responder(200, json!({ "rooms": filhas }))
                }
                None => responder(
                    404,
                    json!({ "errcode": "M_NOT_FOUND", "error": "Sala não existe" }),
                ),
            }
        }
        ("GET", [salas, sala, estado]) if salas == "rooms" && estado == "state" => {
            match guarda.salas.get(sala) {
                Some(alvo) if alvo.membros.contains(&quem) => {
                    let eventos: Vec<Value> = alvo
                        .estado
                        .iter()
                        .map(|((tipo, chave), corpo)| {
                            json!({ "type": tipo, "state_key": chave, "content": corpo })
                        })
                        .collect();
                    responder(200, Value::Array(eventos))
                }
                Some(_) => responder(
                    403,
                    json!({ "errcode": "M_FORBIDDEN", "error": "User not in room" }),
                ),
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
                        json!({ "errcode": "M_NOT_FOUND", "error": "Evento de estado ausente" }),
                    ),
                },
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
            let Some(alvo) = guarda.salas.get_mut(sala) else {
                return responder(
                    404,
                    json!({ "errcode": "M_NOT_FOUND", "error": "Sala não existe" }),
                );
            };
            // O Synapse exige que quem escreve seja membro e tenha poder para o evento.
            if !alvo.membros.contains(&quem) {
                return responder(
                    403,
                    json!({ "errcode": "M_FORBIDDEN", "error": "User not in room" }),
                );
            }
            let poderes = poderes_da_sala(alvo);
            let poder = poder_do_usuario(&poderes, &quem);
            let necessario = nivel_de_estado(&poderes, tipo);
            if poder < necessario {
                return responder(
                    403,
                    json!({ "errcode": "M_FORBIDDEN", "error": "You don't have permission" }),
                );
            }
            alvo.estado.insert((tipo.clone(), chave), corpo);
            responder(200, json!({ "event_id": "$estado" }))
        }
        _ => responder(
            404,
            json!({ "errcode": "M_UNRECOGNIZED", "error": "Rota não existe no Synapse falso" }),
        ),
    }
}

// ───────────────────────────── porta em memória ─────────────────────────────

#[derive(Default)]
struct EstadoPorta {
    espacos: BTreeMap<(OrgId, DepartmentId), EspacoGravado>,
    espaco_das_salas: BTreeMap<(OrgId, ConversationId), String>,
}

/// Imita `store.matrix()`: o primeiro espaço gravado ganha e repetir não muda nada.
#[derive(Clone, Default)]
struct PortaFalsa(Arc<Mutex<EstadoPorta>>);

impl PortaFalsa {
    fn espaco_de(&self, org: OrgId, departamento: DepartmentId) -> Option<EspacoGravado> {
        self.0
            .lock()
            .expect("trava")
            .espacos
            .get(&(org, departamento))
            .cloned()
    }

    fn espaco_da_sala(&self, org: OrgId, conversa: ConversationId) -> Option<String> {
        self.0
            .lock()
            .expect("trava")
            .espaco_das_salas
            .get(&(org, conversa))
            .cloned()
    }
}

impl PortaEspacos for PortaFalsa {
    async fn espaco(
        &self,
        org: OrgId,
        departamento: DepartmentId,
    ) -> Result<Option<EspacoGravado>, ErroPorta> {
        Ok(self.espaco_de(org, departamento))
    }

    async fn garantir_espaco(
        &self,
        org: OrgId,
        departamento: DepartmentId,
        room_id: &str,
        alias: &str,
        origem: OrigemEspaco,
    ) -> Result<EspacoGravado, ErroPorta> {
        let mut guarda = self.0.lock().expect("trava");
        let gravado = guarda
            .espacos
            .entry((org, departamento))
            .or_insert_with(|| EspacoGravado {
                room_id: room_id.to_owned(),
                alias: alias.to_owned(),
                origem,
            });
        Ok(gravado.clone())
    }

    async fn espaco_da_sala(
        &self,
        org: OrgId,
        conversa: ConversationId,
    ) -> Result<Option<String>, ErroPorta> {
        Ok(PortaFalsa::espaco_da_sala(self, org, conversa))
    }

    async fn definir_espaco_da_sala(
        &self,
        org: OrgId,
        conversa: ConversationId,
        espaco: Option<&str>,
    ) -> Result<(), ErroPorta> {
        let mut guarda = self.0.lock().expect("trava");
        match espaco {
            Some(espaco) => {
                guarda
                    .espaco_das_salas
                    .insert((org, conversa), espaco.to_owned());
            }
            None => {
                guarda.espaco_das_salas.remove(&(org, conversa));
            }
        }
        Ok(())
    }
}

// ───────────────────────────── cenário ─────────────────────────────

struct Cenario {
    estado: Arc<Mutex<Estado>>,
    porta: PortaFalsa,
    espacos: Espacos<PortaFalsa>,
    endereco: String,
    agente: Fantasma,
    bot: String,
    org_fixa: OrgId,
    conversa_fixa: ConversationId,
}

impl Cenario {
    fn com_estado(&self) -> std::sync::MutexGuard<'_, Estado> {
        self.estado.lock().expect("trava do Synapse falso")
    }

    fn cliente(&self) -> ClienteMatrix {
        ClienteMatrix::novo(
            ConfigCliente::novo(
                self.endereco.clone(),
                SERVIDOR,
                Segredo::new(CANARIO.to_owned()),
            )
            .expect("configuração do cliente"),
        )
        .expect("cliente")
    }

    /// Sala da conversa criada pelo fantasma do agente (como a MX-11 faz).
    async fn sala_da_conversa(&self) -> String {
        self.cliente()
            .criar_sala(
                self.agente.user_id(),
                &CriarSala {
                    name: Some("Maria".to_owned()),
                    ..CriarSala::default()
                },
            )
            .await
            .expect("createRoom da sala")
    }

    /// Espaço criado pelo operador no Element, com o bot convidado e dentro (permissão dada).
    async fn espaco_do_operador(&self, bot_dentro: bool) -> String {
        let cliente = self.cliente();
        let espaco = cliente
            .criar_sala(
                ATENDENTE,
                &CriarSala {
                    name: Some("Espaço do operador".to_owned()),
                    creation_content: Some(json!({ "type": "m.space" })),
                    ..CriarSala::default()
                },
            )
            .await
            .expect("createRoom do espaço do operador");
        if bot_dentro {
            cliente
                .convidar(ATENDENTE, &espaco, &self.bot)
                .await
                .expect("convite do bot");
            cliente
                .entrar(&self.bot, &espaco)
                .await
                .expect("join do bot");
        }
        espaco
    }

    /// O operador dá poder ao bot no espaço (além de convidá-lo) — o que o Element exige para o
    /// módulo poder escrever `m.space.child` lá.
    async fn dar_poder_ao_bot(&self, espaco: &str, poder: i64) {
        self.cliente()
            .enviar_estado(
                ATENDENTE,
                espaco,
                "m.room.power_levels",
                None,
                &json!({
                    "users": { ATENDENTE: 100, self.bot.clone(): poder },
                    "users_default": 0,
                    "state_default": 50,
                }),
            )
            .await
            .expect("power levels do espaço");
    }

    /// Põe a pessoa no espaço (como o operador convidando o atendente).
    async fn pessoa_no_espaco(&self, pessoa: &str, espaco: &str) {
        let cliente = self.cliente();
        cliente
            .convidar(ATENDENTE, espaco, pessoa)
            .await
            .expect("convite da pessoa");
        cliente
            .entrar(pessoa, espaco)
            .await
            .expect("join da pessoa");
    }

    /// Sala da conversa já registrada na porta (como se a MX-11 tivesse gravado a linha).
    fn com_sala(&self, sala: &str) -> SalaParaEspaco {
        SalaParaEspaco {
            org: self.org(),
            conversa: self.conversa(),
            sala: sala.to_owned(),
            agente: self.agente.clone(),
            departamento: Some(self.departamento("Comercial")),
            existente: None,
        }
    }

    fn org(&self) -> OrgId {
        self.org_fixa
    }

    fn conversa(&self) -> ConversationId {
        self.conversa_fixa
    }

    fn departamento(&self, nome: &str) -> DepartamentoEspaco {
        DepartamentoEspaco {
            id: DepartmentId::generate(),
            nome: nome.to_owned(),
        }
    }
}

async fn cenario() -> Cenario {
    let estado = Arc::new(Mutex::new(Estado {
        token_esperado: CANARIO.to_owned(),
        ..Estado::default()
    }));
    let app = Router::new()
        .fallback(atender)
        .with_state(Arc::clone(&estado));
    let ouvinte = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("porta efêmera");
    let endereco = ouvinte.local_addr().expect("endereço local");
    tokio::spawn(async move {
        axum::serve(ouvinte, app)
            .await
            .expect("Synapse falso no ar");
    });

    let config = MatrixConfig {
        habilitado: true,
        homeserver_url: format!("http://{endereco}"),
        server_name: SERVIDOR.to_owned(),
        prefixo: "ph".to_owned(),
        poder_agente: 50,
        ..MatrixConfig::default()
    };
    let cliente = ClienteMatrix::novo(
        ConfigCliente::das_partes(
            &config,
            &poph_matrix::SegredosMatrix {
                as_token: Segredo::new(CANARIO.to_owned()),
                hs_token: Segredo::new(CANARIO.to_owned()),
            },
        )
        .expect("configuração do cliente"),
    )
    .expect("cliente");
    let ids = IdsFantasmas::novo("ph", SERVIDOR).expect("ids");
    let agente = ids
        .agente(&AgentId::generate().to_string())
        .expect("agente");
    let bot = ids.bot().user_id().to_owned();
    let porta = PortaFalsa::default();
    let espacos = Espacos::com(cliente, ids, porta.clone()).com_limites(LimitesConvite::com(
        2,
        2,
        Duration::ZERO,
        4,
    ));

    Cenario {
        estado,
        porta,
        espacos,
        endereco: format!("http://{endereco}"),
        agente,
        bot,
        org_fixa: OrgId::generate(),
        conversa_fixa: ConversationId::generate(),
    }
}

// ───────────────────────────── testes ─────────────────────────────

#[tokio::test]
async fn cria_o_espaco_quando_falta_e_repetir_nao_cria_o_segundo() {
    let cenario = cenario().await;
    let sala = cenario.sala_da_conversa().await;
    let mut dados = cenario.com_sala(&sala);
    let departamento = dados.departamento.clone().expect("departamento");
    let alias = alias_do_departamento(SERVIDOR, departamento.id);

    let primeira = cenario
        .espacos
        .posicionar(&dados)
        .await
        .expect("posicionar");
    let espaco = match primeira {
        Posicao::Posicionada { espaco, movida, .. } => {
            assert!(!movida, "a sala não veio de outro espaço");
            espaco
        }
        outra => panic!("esperava Posicionada, veio {outra:?}"),
    };

    {
        let guarda = cenario.com_estado();
        assert_eq!(guarda.criacoes, 2, "a sala da conversa e o espaço");
        assert_eq!(
            guarda.contagem("/createRoom"),
            2,
            "dois createRoom no total; o espaço vem pelo alias"
        );
        let no_synapse = guarda.salas.get(&espaco).expect("espaço no Synapse");
        assert_eq!(no_synapse.tipo.as_deref(), Some("m.space"));
        assert_eq!(no_synapse.nome.as_deref(), Some("Comercial"));
        assert_eq!(
            no_synapse.topico.as_deref(),
            Some("Departamento · Comercial")
        );
        assert_eq!(
            guarda.sala_do_alias(&alias).map(String::as_str),
            Some(espaco.as_str()),
            "o espaço nasceu com o alias do departamento"
        );
        assert_eq!(
            no_synapse
                .estado
                .get(&(TIPO_DEPARTAMENTO.to_owned(), String::new()))
                .map(|corpo| corpo["department_id"].clone()),
            Some(json!(departamento.id.to_string())),
            "o espaço diz qual departamento ele é"
        );
        let filho = no_synapse
            .estado
            .get(&(TIPO_FILHO.to_owned(), sala.clone()))
            .expect("m.space.child do espaço");
        assert_eq!(filho["via"], json!([SERVIDOR]));

        let da_sala = guarda.salas.get(&sala).expect("sala");
        let pai = da_sala
            .estado
            .get(&(TIPO_PAI.to_owned(), espaco.clone()))
            .expect("m.space.parent da sala");
        assert_eq!(pai["via"], json!([SERVIDOR]));
        assert_eq!(pai["canonical"], json!(true));
        let regras = da_sala
            .estado
            .get(&(TIPO_REGRAS_ENTRADA.to_owned(), String::new()))
            .expect("join_rules");
        assert_eq!(regras["join_rule"], json!("restricted"));
        assert_eq!(regras["allow"][0]["type"], json!("m.room_membership"));
        assert_eq!(regras["allow"][0]["room_id"], json!(espaco));
    }

    // Repetir: nada de espaço novo e nenhuma escrita no Synapse.
    let antes = cenario.com_estado().requisicoes.len();
    let segunda = cenario.espacos.posicionar(&dados).await.expect("repetir");
    assert_eq!(
        segunda,
        Posicao::JaPosicionada {
            espaco: espaco.clone()
        }
    );
    assert_eq!(
        cenario.com_estado().requisicoes.len(),
        antes,
        "sala já posicionada não fala com o Synapse"
    );
    assert_eq!(cenario.com_estado().criacoes, 2, "nenhum segundo espaço");
    assert_eq!(
        cenario
            .porta
            .espaco_de(dados.org, departamento.id)
            .expect("gravado")
            .origem,
        OrigemEspaco::Automatico
    );

    // Mudar o nome do departamento não recria o espaço (a linha gravada manda).
    dados.departamento = Some(DepartamentoEspaco {
        id: departamento.id,
        nome: "Comercial e Vendas".to_owned(),
    });
    let terceira = cenario.espacos.posicionar(&dados).await.expect("renomeado");
    assert_eq!(
        terceira,
        Posicao::JaPosicionada {
            espaco: espaco.clone()
        }
    );
    assert_eq!(cenario.com_estado().criacoes, 2);
}

#[tokio::test]
async fn liga_um_espaco_que_ja_existe_no_element() {
    let cenario = cenario().await;
    let sala = cenario.sala_da_conversa().await;
    let existente = cenario.espaco_do_operador(true).await;
    cenario.dar_poder_ao_bot(&existente, 50).await;
    let mut dados = cenario.com_sala(&sala);
    let departamento = dados.departamento.clone().expect("departamento");
    dados.existente = Some(existente.clone());

    let posicao = cenario.espacos.posicionar(&dados).await.expect("ligar");
    assert!(
        matches!(&posicao, Posicao::Posicionada { espaco, .. } if *espaco == existente),
        "esperava a sala no espaço do operador, veio {posicao:?}"
    );

    {
        let guarda = cenario.com_estado();
        assert_eq!(guarda.criacoes, 2, "só a sala e o espaço do operador");
        assert!(
            guarda
                .salas
                .get(&existente)
                .expect("espaço")
                .estado
                .contains_key(&(TIPO_FILHO.to_owned(), sala.clone())),
            "o filho foi escrito no espaço existente"
        );
        assert!(
            !guarda
                .aliases
                .contains_key(&alias_do_departamento(SERVIDOR, departamento.id)),
            "ligar a um espaço existente não cria alias novo"
        );
    }

    let gravado = cenario
        .porta
        .espaco_de(dados.org, departamento.id)
        .expect("gravado");
    assert_eq!(gravado.room_id, existente);
    assert_eq!(gravado.origem, OrigemEspaco::Ligado);

    // Repetir não revalida permissão nem escreve de novo.
    let antes = cenario.com_estado().requisicoes.len();
    let repetida = cenario.espacos.posicionar(&dados).await.expect("repetir");
    assert_eq!(
        repetida,
        Posicao::JaPosicionada {
            espaco: existente.clone()
        }
    );
    assert_eq!(cenario.com_estado().requisicoes.len(), antes);
}

#[tokio::test]
async fn ligar_espaco_sem_o_bot_dentro_da_erro_claro_e_nao_grava() {
    let cenario = cenario().await;
    let sala = cenario.sala_da_conversa().await;
    let existente = cenario.espaco_do_operador(false).await;
    let mut dados = cenario.com_sala(&sala);
    let departamento = dados.departamento.clone().expect("departamento");
    dados.existente = Some(existente);

    let erro = cenario
        .espacos
        .posicionar(&dados)
        .await
        .expect_err("sem o bot no espaço tem que falhar");
    match &erro {
        ErroEspaco::SemPermissao { o_que_falta } => {
            assert!(
                o_que_falta.contains(&cenario.bot),
                "o erro diz qual usuário convidar: {o_que_falta}"
            );
            assert!(
                o_que_falta.contains("convide") && o_que_falta.contains("Element"),
                "o erro diz o que fazer: {o_que_falta}"
            );
            assert!(
                o_que_falta.contains("não é membro"),
                "o erro diz que falta o convite, não o poder: {o_que_falta}"
            );
        }
        outro => panic!("esperava SemPermissao, veio {outro:?}"),
    }
    assert_eq!(
        erro.classificacao(),
        poph_matrix::erro::Classificacao::Transitoria,
        "falta de permissão volta a ser tentada depois da correção do operador"
    );
    assert!(
        cenario
            .porta
            .espaco_de(dados.org, departamento.id)
            .is_none(),
        "espaço sem permissão não é gravado"
    );
    assert!(
        cenario
            .porta
            .espaco_da_sala(dados.org, dados.conversa)
            .is_none(),
        "a sala não é dada como posicionada"
    );
}

#[tokio::test]
async fn ligar_espaco_onde_o_bot_nao_tem_poder_da_erro_claro() {
    let cenario = cenario().await;
    let sala = cenario.sala_da_conversa().await;
    let existente = cenario.espaco_do_operador(true).await;
    // O operador tira o poder de evento de estado de todo mundo que não é ele.
    cenario
        .cliente()
        .enviar_estado(
            ATENDENTE,
            &existente,
            "m.room.power_levels",
            None,
            &json!({
                "users": { ATENDENTE: 100 },
                "users_default": 0,
                "state_default": 50,
            }),
        )
        .await
        .expect("power levels do espaço");
    let mut dados = cenario.com_sala(&sala);
    dados.existente = Some(existente);

    let erro = cenario
        .espacos
        .posicionar(&dados)
        .await
        .expect_err("sem poder tem que falhar");
    match &erro {
        ErroEspaco::SemPermissao { o_que_falta } => {
            assert!(
                o_que_falta.contains("poder") && o_que_falta.contains("50"),
                "o erro explica o poder que falta: {o_que_falta}"
            );
        }
        outro => panic!("esperava SemPermissao, veio {outro:?}"),
    }
}

#[tokio::test]
async fn move_a_sala_quando_o_departamento_muda() {
    let cenario = cenario().await;
    let sala = cenario.sala_da_conversa().await;
    let mut dados = cenario.com_sala(&sala);
    let comercial = cenario
        .espacos
        .posicionar(&dados)
        .await
        .expect("no espaço do comercial");
    let Posicao::Posicionada { espaco: antigo, .. } = comercial else {
        panic!("esperava Posicionada");
    };

    // A conversa é transferida: o departamento alvo passa a ser outro.
    dados.departamento = Some(cenario.departamento("Suporte"));
    let nova = cenario
        .espacos
        .posicionar(&dados)
        .await
        .expect("mover de espaço");
    let Posicao::Posicionada {
        espaco: novo,
        movida,
        ..
    } = nova
    else {
        panic!("esperava Posicionada");
    };
    assert!(movida, "a sala veio de outro espaço");
    assert_ne!(novo, antigo);

    let guarda = cenario.com_estado();
    let no_antigo = guarda
        .salas
        .get(&antigo)
        .expect("espaço antigo")
        .estado
        .get(&(TIPO_FILHO.to_owned(), sala.clone()))
        .expect("o filho continua escrito, mas vazio");
    assert_eq!(
        no_antigo,
        &json!({}),
        "conteúdo vazio é como o Matrix tira o filho do espaço"
    );
    assert!(
        guarda
            .salas
            .get(&novo)
            .expect("espaço novo")
            .estado
            .get(&(TIPO_FILHO.to_owned(), sala.clone()))
            .is_some_and(|corpo| corpo["via"] == json!([SERVIDOR])),
        "a sala entrou no espaço novo"
    );
    let da_sala = guarda.salas.get(&sala).expect("sala");
    assert_eq!(
        da_sala
            .estado
            .get(&(TIPO_PAI.to_owned(), antigo.clone()))
            .cloned(),
        Some(json!({})),
        "o pai antigo foi removido"
    );
    assert_eq!(
        da_sala
            .estado
            .get(&(TIPO_PAI.to_owned(), novo.clone()))
            .map(|corpo| corpo["canonical"].clone()),
        Some(json!(true)),
        "o pai novo aponta para o espaço novo"
    );
    drop(guarda);
    assert_eq!(
        cenario.porta.espaco_da_sala(dados.org, dados.conversa),
        Some(novo)
    );
}

#[tokio::test]
async fn conversa_sem_departamento_nao_falha_e_nao_fala_com_o_synapse() {
    let cenario = cenario().await;
    let sala = cenario.sala_da_conversa().await;
    let antes = cenario.com_estado().requisicoes.len();

    let mut dados = cenario.com_sala(&sala);
    dados.departamento = None;
    let posicao = cenario.espacos.posicionar(&dados).await.expect("sem dep");
    assert_eq!(posicao, Posicao::SemDepartamento);
    assert_eq!(
        cenario.com_estado().requisicoes.len(),
        antes,
        "sem departamento, nenhuma chamada de espaço"
    );
    assert!(
        cenario
            .porta
            .espaco_da_sala(dados.org, dados.conversa)
            .is_none()
    );

    // E o P3: alvo, senão padrão do agente, senão nenhum.
    let padrao = DepartmentId::generate();
    assert_eq!(resolver_departamento(None, Some(padrao)), Some(padrao));
    assert_eq!(resolver_departamento(None, None), None);
}

#[tokio::test]
async fn sala_que_fica_sem_departamento_sai_do_espaco() {
    let cenario = cenario().await;
    let sala = cenario.sala_da_conversa().await;
    let mut dados = cenario.com_sala(&sala);
    let Posicao::Posicionada { espaco, .. } =
        cenario.espacos.posicionar(&dados).await.expect("no espaço")
    else {
        panic!("esperava Posicionada");
    };

    dados.departamento = None;
    let posicao = cenario.espacos.posicionar(&dados).await.expect("sem dep");
    assert_eq!(
        posicao,
        Posicao::Removida {
            espaco: espaco.clone()
        }
    );
    {
        let guarda = cenario.com_estado();
        assert_eq!(
            guarda
                .salas
                .get(&espaco)
                .expect("espaço")
                .estado
                .get(&(TIPO_FILHO.to_owned(), sala.clone())),
            Some(&json!({})),
            "o filho foi removido do espaço"
        );
    }
    assert!(
        cenario
            .porta
            .espaco_da_sala(dados.org, dados.conversa)
            .is_none()
    );
}

#[tokio::test]
async fn convida_os_membros_do_espaco_em_lote_limitado() {
    let cenario = cenario().await;
    let sala = cenario.sala_da_conversa().await;
    let mut dados = cenario.com_sala(&sala);
    let departamento = dados.departamento.clone().expect("departamento");
    let existente = cenario.espaco_do_operador(true).await;
    cenario.dar_poder_ao_bot(&existente, 50).await;
    dados.existente = Some(existente);

    let pessoas: Vec<String> = (0..6)
        .map(|n| format!("@atendente{n}:{SERVIDOR}"))
        .collect();
    for pessoa in &pessoas {
        cenario
            .pessoa_no_espaco(pessoa, dados.existente.as_deref().expect("espaço"))
            .await;
    }

    // Teto de 4 convites por passagem (o resto fica para a próxima).
    let posicao = cenario
        .espacos
        .posicionar(&dados)
        .await
        .expect("posicionar");
    let Posicao::Posicionada {
        convidados,
        convites_falhos,
        ..
    } = posicao
    else {
        panic!("esperava Posicionada");
    };
    assert_eq!(convidados, 4, "só o teto da passagem é convidado");
    assert_eq!(convites_falhos, 0);

    {
        let guarda = cenario.com_estado();
        let convidados_na_sala = &guarda.salas.get(&sala).expect("sala").convidados;
        assert_eq!(convidados_na_sala.len(), 4);
        assert!(
            !convidados_na_sala.contains(&cenario.bot),
            "o bot não é convidado para a sala da conversa"
        );
        assert!(
            !convidados_na_sala.contains(cenario.agente.user_id()),
            "o agente já está na sala e não é convidado"
        );
    }
    assert!(
        cenario
            .porta
            .espaco_de(dados.org, departamento.id)
            .is_some()
    );

    // No Element as pessoas aceitam o convite: quem já entrou não é convidado de novo.
    let convidados_na_sala: Vec<String> = cenario
        .com_estado()
        .salas
        .get(&sala)
        .expect("sala")
        .convidados
        .iter()
        .cloned()
        .collect();
    for pessoa in convidados_na_sala {
        cenario
            .cliente()
            .entrar(&pessoa, &sala)
            .await
            .expect("join de quem aceitou");
    }

    cenario
        .porta
        .0
        .lock()
        .expect("trava")
        .espaco_das_salas
        .clear();
    let posicao = cenario.espacos.posicionar(&dados).await.expect("segunda");
    let Posicao::Posicionada { convidados, .. } = posicao else {
        panic!("esperava Posicionada");
    };
    assert_eq!(convidados, 3, "só quem ainda estava de fora");
    assert_eq!(
        cenario
            .com_estado()
            .salas
            .get(&sala)
            .expect("sala")
            .convidados
            .len(),
        7,
        "todos os membros do espaço foram convidados ao todo"
    );
}

#[tokio::test]
async fn convite_falho_nao_grava_o_vinculo_e_a_passagem_seguinte_completa() {
    let cenario = cenario().await;
    let sala = cenario.sala_da_conversa().await;
    let mut dados = cenario.com_sala(&sala);
    let departamento = dados.departamento.clone().expect("departamento");
    let existente = cenario.espaco_do_operador(true).await;
    cenario.dar_poder_ao_bot(&existente, 50).await;
    dados.existente = Some(existente.clone());

    let pessoas: Vec<String> = (0..2)
        .map(|n| format!("@atendente{n}:{SERVIDOR}"))
        .collect();
    for pessoa in &pessoas {
        cenario.pessoa_no_espaco(pessoa, &existente).await;
    }

    // Um convite falha (503, sem queda do Synapse): o vínculo **não** pode ser gravado.
    cenario.com_estado().falhar_convites = 1;
    let primeira = cenario
        .espacos
        .posicionar(&dados)
        .await
        .expect("primeira passagem");
    let Posicao::Posicionada {
        convites_falhos,
        movida,
        ..
    } = primeira
    else {
        panic!("esperava Posicionada");
    };
    assert_eq!(convites_falhos, 1, "um convite falhou");
    assert!(!movida, "a sala não é dada como movida");
    assert!(
        cenario
            .porta
            .espaco_da_sala(dados.org, dados.conversa)
            .is_none(),
        "convite pendente não pode gravar o vínculo: a passagem seguinte sairia por JaPosicionada"
    );

    // Passagem seguinte, com o convite funcionando: completa o lote e só então grava o vínculo.
    let segunda = cenario
        .espacos
        .posicionar(&dados)
        .await
        .expect("segunda passagem");
    let Posicao::Posicionada {
        convites_falhos,
        espaco,
        ..
    } = segunda
    else {
        panic!("esperava Posicionada na retomada");
    };
    assert_eq!(convites_falhos, 0, "a retomada não deixa convite pendente");
    let convidados_na_sala: Vec<String> = cenario
        .com_estado()
        .salas
        .get(&sala)
        .expect("sala")
        .convidados
        .iter()
        .cloned()
        .collect();
    for pessoa in &pessoas {
        assert!(
            convidados_na_sala.contains(pessoa),
            "{pessoa} não foi convidado na passagem seguinte"
        );
    }
    assert_eq!(
        cenario
            .porta
            .espaco_da_sala(dados.org, dados.conversa)
            .as_deref(),
        Some(espaco.as_str()),
        "com os convites fechados, o vínculo é gravado"
    );
    assert!(
        cenario
            .porta
            .espaco_de(dados.org, departamento.id)
            .is_some()
    );

    // Gravado o vínculo, repetir não escreve nada no Synapse.
    let antes = cenario.com_estado().requisicoes.len();
    let terceira = cenario
        .espacos
        .posicionar(&dados)
        .await
        .expect("terceira passagem");
    assert!(
        matches!(terceira, Posicao::JaPosicionada { .. }),
        "esperava JaPosicionada"
    );
    assert_eq!(
        cenario.com_estado().requisicoes.len(),
        antes,
        "nada foi escrito no Synapse"
    );
}

#[tokio::test]
async fn a_sala_aparece_sob_o_espaco_na_hierarquia_do_synapse() {
    let cenario = cenario().await;
    let sala = cenario.sala_da_conversa().await;
    let dados = cenario.com_sala(&sala);
    let Posicao::Posicionada { espaco, .. } = cenario
        .espacos
        .posicionar(&dados)
        .await
        .expect("posicionar")
    else {
        panic!("esperava Posicionada");
    };

    let filhas = cenario
        .cliente()
        .hierarquia(cenario.agente.user_id(), &espaco)
        .await
        .expect("hierarchy");
    assert_eq!(filhas, vec![sala], "a sala está sob o espaço");
}

#[tokio::test]
async fn falha_transitoria_do_synapse_nao_grava_o_vinculo() {
    let cenario = cenario().await;
    let sala = cenario.sala_da_conversa().await;
    let dados = cenario.com_sala(&sala);
    cenario.com_estado().falhar_proxima_criacao = true;

    let erro = cenario
        .espacos
        .posicionar(&dados)
        .await
        .expect_err("Synapse fora do ar");
    assert_eq!(
        erro.classificacao(),
        poph_matrix::erro::Classificacao::Transitoria
    );
    assert!(
        cenario
            .porta
            .espaco_de(dados.org, dados.departamento.clone().expect("dep").id)
            .is_none()
    );
    assert!(
        cenario
            .porta
            .espaco_da_sala(dados.org, dados.conversa)
            .is_none()
    );

    // Na passagem seguinte a sala entra normalmente.
    let posicao = cenario.espacos.posicionar(&dados).await.expect("retomada");
    assert!(matches!(posicao, Posicao::Posicionada { .. }));
}

#[tokio::test]
async fn espaco_informado_invalido_e_erro_de_contrato() {
    let cenario = cenario().await;
    let sala = cenario.sala_da_conversa().await;
    let mut dados = cenario.com_sala(&sala);
    dados.existente = Some("sala-sem-id-nem-alias".to_owned());
    let erro = cenario
        .espacos
        .posicionar(&dados)
        .await
        .expect_err("referência inválida");
    assert!(
        matches!(erro, ErroEspaco::EspacoInvalido { .. }),
        "{erro:?}"
    );
}

// ───────────────────────────── sala sem registro de espaço (MX-51) ─────────────────────────────

/// Quantas requisições recebidas (só o caminho) contêm este trecho.
fn requisicoes_com(cenario: &Cenario, trecho: &str) -> usize {
    cenario
        .com_estado()
        .requisicoes
        .iter()
        .filter(|caminho| caminho.contains(trecho))
        .count()
}

#[tokio::test]
async fn transferir_com_registro_vazio_tira_a_sala_do_espaco_antigo() {
    let cenario = cenario().await;
    let sala = cenario.sala_da_conversa().await;
    let mut dados = cenario.com_sala(&sala);
    dados.departamento = Some(cenario.departamento("Comercial"));
    let Posicao::Posicionada { espaco: antigo, .. } = cenario
        .espacos
        .posicionar(&dados)
        .await
        .expect("no espaço do comercial")
    else {
        panic!("esperava Posicionada");
    };

    // A janela da criação: a sala já está no espaço, mas o registro da conversa ainda não diz isso.
    cenario
        .porta
        .definir_espaco_da_sala(dados.org, dados.conversa, None)
        .await
        .expect("registro vazio");

    dados.departamento = Some(cenario.departamento("Suporte"));
    let escritas_no_antigo_antes =
        requisicoes_com(&cenario, &format!("{antigo}/state/{TIPO_FILHO}/"));
    let nova = cenario
        .espacos
        .posicionar(&dados)
        .await
        .expect("transferência com registro vazio");
    let Posicao::Posicionada {
        espaco: novo,
        movida,
        convites_falhos: 0,
        ..
    } = nova.clone()
    else {
        panic!("esperava Posicionada sem convite falho, veio {nova:?}");
    };
    assert!(movida, "a sala veio do espaço antigo, mesmo sem o registro");
    assert_ne!(novo, antigo);

    {
        let guarda = cenario.com_estado();
        assert_eq!(
            guarda.salas[&antigo]
                .estado
                .get(&(TIPO_FILHO.to_owned(), sala.clone())),
            Some(&json!({})),
            "o m.space.child sai do espaço antigo"
        );
        let da_sala = &guarda.salas[&sala];
        assert_eq!(
            da_sala.estado.get(&(TIPO_PAI.to_owned(), antigo.clone())),
            Some(&json!({})),
            "o m.space.parent do espaço antigo sai da sala"
        );
        assert_eq!(
            da_sala
                .estado
                .get(&(TIPO_PAI.to_owned(), novo.clone()))
                .map(|corpo| corpo["canonical"].clone()),
            Some(json!(true)),
            "o m.space.parent do espaço novo fica na sala"
        );
    }
    let cliente = cenario.cliente();
    let sob_antigo = cliente
        .hierarquia(&cenario.bot, &antigo)
        .await
        .expect("hierarquia do antigo");
    assert!(
        !sob_antigo.contains(&sala),
        "a sala não aparece mais sob o espaço antigo"
    );
    let sob_novo = cliente
        .hierarquia(&cenario.bot, &novo)
        .await
        .expect("hierarquia do novo");
    assert!(sob_novo.contains(&sala), "a sala aparece sob o espaço novo");
    assert_eq!(
        cenario.porta.espaco_da_sala(dados.org, dados.conversa),
        Some(novo.clone())
    );

    // Outra passagem com o registro ainda vazio não escreve no espaço antigo de novo: o pai já
    // tirado (conteúdo vazio) não conta como espaço de onde sair.
    let escritas_antigas = requisicoes_com(&cenario, &format!("{antigo}/state/{TIPO_FILHO}/"));
    assert_eq!(
        escritas_antigas,
        escritas_no_antigo_antes + 1,
        "a transferência tirou o m.space.child do espaço antigo com uma escrita só"
    );
    cenario
        .porta
        .definir_espaco_da_sala(dados.org, dados.conversa, None)
        .await
        .expect("registro vazio de novo");
    let repetida = cenario
        .espacos
        .posicionar(&dados)
        .await
        .expect("repetir com registro vazio");
    assert!(
        matches!(repetida, Posicao::Posicionada { movida: false, .. }),
        "repetir não move a sala de novo, veio {repetida:?}"
    );
    assert_eq!(
        requisicoes_com(&cenario, &format!("{antigo}/state/{TIPO_FILHO}/")),
        escritas_antigas,
        "o espaço antigo não recebe escrita de novo"
    );
}

#[tokio::test]
async fn transferir_com_registro_vazio_e_convite_falho_so_sai_do_antigo_na_passagem_seguinte() {
    let cenario = cenario().await;
    let sala = cenario.sala_da_conversa().await;
    let mut dados = cenario.com_sala(&sala);
    dados.departamento = Some(cenario.departamento("Comercial"));
    let Posicao::Posicionada { espaco: antigo, .. } = cenario
        .espacos
        .posicionar(&dados)
        .await
        .expect("no espaço do comercial")
    else {
        panic!("esperava Posicionada");
    };
    cenario
        .porta
        .definir_espaco_da_sala(dados.org, dados.conversa, None)
        .await
        .expect("registro vazio");

    // O espaço de destino já existe no Element, com alguém dentro que ainda não está na sala.
    let suporte = cenario.espaco_do_operador(true).await;
    cenario.dar_poder_ao_bot(&suporte, 50).await;
    cenario
        .pessoa_no_espaco("@atendente0:matrix-dev.local", &suporte)
        .await;
    dados.departamento = Some(cenario.departamento("Suporte"));
    dados.existente = Some(suporte.clone());

    cenario.com_estado().falhar_convites = 1;
    let primeira = cenario
        .espacos
        .posicionar(&dados)
        .await
        .expect("primeira passagem");
    let Posicao::Posicionada {
        convites_falhos: 1,
        movida: false,
        ..
    } = primeira
    else {
        panic!("esperava um convite falho, veio {primeira:?}");
    };
    assert!(
        cenario
            .porta
            .espaco_da_sala(dados.org, dados.conversa)
            .is_none(),
        "convite pendente não grava o vínculo"
    );
    assert!(
        cenario.com_estado().salas[&antigo]
            .estado
            .get(&(TIPO_FILHO.to_owned(), sala.clone()))
            .is_some_and(|corpo| corpo != &json!({})),
        "com convite pendente a sala ainda não saiu do espaço antigo"
    );

    cenario.com_estado().falhar_convites = 0;
    let segunda = cenario
        .espacos
        .posicionar(&dados)
        .await
        .expect("passagem seguinte");
    let Posicao::Posicionada {
        espaco,
        movida: true,
        convites_falhos: 0,
        ..
    } = segunda
    else {
        panic!("esperava a sala movida na passagem seguinte, veio {segunda:?}");
    };
    assert_eq!(espaco, suporte);
    assert_eq!(
        cenario.porta.espaco_da_sala(dados.org, dados.conversa),
        Some(suporte.clone())
    );
    assert_eq!(
        cenario.com_estado().salas[&antigo]
            .estado
            .get(&(TIPO_FILHO.to_owned(), sala.clone())),
        Some(&json!({})),
        "na passagem seguinte a sala sai do espaço antigo"
    );
}

#[tokio::test]
async fn transferir_com_registro_preenchido_nao_le_o_estado_da_sala() {
    let cenario = cenario().await;
    let sala = cenario.sala_da_conversa().await;
    let mut dados = cenario.com_sala(&sala);
    dados.departamento = Some(cenario.departamento("Comercial"));
    cenario
        .espacos
        .posicionar(&dados)
        .await
        .expect("no espaço do comercial");

    dados.departamento = Some(cenario.departamento("Suporte"));
    let antes = cenario.com_estado().requisicoes.len();
    let nova = cenario
        .espacos
        .posicionar(&dados)
        .await
        .expect("transferência com registro");
    assert!(
        matches!(nova, Posicao::Posicionada { movida: true, .. }),
        "veio {nova:?}"
    );
    let depois: Vec<String> = cenario.com_estado().requisicoes[antes..].to_vec();
    assert!(
        !depois.iter().any(|caminho| caminho.ends_with("/state")),
        "com o registro preenchido, a transferência não lê o estado inteiro da sala: {depois:?}"
    );

    // Repetir com o registro gravado sai por `JaPosicionada`: nenhuma requisição.
    let antes_repetida = cenario.com_estado().requisicoes.len();
    let repetida = cenario
        .espacos
        .posicionar(&dados)
        .await
        .expect("passagem repetida");
    assert!(
        matches!(repetida, Posicao::JaPosicionada { .. }),
        "veio {repetida:?}"
    );
    assert_eq!(
        cenario.com_estado().requisicoes.len(),
        antes_repetida,
        "a passagem repetida não fala com o Synapse"
    );
}
