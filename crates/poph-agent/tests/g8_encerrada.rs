//! Efeito do G8 no turno (HR-36): conversa encerrada pelo cliente.
//!
//! O que se prova aqui, com provedor de modelo falso, busca falsa e Jev falso (nenhum teste toca a
//! rede):
//! 1. com o G8 `encerrada` e confiança no limiar, o turno termina **sem chamar o modelo de texto**,
//!    grava a despedida (a padrão, a configurada no agente, ou nenhuma se o agente pediu vazio) e
//!    conta na métrica;
//! 2. a mensagem seguinte do cliente entra num turno normal e volta a chamar o modelo — nada fica
//!    preso no encerramento;
//! 3. "obrigado, mas ainda tenho uma dúvida" não fecha;
//! 4. abstenção, confiança baixa, resposta fora do contrato, resposta ausente e Jev fora do ar
//!    deixam o turno como era antes da HR-36;
//! 5. gate desligado e primeira mensagem da conversa não perguntam nada e não fecham;
//! 6. se o G1 transfere e o G8 encerra na mesma mensagem, a transferência vence;
//! 7. com o G8 encerrando, a busca em andamento não é aguardada.
//!
//! Os mesmos cenários estão no conjunto medido com o Jev real (`tests/jev/g8.jsonl`).

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use poph_agent::base_conhecimento::{
    BuscadorBase, ConsultarBase, FuturoBusca, ParametrosBusca, TrechoRecuperado,
};
use poph_agent::config_quente::{ConfigQuente, OpcoesConfigQuente};
use poph_agent::limites::{Limitador, OpcoesLimites};
use poph_agent::metricas::{MetricasMemoria, MetricasTurno};
use poph_agent::produtos::{
    BuscadorProdutos, BuscarProduto, FuturoProdutos, ParametrosProdutos, ProdutoEncontrado,
};
use poph_agent::turno::{
    Entrada, FuturoPausa, Motor, ObservadorNulo, OpcoesTurno, Pausa, ResumoTurno,
};
use poph_core::llm::{EventoLlm, MotivoFim, ProvedorFake, Roteiro, Uso};
use poph_jev::{
    ErroJev, G8_DESPEDIDA_PADRAO, JevFalso, LoteResposta, MotorJev, Resposta as RespostaJev,
};
use poph_store::{
    Direcao, EstadoTurno, MudancaAgente, NovaConversa, NovaMensagem, NovoAgente, Pagina, Papel,
    StatusConversa, Store,
};
use poph_types::{
    AgentId, ConversationId, JevGates, MessageId, OrgId, StructuredInstructions, ToolConfig,
};

struct SemEspera;

impl Pausa for SemEspera {
    fn esperar(&self, _duracao: Duration) -> FuturoPausa {
        Box::pin(async {})
    }
}

/// Busca falsa: conta as buscas e demora o que for pedido.
struct BuscaFalsa {
    buscas: Arc<AtomicU64>,
    demora: Duration,
}

impl BuscadorBase for BuscaFalsa {
    fn buscar(
        &self,
        _org: OrgId,
        _agente: AgentId,
        _consulta: String,
        _parametros: ParametrosBusca,
    ) -> FuturoBusca<'_> {
        self.buscas.fetch_add(1, Ordering::SeqCst);
        let demora = self.demora;
        Box::pin(async move {
            if !demora.is_zero() {
                tokio::time::sleep(demora).await;
            }
            let trecho = TrechoRecuperado {
                item: poph_types::KnowledgeItemId::generate(),
                titulo: "Prazo de entrega".to_owned(),
                grupo: "Políticas da loja".to_owned(),
                texto: "Prazo de entrega: até 8 dias úteis.".to_owned(),
                similaridade: 0.82,
                relevancia: 0.5,
            };
            Ok(vec![trecho])
        })
    }
}

struct CatalogoVazio;

impl BuscadorProdutos for CatalogoVazio {
    fn buscar(
        &self,
        _org: OrgId,
        _agente: AgentId,
        _termo: String,
        _categoria: Option<String>,
        _parametros: ParametrosProdutos,
    ) -> FuturoProdutos<'_> {
        let vazio: Vec<ProdutoEncontrado> = Vec::new();
        Box::pin(async move { Ok(vazio) })
    }
}

struct Cenario {
    pasta: PathBuf,
    store: Arc<Store>,
    org: OrgId,
    conversa: ConversationId,
    busca: Arc<dyn BuscadorBase>,
    metricas: MetricasTurno,
}

impl Cenario {
    fn novo(rotulo: &str) -> Self {
        Self::com_demora(rotulo, Duration::ZERO, "")
    }

    /// `jev_json` do agente (despedida, situações). O G1 fica habilitado no agente.
    fn com_demora(rotulo: &str, demora: Duration, jev_json: &str) -> Self {
        let pasta = std::env::temp_dir().join(format!(
            "poph-hr36-{rotulo}-{}-{}",
            std::process::id(),
            AgentId::generate()
        ));
        let _ = std::fs::remove_dir_all(&pasta);
        std::fs::create_dir_all(&pasta).expect("pasta do teste");
        let store = Arc::new(
            Store::abrir_em(&pasta.join("harness.db"), &pasta.join("rag.db"))
                .expect("abre o banco"),
        );
        let org = store
            .orgaos()
            .criar(poph_store::NovaOrg {
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
            .expect("cria o agente");
        let estruturado = StructuredInstructions {
            tools: vec![ToolConfig {
                name: "consultar_base".to_owned(),
                enabled: true,
                when_to_use: String::new(),
            }],
            ..StructuredInstructions::default()
        };
        store
            .instrucoes()
            .salvar(
                org,
                agente.id,
                0,
                &estruturado.to_json(),
                "# Bia\n\nVocê é a atendente da loja.",
                true,
            )
            .expect("grava as instruções");
        store
            .agentes()
            .atualizar(
                org,
                agente.id,
                agente.versao,
                MudancaAgente {
                    jev_json: Some(jev_json.to_owned()),
                    handoff_ativo: Some(true),
                    ..MudancaAgente::default()
                },
            )
            .expect("grava o jev_json do agente");
        let conversa = store
            .conversas()
            .criar(
                org,
                NovaConversa {
                    agent_id: agente.id,
                    integration_id: None,
                    canal: "cli".to_owned(),
                    peer_externo: None,
                    departamento_alvo: None,
                },
            )
            .expect("abre a conversa")
            .id;
        let busca: Arc<dyn BuscadorBase> = Arc::new(BuscaFalsa {
            buscas: Arc::new(AtomicU64::new(0)),
            demora,
        });
        Self {
            pasta,
            store,
            org,
            conversa,
            busca,
            metricas: MetricasTurno::proprio(),
        }
    }

    /// Mensagem que o próprio agente mandou antes (a conversa já tinha resposta).
    fn resposta_do_agente(&self, texto: &str) {
        self.store
            .mensagens()
            .gravar(
                self.org,
                NovaMensagem {
                    conversation_id: self.conversa,
                    direcao: Direcao::Saida,
                    papel: Papel::Assistente,
                    texto: texto.to_owned(),
                    anexos_json: "[]".to_owned(),
                    id_externo: None,
                    turn_id: None,
                    origem: poph_store::Origem::Agente,
                    autor_nome: None,
                },
            )
            .expect("grava a resposta anterior");
    }

    fn falar(&self, texto: &str) -> MessageId {
        self.store
            .mensagens()
            .gravar(
                self.org,
                NovaMensagem {
                    conversation_id: self.conversa,
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
    ) -> (Motor, Arc<ProvedorFake>) {
        let fake = Arc::new(ProvedorFake::com_roteiros(roteiros));
        let config = ConfigQuente::nova(
            Arc::clone(&self.store),
            OpcoesConfigQuente::default(),
            MetricasMemoria::proprio(),
        );
        let limitador = Arc::new(Limitador::novo(
            OpcoesLimites::default(),
            MetricasMemoria::proprio(),
        ));
        let catalogo: Arc<dyn BuscadorProdutos> = Arc::new(CatalogoVazio);
        let mut motor = Motor::novo(
            Arc::clone(&self.store),
            config,
            limitador,
            fake.clone(),
            OpcoesTurno::default(),
            self.metricas.clone(),
        )
        .com_pausa(Arc::new(SemEspera))
        .com_busca(Arc::clone(&self.busca));
        let registro =
            ConsultarBase::registro(Arc::clone(motor.config_quente()), Arc::clone(&self.busca))
                .com(BuscarProduto::nova(
                    Arc::clone(motor.config_quente()),
                    catalogo,
                ));
        motor = motor.com_ferramentas(Arc::new(registro));
        if let Some(jev) = jev {
            motor = motor.com_jev(jev);
        }
        (motor, fake)
    }

    async fn turno(&self, motor: &Motor, gatilho: MessageId) -> ResumoTurno {
        motor
            .executar(
                Entrada::nova(self.org, self.conversa, gatilho),
                &mut ObservadorNulo,
            )
            .await
            .expect("o turno roda")
    }

    /// Mensagens de saída do agente nesta conversa, da mais antiga para a mais nova.
    fn saidas(&self) -> Vec<String> {
        self.store
            .mensagens()
            .listar(self.org, self.conversa, Pagina::com_limite(200))
            .expect("lista as mensagens")
            .into_iter()
            .filter(|mensagem| mensagem.direcao == Direcao::Saida)
            .map(|mensagem| mensagem.texto)
            .collect()
    }

    /// Itens de saída ainda na fila (o que o transporte entregaria).
    fn fila(&self) -> Vec<String> {
        self.store
            .caixa_saida()
            .listar_pendentes(self.org, 50)
            .expect("lista a fila de saída")
            .into_iter()
            .map(|item| item.conteudo_json)
            .collect()
    }

    fn status(&self) -> StatusConversa {
        self.store
            .conversas()
            .obter(self.org, self.conversa)
            .expect("lê a conversa")
            .status
    }
}

impl Drop for Cenario {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.pasta);
    }
}

fn roteiro_texto(texto: &str) -> Roteiro {
    Roteiro::Eventos(vec![
        EventoLlm::Texto(texto.to_owned()),
        EventoLlm::Uso(Uso {
            entrada: 100,
            saida: 10,
        }),
        EventoLlm::Fim(MotivoFim::Concluido),
    ])
}

fn so_g8() -> JevGates {
    JevGates {
        g8: true,
        ..JevGates::default()
    }
}

/// Jev falso que responde pela **última fala do cliente** da conversa enviada: para cada texto,
/// as respostas de cada pergunta (id, escolha, confiança). Texto fora da tabela não recebe resposta.
/// Resposta do Jev para uma pergunta: (id da pergunta, escolha, confiança).
type RespostaRoteirizada = (&'static str, &'static str, f64);
/// Respostas de um texto de cliente, dono e tradução para o Jev falso.
type RespostasDoTexto = Vec<(String, String, f64)>;

fn jev_por_texto(tabela: &[(&str, &[RespostaRoteirizada])]) -> Arc<JevFalso> {
    let tabela: Vec<(String, RespostasDoTexto)> = tabela
        .iter()
        .map(|(texto, itens)| {
            (
                (*texto).to_owned(),
                itens
                    .iter()
                    .map(|(id, escolha, confianca)| {
                        ((*id).to_owned(), (*escolha).to_owned(), *confianca)
                    })
                    .collect(),
            )
        })
        .collect();
    Arc::new(JevFalso::novo(move |pedido| {
        let ultimo = ultima_fala_do_cliente(&pedido.estado);
        let mut respostas = BTreeMap::new();
        if let Some((_, itens)) = tabela.iter().find(|(texto, _)| *texto == ultimo) {
            for (id, escolha, confianca) in itens {
                respostas.insert(
                    id.clone(),
                    RespostaJev {
                        escolha: Some(escolha.clone()),
                        confianca: *confianca,
                        ..RespostaJev::default()
                    },
                );
            }
        }
        Ok(LoteResposta {
            respostas,
            tokens: 12,
            ..LoteResposta::default()
        })
    }))
}

fn ultima_fala_do_cliente(estado: &serde_json::Value) -> String {
    estado["conversa"]["mensagens"]
        .as_array()
        .and_then(|mensagens| {
            mensagens
                .iter()
                .rev()
                .find(|mensagem| mensagem["papel"] == "cliente")
        })
        .and_then(|mensagem| mensagem["texto"].as_str())
        .unwrap_or_default()
        .to_owned()
}

fn motor_jev(jev: Arc<JevFalso>, gates: JevGates) -> Arc<MotorJev> {
    Arc::new(MotorJev::novo(jev, gates, "jev-latest"))
}

const ENCERRA: &str = "era só isso, obrigado";
const DUVIDA: &str = "obrigado, mas ainda tenho uma dúvida: e se a praga voltar?";
const VOLTOU: &str = "oi, voltei. preciso de um orçamento";

#[tokio::test]
async fn g8_encerra_sem_chamar_o_modelo_e_grava_a_despedida_padrao() {
    let cenario = Cenario::novo("padrao");
    cenario.resposta_do_agente("Mais alguma dúvida?");
    let jev = jev_por_texto(&[(ENCERRA, &[("g8_encerrada", "encerrada", 1.0)])]);
    let (motor, provedor) = cenario.motor(
        vec![roteiro_texto("isto não deve sair")],
        Some(motor_jev(Arc::clone(&jev), so_g8())),
    );

    let gatilho = cenario.falar(ENCERRA);
    let resumo = cenario.turno(&motor, gatilho).await;

    assert_eq!(resumo.estado, EstadoTurno::Concluido);
    assert_eq!(resumo.texto, G8_DESPEDIDA_PADRAO);
    assert_eq!(
        provedor.recebidos().len(),
        0,
        "o modelo de texto não é chamado"
    );
    assert_eq!(
        cenario.saidas(),
        vec!["Mais alguma dúvida?", G8_DESPEDIDA_PADRAO]
    );
    assert_eq!(
        cenario.fila().len(),
        1,
        "a despedida vai para a fila de saída"
    );
    assert_eq!(cenario.metricas.encerradas_g8.value(), 1);
    assert_eq!(cenario.metricas.turnos_concluidos.value(), 1);
    assert_eq!(
        cenario.status(),
        StatusConversa::Aberta,
        "o encerramento não muda a conversa"
    );
}

#[tokio::test]
async fn g8_usa_a_despedida_configurada_no_agente() {
    let cenario = Cenario::com_demora(
        "despedida",
        Duration::ZERO,
        r#"{"despedida": "Valeu, até a próxima!"}"#,
    );
    cenario.resposta_do_agente("Mais alguma dúvida?");
    let jev = jev_por_texto(&[(ENCERRA, &[("g8_encerrada", "encerrada", 1.0)])]);
    let (motor, provedor) = cenario.motor(
        vec![roteiro_texto("não sai")],
        Some(motor_jev(jev, so_g8())),
    );

    let gatilho = cenario.falar(ENCERRA);
    let resumo = cenario.turno(&motor, gatilho).await;

    assert_eq!(resumo.texto, "Valeu, até a próxima!");
    assert_eq!(provedor.recebidos().len(), 0);
    assert_eq!(
        cenario.saidas().last().map(String::as_str),
        Some("Valeu, até a próxima!")
    );
}

#[tokio::test]
async fn g8_despedida_vazia_encerra_sem_mandar_mensagem() {
    let cenario = Cenario::com_demora("silencio", Duration::ZERO, r#"{"despedida": ""}"#);
    cenario.resposta_do_agente("Mais alguma dúvida?");
    let jev = jev_por_texto(&[(ENCERRA, &[("g8_encerrada", "encerrada", 1.0)])]);
    let (motor, provedor) = cenario.motor(
        vec![roteiro_texto("não sai")],
        Some(motor_jev(jev, so_g8())),
    );

    let gatilho = cenario.falar(ENCERRA);
    let resumo = cenario.turno(&motor, gatilho).await;

    assert_eq!(resumo.estado, EstadoTurno::Concluido);
    assert!(resumo.texto.is_empty());
    assert_eq!(provedor.recebidos().len(), 0);
    assert_eq!(
        cenario.saidas(),
        vec!["Mais alguma dúvida?"],
        "só a resposta anterior"
    );
    assert!(cenario.fila().is_empty());
    assert_eq!(cenario.metricas.encerradas_g8.value(), 1);
}

#[tokio::test]
async fn g8_a_mensagem_seguinte_reabre_a_conversa_e_volta_ao_modelo() {
    let cenario = Cenario::novo("reabre");
    cenario.resposta_do_agente("Mais alguma dúvida?");
    let jev = jev_por_texto(&[
        (ENCERRA, &[("g8_encerrada", "encerrada", 1.0)]),
        (VOLTOU, &[("g8_encerrada", "continua", 0.95)]),
    ]);
    let (motor, provedor) = cenario.motor(
        vec![roteiro_texto("Claro! Qual o tamanho do imóvel?")],
        Some(motor_jev(jev, so_g8())),
    );

    let primeiro = cenario.falar(ENCERRA);
    cenario.turno(&motor, primeiro).await;
    assert_eq!(provedor.recebidos().len(), 0);

    let segundo = cenario.falar(VOLTOU);
    let resumo = cenario.turno(&motor, segundo).await;

    assert_eq!(resumo.texto, "Claro! Qual o tamanho do imóvel?");
    assert_eq!(
        provedor.recebidos().len(),
        1,
        "a mensagem seguinte entra no turno normal"
    );
    assert_eq!(
        cenario.saidas().last().map(String::as_str),
        Some("Claro! Qual o tamanho do imóvel?")
    );
}

#[tokio::test]
async fn g8_despedida_vazia_nao_engole_a_mensagem_seguinte() {
    let cenario = Cenario::com_demora("silencio-seguinte", Duration::ZERO, r#"{"despedida": ""}"#);
    cenario.resposta_do_agente("Mais alguma dúvida?");
    let jev = jev_por_texto(&[
        (ENCERRA, &[("g8_encerrada", "encerrada", 1.0)]),
        (VOLTOU, &[("g8_encerrada", "continua", 0.95)]),
    ]);
    let (motor, provedor) = cenario.motor(
        vec![roteiro_texto("Claro, em que posso ajudar?")],
        Some(motor_jev(jev, so_g8())),
    );

    let primeiro = cenario.falar(ENCERRA);
    cenario.turno(&motor, primeiro).await;
    let segundo = cenario.falar(VOLTOU);
    let resumo = cenario.turno(&motor, segundo).await;

    assert_eq!(resumo.texto, "Claro, em que posso ajudar?");
    assert_eq!(provedor.recebidos().len(), 1);
    assert_eq!(
        cenario.saidas().last().map(String::as_str),
        Some("Claro, em que posso ajudar?")
    );
}

#[tokio::test]
async fn g8_agradecimento_com_duvida_nao_fecha() {
    let cenario = Cenario::novo("duvida");
    cenario.resposta_do_agente("O serviço tem garantia de 6 meses.");
    let jev = jev_por_texto(&[(DUVIDA, &[("g8_encerrada", "continua", 0.95)])]);
    let (motor, provedor) = cenario.motor(
        vec![roteiro_texto(
            "Se a praga voltar, a garantia cobre a visita.",
        )],
        Some(motor_jev(jev, so_g8())),
    );

    let gatilho = cenario.falar(DUVIDA);
    let resumo = cenario.turno(&motor, gatilho).await;

    assert_eq!(
        resumo.texto,
        "Se a praga voltar, a garantia cobre a visita."
    );
    assert_eq!(provedor.recebidos().len(), 1, "a dúvida segue ao modelo");
    assert_eq!(cenario.metricas.encerradas_g8.value(), 0);
}

#[tokio::test]
async fn g8_mensagens_agrupadas_nao_fecham() {
    // Duas mensagens na mesma janela: uma dúvida e, logo depois, um "obrigado, era só isso". O Jev
    // julga a última e diria `encerrada`, mas a dúvida ficaria sem resposta: o turno não fecha.
    let cenario = Cenario::novo("agrupadas");
    cenario.resposta_do_agente("Mais alguma dúvida?");
    let duvida = "e a garantia, vale por quanto tempo?";
    let jev = jev_por_texto(&[(ENCERRA, &[("g8_encerrada", "encerrada", 1.0)])]);
    let (motor, provedor) = cenario.motor(
        vec![roteiro_texto("A garantia vale por 6 meses.")],
        Some(motor_jev(jev, so_g8())),
    );

    cenario.falar(duvida);
    let gatilho = cenario.falar(ENCERRA);
    let resumo = cenario.turno(&motor, gatilho).await;

    assert_eq!(
        resumo.texto, "A garantia vale por 6 meses.",
        "a dúvida é respondida"
    );
    assert_eq!(provedor.recebidos().len(), 1);
    assert_eq!(cenario.metricas.encerradas_g8.value(), 0);
}

#[tokio::test]
async fn g8_primeira_mensagem_nao_pergunta_nem_fecha() {
    let cenario = Cenario::novo("primeira");
    // "obrigado" de abertura: a regra da HR-36 não deixa o Jev nem ser perguntado.
    let jev = jev_por_texto(&[("obrigado", &[("g8_encerrada", "encerrada", 1.0)])]);
    let (motor, provedor) = cenario.motor(
        vec![roteiro_texto("De nada! Como posso ajudar?")],
        Some(motor_jev(Arc::clone(&jev), so_g8())),
    );

    let gatilho = cenario.falar("obrigado");
    let resumo = cenario.turno(&motor, gatilho).await;

    assert_eq!(
        jev.chamadas(),
        0,
        "sem fala anterior do agente, o G8 nem é perguntado"
    );
    assert_eq!(resumo.texto, "De nada! Como posso ajudar?");
    assert_eq!(provedor.recebidos().len(), 1);
    assert_eq!(cenario.metricas.encerradas_g8.value(), 0);
}

#[tokio::test]
async fn g8_desligado_nao_pergunta_nem_fecha() {
    let cenario = Cenario::novo("desligado");
    cenario.resposta_do_agente("Mais alguma dúvida?");
    let jev = jev_por_texto(&[(ENCERRA, &[("g8_encerrada", "encerrada", 1.0)])]);
    let (motor, provedor) = cenario.motor(
        vec![roteiro_texto("Até mais, qualquer coisa estou aqui.")],
        Some(motor_jev(Arc::clone(&jev), JevGates::default())),
    );

    let gatilho = cenario.falar(ENCERRA);
    let resumo = cenario.turno(&motor, gatilho).await;

    assert_eq!(jev.chamadas(), 0, "com o G8 desligado nenhuma pergunta");
    assert_eq!(resumo.texto, "Até mais, qualquer coisa estou aqui.");
    assert_eq!(
        provedor.recebidos().len(),
        1,
        "comportamento de antes da HR-36"
    );
}

#[tokio::test]
async fn g8_falha_ou_duvida_do_jev_segue_como_hoje() {
    let casos: Vec<(&str, Arc<JevFalso>)> = vec![
        (
            "jev fora do ar",
            Arc::new(JevFalso::falhando(ErroJev::TempoEsgotado { ms: 8000 })),
        ),
        (
            "confiança 0,79",
            jev_por_texto(&[(ENCERRA, &[("g8_encerrada", "encerrada", 0.79)])]),
        ),
        (
            "escolha fora do contrato",
            jev_por_texto(&[(ENCERRA, &[("g8_encerrada", "talvez", 1.0)])]),
        ),
        ("sem resposta", jev_por_texto(&[])),
    ];
    for (rotulo, jev) in casos {
        let cenario = Cenario::novo(&format!("falha-{}", rotulo.replace(' ', "-")));
        cenario.resposta_do_agente("Mais alguma dúvida?");
        let (motor, provedor) = cenario.motor(
            vec![roteiro_texto("Certo, qualquer coisa estou aqui.")],
            Some(motor_jev(jev, so_g8())),
        );

        let gatilho = cenario.falar(ENCERRA);
        let resumo = cenario.turno(&motor, gatilho).await;

        assert_eq!(
            resumo.texto, "Certo, qualquer coisa estou aqui.",
            "{rotulo}"
        );
        assert_eq!(provedor.recebidos().len(), 1, "{rotulo}: o modelo responde");
        assert_eq!(
            cenario.metricas.encerradas_g8.value(),
            0,
            "{rotulo}: não fecha"
        );
        assert_eq!(
            cenario.saidas().last().map(String::as_str),
            Some("Certo, qualquer coisa estou aqui."),
            "{rotulo}"
        );
    }
}

#[tokio::test]
async fn g8_com_transferencia_do_g1_a_transferencia_vence() {
    let cenario = Cenario::novo("g1-vence");
    cenario.resposta_do_agente("Mais alguma dúvida?");
    let texto = "era só isso, tchau, mas quero falar com uma pessoa";
    let jev = jev_por_texto(&[(
        texto,
        &[
            ("g1_transferir", "transferir_agora", 1.0),
            ("g8_encerrada", "encerrada", 1.0),
        ],
    )]);
    let gates = JevGates {
        g1: true,
        g8: true,
        ..JevGates::default()
    };
    let (motor, provedor) =
        cenario.motor(vec![roteiro_texto("não sai")], Some(motor_jev(jev, gates)));

    let gatilho = cenario.falar(texto);
    let resumo = cenario.turno(&motor, gatilho).await;

    assert_eq!(resumo.estado, EstadoTurno::Concluido);
    assert_eq!(
        cenario.status(),
        StatusConversa::Transferida,
        "o G1 transfere"
    );
    assert_eq!(provedor.recebidos().len(), 0);
    assert_eq!(
        cenario.metricas.encerradas_g8.value(),
        0,
        "não conta como encerramento"
    );
}

#[tokio::test]
async fn g8_encerrando_nao_espera_a_busca_em_andamento() {
    let cenario = Cenario::com_demora("abandona", Duration::from_millis(300), "");
    cenario.resposta_do_agente("Mais alguma dúvida?");
    let jev = jev_por_texto(&[(ENCERRA, &[("g8_encerrada", "encerrada", 1.0)])]);
    let (motor, provedor) = cenario.motor(
        vec![roteiro_texto("não sai")],
        Some(motor_jev(jev, so_g8())),
    );

    let gatilho = cenario.falar(ENCERRA);
    let inicio = Instant::now();
    let resumo = cenario.turno(&motor, gatilho).await;
    let levou = inicio.elapsed();

    assert_eq!(resumo.estado, EstadoTurno::Concluido);
    assert_eq!(provedor.recebidos().len(), 0);
    assert!(
        levou < Duration::from_millis(250),
        "encerrando, o turno não pode esperar a busca de 300 ms: levou {levou:?}"
    );
}

/// Padrão de instalação (decisão do mantenedor, 20/11/2026): agente sem configuração de gates. O G8
/// encerra com a despedida padrão sem chamar o modelo, e a mensagem seguinte reabre a conversa.
#[tokio::test]
async fn padrao_de_instalacao_liga_o_g8_sem_configurar_o_agente() {
    let cenario = Cenario::novo("padrao-g8");
    cenario.resposta_do_agente("Mais alguma dúvida?");
    let jev = jev_por_texto(&[
        (ENCERRA, &[("g8_encerrada", "encerrada", 1.0)]),
        (VOLTOU, &[("g8_encerrada", "continua", 0.95)]),
    ]);
    let gates_padrao = poph_types::JevConfig::default().gates;
    assert!(gates_padrao.g8, "o padrão de instalação liga o G8");
    let (motor, provedor) = cenario.motor(
        vec![roteiro_texto("Claro! Qual o tamanho do imóvel?")],
        Some(motor_jev(jev, gates_padrao)),
    );

    let primeiro = cenario.falar(ENCERRA);
    let encerrou = cenario.turno(&motor, primeiro).await;

    assert_eq!(encerrou.texto, G8_DESPEDIDA_PADRAO);
    assert_eq!(
        provedor.recebidos().len(),
        0,
        "sem configuração no agente, o encerramento não chama o modelo de texto"
    );
    assert_eq!(cenario.metricas.encerradas_g8.value(), 1);

    let segundo = cenario.falar(VOLTOU);
    let resumo = cenario.turno(&motor, segundo).await;

    assert_eq!(resumo.texto, "Claro! Qual o tamanho do imóvel?");
    assert_eq!(
        provedor.recebidos().len(),
        1,
        "a mensagem seguinte reabre e volta ao modelo"
    );
}

/// Agente com `{"g8": false}` desliga o G8 mesmo com o padrão ligado: o encerramento volta a ser
/// uma mensagem comum, o Jev não é perguntado sobre o G8 e o modelo responde.
#[tokio::test]
async fn agente_que_desliga_o_g8_volta_ao_comportamento_de_hoje() {
    let cenario = Cenario::com_demora("agente-g8-off", Duration::ZERO, r#"{"g8": false}"#);
    // Com uma resposta anterior do agente, o encerramento seria fechado se o G8 estivesse ligado.
    cenario.resposta_do_agente("Mais alguma dúvida?");
    let jev = jev_por_texto(&[(ENCERRA, &[("g8_encerrada", "encerrada", 1.0)])]);
    let gates_padrao = poph_types::JevConfig::default().gates;
    let (motor, provedor) = cenario.motor(
        vec![roteiro_texto("Até mais, qualquer coisa estou aqui.")],
        Some(motor_jev(jev, gates_padrao)),
    );

    let gatilho = cenario.falar(ENCERRA);
    let resumo = cenario.turno(&motor, gatilho).await;

    assert_eq!(resumo.texto, "Até mais, qualquer coisa estou aqui.");
    assert_eq!(
        provedor.recebidos().len(),
        1,
        "o modelo responde, como antes"
    );
    assert_eq!(cenario.metricas.encerradas_g8.value(), 0);
    assert!(
        resumo
            .jev
            .as_ref()
            .and_then(|jev| jev.escolha(poph_jev::IdGate::G8))
            .is_none(),
        "com o G8 desligado no agente, o Jev não é perguntado sobre ele"
    );
}

/// Todo o texto de um pedido ao modelo (sistema, usuário, assistente e resultado de ferramenta).
fn caracteres_do_pedido(pedido: &poph_core::llm::Pedido) -> usize {
    pedido
        .mensagens
        .iter()
        .map(|mensagem| match mensagem {
            poph_core::llm::Mensagem::Sistema(texto) => texto.chars().count(),
            poph_core::llm::Mensagem::Usuario(partes) => partes
                .iter()
                .map(|parte| match parte {
                    poph_core::llm::Parte::Texto(texto) => texto.chars().count(),
                    _ => 0,
                })
                .sum(),
            poph_core::llm::Mensagem::Assistente { texto, .. } => texto.chars().count(),
            poph_core::llm::Mensagem::Resultado { conteudo, .. } => conteudo.chars().count(),
        })
        .sum()
}

/// Medição (não roda no CI): as mesmas mensagens de encerramento, com o G8 desligado e ligado.
///
/// `cargo test -p poph-agent --test g8_encerrada medicao -- --ignored --nocapture`
///
/// Conta as chamadas ao modelo de texto e os caracteres que iriam no prompt de cada turno que o G8
/// encerra. Com provedor falso: a latência do modelo real não está aqui, só a chamada que deixa de
/// existir.
#[tokio::test]
#[ignore]
async fn medicao_do_encerramento() {
    let mensagens = [
        ENCERRA,
        "tchau, valeu",
        "ok, obrigado, era isso mesmo",
        "certo, muito obrigado, até mais",
    ];
    for ligado in [false, true] {
        let mut chamadas = 0_usize;
        let mut caracteres = 0_usize;
        for (indice, texto) in mensagens.iter().enumerate() {
            let cenario = Cenario::novo(&format!("medicao-{ligado}-{indice}"));
            cenario.resposta_do_agente("Mais alguma dúvida sobre a dedetização?");
            let jev = jev_por_texto(&[(*texto, &[("g8_encerrada", "encerrada", 1.0)])]);
            let gates = if ligado { so_g8() } else { JevGates::default() };
            let (motor, provedor) = cenario.motor(
                vec![roteiro_texto("Certo, qualquer coisa estou aqui.")],
                Some(motor_jev(jev, gates)),
            );
            let gatilho = cenario.falar(texto);
            cenario.turno(&motor, gatilho).await;
            for pedido in provedor.recebidos() {
                chamadas += 1;
                caracteres += caracteres_do_pedido(&pedido);
            }
        }
        let rotulo = if ligado { "G8 ligado" } else { "G8 desligado" };
        println!(
            "{rotulo}: {} mensagens de encerramento → {chamadas} chamadas ao modelo de texto, {caracteres} caracteres de prompt",
            mensagens.len()
        );
    }
}
