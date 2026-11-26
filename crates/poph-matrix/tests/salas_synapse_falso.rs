//! Testes da provisão de sala contra um Synapse **falso** e uma porta em memória (MX-11).
//!
//! O Synapse falso é **com estado**: guarda usuários, salas, aliases, membros, eventos de estado e
//! mensagens, e responde como o Synapse responderia (alias repetido devolve `M_ROOM_IN_USE`, alias
//! desconhecido devolve `M_NOT_FOUND`). Isso é o que permite provar o que só aparece falando com o
//! servidor: queda entre o `createRoom` e a gravação no banco, retomada pelo alias e duas chamadas
//! concorrentes para a mesma conversa.
//!
//! A porta em memória imita o repositório `store.matrix()` da MX-08, inclusive a regra de que a
//! primeira gravação da sala ganha, e sabe "cair" no ponto pedido pelo teste.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::extract::State;
use axum::http::header::AUTHORIZATION;
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use poph_matrix::erro::Classificacao;
use poph_matrix::salas::{
    AVISO_SEM_HISTORICO, ConversaParaSala, ErroPorta, EstadoGravado, PODER_CRIACAO, PortaSalas,
    Provisionador, SalaGravada, TIPO_METADATA, TIPO_SALA,
};
use poph_matrix::{ClienteMatrix, ConfigCliente};
use poph_types::{AgentId, ConversationId, DepartmentId, MatrixConfig, OrgId, Secret};
use serde_json::{Value, json};

/// Valor-canário do token do Application Service.
const CANARIO: &str = "CANARIO-MX11-nao-pode-vazar";

/// Domínio dos ids no teste.
const SERVIDOR: &str = "matrix-dev.local";

// ───────────────────────────── Synapse falso com estado ─────────────────────────────

#[derive(Clone, Debug)]
struct Requisicao {
    metodo: String,
    caminho: String,
    corpo: Value,
}

#[derive(Default, Clone)]
struct SalaFalsa {
    tipo: Option<String>,
    nome: Option<String>,
    topico: Option<String>,
    criador: String,
    membros: BTreeSet<String>,
    convidados: BTreeSet<String>,
    estado: BTreeMap<(String, String), Value>,
    mensagens: Vec<(String, String, Value)>,
    poder: Option<Value>,
}

#[derive(Default)]
struct Estado {
    token_esperado: String,
    usuarios: BTreeMap<String, Option<String>>,
    salas: BTreeMap<String, SalaFalsa>,
    aliases: BTreeMap<String, String>,
    proxima_sala: u32,
    requisicoes: Vec<Requisicao>,
    criacoes: u32,
    falhar_criacao: Option<(u16, Value)>,
}

impl Estado {
    fn registros(&self, sufixo: &str) -> Vec<&Requisicao> {
        self.requisicoes
            .iter()
            .filter(|requisicao| requisicao.caminho.ends_with(sufixo))
            .collect()
    }

    fn contagem(&self, sufixo: &str) -> usize {
        self.registros(sufixo).len()
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

async fn atender(
    State(estado): State<Arc<Mutex<Estado>>>,
    metodo: Method,
    uri: Uri,
    cabecalhos: HeaderMap,
    corpo: String,
) -> Response {
    let corpo: Value = serde_json::from_str(&corpo).unwrap_or(Value::Null);
    let mut guarda = estado.lock().expect("trava do Synapse falso");

    guarda.requisicoes.push(Requisicao {
        metodo: metodo.to_string(),
        caminho: uri.path().to_owned(),
        corpo: corpo.clone(),
    });

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
            if let Some((status, corpo)) = guarda.falhar_criacao.take() {
                return responder(status, corpo);
            }
            // O Synapse real exige poder 100 do criador para escrever o próprio
            // `m.room.power_levels` na criação (defeito achado no aceite, MX-31). Sem esta regra o
            // falso aceitaria uma sala que nasce pela metade no ambiente de verdade.
            if let Some(usuarios) = corpo["power_level_content_override"]["users"].as_object() {
                let poder = usuarios.get(&quem).and_then(Value::as_i64).unwrap_or(0);
                if poder < 100 {
                    return responder(
                        403,
                        json!({
                            "errcode": "M_FORBIDDEN",
                            "error": format!(
                                "You don't have permission to post that to the room. \
                                 user_level ({poder}) < send_level (100)"
                            ),
                        }),
                    );
                }
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
            let mut estado_da_sala = BTreeMap::new();
            if let Some(nome) = corpo["name"].as_str() {
                estado_da_sala.insert(
                    ("m.room.name".to_owned(), String::new()),
                    json!({ "name": nome }),
                );
            }
            // O Synapse real grava o `m.room.power_levels` da criação como evento de estado; sem
            // isso o `GET` dos poderes daria 404 e o rebaixamento não teria o que ler.
            if !corpo["power_level_content_override"].is_null() {
                estado_da_sala.insert(
                    ("m.room.power_levels".to_owned(), String::new()),
                    corpo["power_level_content_override"].clone(),
                );
            }
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
                    estado: estado_da_sala,
                    poder: Some(corpo["power_level_content_override"].clone()),
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
                        json!({ "errcode": "M_NOT_FOUND", "error": "Evento de estado não existe" }),
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

// ───────────────────────────── porta em memória ─────────────────────────────

#[derive(Default)]
struct EstadoPorta {
    salas: BTreeMap<(OrgId, ConversationId), SalaGravada>,
    historico: bool,
    falhas_de_gravacao: u32,
    gravacoes: usize,
    marcas_de_pronta: usize,
    /// Nome do cliente gravado junto com o `ready` (MX-33): `None` = a sala nasceu com o telefone.
    nomes_espelhados: BTreeMap<(OrgId, ConversationId), Option<String>>,
}

/// Imita `store.matrix()`: a primeira gravação da sala ganha e repetir não muda nada.
#[derive(Clone, Default)]
struct PortaFalsa(Arc<Mutex<EstadoPorta>>);

impl PortaFalsa {
    fn historico(&self, valor: bool) {
        self.0.lock().expect("trava").historico = valor;
    }

    fn cair_na_proxima_gravacao(&self, vezes: u32) {
        self.0.lock().expect("trava").falhas_de_gravacao = vezes;
    }

    fn sala_de(&self, org: OrgId, conversa: ConversationId) -> Option<SalaGravada> {
        self.0
            .lock()
            .expect("trava")
            .salas
            .get(&(org, conversa))
            .cloned()
    }

    fn devolver_para_criando(&self, org: OrgId, conversa: ConversationId) {
        if let Some(sala) = self
            .0
            .lock()
            .expect("trava")
            .salas
            .get_mut(&(org, conversa))
        {
            sala.estado = EstadoGravado::Criando;
        }
    }

    fn marcas_de_pronta(&self) -> usize {
        self.0.lock().expect("trava").marcas_de_pronta
    }

    fn nome_espelhado(&self, org: OrgId, conversa: ConversationId) -> Option<Option<String>> {
        self.0
            .lock()
            .expect("trava")
            .nomes_espelhados
            .get(&(org, conversa))
            .cloned()
    }
}

impl PortaSalas for PortaFalsa {
    async fn sala(
        &self,
        org: OrgId,
        conversa: ConversationId,
    ) -> Result<Option<SalaGravada>, ErroPorta> {
        Ok(self.sala_de(org, conversa))
    }

    async fn garantir_sala(
        &self,
        org: OrgId,
        conversa: ConversationId,
        room_id: &str,
    ) -> Result<SalaGravada, ErroPorta> {
        let mut guarda = self.0.lock().expect("trava");
        if guarda.falhas_de_gravacao > 0 {
            guarda.falhas_de_gravacao -= 1;
            return Err(ErroPorta::nova("o banco caiu antes de gravar a sala"));
        }
        guarda.gravacoes += 1;
        let sala = guarda
            .salas
            .entry((org, conversa))
            .or_insert_with(|| SalaGravada {
                room_id: room_id.to_owned(),
                estado: EstadoGravado::Criando,
            });
        Ok(sala.clone())
    }

    async fn marcar_pronta(
        &self,
        org: OrgId,
        conversa: ConversationId,
        nome_espelhado: Option<&str>,
    ) -> Result<SalaGravada, ErroPorta> {
        let mut guarda = self.0.lock().expect("trava");
        guarda.marcas_de_pronta += 1;
        let nome = nome_espelhado.map(str::to_owned);
        let sala = match guarda.salas.get_mut(&(org, conversa)) {
            Some(sala) => {
                sala.estado = EstadoGravado::Pronta;
                sala.clone()
            }
            None => return Err(ErroPorta::nova("sala ausente na porta")),
        };
        guarda.nomes_espelhados.insert((org, conversa), nome);
        Ok(sala)
    }

    async fn teve_mensagens_antes(
        &self,
        _org: OrgId,
        _conversa: ConversationId,
        _corte: i64,
    ) -> Result<bool, ErroPorta> {
        Ok(self.0.lock().expect("trava").historico)
    }
}

// ───────────────────────────── cenário ─────────────────────────────

struct Cenario {
    estado: Arc<Mutex<Estado>>,
    porta: PortaFalsa,
    provisionador: Provisionador<PortaFalsa>,
    dados: ConversaParaSala,
    /// Endereço do Synapse falso (para montar um provisionador novo, como num reinício).
    endereco: String,
}

impl Cenario {
    fn com_estado(&self) -> std::sync::MutexGuard<'_, Estado> {
        self.estado.lock().expect("trava do Synapse falso")
    }

    /// Um provisionador novo sobre o mesmo Synapse e o mesmo banco: a cara de um reinício.
    fn reiniciado(&self) -> Provisionador<PortaFalsa> {
        self.com_poder(50)
    }

    /// Provisionador novo com outro poder de agente (o da seção `[matrix]`).
    fn com_poder(&self, poder: u8) -> Provisionador<PortaFalsa> {
        Provisionador::com(
            self.provisionador_cliente(),
            self.provisionador_usuarios(),
            self.porta.clone(),
            poder,
        )
    }

    fn provisionador_cliente(&self) -> ClienteMatrix {
        ClienteMatrix::novo(
            ConfigCliente::novo(
                self.endereco.clone(),
                SERVIDOR,
                Secret::new(CANARIO.to_owned()),
            )
            .expect("configuração do cliente"),
        )
        .expect("cliente")
    }

    fn provisionador_usuarios(&self) -> poph_matrix::usuarios::UsuariosVirtuais {
        poph_matrix::usuarios::UsuariosVirtuais::novo(
            self.provisionador_cliente(),
            poph_matrix::usuarios::IdsFantasmas::novo("ph", SERVIDOR).expect("ids"),
            64,
        )
    }
}

impl Cenario {
    fn sala_unica(&self) -> (String, SalaFalsa) {
        let guarda = self.com_estado();
        assert_eq!(guarda.salas.len(), 1, "esperava uma sala só");
        let (id, sala) = guarda.salas.iter().next().expect("sala");
        (id.clone(), sala.clone())
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
        enabled: true,
        homeserver_url: format!("http://{endereco}"),
        server_name: SERVIDOR.to_owned(),
        prefix: "ph".to_owned(),
        agent_power: 50,
        ..MatrixConfig::default()
    };
    let cliente = ClienteMatrix::novo(
        ConfigCliente::das_partes(
            &config,
            &poph_matrix::SegredosMatrix {
                as_token: Secret::new(CANARIO.to_owned()),
                hs_token: Secret::new(CANARIO.to_owned()),
            },
        )
        .expect("configuração do cliente"),
    )
    .expect("cliente");
    let porta = PortaFalsa::default();
    let provisionador =
        Provisionador::da_config(cliente, porta.clone(), &config).expect("provisionador");

    let dados = ConversaParaSala {
        org: OrgId::generate(),
        conversa: ConversationId::generate(),
        cliente: "+55 (11) 99999-9999".to_owned(),
        nome_cliente: Some("Maria".to_owned()),
        agente: AgentId::generate(),
        nome_agente: Some("Agente de Teste".to_owned()),
        canal: "whatsapp".to_owned(),
        departamento: Some(DepartmentId::generate()),
        fechada: false,
        corte_backfill: None,
    };

    Cenario {
        estado,
        porta,
        provisionador,
        dados,
        endereco: format!("http://{endereco}"),
    }
}

// ───────────────────────────── testes ─────────────────────────────

#[tokio::test]
async fn provisao_cria_uma_sala_com_tipo_nome_e_metadata_do_frontend() {
    let c = cenario().await;

    let pronta = c
        .provisionador
        .provisionar(&c.dados)
        .await
        .expect("sala pronta");

    let (room_id, sala) = c.sala_unica();
    assert_eq!(pronta.room_id, room_id);
    assert!(!pronta.ja_estava_pronta);

    // Tipo da sala e nome do cliente.
    assert_eq!(sala.tipo.as_deref(), Some(TIPO_SALA));
    assert_eq!(sala.nome.as_deref(), Some("Maria"));
    assert!(
        sala.topico
            .as_deref()
            .unwrap_or_default()
            .contains("whatsapp"),
        "{:?}",
        sala.topico
    );

    // A criação pede o poder que o Synapse exige do criador; o poder final desce para o da
    // configuração (`poder_agente = 50`) na escrita de `m.room.power_levels` que fecha a sala.
    let agente = format!("@ph_ag_{}:{SERVIDOR}", c.dados.agente);
    assert_eq!(
        sala.poder.as_ref().expect("poder")["users"][&agente],
        json!(PODER_CRIACAO),
        "poder do agente na criação"
    );
    assert_eq!(
        sala.estado[&("m.room.power_levels".to_owned(), String::new())]["users"][&agente],
        json!(50),
        "poder final do agente na sala"
    );

    // Cliente e agente dentro da sala.
    let cliente = format!("@ph_wa_5511999999999:{SERVIDOR}");
    assert!(sala.membros.contains(&cliente), "{:?}", sala.membros);
    assert!(sala.membros.contains(&agente), "{:?}", sala.membros);

    // Metadata exatamente com os campos do frontend que o Harness tem.
    let metadata = sala
        .estado
        .get(&(TIPO_METADATA.to_owned(), String::new()))
        .expect("evento de metadata");
    assert_eq!(
        metadata["conversation_id"],
        json!(c.dados.conversa.to_string())
    );
    assert_eq!(metadata["agent_id"], json!(c.dados.agente.to_string()));
    assert_eq!(metadata["integration_type"], json!("whatsapp"));
    assert_eq!(metadata["channel"], json!("whatsapp"));
    assert_eq!(metadata["client_identifier"], json!("+55 (11) 99999-9999"));
    assert_eq!(metadata["client_name"], json!("Maria"));
    assert_eq!(
        metadata["department_id"],
        json!(c.dados.departamento.expect("departamento").to_string())
    );
    assert_eq!(metadata["status"], json!("active"));
    let mut chaves: Vec<&String> = metadata.as_object().expect("objeto").keys().collect();
    chaves.sort();
    assert_eq!(
        chaves,
        vec![
            "agent_id",
            "channel",
            "client_identifier",
            "client_name",
            "conversation_id",
            "department_id",
            "integration_type",
            "status",
        ],
        "campo a mais ou a menos no metadata"
    );

    // Alias determinístico e sem hífen.
    let alias_esperado = c.provisionador.alias(c.dados.conversa);
    assert!(alias_esperado.starts_with("#ph_c_"), "{alias_esperado}");
    let guarda = c.com_estado();
    assert_eq!(
        guarda.aliases.get(&alias_esperado).cloned(),
        Some(room_id.clone())
    );

    // O corpo do `createRoom` leva o alias, o tipo e o poder; o criador é o fantasma do agente.
    let criacao = guarda
        .requisicoes
        .iter()
        .find(|requisicao| requisicao.caminho.ends_with("/createRoom"))
        .expect("createRoom");
    assert_eq!(
        criacao.corpo["room_alias_name"],
        json!(format!(
            "ph_c_{}",
            c.dados.conversa.to_string().replace('-', "")
        ))
    );
    assert_eq!(criacao.corpo["creation_content"]["type"], json!(TIPO_SALA));
    assert_eq!(criacao.corpo["preset"], json!("private_chat"));
    // A criação usa 100: é o que o Synapse exige do criador para escrever o próprio
    // `m.room.power_levels` (MX-31). O poder configurado (`poder_agente = 50`) desce depois.
    assert_eq!(
        criacao.corpo["power_level_content_override"]["users"][&agente],
        json!(100)
    );
    assert_eq!(
        sala.criador, agente,
        "quem cria a sala é o fantasma do agente"
    );

    // Nenhuma requisição carrega o token no corpo ou no caminho: ele só vai no cabeçalho.
    for requisicao in &guarda.requisicoes {
        assert!(
            !requisicao.corpo.to_string().contains(CANARIO),
            "token no corpo de {}",
            requisicao.caminho
        );
        assert!(
            !requisicao.caminho.contains(CANARIO),
            "token no caminho {}",
            requisicao.caminho
        );
        assert!(!requisicao.metodo.is_empty());
    }
    drop(guarda);

    // Linha gravada e `ready`.
    let gravada = c
        .porta
        .sala_de(c.dados.org, c.dados.conversa)
        .expect("linha da sala");
    assert_eq!(gravada.estado, EstadoGravado::Pronta);
    assert_eq!(gravada.room_id, room_id);

    // O nome que a sala recebeu fica gravado junto com o `ready` (MX-33): é o que o laço de estado
    // compara para saber se precisa renomear a sala quando o nome do cliente mudar.
    assert_eq!(
        c.porta.nome_espelhado(c.dados.org, c.dados.conversa),
        Some(Some("Maria".to_owned())),
        "nome do cliente espelhado na criação"
    );
}

#[tokio::test]
async fn sala_criada_sem_nome_do_cliente_fica_com_o_nome_espelhado_vazio() {
    let mut c = cenario().await;
    c.dados.nome_cliente = None;

    c.provisionador
        .provisionar(&c.dados)
        .await
        .expect("sala pronta");

    // Sala nascida com o identificador: a coluna fica vazia e a MX-33 renomeia quando o nome vier.
    assert_eq!(
        c.porta.nome_espelhado(c.dados.org, c.dados.conversa),
        Some(None),
        "sem nome do cliente, o nome espelhado é vazio"
    );
    let (_, sala) = c.sala_unica();
    assert_eq!(sala.nome.as_deref(), Some("+55 (11) 99999-9999"));
}

#[tokio::test]
async fn queda_entre_criar_e_gravar_deixa_uma_sala_so_e_a_retomada_acha_pelo_alias() {
    let c = cenario().await;
    c.porta.cair_na_proxima_gravacao(1);

    let erro = c
        .provisionador
        .provisionar(&c.dados)
        .await
        .expect_err("o banco caiu antes de gravar");
    assert_eq!(
        erro.classificacao(),
        Classificacao::Transitoria,
        "banco indisponível passa: {erro}"
    );

    // A sala existe no Synapse e não existe no banco — exatamente a queda pedida.
    let (room_id_criada, _) = c.sala_unica();
    assert!(c.porta.sala_de(c.dados.org, c.dados.conversa).is_none());

    // Reinício: provisionador novo (cache de fantasmas vazio), mesmo Synapse, mesmo banco.
    let reiniciado = c.reiniciado();
    let pronta = reiniciado
        .provisionar(&c.dados)
        .await
        .expect("a retomada completa a sala");

    assert_eq!(pronta.room_id, room_id_criada, "a sala é a mesma");
    let guarda = c.com_estado();
    assert_eq!(guarda.salas.len(), 1, "nenhuma sala duplicada");
    assert_eq!(guarda.criacoes, 1, "o Synapse criou uma sala só");
    assert!(
        guarda.contagem("/createRoom") >= 2,
        "a retomada tentou criar e caiu no alias em uso"
    );
    drop(guarda);

    let gravada = c
        .porta
        .sala_de(c.dados.org, c.dados.conversa)
        .expect("linha gravada na retomada");
    assert_eq!(gravada.estado, EstadoGravado::Pronta);
    assert_eq!(gravada.room_id, room_id_criada);
}

#[tokio::test]
async fn linha_em_creating_termina_a_criacao_sem_criar_outra_sala() {
    let c = cenario().await;
    c.porta.cair_na_proxima_gravacao(1);
    // Primeira passagem: sala criada no Synapse, gravação falha.
    let _ = c.provisionador.provisionar(&c.dados).await;
    let (room_id, _) = c.sala_unica();
    // O "processo" anterior chegou a gravar a linha em `creating` e caiu antes de terminar.
    c.porta
        .garantir_sala(c.dados.org, c.dados.conversa, &room_id)
        .await
        .expect("grava a linha em creating");
    assert_eq!(
        c.porta
            .sala_de(c.dados.org, c.dados.conversa)
            .expect("linha")
            .estado,
        EstadoGravado::Criando
    );

    let pronta = c
        .reiniciado()
        .provisionar(&c.dados)
        .await
        .expect("termina a criação");

    assert_eq!(pronta.room_id, room_id);
    let guarda = c.com_estado();
    assert_eq!(guarda.salas.len(), 1);
    assert_eq!(
        guarda.criacoes, 1,
        "não cria sala nova ao terminar a criação"
    );
    let sala = guarda.salas.get(&room_id).expect("sala");
    assert!(
        sala.estado
            .contains_key(&(TIPO_METADATA.to_owned(), String::new())),
        "a retomada escreve o metadata que faltava"
    );
    drop(guarda);
    assert_eq!(
        c.porta
            .sala_de(c.dados.org, c.dados.conversa)
            .expect("linha")
            .estado,
        EstadoGravado::Pronta
    );
}

#[tokio::test]
async fn conversas_diferentes_nunca_compartilham_sala() {
    let c = cenario().await;
    let outra = ConversaParaSala {
        conversa: ConversationId::generate(),
        cliente: "+55 (21) 98888-8888".to_owned(),
        nome_cliente: Some("João".to_owned()),
        ..c.dados.clone()
    };

    let primeira = c
        .provisionador
        .provisionar(&c.dados)
        .await
        .expect("primeira sala");
    let segunda = c
        .provisionador
        .provisionar(&outra)
        .await
        .expect("segunda sala");

    assert_ne!(primeira.room_id, segunda.room_id);
    let guarda = c.com_estado();
    assert_eq!(guarda.salas.len(), 2);
    assert_eq!(guarda.aliases.len(), 2);
    assert_ne!(
        c.provisionador.alias(c.dados.conversa),
        c.provisionador.alias(outra.conversa)
    );
    let nomes: BTreeSet<String> = guarda
        .salas
        .values()
        .filter_map(|sala| sala.nome.clone())
        .collect();
    assert_eq!(
        nomes,
        BTreeSet::from(["Maria".to_owned(), "João".to_owned()])
    );
}

#[tokio::test]
async fn duas_chamadas_concorrentes_criam_uma_sala_so() {
    let c = cenario().await;

    let (uma, outra) = tokio::join!(
        c.provisionador.provisionar(&c.dados),
        c.provisionador.provisionar(&c.dados)
    );
    let uma = uma.expect("primeira chamada");
    let outra = outra.expect("segunda chamada");

    assert_eq!(uma.room_id, outra.room_id, "as duas caem na mesma sala");
    let guarda = c.com_estado();
    assert_eq!(guarda.salas.len(), 1, "uma sala só no Synapse");
    assert_eq!(guarda.criacoes, 1, "o alias impediu a segunda criação");
    assert!(
        guarda.contagem("/createRoom") >= 2,
        "a segunda chamada tentou criar e resolveu pelo alias"
    );
}

#[tokio::test]
async fn sala_pronta_nao_fala_com_o_synapse() {
    let c = cenario().await;
    c.provisionador
        .provisionar(&c.dados)
        .await
        .expect("primeira passagem");
    let antes = c.com_estado().requisicoes.len();

    let pronta = c
        .provisionador
        .provisionar(&c.dados)
        .await
        .expect("segunda passagem");
    assert!(pronta.ja_estava_pronta);
    assert_eq!(
        c.com_estado().requisicoes.len(),
        antes,
        "linha pronta não gera requisição"
    );
}

#[tokio::test]
async fn cliente_que_ja_esta_na_sala_nao_recebe_convite_de_novo() {
    let c = cenario().await;
    c.provisionador
        .provisionar(&c.dados)
        .await
        .expect("primeira passagem");
    assert_eq!(c.com_estado().contagem("/invite"), 1);

    // Volta a linha para `creating`, como se a criação tivesse ficado pela metade.
    c.porta.devolver_para_criando(c.dados.org, c.dados.conversa);
    c.provisionador
        .provisionar(&c.dados)
        .await
        .expect("retomada");

    assert_eq!(
        c.com_estado().contagem("/invite"),
        1,
        "o cliente já estava na sala: nenhum convite novo"
    );
    assert_eq!(
        c.porta.marcas_de_pronta(),
        2,
        "a retomada marca pronta de novo"
    );
}

#[tokio::test]
async fn aviso_de_historico_so_quando_havia_mensagem_antes_do_corte() {
    let c = cenario().await;
    let dados = ConversaParaSala {
        corte_backfill: Some(1_000),
        ..c.dados.clone()
    };
    let pronta = c.provisionador.provisionar(&dados).await.expect("sala");
    assert!(!pronta.aviso_enviado);
    let (_, sala) = c.sala_unica();
    assert!(sala.mensagens.is_empty(), "sem histórico: nenhum aviso");

    let com_historico = cenario().await;
    com_historico.porta.historico(true);
    let dados = ConversaParaSala {
        corte_backfill: Some(1_000),
        ..com_historico.dados.clone()
    };
    let pronta = com_historico
        .provisionador
        .provisionar(&dados)
        .await
        .expect("sala");
    assert!(pronta.aviso_enviado);
    let (_, sala) = com_historico.sala_unica();
    assert_eq!(sala.mensagens.len(), 1, "um aviso");
    let (txn, tipo, corpo) = &sala.mensagens[0];
    assert_eq!(tipo, "m.room.message");
    assert_eq!(corpo["msgtype"], json!("m.notice"));
    assert_eq!(corpo["body"], json!(AVISO_SEM_HISTORICO));
    assert!(txn.contains("hist"), "{txn}");

    // Sem corte (org nunca ligou), não há histórico a anunciar.
    let sem_corte = cenario().await;
    sem_corte.porta.historico(true);
    let pronta = sem_corte
        .provisionador
        .provisionar(&ConversaParaSala {
            corte_backfill: None,
            ..sem_corte.dados.clone()
        })
        .await
        .expect("sala");
    assert!(!pronta.aviso_enviado);
    let (_, sala) = sem_corte.sala_unica();
    assert!(sala.mensagens.is_empty(), "sem corte: nenhum aviso");
}

#[tokio::test]
async fn falha_transitoria_do_synapse_nao_grava_sala() {
    let c = cenario().await;
    c.com_estado().falhar_criacao = Some((
        503,
        json!({ "errcode": "M_UNKNOWN", "error": "Synapse indisponível" }),
    ));

    let erro = c
        .provisionador
        .provisionar(&c.dados)
        .await
        .expect_err("o Synapse não respondeu");
    assert_eq!(erro.classificacao(), Classificacao::Transitoria, "{erro}");
    assert!(
        c.porta.sala_de(c.dados.org, c.dados.conversa).is_none(),
        "nada gravado quando a sala não nasceu"
    );
    assert_eq!(c.com_estado().salas.len(), 0);
}

#[tokio::test]
async fn canal_que_o_frontend_nao_conhece_vai_sem_integration_type() {
    let c = cenario().await;
    let dados = ConversaParaSala {
        canal: "console".to_owned(),
        fechada: true,
        nome_cliente: None,
        cliente: "5511999999999".to_owned(),
        departamento: None,
        ..c.dados.clone()
    };
    c.provisionador.provisionar(&dados).await.expect("sala");

    let (_, sala) = c.sala_unica();
    let metadata = sala
        .estado
        .get(&(TIPO_METADATA.to_owned(), String::new()))
        .expect("metadata");
    assert!(metadata.get("integration_type").is_none(), "{metadata}");
    assert_eq!(metadata["channel"], json!("console"));
    assert_eq!(metadata["status"], json!("closed"));
    assert!(metadata.get("department_id").is_none(), "{metadata}");
    assert!(
        metadata.get("client_name").is_none(),
        "sem nome não vai campo vazio: {metadata}"
    );
    // Sem nome de cliente, a sala leva o identificador — é o que distingue as salas no Element.
    assert_eq!(sala.nome.as_deref(), Some("5511999999999"));
}

#[tokio::test]
async fn sala_nasce_com_poder_de_criacao_e_desce_para_o_poder_configurado() {
    let c = cenario().await;
    let agente = format!("@ph_ag_{}:{SERVIDOR}", c.dados.agente);

    // `poder_agente = 10` ficaria abaixo do piso (50): a criação usa 100 e o poder final é 50.
    c.com_poder(10)
        .provisionar(&c.dados)
        .await
        .expect("sala criada mesmo com poder baixo");
    {
        let guarda = c.com_estado();
        let criacao = guarda
            .requisicoes
            .iter()
            .find(|requisicao| requisicao.caminho.ends_with("/createRoom"))
            .expect("createRoom");
        assert_eq!(
            criacao.corpo["power_level_content_override"]["users"][&agente],
            json!(PODER_CRIACAO),
            "a criação precisa do poder que o Synapse exige para escrever os próprios poderes"
        );
        let (_, sala) = guarda.salas.iter().next().expect("sala");
        let poderes = sala
            .estado
            .get(&("m.room.power_levels".to_owned(), String::new()))
            .expect("poderes da sala");
        assert_eq!(
            poderes["users"][&agente],
            json!(50),
            "o piso de poder é o mínimo que mantém o módulo capaz de atualizar a sala"
        );
    }

    // Poder acima do piso é respeitado como poder final.
    let outro = cenario().await;
    let agente_do_outro = format!("@ph_ag_{}:{SERVIDOR}", outro.dados.agente);
    outro
        .com_poder(80)
        .provisionar(&outro.dados)
        .await
        .expect("sala com poder 80");
    let guarda = outro.com_estado();
    let criacao = guarda
        .requisicoes
        .iter()
        .find(|requisicao| requisicao.caminho.ends_with("/createRoom"))
        .expect("createRoom");
    assert_eq!(
        criacao.corpo["power_level_content_override"]["users"][&agente_do_outro],
        json!(PODER_CRIACAO)
    );
    drop(guarda);
    let (_, sala) = outro.sala_unica();
    assert_eq!(
        sala.estado[&("m.room.power_levels".to_owned(), String::new())]["users"][&agente_do_outro],
        json!(80),
        "o poder configurado vale como poder final"
    );
}
