//! Testes dos usuários virtuais contra um Synapse **falso** (MX-10).
//!
//! O Synapse falso responde um roteiro (uma resposta por requisição) e grava o que chegou: método,
//! caminho, consulta, corpo e se o `Authorization` bateu. Nenhum token é guardado: só o booleano da
//! comparação.
//!
//! O que se prova aqui é o comportamento que só aparece falando com o servidor: registro idempotente
//! (`M_USER_IN_USE` é o caso normal), perfil reescrito só quando o nome muda, cache que evita a
//! requisição e teto do cache na vida real.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::extract::State;
use axum::http::header::AUTHORIZATION;
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use poph_matrix::erro::Classificacao;
use poph_matrix::usuarios::{
    ErroFantasmas, Fantasma, IdsFantasmas, TETO_PADRAO_CACHE, UsuariosVirtuais,
};
use poph_matrix::{ClienteMatrix, ConfigCliente};
use poph_types::Secret;
use serde_json::{Value, json};

/// Valor-canário: o token do teste. Se aparecer numa resposta, o teste acusa.
const CANARIO: &str = "CANARIO-MX10-nao-pode-vazar";

// ───────────────────────────── Synapse falso ─────────────────────────────

#[derive(Clone, Debug)]
struct Resposta {
    status: u16,
    corpo: Value,
}

impl Resposta {
    fn ok(corpo: Value) -> Self {
        Self { status: 200, corpo }
    }

    fn erro(status: u16, corpo: Value) -> Self {
        Self { status, corpo }
    }
}

#[derive(Clone, Debug, Default)]
struct Registro {
    metodo: String,
    caminho: String,
    consulta: String,
    corpo: Value,
    autorizacao_bate: bool,
}

#[derive(Default)]
struct Estado {
    roteiro: VecDeque<Resposta>,
    registros: Vec<Registro>,
    token_esperado: String,
}

async fn atender(
    State(estado): State<Arc<Mutex<Estado>>>,
    metodo: Method,
    uri: Uri,
    cabecalhos: HeaderMap,
    corpo: String,
) -> Response {
    let resposta = {
        let mut guarda = estado.lock().expect("trava do Synapse falso");
        let autorizacao = cabecalhos.get(AUTHORIZATION).and_then(|v| v.to_str().ok());
        let registro = Registro {
            metodo: metodo.to_string(),
            caminho: uri.path().to_owned(),
            consulta: uri.query().unwrap_or_default().to_owned(),
            corpo: serde_json::from_str(&corpo).unwrap_or(Value::Null),
            autorizacao_bate: autorizacao
                .map(|valor| valor == format!("Bearer {}", guarda.token_esperado))
                .unwrap_or(false),
        };
        guarda.registros.push(registro);
        guarda.roteiro.pop_front().unwrap_or_else(|| {
            panic!(
                "chegou requisição sem resposta no roteiro: {metodo} {}",
                uri.path()
            )
        })
    };
    let status = StatusCode::from_u16(resposta.status).expect("status válido");
    (
        status,
        [("content-type", "application/json")],
        resposta.corpo.to_string(),
    )
        .into_response()
}

struct Cenario {
    estado: Arc<Mutex<Estado>>,
    usuarios: UsuariosVirtuais,
}

impl Cenario {
    fn enfileirar(&self, resposta: Resposta) {
        self.estado
            .lock()
            .expect("trava")
            .roteiro
            .push_back(resposta);
    }

    fn registros(&self) -> Vec<Registro> {
        self.estado.lock().expect("trava").registros.clone()
    }

    fn total(&self) -> usize {
        self.registros().len()
    }

    fn ultimo(&self) -> Registro {
        self.registros()
            .pop()
            .expect("deveria ter havido alguma requisição")
    }
}

async fn cenario_com(teto: usize) -> Cenario {
    let estado = Arc::new(Mutex::new(Estado {
        token_esperado: CANARIO.to_owned(),
        ..Default::default()
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
    let config = ConfigCliente::novo(
        format!("http://{endereco}"),
        "matrix-dev.local",
        Secret::new(CANARIO.to_owned()),
    )
    .expect("configuração do cliente");
    let cliente = ClienteMatrix::novo(config).expect("cliente");
    let ids = IdsFantasmas::novo("ph", "matrix-dev.local").expect("ids");
    Cenario {
        estado,
        usuarios: UsuariosVirtuais::novo(cliente, ids, teto),
    }
}

async fn cenario() -> Cenario {
    cenario_com(TETO_PADRAO_CACHE).await
}

fn id_de(prefixo: &str, corpo: &str) -> String {
    format!("@ph_{prefixo}_{corpo}:matrix-dev.local")
}

fn registro_de_criacao(id: &str) -> Resposta {
    Resposta::ok(json!({ "user_id": id }))
}

fn registro_ja_existe() -> Resposta {
    Resposta::erro(
        400,
        json!({ "errcode": "M_USER_IN_USE", "error": "User ID already taken." }),
    )
}

fn perfil_com(nome: &str) -> Resposta {
    Resposta::ok(json!({ "displayname": nome }))
}

fn perfil_sem_nome() -> Resposta {
    Resposta::ok(json!({}))
}

fn nome_aplicado() -> Resposta {
    Resposta::ok(json!({}))
}

// ───────────────────────────── cenários ─────────────────────────────

#[tokio::test]
async fn registro_novo_aplica_o_nome_e_a_segunda_chamada_nao_fala_com_o_synapse() {
    let c = cenario().await;
    let id = id_de("wa", "5511999999999");
    c.enfileirar(registro_de_criacao(&id));
    c.enfileirar(nome_aplicado());

    let fantasma = c
        .usuarios
        .garantir_cliente("+55 (11) 99999-9999", "Maria")
        .await
        .expect("fantasma pronto");
    assert_eq!(fantasma.user_id(), id);

    let registros = c.registros();
    assert_eq!(registros.len(), 2, "registro + nome");
    assert_eq!(registros[0].metodo, "POST");
    assert!(registros[0].caminho.ends_with("/register"), "{registros:?}");
    assert!(registros[0].consulta.contains("kind=user"), "{registros:?}");
    assert_eq!(
        registros[0].corpo["username"],
        Value::from("ph_wa_5511999999999")
    );
    assert_eq!(
        registros[0].corpo["type"],
        Value::from("m.login.application_service")
    );
    assert_eq!(registros[0].corpo["inhibit_login"], Value::Bool(true));
    assert!(registros[0].autorizacao_bate, "token fora do cabeçalho");

    assert_eq!(registros[1].metodo, "PUT");
    assert!(
        registros[1].caminho.ends_with("/displayname"),
        "{registros:?}"
    );
    assert_eq!(
        registros[1].corpo["displayname"],
        Value::from("Maria (WhatsApp)")
    );
    // O fantasma age como ele mesmo: o disfarce aponta para o próprio id.
    assert!(
        registros[1]
            .consulta
            .contains("user_id=%40ph_wa_5511999999999")
            || registros[1]
                .consulta
                .contains("user_id=@ph_wa_5511999999999"),
        "disfarce errado: {}",
        registros[1].consulta
    );

    // Segunda chamada, mesmo nome: nada de requisição nova.
    let de_novo = c
        .usuarios
        .garantir_cliente("5511999999999", "Maria")
        .await
        .expect("fantasma pronto");
    assert_eq!(de_novo, fantasma);
    assert_eq!(c.total(), 2, "o cache deveria ter evitado a segunda rodada");
    assert_eq!(c.usuarios.cache(), (1, TETO_PADRAO_CACHE));
}

#[tokio::test]
async fn fantasma_que_ja_existe_nao_reescreve_o_nome_igual() {
    let c = cenario().await;
    let id = id_de("wa", "5521988887777");
    c.enfileirar(registro_ja_existe());
    c.enfileirar(perfil_com("João (WhatsApp)"));

    let fantasma = c
        .usuarios
        .garantir_cliente("5521988887777", "João")
        .await
        .expect("fantasma pronto");
    assert_eq!(fantasma.user_id(), id);

    let registros = c.registros();
    assert_eq!(registros.len(), 2, "registro + leitura de perfil, sem PUT");
    assert_eq!(registros[0].caminho.rsplit('/').next(), Some("register"));
    assert_eq!(registros[1].metodo, "GET");
    assert!(
        registros[1].caminho.contains(&id),
        "leu o perfil de outro id: {}",
        registros[1].caminho
    );
}

#[tokio::test]
async fn nome_que_mudou_e_reescrito_e_o_nome_novo_e_aplicado_uma_vez() {
    let c = cenario().await;
    let id = id_de("wa", "5511977776666");
    c.enfileirar(registro_ja_existe());
    c.enfileirar(perfil_com("Maria Antiga (WhatsApp)"));
    c.enfileirar(nome_aplicado());
    // Nome do agente muda: já está no cache com o nome anterior e o perfil é reescrito direto,
    // sem nova leitura de perfil.
    c.enfileirar(nome_aplicado());

    c.usuarios
        .garantir_cliente("5511977776666", "Maria")
        .await
        .expect("primeira rodada");
    assert_eq!(c.total(), 3, "registro + perfil + nome");

    c.usuarios
        .garantir_cliente("5511977776666", "Maria Nova")
        .await
        .expect("segunda rodada");
    let registros = c.registros();
    assert_eq!(registros.len(), 4, "só o nome mudou, só um PUT");
    let ultimo = &registros[3];
    assert_eq!(ultimo.metodo, "PUT");
    assert!(ultimo.caminho.ends_with("/displayname"), "{ultimo:?}");
    assert_eq!(
        ultimo.corpo["displayname"],
        Value::from("Maria Nova (WhatsApp)")
    );
    assert!(ultimo.caminho.contains(&id));

    // Terceira chamada com o mesmo nome novo: cache bate, nada de requisição.
    c.usuarios
        .garantir_cliente("5511977776666", "Maria Nova")
        .await
        .expect("terceira rodada");
    assert_eq!(c.total(), 4);
}

#[tokio::test]
async fn fantasma_antigo_sem_nome_recebe_o_nome() {
    let c = cenario().await;
    let id = id_de("wa", "5511944443333");
    c.enfileirar(registro_ja_existe());
    c.enfileirar(perfil_sem_nome());
    c.enfileirar(nome_aplicado());

    c.usuarios
        .garantir_cliente("5511944443333", "  Cliente Novo ")
        .await
        .expect("fantasma pronto");
    let registros = c.registros();
    assert_eq!(registros.len(), 3, "registro + perfil + nome");
    assert_eq!(registros[2].metodo, "PUT");
    assert!(registros[2].caminho.contains(&id));
    assert_eq!(
        registros[2].corpo["displayname"],
        Value::from("Cliente Novo (WhatsApp)")
    );
}

#[tokio::test]
async fn agente_usa_o_nome_do_agente_e_o_mesmo_cache_vale() {
    let c = cenario().await;
    let id_agente = "01912345-6789-7abc-8def-0123456789ab";
    let id = id_de("ag", id_agente);
    c.enfileirar(registro_de_criacao(&id));
    c.enfileirar(nome_aplicado());

    let fantasma = c
        .usuarios
        .garantir_agente(id_agente, "Suporte")
        .await
        .expect("fantasma do agente");
    assert_eq!(fantasma.user_id(), id);
    let registros = c.registros();
    assert_eq!(
        registros[0].corpo["username"],
        Value::from(format!("ph_ag_{id_agente}"))
    );
    assert_eq!(registros[1].corpo["displayname"], Value::from("Suporte"));

    // Nome vazio cai no id do agente, sem sufixo de cliente.
    c.enfileirar(nome_aplicado());
    c.usuarios
        .garantir_agente(id_agente, "")
        .await
        .expect("agente sem nome");
    assert_eq!(c.ultimo().corpo["displayname"], Value::from(id_agente));
}

#[tokio::test]
async fn cache_respeita_o_teto_na_vida_real() {
    let c = cenario_com(2).await;
    let telefones = ["5511900000001", "5511900000002", "5511900000003"];
    for telefone in telefones {
        let id = id_de("wa", telefone);
        c.enfileirar(registro_de_criacao(&id));
        c.enfileirar(nome_aplicado());
        c.usuarios
            .garantir_cliente(telefone, "Cliente")
            .await
            .expect("fantasma pronto");
    }
    assert_eq!(c.total(), 6);
    assert_eq!(c.usuarios.cache(), (2, 2), "o cache passou do teto");

    // O primeiro saiu do cache: volta a conferir no Synapse (registro + perfil), sem PUT porque o
    // nome já está igual.
    c.enfileirar(registro_ja_existe());
    c.enfileirar(perfil_com("Cliente (WhatsApp)"));
    c.usuarios
        .garantir_cliente(telefones[0], "Cliente")
        .await
        .expect("fantasma pronto");
    assert_eq!(c.total(), 8, "o fantasma expirado deveria ser reconferido");
    assert_eq!(c.usuarios.cache().0, 2);

    // O último continua no cache: nenhuma requisição nova.
    c.usuarios
        .garantir_cliente(telefones[2], "Cliente")
        .await
        .expect("fantasma pronto");
    assert_eq!(
        c.total(),
        8,
        "o fantasma que ficou no cache foi reconsultado"
    );
}

#[tokio::test]
async fn id_divergente_do_synapse_e_erro_sem_entrar_no_cache() {
    let c = cenario().await;
    c.enfileirar(registro_de_criacao("@ph_wa_outro:matrix-dev.local"));

    let erro = c
        .usuarios
        .garantir_cliente("5511955554444", "Maria")
        .await
        .expect_err("id divergente deveria falhar");
    assert_eq!(erro, ErroFantasmas::UsuarioDivergente);
    assert_eq!(c.total(), 1, "não pode ter tentado aplicar nome");
    assert_eq!(c.usuarios.cache().0, 0);
}

#[tokio::test]
async fn falha_transitoria_propaga_sem_entrar_no_cache() {
    let c = cenario().await;
    c.enfileirar(Resposta::erro(503, json!({ "error": "indisponível" })));

    let erro = c
        .usuarios
        .garantir_cliente("5511933332222", "Maria")
        .await
        .expect_err("503 deveria falhar");
    match erro {
        ErroFantasmas::Chamada(ref chamada) => {
            assert_eq!(chamada.classificacao(), Classificacao::Transitoria);
            assert!(
                !chamada.to_string().contains(CANARIO),
                "vazou token: {chamada}"
            );
        }
        outro => panic!("esperado erro de chamada: {outro:?}"),
    }
    assert_eq!(c.usuarios.cache().0, 0);
}

#[test]
fn fantasma_montado_nao_mostra_segredo_no_debug() {
    let ids = IdsFantasmas::novo("ph", "matrix-dev.local").expect("ids");
    let fantasma: Fantasma = ids.cliente("5511999999999").expect("cliente");
    let debug = format!("{fantasma:?}");
    assert!(
        debug.contains("@ph_wa_5511999999999:matrix-dev.local"),
        "{debug}"
    );
    assert!(!debug.contains(CANARIO));
}
