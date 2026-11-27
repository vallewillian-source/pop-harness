use std::collections::BTreeMap;
use std::fs;
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::extract::State;
use axum::http::header::AUTHORIZATION;
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};

const AS_TOKEN: &str = "CANARIO-AS-mx23-nao-pode-vazar";
const HS_TOKEN: &str = "CANARIO-HS-mx23-nao-pode-vazar";
const SERVIDOR: &str = "matrix-dev.local";

#[derive(Default, Clone)]
struct FakeRoom {
    tipo: Option<String>,
    members: Vec<String>,
    guests: Vec<String>,
    state: BTreeMap<(String, String), Value>,
    messages: Vec<(String, String, Value)>,
}

#[derive(Default)]
struct FakeState {
    users: BTreeMap<String, Option<String>>,
    rooms: BTreeMap<String, FakeRoom>,
    aliases: BTreeMap<String, String>,
    next_room: u32,
    versions: u32,
}

fn respond(status: u16, body: Value) -> Response {
    (
        StatusCode::from_u16(status).expect("status válido"),
        [("content-type", "application/json")],
        body.to_string(),
    )
        .into_response()
}

fn decode(segment: &str) -> String {
    let bytes = segment.as_bytes();
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

fn query_argument(raw: &str, name: &str) -> Option<String> {
    raw.split('&').find_map(|pair| {
        let (chave, value) = pair.split_once('=')?;
        (chave == name).then(|| decode(value))
    })
}

async fn handle(
    State(state): State<Arc<Mutex<FakeState>>>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: String,
) -> Response {
    let body: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
    let mut guard = state.lock().expect("trava do Synapse falso");

    let authorization = headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    if authorization != format!("Bearer {AS_TOKEN}") {
        return respond(
            401,
            json!({ "errcode": "M_UNKNOWN_TOKEN", "error": "Token inválido" }),
        );
    }
    if uri.path() == "/_matrix/client/versions" {
        guard.versions += 1;
        return respond(200, json!({ "versions": ["v1.11"] }));
    }

    let path = uri.path().trim_start_matches("/_matrix/client/v3");
    let parts: Vec<String> = path
        .split('/')
        .filter(|piece| !piece.is_empty())
        .map(decode)
        .collect();
    let raw_query = uri.query().unwrap_or_default().to_owned();
    let who = query_argument(&raw_query, "user_id").unwrap_or_default();

    match (method.as_str(), parts.as_slice()) {
        ("POST", [registro]) if registro == "register" => {
            let localpart = body["username"].as_str().unwrap_or_default();
            let id = format!("@{localpart}:{SERVIDOR}");
            if guard.users.contains_key(&id) {
                return respond(
                    400,
                    json!({ "errcode": "M_USER_IN_USE", "error": "User ID already taken." }),
                );
            }
            guard.users.insert(id.clone(), None);
            respond(200, json!({ "user_id": id }))
        }
        ("GET", [profile, id]) if profile == "profile" => match guard.users.get(id) {
            Some(name) => {
                let mut response = json!({});
                if let Some(name) = name {
                    response["displayname"] = Value::String(name.clone());
                }
                respond(200, response)
            }
            None => respond(
                404,
                json!({ "errcode": "M_NOT_FOUND", "error": "Usuário não existe" }),
            ),
        },
        ("PUT", [profile, id, campo]) if profile == "profile" && campo == "displayname" => {
            guard
                .users
                .insert(id.clone(), body["displayname"].as_str().map(str::to_owned));
            respond(200, json!({}))
        }
        ("POST", [creation]) if creation == "createRoom" => {
            let alias = body["room_alias_name"]
                .as_str()
                .map(|localpart| format!("#{localpart}:{SERVIDOR}"));
            if let Some(alias) = &alias
                && guard.aliases.contains_key(alias)
            {
                return respond(
                    400,
                    json!({ "errcode": "M_ROOM_IN_USE", "error": "Room alias already taken." }),
                );
            }
            guard.next_room += 1;
            let room = format!("!{}:{SERVIDOR}", guard.next_room);
            let mut members = Vec::new();
            if !who.is_empty() {
                members.push(who.clone());
            }
            guard.rooms.insert(
                room.clone(),
                FakeRoom {
                    tipo: body["creation_content"]["type"].as_str().map(str::to_owned),
                    members,
                    ..FakeRoom::default()
                },
            );
            if let Some(alias) = alias {
                guard.aliases.insert(alias, room.clone());
            }
            respond(200, json!({ "room_id": room }))
        }
        ("GET", [diretorio, room, alias]) if diretorio == "directory" && room == "room" => {
            match guard.aliases.get(alias) {
                Some(room) => respond(200, json!({ "room_id": room })),
                None => respond(
                    404,
                    json!({ "errcode": "M_NOT_FOUND", "error": "Alias não encontrado" }),
                ),
            }
        }
        ("POST", [rooms, room, invite]) if rooms == "rooms" && invite == "invite" => {
            match guard.rooms.get_mut(room) {
                Some(target) => {
                    let guest = body["user_id"].as_str().unwrap_or_default().to_owned();
                    if !target.guests.contains(&guest) {
                        target.guests.push(guest);
                    }
                    respond(200, json!({}))
                }
                None => respond(
                    404,
                    json!({ "errcode": "M_NOT_FOUND", "error": "Sala não existe" }),
                ),
            }
        }
        ("POST", [enter, target]) if enter == "join" => {
            let room = if target.starts_with('#') {
                guard.aliases.get(target).cloned()
            } else {
                Some(target.clone())
            };
            match room.and_then(|room| guard.rooms.get_mut(&room).map(|target| (room, target))) {
                Some((room, target)) => {
                    if !target.members.contains(&who) {
                        target.members.push(who.clone());
                    }
                    respond(200, json!({ "room_id": room }))
                }
                None => respond(
                    404,
                    json!({ "errcode": "M_NOT_FOUND", "error": "Sala não existe" }),
                ),
            }
        }
        ("GET", [rooms, room, members]) if rooms == "rooms" && members == "joined_members" => {
            match guard.rooms.get(room) {
                Some(target) => {
                    let joined: BTreeMap<String, Value> = target
                        .members
                        .iter()
                        .map(|id| (id.clone(), json!({})))
                        .collect();
                    respond(200, json!({ "joined": joined }))
                }
                None => respond(
                    404,
                    json!({ "errcode": "M_NOT_FOUND", "error": "Sala não existe" }),
                ),
            }
        }
        ("PUT", [rooms, room, state, tipo, chave @ ..]) if rooms == "rooms" && state == "state" => {
            let chave = chave.join("/");
            match guard.rooms.get_mut(room) {
                Some(target) => {
                    target.state.insert((tipo.clone(), chave), body);
                    respond(200, json!({ "event_id": "$estado" }))
                }
                None => respond(
                    404,
                    json!({ "errcode": "M_NOT_FOUND", "error": "Sala não existe" }),
                ),
            }
        }
        ("GET", [rooms, room, state, tipo, chave @ ..]) if rooms == "rooms" && state == "state" => {
            let chave = chave.join("/");
            match guard.rooms.get(room) {
                Some(target) => match target.state.get(&(tipo.clone(), chave)) {
                    Some(body) => respond(200, body.clone()),
                    None => respond(
                        404,
                        json!({ "errcode": "M_NOT_FOUND", "error": "Evento não existe" }),
                    ),
                },
                None => respond(
                    404,
                    json!({ "errcode": "M_NOT_FOUND", "error": "Sala não existe" }),
                ),
            }
        }
        ("PUT", [rooms, room, send, tipo, txn]) if rooms == "rooms" && send == "send" => {
            match guard.rooms.get_mut(room) {
                Some(target) => {
                    if !target.messages.iter().any(|(ja, _, _)| ja == txn) {
                        target.messages.push((txn.clone(), tipo.clone(), body));
                    }
                    respond(200, json!({ "event_id": format!("$msg_{txn}") }))
                }
                None => respond(
                    404,
                    json!({ "errcode": "M_NOT_FOUND", "error": "Sala não existe" }),
                ),
            }
        }
        _ => respond(
            404,
            json!({ "errcode": "M_UNRECOGNIZED", "error": "Rota não existe no Synapse falso" }),
        ),
    }
}

struct Scenario {
    dir: PathBuf,
    config: PathBuf,
}

impl Scenario {
    fn new(label: &str, homeserver: &str, enabled: bool) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "poph-cli-matrix-modulo-{label}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("dados")).expect("pasta de dados");
        let config = dir.join("poph.toml");
        fs::write(
            &config,
            format!(
                "[dados]\ndiretorio = \"{}\"\n\n[matrix]\nhabilitado = {enabled}\n\
                 homeserver_url = \"{homeserver}\"\nserver_name = \"{SERVIDOR}\"\nprefixo = \"ph\"\n",
                dir.join("dados").display()
            ),
        )
        .expect("escreve o poph.toml");
        Self { dir, config }
    }

    fn poph(&self, args: &[&str], tokens: bool) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_poph"));
        command
            .args(args)
            .current_dir(&self.dir)
            .env_clear()
            .env("POPH_CONFIG", &self.config);
        if tokens {
            command
                .env("POPH_MATRIX_AS_TOKEN", AS_TOKEN)
                .env("POPH_MATRIX_HS_TOKEN", HS_TOKEN);
        }
        command.output().expect("executa o poph")
    }

    fn create_org(&self, slug: &str) -> String {
        let saida = self.poph(
            &[
                "--offline",
                "org",
                "create",
                "--name",
                &format!("Org {slug}"),
                "--slug",
                slug,
                "--json",
            ],
            false,
        );
        assert_eq!(code(&saida), 0, "{}", text(&saida.stderr));
        json_output(&saida)["org"]["id"]
            .as_str()
            .expect("id da org")
            .to_owned()
    }

    fn create_department(&self, slug: &str, name: &str) -> String {
        let saida = self.poph(
            &[
                "--offline",
                "--org",
                slug,
                "department",
                "create",
                "--name",
                name,
                "--json",
            ],
            false,
        );
        assert_eq!(code(&saida), 0, "{}", text(&saida.stderr));
        json_output(&saida)["department"]["id"]
            .as_str()
            .expect("id do departamento")
            .to_owned()
    }
}

impl Drop for Scenario {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn code(saida: &Output) -> i32 {
    saida.status.code().unwrap_or(-1)
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn json_output(saida: &Output) -> Value {
    serde_json::from_slice(&saida.stdout)
        .unwrap_or_else(|error| panic!("a saída não é JSON ({error}): {}", text(&saida.stdout)))
}

fn without_tokens(saida: &Output) {
    for canary in [AS_TOKEN, HS_TOKEN] {
        assert!(
            !text(&saida.stdout).contains(canary),
            "token vazou no stdout"
        );
        assert!(
            !text(&saida.stderr).contains(canary),
            "token vazou no stderr"
        );
    }
}

fn spaces_in_synapse(synapse: &Synapse) -> usize {
    synapse
        .state
        .lock()
        .expect("trava do Synapse falso")
        .rooms
        .values()
        .filter(|room| room.tipo.as_deref() == Some("m.space"))
        .count()
}

#[test]
fn state_turn_on_and_off_through_the_cli_preserve_the_cutoff() {
    let scenario = Scenario::new("ligacao", "http://127.0.0.1:9", true);
    scenario.create_org("loja-mx23");

    let before = scenario.poph(
        &[
            "--offline",
            "--org",
            "loja-mx23",
            "matrix",
            "estado",
            "--json",
        ],
        false,
    );
    assert_eq!(code(&before), 0, "{}", text(&before.stderr));
    without_tokens(&before);
    let before = json_output(&before);
    assert_eq!(before["versao_relatorio"], 1);
    assert_eq!(before["ligado"], false);
    assert!(before["corte_do_backfill_ms"].is_null());
    assert!(
        before["conectado"].is_null(),
        "a CLI não sabe se o Synapse responde"
    );
    assert!(
        before["versao"].is_i64(),
        "a versão é o updated_at (o ETag da API)"
    );

    let turned_on = scenario.poph(
        &[
            "--offline",
            "--org",
            "loja-mx23",
            "matrix",
            "ligar",
            "--json",
        ],
        false,
    );
    assert_eq!(code(&turned_on), 0, "{}", text(&turned_on.stderr));
    without_tokens(&turned_on);
    let turned_on = json_output(&turned_on);
    assert_eq!(turned_on["ligado"], true);
    let cutoff = turned_on["corte_do_backfill_ms"]
        .as_i64()
        .expect("ligar grava o corte do backfill");

    let again = scenario.poph(
        &[
            "--offline",
            "--org",
            "loja-mx23",
            "matrix",
            "ligar",
            "--json",
        ],
        false,
    );
    assert_eq!(code(&again), 0, "{}", text(&again.stderr));
    assert_eq!(json_output(&again)["corte_do_backfill_ms"], cutoff);

    let turned_off = scenario.poph(
        &[
            "--offline",
            "--org",
            "loja-mx23",
            "matrix",
            "desligar",
            "--json",
        ],
        false,
    );
    assert_eq!(code(&turned_off), 0, "{}", text(&turned_off.stderr));
    let turned_off = json_output(&turned_off);
    assert_eq!(turned_off["ligado"], false);
    assert_eq!(
        turned_off["corte_do_backfill_ms"], cutoff,
        "desligar preserva o corte do backfill"
    );

    let state_text = scenario.poph(
        &["--offline", "--org", "loja-mx23", "matrix", "estado"],
        false,
    );
    assert_eq!(code(&state_text), 0, "{}", text(&state_text.stderr));
    let saida = text(&state_text.stdout);
    assert!(
        saida.contains("ligado:                 não"),
        "saída: {saida}"
    );
    assert!(saida.contains("desconhecido pela CLI"), "saída: {saida}");
    without_tokens(&state_text);
}

#[test]
fn org_and_usage_without_the_flags_give_a_clear_error() {
    let scenario = Scenario::new("flags", "http://127.0.0.1:9", true);
    scenario.create_org("existe-mx23");

    let missing = scenario.poph(
        &["--offline", "--org", "nao-existe", "matrix", "estado"],
        false,
    );
    assert_eq!(code(&missing), 1);
    assert!(
        text(&missing.stderr).contains("não existe org com o slug `nao-existe`"),
        "stderr: {}",
        text(&missing.stderr)
    );

    let without_offline = scenario.poph(&["--org", "existe-mx23", "matrix", "ligar"], false);
    assert_eq!(code(&without_offline), 1);
    assert!(text(&without_offline.stderr).contains("--offline"));

    let without_org = scenario.poph(&["--offline", "matrix", "desligar"], false);
    assert_eq!(code(&without_org), 1);
    assert!(text(&without_org.stderr).contains("informe --org"));

    let state = json_output(&scenario.poph(
        &[
            "--offline",
            "--org",
            "existe-mx23",
            "matrix",
            "estado",
            "--json",
        ],
        false,
    ));
    assert_eq!(state["ligado"], false);
}

#[test]
fn space_creates_once_lists_and_turns_off_without_deleting_in_element() {
    let synapse = start_synapse();
    let scenario = Scenario::new("espaco", &synapse.endereco, true);
    scenario.create_org("vendas-mx23");
    let department = scenario.create_department("vendas-mx23", "Vendas");

    let created = scenario.poph(
        &[
            "--offline",
            "--org",
            "vendas-mx23",
            "matrix",
            "espaco",
            "--departamento",
            &department,
            "--json",
        ],
        true,
    );
    assert_eq!(code(&created), 0, "{}", text(&created.stderr));
    without_tokens(&created);
    let created = json_output(&created);
    let space = created["espaco"].as_str().expect("id do espaço").to_owned();
    assert!(space.starts_with('!'), "o espaço é um id de sala: {space}");
    assert_eq!(created["origem"], "auto");
    assert_eq!(spaces_in_synapse(&synapse), 1);

    let repeated = scenario.poph(
        &[
            "--offline",
            "--org",
            "vendas-mx23",
            "matrix",
            "espaco",
            "--departamento",
            &department,
            "--json",
        ],
        true,
    );
    assert_eq!(code(&repeated), 0, "{}", text(&repeated.stderr));
    assert_eq!(json_output(&repeated)["espaco"], space);
    assert_eq!(
        spaces_in_synapse(&synapse),
        1,
        "repetir não cria segundo espaço"
    );

    let listing = scenario.poph(
        &[
            "--offline",
            "--org",
            "vendas-mx23",
            "matrix",
            "espaco",
            "--json",
        ],
        true,
    );
    assert_eq!(code(&listing), 0, "{}", text(&listing.stderr));
    let listing = json_output(&listing);
    let items = listing["espacos"].as_array().expect("lista de espaços");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["departamento_nome"], "Vendas");
    assert_eq!(items[0]["espaco"], space);

    let turned_off = scenario.poph(
        &[
            "--offline",
            "--org",
            "vendas-mx23",
            "matrix",
            "espaco",
            "desligar",
            "--departamento",
            &department,
            "--json",
        ],
        true,
    );
    assert_eq!(code(&turned_off), 0, "{}", text(&turned_off.stderr));
    let turned_off = json_output(&turned_off);
    assert_eq!(turned_off["removido"], true);
    assert_eq!(
        spaces_in_synapse(&synapse),
        1,
        "desligar não apaga o espaço no Element"
    );

    let vazio = json_output(&scenario.poph(
        &[
            "--offline",
            "--org",
            "vendas-mx23",
            "matrix",
            "espaco",
            "--json",
        ],
        true,
    ));
    assert_eq!(vazio["espacos"].as_array().expect("lista").len(), 0);
}

#[test]
fn space_without_tokens_names_the_variable_and_writes_nothing() {
    let scenario = Scenario::new("sem-tokens", "http://127.0.0.1:9", true);
    scenario.create_org("sem-token-mx23");
    let department = scenario.create_department("sem-token-mx23", "Suporte");

    let saida = scenario.poph(
        &[
            "--offline",
            "--org",
            "sem-token-mx23",
            "matrix",
            "espaco",
            "--departamento",
            &department,
        ],
        false,
    );
    assert_eq!(code(&saida), 1);
    let stderr = text(&saida.stderr);
    assert!(stderr.contains("POPH_MATRIX_AS_TOKEN"), "stderr: {stderr}");
    without_tokens(&saida);

    let listing = json_output(&scenario.poph(
        &[
            "--offline",
            "--org",
            "sem-token-mx23",
            "matrix",
            "espaco",
            "--json",
        ],
        false,
    ));
    assert_eq!(listing["espacos"].as_array().expect("lista").len(), 0);
}

#[test]
fn space_with_module_off_refuses_with_the_instruction() {
    let scenario = Scenario::new("desligado", "http://127.0.0.1:9", false);
    scenario.create_org("desligado-mx23");
    let department = scenario.create_department("desligado-mx23", "Vendas");

    let saida = scenario.poph(
        &[
            "--offline",
            "--org",
            "desligado-mx23",
            "matrix",
            "espaco",
            "--departamento",
            &department,
        ],
        true,
    );
    assert_eq!(code(&saida), 1);
    assert!(
        text(&saida.stderr).contains("habilitado = true"),
        "stderr: {}",
        text(&saida.stderr)
    );
    without_tokens(&saida);
}

#[test]
fn department_space_from_another_org_does_not_exist_for_this_org() {
    let synapse = start_synapse();
    let scenario = Scenario::new("outra-org", &synapse.endereco, true);
    scenario.create_org("dona-mx23");
    scenario.create_org("outra-mx23");
    let other_org_department = scenario.create_department("outra-mx23", "Financeiro");

    let saida = scenario.poph(
        &[
            "--offline",
            "--org",
            "dona-mx23",
            "matrix",
            "espaco",
            "--departamento",
            &other_org_department,
        ],
        true,
    );
    assert_eq!(code(&saida), 1);
    assert!(
        text(&saida.stderr).contains("não existe nesta org"),
        "stderr: {}",
        text(&saida.stderr)
    );
    assert_eq!(spaces_in_synapse(&synapse), 0, "nada foi criado no Synapse");

    let desligar = scenario.poph(
        &[
            "--offline",
            "--org",
            "dona-mx23",
            "matrix",
            "espaco",
            "desligar",
            "--departamento",
            &other_org_department,
        ],
        true,
    );
    assert_eq!(code(&desligar), 1);
    assert!(text(&desligar.stderr).contains("não existe nesta org"));
}

#[test]
fn space_refuses_combinations_and_invalid_space() {
    let synapse = start_synapse();
    let scenario = Scenario::new("combinacoes", &synapse.endereco, true);
    scenario.create_org("combina-mx23");
    let department = scenario.create_department("combina-mx23", "Vendas");

    let sozinho = scenario.poph(
        &[
            "--offline",
            "--org",
            "combina-mx23",
            "matrix",
            "espaco",
            "--espaco",
            "!abc:matrix-dev.local",
        ],
        true,
    );
    assert_eq!(code(&sozinho), 1);
    assert!(text(&sozinho.stderr).contains("junto com --departamento"));

    let misturado = scenario.poph(
        &[
            "--offline",
            "--org",
            "combina-mx23",
            "matrix",
            "espaco",
            "--espaco",
            "!abc:matrix-dev.local",
            "desligar",
            "--departamento",
            &department,
        ],
        true,
    );
    assert_eq!(code(&misturado), 1);
    assert!(text(&misturado.stderr).contains("sozinho"));

    let invalid = scenario.poph(
        &[
            "--offline",
            "--org",
            "combina-mx23",
            "matrix",
            "espaco",
            "--departamento",
            &department,
            "--espaco",
            "sem-prefixo",
        ],
        true,
    );
    assert_eq!(code(&invalid), 1);
    assert!(
        text(&invalid.stderr).contains("espaço informado inválido"),
        "stderr: {}",
        text(&invalid.stderr)
    );
    without_tokens(&invalid);
    assert_eq!(spaces_in_synapse(&synapse), 0);
}

struct Synapse {
    endereco: String,
    state: Arc<Mutex<FakeState>>,
}

fn start_synapse() -> Synapse {
    let listener = TcpListener::bind("127.0.0.1:0").expect("porta efêmera");
    listener.set_nonblocking(true).expect("não bloqueante");
    let endereco = format!("http://{}", listener.local_addr().expect("endereço"));
    let state = Arc::new(Mutex::new(FakeState::default()));
    let for_thread = Arc::clone(&state);
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("runtime do Synapse falso");
        runtime.block_on(async move {
            let listener = tokio::net::TcpListener::from_std(listener).expect("ouvinte");
            let app = Router::new().fallback(handle).with_state(for_thread);
            let _ = axum::serve(listener, app).await;
        });
    });
    Synapse { endereco, state }
}
