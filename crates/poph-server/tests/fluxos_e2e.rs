use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use poph_agent::config_quente::{ConfigQuente, OpcoesConfigQuente};
use poph_agent::limites::{Limitador, OpcoesLimites};
use poph_agent::metricas::{MetricasG9, MetricasMemoria, MetricasTurno};
use poph_agent::turno::{
    Entrada, FuturoPausa, Motor, ObservadorNulo, OpcoesTurno, Pausa, ResumoTurno,
};
use poph_core::llm::{ChamadaFerramenta, EventoLlm, MotivoFim, ProvedorFake, Roteiro, Uso};
use poph_fluxos::{Limites, compilar};
use poph_jev::{JevFalso, MotorJev};
use poph_store::{
    Direcao, EstadoTurno, NovaConversa, NovaMensagem, NovaOrg, NovoAgente, NovoFluxo, Papel,
    PublicacaoDeRevisao, StatusConversa, Store,
};
use poph_types::{AgentId, ConversationId, FluxosConfig, GatesJev, MessageId, OrgId, StatusFicha};
use serde_json::{Value, json};

struct SemEspera;

impl Pausa for SemEspera {
    fn esperar(&self, _duracao: Duration) -> FuturoPausa {
        Box::pin(async {})
    }
}

/// As três ferramentas do fluxo, como o agente as habilita (D-04).
const TODAS: [&str; 3] = [
    poph_agent::turno::NOME_EXECUTAR_ACAO,
    poph_agent::turno::NOME_ABANDONAR,
    poph_agent::turno::NOME_FERRAMENTA_INICIAR,
];

/// Banco, org, agente e conversa de verdade numa pasta temporária.
struct Cenario {
    pasta: PathBuf,
    store: Arc<Store>,
    org: OrgId,
    agente: AgentId,
}

impl Cenario {
    fn novo(rotulo: &str) -> Self {
        let pasta = std::env::temp_dir().join(format!(
            "poph-fx24-{rotulo}-{}-{}",
            std::process::id(),
            AgentId::generate()
        ));
        std::fs::create_dir_all(&pasta).expect("pasta do teste");
        let store = Arc::new(
            Store::abrir_em(&pasta.join("harness.db"), &pasta.join("rag.db"))
                .expect("abre o banco"),
        );
        let org = store
            .orgaos()
            .criar(NovaOrg {
                nome: format!("Org {rotulo}"),
                slug: format!("org-{rotulo}-{}", AgentId::generate()),
                config_json: "{}".to_owned(),
            })
            .expect("cria a org")
            .id;
        let agente = store
            .agentes()
            .criar(
                org,
                NovoAgente {
                    nome: format!("Bia {rotulo}"),
                    slug: format!("bia-{rotulo}"),
                    janela_agrupamento_ms: 0,
                    ..NovoAgente::default()
                },
            )
            .expect("cria o agente")
            .id;
        let cenario = Self {
            pasta,
            store,
            org,
            agente,
        };
        cenario.habilitar(&TODAS);
        cenario
    }

    /// Grava as instruções com as ferramentas habilitadas informadas (D-04).
    fn habilitar(&self, ferramentas: &[&str]) {
        let estruturado = json!({
            "tools": ferramentas
                .iter()
                .map(|nome| json!({ "name": nome, "enabled": true }))
                .collect::<Vec<_>>()
        })
        .to_string();
        let atual = self.store.instrucoes().versao(self.org, self.agente);
        let esperada = match atual {
            Ok(Some(versao)) => versao,
            _ => 0,
        };
        self.store
            .instrucoes()
            .salvar(
                self.org,
                self.agente,
                esperada,
                &estruturado,
                "# Bia\n\nVocê é a atendente da FixProLar.",
                true,
            )
            .expect("grava as instruções");
    }

    /// Cria um fluxo com o script dado, publica a revisão e (opcionalmente) libera ao agente.
    fn criar_fluxo(
        &self,
        slug: &str,
        nome: &str,
        script: &str,
        liberar: bool,
    ) -> poph_types::FluxoId {
        let fluxo = self
            .store
            .fluxos()
            .criar(
                self.org,
                NovoFluxo {
                    slug: slug.to_owned(),
                    nome: nome.to_owned(),
                    descricao: String::new(),
                    rascunho_fonte: script.to_owned(),
                },
            )
            .expect("cria o fluxo");
        let compilado = compilar(script, &Limites::default());
        let ir = compilado.ir().expect("o script do teste compila");
        self.store
            .revisoes()
            .publicar_revisao(
                self.org,
                fluxo.id,
                PublicacaoDeRevisao {
                    esperada: fluxo.versao,
                    fonte: script.to_owned(),
                    definicao_json: serde_json::to_string(ir).expect("IR em JSON"),
                    schema_versao: Some(ir.schema_versao),
                    resolucoes_json: "{}".to_owned(),
                    ator_tipo: "system".to_owned(),
                    ator_id: None,
                },
            )
            .expect("publica a revisão");
        if liberar {
            self.store
                .fluxos()
                .liberar(self.org, self.agente, fluxo.id)
                .expect("libera o fluxo");
        }
        fluxo.id
    }

    fn conversa(&self) -> ConversationId {
        self.store
            .conversas()
            .criar(
                self.org,
                NovaConversa {
                    agent_id: self.agente,
                    integration_id: None,
                    canal: "cli".to_owned(),
                    peer_externo: Some("cliente-ficticio".to_owned()),
                    departamento_alvo: None,
                },
            )
            .expect("abre a conversa")
            .id
    }

    fn falar(&self, conversa: ConversationId, texto: &str) -> MessageId {
        self.store
            .mensagens()
            .gravar(
                self.org,
                NovaMensagem {
                    conversation_id: conversa,
                    direcao: Direcao::Entrada,
                    papel: Papel::Usuario,
                    texto: texto.to_owned(),
                    anexos_json: "[]".to_owned(),
                    id_externo: None,
                    turn_id: None,
                    origem: poph_store::Origem::Agente,
                    autor_nome: None,
                },
            )
            .expect("grava a mensagem")
            .id
    }

    fn motor(
        &self,
        roteiros: Vec<Roteiro>,
        jev: Option<Arc<MotorJev>>,
        fluxos: Option<FluxosConfig>,
    ) -> (Motor, Arc<ProvedorFake>) {
        let config = ConfigQuente::nova(
            Arc::clone(&self.store),
            OpcoesConfigQuente::default(),
            MetricasMemoria::proprio(),
        );
        let limitador = Arc::new(Limitador::novo(
            OpcoesLimites::default(),
            MetricasMemoria::proprio(),
        ));
        let provedor = Arc::new(ProvedorFake::com_roteiros(roteiros));
        let mut motor = Motor::novo(
            Arc::clone(&self.store),
            config,
            limitador,
            provedor.clone(),
            OpcoesTurno::default(),
            MetricasTurno::proprio(),
        )
        .com_pausa(Arc::new(SemEspera))
        .com_resumo(poph_agent::resumo::OpcoesResumo {
            ativo: false,
            ..poph_agent::resumo::OpcoesResumo::default()
        })
        .com_metricas_g9(MetricasG9::proprias());
        // FX-24: as ferramentas do fluxo entram no registro do turno, como na montagem real.
        let registro = poph_agent::turno::registrar_ferramentas_do_fluxo(
            poph_agent::turno::RegistroPadrao::novo(),
            Arc::clone(&self.store),
            &fluxos.clone().unwrap_or_default(),
        );
        let registro =
            poph_agent::turno::registrar_ferramenta_encerramento(registro, Arc::clone(&self.store));
        motor = motor.com_ferramentas(Arc::new(registro));
        if let Some(ajustes) = fluxos {
            motor = motor.com_fluxos(&ajustes);
        }
        if let Some(jev) = jev {
            motor = motor.com_jev(jev);
        }
        (motor, provedor)
    }

    async fn turno(
        &self,
        motor: &Motor,
        conversa: ConversationId,
        gatilho: MessageId,
    ) -> ResumoTurno {
        motor
            .executar(
                Entrada::nova(self.org, conversa, gatilho),
                &mut ObservadorNulo,
            )
            .await
            .expect("o turno roda")
    }

    fn ficha(&self, conversa: ConversationId) -> poph_store::Ficha {
        self.store
            .fichas()
            .por_conversa(self.org, conversa)
            .expect("lê a Ficha")
            .expect("a conversa tem Ficha")
    }

    fn execucao(&self, conversa: ConversationId) -> poph_store::Execucao {
        self.store
            .execucoes()
            .da_ficha(self.org, self.ficha(conversa).id, 20)
            .expect("lê as execuções")
            .into_iter()
            .next()
            .expect("a Ficha tem execução")
    }
}

impl Drop for Cenario {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.pasta);
    }
}

// ───────────────────────────────────────────────────────────────────────── utilidades

fn texto(conteudo: &str) -> Roteiro {
    Roteiro::Eventos(vec![
        EventoLlm::Texto(conteudo.to_owned()),
        EventoLlm::Uso(Uso {
            entrada: 10,
            saida: 5,
        }),
        EventoLlm::Fim(MotivoFim::Concluido),
    ])
}

/// Roteiro do modelo que pede uma única ferramenta.
fn chamada(nome: &str, argumentos: Value) -> Roteiro {
    Roteiro::Eventos(vec![
        EventoLlm::Ferramenta(ChamadaFerramenta {
            id: "chamada-1".to_owned(),
            nome: nome.to_owned(),
            argumentos,
        }),
        EventoLlm::Fim(MotivoFim::Concluido),
    ])
}

fn gates_com_g9() -> GatesJev {
    GatesJev {
        g9: true,
        ..GatesJev::default()
    }
}

fn motor_jev(jev: Arc<JevFalso>) -> Arc<MotorJev> {
    Arc::new(MotorJev::novo(jev, gates_com_g9(), "jev-latest"))
}

fn escolhas(itens: &[(&str, &str)]) -> std::collections::BTreeMap<String, String> {
    itens
        .iter()
        .map(|(id, escolha)| ((*id).to_owned(), (*escolha).to_owned()))
        .collect()
}

const VISITA: &str = include_str!("../../../tests/fluxos/exemplos/agendar_visita.star");
const TRIAGEM: &str = include_str!("../../../tests/fluxos/exemplos/triagem_suporte.star");
const ENTREGA: &str = include_str!("../../../tests/fluxos/exemplos/confirmar_entrega.star");
const TOKEN: &str = "pophg_fx35_ficticio";

async fn api(c: &Cenario) -> String {
    use argon2::password_hash::{PasswordHasher, SaltString};
    let sal = SaltString::generate(&mut rand_core::OsRng);
    let hash = argon2::Argon2::default()
        .hash_password(TOKEN.as_bytes(), &sal)
        .unwrap()
        .to_string();
    let mut config = poph_types::HarnessConfig::default();
    config.admin.tokens_globais = vec![poph_types::config::TokenGlobalConfig {
        label: "teste".into(),
        hash,
    }];
    let estado = poph_server::Estado::novo(Arc::clone(&c.store), config).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endereco = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = poph_server::servir_em(listener, estado, std::future::pending::<()>()).await;
    });
    format!("http://{endereco}")
}

async fn get_api(base: &str, c: &Cenario, caminho: &str) -> Value {
    let url = format!("{base}/admin/v1/orgs/{}/fichas/{}", c.org, caminho);
    let resposta = reqwest::Client::new()
        .get(url)
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap();
    assert!(resposta.status().is_success(), "{}", resposta.status());
    resposta.json().await.unwrap()
}

async fn executar(
    c: &Cenario,
    conversa: ConversationId,
    slug: &str,
    roteiros: Vec<Roteiro>,
) -> Arc<ProvedorFake> {
    let jev = Arc::new(JevFalso::com_escolhas(escolhas(&[(
        "g9_iniciar_fluxo",
        slug,
    )])));
    let (motor, provedor) = c.motor(
        roteiros,
        Some(motor_jev(jev)),
        Some(FluxosConfig::default()),
    );
    let mensagem = c.falar(conversa, "Pedido fictício para teste");
    assert_eq!(
        c.turno(&motor, conversa, mensagem).await.estado,
        EstadoTurno::Concluido
    );
    provedor
}

#[tokio::test]
async fn exemplo_visita_com_jev_campos_e_historico_http() {
    let c = Cenario::novo("fx35-visita");
    c.criar_fluxo("agendar_visita", "Agendar visita", VISITA, true);
    let conversa = c.conversa();
    let provedor = executar(
        &c,
        conversa,
        "agendar_visita",
        vec![
            chamada(
                "fluxo_executar_acao",
                json!({"acao":"confirmar","campos":{"endereco":"Rua Exemplo 1","periodo":"tarde"}}),
            ),
            chamada("fluxo_executar_acao", json!({"acao":"ok","campos":{}})),
            texto("Visita combinada."),
        ],
    )
    .await;
    let pedidos = provedor.recebidos();
    assert!(format!("{:?}", pedidos[0].mensagens).contains("Coletar dados"));
    let base = api(&c).await;
    let ficha = c.ficha(conversa);
    let corpo = get_api(&base, &c, &ficha.id.to_string()).await;
    assert_eq!(corpo["status"], "open");
    let run = &corpo["execucoes"][0];
    assert_eq!(run["status"], "concluida", "{corpo}");
    assert_eq!(run["resultado"], "Visita agendada");
    assert_eq!(run["etapas"][0]["no_id"], "coleta");
    assert_eq!(run["etapas"][0]["dados"]["endereco"], "Rua Exemplo 1");
    assert_eq!(run["etapas"][0]["dados"]["periodo"], "tarde");
    assert_eq!(run["etapas"][0]["atores_que_agiram"], json!(["agente"]));
    assert_eq!(run["etapas"][1]["no_id"], "confirmacao");
    let eventos = get_api(&base, &c, &format!("{}/events", ficha.id)).await;
    assert!(eventos["items"].as_array().unwrap().len() >= 3, "{eventos}");
}

#[tokio::test]
async fn exemplo_triagem_exercita_ambos_os_ramos() {
    for (rotulo, resumo, afeta, esperado) in [
        ("grave", "sistema nao abre", "sim", "urgente"),
        ("leve", "duvida simples", "sim", "normal"),
        ("sem-impacto", "sistema nao abre", "nao", "normal"),
    ] {
        let c = Cenario::novo(rotulo);
        c.criar_fluxo("triagem_suporte", "Triagem de suporte", TRIAGEM, true);
        let conversa = c.conversa();
        executar(
            &c,
            conversa,
            "triagem_suporte",
            vec![
                chamada(
                    "fluxo_executar_acao",
                    json!({"acao":"registrado","campos":{"resumo":resumo,"afeta_agora":afeta}}),
                ),
                texto("Entendido."),
            ],
        )
        .await;
        assert_eq!(c.execucao(conversa).no_atual, esperado);
        let base = api(&c).await;
        let ficha = c.ficha(conversa);
        let eventos = get_api(&base, &c, &format!("{}/events", ficha.id)).await;
        assert!(
            eventos.to_string().contains("decisao_resolvida"),
            "{eventos}"
        );
    }
}

async fn evolution_falsa() -> (String, Arc<std::sync::Mutex<Vec<Value>>>) {
    use axum::{Json, Router, extract::State, routing::post};
    let envios = Arc::new(std::sync::Mutex::new(Vec::new()));
    async fn enviar(
        State(envios): State<Arc<std::sync::Mutex<Vec<Value>>>>,
        Json(corpo): Json<Value>,
    ) -> Json<Value> {
        envios.lock().unwrap().push(corpo);
        Json(json!({"key":{"id":"ficticio"}}))
    }
    let router = Router::new()
        .route("/message/sendText/{instancia}", post(enviar))
        .with_state(Arc::clone(&envios));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endereco = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    (format!("http://{endereco}"), envios)
}

#[tokio::test]
async fn exemplo_entrega_notificacao_unica_e_transferencia_com_ficha_aberta() {
    use poph_evolution::{ClienteEvolution, ConfigEvolution, TETO_MIDIA};
    use poph_server::{
        evolution::CanalEvolution,
        fluxos_efeitos::{EfeitosFluxo, OpcoesEfeitosFluxo},
        saida_whatsapp::{OpcoesSaidaWhatsApp, SaidaWhatsApp},
    };
    use poph_types::{IntegrationId, Segredo};
    let c = Cenario::novo("efeitos-fx35");
    c.criar_fluxo("confirmar_entrega", "Confirmar entrega", ENTREGA, true);
    c.store
        .departamentos()
        .criar(
            c.org,
            poph_store::NovoDepartamento {
                nome: "Logística".into(),
                descricao: String::new(),
            },
        )
        .unwrap();
    let integracao: IntegrationId = c
        .store
        .integracao_criar(c.org, c.agente, "whatsapp".into(), "active".into())
        .unwrap()
        .id
        .parse()
        .unwrap();
    let (url, envios) = evolution_falsa().await;
    let canal = CanalEvolution {
        org: c.org,
        instancia: "teste".into(),
        integracao,
        segredo: None,
        cliente: Some(
            ClienteEvolution::novo(
                ConfigEvolution::novo(url, "teste", Segredo::new("ficticia".into())).unwrap(),
            )
            .unwrap(),
        ),
        teto_midia: TETO_MIDIA,
    };
    let canais = vec![canal];
    let efeitos = EfeitosFluxo::novo(
        Arc::clone(&c.store),
        &canais,
        c.pasta.join("blobs"),
        None,
        OpcoesEfeitosFluxo::default(),
    );
    let saida = SaidaWhatsApp::novo(
        Arc::clone(&c.store),
        &canais,
        c.pasta.join("blobs"),
        OpcoesSaidaWhatsApp::default(),
    );
    let conversa = |peer: &str| {
        c.store
            .conversas()
            .criar(
                c.org,
                NovaConversa {
                    agent_id: c.agente,
                    integration_id: Some(integracao),
                    canal: "whatsapp".into(),
                    peer_externo: Some(peer.into()),
                    departamento_alvo: None,
                },
            )
            .unwrap()
            .id
    };
    let primeira = conversa("5511999990001");
    executar(
        &c,
        primeira,
        "confirmar_entrega",
        vec![
            chamada(
                "fluxo_executar_acao",
                json!({"acao":"localizado","campos":{"pedido":"1234"}}),
            ),
            texto("Vou verificar."),
        ],
    )
    .await;
    efeitos.ciclo_da_org(c.org).await.unwrap();
    saida.ciclo().await.unwrap();
    executar(
        &c,
        primeira,
        "confirmar_entrega",
        vec![
            chamada(
                "fluxo_executar_acao",
                json!({"acao":"confirmada","campos":{"recebido":"sim"}}),
            ),
            texto("Concluído."),
        ],
    )
    .await;
    efeitos.ciclo_da_org(c.org).await.unwrap();
    saida.ciclo().await.unwrap();
    // Um ciclo a mais não reenvia o que já saiu (a notificação é uma só).
    efeitos.ciclo_da_org(c.org).await.unwrap();
    saida.ciclo().await.unwrap();
    let enviados = envios.lock().unwrap().clone();
    assert_eq!(
        enviados
            .iter()
            .filter(|v| v["number"] == "5521999990000")
            .count(),
        1,
        "{enviados:?}"
    );
    assert!(
        enviados
            .iter()
            .any(|v| v["text"].as_str().unwrap_or("").contains("pedido 1234")),
        "{enviados:?}"
    );
    let segunda = conversa("5511999990002");
    executar(
        &c,
        segunda,
        "confirmar_entrega",
        vec![
            chamada(
                "fluxo_executar_acao",
                json!({"acao":"localizado","campos":{"pedido":"5678"}}),
            ),
            chamada(
                "fluxo_executar_acao",
                json!({"acao":"nao_recebida","campos":{"recebido":"nao"}}),
            ),
            chamada(
                "fluxo_executar_acao",
                json!({"acao":"registrado","campos":{}}),
            ),
            texto("Encaminhado."),
        ],
    )
    .await;
    efeitos.ciclo_da_org(c.org).await.unwrap();
    assert_eq!(
        c.store.conversas().obter(c.org, segunda).unwrap().status,
        StatusConversa::Transferida
    );
    assert_eq!(c.ficha(segunda).status, StatusFicha::Aberta);
    let base = api(&c).await;
    let eventos = get_api(&base, &c, &format!("{}/events", c.ficha(segunda).id)).await;
    assert!(
        eventos.to_string().contains("atendimento_transferido"),
        "{eventos}"
    );
}

#[tokio::test]
async fn revisao_antiga_acao_humana_e_dois_fluxos_na_mesma_ficha() {
    let c = Cenario::novo("fx35-sequencia");
    let visita_humana = VISITA.replace(
        "nome = \"Confirmar visita\",",
        "nome = \"Confirmar visita\",\n    atores = [\"agente\", \"atendente\"],",
    );
    let fluxo = c.criar_fluxo("agendar_visita", "Agendar visita", &visita_humana, true);
    c.criar_fluxo("triagem_suporte", "Triagem de suporte", TRIAGEM, true);
    let conversa = c.conversa();
    executar(
        &c,
        conversa,
        "agendar_visita",
        vec![
            chamada(
                "fluxo_executar_acao",
                json!({"acao":"confirmar","campos":{"endereco":"Rua Teste","periodo":"manhã"}}),
            ),
            texto("Confirme."),
        ],
    )
    .await;
    let anterior = c.execucao(conversa);
    let nova_fonte = visita_humana.replace("Visita agendada", "Resultado da versão nova");
    let compilado = compilar(&nova_fonte, &Limites::default());
    let ir = compilado.ir().unwrap();
    let versao = c.store.fluxos().obter(c.org, fluxo).unwrap().versao;
    let nova = c
        .store
        .revisoes()
        .publicar_revisao(
            c.org,
            fluxo,
            PublicacaoDeRevisao {
                esperada: versao,
                fonte: nova_fonte,
                definicao_json: ir.para_json(),
                schema_versao: Some(ir.schema_versao),
                resolucoes_json: "{}".into(),
                ator_tipo: "system".into(),
                ator_id: None,
            },
        )
        .unwrap();
    assert_ne!(nova.id, anterior.revisao_id);
    let base = api(&c).await;
    let ficha = c.ficha(conversa);
    let url = format!(
        "{base}/admin/v1/orgs/{}/fichas/{}/flow-runs/{}/actions",
        c.org, ficha.id, anterior.id
    );
    let resposta = reqwest::Client::new()
        .post(url)
        .bearer_auth(TOKEN)
        .header("If-Match", anterior.versao.to_string())
        .json(&json!({"acao":"ok", "campos":{}, "chave_idempotencia":"humano-fx35"}))
        .send()
        .await
        .unwrap();
    let status = resposta.status();
    let corpo: Value = resposta.json().await.unwrap();
    assert_eq!(status, reqwest::StatusCode::OK, "{corpo}");
    assert_eq!(corpo["resultado"], "Visita agendada");
    executar(
        &c,
        conversa,
        "triagem_suporte",
        vec![
            chamada(
                "fluxo_executar_acao",
                json!({"acao":"registrado","campos":{"resumo":"erro simples","afeta_agora":"nao"}}),
            ),
            chamada(
                "fluxo_executar_acao",
                json!({"acao":"encaminhado","campos":{}}),
            ),
            texto("Feito."),
        ],
    )
    .await;
    let historico = get_api(&base, &c, &ficha.id.to_string()).await;
    let runs = historico["execucoes"].as_array().unwrap();
    assert_eq!(runs.len(), 2, "{historico}");
    let visita = runs
        .iter()
        .find(|r| r["id"] == anterior.id.to_string())
        .unwrap();
    assert_eq!(visita["revisao_id"], anterior.revisao_id.to_string());
    assert_eq!(visita["resultado"], "Visita agendada");
    assert_eq!(
        visita["etapas"][1]["atores_que_agiram"],
        json!(["atendente"])
    );
    assert!(runs.iter().any(|r| r["resultado"] == "Normal"));
    let eventos = get_api(&base, &c, &format!("{}/events", ficha.id)).await;
    assert!(eventos.to_string().contains("atendente"), "{eventos}");
}

#[tokio::test]
async fn jev_falha_reserva_inicia_permitido_e_recusa_nao_liberado() {
    let c = Cenario::novo("fx35-reserva");
    let permitido = c.criar_fluxo("agendar_visita", "Agendar visita", VISITA, true);
    c.criar_fluxo("triagem_suporte", "Triagem de suporte", TRIAGEM, false);
    let conversa = c.conversa();
    let jev = Arc::new(JevFalso::falhando(poph_jev::ErroJev::Rede(
        "indisponível no teste".into(),
    )));
    let (motor, provedor) = c.motor(
        vec![
            chamada("fluxo_iniciar", json!({"fluxo_id":"triagem_suporte"})),
            chamada("fluxo_iniciar", json!({"fluxo_id":"agendar_visita"})),
            texto("Certo."),
        ],
        Some(motor_jev(jev)),
        Some(FluxosConfig::default()),
    );
    let mensagem = c.falar(conversa, "Quero agendar");
    c.turno(&motor, conversa, mensagem).await;
    let pedidos = provedor.recebidos();
    assert!(
        pedidos[0]
            .ferramentas
            .iter()
            .any(|f| f.nome == "fluxo_iniciar")
    );
    assert!(!format!("{:?}", pedidos[0].mensagens).contains("triagem_suporte"));
    assert!(format!("{:?}", pedidos[1].mensagens).contains("não está entre os liberados"));
    assert_eq!(c.execucao(conversa).no_atual, "coleta");
    let runs = c
        .store
        .execucoes()
        .da_ficha(c.org, c.ficha(conversa).id, 20)
        .unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].fluxo_id, permitido);
}

#[tokio::test]
async fn encerramento_do_agente_e_proxima_mensagem_abrem_nova_ficha() {
    let c = Cenario::novo("fx35-encerrar");
    c.habilitar(&["atendimento_encerrar"]);
    let integracao = c
        .store
        .integracao_criar(c.org, c.agente, "whatsapp".into(), "active".into())
        .unwrap()
        .id
        .parse()
        .unwrap();
    let entrada = |id: &str| poph_store::NovaEntradaCanal {
        integration_id: integracao,
        agent_id: c.agente,
        canal: "whatsapp".into(),
        id_externo: id.into(),
        peer: "5511999990001".into(),
        evento_json: "{}".into(),
        texto: "Olá".into(),
        anexos: vec![],
        nome_cliente: None,
    };
    let mensagem = c
        .store
        .caixa_entrada()
        .receber_do_canal(c.org, entrada("fx35-1"))
        .unwrap()
        .mensagem;
    let conversa = mensagem.conversation_id;
    let ficha = c.ficha(conversa);
    let (motor, _) = c.motor(
        vec![
            chamada("atendimento_encerrar", json!({"motivo":"resolvido"})),
            texto("Até logo."),
        ],
        None,
        Some(FluxosConfig::default()),
    );
    c.turno(&motor, conversa, mensagem.id).await;
    assert_eq!(c.ficha(conversa).status, StatusFicha::Fechada);
    let proxima = c
        .store
        .caixa_entrada()
        .receber_do_canal(c.org, entrada("fx35-2"))
        .unwrap()
        .mensagem;
    assert_ne!(proxima.conversation_id, conversa);
    assert_ne!(c.ficha(proxima.conversation_id).id, ficha.id);
    let base = api(&c).await;
    assert_eq!(
        get_api(&base, &c, &ficha.id.to_string()).await["status"],
        "closed"
    );
}
