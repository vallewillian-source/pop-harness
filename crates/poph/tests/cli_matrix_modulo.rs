//! Testes de `poph matrix estado|ligar|desligar|espaco` de ponta a ponta (MX-23): processo de verdade,
//! banco de verdade e, nos testes de espaço, um **Synapse falso** no ar numa porta efêmera.
//!
//! O que fica provado:
//! - `estado`, `ligar` e `desligar` mostram e gravam o mesmo que a API (MX-22): o corte do backfill é
//!   gravado na primeira ligação, repetir não move o corte e desligar o preserva;
//! - `espaco --departamento` cria o espaço uma vez só (repetir não cria segundo), `espaco` lista,
//!   `espaco desligar` tira a linha e não apaga nada no Synapse;
//! - org inexistente, uso sem `--offline`/`--org`, módulo desligado, token ausente e departamento de
//!   outra org dão erro claro, sem nenhum valor de token na saída;
//! - nenhuma saída tem o valor dos tokens (valores canário).
//!
//! A checagem de versão (`412`, código de saída 3) é provada por teste unitário em `matrix_admin.rs`,
//! porque a CLI é quem lê a versão e grava; não há como um processo externo mudar a linha no meio.
//!
//! O bloco do Synapse falso é uma cópia das rotas de `crates/poph-server/tests/matrix_api.rs`: cada
//! arquivo de teste é um crate, e um teste de `poph-server` não é importável daqui.

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

/// Token que o Harness apresenta ao Synapse (valor canário: não pode aparecer em saída nenhuma).
const AS_TOKEN: &str = "CANARIO-AS-mx23-nao-pode-vazar";
/// Token que o Synapse apresenta ao Harness (valor canário).
const HS_TOKEN: &str = "CANARIO-HS-mx23-nao-pode-vazar";
/// Domínio dos ids no teste.
const SERVIDOR: &str = "matrix-dev.local";

#[derive(Default, Clone)]
struct SalaFalsa {
    tipo: Option<String>,
    members: Vec<String>,
    guests: Vec<String>,
    state: BTreeMap<(String, String), Value>,
    mensagens: Vec<(String, String, Value)>,
}

#[derive(Default)]
struct EstadoFalso {
    users: BTreeMap<String, Option<String>>,
    rooms: BTreeMap<String, SalaFalsa>,
    aliases: BTreeMap<String, String>,
    proxima_sala: u32,
    versoes: u32,
}

fn responder(status: u16, body: Value) -> Response {
    (
        StatusCode::from_u16(status).expect("status válido"),
        [("content-type", "application/json")],
        body.to_string(),
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
fn consulta(bruta: &str, name: &str) -> Option<String> {
    bruta.split('&').find_map(|par| {
        let (chave, valor) = par.split_once('=')?;
        (chave == name).then(|| decodificar(valor))
    })
}

async fn atender(
    State(state): State<Arc<Mutex<EstadoFalso>>>,
    metodo: Method,
    uri: Uri,
    cabecalhos: HeaderMap,
    body: String,
) -> Response {
    let body: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
    let mut guarda = state.lock().expect("trava do Synapse falso");

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
            let localpart = body["username"].as_str().unwrap_or_default();
            let id = format!("@{localpart}:{SERVIDOR}");
            if guarda.users.contains_key(&id) {
                return responder(
                    400,
                    json!({ "errcode": "M_USER_IN_USE", "error": "User ID already taken." }),
                );
            }
            guarda.users.insert(id.clone(), None);
            responder(200, json!({ "user_id": id }))
        }
        ("GET", [profile, id]) if profile == "profile" => match guarda.users.get(id) {
            Some(name) => {
                let mut resposta = json!({});
                if let Some(name) = name {
                    resposta["displayname"] = Value::String(name.clone());
                }
                responder(200, resposta)
            }
            None => responder(
                404,
                json!({ "errcode": "M_NOT_FOUND", "error": "Usuário não existe" }),
            ),
        },
        ("PUT", [profile, id, campo]) if profile == "profile" && campo == "displayname" => {
            guarda
                .users
                .insert(id.clone(), body["displayname"].as_str().map(str::to_owned));
            responder(200, json!({}))
        }
        ("POST", [criacao]) if criacao == "createRoom" => {
            let alias = body["room_alias_name"]
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
            let room = format!("!{}:{SERVIDOR}", guarda.proxima_sala);
            let mut members = Vec::new();
            if !quem.is_empty() {
                members.push(quem.clone());
            }
            guarda.rooms.insert(
                room.clone(),
                SalaFalsa {
                    tipo: body["creation_content"]["type"].as_str().map(str::to_owned),
                    members,
                    ..SalaFalsa::default()
                },
            );
            if let Some(alias) = alias {
                guarda.aliases.insert(alias, room.clone());
            }
            responder(200, json!({ "room_id": room }))
        }
        ("GET", [diretorio, room, alias]) if diretorio == "directory" && room == "room" => {
            match guarda.aliases.get(alias) {
                Some(room) => responder(200, json!({ "room_id": room })),
                None => responder(
                    404,
                    json!({ "errcode": "M_NOT_FOUND", "error": "Alias não encontrado" }),
                ),
            }
        }
        ("POST", [rooms, room, convite]) if rooms == "rooms" && convite == "invite" => {
            match guarda.rooms.get_mut(room) {
                Some(alvo) => {
                    let convidado = body["user_id"].as_str().unwrap_or_default().to_owned();
                    if !alvo.guests.contains(&convidado) {
                        alvo.guests.push(convidado);
                    }
                    responder(200, json!({}))
                }
                None => responder(
                    404,
                    json!({ "errcode": "M_NOT_FOUND", "error": "Sala não existe" }),
                ),
            }
        }
        ("POST", [enter, alvo]) if enter == "join" => {
            let room = if alvo.starts_with('#') {
                guarda.aliases.get(alvo).cloned()
            } else {
                Some(alvo.clone())
            };
            match room.and_then(|room| guarda.rooms.get_mut(&room).map(|alvo| (room, alvo))) {
                Some((room, alvo)) => {
                    if !alvo.members.contains(&quem) {
                        alvo.members.push(quem.clone());
                    }
                    responder(200, json!({ "room_id": room }))
                }
                None => responder(
                    404,
                    json!({ "errcode": "M_NOT_FOUND", "error": "Sala não existe" }),
                ),
            }
        }
        ("GET", [rooms, room, members]) if rooms == "rooms" && members == "joined_members" => {
            match guarda.rooms.get(room) {
                Some(alvo) => {
                    let juntos: BTreeMap<String, Value> = alvo
                        .members
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
        ("PUT", [rooms, room, state, tipo, chave @ ..]) if rooms == "rooms" && state == "state" => {
            let chave = chave.join("/");
            match guarda.rooms.get_mut(room) {
                Some(alvo) => {
                    alvo.state.insert((tipo.clone(), chave), body);
                    responder(200, json!({ "event_id": "$estado" }))
                }
                None => responder(
                    404,
                    json!({ "errcode": "M_NOT_FOUND", "error": "Sala não existe" }),
                ),
            }
        }
        ("GET", [rooms, room, state, tipo, chave @ ..]) if rooms == "rooms" && state == "state" => {
            let chave = chave.join("/");
            match guarda.rooms.get(room) {
                Some(alvo) => match alvo.state.get(&(tipo.clone(), chave)) {
                    Some(body) => responder(200, body.clone()),
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
        ("PUT", [rooms, room, send, tipo, txn]) if rooms == "rooms" && send == "send" => {
            match guarda.rooms.get_mut(room) {
                Some(alvo) => {
                    if !alvo.mensagens.iter().any(|(ja, _, _)| ja == txn) {
                        alvo.mensagens.push((txn.clone(), tipo.clone(), body));
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

// ───────────────────────────── ambiente de teste ─────────────────────────────

/// Uma pasta de teste com `poph.toml` e banco próprios.
struct Cenario {
    pasta: PathBuf,
    config: PathBuf,
}

impl Cenario {
    /// `homeserver` aponta para o Synapse falso (ou para um endereço morto, quando o teste não fala com ele).
    fn new(rotulo: &str, homeserver: &str, habilitado: bool) -> Self {
        let pasta = std::env::temp_dir().join(format!(
            "poph-cli-matrix-modulo-{rotulo}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&pasta);
        fs::create_dir_all(pasta.join("dados")).expect("pasta de dados");
        let config = pasta.join("poph.toml");
        fs::write(
            &config,
            format!(
                "[dados]\ndiretorio = \"{}\"\n\n[matrix]\nhabilitado = {habilitado}\n\
                 homeserver_url = \"{homeserver}\"\nserver_name = \"{SERVIDOR}\"\nprefixo = \"ph\"\n",
                pasta.join("dados").display()
            ),
        )
        .expect("escreve o poph.toml");
        Self { pasta, config }
    }

    /// Roda o `poph` com ambiente limpo. Com `tokens`, põe os dois tokens do Application Service.
    fn poph(&self, args: &[&str], tokens: bool) -> Output {
        let mut comando = Command::new(env!("CARGO_BIN_EXE_poph"));
        comando
            .args(args)
            .current_dir(&self.pasta)
            .env_clear()
            .env("POPH_CONFIG", &self.config);
        if tokens {
            comando
                .env("POPH_MATRIX_AS_TOKEN", AS_TOKEN)
                .env("POPH_MATRIX_HS_TOKEN", HS_TOKEN);
        }
        comando.output().expect("executa o poph")
    }

    /// Cria a org pela CLI (offline) e devolve o id.
    fn criar_org(&self, slug: &str) -> String {
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
        json_de(&saida)["org"]["id"]
            .as_str()
            .expect("id da org")
            .to_owned()
    }

    /// Cria um departamento pela CLI e devolve o id.
    fn criar_departamento(&self, slug: &str, name: &str) -> String {
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
        json_de(&saida)["department"]["id"]
            .as_str()
            .expect("id do departamento")
            .to_owned()
    }
}

impl Drop for Cenario {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.pasta);
    }
}

fn code(saida: &Output) -> i32 {
    saida.status.code().unwrap_or(-1)
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn json_de(saida: &Output) -> Value {
    serde_json::from_slice(&saida.stdout)
        .unwrap_or_else(|error| panic!("a saída não é JSON ({error}): {}", text(&saida.stdout)))
}

/// Nenhum valor de token pode aparecer na saída do comando.
fn sem_tokens(saida: &Output) {
    for canario in [AS_TOKEN, HS_TOKEN] {
        assert!(
            !text(&saida.stdout).contains(canario),
            "token vazou no stdout"
        );
        assert!(
            !text(&saida.stderr).contains(canario),
            "token vazou no stderr"
        );
    }
}

/// Quantos espaços (`m.space`) o Synapse falso tem.
fn espacos_no_synapse(synapse: &Synapse) -> usize {
    synapse
        .state
        .lock()
        .expect("trava do Synapse falso")
        .rooms
        .values()
        .filter(|room| room.tipo.as_deref() == Some("m.space"))
        .count()
}

// ───────────────────────────── estado, ligar e desligar ─────────────────────────────

#[test]
fn estado_ligar_e_desligar_pela_cli_preservam_o_corte() {
    let cenario = Cenario::new("ligacao", "http://127.0.0.1:9", true);
    cenario.criar_org("loja-mx23");

    // Antes de ligar: desligado e sem corte.
    let antes = cenario.poph(
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
    assert_eq!(code(&antes), 0, "{}", text(&antes.stderr));
    sem_tokens(&antes);
    let antes = json_de(&antes);
    assert_eq!(antes["versao_relatorio"], 1);
    assert_eq!(antes["ligado"], false);
    assert!(antes["corte_do_backfill_ms"].is_null());
    assert!(
        antes["conectado"].is_null(),
        "a CLI não sabe se o Synapse responde"
    );
    assert!(
        antes["versao"].is_i64(),
        "a versão é o updated_at (o ETag da API)"
    );

    // Ligar grava o corte.
    let ligado = cenario.poph(
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
    assert_eq!(code(&ligado), 0, "{}", text(&ligado.stderr));
    sem_tokens(&ligado);
    let ligado = json_de(&ligado);
    assert_eq!(ligado["ligado"], true);
    let corte = ligado["corte_do_backfill_ms"]
        .as_i64()
        .expect("ligar grava o corte do backfill");

    // Repetir ligar é aceito e não move o corte.
    let de_novo = cenario.poph(
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
    assert_eq!(code(&de_novo), 0, "{}", text(&de_novo.stderr));
    assert_eq!(json_de(&de_novo)["corte_do_backfill_ms"], corte);

    // Desligar mantém o corte e não apaga nada.
    let desligado = cenario.poph(
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
    assert_eq!(code(&desligado), 0, "{}", text(&desligado.stderr));
    let desligado = json_de(&desligado);
    assert_eq!(desligado["ligado"], false);
    assert_eq!(
        desligado["corte_do_backfill_ms"], corte,
        "desligar preserva o corte do backfill"
    );

    // O texto do estado mostra o mesmo e avisa que a CLI não sabe a conexão.
    let texto_estado = cenario.poph(
        &["--offline", "--org", "loja-mx23", "matrix", "estado"],
        false,
    );
    assert_eq!(code(&texto_estado), 0, "{}", text(&texto_estado.stderr));
    let saida = text(&texto_estado.stdout);
    assert!(
        saida.contains("ligado:                 não"),
        "saída: {saida}"
    );
    assert!(saida.contains("desconhecido pela CLI"), "saída: {saida}");
    sem_tokens(&texto_estado);
}

#[test]
fn org_e_uso_sem_as_flags_dao_erro_claro() {
    let cenario = Cenario::new("flags", "http://127.0.0.1:9", true);
    cenario.criar_org("existe-mx23");

    let inexistente = cenario.poph(
        &["--offline", "--org", "nao-existe", "matrix", "estado"],
        false,
    );
    assert_eq!(code(&inexistente), 1);
    assert!(
        text(&inexistente.stderr).contains("não existe org com o slug `nao-existe`"),
        "stderr: {}",
        text(&inexistente.stderr)
    );

    let sem_offline = cenario.poph(&["--org", "existe-mx23", "matrix", "ligar"], false);
    assert_eq!(code(&sem_offline), 1);
    assert!(text(&sem_offline.stderr).contains("--offline"));

    let sem_org = cenario.poph(&["--offline", "matrix", "desligar"], false);
    assert_eq!(code(&sem_org), 1);
    assert!(text(&sem_org.stderr).contains("informe --org"));

    // Nada foi gravado por nenhuma dessas chamadas.
    let state = json_de(&cenario.poph(
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

// ───────────────────────────── espaços ─────────────────────────────

#[test]
fn espaco_cria_uma_vez_lista_e_desliga_sem_apagar_no_element() {
    let synapse = subir_synapse();
    let cenario = Cenario::new("espaco", &synapse.endereco, true);
    cenario.criar_org("vendas-mx23");
    let department = cenario.criar_departamento("vendas-mx23", "Vendas");

    // Cria o espaço do departamento.
    let criado = cenario.poph(
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
    assert_eq!(code(&criado), 0, "{}", text(&criado.stderr));
    sem_tokens(&criado);
    let criado = json_de(&criado);
    let space = criado["espaco"].as_str().expect("id do espaço").to_owned();
    assert!(space.starts_with('!'), "o espaço é um id de sala: {space}");
    assert_eq!(criado["origem"], "auto");
    assert_eq!(espacos_no_synapse(&synapse), 1);

    // Repetir não cria segundo espaço: volta o mesmo.
    let repetido = cenario.poph(
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
    assert_eq!(code(&repetido), 0, "{}", text(&repetido.stderr));
    assert_eq!(json_de(&repetido)["espaco"], space);
    assert_eq!(
        espacos_no_synapse(&synapse),
        1,
        "repetir não cria segundo espaço"
    );

    // Lista mostra o espaço com o nome do departamento.
    let lista = cenario.poph(
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
    assert_eq!(code(&lista), 0, "{}", text(&lista.stderr));
    let lista = json_de(&lista);
    let itens = lista["espacos"].as_array().expect("lista de espaços");
    assert_eq!(itens.len(), 1);
    assert_eq!(itens[0]["departamento_nome"], "Vendas");
    assert_eq!(itens[0]["espaco"], space);

    // Desliga: a linha sai do Harness, o espaço continua no Synapse.
    let desligado = cenario.poph(
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
    assert_eq!(code(&desligado), 0, "{}", text(&desligado.stderr));
    let desligado = json_de(&desligado);
    assert_eq!(desligado["removido"], true);
    assert_eq!(
        espacos_no_synapse(&synapse),
        1,
        "desligar não apaga o espaço no Element"
    );

    let vazio = json_de(&cenario.poph(
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
fn espaco_sem_tokens_nomeia_a_variavel_e_nao_grava_nada() {
    let cenario = Cenario::new("sem-tokens", "http://127.0.0.1:9", true);
    cenario.criar_org("sem-token-mx23");
    let department = cenario.criar_departamento("sem-token-mx23", "Suporte");

    let saida = cenario.poph(
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
    sem_tokens(&saida);

    let lista = json_de(&cenario.poph(
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
    assert_eq!(lista["espacos"].as_array().expect("lista").len(), 0);
}

#[test]
fn espaco_com_modulo_desligado_recusa_com_a_instrucao() {
    let cenario = Cenario::new("desligado", "http://127.0.0.1:9", false);
    cenario.criar_org("desligado-mx23");
    let department = cenario.criar_departamento("desligado-mx23", "Vendas");

    let saida = cenario.poph(
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
    sem_tokens(&saida);
}

#[test]
fn espaco_de_departamento_de_outra_org_nao_existe_para_esta_org() {
    let synapse = subir_synapse();
    let cenario = Cenario::new("outra-org", &synapse.endereco, true);
    cenario.criar_org("dona-mx23");
    cenario.criar_org("outra-mx23");
    let departamento_da_outra = cenario.criar_departamento("outra-mx23", "Financeiro");

    let saida = cenario.poph(
        &[
            "--offline",
            "--org",
            "dona-mx23",
            "matrix",
            "espaco",
            "--departamento",
            &departamento_da_outra,
        ],
        true,
    );
    assert_eq!(code(&saida), 1);
    assert!(
        text(&saida.stderr).contains("não existe nesta org"),
        "stderr: {}",
        text(&saida.stderr)
    );
    assert_eq!(
        espacos_no_synapse(&synapse),
        0,
        "nada foi criado no Synapse"
    );

    let desligar = cenario.poph(
        &[
            "--offline",
            "--org",
            "dona-mx23",
            "matrix",
            "espaco",
            "desligar",
            "--departamento",
            &departamento_da_outra,
        ],
        true,
    );
    assert_eq!(code(&desligar), 1);
    assert!(text(&desligar.stderr).contains("não existe nesta org"));
}

#[test]
fn espaco_recusa_combinacoes_e_espaco_invalido() {
    let synapse = subir_synapse();
    let cenario = Cenario::new("combinacoes", &synapse.endereco, true);
    cenario.criar_org("combina-mx23");
    let department = cenario.criar_departamento("combina-mx23", "Vendas");

    // `--espaco` sem `--departamento`: o espaço é de um departamento.
    let sozinho = cenario.poph(
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

    // `espaco desligar` não aceita `--espaco` junto.
    let misturado = cenario.poph(
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

    // Um espaço que não é id nem alias é recusado antes de qualquer chamada ao Synapse.
    let invalido = cenario.poph(
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
    assert_eq!(code(&invalido), 1);
    assert!(
        text(&invalido.stderr).contains("espaço informado inválido"),
        "stderr: {}",
        text(&invalido.stderr)
    );
    sem_tokens(&invalido);
    assert_eq!(espacos_no_synapse(&synapse), 0);
}

// ───────────────────────────── Synapse falso no ar ─────────────────────────────

/// Synapse falso numa porta efêmera, numa thread própria com seu runtime: o teste é síncrono porque
/// roda o `poph` como processo, e o processo fala com o Synapse de verdade por HTTP.
struct Synapse {
    endereco: String,
    state: Arc<Mutex<EstadoFalso>>,
}

fn subir_synapse() -> Synapse {
    let ouvinte = TcpListener::bind("127.0.0.1:0").expect("porta efêmera");
    ouvinte.set_nonblocking(true).expect("não bloqueante");
    let endereco = format!("http://{}", ouvinte.local_addr().expect("endereço"));
    let state = Arc::new(Mutex::new(EstadoFalso::default()));
    let para_a_thread = Arc::clone(&state);
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("runtime do Synapse falso");
        runtime.block_on(async move {
            let listener = tokio::net::TcpListener::from_std(ouvinte).expect("ouvinte");
            let app = Router::new().fallback(atender).with_state(para_a_thread);
            let _ = axum::serve(listener, app).await;
        });
    });
    Synapse { endereco, state }
}
