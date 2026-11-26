//! Resiliência e "cliente primeiro" do módulo Matrix (MX-29).
//!
//! O que se prova aqui, com banco de verdade em pasta temporária e um Synapse **falso** que
//! responde como o Synapse (usuários, salas, aliases, membros, estado, mensagens e deduplicação
//! de envio por `txnId`):
//! - **queda do Harness** (A5): cada "processo" do módulo roda num runtime próprio, numa thread.
//!   A queda é o fim desse runtime: todas as tarefas param no ponto em que estavam, sem código
//!   depois do `await`, como num processo morto. O banco é reaberto do arquivo na subida seguinte.
//!   Os pontos de queda são fixados pelo Synapse falso, que segura a resposta de uma chamada
//!   escolhida (criar sala, convite, envio ou leitura de perfil) até o teste derrubar o processo.
//! - **volta do Synapse** (A4): com o Synapse fora, as mensagens se acumulam; ao voltar, a sala
//!   recebe tudo, na ordem do banco, uma vez só.
//! - **mensagem venenosa**: é isolada e a fila segue.
//! - **cliente primeiro** (A1/A4): o cliente recebe a resposta mesmo com o Synapse fora ou lento.
//!   A comparação de números (módulo ligado × desligado) está em `medicao_cliente_primeiro`, que
//!   é `#[ignore]` e roda por `scripts/medir-resiliencia.sh`.
//!
//! Nada de rede externa. As quedas são simuladas (ver o limite no `docs/MATRIZ_RESILIENCIA.md`).

use std::collections::BTreeMap;
use std::fs;
use std::future::Future;
use std::net::{SocketAddr, TcpListener as StdListener};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axum::Json;
use axum::Router;
use axum::extract::State;
use axum::http::header::AUTHORIZATION;
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use poph_agent::atores::{Atores, OpcoesAtores};
use poph_agent::config_quente::{ConfigQuente, OpcoesConfigQuente};
use poph_agent::entrada_saida::{Fluxo, OpcoesFluxo};
use poph_agent::limites::{Limitador, OpcoesLimites};
use poph_agent::metricas::{MetricasFluxo, MetricasMemoria, MetricasTurno};
use poph_agent::turno::{Motor, OpcoesTurno, ProcessadorTurno};
use poph_core::llm::{FakeProvider, FinishReason, LlmEvent, LlmProvider, Script, Usage};
use poph_evolution::webhook::CABECALHO_SEGREDO;
use poph_evolution::{ClienteEvolution, ConfigEvolution, TETO_MIDIA};
use poph_matrix::cliente::{ClienteMatrix, ConfigCliente};
use poph_matrix::config::{ModuloMatrix, SegredosMatrix};
use poph_matrix::espacos::{DepartamentoEspaco, Espacos, LimitesConvite, Posicao, SalaParaEspaco};
use poph_matrix::usuarios::{Fantasma, IdsFantasmas};
use poph_server::Estado;
use poph_server::evolution::CanalEvolution;
use poph_server::matrix_inicio::{self, AjustesDeInicio, MatrixRodando, PortaEspacosStore};
use poph_server::{entrada_turno, saida_whatsapp};
use poph_store::{
    Direction, NewAgent, NewConversation, NewDepartment, NewMessage, NewOrg, Origin, OutboxState,
    Page, Role, StateRoom, Store,
};
use poph_types::{
    AgentId, ConversationId, DepartmentId, HarnessConfig, IntegrationId, MatrixConfig, OrgId,
    Secret,
};
use serde_json::{Value, json};
use tokio::sync::oneshot;

/// Token que o Harness apresenta ao Synapse.
const AS_TOKEN: &str = "CANARIO-AS-mx29-nao-pode-vazar";
/// Token que o Synapse apresenta ao Harness.
const HS_TOKEN: &str = "CANARIO-HS-mx29-nao-pode-vazar";
/// Domínio dos ids no teste.
const SERVIDOR: &str = "matrix-dev.local";
/// Prefixo dos usuários virtuais.
const PREFIXO: &str = "ph";
/// Segredo do webhook da instância de teste.
const SEGREDO_WEBHOOK: &str = "segredo-webhook-mx29";
/// Chave da Evolution falsa (só para o cabeçalho).
const CHAVE_EVO: &str = "chave-evo-mx29";
/// Instância de WhatsApp de teste.
const INSTANCIA: &str = "loja-mx29";
/// Telefone e JID do cliente de teste (não são de ninguém).
const TELEFONE: &str = "5511999990029";
const JID: &str = "5511999990029@s.whatsapp.net";

// ───────────────────────────── utilidades ─────────────────────────────

fn trava<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|veneno| veneno.into_inner())
}

/// Uma porta livre no loopback (a porta é liberada logo em seguida).
fn porta_livre() -> SocketAddr {
    let ouvinte = StdListener::bind("127.0.0.1:0").expect("porta livre");
    ouvinte.local_addr().expect("endereço")
}

/// Liga numa porta fixa. Tenta por um tempo, porque a porta acabou de ser solta pela queda.
fn ligar_porta(endereco: SocketAddr) -> StdListener {
    for _ in 0..80 {
        if let Ok(ouvinte) = StdListener::bind(endereco) {
            return ouvinte;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    panic!("não consegui religar a porta {endereco}");
}

/// Espera até a condição valer. Falha com o nome do que não aconteceu.
async fn esperar_ate(rotulo: &str, mut condicao: impl FnMut() -> bool) {
    let inicio = Instant::now();
    while inicio.elapsed() < Duration::from_secs(40) {
        if condicao() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("tempo esgotado esperando: {rotulo}");
}

/// Marcador curto e único de uma mensagem do teste. Colchetes evitam que `[m1]` case com `[m10]`.
fn marcador(i: usize) -> String {
    format!("[m{i:02}]")
}

/// Confere que cada marcador aparece uma vez só na sala e que a ordem é a dada.
fn conferir_ordem(textos: &[String], esperados: &[String]) -> Result<(), String> {
    let mut posicoes = Vec::with_capacity(esperados.len());
    for marca in esperados {
        let achados: Vec<usize> = textos
            .iter()
            .enumerate()
            .filter(|(_, texto)| texto.contains(marca.as_str()))
            .map(|(posicao, _)| posicao)
            .collect();
        match achados.as_slice() {
            [unico] => posicoes.push(*unico),
            _ => {
                return Err(format!(
                    "{marca} aparece {} vezes na sala (esperado 1)",
                    achados.len()
                ));
            }
        }
    }
    if posicoes.windows(2).all(|par| par[0] < par[1]) {
        Ok(())
    } else {
        Err("as mensagens da sala não estão na ordem do banco".to_owned())
    }
}

/// Mediana e percentil 95 de uma lista de milissegundos.
fn estatisticas(valores: &[f64]) -> (f64, f64, f64, f64) {
    let mut ordenados = valores.to_vec();
    ordenados.sort_by(|a, b| a.partial_cmp(b).expect("números comparáveis"));
    let n = ordenados.len();
    let mediana = if n % 2 == 1 {
        ordenados[n / 2]
    } else {
        (ordenados[n / 2 - 1] + ordenados[n / 2]) / 2.0
    };
    let indice_p95 = ((n as f64 * 0.95).ceil() as usize).clamp(1, n) - 1;
    let media = ordenados.iter().sum::<f64>() / n as f64;
    (mediana, ordenados[indice_p95], media, ordenados[n - 1])
}

// ───────────────────────────── processo simulado ─────────────────────────────

/// Um "processo" do Harness ou do Synapse: um runtime próprio numa thread.
///
/// Derrubar é o mesmo que matar o processo: quando a thread sai do `block_on`, o runtime é
/// desmontado e todas as tarefas dele são canceladas no ponto em que estavam (um `await`).
struct Hospedeiro {
    parar: Option<oneshot::Sender<()>>,
    fio: Option<std::thread::JoinHandle<()>>,
}

impl Hospedeiro {
    fn subir<F, Fut>(nome: &'static str, principal: F) -> Self
    where
        F: FnOnce(oneshot::Receiver<()>) -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let (parar, recebe) = oneshot::channel();
        let fio = std::thread::Builder::new()
            .name(nome.to_owned())
            .spawn(move || {
                let rt = tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(2)
                    .enable_all()
                    .build()
                    .expect("runtime do processo simulado");
                rt.block_on(principal(recebe));
                // `rt` cai aqui: as tarefas que ainda estavam no ar são canceladas.
            })
            .expect("thread do processo simulado");
        Self {
            parar: Some(parar),
            fio: Some(fio),
        }
    }

    /// Derruba o processo e espera a thread sair.
    fn derrubar(mut self) {
        self.encerrar();
    }

    fn encerrar(&mut self) {
        if let Some(parar) = self.parar.take() {
            let _ = parar.send(());
        }
        if let Some(fio) = self.fio.take() {
            let _ = fio.join();
        }
    }
}

impl Drop for Hospedeiro {
    fn drop(&mut self) {
        self.encerrar();
    }
}

// ───────────────────────────── Synapse falso ─────────────────────────────

#[derive(Default, Clone)]
struct SalaFalsa {
    membros: Vec<String>,
    convidados: Vec<String>,
    estado: BTreeMap<(String, String), Value>,
    /// `(txnId, tipo, corpo)` das mensagens aceitas (uma por `txnId`, como no Synapse).
    mensagens: Vec<(String, String, Value)>,
}

/// Chamadas que o teste pode segurar: o Synapse grava o efeito e não responde.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Alvo {
    CriarSala,
    Convite,
    Envio,
    Perfil,
}

/// Segura a `ocorrencia`-ésima chamada do tipo `alvo` (com `texto` no corpo, quando dado).
#[derive(Clone)]
struct Segurar {
    alvo: Alvo,
    texto: Option<String>,
    ocorrencia: u32,
    vistas: u32,
}

impl Segurar {
    fn novo(alvo: Alvo, ocorrencia: u32, texto: Option<&str>) -> Self {
        Self {
            alvo,
            texto: texto.map(str::to_owned),
            ocorrencia,
            vistas: 0,
        }
    }
}

#[derive(Default)]
struct EstadoFalso {
    usuarios: BTreeMap<String, Option<String>>,
    salas: BTreeMap<String, SalaFalsa>,
    aliases: BTreeMap<String, String>,
    proxima_sala: u32,
    /// Atraso antes de responder cada chamada (Synapse lento).
    atraso: Duration,
    segurar: Option<Segurar>,
    /// Liga quando uma chamada segurada chega.
    chegou: bool,
    /// Chamadas recebidas (antes do atraso) e respostas dadas.
    pedidos: u32,
    respondidos: u32,
    /// Convite de número `n` responde 503, uma vez só.
    falhar_convite: Option<u32>,
    convites_vistos: u32,
    /// Envio cujo corpo contém este texto recebe 400 (mensagem venenosa).
    rejeitar_texto: Option<String>,
    /// Todas as tentativas de envio, repetidas ou não: `(txnId, texto)`.
    envios_brutos: Vec<(String, String)>,
}

impl EstadoFalso {
    /// `true` quando esta chamada deve ficar sem resposta.
    fn segurar_se(&mut self, alvo: Alvo, detalhe: &str) -> bool {
        let Some(segurar) = self.segurar.as_mut() else {
            return false;
        };
        if segurar.alvo != alvo {
            return false;
        }
        if let Some(texto) = &segurar.texto
            && !detalhe.contains(texto.as_str())
        {
            return false;
        }
        segurar.vistas += 1;
        if segurar.vistas == segurar.ocorrencia {
            self.chegou = true;
            return true;
        }
        false
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
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(byte) =
                u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or(""), 16)
        {
            saida.push(byte);
            i += 3;
            continue;
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

enum Decisao {
    Responde(Response),
    /// A chamada fica sem resposta até o cliente (o Harness) cair.
    Segura,
}

fn executar(
    g: &mut EstadoFalso,
    metodo: &Method,
    uri: &Uri,
    cabecalhos: &HeaderMap,
    corpo: &Value,
) -> Decisao {
    let autorizacao = cabecalhos
        .get(AUTHORIZATION)
        .and_then(|valor| valor.to_str().ok())
        .unwrap_or_default();
    if autorizacao != format!("Bearer {AS_TOKEN}") {
        return Decisao::Responde(responder(
            401,
            json!({ "errcode": "M_UNKNOWN_TOKEN", "error": "Token inválido" }),
        ));
    }
    // A sonda de conexão do módulo fala com `/_matrix/client/versions` (sem `/v3`).
    if uri.path() == "/_matrix/client/versions" {
        return Decisao::Responde(responder(200, json!({ "versions": ["v1.11"] })));
    }

    let caminho = uri.path().trim_start_matches("/_matrix/client/v3");
    let partes: Vec<String> = caminho
        .split('/')
        .filter(|pedaco| !pedaco.is_empty())
        .map(decodificar)
        .collect();
    let quem = consulta(uri.query().unwrap_or_default(), "user_id").unwrap_or_default();
    let nao_existe = || {
        responder(
            404,
            json!({ "errcode": "M_NOT_FOUND", "error": "Não existe no Synapse falso" }),
        )
    };

    let resposta = match (metodo.as_str(), partes.as_slice()) {
        ("POST", [registro]) if registro == "register" => {
            let localpart = corpo["username"].as_str().unwrap_or_default();
            let id = format!("@{localpart}:{SERVIDOR}");
            if g.usuarios.contains_key(&id) {
                responder(
                    400,
                    json!({ "errcode": "M_USER_IN_USE", "error": "User ID already taken." }),
                )
            } else {
                g.usuarios.insert(id.clone(), None);
                responder(200, json!({ "user_id": id }))
            }
        }
        ("GET", [perfil, id]) if perfil == "profile" => {
            if g.segurar_se(Alvo::Perfil, id) {
                return Decisao::Segura;
            }
            match g.usuarios.get(id) {
                Some(nome) => {
                    let mut corpo = json!({});
                    if let Some(nome) = nome {
                        corpo["displayname"] = Value::String(nome.clone());
                    }
                    responder(200, corpo)
                }
                None => nao_existe(),
            }
        }
        ("PUT", [perfil, id, campo]) if perfil == "profile" && campo == "displayname" => {
            g.usuarios
                .insert(id.clone(), corpo["displayname"].as_str().map(str::to_owned));
            responder(200, json!({}))
        }
        ("POST", [criacao]) if criacao == "createRoom" => {
            let alias = corpo["room_alias_name"]
                .as_str()
                .map(|localpart| format!("#{localpart}:{SERVIDOR}"));
            if let Some(alias) = &alias
                && g.aliases.contains_key(alias)
            {
                return Decisao::Responde(responder(
                    400,
                    json!({ "errcode": "M_ROOM_IN_USE", "error": "Room alias already taken." }),
                ));
            }
            g.proxima_sala += 1;
            let sala = format!("!{}:{SERVIDOR}", g.proxima_sala);
            let membros = if quem.is_empty() {
                Vec::new()
            } else {
                vec![quem.clone()]
            };
            g.salas.insert(
                sala.clone(),
                SalaFalsa {
                    membros,
                    ..SalaFalsa::default()
                },
            );
            if let Some(alias) = alias {
                g.aliases.insert(alias.clone(), sala.clone());
                // O Synapse grava a sala e o alias antes de responder: a queda aqui deixa a sala
                // criada e sem resposta para o Harness.
                if g.segurar_se(Alvo::CriarSala, &alias) {
                    return Decisao::Segura;
                }
            }
            responder(200, json!({ "room_id": sala }))
        }
        ("GET", [diretorio, sala, alias]) if diretorio == "directory" && sala == "room" => {
            match g.aliases.get(alias) {
                Some(sala) => responder(200, json!({ "room_id": sala })),
                None => nao_existe(),
            }
        }
        ("POST", [salas, sala, convite]) if salas == "rooms" && convite == "invite" => {
            g.convites_vistos += 1;
            if g.falhar_convite == Some(g.convites_vistos) {
                g.falhar_convite = None;
                return Decisao::Responde(responder(
                    503,
                    json!({ "errcode": "M_UNKNOWN", "error": "indisponível" }),
                ));
            }
            match g.salas.get_mut(sala) {
                Some(alvo) => {
                    let convidado = corpo["user_id"].as_str().unwrap_or_default().to_owned();
                    if !alvo.convidados.contains(&convidado) {
                        alvo.convidados.push(convidado.clone());
                    }
                    if g.segurar_se(Alvo::Convite, &convidado) {
                        return Decisao::Segura;
                    }
                    responder(200, json!({}))
                }
                None => nao_existe(),
            }
        }
        ("POST", [entrar, alvo]) if entrar == "join" => {
            let sala = if alvo.starts_with('#') {
                g.aliases.get(alvo).cloned()
            } else {
                Some(alvo.clone())
            };
            match sala.and_then(|sala| g.salas.get_mut(&sala).map(|alvo| (sala, alvo))) {
                Some((sala, alvo)) => {
                    if !alvo.membros.contains(&quem) {
                        alvo.membros.push(quem.clone());
                    }
                    responder(200, json!({ "room_id": sala }))
                }
                None => nao_existe(),
            }
        }
        ("GET", [salas, sala, membros]) if salas == "rooms" && membros == "joined_members" => {
            match g.salas.get(sala) {
                Some(alvo) => {
                    let juntos: BTreeMap<String, Value> = alvo
                        .membros
                        .iter()
                        .map(|id| (id.clone(), json!({})))
                        .collect();
                    responder(200, json!({ "joined": juntos }))
                }
                None => nao_existe(),
            }
        }
        ("PUT", [salas, sala, estado, tipo, chave @ ..])
            if salas == "rooms" && estado == "state" =>
        {
            let chave = chave.join("/");
            match g.salas.get_mut(sala) {
                Some(alvo) => {
                    alvo.estado.insert((tipo.clone(), chave), corpo.clone());
                    responder(200, json!({ "event_id": "$estado" }))
                }
                None => nao_existe(),
            }
        }
        // Estado inteiro da sala (`GET /rooms/{id}/state`): a leitura que acha os espaços de onde a
        // sala é filha quando o registro da conversa está vazio (MX-51).
        ("GET", [salas, sala, estado]) if salas == "rooms" && estado == "state" => {
            match g.salas.get(sala) {
                Some(alvo) => {
                    let eventos: Vec<Value> = alvo
                    .estado
                    .iter()
                    .map(|((tipo, chave), corpo)| json!({ "type": tipo, "state_key": chave, "content": corpo }))
                    .collect();
                    responder(200, Value::Array(eventos))
                }
                None => nao_existe(),
            }
        }
        ("GET", [salas, sala, estado, tipo, chave @ ..])
            if salas == "rooms" && estado == "state" =>
        {
            let chave = chave.join("/");
            match g
                .salas
                .get(sala)
                .and_then(|alvo| alvo.estado.get(&(tipo.clone(), chave)))
            {
                Some(corpo) => responder(200, corpo.clone()),
                None => nao_existe(),
            }
        }
        ("PUT", [salas, sala, envio, tipo, txn]) if salas == "rooms" && envio == "send" => {
            let texto = corpo["body"]
                .as_str()
                .map_or_else(|| corpo.to_string(), str::to_owned);
            g.envios_brutos.push((txn.clone(), texto.clone()));
            if g.rejeitar_texto
                .as_deref()
                .is_some_and(|marca| texto.contains(marca))
            {
                return Decisao::Responde(responder(
                    400,
                    json!({ "errcode": "M_BAD_JSON", "error": "Corpo recusado" }),
                ));
            }
            match g.salas.get_mut(sala) {
                Some(alvo) => {
                    if !alvo.mensagens.iter().any(|(ja, _, _)| ja == txn) {
                        alvo.mensagens
                            .push((txn.clone(), tipo.clone(), corpo.clone()));
                    }
                    if g.segurar_se(Alvo::Envio, &texto) {
                        return Decisao::Segura;
                    }
                    responder(200, json!({ "event_id": format!("$msg_{txn}") }))
                }
                None => nao_existe(),
            }
        }
        _ => responder(
            404,
            json!({ "errcode": "M_UNRECOGNIZED", "error": "Rota não existe no Synapse falso" }),
        ),
    };
    Decisao::Responde(resposta)
}

async fn atender(
    State(estado): State<Arc<Mutex<EstadoFalso>>>,
    metodo: Method,
    uri: Uri,
    cabecalhos: HeaderMap,
    corpo: String,
) -> Response {
    let corpo: Value = serde_json::from_str(&corpo).unwrap_or(Value::Null);
    // A sonda de conexão do módulo não conta nem sofre o atraso: só as chamadas do espelho e do
    // estado contam como o Synapse "lento" ou "ocupado" do cenário.
    let sonda = uri.path() == "/_matrix/client/versions";
    let atraso = {
        let mut g = trava(&estado);
        if !sonda {
            g.pedidos += 1;
        }
        g.atraso
    };
    if !atraso.is_zero() && !sonda {
        tokio::time::sleep(atraso).await;
    }
    let decisao = {
        let mut g = trava(&estado);
        executar(&mut g, &metodo, &uri, &cabecalhos, &corpo)
    };
    match decisao {
        Decisao::Segura => {
            std::future::pending::<()>().await;
            unreachable!("a chamada segurada nunca tem resposta")
        }
        Decisao::Responde(resposta) => {
            if !sonda {
                trava(&estado).respondidos += 1;
            }
            resposta
        }
    }
}

async fn principal_synapse(
    ouvinte: StdListener,
    estado: Arc<Mutex<EstadoFalso>>,
    recebe: oneshot::Receiver<()>,
) {
    ouvinte.set_nonblocking(true).expect("modo não bloqueante");
    let ouvinte = tokio::net::TcpListener::from_std(ouvinte).expect("ouvinte do Synapse falso");
    let app = Router::new().fallback(atender).with_state(estado);
    tokio::spawn(async move {
        let _ = axum::serve(ouvinte, app).await;
    });
    let _ = recebe.await;
}

/// Synapse falso num endereço fixo. Pode ser derrubado e religado nele (o estado é mantido,
/// como o do Synapse real depois de reiniciar).
struct Synapse {
    endereco: String,
    addr: SocketAddr,
    estado: Arc<Mutex<EstadoFalso>>,
    processo: Option<Hospedeiro>,
}

impl Synapse {
    /// Ainda sem subir: o endereço existe, a porta está fechada (Synapse fora).
    fn em(addr: SocketAddr) -> Self {
        Self {
            endereco: format!("http://{addr}"),
            addr,
            estado: Arc::new(Mutex::new(EstadoFalso::default())),
            processo: None,
        }
    }

    /// Sobe já no ar, numa porta livre.
    fn no_ar() -> Self {
        let mut synapse = Self::em(porta_livre());
        synapse.subir();
        synapse
    }

    fn subir(&mut self) {
        if self.processo.is_some() {
            return;
        }
        let ouvinte = ligar_porta(self.addr);
        let estado = Arc::clone(&self.estado);
        self.processo = Some(Hospedeiro::subir("synapse-falso", move |recebe| {
            principal_synapse(ouvinte, estado, recebe)
        }));
    }

    fn derrubar(&mut self) {
        if let Some(mut processo) = self.processo.take() {
            processo.encerrar();
        }
    }

    fn configurar<R>(&self, f: impl FnOnce(&mut EstadoFalso) -> R) -> R {
        f(&mut trava(&self.estado))
    }

    fn ver<R>(&self, f: impl FnOnce(&EstadoFalso) -> R) -> R {
        f(&trava(&self.estado))
    }

    /// Cria uma sala já com membros e estado (para o cenário de espaço).
    fn semear_sala(&self, id: &str, membros: &[&str], estado: Vec<((&str, &str), Value)>) {
        self.configurar(|g| {
            let mut sala = SalaFalsa {
                membros: membros.iter().map(|m| (*m).to_owned()).collect(),
                ..SalaFalsa::default()
            };
            for ((tipo, chave), valor) in estado {
                sala.estado
                    .insert((tipo.to_owned(), chave.to_owned()), valor);
            }
            g.salas.insert(id.to_owned(), sala);
        });
    }
}

impl Drop for Synapse {
    fn drop(&mut self) {
        self.derrubar();
    }
}

/// A única sala do Synapse falso (`None` quando há zero ou mais de uma).
fn sala_unica(sinapse: &Synapse) -> Option<String> {
    sinapse.ver(|g| {
        if g.salas.len() == 1 {
            g.salas.keys().next().cloned()
        } else {
            None
        }
    })
}

/// Textos (corpo em JSON) das mensagens de uma sala, na ordem em que foram aceitas.
fn textos_da_sala(sinapse: &Synapse, sala: &str) -> Vec<String> {
    sinapse.ver(|g| {
        g.salas
            .get(sala)
            .map(|alvo| {
                alvo.mensagens
                    .iter()
                    .map(|(_, _, corpo)| corpo.to_string())
                    .collect()
            })
            .unwrap_or_default()
    })
}

// ───────────────────────────── banco e configuração ─────────────────────────────

/// Banco temporário com org, agente e conversa, e a conexão do próprio teste.
struct Base {
    pasta: PathBuf,
    loja: Arc<Store>,
    org: OrgId,
    agente: AgentId,
    conversa: ConversationId,
}

impl Base {
    fn novo(rotulo: &str) -> Self {
        let carimbo = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duracao| duracao.as_nanos())
            .unwrap_or(0);
        let pasta = std::env::temp_dir().join(format!(
            "poph-mx29-{rotulo}-{}-{carimbo}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&pasta);
        fs::create_dir_all(&pasta).expect("pasta do teste");
        let loja = Arc::new(
            Store::open_at(&pasta.join("harness.db"), &pasta.join("rag.db")).expect("banco"),
        );
        let org = loja
            .orgs()
            .create(NewOrg {
                name: format!("Org {rotulo}"),
                slug: format!("org-{rotulo}"),
                config_json: String::new(),
            })
            .expect("cria a org")
            .id;
        let agente = loja
            .agents()
            .create(
                org,
                NewAgent {
                    name: "Bia".to_owned(),
                    slug: "bia".to_owned(),
                    // Sem janela de agrupamento: o teste não espera o cliente terminar de digitar.
                    grouping_window_ms: 0,
                    ..NewAgent::default()
                },
            )
            .expect("cria o agente")
            .id;
        let conversa = loja
            .conversations()
            .create(
                org,
                NewConversation {
                    agent_id: agente,
                    integration_id: None,
                    canal: "whatsapp".to_owned(),
                    peer_externo: Some(TELEFONE.to_owned()),
                    target_department: None,
                },
            )
            .expect("cria a conversa")
            .id;
        Self {
            pasta,
            loja,
            org,
            agente,
            conversa,
        }
    }

    fn db(&self) -> PathBuf {
        self.pasta.join("harness.db")
    }

    fn rag(&self) -> PathBuf {
        self.pasta.join("rag.db")
    }

    /// Liga o módulo nesta org (o corte do backfill fica em agora).
    fn ligar(&self) {
        self.loja.matrix().enable(self.org).expect("liga o módulo");
    }

    fn gravar(&self, direcao: Direction, papel: Role, texto: &str) -> String {
        self.loja
            .messages()
            .write(
                self.org,
                NewMessage {
                    conversation_id: self.conversa,
                    direction: direcao,
                    role: papel,
                    origin: Origin::Agente,
                    author_name: None,
                    texto: texto.to_owned(),
                    attachments_json: "[]".to_owned(),
                    external_id: None,
                    turn_id: None,
                },
            )
            .expect("grava a mensagem")
            .id
            .to_string()
    }

    /// Mensagem do cliente (entrada, papel `user`).
    fn cliente(&self, texto: &str) -> String {
        self.gravar(Direction::Inbound, Role::User, texto)
    }

    /// Resposta do agente (saída, papel `assistant`).
    fn agente_fala(&self, texto: &str) -> String {
        self.gravar(Direction::Outbound, Role::Assistant, texto)
    }
}

impl Drop for Base {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.pasta);
    }
}

fn config_matrix(url: &str, intervalo_ms: u64) -> MatrixConfig {
    MatrixConfig {
        enabled: true,
        homeserver_url: url.to_owned(),
        server_name: SERVIDOR.to_owned(),
        prefix: PREFIXO.to_owned(),
        concurrency: 4,
        interval_ms: intervalo_ms,
        ..MatrixConfig::default()
    }
}

fn modulo_de(config: &MatrixConfig) -> ModuloMatrix {
    ModuloMatrix {
        config: config.clone(),
        segredos: SegredosMatrix {
            as_token: Secret::new(AS_TOKEN.to_owned()),
            hs_token: Secret::new(HS_TOKEN.to_owned()),
        },
    }
}

/// Sonda curta para os testes (o padrão de produção espera 30 s entre sondas).
fn ajustes_de_teste() -> AjustesDeInicio {
    AjustesDeInicio {
        periodo_da_sonda: Duration::from_millis(100),
        teto_da_sonda: Duration::from_millis(200),
        prazo_de_parada: Duration::from_secs(1),
    }
}

fn cliente_matrix(url: &str) -> ClienteMatrix {
    ClienteMatrix::novo(
        ConfigCliente::novo(url, SERVIDOR, Secret::new(AS_TOKEN.to_owned()))
            .expect("config do cliente"),
    )
    .expect("cliente Matrix")
}

// ───────────────────────────── processos do módulo ─────────────────────────────

/// O módulo (espelho de mensagens e de estado, sala, espaço, aviso de falha) num processo próprio.
/// Derrubar o processo é a queda do Harness; subir de novo lê o banco do arquivo.
fn processo_do_modulo(base: &Base, config: MatrixConfig) -> Hospedeiro {
    let db = base.db();
    let rag = base.rag();
    Hospedeiro::subir("harness-modulo", move |recebe| async move {
        let store = Arc::new(Store::open_at(&db, &rag).expect("abre o banco no processo"));
        let harness = HarnessConfig {
            matrix: Some(config.clone()),
            ..HarnessConfig::default()
        };
        let mut estado = Estado::novo(store, harness).expect("estado do servidor");
        let _rodando =
            matrix_inicio::iniciar_com_ajustes(&mut estado, modulo_de(&config), ajustes_de_teste());
        let _ = recebe.await;
    })
}

/// O processo do Harness que recebe as transações do Synapse (receptor do Application Service),
/// com o módulo ligado. Numa porta fixa, para o teste "Synapse" religar no mesmo endereço.
fn processo_do_receptor(base: &Base, config: MatrixConfig, ouvinte: StdListener) -> Hospedeiro {
    let db = base.db();
    let rag = base.rag();
    Hospedeiro::subir("harness-receptor", move |recebe| async move {
        ouvinte.set_nonblocking(true).expect("modo não bloqueante");
        let ouvinte = tokio::net::TcpListener::from_std(ouvinte).expect("ouvinte do Harness");
        let store = Arc::new(Store::open_at(&db, &rag).expect("abre o banco no processo"));
        let harness = HarnessConfig {
            matrix: Some(config.clone()),
            ..HarnessConfig::default()
        };
        let mut estado = Estado::novo(store, harness).expect("estado do servidor");
        let _rodando =
            matrix_inicio::iniciar_com_ajustes(&mut estado, modulo_de(&config), ajustes_de_teste());
        let app = poph_server::router(estado);
        tokio::spawn(async move {
            let _ = axum::serve(ouvinte, app).await;
        });
        let _ = recebe.await;
    })
}

// ───────────────────────────── servidor com turno e WhatsApp ─────────────────────────────
//
// Para o teste "cliente primeiro": o servidor de verdade (webhook, turno com provedor falso,
// saída pela Evolution falsa), com ou sem o módulo Matrix ligado.

/// Evolution falsa que anota a hora de chegada de cada envio.
#[derive(Clone, Default)]
struct EvolucaoFalsa {
    chegadas: Arc<Mutex<Vec<Instant>>>,
}

async fn enviar_falso(
    State(evo): State<EvolucaoFalsa>,
    Json(_corpo): Json<Value>,
) -> (StatusCode, Json<Value>) {
    trava(&evo.chegadas).push(Instant::now());
    let n = trava(&evo.chegadas).len();
    (
        StatusCode::OK,
        Json(json!({ "key": { "id": format!("MSG-SAIDA-{n}") } })),
    )
}

async fn subir_evolucao(evo: EvolucaoFalsa) -> String {
    let roteador = Router::new()
        .route("/message/sendText/{instancia}", post(enviar_falso))
        .route("/message/sendMedia/{instancia}", post(enviar_falso))
        .with_state(evo);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("porta da Evolution falsa");
    let endereco = listener.local_addr().expect("endereço");
    tokio::spawn(async move {
        let _ = axum::serve(listener, roteador).await;
    });
    format!("http://{endereco}")
}

fn resposta_do_agente(i: usize) -> Script {
    Script::Events(vec![
        LlmEvent::Texto(format!("resposta do agente {}", marcador(i))),
        LlmEvent::Usage(Usage {
            input: 120,
            saida: 30,
        }),
        LlmEvent::Fim(FinishReason::Completed),
    ])
}

/// Servidor no ar: webhook, turno, saída e (opcionalmente) o módulo Matrix.
struct Servidor {
    /// Provedor de LLM falso do turno: conta as chamadas feitas por ele.
    llm: Arc<FakeProvider>,
    base_url: String,
    http: reqwest::Client,
    org: OrgId,
    evo: EvolucaoFalsa,
    desligar: Option<oneshot::Sender<()>>,
    tarefas: Vec<tokio::task::JoinHandle<()>>,
    modulo: Option<MatrixRodando>,
}

impl Servidor {
    async fn webhook(&self, id: &str, texto: &str) -> StatusCode {
        let corpo = json!({
            "event": "messages.upsert",
            "instance": INSTANCIA,
            "apikey": CHAVE_EVO,
            "destination": "http://harness.teste/webhook/evolution/x/loja",
            "data": {
                "key": {"id": id, "remoteJid": JID, "fromMe": false},
                "pushName": "Cliente Teste",
                "messageType": "conversation",
                "message": {"conversation": texto},
                "messageTimestamp": 1795177000
            }
        });
        let url = format!(
            "{}/webhook/evolution/{}/{INSTANCIA}",
            self.base_url, self.org
        );
        self.http
            .post(url)
            .header(CABECALHO_SEGREDO, SEGREDO_WEBHOOK)
            .json(&corpo)
            .send()
            .await
            .expect("entrega o webhook")
            .status()
    }

    /// Quantas vezes o turno chamou o provedor de LLM (falso) até agora.
    fn chamadas_llm(&self) -> usize {
        self.llm.received().len()
    }

    fn chegadas(&self) -> Vec<Instant> {
        trava(&self.evo.chegadas).clone()
    }

    async fn encerrar(mut self) {
        if let Some(desligar) = self.desligar.take() {
            let _ = desligar.send(());
        }
        for tarefa in self.tarefas.drain(..) {
            tarefa.abort();
        }
        if let Some(modulo) = self.modulo.take() {
            modulo.parar().await;
        }
    }
}

/// Sobe o servidor com o turno (provedor falso com `respostas` roteiros) e, se `config` for
/// `Some`, o módulo Matrix com `ajustes`. A org do `base` precisa estar ligada antes, se houver
/// módulo.
async fn subir_servidor(
    base: &Base,
    config: Option<MatrixConfig>,
    ajustes: AjustesDeInicio,
    respostas: usize,
) -> Servidor {
    let store = Arc::new(Store::open_at(&base.db(), &base.rag()).expect("banco do servidor"));
    let evo = EvolucaoFalsa::default();
    let url_evo = subir_evolucao(evo.clone()).await;
    let integracao: IntegrationId = store
        .create_integration(
            base.org,
            base.agente,
            "whatsapp".to_owned(),
            "active".to_owned(),
        )
        .expect("cria a integração")
        .id
        .parse()
        .expect("id da integração");
    let cliente_evo = ClienteEvolution::novo(
        ConfigEvolution::novo(url_evo, INSTANCIA, Secret::new(CHAVE_EVO.to_owned()))
            .expect("config da Evolution falsa"),
    )
    .expect("cliente da Evolution falsa");
    let canal = CanalEvolution {
        org: base.org,
        instancia: INSTANCIA.to_owned(),
        integracao,
        segredo: Some(Secret::new(SEGREDO_WEBHOOK.to_owned())),
        cliente: Some(cliente_evo),
        teto_midia: TETO_MIDIA,
    };
    store
        .instructions()
        .save(
            base.org,
            base.agente,
            0,
            "{}",
            "# Atendente\n\nVocê atende a loja.",
            true,
        )
        .expect("grava as instruções");

    let harness = HarnessConfig {
        matrix: config.clone(),
        ..HarnessConfig::default()
    };
    let estado = Estado::novo(Arc::clone(&store), harness)
        .expect("estado do servidor")
        .com_canais_evolution(vec![canal]);

    // Turno com provedor falso, como em `turno_whatsapp`.
    let roteiros: Vec<Script> = (0..respostas).map(resposta_do_agente).collect();
    let fake = Arc::new(FakeProvider::with_scripts(roteiros));
    let provedor: Arc<dyn LlmProvider> = fake.clone();
    let registro = &estado.registro;
    let config_quente = ConfigQuente::nova(
        Arc::clone(&store),
        OpcoesConfigQuente::default(),
        MetricasMemoria::proprio(),
    );
    let limitador = Arc::new(Limitador::novo(
        OpcoesLimites::default(),
        MetricasMemoria::proprio(),
    ));
    let motor = Motor::novo(
        Arc::clone(&store),
        config_quente,
        limitador,
        provedor,
        // Janela de agrupamento de 1 ms em vez dos 2 s padrão: o piso de 2 s não é o que se mede
        // aqui. Vale igual para os quatro cenários da medição.
        OpcoesTurno {
            janela_padrao: Duration::from_millis(1),
            ..OpcoesTurno::default()
        },
        MetricasTurno::registrar(registro).expect("métricas do turno"),
    );
    let processador = Arc::new(ProcessadorTurno::novo(Arc::new(motor)));
    let atores = Atores::novo(
        processador.clone(),
        OpcoesAtores::default(),
        MetricasMemoria::proprio(),
    );
    let fluxo = Fluxo::com_metricas(
        Arc::clone(&store),
        atores,
        OpcoesFluxo::default(),
        MetricasFluxo::registrar(registro).expect("métricas da fila"),
    );
    processador.ligar_fluxo(fluxo.clone());
    let mut estado = estado.com_turno(fluxo, Duration::from_millis(20));

    let modulo = match config {
        Some(config) => {
            matrix_inicio::iniciar_com_ajustes(&mut estado, modulo_de(&config), ajustes)
        }
        None => None,
    };
    let mut tarefas = Vec::new();
    if let Some(saida) = saida_whatsapp::iniciar(&estado) {
        tarefas.push(saida);
    }
    if let Some(turno) = entrada_turno::iniciar(&estado) {
        tarefas.push(turno);
    }

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("porta do servidor");
    let endereco = listener.local_addr().expect("endereço");
    let (desligar, sinal) = oneshot::channel::<()>();
    let app = poph_server::router(estado);
    tarefas.push(tokio::spawn(async move {
        let _ = axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = sinal.await;
            })
            .await;
    }));

    Servidor {
        llm: fake,
        base_url: format!("http://{endereco}"),
        http: reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .build()
            .expect("cliente HTTP"),
        org: base.org,
        evo,
        desligar: Some(desligar),
        tarefas,
        modulo,
    }
}

/// Espera o envio número `i` (0-based) chegar à Evolution falsa e devolve a hora da chegada.
async fn esperar_envio(servidor: &Servidor, i: usize) -> Instant {
    let inicio = Instant::now();
    loop {
        if let Some(chegada) = servidor.chegadas().get(i) {
            return *chegada;
        }
        if inicio.elapsed() > Duration::from_secs(30) {
            panic!("o envio {} não chegou à Evolution falsa", i + 1);
        }
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
}

// ───────────────────────────── cliente primeiro ─────────────────────────────

/// Sem Synapse: o cliente recebe a resposta e o módulo, ligado, não tira esse caminho.
#[tokio::test]
async fn cliente_recebe_resposta_com_synapse_fora_e_modulo_ligado() {
    let base = Base::novo("fora");
    base.ligar();
    let url_fechada = format!("http://{}", porta_livre());
    let config = config_matrix(&url_fechada, 50);
    let servidor = subir_servidor(&base, Some(config), ajustes_de_teste(), 8).await;

    for i in 0..8 {
        let status = servidor
            .webhook(
                &format!("MSG-FORA-{i}"),
                &format!("pergunta {}", marcador(i)),
            )
            .await;
        assert_eq!(status, StatusCode::OK);
        let _ = esperar_envio(&servidor, i).await;
    }
    assert_eq!(
        servidor.chegadas().len(),
        8,
        "com o Synapse fora, as 8 respostas saem para o cliente"
    );
    servidor.encerrar().await;
}

/// Synapse lento (5 s por chamada): a resposta sai antes de o Synapse responder a nada.
///
/// Prova por ordem, sem medir tempo: quando a segunda resposta chega à Evolution, nenhuma chamada
/// que o módulo fez ao Synapse já foi respondida. Com 5 s de atraso, o envio tem de chegar antes.
#[tokio::test]
async fn resposta_sai_antes_de_o_synapse_lento_responder() {
    let synapse = Synapse::no_ar();
    synapse.configurar(|g| g.atraso = Duration::from_secs(5));
    let base = Base::novo("lento");
    base.ligar();
    let config = config_matrix(&synapse.endereco, 50);
    let servidor = subir_servidor(&base, Some(config), ajustes_de_teste(), 2).await;

    assert_eq!(
        servidor.webhook("MSG-LENTO-1", "primeira").await,
        StatusCode::OK
    );
    let _ = esperar_envio(&servidor, 0).await;
    // O módulo já está com uma chamada ao Synapse em andamento (a primeira mensagem espelhada).
    esperar_ate("o módulo chamar o Synapse lento", || {
        synapse.ver(|g| g.pedidos >= 1)
    })
    .await;

    assert_eq!(
        servidor.webhook("MSG-LENTO-2", "segunda").await,
        StatusCode::OK
    );
    let _ = esperar_envio(&servidor, 1).await;
    let respondidos = synapse.ver(|g| g.respondidos);
    assert_eq!(
        respondidos, 0,
        "a segunda resposta saiu antes de qualquer chamada lenta ao Synapse terminar"
    );
    servidor.encerrar().await;
}

// ───────────────────────────── volta do Synapse (A4) ─────────────────────────────

/// Synapse fora enquanto as mensagens chegam: nada se perde e, ao voltar, a sala recebe tudo na
/// ordem do banco, uma vez só.
#[tokio::test]
async fn volta_do_synapse_esgota_a_fila_em_ordem_sem_duplicata() {
    let mut synapse = Synapse::em(porta_livre());
    let base = Base::novo("volta");
    base.ligar();
    // Sobe o módulo com o Synapse fora (a porta está fechada). Só depois as mensagens chegam.
    let processo = processo_do_modulo(&base, config_matrix(&synapse.endereco, 50));

    let mut esperados = Vec::new();
    for i in 0..5 {
        base.cliente(&format!("cliente {}", marcador(2 * i)));
        base.agente_fala(&format!("agente {}", marcador(2 * i + 1)));
        esperados.push(marcador(2 * i));
        esperados.push(marcador(2 * i + 1));
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
    // Synapse fora por um tempo: os ciclos falham e a fila espera.
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(
        base.loja
            .matrix()
            .read_cursors(base.org)
            .expect("cursores")
            .messages,
        0,
        "com o Synapse fora, o cursor não anda"
    );

    synapse.subir();
    esperar_ate("a sala receber as mensagens", || {
        sala_unica(&synapse).is_some_and(|sala| {
            conferir_ordem(&textos_da_sala(&synapse, &sala), &esperados).is_ok()
        })
    })
    .await;

    let sala = sala_unica(&synapse).expect("uma sala só");
    let textos = textos_da_sala(&synapse, &sala);
    conferir_ordem(&textos, &esperados).expect("ordem e uma vez só");
    // O Synapse recebeu tentativas repetidas (ciclos com falha), mas aceitou cada txnId uma vez.
    let brutos = synapse.ver(|g| g.envios_brutos.len());
    assert!(
        brutos >= esperados.len(),
        "cada mensagem foi tentada pelo menos uma vez"
    );
    processo.derrubar();
    drop(synapse);
}

// ───────────────────────────── mensagem venenosa ─────────────────────────────

/// Uma mensagem que o Synapse recusa de vez é isolada, e as seguintes saem.
#[tokio::test]
async fn mensagem_venenosa_e_isolada_e_a_fila_segue() {
    let synapse = Synapse::no_ar();
    synapse.configurar(|g| g.rejeitar_texto = Some("VENENO".to_owned()));
    let base = Base::novo("veneno");
    base.ligar();

    let mut esperados = Vec::new();
    base.cliente(&format!("primeira {}", marcador(0)));
    esperados.push(marcador(0));
    base.cliente(&format!("VENENO {}", marcador(1)));
    base.cliente(&format!("depois {}", marcador(2)));
    esperados.push(marcador(2));
    base.agente_fala(&format!("resposta {}", marcador(3)));
    esperados.push(marcador(3));

    let processo = processo_do_modulo(&base, config_matrix(&synapse.endereco, 50));
    esperar_ate(
        "as mensagens boas chegarem e a venenosa for isolada",
        || {
            let falhas = base.loja.matrix().count_failures(base.org).unwrap_or(0);
            if falhas != 1 {
                return false;
            }
            sala_unica(&synapse).is_some_and(|sala| {
                conferir_ordem(&textos_da_sala(&synapse, &sala), &esperados).is_ok()
            })
        },
    )
    .await;

    let sala = sala_unica(&synapse).expect("uma sala");
    let textos = textos_da_sala(&synapse, &sala);
    assert!(
        textos.iter().all(|texto| !texto.contains("VENENO")),
        "a mensagem venenosa não entrou na sala"
    );
    conferir_ordem(&textos, &esperados).expect("as boas saem na ordem");

    let falhas = base
        .loja
        .matrix()
        .list_failures(base.org, 10)
        .expect("falhas");
    assert_eq!(falhas.len(), 1, "uma mensagem isolada");
    let motivo = falhas[0].motivo.clone();
    assert!(
        !motivo.contains("VENENO") && !motivo.contains("depois"),
        "o motivo isolado é código curto, sem texto de cliente"
    );
    processo.derrubar();
    drop(synapse);
}

// ───────────────────────────── queda do Harness (A5) ─────────────────────────────

/// Queda no meio da criação da sala: o Synapse criou a sala e não respondeu. Ao subir, o módulo
/// acha a sala pelo alias, completa a linha e não cria outra.
#[tokio::test]
async fn queda_na_criacao_de_sala_nao_duplica_a_sala() {
    let synapse = Synapse::no_ar();
    let base = Base::novo("c1");
    base.ligar();
    let config = config_matrix(&synapse.endereco, 50);
    let esperados: Vec<String> = (0..3).map(marcador).collect();
    base.cliente(&format!("primeira {}", marcador(0)));
    base.agente_fala(&format!("resposta {}", marcador(1)));
    base.cliente(&format!("segunda {}", marcador(2)));

    synapse.configurar(|g| g.segurar = Some(Segurar::novo(Alvo::CriarSala, 1, None)));
    let primeiro = processo_do_modulo(&base, config.clone());
    esperar_ate("o módulo pedir a criação da sala", || {
        synapse.ver(|g| g.chegou)
    })
    .await;
    primeiro.derrubar();

    // No meio da queda: uma sala no Synapse, a linha ainda não está pronta.
    assert_eq!(
        synapse.ver(|g| g.salas.len()),
        1,
        "o Synapse criou uma sala"
    );
    let linha = base
        .loja
        .matrix()
        .room(base.org, base.conversa)
        .expect("linha");
    assert!(
        linha.is_none_or(|sala| !matches!(sala.state, StateRoom::Ready)),
        "a linha ainda não estava pronta na queda"
    );

    synapse.configurar(|g| g.segurar = None);
    let segundo = processo_do_modulo(&base, config);
    esperar_ate("as três mensagens na sala", || {
        sala_unica(&synapse).is_some_and(|sala| {
            conferir_ordem(&textos_da_sala(&synapse, &sala), &esperados).is_ok()
        })
    })
    .await;
    segundo.derrubar();

    let salas = synapse.ver(|g| g.salas.len());
    assert_eq!(salas, 1, "uma sala só depois da retomada");
    let aliases = synapse.ver(|g| g.aliases.len());
    assert_eq!(aliases, 1, "um alias só");
    let sala = sala_unica(&synapse).expect("sala");
    let gravada = base
        .loja
        .matrix()
        .room(base.org, base.conversa)
        .expect("linha")
        .expect("a conversa tem sala");
    assert_eq!(
        gravada.room_id, sala,
        "a linha aponta para a sala do Synapse"
    );
    assert!(
        matches!(gravada.state, StateRoom::Ready),
        "a linha termina pronta"
    );
    drop(synapse);
}

/// Queda no meio do envio do espelho: o Synapse aceitou a mensagem e não respondeu. Ao subir, o
/// módulo reenvia com o mesmo `txnId`, e a sala não recebe a mensagem duas vezes.
#[tokio::test]
async fn queda_no_envio_do_espelho_nao_duplica_a_mensagem() {
    let synapse = Synapse::no_ar();
    let base = Base::novo("c3");
    base.ligar();
    let config = config_matrix(&synapse.endereco, 50);
    base.cliente(&format!("inicial {}", marcador(0)));
    base.agente_fala(&format!("resposta {}", marcador(1)));

    // Primeiro, o módulo espelha o que já existe e para (queda limpa).
    let primeiro = processo_do_modulo(&base, config.clone());
    esperar_ate("duas mensagens iniciais na sala", || {
        sala_unica(&synapse).is_some_and(|sala| textos_da_sala(&synapse, &sala).len() == 2)
    })
    .await;
    primeiro.derrubar();

    // Chega uma mensagem nova; a próxima tentativa de envio dela cai no meio.
    base.cliente(&format!("nova {}", marcador(2)));
    synapse.configurar(|g| g.segurar = Some(Segurar::novo(Alvo::Envio, 1, Some("[m02]"))));
    let segundo = processo_do_modulo(&base, config.clone());
    esperar_ate("o envio da mensagem nova chegar ao Synapse", || {
        synapse.ver(|g| g.chegou)
    })
    .await;
    segundo.derrubar();

    synapse.configurar(|g| g.segurar = None);
    let terceiro = processo_do_modulo(&base, config);
    let esperados: Vec<String> = (0..3).map(marcador).collect();
    esperar_ate("a mensagem nova reenviada e na sala", || {
        let reenvios = synapse.ver(|g| {
            g.envios_brutos
                .iter()
                .filter(|(_, texto)| texto.contains("[m02]"))
                .count()
        });
        reenvios >= 2
            && sala_unica(&synapse).is_some_and(|sala| {
                conferir_ordem(&textos_da_sala(&synapse, &sala), &esperados).is_ok()
            })
    })
    .await;
    terceiro.derrubar();

    let tentativas = synapse.ver(|g| {
        g.envios_brutos
            .iter()
            .filter(|(_, texto)| texto.contains("[m02]"))
            .count()
    });
    assert!(
        tentativas >= 2,
        "a mensagem foi reenviada depois da queda ({tentativas} tentativas)"
    );
    let sala = sala_unica(&synapse).expect("sala");
    let textos = textos_da_sala(&synapse, &sala);
    conferir_ordem(&textos, &esperados).expect("uma vez só, na ordem");
    drop(synapse);
}

/// Queda no meio do lote de convites de um espaço: o `posicionar` roda num processo próprio e
/// cai com o terceiro convite no ar. Na passagem seguinte, a sala acaba posicionada e todos os
/// humanos do espaço ficam convidados, sem ninguém de fora.
#[tokio::test]
async fn queda_no_meio_do_lote_de_convites_termina_sem_perda() {
    let synapse = Synapse::no_ar();
    let config = config_matrix(&synapse.endereco, 50);
    let base = Base::novo("c2");
    base.ligar();
    let dados = montar_espaco(&base, &synapse, &config);

    synapse.configurar(|g| g.segurar = Some(Segurar::novo(Alvo::Convite, 3, None)));
    let do_processo = dados.clone();
    let db = base.db();
    let rag = base.rag();
    // A queda interrompe o trabalho em qualquer `await` (o convite segurado, aqui). Sem o
    // `select`, o processo ficaria preso no convite até o cliente HTTP estourar o prazo.
    let primeiro = Hospedeiro::subir("posicionar", move |recebe| async move {
        let store = Arc::new(Store::open_at(&db, &rag).expect("banco do processo"));
        tokio::select! {
            _ = posicionar_com(&do_processo, store, limites_de_lote()) => {}
            _ = recebe => {}
        }
    });
    esperar_ate("o terceiro convite chegar ao Synapse", || {
        synapse.ver(|g| g.chegou)
    })
    .await;
    primeiro.derrubar();

    synapse.configurar(|g| g.segurar = None);
    let resultado = posicionar_com(&dados, Arc::clone(&base.loja), limites_de_lote())
        .await
        .expect("passagem seguinte");
    assert!(
        matches!(resultado, Posicao::Posicionada { .. }),
        "a passagem seguinte põe a sala no espaço: {resultado:?}"
    );
    let convidados = convidados_da_sala(&synapse, &dados.sala);
    for humano in &dados.humanos {
        assert!(
            convidados.contains(humano),
            "{humano} ficou sem convite depois da queda"
        );
    }
    assert_eq!(
        convidados.len(),
        dados.humanos.len(),
        "nenhum convite fora do espaço"
    );
    drop(synapse);
}

/// Queda antes do registro da resposta humana (o perfil do atendente ainda estava sendo lido).
/// Na subida, a transação chega de novo e é gravada uma vez só.
#[tokio::test]
async fn queda_antes_do_registro_da_resposta_grava_uma_vez() {
    let synapse = Synapse::no_ar();
    let base = Base::novo("c4a");
    base.ligar();
    let config = config_matrix(&synapse.endereco, 50);
    let (_conversa, sala) = sala_pronta_para_resposta(&base);
    let porta = porta_livre();
    let ouvinte = ligar_porta(porta);
    let corpo = transacao_da_resposta(&sala, "$evento-c4a", "olá, o pedido saiu hoje");

    synapse.configurar(|g| g.segurar = Some(Segurar::novo(Alvo::Perfil, 1, None)));
    let primeiro = processo_do_receptor(&base, config.clone(), ouvinte);
    let envio = tokio::spawn(enviar_transacao(porta, "TXN-C4A".to_owned(), corpo.clone()));
    esperar_ate("o módulo ler o perfil do atendente", || {
        synapse.ver(|g| g.chegou)
    })
    .await;
    primeiro.derrubar();
    let _ = envio.await;

    assert_eq!(
        fila_pendente(&base),
        0,
        "antes do registro, nada entrou na fila"
    );

    synapse.configurar(|g| g.segurar = None);
    let segundo = processo_do_receptor(&base, config, ligar_porta(porta));
    let (status, _) = enviar_transacao(porta, "TXN-C4A".to_owned(), corpo).await;
    assert_eq!(status, StatusCode::OK, "a retransmissão é aceita");
    segundo.derrubar();

    assert_eq!(fila_pendente(&base), 1, "uma resposta na fila");
    assert_eq!(
        respostas_humanas(&base),
        1,
        "uma mensagem do atendente gravada"
    );
    drop(synapse);
}

/// A transação já gravada chega de novo depois da queda (o Synapse não recebeu o `200`). A
/// memória de transações do processo se perdeu, então é a idempotência por evento que segura.
#[tokio::test]
async fn retransmissao_depois_da_queda_nao_gera_segunda_resposta() {
    let synapse = Synapse::no_ar();
    let base = Base::novo("c4b");
    base.ligar();
    let config = config_matrix(&synapse.endereco, 50);
    let (_conversa, sala) = sala_pronta_para_resposta(&base);
    let porta = porta_livre();
    let corpo = transacao_da_resposta(&sala, "$evento-c4b", "pedido enviado");

    let primeiro = processo_do_receptor(&base, config.clone(), ligar_porta(porta));
    let (status, _) = enviar_transacao(porta, "TXN-C4B".to_owned(), corpo.clone()).await;
    assert_eq!(status, StatusCode::OK);
    primeiro.derrubar();

    let segundo = processo_do_receptor(&base, config, ligar_porta(porta));
    let (status, _) = enviar_transacao(porta, "TXN-C4B".to_owned(), corpo).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "a retransmissão é aceita depois da queda"
    );
    segundo.derrubar();

    assert_eq!(
        fila_pendente(&base),
        1,
        "a retransmissão não cria segunda entrada na fila"
    );
    assert_eq!(
        respostas_humanas(&base),
        1,
        "uma mensagem do atendente gravada"
    );
    drop(synapse);
}

// ───────────────────────────── defeito achado: convite falho ─────────────────────────────

/// Um convite que falha (o Synapse responde 503 uma vez, sem queda) tem de ser refeito na passagem
/// seguinte. Enquanto o vínculo da sala com o espaço for gravado apesar do convite falho, a passagem
/// seguinte vê "já posicionada" e não tenta o convite de novo — foi o defeito que este teste achou
/// (ver `docs/MATRIZ_RESILIENCIA.md`), corrigido em `poph-matrix/src/espacos.rs`.
#[tokio::test]
async fn convite_falho_no_lote_e_refeito_na_passagem_seguinte() {
    let synapse = Synapse::no_ar();
    let config = config_matrix(&synapse.endereco, 50);
    let base = Base::novo("convite-falho");
    base.ligar();
    let dados = montar_espaco(&base, &synapse, &config);
    synapse.configurar(|g| g.falhar_convite = Some(2));

    let primeira = posicionar_com(&dados, Arc::clone(&base.loja), limites_de_lote())
        .await
        .expect("primeira passagem");
    assert!(
        matches!(
            primeira,
            Posicao::Posicionada {
                convites_falhos: 1,
                ..
            }
        ),
        "a primeira passagem registra um convite falho"
    );
    let _ = posicionar_com(&dados, Arc::clone(&base.loja), limites_de_lote())
        .await
        .expect("segunda passagem");

    let convidados = convidados_da_sala(&synapse, &dados.sala);
    for humano in &dados.humanos {
        assert!(
            convidados.contains(humano),
            "{humano} não foi convidado: a passagem seguinte não refez o convite falho"
        );
    }
    drop(synapse);
}

// ───────────────────────────── medição: cliente primeiro ─────────────────────────────

/// Os quatro cenários da comparação. "Sem módulo" é o estado padrão (`[matrix]` desligado).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Modo {
    SemModulo,
    SynapseFora,
    SynapseLento,
    SynapseNoAr,
}

impl Modo {
    const TODOS: [Modo; 4] = [
        Modo::SemModulo,
        Modo::SynapseFora,
        Modo::SynapseLento,
        Modo::SynapseNoAr,
    ];

    fn rotulo(self) -> &'static str {
        match self {
            Modo::SemModulo => "sem_modulo",
            Modo::SynapseFora => "modulo_synapse_fora",
            Modo::SynapseLento => "modulo_synapse_lento_5s",
            Modo::SynapseNoAr => "modulo_synapse_no_ar",
        }
    }
}

/// Conta as respostas do agente já gravadas na conversa da org (o que o turno produziu).
fn respostas_gravadas(loja: &Store, org: OrgId) -> usize {
    // Soma em todas as conversas da org: a conversa de teste da base também está lá, vazia.
    let Ok(conversas) = loja.conversations().list(org, None, Page::with_limit(50)) else {
        return 0;
    };
    conversas
        .iter()
        .map(|conversa| {
            loja.messages()
                .list(org, conversa.id, Page::with_limit(1000))
                .map(|mensagens| {
                    mensagens
                        .iter()
                        .filter(|mensagem| mensagem.direction == Direction::Outbound)
                        .count()
                })
                .unwrap_or(0)
        })
        .sum()
}

/// Uma amostra da medição: o instante em que a resposta foi gravada e o da chegada do envio, em
/// milissegundos desde o webhook.
struct Amostra {
    gravada_ms: f64,
    envio_ms: f64,
}

/// Uma rodada: `amostras` mensagens em sequência por um servidor novo.
async fn medir_rodada(modo: Modo, rodada: usize, amostras: usize) -> Vec<Amostra> {
    let base = Base::novo(&format!("med-{}-{rodada}", modo.rotulo()));
    let mut synapse: Option<Synapse> = None;
    let config = match modo {
        Modo::SemModulo => None,
        _ => {
            base.ligar();
            let mut sinapse = Synapse::em(porta_livre());
            match modo {
                Modo::SynapseFora => {}
                Modo::SynapseLento => {
                    sinapse.subir();
                    sinapse.configurar(|g| g.atraso = Duration::from_secs(5));
                }
                _ => sinapse.subir(),
            }
            let config = config_matrix(&sinapse.endereco, 500);
            synapse = Some(sinapse);
            Some(config)
        }
    };
    // Roteiros de sobra: o turno chama o modelo mais de uma vez por mensagem em alguns casos.
    let servidor = subir_servidor(&base, config, AjustesDeInicio::padrao(), amostras * 4).await;
    let mut resultado = Vec::with_capacity(amostras);
    for i in 0..amostras {
        let antes = Instant::now();
        let status = servidor
            .webhook(&format!("MSG-MED-{rodada}-{i}"), &format!("pergunta {i}"))
            .await;
        assert_eq!(status, StatusCode::OK, "webhook aceito");
        // Primeiro: a resposta gravada no banco (o turno terminou). Depois: o envio à Evolution,
        // que passa pelo laço de saída (intervalo de 1 s em produção).
        let inicio_espera = Instant::now();
        while respostas_gravadas(&base.loja, base.org) < i + 1 {
            assert!(
                inicio_espera.elapsed() < Duration::from_secs(30),
                "a resposta {} não foi gravada",
                i + 1
            );
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        let gravada = Instant::now();
        let chegada = esperar_envio(&servidor, i).await;
        resultado.push(Amostra {
            gravada_ms: gravada.duration_since(antes).as_secs_f64() * 1000.0,
            envio_ms: chegada.duration_since(antes).as_secs_f64() * 1000.0,
        });
    }
    let chamadas = servidor.chamadas_llm();
    println!(
        "{} rodada {rodada}: {chamadas} chamadas ao modelo falso para {amostras} mensagens",
        modo.rotulo()
    );
    servidor.encerrar().await;
    drop(synapse);
    resultado
}

/// Mede o caminho do cliente nos quatro cenários, em rodadas repetidas, e grava o CSV.
///
/// Duas medidas por mensagem: `gravada_ms` (até a resposta entrar no banco, o trabalho do turno) e
/// `envio_ms` (até o envio chegar à Evolution falsa, o que o cliente recebe, com o laço de saída de
/// produção). `MX29_RODADAS` (padrão 3) e `MX29_AMOSTRAS` (padrão 20) controlam o tamanho.
/// `MX29_CSV` recebe o caminho do arquivo. Roda por `scripts/medir-resiliencia.sh`.
#[tokio::test]
#[ignore = "medição: rodar por scripts/medir-resiliencia.sh"]
async fn medicao_cliente_primeiro() {
    let rodadas: usize = std::env::var("MX29_RODADAS")
        .ok()
        .and_then(|valor| valor.parse().ok())
        .unwrap_or(3);
    let amostras: usize = std::env::var("MX29_AMOSTRAS")
        .ok()
        .and_then(|valor| valor.parse().ok())
        .unwrap_or(20);
    let saida = std::env::var("MX29_CSV")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("mx29-medicao.csv"));

    let mut csv = String::from("modo,rodada,amostra,gravada_ms,envio_ms\n");
    let mut resumo = Vec::new();
    for modo in Modo::TODOS {
        let mut gravadas = Vec::new();
        let mut envios = Vec::new();
        for rodada in 1..=rodadas {
            let amostras_da_rodada = medir_rodada(modo, rodada, amostras).await;
            for (i, amostra) in amostras_da_rodada.iter().enumerate() {
                csv.push_str(&format!(
                    "{},{rodada},{i},{:.3},{:.3}\n",
                    modo.rotulo(),
                    amostra.gravada_ms,
                    amostra.envio_ms
                ));
            }
            let g: Vec<f64> = amostras_da_rodada.iter().map(|a| a.gravada_ms).collect();
            let e: Vec<f64> = amostras_da_rodada.iter().map(|a| a.envio_ms).collect();
            let (mg, pg, _, _) = estatisticas(&g);
            let (me, pe, _, _) = estatisticas(&e);
            println!(
                "{} rodada {rodada}: gravada mediana {mg:.1} ms p95 {pg:.1} ms | envio mediana {me:.1} ms p95 {pe:.1} ms",
                modo.rotulo()
            );
            gravadas.extend(g);
            envios.extend(e);
        }
        let (mg, pg, _, xg) = estatisticas(&gravadas);
        let (me, pe, _, xe) = estatisticas(&envios);
        resumo.push(format!(
            "{}: n={} | gravada mediana={mg:.1} p95={pg:.1} max={xg:.1} | envio mediana={me:.1} p95={pe:.1} max={xe:.1} (ms)",
            modo.rotulo(),
            gravadas.len()
        ));
    }
    fs::write(&saida, csv).expect("grava o CSV da medição");
    println!("--- resumo ---");
    for linha in &resumo {
        println!("{linha}");
    }
    println!("CSV: {}", saida.display());
}

// ───────────────────────────── cenários auxiliares ─────────────────────────────

/// Espaço de teste: um espaço já existente (ligado), a sala da conversa e cinco humanos nele.
#[derive(Clone)]
struct DadosEspaco {
    org: OrgId,
    conversa: ConversationId,
    agente: AgentId,
    sala: String,
    espaco: String,
    departamento: DepartmentId,
    nome_departamento: String,
    humanos: Vec<String>,
    url: String,
    config: MatrixConfig,
}

/// Lote pequeno e sem pausa: o convite cai no meio do lote com facilidade.
fn limites_de_lote() -> LimitesConvite {
    LimitesConvite::com(2, 1, Duration::ZERO, 100)
}

/// Monta o espaço no Synapse falso e a sala da conversa no banco.
fn montar_espaco(base: &Base, synapse: &Synapse, config: &MatrixConfig) -> DadosEspaco {
    let ids = IdsFantasmas::da_config(config).expect("ids");
    let bot = ids.bot().user_id().to_owned();
    let departamento = base
        .loja
        .departments()
        .create(
            base.org,
            NewDepartment {
                name: "Vendas".to_owned(),
                description: String::new(),
            },
        )
        .expect("cria o departamento")
        .id;
    let humanos: Vec<String> = (0..5).map(|i| format!("@humano{i}:{SERVIDOR}")).collect();
    let espaco = format!("!espaco-mx29:{SERVIDOR}");
    let sala = format!("!sala-mx29:{SERVIDOR}");
    let mut no_espaco: Vec<&str> = vec![bot.as_str()];
    no_espaco.extend(humanos.iter().map(String::as_str));
    synapse.semear_sala(
        &espaco,
        &no_espaco,
        vec![(
            ("m.room.power_levels", ""),
            json!({ "users": { bot.clone(): 100 } }),
        )],
    );
    synapse.semear_sala(&sala, &[], Vec::new());
    base.loja
        .matrix()
        .ensure_room(base.org, base.conversa, &sala, None)
        .expect("linha da sala");
    base.loja
        .matrix()
        .mark_room_ready(base.org, base.conversa)
        .expect("sala pronta");
    DadosEspaco {
        org: base.org,
        conversa: base.conversa,
        agente: base.agente,
        sala,
        espaco,
        departamento,
        nome_departamento: "Vendas".to_owned(),
        humanos,
        url: synapse.endereco.clone(),
        config: config.clone(),
    }
}

/// Põe a sala no espaço com o `posicionar` da MX-12, sobre o `store` dado.
async fn posicionar_com(
    dados: &DadosEspaco,
    store: Arc<Store>,
    limites: LimitesConvite,
) -> Result<Posicao, poph_matrix::espacos::ErroEspaco> {
    let ids = IdsFantasmas::da_config(&dados.config).expect("ids");
    let agente: Fantasma = ids
        .agente(&dados.agente.to_string())
        .expect("fantasma do agente");
    let espacos = Espacos::com(cliente_matrix(&dados.url), ids, PortaEspacosStore(store))
        .com_limites(limites);
    let sala = SalaParaEspaco {
        org: dados.org,
        conversa: dados.conversa,
        sala: dados.sala.clone(),
        agente,
        departamento: Some(DepartamentoEspaco {
            id: dados.departamento,
            nome: dados.nome_departamento.clone(),
        }),
        existente: Some(dados.espaco.clone()),
    };
    espacos.posicionar(&sala).await
}

fn convidados_da_sala(synapse: &Synapse, sala: &str) -> Vec<String> {
    synapse.ver(|g| {
        g.salas
            .get(sala)
            .map(|alvo| alvo.convidados.clone())
            .unwrap_or_default()
    })
}

/// Sala já gravada como pronta para a conversa, com o id dela no Synapse.
fn sala_pronta_para_resposta(base: &Base) -> (ConversationId, String) {
    let sala = format!("!resposta-mx29:{SERVIDOR}");
    base.loja
        .matrix()
        .ensure_room(base.org, base.conversa, &sala, None)
        .expect("linha da sala");
    base.loja
        .matrix()
        .mark_room_ready(base.org, base.conversa)
        .expect("sala pronta");
    (base.conversa, sala)
}

/// Transação do Synapse com uma mensagem de atendente na sala.
fn transacao_da_resposta(sala: &str, evento: &str, texto: &str) -> Value {
    json!({
        "events": [{
            "type": "m.room.message",
            "room_id": sala,
            "event_id": evento,
            "sender": format!("@ana:{SERVIDOR}"),
            "content": { "msgtype": "m.text", "body": texto }
        }]
    })
}

/// Entrega uma transação ao receptor do Application Service, como o Synapse faria.
async fn enviar_transacao(endereco: SocketAddr, txn: String, corpo: Value) -> (StatusCode, Value) {
    let url = format!("http://{endereco}/_matrix/app/v1/transactions/{txn}");
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .expect("cliente HTTP");
    match http
        .put(url)
        .header(AUTHORIZATION, format!("Bearer {HS_TOKEN}"))
        .json(&corpo)
        .send()
        .await
    {
        Ok(resposta) => {
            let status = resposta.status();
            let valor = resposta.json::<Value>().await.unwrap_or(Value::Null);
            (status, valor)
        }
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, Value::Null),
    }
}

/// Quantas entradas estão pendentes na fila de saída da org.
fn fila_pendente(base: &Base) -> u64 {
    base.loja
        .outbox()
        .count_by_state(base.org, OutboxState::Pending)
        .expect("conta a fila")
}

/// Quantas mensagens do atendente (origem humana) estão gravadas na conversa.
fn respostas_humanas(base: &Base) -> usize {
    base.loja
        .messages()
        .list(base.org, base.conversa, Page::with_limit(100))
        .expect("mensagens")
        .into_iter()
        .filter(|mensagem| mensagem.origin == Origin::Human)
        .count()
}
