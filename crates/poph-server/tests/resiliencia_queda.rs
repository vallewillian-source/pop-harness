//! Resiliência a queda no meio do turno (HR-57).
//!
//! Cada teste sobe um servidor de verdade (`servir_em`, porta efêmera) com o turno montado sobre um
//! **provedor de LLM falso**; nada de rede externa e nenhuma chave. O banco é temporário.
//!
//! O que se prova aqui (o `kill -9` no binário real é o roteiro `/tmp/hr57-run/queda.py`):
//! - turno que ficou `running` de um processo anterior vira `failed` na subida, com motivo, e a
//!   conversa aceita a próxima mensagem imediatamente;
//! - mensagem do cliente nunca se perde;
//! - falha do provedor (fora do ar, tempo esgotado, stream cortado) termina o turno em `failed`
//!   com motivo, sem resposta inventada e sem duplicar a resposta;
//! - a reserva do turno de teste vence, para a conversa não ficar presa em `409` para sempre.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use argon2::Argon2;
use argon2::password_hash::{PasswordHasher, SaltString};
use poph_agent::atores::{Atores, OpcoesAtores};
use poph_agent::config_quente::{ConfigQuente, OpcoesConfigQuente};
use poph_agent::entrada_saida::{Fluxo, OpcoesFluxo};
use poph_agent::limites::{Limitador, OpcoesLimites};
use poph_agent::metricas::{MetricasMemoria, MetricasTurno};
use poph_agent::turno::{Motor, OpcoesTurno, ProcessadorTurno};
use poph_core::llm::{ErroLlm, EventoLlm, LlmProvider, MotivoFim, ProvedorFake, Roteiro};
use poph_server::Estado;
use poph_server::entrada_turno::MOTIVO_INTERROMPIDO;
use poph_store::{
    Direction, NewAgent, NewConversation, NewMessage, NewOrg, Page, Role, Store, TurnState,
};
use poph_types::config::GlobalTokenConfig;
use poph_types::{ConversationId, HarnessConfig, OrgId};
use rand_core::OsRng;
use reqwest::{Client, StatusCode};
use serde_json::Value;
use tokio::sync::oneshot;

const TOKEN_GLOBAL: &str = "pophg_teste-global-hr57";
const PERGUNTA: &str = "qual o prazo de entrega?";
const RESPOSTA: &str = "o prazo é de cinco dias úteis";

fn hash_argon2(valor: &str) -> String {
    let sal = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(valor.as_bytes(), &sal)
        .expect("hash argon2")
        .to_string()
}

/// Corpo multipart montado à mão: `text` só.
fn multipart(texto: &str) -> (String, Vec<u8>) {
    let limite = "hr57-boundary";
    let corpo = format!(
        "--{limite}\r\nContent-Disposition: form-data; name=\"text\"\r\n\r\n{texto}\r\n--{limite}--\r\n"
    )
    .into_bytes();
    (format!("multipart/form-data; boundary={limite}"), corpo)
}

fn pasta_de_teste(rotulo: &str) -> PathBuf {
    let carimbo = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|duracao| duracao.as_nanos())
        .unwrap_or(0);
    let pasta = std::env::temp_dir().join(format!(
        "poph-resiliencia-{rotulo}-{}-{carimbo}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&pasta);
    fs::create_dir_all(pasta.join("dados")).expect("pasta de dados");
    pasta
}

fn configuracao(pasta: &Path) -> HarnessConfig {
    let mut config = HarnessConfig::default();
    config.data.directory = pasta.join("dados");
    config.server.port = 7484;
    config.admin.tokens_globais = vec![GlobalTokenConfig {
        label: "teste".to_owned(),
        hash: hash_argon2(TOKEN_GLOBAL),
    }];
    config.validar().expect("configuração válida");
    config
}

fn abrir_store(pasta: &Path) -> (Arc<Store>, HarnessConfig) {
    let config = configuracao(pasta);
    let store = Arc::new(Store::open(&config).expect("abre o banco"));
    (store, config)
}

fn semear_org_e_agente(store: &Store) -> (OrgId, poph_types::AgentId) {
    let org = store
        .orgs()
        .criar(NewOrg {
            name: "Org HR-57".to_owned(),
            slug: "org-hr57".to_owned(),
            config_json: "{}".to_owned(),
        })
        .expect("org")
        .id;
    let agente = store
        .agents()
        .criar(
            org,
            NewAgent {
                name: "Bia".to_owned(),
                slug: "bia".to_owned(),
                // Sem janela de agrupamento: o teste não espera 2 s por turno.
                grouping_window_ms: 1,
                ..Default::default()
            },
        )
        .expect("agente")
        .id;
    (org, agente)
}

/// Conversa de console (`channel = "test"`) já aberta no banco.
fn abrir_conversa(
    store: &Store,
    org: OrgId,
    agente: poph_types::AgentId,
    canal: &str,
) -> ConversationId {
    store
        .conversas()
        .criar(
            org,
            NewConversation {
                agent_id: agente,
                integration_id: None,
                canal: canal.to_owned(),
                peer_externo: None,
                target_department: None,
            },
        )
        .expect("conversa")
        .id
}

/// Mensagem do cliente gravada, como a que o processo anterior deixou.
fn gravar_pergunta(
    store: &Store,
    org: OrgId,
    conversa: ConversationId,
    texto: &str,
) -> poph_types::MessageId {
    store
        .messages()
        .gravar(
            org,
            NewMessage {
                conversation_id: conversa,
                direction: Direction::Inbound,
                role: Role::User,
                texto: texto.to_owned(),
                attachments_json: "[]".to_owned(),
                id_externo: None,
                turn_id: None,
                origin: poph_store::Origin::Agente,
                author_name: None,
            },
        )
        .expect("mensagem")
        .id
}

struct Servidor {
    base: String,
    cliente: Client,
    store: Arc<Store>,
    org: OrgId,
    agente: poph_types::AgentId,
    desligar: Option<oneshot::Sender<()>>,
    pasta: PathBuf,
}

impl Drop for Servidor {
    fn drop(&mut self) {
        if let Some(desligar) = self.desligar.take() {
            let _ = desligar.send(());
        }
        let _ = fs::remove_dir_all(&self.pasta);
    }
}

impl Servidor {
    /// Sobe o servidor no banco já preparado por `preparar`, com os roteiros do provedor falso.
    async fn subir(
        rotulo: &str,
        preparar: impl FnOnce(&Store) -> (OrgId, poph_types::AgentId),
        roteiros: Vec<Roteiro>,
        validade_da_reserva: Option<Duration>,
    ) -> Self {
        let pasta = pasta_de_teste(rotulo);
        let (store, config) = abrir_store(&pasta);
        let (org, agente) = preparar(&store);

        let fake = Arc::new(ProvedorFake::com_roteiros(roteiros));
        let provedor: Arc<dyn LlmProvider> = fake.clone();
        let motor = Motor::novo(
            Arc::clone(&store),
            ConfigQuente::nova(
                Arc::clone(&store),
                OpcoesConfigQuente::default(),
                MetricasMemoria::proprio(),
            ),
            Arc::new(Limitador::novo(
                OpcoesLimites::default(),
                MetricasMemoria::proprio(),
            )),
            provedor,
            OpcoesTurno::default(),
            MetricasTurno::proprio(),
        );
        let motor = Arc::new(motor);
        let processador = Arc::new(ProcessadorTurno::novo(Arc::clone(&motor)));
        let atores = Atores::novo(
            processador.clone(),
            OpcoesAtores::default(),
            MetricasMemoria::proprio(),
        );
        let fluxo = Fluxo::nova(Arc::clone(&store), atores, OpcoesFluxo::default());
        processador.ligar_fluxo(fluxo.clone());

        let mut estado = Estado::novo(Arc::clone(&store), config)
            .expect("estado do servidor")
            .com_motor(motor)
            .com_turno(fluxo, Duration::from_millis(20));
        if let Some(validade) = validade_da_reserva {
            estado = estado.com_validade_da_reserva(validade);
        }

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("porta efêmera");
        let endereco = listener.local_addr().expect("endereço local");
        let (desligar, sinal) = oneshot::channel::<()>();
        tokio::spawn(async move {
            let _ = poph_server::servir_em(listener, estado, async {
                let _ = sinal.await;
            })
            .await;
        });
        let servidor = Self {
            base: format!("http://{endereco}"),
            cliente: Client::builder()
                .timeout(Duration::from_secs(20))
                .build()
                .expect("cliente HTTP"),
            store,
            org,
            agente,
            desligar: Some(desligar),
            pasta,
        };
        servidor.esperar_subir().await;
        servidor
    }

    async fn esperar_subir(&self) {
        let inicio = Instant::now();
        loop {
            match self
                .cliente
                .get(format!("{}/healthz", self.base))
                .send()
                .await
            {
                Ok(resposta) if resposta.status().is_success() => return,
                _ if inicio.elapsed() < Duration::from_secs(10) => {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                outro => panic!("servidor não subiu: {outro:?}"),
            }
        }
    }

    fn raiz(&self) -> String {
        format!(
            "/admin/v1/orgs/{}/agents/{}/test-chat",
            self.org, self.agente
        )
    }

    async fn criar_conversa(&self) -> String {
        let resposta = self
            .cliente
            .post(format!("{}{}", self.base, self.raiz()))
            .bearer_auth(TOKEN_GLOBAL)
            .send()
            .await
            .expect("criar conversa");
        assert_eq!(resposta.status(), StatusCode::CREATED);
        let corpo: Value = resposta.json().await.expect("json da conversa");
        corpo["id"].as_str().expect("id da conversa").to_owned()
    }

    async fn enviar(&self, conversa: &str, texto: &str) -> reqwest::Response {
        let (tipo, corpo) = multipart(texto);
        self.cliente
            .post(format!("{}{}/{conversa}/messages", self.base, self.raiz()))
            .bearer_auth(TOKEN_GLOBAL)
            .header("content-type", tipo)
            .body(corpo)
            .send()
            .await
            .expect("envia a mensagem")
    }

    /// Espera o último turno sair de `running` e devolve o estado dele.
    async fn esperar_turno_terminar(&self, conversa: ConversationId) -> (String, Option<String>) {
        let inicio = Instant::now();
        loop {
            let turnos = self
                .store
                .turns()
                .listar(self.org, conversa, Page::with_limit(50))
                .expect("turnos");
            if let Some(turno) = turnos.last()
                && turno.estado != TurnState::Running
            {
                return (turno.estado.as_text().to_owned(), turno.error.clone());
            }
            assert!(
                inicio.elapsed() < Duration::from_secs(15),
                "o turno não terminou a tempo: {turnos:?}"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    fn mensagens_de_saida(&self, conversa: ConversationId) -> usize {
        self.store
            .messages()
            .listar(self.org, conversa, Page::with_limit(200))
            .expect("mensagens")
            .iter()
            .filter(|mensagem| mensagem.direction == Direction::Outbound)
            .count()
    }

    fn mensagens_de_entrada(&self, conversa: ConversationId) -> usize {
        self.store
            .messages()
            .listar(self.org, conversa, Page::with_limit(200))
            .expect("mensagens")
            .iter()
            .filter(|mensagem| mensagem.direction == Direction::Inbound)
            .count()
    }
}

fn roteiro_ok() -> Roteiro {
    Roteiro::Eventos(vec![
        EventoLlm::Texto(RESPOSTA.into()),
        EventoLlm::Fim(MotivoFim::Concluido),
    ])
}

// ─────────────────────── turno interrompido por queda ───────────────────────

/// Cenário do `kill -9`: o processo anterior deixou a conversa aberta, a mensagem do cliente
/// gravada e o turno `running`. Na subida, o turno vira falha registrada e a conversa volta a
/// aceitar mensagem na hora.
#[tokio::test]
async fn turno_interrompido_vira_falha_na_subida_e_a_conversa_aceita_a_proxima_mensagem() {
    let (conversa_anterior, turno_anterior) =
        (std::cell::Cell::new(None), std::cell::Cell::new(None));
    let servidor = Servidor::subir(
        "orfao",
        |store| {
            let (org, agente) = semear_org_e_agente(store);
            let conversa = abrir_conversa(store, org, agente, "test");
            gravar_pergunta(store, org, conversa, PERGUNTA);
            let turno = store
                .turns()
                .iniciar(org, conversa, Some(agente))
                .expect("turno do processo anterior");
            conversa_anterior.set(Some(conversa));
            turno_anterior.set(Some(turno.id));
            (org, agente)
        },
        vec![roteiro_ok(), roteiro_ok()],
        None,
    )
    .await;

    let conversa = conversa_anterior.get().expect("conversa semeada");
    let turno = turno_anterior.get().expect("turno semeado");

    // (ii) o turno não fica órfão em "em andamento": vira falha, com o motivo da queda.
    let gravado = servidor
        .store
        .turns()
        .obter(servidor.org, turno)
        .expect("turno gravado");
    assert_eq!(gravado.estado, TurnState::Failed);
    assert_eq!(gravado.error.as_deref(), Some(MOTIVO_INTERROMPIDO));
    assert!(
        gravado.finished_at.is_some(),
        "o turno órfão ganha hora de fim"
    );

    // (i) a mensagem do cliente continua gravada.
    assert_eq!(servidor.mensagens_de_entrada(conversa), 1);
    assert_eq!(servidor.mensagens_de_saida(conversa), 0);

    // (iii) a conversa aceita a próxima mensagem imediatamente (sem 409 herdado da queda).
    let resposta = servidor.enviar(&conversa.to_string(), "e o frete?").await;
    assert_eq!(
        resposta.status(),
        StatusCode::OK,
        "sem 409 turno_em_andamento"
    );
    let corpo = resposta.text().await.expect("corpo do SSE");
    assert!(corpo.contains("event: fim"), "o turno novo fecha: {corpo}");
    assert!(
        corpo.contains("\"state\":\"done\""),
        "o turno novo conclui: {corpo}"
    );

    // (iv) não sai resposta duplicada, e a nova pergunta foi respondida uma vez.
    let (estado, erro) = servidor.esperar_turno_terminar(conversa).await;
    assert_eq!(estado, "done", "último turno concluiu: {erro:?}");
    assert_eq!(
        servidor.mensagens_de_saida(conversa),
        1,
        "uma única resposta do agente na conversa"
    );
}

// ─────────────────────── falha do provedor ───────────────────────

/// Provedor fora do ar: o turno termina `failed` com motivo, a mensagem do cliente fica gravada,
/// nada de resposta inventada, e a próxima mensagem funciona.
#[tokio::test]
async fn modelo_indisponivel_deixa_turno_falho_sem_perder_a_mensagem() {
    let servidor = Servidor::subir(
        "indisponivel",
        semear_org_e_agente,
        vec![
            Roteiro::Falha(ErroLlm::Indisponivel {
                tentativas: 2,
                ultimo: "HTTP 503".to_owned(),
            }),
            roteiro_ok(),
        ],
        None,
    )
    .await;
    let conversa = servidor.criar_conversa().await;

    let resposta = servidor.enviar(&conversa, PERGUNTA).await;
    assert_eq!(resposta.status(), StatusCode::OK);
    let corpo = resposta.text().await.expect("corpo do SSE");
    assert!(
        corpo.contains("\"state\":\"failed\""),
        "o fim do turno diz que falhou: {corpo}"
    );
    assert!(
        !corpo.contains("\"state\":\"done\""),
        "nunca conclui sem resposta: {corpo}"
    );

    let id: ConversationId = conversa.parse().expect("conversa");
    let (estado, erro) = servidor.esperar_turno_terminar(id).await;
    assert_eq!(estado, "failed");
    assert!(
        erro.as_deref().is_some_and(|erro| !erro.is_empty()),
        "o motivo da falha fica gravado: {erro:?}"
    );
    assert_eq!(
        servidor.mensagens_de_entrada(id),
        1,
        "a mensagem não se perde"
    );
    assert_eq!(servidor.mensagens_de_saida(id), 0, "não inventa resposta");

    // A próxima mensagem funciona normalmente.
    let resposta = servidor.enviar(&conversa, "pode tentar de novo?").await;
    assert_eq!(resposta.status(), StatusCode::OK);
    let corpo = resposta.text().await.expect("corpo do SSE");
    assert!(
        corpo.contains("\"state\":\"done\""),
        "segundo turno conclui: {corpo}"
    );
    assert_eq!(
        servidor.mensagens_de_saida(id),
        1,
        "uma resposta, sem duplicar"
    );
}

/// Stream cortado no meio: parte do texto chegou, a conexão caiu. O turno falha com motivo e não
/// grava meia resposta como se fosse a resposta inteira.
#[tokio::test]
async fn stream_cortado_no_meio_deixa_turno_falho_sem_resposta_parcial() {
    let servidor = Servidor::subir(
        "stream-cortado",
        semear_org_e_agente,
        vec![
            Roteiro::Pendurado {
                eventos: vec![EventoLlm::Texto("o prazo é de ".into())],
                depois: Duration::from_millis(50),
                erro: ErroLlm::ConexaoInterrompida("conexão caiu".to_owned()),
            },
            roteiro_ok(),
        ],
        None,
    )
    .await;
    let conversa = servidor.criar_conversa().await;

    let resposta = servidor.enviar(&conversa, PERGUNTA).await;
    assert_eq!(resposta.status(), StatusCode::OK);
    let corpo = resposta.text().await.expect("corpo do SSE");
    assert!(
        corpo.contains("\"state\":\"failed\""),
        "o turno falha: {corpo}"
    );

    let id: ConversationId = conversa.parse().expect("conversa");
    let (estado, erro) = servidor.esperar_turno_terminar(id).await;
    assert_eq!(estado, "failed");
    assert!(
        erro.as_deref().is_some_and(|erro| erro.contains("caiu")),
        "o motivo do corte fica gravado: {erro:?}"
    );
    assert_eq!(
        servidor.mensagens_de_saida(id),
        0,
        "texto parcial não vira resposta na conversa"
    );

    let resposta = servidor.enviar(&conversa, "e aí?").await;
    assert_eq!(resposta.status(), StatusCode::OK);
    let corpo = resposta.text().await.expect("corpo do SSE");
    assert!(corpo.contains("\"state\":\"done\""));
}

// ─────────────────────── saída do console presa na queda ───────────────────────

/// Item de saída de conversa do console que ficou em `sending` quando o processo morreu é
/// confirmado na subida: nada fica preso em `sending` para sempre.
#[tokio::test]
async fn saida_do_console_presa_em_sending_e_confirmada_na_subida() {
    let servidor = Servidor::subir(
        "console-preso",
        |store| {
            let (org, agente) = semear_org_e_agente(store);
            let conversa = abrir_conversa(store, org, agente, "test");
            let turno = store
                .turns()
                .iniciar(org, conversa, Some(agente))
                .expect("turno do processo anterior");
            let (_, item) = store
                .outbox()
                .enfileirar(
                    org,
                    poph_store::NewOutboxItem {
                        conversation_id: conversa,
                        turn_id: turno.id,
                        seq: 0,
                        tipo: "text".to_owned(),
                        content_json: "{\"texto\":\"resposta já mostrada\"}".to_owned(),
                    },
                )
                .expect("enfileira a resposta");
            store
                .outbox()
                .iniciar_envio(org, item.id)
                .expect("marca como em envio");
            (org, agente)
        },
        Vec::new(),
        None,
    )
    .await;

    let inicio = Instant::now();
    loop {
        let enviados = servidor
            .store
            .outbox()
            .contar_por_estado(servidor.org, poph_store::OutboxState::Sent)
            .expect("conta a fila");
        if enviados == 1 {
            break;
        }
        assert!(
            inicio.elapsed() < Duration::from_secs(10),
            "o item preso em `sending` não foi confirmado na subida"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(
        servidor
            .store
            .outbox()
            .contar_por_estado(servidor.org, poph_store::OutboxState::Sending)
            .expect("conta a fila"),
        0,
        "nada fica em sending"
    );
}

// ─────────────────────── reserva de turno de teste ───────────────────────

/// A reserva do turno de teste vence: uma reserva vazada não prende a conversa em `409` para
/// sempre.
#[tokio::test]
async fn reserva_de_turno_de_teste_vence() {
    let pasta = pasta_de_teste("reserva");
    let (store, config) = abrir_store(&pasta);
    let (org, agente) = semear_org_e_agente(&store);
    let conversa = abrir_conversa(&store, org, agente, "test");
    let estado = Estado::novo(store, config)
        .expect("estado")
        .com_validade_da_reserva(Duration::from_millis(60));

    assert!(estado.reservar_turno_de_teste(conversa));
    assert!(
        !estado.reservar_turno_de_teste(conversa),
        "enquanto vale, a segunda aba recebe 409"
    );
    tokio::time::sleep(Duration::from_millis(90)).await;
    assert!(
        estado.reservar_turno_de_teste(conversa),
        "vencida a reserva, a conversa volta a aceitar mensagem"
    );
    estado.liberar_turno_de_teste(conversa);
    assert!(estado.reservar_turno_de_teste(conversa));
    let _ = fs::remove_dir_all(&pasta);
}
