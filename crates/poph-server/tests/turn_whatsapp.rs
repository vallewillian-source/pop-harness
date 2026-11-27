
use std::collections::VecDeque;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, SystemTime};

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::post;
use axum::{Json, Router};
use poph_agent::actors::{ActorOptions, Actors};
use poph_agent::config_hot::{ConfigHot, HotConfigOptions};
use poph_agent::entry_output::{FlowOptions, Fluxo};
use poph_agent::limites::{LimitOptions, Limiter};
use poph_agent::metrics::{MetricsFlow, MetricsMemory, MetricsTurn};
use poph_agent::turno::{TurnEngine, TurnOptions, TurnProcessor};
use poph_core::llm::{FakeProvider, FinishReason, LlmEvent, LlmProvider, Script, Usage};
use poph_evolution::webhook::SECRET_HEADER;
use poph_evolution::{ATTACHMENT_CAP, ConfigEvolution, EvolutionClient};
use poph_server::ServerState;
use poph_server::evolution::EvolutionChannel;
use poph_store::{
    ConversationStatus, Direction, NewAgent, NewConversation, NewOrg, NewOutboxItem, Origin,
    OutboxState, Page, Store, TurnState,
};
use poph_types::{AgentId, ConversationId, HarnessConfig, IntegrationId, OrgId, Registry, Secret};
use reqwest::Client;
use serde_json::{Value, json};
use tokio::sync::oneshot;
use tracing_subscriber::fmt::MakeWriter;

const SEGREDO: &str = "segredo-webhook-turno-26h";
const CHAVE_EVO: &str = "chave-evo-turno-26h-teste";
const TELEFONE: &str = "5511999990001";
const JID: &str = "5511999990001@s.whatsapp.net";
const PERGUNTA: &str = "qual o prazo de entrega do pedido quatro quatro sete um";
const RESPOSTA: &str = "o prazo é de cinco dias úteis após a confirmação";
const INSTANCIA: &str = "loja";


#[derive(Clone)]
struct Captura(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for Captura {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(|veneno| veneno.into_inner())
            .extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for Captura {
    type Writer = Captura;
    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

fn logs_capturados() -> Arc<Mutex<Vec<u8>>> {
    static LOGS: OnceLock<Arc<Mutex<Vec<u8>>>> = OnceLock::new();
    LOGS.get_or_init(|| {
        let buffer = Arc::new(Mutex::new(Vec::new()));
        let _ = tracing_subscriber::fmt()
            .with_writer(Captura(Arc::clone(&buffer)))
            .with_ansi(false)
            .with_max_level(tracing::Level::DEBUG)
            .try_init();
        buffer
    })
    .clone()
}

fn logs_ate_agora() -> String {
    let bytes = logs_capturados()
        .lock()
        .unwrap_or_else(|veneno| veneno.into_inner())
        .clone();
    String::from_utf8_lossy(&bytes).into_owned()
}


#[derive(Debug, Clone)]
struct Envio {
    apikey: Option<String>,
    corpo: Value,
}

#[derive(Clone, Default)]
struct EvolucaoFalsa {
    envios: Arc<Mutex<Vec<Envio>>>,
    roteiro: Arc<Mutex<VecDeque<u16>>>,
    contador: Arc<AtomicUsize>,
}

impl EvolucaoFalsa {
    fn envios(&self) -> Vec<Envio> {
        self.envios
            .lock()
            .unwrap_or_else(|veneno| veneno.into_inner())
            .clone()
    }
}

async fn enviar_falso(
    State(evo): State<EvolucaoFalsa>,
    Path(_instancia): Path<String>,
    cabecalhos: HeaderMap,
    Json(corpo): Json<Value>,
) -> (StatusCode, Json<Value>) {
    let apikey = cabecalhos
        .get("apikey")
        .and_then(|valor| valor.to_str().ok())
        .map(str::to_owned);
    evo.envios
        .lock()
        .unwrap_or_else(|veneno| veneno.into_inner())
        .push(Envio { apikey, corpo });
    let n = evo.contador.fetch_add(1, Ordering::SeqCst) + 1;
    let status = evo
        .roteiro
        .lock()
        .unwrap_or_else(|veneno| veneno.into_inner())
        .pop_front()
        .unwrap_or(200);
    (
        StatusCode::from_u16(status).expect("status do roteiro"),
        Json(json!({"key": {"id": format!("MSG-SAIDA-{n}")}})),
    )
}

async fn subir_evolucao_falsa(evo: EvolucaoFalsa) -> String {
    let roteador = Router::new()
        .route("/message/sendText/{instancia}", post(enviar_falso))
        .route("/message/sendMedia/{instancia}", post(enviar_falso))
        .with_state(evo);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("porta efêmera da Evolution falsa");
    let endereco = listener.local_addr().expect("endereço da Evolution falsa");
    tokio::spawn(async move {
        let _ = axum::serve(listener, roteador).await;
    });
    format!("http://{endereco}")
}


struct Cenario {
    pasta: PathBuf,
    store: Arc<Store>,
    org: OrgId,
    agente: AgentId,
    integracao: IntegrationId,
    evo: EvolucaoFalsa,
    url_evo: String,
}

impl Cenario {
    async fn novo(rotulo: &str) -> Self {
        logs_capturados();
        let carimbo = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let pasta = std::env::temp_dir().join(format!(
            "poph-turno-26h-{rotulo}-{}-{carimbo}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&pasta);
        fs::create_dir_all(&pasta).expect("pasta do teste");
        let store = Arc::new(
            Store::open_at(&pasta.join("harness.db"), &pasta.join("rag.db")).expect("abre o banco"),
        );
        let org = store
            .orgs()
            .create(NewOrg {
                name: "Loja Turno".to_owned(),
                slug: format!("loja-turno-{rotulo}"),
                config_json: String::new(),
            })
            .expect("cria a org")
            .id;
        let agente = store
            .agents()
            .create(
                org,
                NewAgent {
                    name: "Atendente Loja".to_owned(),
                    slug: "atendente-loja".to_owned(),
                    modelo: "gemini-3.8-flash-tiered".to_owned(),
                    grouping_window_ms: 0,
                    ..Default::default()
                },
            )
            .expect("cria o agente")
            .id;
        store
            .instructions()
            .save(
                org,
                agente,
                0,
                "{}",
                "# Atendente\n\nVocê atende a loja.",
                true,
            )
            .expect("grava as instruções");
        let integracao: IntegrationId = store
            .create_integration(org, agente, "whatsapp".to_owned(), "active".to_owned())
            .expect("cria a integração")
            .id
            .parse()
            .expect("id da integração");

        let evo = EvolucaoFalsa::default();
        let url_evo = subir_evolucao_falsa(evo.clone()).await;
        Self {
            pasta,
            store,
            org,
            agente,
            integracao,
            evo,
            url_evo,
        }
    }

    fn canal(&self) -> EvolutionChannel {
        let cliente = EvolutionClient::new(
            ConfigEvolution::new(
                self.url_evo.clone(),
                INSTANCIA,
                Secret::new(CHAVE_EVO.to_owned()),
            )
            .expect("config da Evolution falsa"),
        )
        .expect("cliente da Evolution falsa");
        EvolutionChannel {
            org: self.org,
            instance: INSTANCIA.to_owned(),
            integration: self.integracao,
            secret: Some(Secret::new(SEGREDO.to_owned())),
            client: Some(cliente),
            attachment_limit: ATTACHMENT_CAP,
        }
    }

    fn fluxo(
        &self,
        registro: &Registry,
        roteiros: Vec<Script>,
        opcoes: FlowOptions,
    ) -> (Fluxo, Arc<FakeProvider>) {
        let fake = Arc::new(FakeProvider::with_scripts(roteiros));
        let provedor: Arc<dyn LlmProvider> = fake.clone();
        let config = ConfigHot::new(
            Arc::clone(&self.store),
            HotConfigOptions::default(),
            MetricsMemory::own(),
        );
        let limitador = Arc::new(Limiter::new(LimitOptions::default(), MetricsMemory::own()));
        let motor = TurnEngine::new(
            Arc::clone(&self.store),
            config,
            limitador,
            provedor,
            TurnOptions::default(),
            MetricsTurn::register(registro).expect("métricas do turno"),
        );
        let processador = Arc::new(TurnProcessor::new(Arc::new(motor)));
        let atores = Actors::new(
            processador.clone(),
            ActorOptions::default(),
            MetricsMemory::own(),
        );
        let fluxo = Fluxo::with_metrics(
            Arc::clone(&self.store),
            atores,
            opcoes,
            MetricsFlow::register(registro).expect("métricas da fila"),
        );
        processador.attach_io(fluxo.clone());
        (fluxo, fake)
    }

    async fn subir(&self, roteiros: Vec<Script>, ligar_turno: bool) -> Servidor {
        self.subir_com(roteiros, ligar_turno, FlowOptions::default())
            .await
    }

    async fn subir_com(
        &self,
        roteiros: Vec<Script>,
        ligar_turno: bool,
        opcoes: FlowOptions,
    ) -> Servidor {
        let estado = ServerState::new(Arc::clone(&self.store), HarnessConfig::default())
            .expect("estado do servidor")
            .with_evolution_channels(vec![self.canal()]);
        let (estado, llm) = if ligar_turno {
            let (fluxo, llm) = self.fluxo(&estado.registry, roteiros, opcoes);
            (
                estado.with_turn(fluxo, Duration::from_millis(20)),
                Some(llm),
            )
        } else {
            (estado, None)
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("porta efêmera");
        let endereco = listener.local_addr().expect("endereço local");
        let (desligar, sinal) = oneshot::channel::<()>();
        tokio::spawn(async move {
            let _ = poph_server::serve_at(listener, estado, async {
                let _ = sinal.await;
            })
            .await;
        });
        Servidor {
            base: format!("http://{endereco}"),
            cliente: Client::new(),
            org: self.org,
            llm,
            desligar: Some(desligar),
        }
    }

    fn conversa(&self) -> Option<ConversationId> {
        self.store
            .conversations()
            .list(self.org, None, Page::with_limit(10))
            .ok()?
            .first()
            .map(|conversa| conversa.id)
    }

    fn entradas_pendentes(&self) -> u64 {
        self.store
            .inbox()
            .count_pending(self.org)
            .expect("conta a fila de entrada")
    }

    fn saidas(&self, estado: OutboxState) -> u64 {
        self.store
            .outbox()
            .count_by_state(self.org, estado)
            .expect("conta a fila de saída")
    }

    fn turnos(&self) -> Vec<poph_store::Turno> {
        match self.conversa() {
            Some(conversa) => self
                .store
                .turns()
                .list(self.org, conversa, Page::with_limit(50))
                .expect("lista os turnos"),
            None => Vec::new(),
        }
    }

    fn respostas_gravadas(&self) -> Vec<String> {
        let Some(conversa) = self.conversa() else {
            return Vec::new();
        };
        self.store
            .messages()
            .list(self.org, conversa, Page::with_limit(100))
            .expect("lista as mensagens")
            .into_iter()
            .filter(|mensagem| mensagem.direction == Direction::Outbound)
            .map(|mensagem| mensagem.texto)
            .collect()
    }
}

impl Drop for Cenario {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.pasta);
    }
}

struct Servidor {
    base: String,
    cliente: Client,
    org: OrgId,
    llm: Option<Arc<FakeProvider>>,
    desligar: Option<oneshot::Sender<()>>,
}

impl Servidor {
    async fn webhook(&self, corpo: Value) -> (StatusCode, Value) {
        let url = format!("{}/webhook/evolution/{}/{INSTANCIA}", self.base, self.org);
        let resposta = self
            .cliente
            .post(url)
            .header(SECRET_HEADER, SEGREDO)
            .json(&corpo)
            .send()
            .await
            .expect("entrega o webhook");
        let status = resposta.status();
        let valor = resposta.json::<Value>().await.unwrap_or(Value::Null);
        (status, valor)
    }

    async fn metricas(&self) -> String {
        self.cliente
            .get(format!("{}/metrics", self.base))
            .send()
            .await
            .expect("lê as métricas")
            .text()
            .await
            .expect("texto das métricas")
    }

    fn llm_recebidos(&self) -> usize {
        self.llm.as_ref().map_or(0, |llm| llm.received().len())
    }

    async fn desligar(mut self) {
        if let Some(desligar) = self.desligar.take() {
            let _ = desligar.send(());
        }
        tokio::time::sleep(Duration::from_millis(60)).await;
    }
}

fn linha_em_info(logs: &str, texto: &str) -> bool {
    logs.lines()
        .any(|linha| linha.contains(" INFO ") && linha.contains(texto))
}

fn valor_metrica(texto: &str, nome: &str) -> f64 {
    for linha in texto.lines() {
        if linha.starts_with('#') {
            continue;
        }
        let mut partes = linha.split_whitespace();
        if partes.next() == Some(nome)
            && let Some(valor) = partes.next().and_then(|bruto| bruto.parse::<f64>().ok())
        {
            return valor;
        }
    }
    0.0
}

async fn esperar(rotulo: &str, mut condicao: impl FnMut() -> bool) {
    for _ in 0..400 {
        if condicao() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let logs = logs_ate_agora();
    let cauda: String = logs
        .chars()
        .rev()
        .take(2500)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    panic!("não aconteceu a tempo: {rotulo}\n--- cauda dos logs ---\n{cauda}");
}

fn evento_texto(id: &str, texto: &str) -> Value {
    json!({
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
    })
}

fn evento_do_aparelho(id: &str, texto: &str) -> Value {
    let mut corpo = evento_texto(id, texto);
    corpo["data"]["key"]["fromMe"] = json!(true);
    corpo
}

fn resposta(texto: &str) -> Script {
    Script::Events(vec![
        LlmEvent::Texto(texto.to_owned()),
        LlmEvent::Usage(Usage {
            input: 120,
            saida: 30,
        }),
        LlmEvent::Fim(FinishReason::Completed),
    ])
}


#[tokio::test]
async fn mensagem_do_cliente_vira_resposta_pela_evolution_uma_vez() {
    let cenario = Cenario::novo("feliz").await;
    let servidor = cenario.subir(vec![resposta(RESPOSTA)], true).await;

    let (status, _) = servidor
        .webhook(evento_texto("MSG-FELIZ-1", PERGUNTA))
        .await;
    assert_eq!(status, StatusCode::OK);

    esperar("a resposta sair pela Evolution", || {
        cenario.saidas(OutboxState::Sent) == 1
    })
    .await;

    assert_eq!(
        servidor.llm_recebidos(),
        1,
        "um turno, uma chamada ao modelo"
    );
    let envios = cenario.evo.envios();
    assert_eq!(envios.len(), 1, "a resposta sai uma vez");
    assert_eq!(envios[0].apikey.as_deref(), Some(CHAVE_EVO));
    assert!(
        envios[0].corpo.to_string().contains(RESPOSTA),
        "o envio leva a resposta do agente"
    );
    assert!(
        !envios[0].corpo.to_string().contains(CHAVE_EVO),
        "a chave não vai no corpo do envio"
    );

    assert_eq!(cenario.entradas_pendentes(), 0);
    let turnos = cenario.turnos();
    assert_eq!(turnos.len(), 1);
    assert_eq!(turnos[0].state, TurnState::Completed);
    assert_eq!(cenario.respostas_gravadas(), vec![RESPOSTA.to_owned()]);

    let logs = logs_ate_agora();
    assert!(
        linha_em_info(&logs, "turno concluído"),
        "o turno deixou uma linha de info"
    );
    for proibido in [PERGUNTA, RESPOSTA, TELEFONE, JID, CHAVE_EVO, SEGREDO] {
        assert!(!logs.contains(proibido), "log não pode conter `{proibido}`");
    }

    servidor.desligar().await;
}

#[tokio::test]
async fn reentrega_do_mesmo_evento_nao_gera_segundo_turno() {
    let cenario = Cenario::novo("reentrega").await;
    let servidor = cenario.subir(vec![resposta(RESPOSTA)], true).await;

    let (_, primeira) = servidor
        .webhook(evento_texto("MSG-REENT-1", PERGUNTA))
        .await;
    assert_eq!(primeira["situacao"], "nova");
    esperar("a primeira resposta sair", || {
        cenario.saidas(OutboxState::Sent) == 1
    })
    .await;

    let (status, segunda) = servidor
        .webhook(evento_texto("MSG-REENT-1", PERGUNTA))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(segunda["situacao"], "duplicada");
    tokio::time::sleep(Duration::from_millis(300)).await;

    assert_eq!(servidor.llm_recebidos(), 1, "a reentrega não abre turno");
    assert_eq!(
        cenario.evo.envios().len(),
        1,
        "a reentrega não gera segundo envio"
    );
    assert_eq!(cenario.respostas_gravadas().len(), 1);

    servidor.desligar().await;
}

#[tokio::test]
async fn conversa_transferida_nao_responde_e_conclui_com_motivo() {
    let cenario = Cenario::novo("transferida").await;
    let servidor = cenario
        .subir(
            vec![resposta(RESPOSTA), resposta("isto não deveria sair")],
            true,
        )
        .await;

    servidor
        .webhook(evento_texto("MSG-TRANS-1", PERGUNTA))
        .await;
    esperar("a primeira resposta sair", || {
        cenario.saidas(OutboxState::Sent) == 1
    })
    .await;

    let conversa = cenario.conversa().expect("a conversa existe");
    let atual = cenario
        .store
        .conversations()
        .get(cenario.org, conversa)
        .expect("lê a conversa");
    cenario
        .store
        .conversations()
        .change_status(
            cenario.org,
            conversa,
            atual.version,
            ConversationStatus::Transferred,
            None,
        )
        .expect("transfere a conversa");

    servidor
        .webhook(evento_texto("MSG-TRANS-2", "estou esperando alguém"))
        .await;
    esperar("a entrada ser concluída", || {
        cenario.entradas_pendentes() == 0
    })
    .await;
    tokio::time::sleep(Duration::from_millis(200)).await;

    assert_eq!(
        servidor.llm_recebidos(),
        1,
        "transferida não chama o modelo de novo"
    );
    assert_eq!(
        cenario.evo.envios().len(),
        1,
        "nada sai depois da transferência"
    );
    assert_eq!(cenario.saidas(OutboxState::Pending), 0);
    let turnos = cenario.turnos();
    let ultimo = turnos.last().expect("há turno");
    assert_eq!(ultimo.state, TurnState::Skipped);
    assert!(
        ultimo
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("não responde"),
        "o motivo fica registrado no turno"
    );
    assert_eq!(cenario.respostas_gravadas().len(), 1);

    servidor.desligar().await;
}

#[tokio::test]
async fn mensagem_do_aparelho_nao_e_respondida_pelo_agente() {
    let cenario = Cenario::novo("aparelho").await;
    cenario
        .store
        .matrix()
        .enable(cenario.org)
        .expect("liga o módulo Matrix da org");
    let servidor = cenario
        .subir(
            vec![resposta(RESPOSTA), resposta("isto não deveria sair")],
            true,
        )
        .await;

    let (status, corpo) = servidor
        .webhook(evento_do_aparelho("MSG-AP-1", PERGUNTA))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(corpo["efeito"], "aparelho");
    assert_eq!(corpo["situacao"], "nova");

    tokio::time::sleep(Duration::from_millis(250)).await;
    assert_eq!(servidor.llm_recebidos(), 0, "o modelo não é chamado");
    assert!(cenario.evo.envios().is_empty(), "nada sai para o cliente");
    assert_eq!(
        cenario.entradas_pendentes(),
        0,
        "a mensagem do aparelho não entra na fila do turno"
    );
    assert_eq!(
        cenario.saidas(OutboxState::Pending),
        0,
        "fila de saída vazia"
    );
    assert!(cenario.turnos().is_empty(), "nenhum turno aberto");

    let conversa = cenario.conversa().expect("a conversa existe no banco");
    let gravadas = cenario
        .store
        .messages()
        .list(cenario.org, conversa, Page::with_limit(20))
        .expect("lista as mensagens");
    assert_eq!(gravadas.len(), 1);
    assert_eq!(gravadas[0].direction, Direction::Outbound);
    assert_eq!(gravadas[0].origin, Origin::Device);
    assert!(gravadas[0].turn_id.is_none());

    servidor.webhook(evento_texto("MSG-AP-2", PERGUNTA)).await;
    esperar("a entrada ser concluída", || {
        cenario.entradas_pendentes() == 0
    })
    .await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(
        servidor.llm_recebidos(),
        0,
        "conversa transferida pelo aparelho não chama o modelo"
    );
    assert_eq!(cenario.evo.envios().len(), 0, "nada sai depois da pausa");
    let turnos = cenario.turnos();
    let ultimo = turnos.last().expect("há turno");
    assert_eq!(ultimo.state, TurnState::Skipped);

    servidor.desligar().await;
}

#[tokio::test]
async fn integracao_inativa_nao_responde_e_conclui_com_motivo() {
    let cenario = Cenario::novo("inativa").await;
    let servidor = cenario.subir(vec![resposta(RESPOSTA)], true).await;

    let atual = cenario
        .store
        .get_integration(cenario.org, cenario.agente, cenario.integracao)
        .expect("lê a integração");
    cenario
        .store
        .update_integration(
            cenario.org,
            cenario.agente,
            cenario.integracao,
            poph_store::IntegrationChange {
                version: atual.version,
                status: Some("inactive"),
                ..Default::default()
            },
        )
        .expect("desativa a integração");

    let (status, _) = servidor.webhook(evento_texto("MSG-INAT-1", PERGUNTA)).await;
    assert_eq!(status, StatusCode::OK, "a entrega é aceita");
    esperar("a entrada ser concluída", || {
        cenario.entradas_pendentes() == 0
    })
    .await;
    tokio::time::sleep(Duration::from_millis(200)).await;

    assert_eq!(
        servidor.llm_recebidos(),
        0,
        "integração inativa não chama o modelo"
    );
    assert!(cenario.evo.envios().is_empty(), "nada sai pela Evolution");
    assert_eq!(cenario.saidas(OutboxState::Pending), 0);
    let turnos = cenario.turnos();
    assert_eq!(turnos.len(), 1);
    assert_eq!(turnos[0].state, TurnState::Skipped);
    assert!(
        turnos[0]
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("não respondida"),
        "o motivo fica registrado no turno"
    );
    assert_eq!(
        cenario.respostas_gravadas().len(),
        0,
        "a mensagem do cliente não gera resposta"
    );
    let logs = logs_ate_agora();
    assert!(
        linha_em_info(&logs, "turno sem resposta"),
        "o motivo de não responder sai no log, em info"
    );
    assert!(
        !logs.contains(PERGUNTA),
        "o texto do cliente não vai para o log"
    );

    servidor.desligar().await;
}

#[tokio::test]
async fn sem_turno_ligado_a_mensagem_fica_na_fila_e_e_respondida_na_subida() {
    let cenario = Cenario::novo("sem-turno").await;

    let parado = cenario.subir(Vec::new(), false).await;
    let (status, _) = parado.webhook(evento_texto("MSG-FILA-1", PERGUNTA)).await;
    assert_eq!(status, StatusCode::OK);
    esperar("a mensagem entrar na fila", || {
        cenario.entradas_pendentes() == 1
    })
    .await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(
        cenario.entradas_pendentes(),
        1,
        "a mensagem segue na fila, sem ser perdida"
    );
    assert!(cenario.evo.envios().is_empty());
    parado.desligar().await;

    let servidor = cenario.subir(vec![resposta(RESPOSTA)], true).await;
    esperar("a resposta da fila sair", || {
        cenario.saidas(OutboxState::Sent) == 1
    })
    .await;
    assert_eq!(servidor.llm_recebidos(), 1);
    assert_eq!(cenario.evo.envios().len(), 1);
    assert_eq!(cenario.entradas_pendentes(), 0);

    servidor.desligar().await;
}

#[tokio::test]
async fn resposta_que_ficou_em_envio_na_queda_sai_na_subida() {
    let cenario = Cenario::novo("queda").await;

    let conversa = cenario
        .store
        .conversations()
        .create(
            cenario.org,
            NewConversation {
                agent_id: cenario.agente,
                integration_id: Some(cenario.integracao),
                canal: "whatsapp".to_owned(),
                peer_externo: Some(TELEFONE.to_owned()),
                target_department: None,
            },
        )
        .expect("abre a conversa")
        .id;
    let turno = cenario
        .store
        .turns()
        .start(cenario.org, conversa, Some(cenario.agente))
        .expect("abre o turno");
    let (_, item) = cenario
        .store
        .outbox()
        .enqueue(
            cenario.org,
            NewOutboxItem {
                conversation_id: conversa,
                turn_id: turno.id,
                seq: 1,
                tipo: "text".to_owned(),
                content_json: json!({"texto": RESPOSTA}).to_string(),
            },
        )
        .expect("enfileira a resposta");
    cenario
        .store
        .outbox()
        .start_send(cenario.org, item.id)
        .expect("marca como em envio");
    assert_eq!(cenario.saidas(OutboxState::Sending), 1);

    let servidor = cenario.subir(Vec::new(), false).await;
    esperar("a resposta retomada sair", || {
        cenario.saidas(OutboxState::Sent) == 1
    })
    .await;
    let envios = cenario.evo.envios();
    assert_eq!(envios.len(), 1, "sai uma vez, na retomada");
    assert!(envios[0].corpo.to_string().contains(RESPOSTA));

    servidor.desligar().await;
}

#[tokio::test]
async fn turno_interrompido_por_queda_e_retomado_uma_vez_na_subida() {
    let cenario = Cenario::novo("queda-turno").await;

    let entrega = cenario
        .store
        .inbox()
        .receive_from_channel(
            cenario.org,
            poph_store::NewChannelInboxItem {
                integration_id: cenario.integracao,
                agent_id: cenario.agente,
                canal: "whatsapp".to_owned(),
                external_id: "MSG-QUEDA-TURNO".to_owned(),
                peer: TELEFONE.to_owned(),
                event_json: json!({"texto": PERGUNTA}).to_string(),
                texto: PERGUNTA.to_owned(),
                attachments: Vec::new(),
                customer_name: None,
            },
        )
        .expect("entrega do processo anterior");
    let conversa = entrega.mensagem.conversation_id;
    let turno_interrompido = cenario
        .store
        .turns()
        .start(cenario.org, conversa, Some(cenario.agente))
        .expect("turno do processo anterior");

    let servidor = cenario
        .subir(vec![resposta(RESPOSTA), resposta(RESPOSTA)], true)
        .await;
    esperar("a resposta retomada sair", || {
        cenario.saidas(OutboxState::Sent) == 1
    })
    .await;

    let gravado = cenario
        .store
        .turns()
        .get(cenario.org, turno_interrompido.id)
        .expect("turno interrompido");
    assert_eq!(gravado.state, TurnState::Failed);
    assert_eq!(
        gravado.error.as_deref(),
        Some(poph_server::turn_input::INTERRUPTED_REASON)
    );

    assert_eq!(servidor.llm_recebidos(), 1);
    assert_eq!(cenario.respostas_gravadas(), vec![RESPOSTA.to_owned()]);
    assert_eq!(cenario.evo.envios().len(), 1, "sai uma vez, na retomada");
    assert_eq!(cenario.entradas_pendentes(), 0);
    assert_eq!(
        cenario.saidas(OutboxState::Sending),
        0,
        "nada preso em envio"
    );

    let (status, _) = servidor
        .webhook(evento_texto("MSG-QUEDA-2", PERGUNTA))
        .await;
    assert_eq!(status, StatusCode::OK);
    esperar("a segunda resposta sair", || {
        cenario.saidas(OutboxState::Sent) == 2
    })
    .await;
    assert_eq!(cenario.respostas_gravadas().len(), 2);

    servidor.desligar().await;
}

#[tokio::test]
async fn metricas_do_turno_aparecem_no_metrics() {
    let cenario = Cenario::novo("metricas").await;
    let servidor = cenario.subir(vec![resposta(RESPOSTA)], true).await;

    servidor.webhook(evento_texto("MSG-MET-1", PERGUNTA)).await;
    esperar("a resposta sair", || cenario.saidas(OutboxState::Sent) == 1).await;

    let texto = servidor.metricas().await;
    assert_eq!(valor_metrica(&texto, "poph_turnos_iniciados_total"), 1.0);
    assert_eq!(valor_metrica(&texto, "poph_turnos_concluidos_total"), 1.0);
    assert_eq!(valor_metrica(&texto, "poph_turnos_falhos_total"), 0.0);

    servidor.desligar().await;
}


fn falha_do_provedor() -> Script {
    Script::Failure(poph_core::llm::LlmError::Unavailable {
        tentativas: 3,
        last_error: "503 indisponível".to_owned(),
    })
}

#[tokio::test]
async fn falha_do_modelo_repete_e_a_resposta_sai_uma_vez_pela_evolution() {
    let cenario = Cenario::novo("falha-repete").await;
    let opcoes = FlowOptions::default().with_failure_model(3, Duration::from_millis(50));
    let servidor = cenario
        .subir_com(
            vec![falha_do_provedor(), falha_do_provedor(), resposta(RESPOSTA)],
            true,
            opcoes,
        )
        .await;

    let (status, _) = servidor
        .webhook(evento_texto("MSG-FALHA-1", PERGUNTA))
        .await;
    assert_eq!(status, StatusCode::OK);

    esperar("a resposta sair depois das repetições", || {
        cenario.saidas(OutboxState::Sent) == 1
    })
    .await;

    assert_eq!(servidor.llm_recebidos(), 3, "uma chamada por tentativa");
    let envios = cenario.evo.envios();
    assert_eq!(envios.len(), 1, "a resposta sai uma vez");
    assert!(envios[0].corpo.to_string().contains(RESPOSTA));
    assert_eq!(cenario.entradas_pendentes(), 0);
    let turnos = cenario.turnos();
    assert_eq!(
        turnos
            .iter()
            .filter(|t| t.state == TurnState::Failed)
            .count(),
        2
    );
    assert_eq!(cenario.respostas_gravadas(), vec![RESPOSTA.to_owned()]);

    let metricas = servidor.metricas().await;
    assert_eq!(
        valor_metrica(&metricas, "poph_turnos_falha_modelo_adiadas_total"),
        2.0
    );
    assert_eq!(
        valor_metrica(&metricas, "poph_turnos_falha_modelo_definitivas_total"),
        0.0
    );
    let logs = logs_ate_agora();
    assert!(
        logs.contains("falha do modelo: mensagem adiada para nova tentativa"),
        "o adiamento deixou uma linha"
    );
    for proibido in [PERGUNTA, RESPOSTA, TELEFONE, JID, CHAVE_EVO, SEGREDO] {
        assert!(!logs.contains(proibido), "log não pode conter `{proibido}`");
    }

    servidor.desligar().await;
}

#[tokio::test]
async fn falha_do_modelo_esgotada_nao_manda_nada_e_registra_o_motivo() {
    let cenario = Cenario::novo("falha-esgota").await;
    let opcoes = FlowOptions::default().with_failure_model(3, Duration::from_millis(50));
    let servidor = cenario
        .subir_com(
            vec![
                falha_do_provedor(),
                falha_do_provedor(),
                falha_do_provedor(),
            ],
            true,
            opcoes,
        )
        .await;

    servidor
        .webhook(evento_texto("MSG-ESGOTA-1", PERGUNTA))
        .await;

    esperar("a entrada sair da fila depois da última tentativa", || {
        cenario.entradas_pendentes() == 0
    })
    .await;

    assert_eq!(servidor.llm_recebidos(), 3);
    assert!(cenario.evo.envios().is_empty(), "nada foi enviado");
    assert!(cenario.respostas_gravadas().is_empty());
    assert_eq!(cenario.saidas(OutboxState::Sent), 0);

    let metricas = servidor.metricas().await;
    assert_eq!(
        valor_metrica(&metricas, "poph_turnos_falha_modelo_definitivas_total"),
        1.0
    );
    let logs = logs_ate_agora();
    assert!(
        logs.contains("tentativas esgotadas; mensagem concluída sem resposta"),
        "a falha definitiva deixou uma linha"
    );
    for proibido in [PERGUNTA, TELEFONE, JID, CHAVE_EVO, SEGREDO] {
        assert!(!logs.contains(proibido), "log não pode conter `{proibido}`");
    }

    servidor.desligar().await;
}

#[tokio::test]
async fn recusa_do_provedor_401_nao_repete_avisa_a_operacao_e_nao_manda_nada() {
    let cenario = Cenario::novo("recusa-401").await;
    let opcoes = FlowOptions::default().with_failure_model(3, Duration::from_millis(50));
    let servidor = cenario
        .subir_com(
            vec![Script::Failure(poph_core::llm::LlmError::Rejected {
                status: 401,
                mensagem: "chave recusada".to_owned(),
            })],
            true,
            opcoes,
        )
        .await;

    servidor.webhook(evento_texto("MSG-401-1", PERGUNTA)).await;
    esperar("a entrada sair da fila na primeira tentativa", || {
        cenario.entradas_pendentes() == 0
    })
    .await;

    assert_eq!(servidor.llm_recebidos(), 1, "a recusa não é repetida");
    assert!(cenario.evo.envios().is_empty(), "nada foi enviado");
    assert!(cenario.respostas_gravadas().is_empty());

    let metricas = servidor.metricas().await;
    assert_eq!(
        valor_metrica(&metricas, "poph_turnos_falha_modelo_recusadas_total"),
        1.0
    );
    assert_eq!(
        valor_metrica(&metricas, "poph_turnos_falha_modelo_adiadas_total"),
        0.0
    );
    let logs = logs_ate_agora();
    assert!(
        logs.contains("provedor recusou o pedido"),
        "o aviso da recusa apareceu no log"
    );
    for proibido in [PERGUNTA, RESPOSTA, TELEFONE, JID, CHAVE_EVO, SEGREDO] {
        assert!(!logs.contains(proibido), "log não pode conter `{proibido}`");
    }

    servidor.desligar().await;
}
