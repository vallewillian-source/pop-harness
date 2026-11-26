//! Turno no `poph serve` (HR-26h): mensagem do WhatsApp vira resposta, ponta a ponta.
//!
//! Cada teste sobe um servidor de verdade (porta efêmera, `servir_em`) com:
//! - o webhook da Evolution (entrada, HR-26b);
//! - o turno com um **provedor de LLM falso** (`ProvedorFake`), contando as chamadas;
//! - a saída pela fila (`saida_whatsapp`) apontando para uma **Evolution falsa** em `127.0.0.1`,
//!   que registra cada envio (com a chave do cabeçalho) e responde o roteiro do teste.
//!
//! Nada de rede externa, nenhum número de WhatsApp. O banco é temporário.
//!
//! O que se prova: mensagem entra e sai uma vez (ordem e contagem de chamadas nos dois servidores
//! falsos); reentrega do mesmo evento não abre segundo turno; conversa transferida e integração
//! inativa não respondem e concluem a entrada com o motivo; sem o turno ligado a mensagem fica na
//! fila e é respondida quando o servidor sobe com ele; resposta que ficou em `sending` na queda é
//! enviada na subida; métricas do turno aparecem no `/metrics`; logs do turno não trazem telefone,
//! JID, chave, segredo nem texto do cliente ou da resposta.

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
use poph_agent::atores::{Atores, OpcoesAtores};
use poph_agent::config_quente::{ConfigQuente, OpcoesConfigQuente};
use poph_agent::entrada_saida::{Fluxo, OpcoesFluxo};
use poph_agent::limites::{Limitador, OpcoesLimites};
use poph_agent::metricas::{MetricasFluxo, MetricasMemoria, MetricasTurno};
use poph_agent::turno::{Motor, OpcoesTurno, ProcessadorTurno};
use poph_core::llm::{EventoLlm, LlmProvider, MotivoFim, ProvedorFake, Roteiro, Uso};
use poph_evolution::webhook::CABECALHO_SEGREDO;
use poph_evolution::{ClienteEvolution, ConfigEvolution, TETO_MIDIA};
use poph_server::Estado;
use poph_server::evolution::CanalEvolution;
use poph_store::{
    ConversationStatus, Direction, NewAgent, NewConversation, NewOrg, NewOutboxItem, Origin,
    OutboxState, Page, Store, TurnState,
};
use poph_types::{AgentId, ConversationId, HarnessConfig, IntegrationId, OrgId, Registry, Secret};
use reqwest::Client;
use serde_json::{Value, json};
use tokio::sync::oneshot;
use tracing_subscriber::fmt::MakeWriter;

/// Segredo do webhook da instância `loja` (só para o cabeçalho das entregas falsas).
const SEGREDO: &str = "segredo-webhook-turno-26h";
/// Chave da Evolution (só para o cabeçalho das requisições de envio).
const CHAVE_EVO: &str = "chave-evo-turno-26h-teste";
/// Número de cliente falso. Não é de ninguém.
const TELEFONE: &str = "5511999990001";
const JID: &str = "5511999990001@s.whatsapp.net";
/// Texto do cliente e resposta do agente: não podem aparecer em log.
const PERGUNTA: &str = "qual o prazo de entrega do pedido quatro quatro sete um";
const RESPOSTA: &str = "o prazo é de cinco dias úteis após a confirmação";
const INSTANCIA: &str = "loja";

// ───────────────────────── captura de logs (para provar o que NÃO sai) ─────────────────────────

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

/// Instala, uma vez no processo de testes, o coletor de logs. Todo teste deste arquivo usa o mesmo.
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

// ───────────────────────── Evolution falsa: registra cada envio ─────────────────────────

#[derive(Debug, Clone)]
struct Envio {
    apikey: Option<String>,
    corpo: Value,
}

#[derive(Clone, Default)]
struct EvolucaoFalsa {
    envios: Arc<Mutex<Vec<Envio>>>,
    /// Status a devolver em cada envio, na ordem. Vazio = 200.
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

// ───────────────────────── cenário: banco, org, agente e integração ─────────────────────────

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
            .criar(NewOrg {
                name: "Loja Turno".to_owned(),
                slug: format!("loja-turno-{rotulo}"),
                config_json: String::new(),
            })
            .expect("cria a org")
            .id;
        let agente = store
            .agents()
            .criar(
                org,
                NewAgent {
                    name: "Atendente Loja".to_owned(),
                    slug: "atendente-loja".to_owned(),
                    modelo: "gemini-3.8-flash-tiered".to_owned(),
                    // Sem janela de agrupamento: o teste não espera o cliente terminar de digitar.
                    grouping_window_ms: 0,
                    ..Default::default()
                },
            )
            .expect("cria o agente")
            .id;
        store
            .instructions()
            .salvar(
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

    /// Instância `loja`, com segredo e cliente de envio apontando para a Evolution falsa.
    fn canal(&self) -> CanalEvolution {
        let cliente = ClienteEvolution::novo(
            ConfigEvolution::novo(
                self.url_evo.clone(),
                INSTANCIA,
                Secret::new(CHAVE_EVO.to_owned()),
            )
            .expect("config da Evolution falsa"),
        )
        .expect("cliente da Evolution falsa");
        CanalEvolution {
            org: self.org,
            instancia: INSTANCIA.to_owned(),
            integracao: self.integracao,
            segredo: Some(Secret::new(SEGREDO.to_owned())),
            cliente: Some(cliente),
            teto_midia: TETO_MIDIA,
        }
    }

    /// Fluxo de entrada e saída com o turno de verdade e o provedor falso. As métricas vão para o
    /// registro do servidor, como na subida real.
    fn fluxo(
        &self,
        registro: &Registry,
        roteiros: Vec<Roteiro>,
        opcoes: OpcoesFluxo,
    ) -> (Fluxo, Arc<ProvedorFake>) {
        let fake = Arc::new(ProvedorFake::com_roteiros(roteiros));
        let provedor: Arc<dyn LlmProvider> = fake.clone();
        let config = ConfigQuente::nova(
            Arc::clone(&self.store),
            OpcoesConfigQuente::default(),
            MetricasMemoria::proprio(),
        );
        let limitador = Arc::new(Limitador::novo(
            OpcoesLimites::default(),
            MetricasMemoria::proprio(),
        ));
        let motor = Motor::novo(
            Arc::clone(&self.store),
            config,
            limitador,
            provedor,
            OpcoesTurno::default(),
            MetricasTurno::registrar(registro).expect("métricas do turno"),
        );
        let processador = Arc::new(ProcessadorTurno::novo(Arc::new(motor)));
        let atores = Atores::novo(
            processador.clone(),
            OpcoesAtores::default(),
            MetricasMemoria::proprio(),
        );
        let fluxo = Fluxo::com_metricas(
            Arc::clone(&self.store),
            atores,
            opcoes,
            MetricasFluxo::registrar(registro).expect("métricas da fila"),
        );
        processador.ligar_fluxo(fluxo.clone());
        (fluxo, fake)
    }

    /// Sobe o servidor. Com `ligar_turno = false`, ele recebe e grava, mas não roda turno.
    async fn subir(&self, roteiros: Vec<Roteiro>, ligar_turno: bool) -> Servidor {
        self.subir_com(roteiros, ligar_turno, OpcoesFluxo::default())
            .await
    }

    /// Como [`Cenario::subir`], com a política do fluxo escolhida pelo teste.
    async fn subir_com(
        &self,
        roteiros: Vec<Roteiro>,
        ligar_turno: bool,
        opcoes: OpcoesFluxo,
    ) -> Servidor {
        let estado = Estado::novo(Arc::clone(&self.store), HarnessConfig::default())
            .expect("estado do servidor")
            .com_canais_evolution(vec![self.canal()]);
        let (estado, llm) = if ligar_turno {
            let (fluxo, llm) = self.fluxo(&estado.registro, roteiros, opcoes);
            (
                estado.com_turno(fluxo, Duration::from_millis(20)),
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
            let _ = poph_server::servir_em(listener, estado, async {
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

    /// Conversa do cliente (a primeira da org; o teste só abre uma).
    fn conversa(&self) -> Option<ConversationId> {
        self.store
            .conversas()
            .listar(self.org, None, Page::with_limit(10))
            .ok()?
            .first()
            .map(|conversa| conversa.id)
    }

    fn entradas_pendentes(&self) -> u64 {
        self.store
            .inbox()
            .contar_pendentes(self.org)
            .expect("conta a fila de entrada")
    }

    fn saidas(&self, estado: OutboxState) -> u64 {
        self.store
            .outbox()
            .contar_por_estado(self.org, estado)
            .expect("conta a fila de saída")
    }

    fn turnos(&self) -> Vec<poph_store::Turno> {
        match self.conversa() {
            Some(conversa) => self
                .store
                .turns()
                .listar(self.org, conversa, Page::with_limit(50))
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
            .listar(self.org, conversa, Page::with_limit(100))
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

/// Servidor no ar: entrega webhooks e lê as métricas. Ao cair, o servidor para.
struct Servidor {
    base: String,
    cliente: Client,
    org: OrgId,
    /// Provedor falso do turno (`None` quando o turno não está ligado neste servidor).
    llm: Option<Arc<ProvedorFake>>,
    desligar: Option<oneshot::Sender<()>>,
}

impl Servidor {
    /// Entrega um webhook de texto. Devolve o status e o corpo da resposta.
    async fn webhook(&self, corpo: Value) -> (StatusCode, Value) {
        let url = format!("{}/webhook/evolution/{}/{INSTANCIA}", self.base, self.org);
        let resposta = self
            .cliente
            .post(url)
            .header(CABECALHO_SEGREDO, SEGREDO)
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
        self.llm.as_ref().map_or(0, |llm| llm.recebidos().len())
    }

    async fn desligar(mut self) {
        if let Some(desligar) = self.desligar.take() {
            let _ = desligar.send(());
        }
        tokio::time::sleep(Duration::from_millis(60)).await;
    }
}

/// Há uma linha de nível `INFO` com o texto dado? (O nível também é conferido.)
fn linha_em_info(logs: &str, texto: &str) -> bool {
    logs.lines()
        .any(|linha| linha.contains(" INFO ") && linha.contains(texto))
}

/// Valor de uma métrica no texto do `/metrics` (0 quando não aparece).
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

/// Espera uma condição (polling curto). Falha com o nome do que não aconteceu.
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

/// A mesma entrega de texto, com `fromMe = true`: o que o cliente digitou no celular (MX-41).
fn evento_do_aparelho(id: &str, texto: &str) -> Value {
    let mut corpo = evento_texto(id, texto);
    corpo["data"]["key"]["fromMe"] = json!(true);
    corpo
}

fn resposta(texto: &str) -> Roteiro {
    Roteiro::Eventos(vec![
        EventoLlm::Texto(texto.to_owned()),
        EventoLlm::Uso(Uso {
            entrada: 120,
            saida: 30,
        }),
        EventoLlm::Fim(MotivoFim::Concluido),
    ])
}

// ───────────────────────────────────────── testes ─────────────────────────────────────────

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

    // Uma chamada ao modelo, um envio, a chave só no cabeçalho.
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

    // A entrada saiu da fila, o turno concluiu e a resposta ficou na conversa.
    assert_eq!(cenario.entradas_pendentes(), 0);
    let turnos = cenario.turnos();
    assert_eq!(turnos.len(), 1);
    assert_eq!(turnos[0].estado, TurnState::Completed);
    assert_eq!(cenario.respostas_gravadas(), vec![RESPOSTA.to_owned()]);

    // Logs: o turno deixou linha, e nada sensível saiu nelas.
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

    // A Evolution reentrega o mesmo evento (o id da mensagem é o mesmo).
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

    // Um humano assume: a conversa vai para transferida.
    let conversa = cenario.conversa().expect("a conversa existe");
    let atual = cenario
        .store
        .conversas()
        .get(cenario.org, conversa)
        .expect("lê a conversa");
    cenario
        .store
        .conversas()
        .mudar_status(
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
    assert_eq!(ultimo.estado, TurnState::Skipped);
    assert!(
        ultimo
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("não responde"),
        "o motivo fica registrado no turno"
    );
    // A mensagem do cliente continua gravada.
    assert_eq!(cenario.respostas_gravadas().len(), 1);

    servidor.desligar().await;
}

/// Aceite da MX-41: a mensagem digitada no aparelho **não** é respondida pelo agente — nem na
/// hora (não há turno), nem depois (a conversa ficou `transferred` e a próxima mensagem do cliente
/// também não chama o modelo).
#[tokio::test]
async fn mensagem_do_aparelho_nao_e_respondida_pelo_agente() {
    let cenario = Cenario::novo("aparelho").await;
    cenario
        .store
        .matrix()
        .ligar(cenario.org)
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

    // A mensagem existe, é saída do dispositivo e não tem turno.
    let conversa = cenario.conversa().expect("a conversa existe no banco");
    let gravadas = cenario
        .store
        .messages()
        .listar(cenario.org, conversa, Page::with_limit(20))
        .expect("lista as mensagens");
    assert_eq!(gravadas.len(), 1);
    assert_eq!(gravadas[0].direction, Direction::Outbound);
    assert_eq!(gravadas[0].origin, Origin::Device);
    assert!(gravadas[0].turn_id.is_none());

    // E o cliente que escrever depois também não é respondido: quem atende é o humano no aparelho.
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
    assert_eq!(ultimo.estado, TurnState::Skipped);

    servidor.desligar().await;
}

#[tokio::test]
async fn integracao_inativa_nao_responde_e_conclui_com_motivo() {
    let cenario = Cenario::novo("inativa").await;
    let servidor = cenario.subir(vec![resposta(RESPOSTA)], true).await;

    // A integração é desativada no painel: as mensagens entram, mas o agente não responde.
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
    assert_eq!(turnos[0].estado, TurnState::Skipped);
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

    // Servidor sem o modelo (chave ausente): recebe e grava, não responde.
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

    // O servidor sobe com o turno: a fila é lida e a mensagem é respondida.
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

    // Simula a queda no meio do envio: a resposta já está na fila, marcada como em envio.
    let conversa = cenario
        .store
        .conversas()
        .criar(
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
        .iniciar(cenario.org, conversa, Some(cenario.agente))
        .expect("abre o turno");
    let (_, item) = cenario
        .store
        .outbox()
        .enfileirar(
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
        .iniciar_envio(cenario.org, item.id)
        .expect("marca como em envio");
    assert_eq!(cenario.saidas(OutboxState::Sending), 1);

    // Ao subir, a saída retoma o que ficou em envio e entrega.
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

    // Estado deixado pelo processo que morreu no meio do atendimento: a entrega do cliente está
    // na fila (não concluída) e o turno ficou aberto.
    let entrega = cenario
        .store
        .inbox()
        .receber_do_canal(
            cenario.org,
            poph_store::NewChannelInboxItem {
                integration_id: cenario.integracao,
                agent_id: cenario.agente,
                canal: "whatsapp".to_owned(),
                id_externo: "MSG-QUEDA-TURNO".to_owned(),
                peer: TELEFONE.to_owned(),
                event_json: json!({"texto": PERGUNTA}).to_string(),
                texto: PERGUNTA.to_owned(),
                anexos: Vec::new(),
                customer_name: None,
            },
        )
        .expect("entrega do processo anterior");
    let conversa = entrega.mensagem.conversation_id;
    let turno_interrompido = cenario
        .store
        .turns()
        .iniciar(cenario.org, conversa, Some(cenario.agente))
        .expect("turno do processo anterior");

    // A subida: o turno interrompido fecha como falha e a entrega pendente é processada de novo.
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
        .obter(cenario.org, turno_interrompido.id)
        .expect("turno interrompido");
    assert_eq!(gravado.estado, TurnState::Failed);
    assert_eq!(
        gravado.error.as_deref(),
        Some(poph_server::entrada_turno::MOTIVO_INTERROMPIDO)
    );

    // Uma resposta só: uma chamada ao modelo, uma mensagem do agente e um envio ao cliente.
    assert_eq!(servidor.llm_recebidos(), 1);
    assert_eq!(cenario.respostas_gravadas(), vec![RESPOSTA.to_owned()]);
    assert_eq!(cenario.evo.envios().len(), 1, "sai uma vez, na retomada");
    assert_eq!(cenario.entradas_pendentes(), 0);
    assert_eq!(
        cenario.saidas(OutboxState::Sending),
        0,
        "nada preso em envio"
    );

    // E a conversa segue atendendo: mensagem nova responde normalmente.
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

// ───────────────────────── falha do modelo (HR-26i) ─────────────────────────

fn falha_do_provedor() -> Roteiro {
    Roteiro::Falha(poph_core::llm::ErroLlm::Indisponivel {
        tentativas: 3,
        ultimo: "503 indisponível".to_owned(),
    })
}

#[tokio::test]
async fn falha_do_modelo_repete_e_a_resposta_sai_uma_vez_pela_evolution() {
    let cenario = Cenario::novo("falha-repete").await;
    // Espera curta para o teste: 50 ms antes da segunda tentativa e 200 ms antes da terceira.
    let opcoes = OpcoesFluxo::default().com_falha_modelo(3, Duration::from_millis(50));
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

    // Três chamadas ao modelo (uma por tentativa), e um só envio pela Evolution.
    assert_eq!(servidor.llm_recebidos(), 3, "uma chamada por tentativa");
    let envios = cenario.evo.envios();
    assert_eq!(envios.len(), 1, "a resposta sai uma vez");
    assert!(envios[0].corpo.to_string().contains(RESPOSTA));
    assert_eq!(cenario.entradas_pendentes(), 0);
    let turnos = cenario.turnos();
    assert_eq!(
        turnos
            .iter()
            .filter(|t| t.estado == TurnState::Failed)
            .count(),
        2
    );
    assert_eq!(cenario.respostas_gravadas(), vec![RESPOSTA.to_owned()]);

    // Métricas e log: a falha deixou rastro, sem dado sensível.
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
    let opcoes = OpcoesFluxo::default().com_falha_modelo(3, Duration::from_millis(50));
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

    // Exatamente três chamadas, nenhum envio, e nenhuma resposta gravada.
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
    let opcoes = OpcoesFluxo::default().com_falha_modelo(3, Duration::from_millis(50));
    let servidor = cenario
        .subir_com(
            vec![Roteiro::Falha(poph_core::llm::ErroLlm::Recusado {
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

    // Uma só chamada ao modelo, nenhum envio e nenhuma resposta gravada.
    assert_eq!(servidor.llm_recebidos(), 1, "a recusa não é repetida");
    assert!(cenario.evo.envios().is_empty(), "nada foi enviado");
    assert!(cenario.respostas_gravadas().is_empty());

    // A operação vê na hora: métrica de recusa e aviso no log, sem dado sensível.
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
