//! Receptor do Application Service do Matrix (MX-16).
//!
//! São as rotas que o Synapse chama quando alguém escreve numa sala, mais as duas consultas e o
//! `ping`. Elas **não** pertencem à API administrativa: não usam token de org, não entram no OpenAPI
//! (`/admin/v1/openapi.json`) nem no inventário de `docs/API.md`, e o Console nunca as vê.
//!
//! ## O contrato prático (medido no spike da MX-02)
//!
//! O Synapse trata **qualquer** resposta não-2xx de uma transação como "o Application Service está
//! fora do ar": ele marca o AS como DOWN, para de mandar transação nova e reenvia a mais antiga com
//! espera de 2 s, 4 s, 8 s… até 512 s. Um `4xx` por conteúdo, portanto, bloqueia o atendimento por
//! minutos. Daí as quatro regras deste módulo:
//!
//! 1. `200 {}` só depois de processar de verdade (a MX-18 grava a resposta do atendente antes de
//!    responder);
//! 2. erro transitório **nosso** (banco indisponível) → status `5xx`, para o Synapse reenviar;
//! 3. corpo grande demais, JSON inválido ou transação sem `events` → `200` sem processar, com uma
//!    linha de log só com metadado (`txnId` e tamanho), nunca texto de cliente;
//! 4. `4xx` só para o `hs_token` (401 sem token, 403 com token errado), que é erro de configuração
//!    e some quando o token é corrigido.
//!
//! A retransmissão é idempotente por `txnId`: a transação que já foi processada responde `200` sem
//! chamar a entrega de novo. A memória disso é **limitada** ([`TETO_PADRAO_TRANSACOES`]) e FIFO — se
//! ela esquecer uma transação antiga, o pior caso é processar de novo, nunca perder uma.
//!
//! ## O que esta tarefa não faz
//!
//! Classificar os eventos (MX-17), gravar a resposta do atendente (MX-18) e ligar o módulo no
//! `poph serve` (MX-20). A lista de eventos sai daqui por [`EntregaDeEventos`], injetada por quem
//! liga o módulo — a mesma escolha de fronteira da porta `PortaSalas` da MX-11. Enquanto ninguém
//! injetar um receptor, `Estado::matrix_as` é `None` e as rotas respondem `404`: é o módulo
//! desligado.
//!
//! ## Por que as rotas existem mesmo com o módulo desligado
//!
//! As rotas são registradas sempre e quem responde `404` é o handler, quando não há receptor. Se
//! elas não existissem, `GET` cairia na página do Console embutido (o Console responde qualquer
//! `GET` com o `index.html`) e o Synapse entenderia "usuário existe". Com o handler no caminho, o
//! Synapse recebe o `404` que o contrato pede.

use std::collections::{HashSet, VecDeque};
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use axum::Json;
use axum::Router;
use axum::body::Body;
use axum::extract::{DefaultBodyLimit, Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use futures::StreamExt;
use poph_matrix::ModuloMatrix;
use poph_types::Secret;
use serde_json::{Value, json};

use crate::erro::ApiError;
use crate::estado::Estado;

/// Teto do corpo de uma transação: 1 MiB.
///
/// Uma transação traz os eventos de uma janela de envio do Synapse; 1 MiB cobre com folga um lote
/// de mensagens de texto. Acima disso o corpo é descartado **sem** processar e a resposta é `200`
/// (ver a regra 3 do módulo) — o Synapse não pode receber `413`, que o marcaria como fora do ar.
pub const TETO_CORPO: usize = 1024 * 1024;

/// Quantas transações já processadas a memória do receptor guarda (padrão).
///
/// O `txnId` é um contador pequeno por AS; 4096 cobre com folga a janela de repetição do Synapse.
pub const TETO_PADRAO_TRANSACOES: usize = 4096;

/// Base canônica das rotas do Application Service (PRD §6.3).
pub const BASE_V1: &str = "/_matrix/app/v1";

/// A mesma família de rotas sem o `/v1` (compatibilidade com registro antigo).
pub const BASE_SEM_VERSAO: &str = "/_matrix/app";

/// As duas famílias com o prefixo `/matrix` na frente.
///
/// O registro do Application Service no Synapse de desenvolvimento (e o de produção, que ele
/// reflete) aponta para uma base com `/matrix`, e o Synapse chama `<url>/_matrix/app/v1/…`: sem
/// estas rotas a entrega respondia `404` e a resposta do atendente nunca chegava ao cliente
/// (defeito achado no aceite, MX-31). Atender os dois caminhos evita ter de mexer no registro do
/// Synapse, que é ambiente compartilhado.
pub const BASES_COM_PREFIXO: [&str; 2] = ["/matrix/_matrix/app/v1", "/matrix/_matrix/app"];

/// `errcode` do Matrix para "não é meu" nas consultas de usuário e alias.
const ERRCODE_NAO_ENCONTRADO: &str = "M_NOT_FOUND";

// ───────────────────────────── ponto de extensão ─────────────────────────────

/// Futuro devolvido por [`EntregaDeEventos::entregar`].
///
/// É `Pin<Box<…>>` (e não `async fn` no trait) para o trait continuar utilizável como
/// `Arc<dyn EntregaDeEventos>`: o receptor é um campo do `Estado`, que não é genérico.
pub type FuturoDaEntrega<'a> = Pin<Box<dyn Future<Output = Result<(), ErroEntrega>> + Send + 'a>>;

/// Quem recebe os eventos de uma transação já autenticada, validada e deduplicada.
///
/// A implementação é de quem liga o módulo (`poph-server` na MX-20): ela classifica os eventos
/// (MX-17) e grava a resposta do atendente (MX-18). O receptor só garante o contrato do Synapse —
/// **erro transitório devolvido aqui vira `5xx`**, e a transação não é marcada como processada.
pub trait EntregaDeEventos: Send + Sync {
    /// Processa os eventos de `transacao` (o `txnId` do Synapse, que identifica o lote).
    ///
    /// `Err` significa "não consegui agora": o Synapse vai reenviar a mesma transação. A
    /// implementação precisa ser idempotente por evento, porque o reenvio pode chegar depois de um
    /// processamento parcial.
    fn entregar(&self, transacao: &str, eventos: Vec<Value>) -> FuturoDaEntrega<'_>;
}

/// Falha ao processar uma transação.
///
/// O `motivo` é um código curto nosso (`"banco indisponível"`, `"gravação recusada"`), nunca texto
/// de cliente: ele vai para o log e para lugar nenhum mais.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("não consegui processar a transação do Application Service: {motivo}")]
pub struct ErroEntrega {
    /// Motivo curto, sem dado de cliente.
    pub motivo: String,
}

impl ErroEntrega {
    /// Monta o erro com um motivo curto.
    pub fn novo(motivo: impl Into<String>) -> Self {
        Self {
            motivo: motivo.into(),
        }
    }
}

// ───────────────────────────── receptor ─────────────────────────────

/// O Application Service pronto para atender o Synapse: `hs_token`, prefixo dos fantasmas e a
/// entrega dos eventos.
///
/// O `hs_token` vive em [`Segredo`]: não sai em `Debug`, `Display`, log, erro nem resposta. A
/// comparação com o que chega no cabeçalho é feita em tempo constante.
pub struct ReceptorAs {
    hs_token: Secret,
    prefixo: String,
    server_name: String,
    entrega: Arc<dyn EntregaDeEventos>,
    processadas: Mutex<Processadas>,
}

impl ReceptorAs {
    /// Monta o receptor com o teto padrão de transações lembradas.
    pub fn novo(
        prefixo: impl Into<String>,
        server_name: impl Into<String>,
        hs_token: Secret,
        entrega: Arc<dyn EntregaDeEventos>,
    ) -> Self {
        Self::com_teto(
            prefixo,
            server_name,
            hs_token,
            entrega,
            TETO_PADRAO_TRANSACOES,
        )
    }

    /// Igual a [`ReceptorAs::novo`], com o teto da memória de transações escolhido pelo chamador.
    pub fn com_teto(
        prefixo: impl Into<String>,
        server_name: impl Into<String>,
        hs_token: Secret,
        entrega: Arc<dyn EntregaDeEventos>,
        teto_transacoes: usize,
    ) -> Self {
        Self {
            hs_token,
            prefixo: prefixo.into(),
            server_name: server_name.into(),
            entrega,
            processadas: Mutex::new(Processadas::novo(teto_transacoes)),
        }
    }

    /// Monta o receptor a partir do módulo já preparado (`ModuloMatrix::preparar`), que traz a
    /// configuração validada e os dois tokens lidos do ambiente.
    pub fn do_modulo(modulo: ModuloMatrix, entrega: Arc<dyn EntregaDeEventos>) -> Self {
        Self::novo(
            modulo.config.prefix.clone(),
            modulo.config.server_name.clone(),
            modulo.segredos.hs_token.clone(),
            entrega,
        )
    }

    /// Teto da memória de transações (usado nos testes).
    pub fn teto_de_transacoes(&self) -> usize {
        self.processadas_guardadas()
            .lock()
            .unwrap_or_else(|veneno| veneno.into_inner())
            .teto
    }

    /// Confere o `hs_token` do cabeçalho `Authorization: Bearer <token>`.
    ///
    /// - sem cabeçalho, sem `Bearer` ou com token vazio → `401`;
    /// - token diferente do configurado → `403`.
    ///
    /// A comparação é em tempo constante (`ring`): o conteúdo não vaza pelo tempo de resposta.
    fn autorizar(&self, cabecalhos: &HeaderMap) -> Result<(), ApiError> {
        let valor = cabecalhos
            .get(header::AUTHORIZATION)
            .and_then(|valor| valor.to_str().ok());
        let Some(valor) = valor else {
            return Err(ApiError::nao_autorizado());
        };
        let (esquema, apresentado) = valor.split_once(' ').unwrap_or((valor, ""));
        if !esquema.eq_ignore_ascii_case("bearer") || apresentado.trim().is_empty() {
            return Err(ApiError::nao_autorizado());
        }
        let esperado = self.hs_token.expose().as_bytes();
        if !igual_em_tempo_constante(esperado, apresentado.trim().as_bytes()) {
            return Err(ApiError::proibido("token do Application Service inválido"));
        }
        Ok(())
    }

    /// A transação já foi processada por este processo?
    fn ja_processada(&self, transacao: &str) -> bool {
        self.processadas_guardadas()
            .lock()
            .unwrap_or_else(|veneno| veneno.into_inner())
            .contem(transacao)
    }

    /// Lembra a transação processada (FIFO, com teto).
    fn marcar_processada(&self, transacao: &str) {
        self.processadas_guardadas()
            .lock()
            .unwrap_or_else(|veneno| veneno.into_inner())
            .registrar(transacao);
    }

    /// O `Mutex` do cache nunca fica travado durante `await`; cache envenenado é recuperado (o pior
    /// caso é reprocessar uma transação, nunca recusar uma legítima).
    fn processadas_guardadas(&self) -> &Mutex<Processadas> {
        &self.processadas
    }

    /// O id pertence ao namespace desta instância? (`@<prefixo>_…:<server_name>`)
    fn usuario_do_modulo(&self, usuario: &str) -> bool {
        self.no_namespace(usuario, '@')
    }

    /// O alias pertence ao namespace desta instância? (`#<prefixo>_…:<server_name>`)
    fn alias_do_modulo(&self, alias: &str) -> bool {
        self.no_namespace(alias, '#')
    }

    fn no_namespace(&self, id: &str, sigla: char) -> bool {
        let Some(resto) = id.strip_prefix(sigla) else {
            return false;
        };
        let Some((localpart, dominio)) = resto.split_once(':') else {
            return false;
        };
        let marca = format!("{}_", self.prefixo);
        dominio == self.server_name
            && localpart.len() > marca.len()
            && localpart.starts_with(&marca)
    }
}

/// Compara dois segredos em tempo constante.
///
/// O comprimento é público (é uma chave de origem), o conteúdo não: o laço percorre sempre o mesmo
/// número de bytes e acumula a diferença. Mesma abordagem do segredo do webhook da Evolution
/// (`poph-evolution`); o `verify_slices_are_equal` do `ring` está depreciado e não serve.
fn igual_em_tempo_constante(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diferenca = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diferenca |= x ^ y;
    }
    diferenca == 0
}

impl fmt::Debug for ReceptorAs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ReceptorAs")
            .field("prefixo", &self.prefixo)
            .field("server_name", &self.server_name)
            .field("hs_token", &self.hs_token)
            .finish_non_exhaustive()
    }
}

/// Memória FIFO das transações já processadas, com teto.
#[derive(Debug)]
struct Processadas {
    teto: usize,
    ordem: VecDeque<String>,
    conjunto: HashSet<String>,
}

impl Processadas {
    fn novo(teto: usize) -> Self {
        Self {
            // Teto zero não faz sentido (nada seria lembrado): vale 1.
            teto: teto.max(1),
            ordem: VecDeque::new(),
            conjunto: HashSet::new(),
        }
    }

    fn contem(&self, transacao: &str) -> bool {
        self.conjunto.contains(transacao)
    }

    fn registrar(&mut self, transacao: &str) {
        if self.conjunto.contains(transacao) {
            return;
        }
        while self.ordem.len() >= self.teto {
            match self.ordem.pop_front() {
                Some(antiga) => {
                    self.conjunto.remove(&antiga);
                }
                None => break,
            }
        }
        self.ordem.push_back(transacao.to_owned());
        self.conjunto.insert(transacao.to_owned());
    }
}

// ───────────────────────────── rotas ─────────────────────────────

/// Rotas do Application Service, no estado `Estado` (quem aplica é o roteador principal).
///
/// Registra a família canônica (`/_matrix/app/v1/…`), a mesma sem o `/v1`, a raiz pura
/// (`/transactions/…` — PRD §6.3) e as duas com o prefixo `/matrix` na frente
/// ([`BASES_COM_PREFIXO`]). As rotas com e sem prefixo respondem igual: mesmo `hs_token`, mesma
/// idempotência por `txnId` e mesmo `200`/`503`. O limite de corpo do `axum` é desligado nessas
/// rotas de propósito: o teto é aplicado aqui na mão (ver [`ler_corpo`]), porque um `413` do `axum`
/// derrubaria o AS para o Synapse.
pub fn rotas() -> Router<Estado> {
    let mut rotas = Router::new();
    let bases = [BASE_V1, BASE_SEM_VERSAO, ""]
        .into_iter()
        .chain(BASES_COM_PREFIXO);
    for base in bases {
        rotas = rotas
            .route(&format!("{base}/transactions/{{txn_id}}"), put(transacao))
            .route(&format!("{base}/users/{{user_id}}"), get(usuario))
            .route(&format!("{base}/rooms/{{room_alias}}"), get(alias))
            .route(&format!("{base}/ping"), post(ping));
    }
    rotas.layer(DefaultBodyLimit::disable())
}

/// `PUT …/transactions/{txnId}` — o lote de eventos que o Synapse entrega.
async fn transacao(
    State(estado): State<Estado>,
    Path(txn_id): Path<String>,
    cabecalhos: HeaderMap,
    corpo: Body,
) -> Response {
    atender_transacao(estado.matrix_as.as_deref(), &txn_id, &cabecalhos, corpo).await
}

/// `GET …/users/{userId}` — "este usuário é seu?".
async fn usuario(
    State(estado): State<Estado>,
    Path(usuario): Path<String>,
    cabecalhos: HeaderMap,
) -> Response {
    atender_usuario(estado.matrix_as.as_deref(), &usuario, &cabecalhos)
}

/// `GET …/rooms/{roomAlias}` — "este alias é seu?".
async fn alias(
    State(estado): State<Estado>,
    Path(alias): Path<String>,
    cabecalhos: HeaderMap,
) -> Response {
    atender_alias(estado.matrix_as.as_deref(), &alias, &cabecalhos)
}

/// `POST …/ping` — o Synapse não usa hoje (spike da MX-02), mas a rota é barata de manter.
async fn ping(State(estado): State<Estado>, cabecalhos: HeaderMap) -> Response {
    atender_ping(estado.matrix_as.as_deref(), &cabecalhos)
}

// ───────────────────────────── atendimento ─────────────────────────────
//
// As funções abaixo recebem o receptor já resolvido (e não o `Estado`) para poderem ser chamadas
// direto no teste de unidade, sem banco nem servidor.

pub(crate) async fn atender_transacao(
    receptor: Option<&ReceptorAs>,
    txn_id: &str,
    cabecalhos: &HeaderMap,
    corpo: Body,
) -> Response {
    let Some(receptor) = receptor else {
        return modulo_desligado();
    };
    if let Err(erro) = receptor.autorizar(cabecalhos) {
        return erro.into_response();
    }

    let Some(bytes) = ler_corpo(corpo, TETO_CORPO).await else {
        tracing::warn!(
            txn = %txn_id,
            teto_bytes = TETO_CORPO,
            "transação do Application Service acima do limite: descartada sem processar"
        );
        return aceito();
    };
    let Some(eventos) = eventos_da_transacao(&bytes) else {
        tracing::warn!(
            txn = %txn_id,
            bytes = bytes.len(),
            "transação do Application Service malformada: descartada sem processar"
        );
        return aceito();
    };
    if receptor.ja_processada(txn_id) {
        tracing::debug!(
            txn = %txn_id,
            "transação do Application Service repetida: respondendo sem reprocessar"
        );
        return aceito();
    }

    let quantidade = eventos.len();
    match receptor.entrega.entregar(txn_id, eventos).await {
        Ok(()) => {
            // Só depois de processar: se o processo cair antes disto, o reenvio do Synapse
            // reprocessa (a entrega é idempotente por evento).
            receptor.marcar_processada(txn_id);
            tracing::debug!(
                txn = %txn_id,
                eventos = quantidade,
                "transação do Application Service processada"
            );
            aceito()
        }
        Err(erro) => {
            tracing::warn!(
                txn = %txn_id,
                motivo = %erro.motivo,
                "transação do Application Service não foi processada: 5xx para o Synapse reenviar"
            );
            ApiError::novo(
                StatusCode::SERVICE_UNAVAILABLE,
                "matrix_indisponivel",
                "não consegui processar a transação agora; o Synapse vai reenviar",
            )
            .into_response()
        }
    }
}

pub(crate) fn atender_usuario(
    receptor: Option<&ReceptorAs>,
    usuario: &str,
    cabecalhos: &HeaderMap,
) -> Response {
    let Some(receptor) = receptor else {
        return modulo_desligado();
    };
    if let Err(erro) = receptor.autorizar(cabecalhos) {
        return erro.into_response();
    }
    if receptor.usuario_do_modulo(usuario) {
        aceito()
    } else {
        nao_e_meu("usuário")
    }
}

pub(crate) fn atender_alias(
    receptor: Option<&ReceptorAs>,
    alias: &str,
    cabecalhos: &HeaderMap,
) -> Response {
    let Some(receptor) = receptor else {
        return modulo_desligado();
    };
    if let Err(erro) = receptor.autorizar(cabecalhos) {
        return erro.into_response();
    }
    if receptor.alias_do_modulo(alias) {
        aceito()
    } else {
        nao_e_meu("alias de sala")
    }
}

pub(crate) fn atender_ping(receptor: Option<&ReceptorAs>, cabecalhos: &HeaderMap) -> Response {
    let Some(receptor) = receptor else {
        return modulo_desligado();
    };
    if let Err(erro) = receptor.autorizar(cabecalhos) {
        return erro.into_response();
    }
    aceito()
}

/// `200 {}` — o que o Synapse espera de uma transação aceita.
fn aceito() -> Response {
    Json(json!({})).into_response()
}

/// `404` do módulo desligado: nesta instância as rotas do Application Service não existem.
fn modulo_desligado() -> Response {
    ApiError::novo(
        StatusCode::NOT_FOUND,
        "not_found",
        "o módulo Matrix está desligado nesta instância",
    )
    .into_response()
}

/// `404` de "não é do meu namespace", com o `errcode` do Matrix junto.
fn nao_e_meu(o_que: &str) -> Response {
    ApiError::novo(
        StatusCode::NOT_FOUND,
        "not_found",
        format!("este {o_que} não pertence a esta instância"),
    )
    .com_detalhes(json!({ "errcode": ERRCODE_NAO_ENCONTRADO }))
    .into_response()
}

/// Lê o corpo até `teto` bytes. `None` = passou do teto (ou a leitura falhou).
async fn ler_corpo(corpo: Body, teto: usize) -> Option<axum::body::Bytes> {
    let mut fluxo = corpo.into_data_stream();
    let mut dados: Vec<u8> = Vec::new();
    while let Some(parte) = fluxo.next().await {
        let parte = parte.ok()?;
        if dados.len() + parte.len() > teto {
            return None;
        }
        dados.extend_from_slice(&parte);
    }
    Some(dados.into())
}

/// Extrai os `events` do corpo de uma transação. `None` = corpo malformado.
fn eventos_da_transacao(bytes: &[u8]) -> Option<Vec<Value>> {
    let valor: Value = serde_json::from_slice(bytes).ok()?;
    match valor.get("events") {
        Some(Value::Array(eventos)) => Some(eventos.clone()),
        _ => None,
    }
}

#[cfg(test)]
mod testes {
    use std::io::Write;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    use poph_types::Sensitive;

    use super::*;

    /// Valor-canário: se aparecer em resposta, log ou erro, o teste acusa.
    const CANARIO: &str = "CANARIO-HS-TOKEN-4c1f-TAO-SECRETO";

    /// Entrega que só anota o que chegou (e pode fingir falha).
    #[derive(Default)]
    struct Anotadora {
        chamadas: Mutex<Vec<(String, Vec<Value>)>>,
        falhar: AtomicBool,
    }

    impl Anotadora {
        fn chamadas(&self) -> usize {
            self.chamadas.lock().expect("trava").len()
        }
    }

    impl EntregaDeEventos for Anotadora {
        fn entregar(&self, transacao: &str, eventos: Vec<Value>) -> FuturoDaEntrega<'_> {
            if self.falhar.load(Ordering::SeqCst) {
                return Box::pin(async { Err(ErroEntrega::novo("banco indisponível")) });
            }
            self.chamadas
                .lock()
                .expect("trava")
                .push((transacao.to_owned(), eventos));
            Box::pin(async { Ok(()) })
        }
    }

    fn receptor(entrega: Arc<Anotadora>, teto: usize) -> ReceptorAs {
        ReceptorAs::com_teto(
            "ph",
            "matrix-dev.local",
            Sensitive::new(CANARIO.to_owned()),
            entrega,
            teto,
        )
    }

    fn cabecalho_bearer(token: &str) -> HeaderMap {
        let mut cabecalhos = HeaderMap::new();
        cabecalhos.insert(
            header::AUTHORIZATION,
            format!("Bearer {token}").parse().expect("cabeçalho"),
        );
        cabecalhos
    }

    fn transacao_com(evento: &str) -> Body {
        Body::from(
            json!({ "events": [{ "type": "m.room.message", "event_id": evento }] }).to_string(),
        )
    }

    fn status(resposta: &Response) -> StatusCode {
        resposta.status()
    }

    #[test]
    fn memoria_respeita_o_teto_e_esquece_a_mais_antiga() {
        let mut memoria = Processadas::novo(2);
        memoria.registrar("1");
        memoria.registrar("2");
        assert!(memoria.contem("1") && memoria.contem("2"));
        memoria.registrar("3");
        assert!(!memoria.contem("1"), "a mais antiga devia ter saído");
        assert!(memoria.contem("2") && memoria.contem("3"));
        assert_eq!(memoria.ordem.len(), 2, "a memória cresceu além do teto");
        // Registrar de novo o que já está não muda nada.
        memoria.registrar("3");
        assert_eq!(memoria.ordem.len(), 2);
    }

    #[test]
    fn teto_zero_vale_um() {
        assert_eq!(Processadas::novo(0).teto, 1);
    }

    #[test]
    fn namespace_do_usuario_e_do_alias() {
        let anotadora = Arc::new(Anotadora::default());
        let receptor = receptor(Arc::clone(&anotadora), 8);
        assert!(receptor.usuario_do_modulo("@ph_wa_5511999999999:matrix-dev.local"));
        assert!(receptor.usuario_do_modulo("@ph_ag_agt_1:matrix-dev.local"));
        assert!(receptor.alias_do_modulo("#ph_c_0123456789abcdef:matrix-dev.local"));
        // Fora do namespace: outro prefixo, outro domínio, id sem domínio, id vazio.
        for fora in [
            "@wa_5511999999999:matrix-dev.local",
            "@ph_wa_1:outro.servidor",
            "@ph_wa_1",
            "@ph_:matrix-dev.local",
            "ph_wa_1:matrix-dev.local",
        ] {
            assert!(
                !receptor.usuario_do_modulo(fora),
                "`{fora}` não podia ser do módulo"
            );
        }
        assert!(!receptor.alias_do_modulo("@ph_c_1:matrix-dev.local"));
    }

    #[tokio::test]
    async fn sem_token_da_401_e_token_errado_da_403() {
        let anotadora = Arc::new(Anotadora::default());
        let receptor = receptor(Arc::clone(&anotadora), 8);

        let sem_cabecalho =
            atender_transacao(Some(&receptor), "1", &HeaderMap::new(), transacao_com("$a")).await;
        assert_eq!(status(&sem_cabecalho), StatusCode::UNAUTHORIZED);

        let mut so_bearer = HeaderMap::new();
        so_bearer.insert(header::AUTHORIZATION, "Bearer ".parse().expect("cabeçalho"));
        let vazio = atender_transacao(Some(&receptor), "1", &so_bearer, transacao_com("$a")).await;
        assert_eq!(status(&vazio), StatusCode::UNAUTHORIZED);

        // Mesmo tamanho e tamanho diferente: os dois são 403, sem pânico.
        for errado in [format!("{CANARIO}x"), "curto".to_owned()] {
            let resposta = atender_transacao(
                Some(&receptor),
                "1",
                &cabecalho_bearer(&errado),
                transacao_com("$a"),
            )
            .await;
            assert_eq!(status(&resposta), StatusCode::FORBIDDEN, "token `{errado}`");
        }
        assert_eq!(anotadora.chamadas(), 0, "nada podia ter sido processado");
    }

    #[tokio::test]
    async fn transacao_valida_processa_uma_vez_e_a_repeticao_nao_reprocessa() {
        let anotadora = Arc::new(Anotadora::default());
        let receptor = receptor(Arc::clone(&anotadora), 8);
        let cabecalhos = cabecalho_bearer(CANARIO);

        let primeira =
            atender_transacao(Some(&receptor), "42", &cabecalhos, transacao_com("$a")).await;
        assert_eq!(status(&primeira), StatusCode::OK);
        assert_eq!(anotadora.chamadas(), 1);

        let segunda =
            atender_transacao(Some(&receptor), "42", &cabecalhos, transacao_com("$a")).await;
        assert_eq!(status(&segunda), StatusCode::OK);
        assert_eq!(anotadora.chamadas(), 1, "a repetição não podia reprocessar");

        // Transação diferente com o mesmo corpo processa normalmente.
        let outra =
            atender_transacao(Some(&receptor), "43", &cabecalhos, transacao_com("$a")).await;
        assert_eq!(status(&outra), StatusCode::OK);
        assert_eq!(anotadora.chamadas(), 2);
    }

    #[tokio::test]
    async fn falha_da_entrega_da_5xx_e_nao_marca_a_transacao() {
        let anotadora = Arc::new(Anotadora::default());
        let receptor = receptor(Arc::clone(&anotadora), 8);
        let cabecalhos = cabecalho_bearer(CANARIO);

        anotadora.falhar.store(true, Ordering::SeqCst);
        let falhou =
            atender_transacao(Some(&receptor), "7", &cabecalhos, transacao_com("$a")).await;
        assert_eq!(status(&falhou), StatusCode::SERVICE_UNAVAILABLE);

        // O reenvio do Synapse tem de ser processado: a falha não pode ter marcado a transação.
        anotadora.falhar.store(false, Ordering::SeqCst);
        let reenviada =
            atender_transacao(Some(&receptor), "7", &cabecalhos, transacao_com("$a")).await;
        assert_eq!(status(&reenviada), StatusCode::OK);
        assert_eq!(anotadora.chamadas(), 1);
    }

    #[tokio::test]
    async fn corpo_malformado_ou_grande_responde_200_sem_processar() {
        let anotadora = Arc::new(Anotadora::default());
        let receptor = receptor(Arc::clone(&anotadora), 8);
        let cabecalhos = cabecalho_bearer(CANARIO);

        for corpo in [
            Body::from("isto não é json"),
            Body::from("{ \"events\": \"não é lista\" }"),
            Body::from("{}"),
            Body::from(""),
        ] {
            let resposta = atender_transacao(Some(&receptor), "9", &cabecalhos, corpo).await;
            assert_eq!(status(&resposta), StatusCode::OK, "corpo malformado");
        }

        let grande = Body::from(vec![b'x'; TETO_CORPO + 1]);
        let resposta = atender_transacao(Some(&receptor), "10", &cabecalhos, grande).await;
        assert_eq!(status(&resposta), StatusCode::OK, "corpo acima do teto");
        assert_eq!(anotadora.chamadas(), 0, "nada podia ter sido processado");
    }

    #[tokio::test]
    async fn modulo_desligado_da_404_nas_quatro_rotas() {
        let cabecalhos = cabecalho_bearer(CANARIO);
        let transacoes = atender_transacao(None, "1", &cabecalhos, transacao_com("$a")).await;
        assert_eq!(status(&transacoes), StatusCode::NOT_FOUND);
        let usuario = atender_usuario(None, "@ph_wa_1:matrix-dev.local", &cabecalhos);
        assert_eq!(status(&usuario), StatusCode::NOT_FOUND);
        let alias = atender_alias(None, "#ph_c_1:matrix-dev.local", &cabecalhos);
        assert_eq!(status(&alias), StatusCode::NOT_FOUND);
        let ping = atender_ping(None, &cabecalhos);
        assert_eq!(status(&ping), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn valor_canario_nunca_aparece_em_resposta_nem_log() {
        let anotadora = Arc::new(Anotadora::default());
        let receptor = receptor(Arc::clone(&anotadora), 8);

        let capturado = Arc::new(Mutex::new(Vec::<u8>::new()));
        let assinante = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::TRACE)
            .with_ansi(false)
            .with_writer({
                let capturado = Arc::clone(&capturado);
                move || Escritor(Arc::clone(&capturado))
            })
            .finish();
        let _guarda = tracing::subscriber::set_default(assinante);

        let mut corpos = Vec::new();
        // Token certo (200), token errado (403), sem token (401) e corpo malformado (200).
        for (cabecalhos, corpo) in [
            (cabecalho_bearer(CANARIO), transacao_com("$a")),
            (cabecalho_bearer("outro-token"), transacao_com("$a")),
            (HeaderMap::new(), transacao_com("$a")),
            (cabecalho_bearer(CANARIO), Body::from("lixo")),
        ] {
            let resposta = atender_transacao(Some(&receptor), "77", &cabecalhos, corpo).await;
            let bytes = axum::body::to_bytes(resposta.into_body(), 64 * 1024)
                .await
                .expect("corpo da resposta");
            corpos.push(String::from_utf8_lossy(&bytes).to_string());
        }

        let log = String::from_utf8(capturado.lock().expect("trava").clone()).expect("utf-8");
        assert!(!log.trim().is_empty(), "o teste precisa de log para provar");
        for corpo in &corpos {
            assert!(!corpo.contains(CANARIO), "vazou na resposta: {corpo}");
        }
        assert!(!log.contains(CANARIO), "vazou no log: {log}");
        // O `Debug` do receptor também não pode mostrar o token.
        assert!(!format!("{receptor:?}").contains(CANARIO));
    }

    /// Escritor de log em memória para o teste do valor-canário.
    struct Escritor(Arc<Mutex<Vec<u8>>>);

    impl Write for Escritor {
        fn write(&mut self, dados: &[u8]) -> std::io::Result<usize> {
            self.0.lock().expect("trava").extend_from_slice(dados);
            Ok(dados.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
}
