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

// ───────────────────────────── ambiente de teste ─────────────────────────────

/// Uma pasta de teste com `poph.toml` e banco próprios.
struct Cenario {
    pasta: PathBuf,
    config: PathBuf,
}

impl Cenario {
    /// `homeserver` aponta para o Synapse falso (ou para um endereço morto, quando o teste não fala com ele).
    fn novo(rotulo: &str, homeserver: &str, habilitado: bool) -> Self {
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
        assert_eq!(codigo(&saida), 0, "{}", texto(&saida.stderr));
        json_de(&saida)["org"]["id"]
            .as_str()
            .expect("id da org")
            .to_owned()
    }

    /// Cria um departamento pela CLI e devolve o id.
    fn criar_departamento(&self, slug: &str, nome: &str) -> String {
        let saida = self.poph(
            &[
                "--offline",
                "--org",
                slug,
                "department",
                "create",
                "--name",
                nome,
                "--json",
            ],
            false,
        );
        assert_eq!(codigo(&saida), 0, "{}", texto(&saida.stderr));
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

fn codigo(saida: &Output) -> i32 {
    saida.status.code().unwrap_or(-1)
}

fn texto(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn json_de(saida: &Output) -> Value {
    serde_json::from_slice(&saida.stdout)
        .unwrap_or_else(|erro| panic!("a saída não é JSON ({erro}): {}", texto(&saida.stdout)))
}

/// Nenhum valor de token pode aparecer na saída do comando.
fn sem_tokens(saida: &Output) {
    for canario in [AS_TOKEN, HS_TOKEN] {
        assert!(
            !texto(&saida.stdout).contains(canario),
            "token vazou no stdout"
        );
        assert!(
            !texto(&saida.stderr).contains(canario),
            "token vazou no stderr"
        );
    }
}

/// Quantos espaços (`m.space`) o Synapse falso tem.
fn espacos_no_synapse(synapse: &Synapse) -> usize {
    synapse
        .estado
        .lock()
        .expect("trava do Synapse falso")
        .salas
        .values()
        .filter(|sala| sala.tipo.as_deref() == Some("m.space"))
        .count()
}

// ───────────────────────────── estado, ligar e desligar ─────────────────────────────

#[test]
fn estado_ligar_e_desligar_pela_cli_preservam_o_corte() {
    let cenario = Cenario::novo("ligacao", "http://127.0.0.1:9", true);
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
    assert_eq!(codigo(&antes), 0, "{}", texto(&antes.stderr));
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
    assert_eq!(codigo(&ligado), 0, "{}", texto(&ligado.stderr));
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
    assert_eq!(codigo(&de_novo), 0, "{}", texto(&de_novo.stderr));
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
    assert_eq!(codigo(&desligado), 0, "{}", texto(&desligado.stderr));
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
    assert_eq!(codigo(&texto_estado), 0, "{}", texto(&texto_estado.stderr));
    let saida = texto(&texto_estado.stdout);
    assert!(
        saida.contains("ligado:                 não"),
        "saída: {saida}"
    );
    assert!(saida.contains("desconhecido pela CLI"), "saída: {saida}");
    sem_tokens(&texto_estado);
}

#[test]
fn org_e_uso_sem_as_flags_dao_erro_claro() {
    let cenario = Cenario::novo("flags", "http://127.0.0.1:9", true);
    cenario.criar_org("existe-mx23");

    let inexistente = cenario.poph(
        &["--offline", "--org", "nao-existe", "matrix", "estado"],
        false,
    );
    assert_eq!(codigo(&inexistente), 1);
    assert!(
        texto(&inexistente.stderr).contains("não existe org com o slug `nao-existe`"),
        "stderr: {}",
        texto(&inexistente.stderr)
    );

    let sem_offline = cenario.poph(&["--org", "existe-mx23", "matrix", "ligar"], false);
    assert_eq!(codigo(&sem_offline), 1);
    assert!(texto(&sem_offline.stderr).contains("--offline"));

    let sem_org = cenario.poph(&["--offline", "matrix", "desligar"], false);
    assert_eq!(codigo(&sem_org), 1);
    assert!(texto(&sem_org.stderr).contains("informe --org"));

    // Nada foi gravado por nenhuma dessas chamadas.
    let estado = json_de(&cenario.poph(
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
    assert_eq!(estado["ligado"], false);
}

// ───────────────────────────── espaços ─────────────────────────────

#[test]
fn espaco_cria_uma_vez_lista_e_desliga_sem_apagar_no_element() {
    let synapse = subir_synapse();
    let cenario = Cenario::novo("espaco", &synapse.endereco, true);
    cenario.criar_org("vendas-mx23");
    let departamento = cenario.criar_departamento("vendas-mx23", "Vendas");

    // Cria o espaço do departamento.
    let criado = cenario.poph(
        &[
            "--offline",
            "--org",
            "vendas-mx23",
            "matrix",
            "espaco",
            "--departamento",
            &departamento,
            "--json",
        ],
        true,
    );
    assert_eq!(codigo(&criado), 0, "{}", texto(&criado.stderr));
    sem_tokens(&criado);
    let criado = json_de(&criado);
    let espaco = criado["espaco"].as_str().expect("id do espaço").to_owned();
    assert!(
        espaco.starts_with('!'),
        "o espaço é um id de sala: {espaco}"
    );
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
            &departamento,
            "--json",
        ],
        true,
    );
    assert_eq!(codigo(&repetido), 0, "{}", texto(&repetido.stderr));
    assert_eq!(json_de(&repetido)["espaco"], espaco);
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
    assert_eq!(codigo(&lista), 0, "{}", texto(&lista.stderr));
    let lista = json_de(&lista);
    let itens = lista["espacos"].as_array().expect("lista de espaços");
    assert_eq!(itens.len(), 1);
    assert_eq!(itens[0]["departamento_nome"], "Vendas");
    assert_eq!(itens[0]["espaco"], espaco);

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
            &departamento,
            "--json",
        ],
        true,
    );
    assert_eq!(codigo(&desligado), 0, "{}", texto(&desligado.stderr));
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
    let cenario = Cenario::novo("sem-tokens", "http://127.0.0.1:9", true);
    cenario.criar_org("sem-token-mx23");
    let departamento = cenario.criar_departamento("sem-token-mx23", "Suporte");

    let saida = cenario.poph(
        &[
            "--offline",
            "--org",
            "sem-token-mx23",
            "matrix",
            "espaco",
            "--departamento",
            &departamento,
        ],
        false,
    );
    assert_eq!(codigo(&saida), 1);
    let stderr = texto(&saida.stderr);
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
    let cenario = Cenario::novo("desligado", "http://127.0.0.1:9", false);
    cenario.criar_org("desligado-mx23");
    let departamento = cenario.criar_departamento("desligado-mx23", "Vendas");

    let saida = cenario.poph(
        &[
            "--offline",
            "--org",
            "desligado-mx23",
            "matrix",
            "espaco",
            "--departamento",
            &departamento,
        ],
        true,
    );
    assert_eq!(codigo(&saida), 1);
    assert!(
        texto(&saida.stderr).contains("habilitado = true"),
        "stderr: {}",
        texto(&saida.stderr)
    );
    sem_tokens(&saida);
}

#[test]
fn espaco_de_departamento_de_outra_org_nao_existe_para_esta_org() {
    let synapse = subir_synapse();
    let cenario = Cenario::novo("outra-org", &synapse.endereco, true);
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
    assert_eq!(codigo(&saida), 1);
    assert!(
        texto(&saida.stderr).contains("não existe nesta org"),
        "stderr: {}",
        texto(&saida.stderr)
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
    assert_eq!(codigo(&desligar), 1);
    assert!(texto(&desligar.stderr).contains("não existe nesta org"));
}

#[test]
fn espaco_recusa_combinacoes_e_espaco_invalido() {
    let synapse = subir_synapse();
    let cenario = Cenario::novo("combinacoes", &synapse.endereco, true);
    cenario.criar_org("combina-mx23");
    let departamento = cenario.criar_departamento("combina-mx23", "Vendas");

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
    assert_eq!(codigo(&sozinho), 1);
    assert!(texto(&sozinho.stderr).contains("junto com --departamento"));

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
            &departamento,
        ],
        true,
    );
    assert_eq!(codigo(&misturado), 1);
    assert!(texto(&misturado.stderr).contains("sozinho"));

    // Um espaço que não é id nem alias é recusado antes de qualquer chamada ao Synapse.
    let invalido = cenario.poph(
        &[
            "--offline",
            "--org",
            "combina-mx23",
            "matrix",
            "espaco",
            "--departamento",
            &departamento,
            "--espaco",
            "sem-prefixo",
        ],
        true,
    );
    assert_eq!(codigo(&invalido), 1);
    assert!(
        texto(&invalido.stderr).contains("espaço informado inválido"),
        "stderr: {}",
        texto(&invalido.stderr)
    );
    sem_tokens(&invalido);
    assert_eq!(espacos_no_synapse(&synapse), 0);
}

// ───────────────────────────── Synapse falso no ar ─────────────────────────────

/// Synapse falso numa porta efêmera, numa thread própria com seu runtime: o teste é síncrono porque
/// roda o `poph` como processo, e o processo fala com o Synapse de verdade por HTTP.
struct Synapse {
    endereco: String,
    estado: Arc<Mutex<EstadoFalso>>,
}

fn subir_synapse() -> Synapse {
    let ouvinte = TcpListener::bind("127.0.0.1:0").expect("porta efêmera");
    ouvinte.set_nonblocking(true).expect("não bloqueante");
    let endereco = format!("http://{}", ouvinte.local_addr().expect("endereço"));
    let estado = Arc::new(Mutex::new(EstadoFalso::default()));
    let para_a_thread = Arc::clone(&estado);
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
    Synapse { endereco, estado }
}
