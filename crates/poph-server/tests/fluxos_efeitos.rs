//! Fila de efeitos do fluxo (FX-21): o efeito sai da fila e chega ao destino.
//!
//! Banco temporário, entregador de verdade ([`EfeitosFluxo`]) e uma Evolution falsa em
//! `127.0.0.1` que registra cada requisição e responde o roteiro do teste. Nada de rede externa,
//! nada de número real.
//!
//! O que fica provado aqui:
//!
//! - `mensagem_ao_cliente` entra na `outbox` da conversa e chega ao cliente **uma vez** (ponta a
//!   ponta com a saída do WhatsApp, `SaidaWhatsApp`);
//! - `notificar_whatsapp` vai **para outro número**, pela integração da conversa, e não para o
//!   cliente;
//! - sem chave/integração da Evolution o efeito fica `pendente`, **não** falha e não trava o laço;
//! - a mensagem ao cliente espera o turno da conversa terminar (PRD §6.8);
//! - queda entre enfileirar e confirmar reenvia, e a duplicata é aceita e documentada (D11);
//! - falha definitiva (4xx) vira `falhou` com evento e métrica, sem desfazer a transição;
//! - 5xx volta para a fila e passa na tentativa seguinte, respeitando a espera;
//! - `encaminhar` marca a conversa como transferida, mantém a Ficha aberta e grava a auditoria
//!   `system`;
//! - departamento que não existe vira `falhou` sem reverter nada;
//! - o teto de 4.000 caracteres do WhatsApp é respeitado;
//! - o telefone (cliente e equipe) não aparece em log nem no motivo gravado.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use axum::Json;
use axum::Router;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::post;
use poph_agent::Backoff;
use poph_evolution::{ATTACHMENT_CAP, ConfigEvolution, EvolutionClient};
use poph_fluxos::EfeitoProduzido;
use poph_fluxos::ir::Efeito;
use poph_server::evolution::CanalEvolution;
use poph_server::fluxos_efeitos::{
    EfeitosFluxo, MOTIVO_TENTATIVAS_ESGOTADAS, OpcoesEfeitosFluxo, RelatoEfeitos,
    efeito_da_transicao,
};
use poph_server::fluxos_metricas::MetricasFluxos;
use poph_server::saida_whatsapp::{OpcoesSaidaWhatsApp, SaidaWhatsApp};
use poph_store::ConversationStatus;
use poph_store::{
    Agente, ExecutionStart, ExecutionState, FlowEffect, NewAgent, NewConversation, NewDepartment,
    NewOrg, Store, StoredExecution, TransitionEffect,
};
use poph_types::{
    DepartmentId, EffectState, EventActor, ExecutionStatus, FlowId, HarnessConfig, IntegrationId,
    OrgId, RecordEventKind, RecordId, RecordStatus, Registry as RegistroDeMetricas, RevisionId,
    Secret,
};
use serde_json::{Value, json};
use tracing_subscriber::fmt::MakeWriter;

/// Número do cliente fictício (não é de ninguém).
const TELEFONE: &str = "5521999990001";
/// Número da equipe fictícia, o destino do `notificar_whatsapp`.
const TELEFONE_EQUIPE: &str = "5521988880002";
/// Chave da Evolution dos testes (só para o cabeçalho `apikey`).
const CHAVE: &str = "chave-evo-fx21-teste";
/// Teto de mídia do canal.
const TETO: u64 = ATTACHMENT_CAP;

// ───────────────────────── captura de logs (para provar o que NÃO sai) ─────────────────────────

#[derive(Clone)]
struct Captura(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for Captura {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
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

// ───────────────────────── Evolution falsa: registra e responde o roteiro ─────────────────────────

#[derive(Clone, Debug)]
struct Registro {
    caminho: String,
    apikey: Option<String>,
    corpo: Value,
}

#[derive(Clone)]
struct EvolucaoFalsa {
    registros: Arc<Mutex<Vec<Registro>>>,
    roteiro: Arc<Mutex<Vec<(u16, Value)>>>,
    chamadas: Arc<AtomicUsize>,
}

async fn enviar_falso(
    State(evo): State<EvolucaoFalsa>,
    Path(instancia): Path<String>,
    cabecalhos: HeaderMap,
    Json(corpo): Json<Value>,
) -> (StatusCode, Json<Value>) {
    let indice = evo.chamadas.fetch_add(1, Ordering::SeqCst);
    let apikey = cabecalhos
        .get("apikey")
        .and_then(|valor| valor.to_str().ok())
        .map(str::to_owned);
    evo.registros.lock().unwrap().push(Registro {
        caminho: format!("instancia={instancia}"),
        apikey,
        corpo,
    });
    let roteiro = evo.roteiro.lock().unwrap();
    let (status, resposta) = roteiro
        .get(indice)
        .or_else(|| roteiro.last())
        .cloned()
        .unwrap_or((200, json!({"key": {"id": "MSG-FX21"}})));
    (
        StatusCode::from_u16(status).expect("status do roteiro"),
        Json(resposta),
    )
}

async fn subir_evolucao_falsa(
    roteiro: Vec<(u16, Value)>,
) -> (String, EvolucaoFalsa, tokio::task::JoinHandle<()>) {
    let evo = EvolucaoFalsa {
        registros: Arc::new(Mutex::new(Vec::new())),
        roteiro: Arc::new(Mutex::new(roteiro)),
        chamadas: Arc::new(AtomicUsize::new(0)),
    };
    let roteador = Router::new()
        .route("/message/sendText/{instancia}", post(enviar_falso))
        .route("/message/sendMedia/{instancia}", post(enviar_falso))
        .with_state(evo.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("porta efêmera da Evolution falsa");
    let endereco = listener.local_addr().expect("endereço da Evolution falsa");
    let tarefa = tokio::spawn(async move {
        let _ = axum::serve(listener, roteador).await;
    });
    (format!("http://{endereco}"), evo, tarefa)
}

// ───────────────────────── cenário: banco, org, integração, Ficha e execução ─────────────────────────

/// Como a instância da Evolution aparece para o entregador.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Instancia {
    /// Chave presente e Evolution falsa no ar.
    Normal,
    /// Chave ausente no ambiente: o canal existe na configuração, mas não consegue enviar.
    SemChave,
    /// Nenhuma instância configurada para a org.
    SemCanal,
}

struct Cenario {
    store: Arc<Store>,
    relogio: Arc<AtomicI64>,
    org: OrgId,
    agente: Agente,
    integracao: Option<IntegrationId>,
    /// Conversa de WhatsApp do atendimento principal (nasce com Ficha).
    conversa: poph_types::ConversationId,
    ficha: RecordId,
    fluxo: FlowId,
    revisao: RevisionId,
    evo: EvolucaoFalsa,
    canais: Vec<CanalEvolution>,
    metricas: Arc<MetricasFluxos>,
    blobs: PathBuf,
    pasta: PathBuf,
    tarefa_evo: tokio::task::JoinHandle<()>,
}

impl Cenario {
    async fn novo(rotulo: &str, modo: Instancia, roteiro: Vec<(u16, Value)>) -> Self {
        logs_capturados();
        let carimbo = std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let pasta = std::env::temp_dir().join(format!(
            "poph-fx21-{rotulo}-{}-{carimbo}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&pasta);
        fs::create_dir_all(pasta.join("dados")).expect("pasta de dados");

        let mut config = HarnessConfig::default();
        config.data.directory = pasta.join("dados");
        config.server.port = 7481;
        config.validar().expect("configuração válida");
        let blobs = config.blobs();

        // Relógio injetado: a espera entre tentativas não depende do relógio de parede.
        let relogio = Arc::new(AtomicI64::new(1_700_000_000_000));
        let ponteiro = Arc::clone(&relogio);
        let store = Arc::new(
            Store::open(&config)
                .expect("abre o banco")
                .with_clock(Arc::new(move || ponteiro.load(Ordering::SeqCst))),
        );

        let org = store
            .orgs()
            .create(NewOrg {
                name: "Loja FX-21".to_owned(),
                slug: "loja-fx21".to_owned(),
                config_json: String::new(),
            })
            .expect("cria a org")
            .id;
        let agente = store
            .agents()
            .create(
                org,
                NewAgent {
                    name: "Atendente FX-21".to_owned(),
                    slug: "atendente-fx21".to_owned(),
                    ..Default::default()
                },
            )
            .expect("cria o agente");

        let integracao: Option<IntegrationId> = if modo == Instancia::SemCanal {
            None
        } else {
            Some(
                store
                    .create_integration(org, agente.id, "whatsapp".to_owned(), "active".to_owned())
                    .expect("cria a integração")
                    .id
                    .parse()
                    .expect("id da integração"),
            )
        };

        let conversa = store
            .conversations()
            .create(
                org,
                NewConversation {
                    agent_id: agente.id,
                    integration_id: integracao,
                    canal: "whatsapp".to_owned(),
                    peer_externo: Some(TELEFONE.to_owned()),
                    target_department: None,
                },
            )
            .expect("cria a conversa")
            .id;
        let ficha = store
            .cases()
            .by_conversation(org, conversa)
            .expect("consulta a Ficha")
            .expect("a conversa nasce com Ficha")
            .id;

        // Fluxo e revisão gravados direto: a publicação é a FX-09.
        let (fluxo, revisao) = fluxo_publicado(&store, org);

        let (url, evo, tarefa_evo) = subir_evolucao_falsa(roteiro).await;
        let cliente = if modo == Instancia::SemChave {
            None
        } else {
            Some(
                EvolutionClient::new(
                    ConfigEvolution::new(url, "loja", Secret::new(CHAVE.to_owned()))
                        .expect("config da Evolution falsa"),
                )
                .expect("cliente da Evolution falsa"),
            )
        };
        let canais = match (modo, integracao) {
            (Instancia::SemCanal, _) | (_, None) => Vec::new(),
            (_, Some(integracao)) => vec![CanalEvolution {
                org,
                instancia: "loja".to_owned(),
                integracao,
                segredo: None,
                cliente,
                teto_midia: TETO,
            }],
        };

        let registro = RegistroDeMetricas::new();
        let metricas =
            Arc::new(MetricasFluxos::registrar(&registro).expect("registra as métricas de fluxos"));

        Self {
            store,
            relogio,
            org,
            agente,
            integracao,
            conversa,
            ficha,
            fluxo,
            revisao,
            evo,
            canais,
            metricas,
            blobs,
            pasta,
            tarefa_evo,
        }
    }

    fn efeitos(&self) -> EfeitosFluxo {
        EfeitosFluxo::novo(
            Arc::clone(&self.store),
            &self.canais,
            self.blobs.clone(),
            Some(Arc::clone(&self.metricas)),
            OpcoesEfeitosFluxo {
                // Espera curta para o teste não depender de relógio de parede.
                backoff: Backoff {
                    base: Duration::from_millis(1),
                    factor: 2,
                    cap: Duration::from_millis(5),
                },
                espera_sem_integracao: Duration::from_millis(10),
                ..OpcoesEfeitosFluxo::default()
            },
        )
    }

    fn saida(&self) -> SaidaWhatsApp {
        SaidaWhatsApp::novo(
            Arc::clone(&self.store),
            &self.canais,
            self.blobs.clone(),
            OpcoesSaidaWhatsApp {
                backoff: Backoff {
                    base: Duration::from_millis(1),
                    factor: 2,
                    cap: Duration::from_millis(5),
                },
                ..OpcoesSaidaWhatsApp::default()
            },
        )
    }

    /// Avança o relógio injetado (a espera entre tentativas fica no passado).
    fn avancar(&self, ms: i64) {
        self.relogio.fetch_add(ms, Ordering::SeqCst);
    }

    /// Abre a execução com os efeitos de entrada dados, na Ficha principal.
    fn abrir(&self, efeitos: Vec<TransitionEffect>) -> StoredExecution {
        self.abrir_na_ficha(self.ficha, efeitos)
    }

    fn abrir_na_ficha(&self, ficha: RecordId, efeitos: Vec<TransitionEffect>) -> StoredExecution {
        self.store
            .flows()
            .release(self.org, self.agente.id, self.fluxo)
            .expect("libera o fluxo ao agente");
        self.store
            .executions()
            .start_execution(
                self.org,
                ExecutionStart {
                    chave_idempotencia: None,
                    case: ficha,
                    flow: self.fluxo,
                    revision: self.revisao,
                    actor: EventActor::Agent,
                    ator_ref: Some("agente-de-teste"),
                    jev_decision: None,
                    initial_state: ExecutionState {
                        status: ExecutionStatus::Active,
                        no_atual: "boas_vindas".to_owned(),
                        dados_json: "{}".to_owned(),
                        acting_actors: Vec::new(),
                        result: None,
                        reason: None,
                    },
                    inbound_effects: efeitos,
                },
            )
            .expect("abre a execução")
    }

    /// Outra Ficha (e outra conversa) na mesma org, para o teste de laço com dois efeitos.
    fn outra_ficha(&self, integracao: Option<IntegrationId>) -> RecordId {
        let canal = if integracao.is_some() {
            "whatsapp"
        } else {
            "console"
        };
        let conversa = self
            .store
            .conversations()
            .create(
                self.org,
                NewConversation {
                    agent_id: self.agente.id,
                    integration_id: integracao,
                    canal: canal.to_owned(),
                    peer_externo: Some("5521977770003".to_owned()),
                    target_department: None,
                },
            )
            .expect("cria a segunda conversa")
            .id;
        self.store
            .cases()
            .by_conversation(self.org, conversa)
            .expect("consulta")
            .expect("Ficha da segunda conversa")
            .id
    }

    fn efeitos_da_ficha(&self, execucao: poph_types::ExecutionId) -> Vec<FlowEffect> {
        self.store
            .executions()
            .effects(self.org, execucao)
            .expect("lê os efeitos")
    }

    fn eventos(&self, ficha: RecordId) -> Vec<poph_store::RecordEvent> {
        self.store
            .cases()
            .events(self.org, ficha, None, 100)
            .expect("lê os eventos")
    }

    fn conversa(&self) -> poph_store::Conversa {
        self.store
            .conversations()
            .get(self.org, self.conversa)
            .expect("lê a conversa")
    }

    fn itens_da_saida(&self) -> Vec<poph_store::OutboxItem> {
        self.store
            .outbox()
            .list_pending(self.org, 100)
            .expect("lê a outbox")
    }

    /// Quantos itens existem na `outbox` da org, em qualquer estado.
    fn total_de_itens(&self) -> i64 {
        let conn = rusqlite::Connection::open(self.store.harness_path()).expect("abre para contar");
        conn.query_row(
            "SELECT COUNT(*) FROM outbox WHERE org_id = ?1",
            rusqlite::params![self.org.to_string()],
            |linha| linha.get::<_, i64>(0),
        )
        .expect("conta a outbox")
    }

    fn registros(&self) -> Vec<Registro> {
        self.evo.registros.lock().unwrap().clone()
    }
}

impl Drop for Cenario {
    fn drop(&mut self) {
        self.tarefa_evo.abort();
        let _ = fs::remove_dir_all(&self.pasta);
    }
}

/// Fluxo e revisão publicados, gravados por SQL (a publicação é a FX-09).
fn fluxo_publicado(store: &Store, org: OrgId) -> (FlowId, RevisionId) {
    let fluxo: FlowId = uuid::Uuid::now_v7().to_string().parse().expect("fluxo");
    let revisao: RevisionId = uuid::Uuid::now_v7().to_string().parse().expect("revisão");
    let conn = rusqlite::Connection::open(store.harness_path()).expect("abre para gravar");
    conn.execute(
        "INSERT INTO fluxos (id, org_id, slug, nome, descricao, status, rascunho_fonte,
                             revisao_atual, version)
         VALUES (?1, ?2, 'boas_vindas', 'Boas-vindas', '', 'active', '', 1, 1)",
        rusqlite::params![fluxo.to_string(), org.to_string()],
    )
    .expect("fluxo");
    conn.execute(
        "INSERT INTO fluxo_revisoes (id, org_id, fluxo_id, numero, fonte, fonte_sha256,
                                    definicao_json, schema_versao, publicada_em, publicada_por,
                                    resolucoes_json)
         VALUES (?1, ?2, ?3, 1, 'fluxo()', 'sha', '{}', 1, 1, 'admin_token', '{}')",
        rusqlite::params![revisao.to_string(), org.to_string(), fluxo.to_string()],
    )
    .expect("revisão");
    (fluxo, revisao)
}

// ───────────────────────── efeitos de teste (pelo tradutor do produto) ─────────────────────────

/// `mensagem_ao_cliente` como o motor descreve e como a fila grava.
fn ao_cliente(texto: &str) -> TransitionEffect {
    efeito_da_transicao(&EfeitoProduzido {
        chave: "etapa:boas_vindas/0".to_owned(),
        no: "etapa:boas_vindas".to_owned(),
        indice: 0,
        efeito: Efeito::MensagemAoCliente {
            texto: texto.to_owned(),
        },
    })
}

/// `notificar_whatsapp` para a equipe.
fn notificar(mensagem: &str) -> TransitionEffect {
    efeito_da_transicao(&EfeitoProduzido {
        chave: "acao:boas_vindas.registrar/0".to_owned(),
        no: "acao:boas_vindas.registrar".to_owned(),
        indice: 0,
        efeito: Efeito::NotificarWhatsapp {
            para: TELEFONE_EQUIPE.to_owned(),
            mensagem: mensagem.to_owned(),
        },
    })
}

/// `encaminhar` para o departamento pelo nome.
fn encaminhar(departamento: &str) -> TransitionEffect {
    efeito_da_transicao(&EfeitoProduzido {
        chave: "acao:boas_vindas.transferir/0".to_owned(),
        no: "acao:boas_vindas.transferir".to_owned(),
        indice: 0,
        efeito: Efeito::Encaminhar {
            departamento: departamento.to_owned(),
        },
    })
}

fn departamento(cenario: &Cenario, nome: &str) -> DepartmentId {
    cenario
        .store
        .departments()
        .create(
            cenario.org,
            NewDepartment {
                name: nome.to_owned(),
                description: String::new(),
            },
        )
        .expect("cria o departamento")
        .id
}

// ───────────────────────── 1. mensagem ao cliente, ponta a ponta ─────────────────────────

#[tokio::test]
async fn mensagem_ao_cliente_chega_ao_cliente_uma_vez() {
    let cenario = Cenario::novo("ao-cliente", Instancia::Normal, Vec::new()).await;
    let aberta = cenario.abrir(vec![ao_cliente("Recebemos seu pedido!")]);
    let execucao = aberta.execution.id;
    assert_eq!(
        aberta.effects.len(),
        1,
        "o efeito entrou na fila na abertura"
    );

    let efeitos = cenario.efeitos();
    let relato = efeitos.ciclo_da_org(cenario.org).await.expect("ciclo");
    assert_eq!(relato.enviados, 1, "um efeito entregue");

    let fila = cenario.efeitos_da_ficha(execucao);
    assert_eq!(fila[0].state, EffectState::Sent);
    assert!(fila[0].enviado_em.is_some(), "carimbo de entrega");

    // A mensagem entrou na `outbox` da conversa, uma vez.
    let itens = cenario.itens_da_saida();
    assert_eq!(itens.len(), 1, "um item na outbox");
    assert_eq!(itens[0].conversation_id, cenario.conversa);
    assert_eq!(
        serde_json::from_str::<Value>(&itens[0].content_json).expect("payload")["texto"],
        "Recebemos seu pedido!"
    );

    // Ponta a ponta: a saída do WhatsApp entrega ao cliente pela Evolution falsa.
    let saida = cenario.saida();
    let enviados = saida.ciclo().await.expect("ciclo da saída");
    assert_eq!(enviados.enviados, 1);
    let registros = cenario.registros();
    assert_eq!(registros.len(), 1, "uma chamada à Evolution");
    assert_eq!(registros[0].corpo["number"], TELEFONE);
    assert_eq!(registros[0].corpo["text"], "Recebemos seu pedido!");
    assert_eq!(
        registros[0].caminho, "instancia=loja",
        "pela instância da org"
    );

    // Segunda passagem: nada é reenviado.
    let relato = efeitos
        .ciclo_da_org(cenario.org)
        .await
        .expect("segundo ciclo");
    assert_eq!(relato.enviados, 0, "efeito entregue não volta para a fila");
    assert_eq!(cenario.total_de_itens(), 1, "sem segundo item na outbox");

    // Métricas: um efeito entrou na fila, um foi entregue, nenhuma falha.
    assert_eq!(cenario.metricas.efeitos_pendentes.value(), 1);
    assert_eq!(cenario.metricas.efeitos_enviados.value(), 1);
    assert_eq!(cenario.metricas.efeitos_falhas.value(), 0);

    // Evento na linha do tempo da Ficha, uma vez.
    let enviados: Vec<_> = cenario
        .eventos(cenario.ficha)
        .into_iter()
        .filter(|evento| evento.tipo == RecordEventKind::EffectSent)
        .collect();
    assert_eq!(enviados.len(), 1, "um evento `efeito_enviado`");
}

// ───────────────────────── 2. notificação para outro número ─────────────────────────

#[tokio::test]
async fn notificar_whatsapp_vai_para_a_equipe_e_nao_para_o_cliente() {
    let cenario = Cenario::novo("notificar", Instancia::Normal, Vec::new()).await;
    let aberta = cenario.abrir(vec![notificar("Visita marcada para amanhã")]);
    let execucao = aberta.execution.id;

    let relato = cenario
        .efeitos()
        .ciclo_da_org(cenario.org)
        .await
        .expect("ciclo");
    assert_eq!(relato.enviados, 1);

    let registros = cenario.registros();
    assert_eq!(registros.len(), 1);
    assert_eq!(
        registros[0].corpo["number"], TELEFONE_EQUIPE,
        "vai para o número da equipe"
    );
    assert_ne!(
        registros[0].corpo["number"], TELEFONE,
        "não vai para o cliente"
    );
    assert_eq!(registros[0].corpo["text"], "Visita marcada para amanhã");
    assert_eq!(registros[0].apikey.as_deref(), Some(CHAVE));

    let fila = cenario.efeitos_da_ficha(execucao);
    assert_eq!(fila[0].state, EffectState::Sent);
    assert!(
        cenario.itens_da_saida().is_empty(),
        "a notificação não usa a outbox da conversa"
    );
}

// ───────────────────────── 3. sem integração: pendente, sem falhar ─────────────────────────

#[tokio::test]
async fn sem_integracao_o_efeito_fica_pendente_e_nao_trava_o_laco() {
    let cenario = Cenario::novo("sem-chave", Instancia::SemChave, Vec::new()).await;
    let aberta = cenario.abrir(vec![notificar("aviso para a equipe")]);
    let execucao = aberta.execution.id;

    let efeitos = cenario.efeitos();
    let relato = efeitos.ciclo_da_org(cenario.org).await.expect("ciclo");
    assert_eq!(relato.adiados, 1, "espera, não falha");
    assert_eq!(relato.definitivos, 0);

    let fila = cenario.efeitos_da_ficha(execucao);
    assert_eq!(fila[0].state, EffectState::Pending, "continua na fila");
    assert_eq!(fila[0].attempts, 0, "esperar não consome tentativa");
    assert!(fila[0].ultimo_erro.is_none(), "esperar não é erro");
    assert!(fila[0].proxima_tentativa.is_some(), "a espera fica marcada");

    // No ciclo seguinte, antes da espera vencer, o efeito nem é lido: nada de laço apertado.
    let relato = efeitos.ciclo_da_org(cenario.org).await.expect("ciclo");
    assert_eq!(relato.adiados, 0, "a espera segura o efeito");
    assert_eq!(cenario.registros().len(), 0, "nenhuma chamada à Evolution");

    // E o laço não fica preso nele: um efeito de outra Ficha é entregue no mesmo ciclo.
    let outra = cenario.outra_ficha(None);
    cenario.abrir_na_ficha(outra, vec![ao_cliente("Mensagem de outra Ficha")]);
    cenario.avancar(60);
    let relato = efeitos.ciclo_da_org(cenario.org).await.expect("ciclo");
    assert_eq!(relato.enviados, 1, "o outro efeito foi entregue");
    assert_eq!(relato.adiados, 1, "o da equipe continua esperando");
}

// ───────────────────────── 4. queda no meio: reenvia e aceita duplicata ─────────────────────────

#[tokio::test]
async fn queda_entre_enfileirar_e_confirmar_reenvia_e_aceita_duplicata() {
    let cenario = Cenario::novo("queda", Instancia::Normal, Vec::new()).await;
    let aberta = cenario.abrir(vec![ao_cliente("Bem-vindo!")]);
    let execucao = aberta.execution.id;
    let id = cenario.efeitos_da_ficha(execucao)[0].id;

    // Simula a janela da queda: o trabalhador pegou o efeito (`enviando`) e enfileirou a mensagem,
    // e morreu antes de confirmar. É exatamente o que o entregador faz em duas chamadas separadas.
    cenario
        .store
        .effects()
        .start_send(cenario.org, id)
        .expect("pega o efeito");
    let (_, item) = cenario
        .store
        .outbox()
        .enqueue(
            cenario.org,
            poph_store::NewOutboxItem {
                conversation_id: cenario.conversa,
                turn_id: poph_types::TurnId::generate(),
                seq: 0,
                tipo: "text".to_owned(),
                content_json: json!({"texto": "Bem-vindo!"}).to_string(),
            },
        )
        .expect("enfileira a mensagem");
    assert!(!item.id.to_string().is_empty(), "o item da outbox existe");
    assert_eq!(
        cenario.efeitos_da_ficha(execucao)[0].state,
        EffectState::Sending,
        "a marca da tentativa ficou gravada"
    );

    // A passagem seguinte encontra `enviando` e entrega de novo (pelo menos uma vez, D11).
    let relato = cenario
        .efeitos()
        .ciclo_da_org(cenario.org)
        .await
        .expect("ciclo");
    assert_eq!(relato.enviados, 1, "o reenvio acontece");
    assert_eq!(
        cenario.efeitos_da_ficha(execucao)[0].state,
        EffectState::Sent
    );

    // A duplicata é o preço aceito: duas mensagens iguais na fila de saída, e o cliente recebe as
    // duas. Está documentado no PRD (entrega pelo menos uma vez).
    let itens = cenario.itens_da_saida();
    assert_eq!(itens.len(), 2, "a duplicata rara é aceita, não escondida");
    cenario.saida().ciclo().await.expect("ciclo da saída");
    assert_eq!(cenario.registros().len(), 2, "as duas chegam ao cliente");
}

// ───────────────────────── 5. falha definitiva ─────────────────────────

#[tokio::test]
async fn falha_definitiva_vira_falhou_com_evento_e_metrica() {
    let cenario = Cenario::novo(
        "definitiva",
        Instancia::Normal,
        vec![(400, json!({"message": "número inválido"}))],
    )
    .await;
    let aberta = cenario.abrir(vec![notificar("aviso")]);
    let execucao = aberta.execution.id;

    let relato = cenario
        .efeitos()
        .ciclo_da_org(cenario.org)
        .await
        .expect("ciclo");
    assert_eq!(relato.definitivos, 1, "4xx não repete");

    let fila = cenario.efeitos_da_ficha(execucao);
    assert_eq!(fila[0].state, EffectState::Failed);
    assert!(fila[0].ultimo_erro.is_some(), "o motivo fica gravado");
    assert_eq!(fila[0].attempts, 1);
    assert_eq!(cenario.metricas.efeitos_falhas.value(), 1);

    // A transição continua valendo (PRD L11): execução ativa, conversa aberta.
    let atual = cenario
        .store
        .executions()
        .get(cenario.org, execucao)
        .expect("lê a execução");
    assert_eq!(atual.status, ExecutionStatus::Active);
    assert_eq!(cenario.conversa().status, ConversationStatus::Open);

    let falhas: Vec<_> = cenario
        .eventos(cenario.ficha)
        .into_iter()
        .filter(|evento| evento.tipo == RecordEventKind::EffectFailed)
        .collect();
    assert_eq!(falhas.len(), 1, "um evento `efeito_falhou`");

    // E o efeito não volta para a fila.
    let relato = cenario
        .efeitos()
        .ciclo_da_org(cenario.org)
        .await
        .expect("ciclo");
    assert_eq!(relato, RelatoEfeitos::default(), "nada mais a fazer");
}

// ───────────────────────── 6. 5xx: tenta de novo e passa ─────────────────────────

#[tokio::test]
async fn falha_transitoria_tenta_de_novo_e_passa() {
    let cenario = Cenario::novo(
        "transitoria",
        Instancia::Normal,
        vec![
            (503, json!({"message": "indisponível"})),
            (200, json!({"key": {"id": "MSG-FX21-2"}})),
        ],
    )
    .await;
    let aberta = cenario.abrir(vec![notificar("aviso")]);
    let execucao = aberta.execution.id;

    let efeitos = cenario.efeitos();
    let relato = efeitos.ciclo_da_org(cenario.org).await.expect("ciclo");
    assert_eq!(relato.transitorios, 1);
    let fila = cenario.efeitos_da_ficha(execucao);
    assert_eq!(fila[0].state, EffectState::Pending, "volta para a fila");
    assert_eq!(fila[0].attempts, 1);
    assert!(fila[0].proxima_tentativa.is_some(), "espera marcada");

    // Antes da espera vencer, não tenta de novo (não é laço apertado).
    let relato = efeitos.ciclo_da_org(cenario.org).await.expect("ciclo");
    assert_eq!(relato, RelatoEfeitos::default());
    assert_eq!(cenario.registros().len(), 1, "uma tentativa até aqui");

    // Vencida a espera, a segunda tentativa passa.
    cenario.avancar(50);
    let relato = efeitos.ciclo_da_org(cenario.org).await.expect("ciclo");
    assert_eq!(relato.enviados, 1);
    assert_eq!(cenario.registros().len(), 2);
    let fila = cenario.efeitos_da_ficha(execucao);
    assert_eq!(fila[0].state, EffectState::Sent);
    assert_eq!(fila[0].attempts, 2, "duas tentativas contadas");
}

// ───────────────────────── 7. encaminhar ─────────────────────────

#[tokio::test]
async fn encaminhar_transfere_a_conversa_e_mantem_a_ficha_aberta() {
    let cenario = Cenario::novo("encaminhar", Instancia::Normal, Vec::new()).await;
    let comercial = departamento(&cenario, "Comercial");
    let aberta = cenario.abrir(vec![encaminhar("Comercial")]);
    let execucao = aberta.execution.id;

    let relato = cenario
        .efeitos()
        .ciclo_da_org(cenario.org)
        .await
        .expect("ciclo");
    assert_eq!(relato.enviados, 1);

    // A conversa vai para o departamento e o agente para de responder (D12, guarda do turno).
    let conversa = cenario.conversa();
    assert_eq!(conversa.status, ConversationStatus::Transferred);
    assert_eq!(conversa.target_department, Some(comercial));

    // A Ficha continua aberta.
    let ficha = cenario
        .store
        .cases()
        .get(cenario.org, cenario.ficha)
        .expect("lê a Ficha");
    assert_eq!(ficha.status, RecordStatus::Open);

    // Auditoria da transferência, assinada pelo sistema.
    let auditoria = cenario
        .store
        .audit()
        .list(
            cenario.org,
            poph_store::Page {
                after: None,
                limit: 50,
            },
        )
        .expect("lê a auditoria");
    let transferencia = auditoria
        .iter()
        .find(|registro| registro.acao == "transfer")
        .expect("auditoria da transferência");
    assert_eq!(transferencia.ator_tipo, "system");

    // Dois eventos: o efeito entregue e o fato de negócio (a conversa foi transferida).
    let tipos: Vec<RecordEventKind> = cenario
        .eventos(cenario.ficha)
        .into_iter()
        .map(|evento| evento.tipo)
        .collect();
    assert!(tipos.contains(&RecordEventKind::EffectSent));
    assert!(tipos.contains(&RecordEventKind::ServiceTransferred));

    assert_eq!(
        cenario.efeitos_da_ficha(execucao)[0].state,
        EffectState::Sent
    );
}

// ───────────────────────── 8. departamento inexistente ─────────────────────────

#[tokio::test]
async fn departamento_inexistente_vira_falhou_sem_reverter_a_transicao() {
    let cenario = Cenario::novo("sem-departamento", Instancia::Normal, Vec::new()).await;
    // A org tem departamentos — só não tem o que o fluxo pede. É o caso que distingue "não achei o
    // nome" de "peguei o primeiro que apareceu".
    departamento(&cenario, "Comercial");
    departamento(&cenario, "Suporte");
    let aberta = cenario.abrir(vec![encaminhar("Financeiro")]);
    let execucao = aberta.execution.id;

    let relato = cenario
        .efeitos()
        .ciclo_da_org(cenario.org)
        .await
        .expect("ciclo");
    assert_eq!(relato.definitivos, 1);

    let fila = cenario.efeitos_da_ficha(execucao);
    assert_eq!(fila[0].state, EffectState::Failed);
    let motivo = fila[0].ultimo_erro.clone().unwrap_or_default();
    assert!(
        motivo.contains("Financeiro"),
        "o motivo diz qual departamento faltou: {motivo}"
    );

    // Nada revertido: conversa aberta, execução ativa, Ficha aberta.
    assert_eq!(cenario.conversa().status, ConversationStatus::Open);
    let atual = cenario
        .store
        .executions()
        .get(cenario.org, execucao)
        .expect("lê a execução");
    assert_eq!(atual.status, ExecutionStatus::Active);
}

// ───────────────────────── 9. limite de 4.000 caracteres ─────────────────────────

#[tokio::test]
async fn o_limite_de_4000_caracteres_do_whatsapp_e_respeitado() {
    let cenario = Cenario::novo("limite", Instancia::Normal, Vec::new()).await;
    let longo = "a".repeat(5_000);
    cenario.abrir(vec![notificar(&longo)]);

    let relato = cenario
        .efeitos()
        .ciclo_da_org(cenario.org)
        .await
        .expect("ciclo");
    assert_eq!(relato.enviados, 1);

    let registros = cenario.registros();
    let enviado = registros[0].corpo["text"]
        .as_str()
        .expect("texto enviado")
        .to_owned();
    assert_eq!(enviado.chars().count(), 4_000, "corta no teto do canal");
    assert!(enviado.ends_with('…'), "e avisa que cortou: {enviado:.10}");
}

// ───────────────────────── 10. telefone nunca em log ─────────────────────────

#[tokio::test]
async fn o_telefone_nao_aparece_em_log_nem_no_motivo_gravado() {
    let logs = logs_capturados();
    let cenario = Cenario::novo(
        "telefone",
        Instancia::Normal,
        vec![
            (200, json!({"key": {"id": "MSG-FX21-3"}})),
            (400, json!({"message": "destino recusado"})),
        ],
    )
    .await;
    // Duas notificações: a primeira passa, a segunda é recusada (400) — sucesso e falha no log.
    cenario.abrir(vec![notificar("aviso 1")]);
    let outra = cenario.outra_ficha(cenario.integracao);
    let falha = cenario.abrir_na_ficha(outra, vec![notificar("aviso 2")]);
    let efeitos = cenario.efeitos();
    let relato = efeitos.ciclo_da_org(cenario.org).await.expect("ciclo");
    assert_eq!(relato.enviados, 1);
    assert_eq!(relato.definitivos, 1);

    // O motivo gravado também não leva telefone.
    let motivo = cenario.efeitos_da_ficha(falha.execution.id)[0]
        .ultimo_erro
        .clone()
        .unwrap_or_default();
    assert!(!motivo.is_empty(), "a falha tem motivo");
    assert!(
        !motivo.contains(TELEFONE_EQUIPE),
        "motivo sem telefone: {motivo}"
    );
    assert!(!motivo.contains(CHAVE), "motivo sem chave: {motivo}");

    let texto = String::from_utf8_lossy(&logs.lock().unwrap()).to_string();
    assert!(!texto.is_empty(), "o coletor de log funcionou");
    assert!(
        !texto.contains(TELEFONE),
        "o telefone do cliente não pode aparecer no log"
    );
    assert!(
        !texto.contains(TELEFONE_EQUIPE),
        "o telefone da equipe não pode aparecer no log"
    );
    assert!(
        !texto.contains(CHAVE),
        "a chave da Evolution não pode aparecer no log"
    );
}

// ───────────────────────── 11. espera o turno da conversa ─────────────────────────

#[tokio::test]
async fn a_mensagem_ao_cliente_espera_o_turno_da_conversa() {
    let cenario = Cenario::novo("espera-turno", Instancia::Normal, Vec::new()).await;
    let aberta = cenario.abrir(vec![ao_cliente("Mensagem fixa da etapa")]);
    let execucao = aberta.execution.id;

    // O turno da conversa está rodando: a mensagem fixa não pode chegar antes da resposta.
    let turno = cenario
        .store
        .turns()
        .start(cenario.org, cenario.conversa, Some(cenario.agente.id))
        .expect("abre o turno");

    let efeitos = cenario.efeitos();
    let relato = efeitos.ciclo_da_org(cenario.org).await.expect("ciclo");
    assert_eq!(relato.adiados, 1);
    assert!(cenario.itens_da_saida().is_empty(), "nada na outbox ainda");
    let fila = cenario.efeitos_da_ficha(execucao);
    assert_eq!(fila[0].state, EffectState::Pending);
    assert_eq!(fila[0].attempts, 0, "esperar o turno não consome tentativa");

    // Terminado o turno, a mensagem entra na fila de saída.
    cenario
        .store
        .turns()
        .finalise(
            cenario.org,
            turno.id,
            poph_store::TurnFinalization {
                state: Some(poph_store::TurnState::Completed),
                ..Default::default()
            },
        )
        .expect("fecha o turno");
    cenario.avancar(50);
    let relato = efeitos.ciclo_da_org(cenario.org).await.expect("ciclo");
    assert_eq!(relato.enviados, 1);
    assert_eq!(cenario.itens_da_saida().len(), 1);
}

// ───────────────────────── 12. conversa encerrada ─────────────────────────

#[tokio::test]
async fn mensagem_ao_cliente_com_conversa_encerrada_vira_falhou() {
    let cenario = Cenario::novo("encerrada", Instancia::Normal, Vec::new()).await;
    let aberta = cenario.abrir(vec![ao_cliente("Mensagem fixa")]);
    let execucao = aberta.execution.id;

    // O atendimento fecha antes de o efeito ser entregue.
    cenario
        .store
        .conversations()
        .close_audited(
            cenario.org,
            cenario.conversa,
            cenario.conversa().version,
            "atendente-de-teste".to_owned(),
        )
        .expect("fecha a conversa");

    let relato = cenario
        .efeitos()
        .ciclo_da_org(cenario.org)
        .await
        .expect("ciclo");
    assert_eq!(relato.definitivos, 1);

    let fila = cenario.efeitos_da_ficha(execucao);
    assert_eq!(fila[0].state, EffectState::Failed);
    assert!(
        fila[0]
            .ultimo_erro
            .as_deref()
            .unwrap_or_default()
            .contains("encerrada"),
        "o motivo diz que a conversa está encerrada: {:?}",
        fila[0].ultimo_erro
    );
    assert_eq!(
        cenario.total_de_itens(),
        0,
        "nada entra na outbox de conversa encerrada"
    );
}

// ───────────────────────── 12. esgotamento de tentativas ─────────────────────────

#[tokio::test]
async fn tentativas_esgotadas_viram_falhou_com_o_motivo_certo() {
    let cenario = Cenario::novo(
        "esgotado",
        Instancia::Normal,
        vec![(503, json!({"message": "indisponível"}))],
    )
    .await;
    let aberta = cenario.abrir(vec![notificar("aviso")]);
    let execucao = aberta.execution.id;
    let efeitos = EfeitosFluxo::novo(
        Arc::clone(&cenario.store),
        &cenario.canais,
        cenario.blobs.clone(),
        Some(Arc::clone(&cenario.metricas)),
        OpcoesEfeitosFluxo {
            max_tentativas: 2,
            backoff: Backoff {
                base: Duration::from_millis(1),
                factor: 2,
                cap: Duration::from_millis(5),
            },
            espera_sem_integracao: Duration::from_millis(10),
            ..OpcoesEfeitosFluxo::default()
        },
    );

    efeitos.ciclo_da_org(cenario.org).await.expect("1ª");
    cenario.avancar(50);
    efeitos.ciclo_da_org(cenario.org).await.expect("2ª");
    cenario.avancar(50);
    let relato = efeitos.ciclo_da_org(cenario.org).await.expect("3ª");
    assert_eq!(relato.esgotados, 1);

    let fila = cenario.efeitos_da_ficha(execucao);
    assert_eq!(fila[0].state, EffectState::Failed);
    assert_eq!(fila[0].attempts, 2, "o teto de tentativas vale");
    assert_eq!(
        fila[0].ultimo_erro.as_deref(),
        Some(MOTIVO_TENTATIVAS_ESGOTADAS)
    );
    assert_eq!(cenario.metricas.efeitos_falhas.value(), 1);
}
