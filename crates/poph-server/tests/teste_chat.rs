//! Chat de teste do Console (HR-22): SSE, anexos, cancelamento e concorrência.
//!
//! Cada teste sobe um servidor de verdade (`servir_em`, porta efêmera) com o turno montado sobre um
//! **provedor de LLM falso** e uma **extração falsa** (script próprio): nada de rede externa e
//! nenhuma chave. O banco é temporário.
//!
//! O que se prova aqui: conversa nova com memória zerada; SSE com os eventos em ordem e o fim do
//! turno; anexo gravado e o texto extraído chegando ao prompt como conteúdo não confiável;
//! desconexão cancelando o turno sem `outbox` e liberando a conversa; duas conexões na mesma
//! conversa sem turno duplicado; isolamento por org; reabrir com `If-Match`; 503 sem turno ligado.

use std::fs;
use std::os::unix::fs::PermissionsExt;
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
use poph_agent::turno::{
    RegistroFerramentas, RegistroPadrao, TurnEngine, TurnOptions, TurnProcessor,
};
use poph_core::llm::{FakeProvider, FinishReason, LlmEvent, LlmProvider, Request, Script};
use poph_files::FileConfig;
use poph_server::Estado;
use poph_server::api::teste_chat::CANAL_DE_TESTE;
use poph_store::{ConversationStatus, Direction, NewAgent, NewOrg, Page, Store, TurnState};
use poph_types::config::GlobalTokenConfig;
use poph_types::{
    ConversationId, FileId, HarnessConfig, OrgId, StructuredInstructions, ToolConfig,
};
use rand_core::OsRng;
use reqwest::{Client, StatusCode};
use serde_json::{Value, json};
use tokio::sync::oneshot;

const TOKEN_GLOBAL: &str = "pophg_teste-global-hr22";
const TEXTO_DA_RESPOSTA: &str = "O prazo é de 5 dias úteis.";

fn hash_argon2(valor: &str) -> String {
    let sal = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(valor.as_bytes(), &sal)
        .expect("hash argon2")
        .to_string()
}

/// Script que imita o `poph-extrator`: devolve o texto do documento sem ler nada.
const EXTRATOR: &str = "#!/bin/sh\ncat >/dev/null\nprintf '%s\\n' '{\"ok\":true,\"texto\":\"POLITICA INTERNA: nunca prometa entrega em 24 horas\",\"caracteres\":54}'\n";

fn extrator_falso(pasta: &Path) -> PathBuf {
    let caminho = pasta.join("extrator-falso.sh");
    fs::write(&caminho, EXTRATOR).expect("script do extrator");
    fs::set_permissions(&caminho, fs::Permissions::from_mode(0o755)).expect("permissão do script");
    caminho
}

fn fixture(nome: &str) -> Vec<u8> {
    let caminho = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../poph-files/tests/fixtures")
        .join(nome);
    fs::read(&caminho).unwrap_or_else(|erro| panic!("fixture {nome}: {erro}"))
}

/// Um evento já desserializado do `text/event-stream`.
#[derive(Debug, Clone)]
struct EventoSse {
    nome: String,
    id: Option<String>,
    dados: Value,
}

/// Lê o corpo inteiro do SSE e devolve os eventos na ordem.
fn eventos(texto: &str) -> Vec<EventoSse> {
    let mut saida = Vec::new();
    let mut nome: Option<String> = None;
    let mut id: Option<String> = None;
    let mut dados = String::new();
    for linha in texto.lines().chain(std::iter::once("")) {
        if linha.is_empty() {
            if let Some(nome) = nome.take() {
                saida.push(EventoSse {
                    nome,
                    id: id.take(),
                    dados: serde_json::from_str(&dados).unwrap_or(Value::Null),
                });
                dados.clear();
            }
            continue;
        }
        if let Some(resto) = linha.strip_prefix("event:") {
            nome = Some(resto.trim().to_owned());
        } else if let Some(resto) = linha.strip_prefix("id:") {
            id = Some(resto.trim().to_owned());
        } else if let Some(resto) = linha.strip_prefix("data:") {
            dados.push_str(resto.trim_start());
        }
    }
    saida
}

fn nomes(eventos: &[EventoSse]) -> Vec<String> {
    eventos.iter().map(|e| e.nome.clone()).collect()
}

/// Corpo multipart montado à mão (o cliente de teste não traz a feature `multipart`).
fn multipart(texto: Option<&str>, arquivos: &[(&str, &[u8])]) -> (String, Vec<u8>) {
    let limite = "hr22-boundary";
    let mut corpo = Vec::new();
    if let Some(texto) = texto {
        corpo.extend_from_slice(
            format!(
                "--{limite}\r\nContent-Disposition: form-data; name=\"text\"\r\n\r\n{texto}\r\n"
            )
            .as_bytes(),
        );
    }
    for (nome, bytes) in arquivos {
        corpo.extend_from_slice(
            format!(
                "--{limite}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{nome}\"\r\nContent-Type: application/octet-stream\r\n\r\n"
            )
            .as_bytes(),
        );
        corpo.extend_from_slice(bytes);
        corpo.extend_from_slice(b"\r\n");
    }
    corpo.extend_from_slice(format!("--{limite}--\r\n").as_bytes());
    (format!("multipart/form-data; boundary={limite}"), corpo)
}

struct Cenario {
    base: String,
    cliente: Client,
    store: Arc<Store>,
    org: OrgId,
    agente: poph_types::AgentId,
    llm: Arc<FakeProvider>,
    /// Arquivo semeado na biblioteca do agente (HR-46d), quando o cenário pediu um.
    arquivo: Option<FileId>,
    _pasta: PathBuf,
    desligar: Option<oneshot::Sender<()>>,
}

impl Drop for Cenario {
    fn drop(&mut self) {
        if let Some(desligar) = self.desligar.take() {
            let _ = desligar.send(());
        }
        let _ = fs::remove_dir_all(&self._pasta);
    }
}

impl Cenario {
    /// Sobe o servidor. `ligar_turno = false` prova o `503` sem chave.
    async fn novo(rotulo: &str, roteiros: Vec<Script>, ligar_turno: bool) -> Self {
        Self::construir(rotulo, ligar_turno, None, |_| roteiros).await
    }

    /// Sobe o servidor com um arquivo na biblioteca do agente e a ferramenta `enviar_arquivo`
    /// habilitada (HR-46d). Os roteiros recebem o id desse arquivo para o modelo pedi-lo.
    async fn com_biblioteca(
        rotulo: &str,
        conteudo: &[u8],
        roteiros: impl FnOnce(FileId) -> Vec<Script>,
    ) -> Self {
        Self::construir(rotulo, true, Some(conteudo), |id| {
            roteiros(id.expect("arquivo semeado"))
        })
        .await
    }

    /// Sobe o servidor. `semear` é o conteúdo de um arquivo que entra na biblioteca do agente (com
    /// a ferramenta `enviar_arquivo` habilitada) e `roteiros` monta o que o provedor falso responde
    /// — ele recebe o id do arquivo semeado, quando houver.
    async fn construir(
        rotulo: &str,
        ligar_turno: bool,
        semear: Option<&[u8]>,
        roteiros: impl FnOnce(Option<FileId>) -> Vec<Script>,
    ) -> Self {
        let carimbo = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let pasta = std::env::temp_dir().join(format!(
            "poph-teste-chat-{rotulo}-{}-{carimbo}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&pasta);
        fs::create_dir_all(pasta.join("dados")).expect("pasta de dados");

        let mut config = HarnessConfig::default();
        config.data.directory = pasta.join("dados");
        config.server.port = 7483;
        config.admin.tokens_globais = vec![GlobalTokenConfig {
            label: "teste".to_owned(),
            hash: hash_argon2(TOKEN_GLOBAL),
        }];
        config.validar().expect("configuração válida");

        let store = Arc::new(Store::open(&config).expect("abre o banco"));
        let org = store
            .orgs()
            .create(NewOrg {
                name: "Org HR-22".to_owned(),
                slug: "org-hr22".to_owned(),
                config_json: "{}".to_owned(),
            })
            .expect("org")
            .id;
        let agente = store
            .agents()
            .create(
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

        // HR-46d: biblioteca com um arquivo e `enviar_arquivo` habilitada nas instruções — é por
        // elas que o turno sabe quais ferramentas oferecer ao modelo (D-04).
        let arquivo = semear.map(|bytes| {
            let blobs = pasta.join("dados").join("blobs");
            let gravado = poph_files::blob::write(&blobs, org, bytes).expect("grava o blob");
            let arquivo = store
                .library()
                .create(
                    org,
                    poph_store::NewLibraryFile {
                        agent_id: agente,
                        name: "catalogo.pdf".to_owned(),
                        description: "catálogo de serviços".to_owned(),
                        mime: "application/pdf".to_owned(),
                        size_bytes: bytes.len() as u64,
                        sha256: gravado.sha256.clone(),
                        blob_path: format!("{org}/{}", gravado.sha256),
                    },
                )
                .expect("arquivo na biblioteca");
            let instrucoes = StructuredInstructions {
                tools: vec![ToolConfig {
                    name: poph_agent::NOME_ENVIAR_ARQUIVO.to_owned(),
                    enabled: true,
                    when_to_use: String::new(),
                }],
                ..StructuredInstructions::default()
            };
            store
                .instructions()
                .save(
                    org,
                    agente,
                    0,
                    &instrucoes.to_json(),
                    "# Bia\n\nVocê é a atendente da FixProLar.",
                    true,
                )
                .expect("instruções com a ferramenta");
            arquivo.id
        });

        let fake = Arc::new(FakeProvider::with_scripts(roteiros(arquivo)));
        let mut estado = Estado::novo(Arc::clone(&store), config).expect("estado do servidor");
        let mut arquivos = FileConfig::default(pasta.join("dados").join("blobs"));
        arquivos.extractor_path = Some(extrator_falso(&pasta));
        estado = estado.com_arquivos(arquivos);

        if ligar_turno {
            let provedor: Arc<dyn LlmProvider> = fake.clone();
            let mut motor = TurnEngine::new(
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
                TurnOptions::default(),
                MetricasTurno::proprio(),
            );
            if arquivo.is_some() {
                let registro =
                    poph_agent::biblioteca::registrar(RegistroPadrao::novo(), Arc::clone(&store));
                motor = motor.with_tools(Arc::new(registro) as Arc<dyn RegistroFerramentas>);
            }
            let motor = Arc::new(motor);
            let processador = Arc::new(TurnProcessor::new(Arc::clone(&motor)));
            let atores = Atores::novo(
                processador.clone(),
                OpcoesAtores::default(),
                MetricasMemoria::proprio(),
            );
            let fluxo = Fluxo::nova(Arc::clone(&store), atores, OpcoesFluxo::default());
            processador.attach_io(fluxo.clone());
            estado = estado
                .com_motor(motor)
                .com_turno(fluxo, Duration::from_millis(20));
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
        let cenario = Self {
            base: format!("http://{endereco}"),
            cliente: Client::builder()
                .timeout(Duration::from_secs(20))
                .build()
                .expect("cliente HTTP"),
            store,
            org,
            agente,
            llm: fake,
            arquivo,
            _pasta: pasta,
            desligar: Some(desligar),
        };
        cenario.esperar_subir().await;
        cenario
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
        assert_eq!(corpo["channel"], CANAL_DE_TESTE);
        assert_eq!(corpo["status"], "open");
        assert_eq!(corpo["agent_id"], self.agente.to_string());
        corpo["id"].as_str().expect("id da conversa").to_owned()
    }

    fn url_mensagens(&self, conversa: &str) -> String {
        format!("{}{}/{conversa}/messages", self.base, self.raiz())
    }

    async fn enviar(
        &self,
        conversa: &str,
        texto: Option<&str>,
        arquivos: &[(&str, &[u8])],
    ) -> reqwest::Response {
        let (tipo, corpo) = multipart(texto, arquivos);
        self.cliente
            .post(self.url_mensagens(conversa))
            .bearer_auth(TOKEN_GLOBAL)
            .header("content-type", tipo)
            .body(corpo)
            .send()
            .await
            .expect("envia a mensagem")
    }

    async fn historico(&self, conversa: &str) -> Value {
        self.cliente
            .get(format!("{}{}/{conversa}", self.base, self.raiz()))
            .bearer_auth(TOKEN_GLOBAL)
            .send()
            .await
            .expect("histórico")
            .json()
            .await
            .expect("json do histórico")
    }

    /// Espera o último turno sair de `running`.
    async fn esperar_turno_terminar(&self, conversa: &str) -> Value {
        let id: ConversationId = conversa.parse().expect("id da conversa");
        let inicio = Instant::now();
        loop {
            let turnos = self
                .store
                .turns()
                .list(self.org, id, Page::with_limit(50))
                .expect("turnos");
            if let Some(turno) = turnos.last()
                && turno.state != TurnState::Running
            {
                return json!({
                    "state": turno.state.as_text(),
                    "error": turno.error,
                });
            }
            assert!(
                inicio.elapsed() < Duration::from_secs(10),
                "o turno não terminou a tempo: {turnos:?}"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    fn mensagens_saida(&self, conversa: &str) -> usize {
        let id: ConversationId = conversa.parse().expect("id da conversa");
        self.store
            .messages()
            .list(self.org, id, Page::with_limit(200))
            .expect("mensagens")
            .iter()
            .filter(|m| m.direction == Direction::Outbound)
            .count()
    }
}

fn roteiro_padrao() -> Vec<Script> {
    vec![Script::Events(vec![
        LlmEvent::Texto("O prazo é ".into()),
        LlmEvent::Texto("de 5 dias úteis.".into()),
        LlmEvent::Fim(FinishReason::Completed),
    ])]
}

fn texto_do_pedido(pedido: &Request) -> String {
    let mut texto = String::new();
    for mensagem in &pedido.messages {
        match mensagem {
            poph_core::llm::Message::User(partes) => {
                for parte in partes {
                    if let poph_core::llm::Part::Texto(trecho) = parte {
                        texto.push_str(trecho);
                        texto.push('\n');
                    }
                }
            }
            poph_core::llm::Message::System(instrucoes) => texto.push_str(instrucoes),
            _ => {}
        }
    }
    texto
}

#[tokio::test]
async fn conversa_de_teste_recebe_sse_em_ordem_e_anexo_com_texto_no_prompt() {
    let c = Cenario::novo("stream", roteiro_padrao(), true).await;
    let conversa = c.criar_conversa().await;

    let docx = fixture("procedimento.docx");
    let resposta = c
        .enviar(
            &conversa,
            Some("qual o prazo?"),
            &[("procedimento.docx", &docx)],
        )
        .await;
    assert_eq!(resposta.status(), StatusCode::OK);
    assert!(
        resposta
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.starts_with("text/event-stream")),
        "content-type do SSE"
    );
    let corpo = resposta.text().await.expect("corpo do SSE");
    let eventos = eventos(&corpo);

    // 1. Ordem: `mensagem` confirma o que foi gravado, depois o turno abre, os deltas chegam na
    //    ordem e o `fim` fecha.
    assert_eq!(eventos[0].nome, "mensagem", "eventos: {}", corpo);
    assert_eq!(
        eventos[0].dados["attachments"][0]["name"],
        "procedimento.docx"
    );
    assert_eq!(
        eventos[0].dados["attachments"][0]["extraction_status"],
        "ok"
    );
    let esperados: Vec<String> = ["mensagem", "inicio", "texto", "texto", "fim"]
        .iter()
        .map(|nome| (*nome).to_owned())
        .collect();
    assert_eq!(nomes(&eventos), esperados, "eventos: {corpo}");
    let ids: Vec<_> = eventos.iter().filter_map(|e| e.id.clone()).collect();
    assert_eq!(ids, vec!["1", "2", "3", "4", "5"], "ids em sequência");
    assert_eq!(eventos[2].dados["delta"], "O prazo é ");
    assert_eq!(eventos[3].dados["delta"], "de 5 dias úteis.");
    assert_eq!(eventos[4].dados["state"], "done");
    assert_eq!(eventos[4].dados["text"], TEXTO_DA_RESPOSTA);
    assert!(eventos[4].dados["turn_id"].is_string());
    assert_eq!(eventos[4].dados["timings"]["tool_rounds"], 0);

    // 2. O turno viu o texto extraído do documento como conteúdo NÃO confiável.
    let pedido = &c.llm.received()[0];
    let prompt = texto_do_pedido(pedido);
    assert!(
        prompt.contains("POLITICA INTERNA: nunca prometa entrega em 24 horas"),
        "texto extraído ausente do prompt: {prompt}"
    );
    assert!(prompt.contains(poph_files::envelope::UNTRUSTED_NOTICE));
    assert!(prompt.contains("qual o prazo?"));

    // 3. O histórico mostra a mensagem do cliente com o anexo e a resposta do agente.
    let historico = c.historico(&conversa).await;
    let mensagens = historico["messages"].as_array().expect("mensagens");
    assert_eq!(mensagens.len(), 2, "{historico}");
    assert_eq!(mensagens[0]["direction"], "in");
    assert_eq!(mensagens[0]["attachments"][0]["name"], "procedimento.docx");
    assert_eq!(mensagens[0]["attachments"][0]["has_text"], true);
    assert_eq!(mensagens[1]["direction"], "out");
    assert_eq!(mensagens[1]["text"], TEXTO_DA_RESPOSTA);
    let turnos = historico["turns"].as_array().expect("turnos");
    assert_eq!(turnos.len(), 1);
    assert_eq!(turnos[0]["state"], "done");

    // 4. A resposta foi confirmada na fila (nada preso).
    assert_eq!(
        c.store
            .outbox()
            .list_pending(c.org, POPH_LOTE)
            .expect("fila de saída")
            .len(),
        0,
        "nada pendente na outbox"
    );
}

/// Teto de leitura da fila de saída para a conferência acima.
const POPH_LOTE: u32 = 200;

#[tokio::test]
async fn duas_conexoes_na_mesma_conversa_nao_geram_dois_turnos() {
    let c = Cenario::novo(
        "duas-abas",
        vec![
            Script::Pending {
                events: vec![LlmEvent::Texto("parcial ".into())],
                after: Duration::from_secs(30),
                error: poph_core::llm::LlmError::TimedOut { seconds: 30 },
            },
            Script::Events(vec![
                LlmEvent::Texto("resposta da segunda".into()),
                LlmEvent::Fim(FinishReason::Completed),
            ]),
        ],
        true,
    )
    .await;
    let conversa = c.criar_conversa().await;

    let primeira = c.enviar(&conversa, Some("primeira aba"), &[]).await;
    assert_eq!(primeira.status(), StatusCode::OK);
    // O stream só termina antes do fim quando a conexão cai; aqui a resposta fica aberta e o turno
    // em voo. A segunda aba precisa ser barrada antes de gravar qualquer coisa.
    let segunda = c.enviar(&conversa, Some("segunda aba"), &[]).await;
    assert_eq!(segunda.status(), StatusCode::CONFLICT);
    let corpo: Value = segunda.json().await.expect("erro");
    assert_eq!(corpo["code"], "turno_em_andamento");

    drop(primeira);
    let estado = c.esperar_turno_terminar(&conversa).await;
    assert_eq!(estado["state"], "failed", "cancelado: {estado}");
    assert_eq!(c.mensagens_saida(&conversa), 0, "sem resposta parcial");

    // A conversa volta a aceitar mensagem depois que o turno cancela: a reserva foi liberada.
    let terceira = c.enviar(&conversa, Some("agora vai"), &[]).await;
    assert_eq!(terceira.status(), StatusCode::OK);
    let corpo = terceira.text().await.expect("SSE");
    let eventos = eventos(&corpo);
    assert_eq!(
        eventos
            .last()
            .map(|e| (e.nome.as_str(), e.dados["state"].as_str())),
        Some(("fim", Some("done"))),
        "eventos: {corpo}"
    );
    // Uma resposta só: a mensagem da segunda aba não entrou no turno por trás.
    assert_eq!(c.mensagens_saida(&conversa), 1);
}

#[tokio::test]
async fn desconexao_cancela_o_turno_sem_fila_presa() {
    let c = Cenario::novo(
        "cancelamento",
        vec![Script::Pending {
            events: vec![LlmEvent::Texto("começou ".into())],
            after: Duration::from_secs(30),
            error: poph_core::llm::LlmError::TimedOut { seconds: 30 },
        }],
        true,
    )
    .await;
    let conversa = c.criar_conversa().await;

    let resposta = c.enviar(&conversa, Some("me responde"), &[]).await;
    assert_eq!(resposta.status(), StatusCode::OK);
    // Lê até o primeiro pedaço de texto do modelo (o turno já está rodando) e fecha a conexão.
    use futures::StreamExt;
    let mut fluxo = resposta.bytes_stream();
    let mut recebido = String::new();
    let prazo = Instant::now();
    while !recebido.contains("começou") {
        let pedaco = tokio::time::timeout(Duration::from_secs(5), fluxo.next())
            .await
            .expect("o stream precisa abrir")
            .expect("o SSE precisa emitir")
            .expect("bytes do SSE");
        recebido.push_str(&String::from_utf8_lossy(&pedaco));
        assert!(prazo.elapsed() < Duration::from_secs(10), "SSE: {recebido}");
    }
    drop(fluxo);

    let estado = c.esperar_turno_terminar(&conversa).await;
    assert_eq!(estado["state"], "failed", "{estado}");
    let erro = estado["error"].as_str().unwrap_or_default().to_lowercase();
    assert!(
        erro.contains("cancel"),
        "o turno precisa registrar o cancelamento: {estado}"
    );
    // Nada de resposta parcial na conversa nem presa na fila.
    assert_eq!(c.mensagens_saida(&conversa), 0);
    assert_eq!(
        c.store
            .outbox()
            .list_pending(c.org, POPH_LOTE)
            .expect("fila")
            .len(),
        0
    );
}

#[tokio::test]
async fn sem_turno_ligado_a_rota_responde_503() {
    let c = Cenario::novo("sem-turno", Vec::new(), false).await;
    let conversa = c.criar_conversa().await;
    let resposta = c.enviar(&conversa, Some("oi"), &[]).await;
    assert_eq!(resposta.status(), StatusCode::SERVICE_UNAVAILABLE);
    let corpo: Value = resposta.json().await.expect("erro");
    assert_eq!(corpo["code"], "turno_indisponivel");
}

#[tokio::test]
async fn isolamento_por_org_e_reabertura_com_if_match() {
    let c = Cenario::novo("isolamento", Vec::new(), false).await;
    let conversa = c.criar_conversa().await;

    // Token de outra org: 403 nas três rotas.
    let outra = c
        .store
        .orgs()
        .create(NewOrg {
            name: "Outra".to_owned(),
            slug: "outra-org-hr22".to_owned(),
            config_json: "{}".to_owned(),
        })
        .expect("org")
        .id;
    let token = c
        .cliente
        .post(format!("{}/admin/v1/orgs/{outra}/tokens", c.base))
        .bearer_auth(TOKEN_GLOBAL)
        .json(&json!({"label": "outra"}))
        .send()
        .await
        .expect("token da outra org")
        .json::<Value>()
        .await
        .expect("json do token")["value"]
        .as_str()
        .expect("valor do token")
        .to_owned();
    for (metodo, url) in [
        ("GET", format!("{}{}", c.base, c.raiz())),
        ("GET", format!("{}{}/{}", c.base, c.raiz(), conversa)),
        ("POST", format!("{}{}", c.base, c.raiz())),
    ] {
        let pedido = if metodo == "GET" {
            c.cliente.get(url)
        } else {
            c.cliente.post(url)
        };
        let resposta = pedido.bearer_auth(&token).send().await.expect("pedido");
        assert_eq!(resposta.status(), StatusCode::FORBIDDEN, "{metodo}");
    }
    // Conversa de um agente não abre pelo outro agente.
    let outro_agente = c
        .store
        .agents()
        .create(
            c.org,
            NewAgent {
                name: "Outro".to_owned(),
                slug: "outro-agente-hr22".to_owned(),
                ..Default::default()
            },
        )
        .expect("agente")
        .id;
    let resposta = c
        .cliente
        .get(format!(
            "{}/admin/v1/orgs/{}/agents/{outro_agente}/test-chat/{conversa}",
            c.base, c.org
        ))
        .bearer_auth(TOKEN_GLOBAL)
        .send()
        .await
        .expect("histórico com outro agente");
    assert_eq!(resposta.status(), StatusCode::NOT_FOUND);

    // Reabrir: sem `If-Match` 428, versão velha 412, versão certa 200 (volta para `open`).
    let id: ConversationId = conversa.parse().expect("id");
    c.store
        .conversations()
        .change_status(c.org, id, 1, ConversationStatus::Transferred, None)
        .expect("transfere");
    let url = format!("{}{}/{conversa}/reopen", c.base, c.raiz());
    let sem = c
        .cliente
        .post(&url)
        .bearer_auth(TOKEN_GLOBAL)
        .send()
        .await
        .expect("sem If-Match");
    assert_eq!(sem.status(), StatusCode::PRECONDITION_REQUIRED);
    let velha = c
        .cliente
        .post(&url)
        .bearer_auth(TOKEN_GLOBAL)
        .header("If-Match", "1")
        .send()
        .await
        .expect("versão velha");
    assert_eq!(velha.status(), StatusCode::PRECONDITION_FAILED);
    let certa = c
        .cliente
        .post(&url)
        .bearer_auth(TOKEN_GLOBAL)
        .header("If-Match", "2")
        .send()
        .await
        .expect("versão certa");
    assert_eq!(certa.status(), StatusCode::OK);
    let corpo: Value = certa.json().await.expect("json");
    assert_eq!(corpo["status"], "open");
}

#[tokio::test]
async fn listagem_de_conversas_de_teste_e_mais_recente_primeiro_com_cursor() {
    let c = Cenario::novo("listagem", roteiro_padrao(), true).await;
    let primeira = c.criar_conversa().await;
    let segunda = c.criar_conversa().await;
    let terceira = c.criar_conversa().await;

    let url = format!("{}{}?limit=2", c.base, c.raiz());
    let pagina: Value = c
        .cliente
        .get(&url)
        .bearer_auth(TOKEN_GLOBAL)
        .send()
        .await
        .expect("lista")
        .json()
        .await
        .expect("json da lista");
    let itens = pagina["items"].as_array().expect("items");
    assert_eq!(itens.len(), 2);
    assert_eq!(itens[0]["id"], terceira);
    assert_eq!(itens[1]["id"], segunda);
    let cursor = pagina["next_cursor"].as_str().expect("cursor").to_owned();

    let proxima: Value = c
        .cliente
        .get(format!("{url}&cursor={cursor}"))
        .bearer_auth(TOKEN_GLOBAL)
        .send()
        .await
        .expect("lista com cursor")
        .json()
        .await
        .expect("json");
    let itens = proxima["items"].as_array().expect("items");
    assert_eq!(itens.len(), 1);
    assert_eq!(itens[0]["id"], primeira);
    assert_eq!(proxima["next_cursor"], Value::Null);

    // Mensagem sem texto e sem arquivo é recusada antes de gravar.
    let resposta = c.enviar(&primeira, None, &[]).await;
    assert_eq!(resposta.status(), StatusCode::BAD_REQUEST);
    let corpo: Value = resposta.json().await.expect("erro");
    assert_eq!(corpo["code"], "validation_error");
    assert_eq!(corpo["details"]["field"], "text");
}

#[tokio::test]
async fn arquivo_recusado_volta_no_campo_file_e_nao_grava_mensagem() {
    let c = Cenario::novo("arquivo-recusado", roteiro_padrao(), true).await;
    let conversa = c.criar_conversa().await;
    let resposta = c
        .enviar(
            &conversa,
            Some("segue a nota"),
            &[("nota.txt", b"oi, tudo bem?")],
        )
        .await;
    assert_eq!(resposta.status(), StatusCode::BAD_REQUEST);
    let corpo: Value = resposta.json().await.expect("erro");
    assert_eq!(corpo["code"], "validation_error");
    assert_eq!(corpo["details"]["field"], "file");
    let historico = c.historico(&conversa).await;
    assert_eq!(
        historico["messages"].as_array().map(|m| m.len()),
        Some(0),
        "nada gravado: {historico}"
    );
    // A conversa continua livre para a próxima mensagem.
    let ok = c.enviar(&conversa, Some("agora vai"), &[]).await;
    assert_eq!(ok.status(), StatusCode::OK);
    let corpo = ok.text().await.expect("SSE");
    assert!(corpo.contains("event: fim"), "{corpo}");
}

/// Roteiro de um turno que pede `enviar_arquivo` ao modelo e depois responde em texto (HR-46d).
fn roteiro_envia_arquivo(arquivo: FileId) -> Vec<Script> {
    vec![
        Script::Events(vec![
            LlmEvent::Tool(poph_core::llm::ToolCall {
                id: "call-catalogo".to_owned(),
                nome: poph_agent::NOME_ENVIAR_ARQUIVO.to_owned(),
                arguments: json!({ "id": arquivo.to_string() }),
            }),
            LlmEvent::Fim(FinishReason::ToolCalls),
        ]),
        Script::Events(vec![
            LlmEvent::Texto("Estou enviando o catálogo.".into()),
            LlmEvent::Fim(FinishReason::Completed),
        ]),
    ]
}

/// Arquivo que o **agente** enviou (HR-46d): o histórico diz qual é, sem vazar o caminho do blob, e
/// o download pela rota de conteúdo devolve os mesmos bytes — é o ciclo que a tela vai fechar.
#[tokio::test]
async fn historico_mostra_o_arquivo_do_agente_e_o_download_devolve_os_bytes() {
    const CONTEUDO: &[u8] = b"%PDF-1.4\ncatalogo de servicos HR-46d\n";
    let c = Cenario::com_biblioteca("arquivo-enviado", CONTEUDO, roteiro_envia_arquivo).await;
    let conversa = c.criar_conversa().await;

    let resposta = c.enviar(&conversa, Some("manda o catálogo"), &[]).await;
    assert_eq!(resposta.status(), StatusCode::OK);
    let corpo = resposta.text().await.expect("SSE");
    let eventos = eventos(&corpo);
    assert_eq!(
        eventos
            .last()
            .map(|e| (e.nome.as_str(), e.dados["state"].as_str())),
        Some(("fim", Some("done"))),
        "eventos: {corpo}"
    );

    let historico = c.historico(&conversa).await;
    let bruto = historico.to_string();
    assert!(
        !bruto.contains("blob_ref"),
        "o caminho do blob nunca sai na resposta: {bruto}"
    );
    assert!(
        !bruto.contains("blobs/"),
        "nenhum caminho de pasta: {bruto}"
    );
    assert!(
        !bruto.contains("catalogo de servicos"),
        "nenhum conteúdo do arquivo na resposta: {bruto}"
    );

    let turnos = historico["turns"].as_array().expect("turnos");
    assert_eq!(turnos.len(), 1, "um turno: {historico}");
    let arquivos = turnos[0]["files_out"].as_array().expect("files_out");
    assert_eq!(arquivos.len(), 1, "um arquivo enviado: {historico}");
    let enviado = &arquivos[0];
    assert_eq!(enviado["arquivo_id"], c.arquivo.unwrap().to_string());
    assert_eq!(enviado["name"], "catalogo.pdf");
    assert_eq!(enviado["mime"], "application/pdf");
    assert_eq!(enviado["size_bytes"], CONTEUDO.len());
    // O estado é o real da fila: o transporte de console confirma a saída antes do `fim`.
    assert!(
        ["pending", "sent"].contains(&enviado["state"].as_str().unwrap_or("")),
        "{enviado}"
    );
    assert_eq!(enviado["turn_id"], turnos[0]["id"]);
    assert!(enviado["seq"].as_u64().unwrap_or(0) >= 1, "{enviado}");
    assert!(enviado["created_at"].as_i64().unwrap_or(0) > 0, "{enviado}");

    // O download pela rota de conteúdo fecha o ciclo com os mesmos bytes.
    let url = format!(
        "{}/admin/v1/orgs/{}/agents/{}/files/{}/content",
        c.base,
        c.org,
        c.agente,
        enviado["arquivo_id"].as_str().expect("arquivo_id")
    );
    let resposta = c
        .cliente
        .get(&url)
        .bearer_auth(TOKEN_GLOBAL)
        .send()
        .await
        .expect("download");
    assert_eq!(resposta.status(), StatusCode::OK);
    assert_eq!(
        resposta
            .headers()
            .get("x-content-sha256")
            .and_then(|v| v.to_str().ok()),
        enviado["sha256"].as_str(),
        "o sha256 da resposta é o mesmo do histórico"
    );
    let baixado = resposta.bytes().await.expect("bytes").to_vec();
    assert_eq!(baixado, CONTEUDO, "bytes idênticos");
    assert_eq!(
        poph_files::blob::sha256_hex(&baixado),
        enviado["sha256"].as_str().unwrap_or_default()
    );

    // Token de outra org não lê o histórico desta.
    let outra = c
        .store
        .orgs()
        .create(NewOrg {
            name: "Outra".to_owned(),
            slug: "outra-org-hr46d".to_owned(),
            config_json: "{}".to_owned(),
        })
        .expect("org")
        .id;
    let token = c
        .cliente
        .post(format!("{}/admin/v1/orgs/{outra}/tokens", c.base))
        .bearer_auth(TOKEN_GLOBAL)
        .json(&json!({"label": "outra"}))
        .send()
        .await
        .expect("token da outra org")
        .json::<Value>()
        .await
        .expect("json do token")["value"]
        .as_str()
        .expect("valor do token")
        .to_owned();
    let alheio = c
        .cliente
        .get(format!("{}{}/{conversa}", c.base, c.raiz()))
        .bearer_auth(&token)
        .send()
        .await
        .expect("histórico alheio");
    assert_eq!(alheio.status(), StatusCode::FORBIDDEN);

    // Conversa que não mandou arquivo: a lista do turno vem vazia (não nula).
    let sem_arquivo = c.criar_conversa().await;
    let resposta = c.enviar(&sem_arquivo, Some("só texto"), &[]).await;
    assert_eq!(resposta.status(), StatusCode::OK);
    let _ = resposta.text().await.expect("SSE");
    let historico = c.historico(&sem_arquivo).await;
    let turnos = historico["turns"].as_array().expect("turnos");
    assert_eq!(turnos.len(), 1);
    assert_eq!(
        turnos[0]["files_out"].as_array().map(Vec::len),
        Some(0),
        "lista vazia: {historico}"
    );
}

/// Registro das chamadas de ferramenta (HR-47b): o histórico separa executada de recusada e traz
/// argumentos/resultado resumidos, mantendo `tools` como lista de nomes.
#[tokio::test]
async fn historico_traz_o_registro_das_chamadas_de_ferramenta() {
    const CONTEUDO: &[u8] = b"%PDF-1.4\ncatalogo de servicos HR-47b\n";
    let c = Cenario::com_biblioteca("registro-chamadas", CONTEUDO, roteiro_envia_arquivo).await;
    let conversa = c.criar_conversa().await;

    let resposta = c.enviar(&conversa, Some("manda o catálogo"), &[]).await;
    assert_eq!(resposta.status(), StatusCode::OK);
    let _ = resposta.text().await.expect("SSE");

    let historico = c.historico(&conversa).await;
    let turnos = historico["turns"].as_array().expect("turnos");
    assert_eq!(turnos.len(), 1, "um turno: {historico}");
    let turno = &turnos[0];

    // `tools` continua sendo a lista de nomes, no formato antigo.
    assert_eq!(
        turno["tools"],
        json!([poph_agent::NOME_ENVIAR_ARQUIVO]),
        "{historico}"
    );

    let chamadas = turno["tool_calls"].as_array().expect("tool_calls");
    assert_eq!(chamadas.len(), 1, "uma chamada: {historico}");
    let chamada = &chamadas[0];
    assert_eq!(chamada["nome"], poph_agent::NOME_ENVIAR_ARQUIVO);
    assert_eq!(chamada["estado"], "executada");
    assert!(chamada["motivo"].is_null());
    assert_eq!(chamada["rodada"], 0);
    assert_eq!(chamada["ordem"], 1);
    assert!(
        chamada["argumentos"]
            .as_str()
            .expect("argumentos")
            .contains(&c.arquivo.expect("arquivo").to_string()),
        "o argumento traz o id que o modelo enviou: {chamada}"
    );
    assert!(
        chamada["resultado"]
            .as_str()
            .expect("resultado")
            .contains("Arquivo enfileirado"),
        "o resultado é o texto devolvido ao modelo: {chamada}"
    );

    // Turno sem chamada de ferramenta: lista vazia, nunca nula.
    let sem_ferramenta = c.criar_conversa().await;
    let resposta = c.enviar(&sem_ferramenta, Some("só texto"), &[]).await;
    assert_eq!(resposta.status(), StatusCode::OK);
    let _ = resposta.text().await.expect("SSE");
    let historico = c.historico(&sem_ferramenta).await;
    let turnos = historico["turns"].as_array().expect("turnos");
    assert_eq!(
        turnos[0]["tool_calls"].as_array().map(Vec::len),
        Some(0),
        "lista vazia: {historico}"
    );
}
