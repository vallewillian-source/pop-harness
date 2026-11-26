//! Ferramentas do fluxo no turno (FX-24, PRD §6.5).
//!
//! Modelo e Jev falsos, banco de verdade. O que fica provado:
//!
//! 1. ação válida avança e a resposta ao modelo diz a etapa nova (e o efeito vai para a fila, não
//!    para a `outbox`: ele sai depois do texto final);
//! 2. campo faltando devolve a lista do que falta e nada muda no banco;
//! 3. ação inexistente devolve as válidas e nada muda;
//! 4. ferramenta fora da lista habilitada do agente é recusada (`nao_habilitada`) e nem é oferecida;
//! 5. `fluxo_iniciar` só existe na reserva e valida como o G9 (fluxo inativo ou não liberado nunca
//!    inicia);
//! 6. chave de idempotência repetida não duplica evento, efeito nem versão;
//! 7. `max_ferramentas_por_mensagem` continua valendo com as ferramentas novas;
//! 8. abandonar grava o motivo, não gera efeito;
//! 9. ação que chega a um `fim(encerra_atendimento=True)` fecha a Ficha e a conversa;
//! 10. motor desligado (`[fluxos] ativo = false`) não registra nem oferece as três ferramentas.
//!
//! Nomes, textos e ids são fictícios.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use poph_agent::config_quente::{ConfigQuente, OpcoesConfigQuente};
use poph_agent::limites::{Limitador, OpcoesLimites};
use poph_agent::metricas::{MetricasG9, MetricasMemoria, MetricasTurno};
use poph_agent::turno::{
    Entrada, FuturoPausa, Motor, ObservadorNulo, OpcoesTurno, Pausa, ResumoTurno,
};
use poph_core::llm::{
    ChamadaFerramenta, EventoLlm, Mensagem, MotivoFim, Pedido, ProvedorFake, Roteiro, Uso,
};
use poph_fluxos::{Limites, compilar};
use poph_jev::{G9_NENHUM, JevFalso, MotorJev};
use poph_store::{
    Direcao, EstadoTurno, MudancaAgente, NovaConversa, NovaMensagem, NovaOrg, NovoAgente,
    NovoFluxo, Papel, PublicacaoDeRevisao, StatusConversa, Store,
};
use poph_types::{
    AgentId, ClosedBy, ConversationId, ExecutionStatus, FlowsConfig, JevGates, MessageId, OrgId,
    RecordEventKind, RecordStatus,
};
use serde_json::{Value, json};

struct SemEspera;

impl Pausa for SemEspera {
    fn esperar(&self, _duracao: Duration) -> FuturoPausa {
        Box::pin(async {})
    }
}

/// Duas etapas: `coleta` (com campo obrigatório e um efeito na ação) e `confirmacao`, que conclui o
/// fluxo e encerra o atendimento.
const SCRIPT: &str = r#"fluxo(
    id = "agendar_visita",
    nome = "Agendar visita",
    inicio = "coleta",
    gatilho = gatilho(quando = "O cliente quer marcar uma visita técnica."),
)

etapa(
    id = "coleta",
    nome = "Coleta",
    instrucao = "Peça o nome do cliente.",
    campos = [campo("nome", "Nome", obrigatorio = True)],
    acoes = [
        acao(
            "seguir",
            "Seguir",
            exige = ["nome"],
            vai_para = "confirmacao",
            efeitos = [mensagem_ao_cliente("Já vou confirmar sua visita.")],
        ),
    ],
)

etapa(
    id = "confirmacao",
    nome = "Confirmação",
    instrucao = "Confirme a visita com o cliente.",
    acoes = [
        acao(
            "confirmar",
            "Confirmar",
            vai_para = fim("pronto", resultado = "Visita agendada", encerra_atendimento = True),
        ),
    ],
)
"#;

/// Fluxo auxiliar: mantém a reserva viva (um fluxo liberado no catálogo) quando o alvo do teste
/// não pode ser liberado ou está inativo.
const SCRIPT_B: &str = r#"fluxo(
    id = "confirmar_dados",
    nome = "Confirmar dados",
    inicio = "dados",
)

etapa(
    id = "dados",
    nome = "Dados",
    acoes = [acao("seguir", "Seguir", vai_para = fim("pronto"))],
)
"#;

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
        let _ = std::fs::remove_dir_all(&pasta);
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

    /// Cria o fluxo, publica a primeira revisão e o libera ao agente.
    fn fluxo(&self) -> poph_types::FlowId {
        self.criar_fluxo("agendar_visita", "Agendar visita", SCRIPT, true)
    }

    /// Cria um fluxo com o script dado, publica a revisão e (opcionalmente) libera ao agente.
    fn criar_fluxo(
        &self,
        slug: &str,
        nome: &str,
        script: &str,
        liberar: bool,
    ) -> poph_types::FlowId {
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

    /// Deixa o fluxo inativo (não inicia execução nova).
    fn desativar(&self, fluxo: poph_types::FlowId) {
        let atual = self
            .store
            .fluxos()
            .obter(self.org, fluxo)
            .expect("lê o fluxo");
        self.store
            .fluxos()
            .atualizar(
                self.org,
                fluxo,
                atual.versao,
                poph_store::MudancaFluxo {
                    status: Some(poph_store::Status::Inativo),
                    ..poph_store::MudancaFluxo::default()
                },
            )
            .expect("desativa o fluxo");
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
        fluxos: Option<FlowsConfig>,
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

    fn ativa(&self, conversa: ConversationId) -> Option<poph_store::Execucao> {
        self.store
            .execucoes()
            .ativa_da_ficha(self.org, self.ficha(conversa).id)
            .expect("lê a execução ativa")
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

    fn eventos(&self, conversa: ConversationId) -> Vec<poph_store::EventoFicha> {
        self.store
            .fichas()
            .eventos(self.org, self.ficha(conversa).id, None, 50)
            .expect("lê os eventos")
    }

    fn pendentes(&self) -> Vec<poph_store::EfeitoDoFluxo> {
        self.store
            .efeitos()
            .listar_pendentes(self.org, 20)
            .expect("lê a fila de efeitos")
    }

    /// Mensagens gravadas na conversa (a `outbox` do canal não é tocada por este turno).
    fn mensagens(&self, conversa: ConversationId) -> Vec<poph_store::Mensagem> {
        self.store
            .mensagens()
            .listar(self.org, conversa, poph_store::Pagina::com_limite(50))
            .expect("lista as mensagens")
    }

    fn turno_gravado(&self, conversa: ConversationId) -> poph_store::Turno {
        self.store
            .turnos()
            .listar(self.org, conversa, poph_store::Pagina::com_limite(5))
            .expect("lista os turnos")
            .into_iter()
            .next()
            .expect("há um turno gravado")
    }

    fn chamadas(&self, conversa: ConversationId) -> Vec<Value> {
        let turno = self.turno_gravado(conversa);
        let registro: Value =
            serde_json::from_str(&turno.ferramentas_json).expect("tools_json é JSON");
        registro["chamadas"].as_array().cloned().unwrap_or_default()
    }

    /// Liga/desliga um ajuste do agente (o teto de chamadas por mensagem, por exemplo).
    fn ajustar(&self, mudanca: MudancaAgente) {
        let atual = self
            .store
            .agentes()
            .obter(self.org, self.agente)
            .expect("lê o agente");
        self.store
            .agentes()
            .atualizar(self.org, self.agente, atual.versao, mudanca)
            .expect("ajusta o agente");
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

/// Roteiro do modelo que pede duas ferramentas na mesma mensagem.
fn duas_chamadas(primeira: (&str, Value), segunda: (&str, Value)) -> Roteiro {
    Roteiro::Eventos(vec![
        EventoLlm::Ferramenta(ChamadaFerramenta {
            id: "chamada-1".to_owned(),
            nome: primeira.0.to_owned(),
            argumentos: primeira.1,
        }),
        EventoLlm::Ferramenta(ChamadaFerramenta {
            id: "chamada-2".to_owned(),
            nome: segunda.0.to_owned(),
            argumentos: segunda.1,
        }),
        EventoLlm::Fim(MotivoFim::Concluido),
    ])
}

fn gates_com_g9() -> JevGates {
    JevGates {
        g9: true,
        ..JevGates::default()
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

/// O que o modelo recebeu como resultado das ferramentas (a mensagem de `Resultado` do pedido).
fn resultados(pedido: &Pedido) -> Vec<String> {
    pedido
        .mensagens
        .iter()
        .filter_map(|mensagem| match mensagem {
            Mensagem::Resultado { conteudo, .. } => Some(conteudo.clone()),
            _ => None,
        })
        .collect()
}

/// Nomes das ferramentas oferecidas no pedido.
fn oferecidas(pedido: &Pedido) -> Vec<String> {
    pedido
        .ferramentas
        .iter()
        .map(|ferramenta| ferramenta.nome.clone())
        .collect()
}

/// Um turno com o Jev escolhendo o fluxo (a execução abre antes do modelo).
async fn turno_com_fluxo(
    cenario: &Cenario,
    roteiros: Vec<Roteiro>,
    conversa: ConversationId,
) -> (Arc<ProvedorFake>, ResumoTurno) {
    let jev = Arc::new(JevFalso::com_escolhas(escolhas(&[(
        "g9_iniciar_fluxo",
        "agendar_visita",
    )])));
    let (motor, provedor) =
        cenario.motor(roteiros, Some(motor_jev(jev)), Some(FlowsConfig::default()));
    let gatilho = cenario.falar(conversa, "queria marcar uma visita técnica");
    let resumo = cenario.turno(&motor, conversa, gatilho).await;
    (provedor, resumo)
}

// ──────────────────────────────────────────────────────────────────────────── testes

/// 1. Ação válida avança, a resposta ao modelo diz a etapa nova e o efeito fica na fila — não vai
///    para a `outbox` (ele sai depois do texto final do turno, FX-21).
#[tokio::test]
async fn acao_valida_avanca_e_a_resposta_diz_a_etapa_nova() {
    let cenario = Cenario::novo("avanca");
    cenario.fluxo();
    let conversa = cenario.conversa();
    let (provedor, resumo) = turno_com_fluxo(
        &cenario,
        vec![
            chamada(
                "fluxo_executar_acao",
                json!({"acao": "seguir", "campos": {"nome": "Ana Teste"}}),
            ),
            texto("Pronto, Ana! Visita confirmada."),
        ],
        conversa,
    )
    .await;
    assert_eq!(resumo.estado, EstadoTurno::Concluido);

    // O banco avançou de etapa.
    let execucao = cenario.execucao(conversa);
    assert_eq!(execucao.no_atual, "confirmacao");
    assert_eq!(execucao.status, ExecutionStatus::Active);

    // A ferramenta foi oferecida no primeiro pedido e a resposta da etapa nova chegou ao modelo.
    let pedidos = provedor.recebidos();
    assert_eq!(
        pedidos.len(),
        2,
        "uma rodada de ferramenta e a resposta final"
    );
    assert!(
        oferecidas(&pedidos[0]).contains(&"fluxo_executar_acao".to_owned()),
        "com execução ativa a ferramenta é oferecida: {:?}",
        oferecidas(&pedidos[0])
    );
    let resultado = resultados(&pedidos[1])
        .into_iter()
        .next()
        .expect("o modelo recebeu o resultado da ferramenta");
    assert!(resultado.contains("avançou"), "{resultado}");
    assert!(resultado.contains("Confirmação"), "{resultado}");
    assert!(resultado.contains("`confirmar`"), "{resultado}");
    assert!(
        resultado.contains("agendou uma mensagem própria"),
        "a resposta avisa do efeito enfileirado: {resultado}"
    );

    // O evento `acao_executada` está na Ficha, assinado pelo agente.
    let eventos = cenario.eventos(conversa);
    let acao = eventos
        .iter()
        .find(|evento| evento.tipo == RecordEventKind::ActionExecuted)
        .expect("há o evento acao_executada");
    assert_eq!(acao.ator_tipo, poph_types::EventActor::Agent);

    // O efeito `mensagem_ao_cliente` está na fila do fluxo, e não na conversa: quem entrega é o
    // laço do servidor, que espera o turno terminar (FX-21).
    let pendentes = cenario.pendentes();
    assert_eq!(pendentes.len(), 1, "um efeito na fila");
    assert_eq!(pendentes[0].tipo, "mensagem_ao_cliente");
    let saida: Vec<_> = cenario
        .mensagens(conversa)
        .into_iter()
        .filter(|mensagem| mensagem.direcao == Direcao::Saida)
        .collect();
    assert_eq!(
        saida.len(),
        1,
        "só a resposta do agente foi para a conversa"
    );
    assert!(saida[0].texto.contains("Pronto, Ana!"));

    // O registro do turno guarda a chamada executada.
    let chamadas = cenario.chamadas(conversa);
    assert_eq!(chamadas.len(), 1);
    assert_eq!(chamadas[0]["nome"], "fluxo_executar_acao");
    assert_eq!(chamadas[0]["estado"], "executada");
}

/// 2. Campo obrigatório faltando: a resposta lista o que falta e nada muda no banco.
#[tokio::test]
async fn campo_faltando_devolve_a_lista_e_nada_muda() {
    let cenario = Cenario::novo("campo");
    cenario.fluxo();
    let conversa = cenario.conversa();
    let (provedor, _) = turno_com_fluxo(
        &cenario,
        vec![
            chamada("fluxo_executar_acao", json!({"acao": "seguir"})),
            texto("Pode me dizer seu nome?"),
        ],
        conversa,
    )
    .await;

    let resultado = resultados(&provedor.recebidos()[1])
        .into_iter()
        .next()
        .expect("o modelo recebeu a recusa");
    assert!(
        resultado.contains("exige o campo `nome`"),
        "a recusa diz qual campo falta: {resultado}"
    );
    assert!(
        resultado.contains("Campos obrigatórios que faltam: `nome`"),
        "e a descrição da etapa repete a lista: {resultado}"
    );

    // Nada mudou: mesma etapa, mesma versão, sem evento de ação.
    let execucao = cenario.execucao(conversa);
    assert_eq!(execucao.no_atual, "coleta");
    assert_eq!(execucao.versao, 1, "a versão não anda na recusa");
    assert!(
        cenario
            .eventos(conversa)
            .iter()
            .all(|evento| evento.tipo != RecordEventKind::ActionExecuted),
        "nenhum evento de ação gravado"
    );
    assert!(cenario.pendentes().is_empty(), "nenhum efeito enfileirado");
}

/// 3. Ação que não existe na etapa: a resposta traz as válidas e nada muda.
#[tokio::test]
async fn acao_inexistente_devolve_as_validas_e_nada_muda() {
    let cenario = Cenario::novo("inexistente");
    cenario.fluxo();
    let conversa = cenario.conversa();
    let (provedor, _) = turno_com_fluxo(
        &cenario,
        vec![
            chamada("fluxo_executar_acao", json!({"acao": "cancelar_tudo"})),
            texto("Certo, não vou marcar nada."),
        ],
        conversa,
    )
    .await;

    let resultado = resultados(&provedor.recebidos()[1])
        .into_iter()
        .next()
        .expect("o modelo recebeu a recusa");
    assert!(resultado.contains("`cancelar_tudo`"), "{resultado}");
    assert!(
        resultado.contains("válidas: `seguir`") || resultado.contains("Ações válidas: `seguir`"),
        "a recusa traz as ações válidas: {resultado}"
    );
    let execucao = cenario.execucao(conversa);
    assert_eq!(execucao.versao, 1);
    assert_eq!(execucao.no_atual, "coleta");
}

/// 4. Ferramenta fora da lista habilitada do agente: não é oferecida e, se o modelo inventar a
///    chamada, é recusada com `nao_habilitada`.
#[tokio::test]
async fn ferramenta_desabilitada_no_agente_e_recusada_sem_executar() {
    let cenario = Cenario::novo("desabilitada");
    cenario.habilitar(&["fluxo_abandonar"]);
    cenario.fluxo();
    let conversa = cenario.conversa();
    let (provedor, _) = turno_com_fluxo(
        &cenario,
        vec![
            chamada(
                "fluxo_executar_acao",
                json!({"acao": "seguir", "campos": {"nome": "Ana Teste"}}),
            ),
            texto("Sem problemas."),
        ],
        conversa,
    )
    .await;

    let nomes = oferecidas(&provedor.recebidos()[0]);
    assert!(
        !nomes.contains(&"fluxo_executar_acao".to_owned()),
        "só a habilitada é oferecida: {nomes:?}"
    );
    assert!(nomes.contains(&"fluxo_abandonar".to_owned()), "{nomes:?}");

    let resultado = resultados(&provedor.recebidos()[1])
        .into_iter()
        .next()
        .expect("o modelo recebeu a recusa");
    assert!(resultado.contains("não está habilitada"), "{resultado}");

    let chamadas = cenario.chamadas(conversa);
    assert_eq!(chamadas[0]["estado"], "recusada");
    assert_eq!(chamadas[0]["motivo"], "nao_habilitada");

    let execucao = cenario.execucao(conversa);
    assert_eq!(execucao.no_atual, "coleta", "nada mudou");
    assert_eq!(execucao.versao, 1);
}

/// 5. `fluxo_iniciar` só existe na reserva e valida como o G9.
#[tokio::test]
async fn fluxo_iniciar_so_aparece_na_reserva_e_valida_como_o_g9() {
    // (a) Jev respondeu `nenhum`: não é reserva, a ferramenta não é oferecida nem executada.
    let cenario = Cenario::novo("iniciar-nenhum");
    cenario.fluxo();
    let conversa = cenario.conversa();
    let jev = Arc::new(JevFalso::com_escolhas(escolhas(&[(
        "g9_iniciar_fluxo",
        G9_NENHUM,
    )])));
    let (motor, provedor) = cenario.motor(
        vec![
            chamada("fluxo_iniciar", json!({"fluxo_id": "agendar_visita"})),
            texto("Certo."),
        ],
        Some(motor_jev(jev)),
        Some(FlowsConfig::default()),
    );
    let gatilho = cenario.falar(conversa, "bom dia, tudo bem?");
    cenario.turno(&motor, conversa, gatilho).await;
    let nomes = oferecidas(&provedor.recebidos()[0]);
    assert!(
        !nomes.contains(&"fluxo_iniciar".to_owned()),
        "com o Jev respondendo, iniciar fluxo não é do modelo: {nomes:?}"
    );
    assert!(
        resultados(&provedor.recebidos()[1])[0].contains("não é sua decisão"),
        "a chamada inventada é recusada"
    );
    assert!(cenario.ativa(conversa).is_none(), "nada iniciou");

    // (b) Jev fora do ar: reserva. A ferramenta aparece e o modelo abre a execução.
    let cenario = Cenario::novo("iniciar-reserva");
    cenario.fluxo();
    let conversa = cenario.conversa();
    let (motor, provedor) = cenario.motor(
        vec![
            chamada("fluxo_iniciar", json!({"fluxo_id": "agendar_visita"})),
            texto("Perfeito, vou agendar."),
        ],
        None,
        Some(FlowsConfig::default()),
    );
    let gatilho = cenario.falar(conversa, "queria marcar uma visita");
    cenario.turno(&motor, conversa, gatilho).await;
    let nomes = oferecidas(&provedor.recebidos()[0]);
    assert!(
        nomes.contains(&"fluxo_iniciar".to_owned()),
        "na reserva a ferramenta é oferecida: {nomes:?}"
    );
    let ativa = cenario.ativa(conversa).expect("o modelo abriu a execução");
    assert_eq!(ativa.no_atual, "coleta");
    let resultado = resultados(&provedor.recebidos()[1])
        .into_iter()
        .next()
        .expect("o modelo recebeu o estado da etapa de entrada");
    assert!(resultado.contains("iniciado"), "{resultado}");
    assert!(resultado.contains("`seguir`"), "{resultado}");
    // O evento fica assinado pelo agente, não pelo Jev.
    let iniciado = cenario
        .eventos(conversa)
        .into_iter()
        .find(|evento| evento.tipo == RecordEventKind::FlowStarted)
        .expect("evento fluxo_iniciado");
    assert_eq!(iniciado.ator_tipo, poph_types::EventActor::Agent);

    // (c) Fluxo não liberado ao agente nunca inicia, mesmo com a ferramenta na mão. O fluxo
    //     liberado do catálogo mantém a reserva viva; o alvo é um fluxo publicado sem permissão.
    let cenario = Cenario::novo("iniciar-nao-liberado");
    cenario.criar_fluxo("confirmar_dados", "Confirmar dados", SCRIPT_B, true);
    cenario.criar_fluxo("agendar_visita", "Agendar visita", SCRIPT, false);
    let conversa = cenario.conversa();
    let (motor, provedor) = cenario.motor(
        vec![
            chamada("fluxo_iniciar", json!({"fluxo_id": "agendar_visita"})),
            texto("Não encontrei esse roteiro."),
        ],
        None,
        Some(FlowsConfig::default()),
    );
    let gatilho = cenario.falar(conversa, "queria marcar uma visita");
    cenario.turno(&motor, conversa, gatilho).await;
    let nomes = oferecidas(&provedor.recebidos()[0]);
    assert!(
        nomes.contains(&"fluxo_iniciar".to_owned()),
        "a reserva acontece pelo fluxo liberado: {nomes:?}"
    );
    let resultado = resultados(&provedor.recebidos()[1])
        .into_iter()
        .next()
        .expect("o modelo recebeu a recusa");
    assert!(
        resultado.contains("não está entre os liberados"),
        "{resultado}"
    );
    assert!(cenario.ativa(conversa).is_none(), "nada iniciou");

    // (d) Fluxo liberado, mas inativo: recusado pela mesma validação do G9.
    let cenario = Cenario::novo("iniciar-inativo");
    let alvo = cenario.fluxo();
    cenario.desativar(alvo);
    cenario.criar_fluxo("confirmar_dados", "Confirmar dados", SCRIPT_B, true);
    let conversa = cenario.conversa();
    let (motor, provedor) = cenario.motor(
        vec![
            chamada("fluxo_iniciar", json!({"fluxo_id": "agendar_visita"})),
            texto("Esse roteiro está fora do ar."),
        ],
        None,
        Some(FlowsConfig::default()),
    );
    let gatilho = cenario.falar(conversa, "queria marcar uma visita");
    cenario.turno(&motor, conversa, gatilho).await;
    let resultado = resultados(&provedor.recebidos()[1])
        .into_iter()
        .next()
        .expect("o modelo recebeu a recusa");
    assert!(
        resultado.contains("está inativo e não inicia execução nova"),
        "{resultado}"
    );
    assert!(cenario.ativa(conversa).is_none(), "nada iniciou");
}

/// 6. Repetir a chave de idempotência não duplica evento, efeito nem versão.
#[tokio::test]
async fn chave_repetida_nao_duplica_evento_nem_efeito() {
    let cenario = Cenario::novo("chave");
    cenario.fluxo();
    let conversa = cenario.conversa();
    // Primeira chamada com uma ação válida, segunda com a mesma chave: o banco reconhece a marca.
    let (_provedor, _) = turno_com_fluxo(
        &cenario,
        vec![
            chamada(
                "fluxo_executar_acao",
                json!({"acao": "seguir", "campos": {"nome": "Ana Teste"}, "chave_idempotencia": "acao-do-turno-1"}),
            ),
            texto("Certo."),
        ],
        conversa,
    )
    .await;
    let execucao = cenario.execucao(conversa);
    assert_eq!(execucao.no_atual, "confirmacao");
    let eventos_antes = cenario.eventos(conversa).len();
    let efeitos_antes = cenario.pendentes().len();
    assert_eq!(efeitos_antes, 1, "o efeito da ação entrou uma vez");

    // O mesmo pedido outra vez (mesma chave), agora com a execução em `confirmacao`.
    let (motor, provedor) = cenario.motor(
        vec![
            chamada(
                "fluxo_executar_acao",
                json!({"acao": "seguir", "campos": {"nome": "Ana Teste"}, "chave_idempotencia": "acao-do-turno-1"}),
            ),
            texto("De novo."),
        ],
        None,
        Some(FlowsConfig::default()),
    );
    let gatilho = cenario.falar(conversa, "como ficou?");
    cenario.turno(&motor, conversa, gatilho).await;
    let resultado = resultados(&provedor.recebidos()[1])
        .into_iter()
        .next()
        .expect("o modelo recebeu a resposta da repetição");
    assert!(
        resultado.contains("já tinha sido registrada"),
        "{resultado}"
    );

    let execucao = cenario.execucao(conversa);
    assert_eq!(execucao.no_atual, "confirmacao", "não andou de novo");
    assert_eq!(execucao.versao, 2, "a versão ficou como estava");
    assert_eq!(cenario.pendentes().len(), efeitos_antes, "sem efeito novo");
    assert_eq!(
        cenario.eventos(conversa).len(),
        eventos_antes,
        "sem evento novo"
    );
}

/// 7. O teto de chamadas por mensagem continua valendo com as ferramentas novas.
#[tokio::test]
async fn limite_de_chamadas_por_mensagem_continua_valendo() {
    let cenario = Cenario::novo("limite");
    cenario.ajustar(MudancaAgente {
        max_ferramentas_por_mensagem: Some(1),
        ..MudancaAgente::default()
    });
    cenario.fluxo();
    let conversa = cenario.conversa();
    let (provedor, _) = turno_com_fluxo(
        &cenario,
        vec![
            duas_chamadas(
                (
                    "fluxo_executar_acao",
                    json!({"acao": "seguir", "campos": {"nome": "Ana Teste"}}),
                ),
                ("fluxo_abandonar", json!({"motivo": "cliente desistiu"})),
            ),
            texto("Certo."),
        ],
        conversa,
    )
    .await;

    let resultados = resultados(&provedor.recebidos()[1]);
    assert!(
        resultados
            .iter()
            .any(|texto| texto.contains("limite de 1 chamadas")),
        "a segunda chamada é recusada pelo teto: {resultados:?}"
    );
    let chamadas = cenario.chamadas(conversa);
    assert_eq!(chamadas.len(), 2);
    assert_eq!(chamadas[1]["estado"], "recusada");
    assert_eq!(chamadas[1]["motivo"], "limite_de_chamadas");

    // A primeira ação foi aplicada normalmente.
    let execucao = cenario.execucao(conversa);
    assert_eq!(execucao.no_atual, "confirmacao");
}

/// 8. Abandonar grava o motivo, encerra a execução e não gera efeito.
#[tokio::test]
async fn abandonar_grava_motivo_e_nao_gera_efeito() {
    let cenario = Cenario::novo("abandonar");
    cenario.fluxo();
    let conversa = cenario.conversa();
    let (provedor, _) = turno_com_fluxo(
        &cenario,
        vec![
            chamada(
                "fluxo_abandonar",
                json!({"motivo": "cliente desistiu da visita"}),
            ),
            texto("Sem problemas, fico à disposição."),
        ],
        conversa,
    )
    .await;

    let resultado = resultados(&provedor.recebidos()[1])
        .into_iter()
        .next()
        .expect("o modelo recebeu a confirmação");
    assert!(resultado.contains("abandonado"), "{resultado}");

    let execucao = cenario.execucao(conversa);
    assert_eq!(execucao.status, ExecutionStatus::Abandoned);
    assert_eq!(
        execucao.motivo_fim.as_deref(),
        Some("cliente desistiu da visita")
    );
    assert!(
        cenario.ativa(conversa).is_none(),
        "não há mais execução ativa"
    );
    assert!(cenario.pendentes().is_empty(), "abandono não gera efeito");
    let evento = cenario
        .eventos(conversa)
        .into_iter()
        .find(|evento| evento.tipo == RecordEventKind::ExecutionAbandoned)
        .expect("evento de abandono");
    assert_eq!(evento.ator_tipo, poph_types::EventActor::Agent);
}

/// 9. Ação que chega a um `fim(encerra_atendimento = True)` fecha a Ficha e a conversa.
#[tokio::test]
async fn acao_que_encerra_o_atendimento_fecha_a_ficha() {
    let cenario = Cenario::novo("encerra");
    cenario.fluxo();
    let conversa = cenario.conversa();
    let (provedor, _) = turno_com_fluxo(
        &cenario,
        vec![
            chamada(
                "fluxo_executar_acao",
                json!({"acao": "seguir", "campos": {"nome": "Ana Teste"}}),
            ),
            chamada("fluxo_executar_acao", json!({"acao": "confirmar"})),
            texto("Visita confirmada, até lá!"),
        ],
        conversa,
    )
    .await;

    let resultado = resultados(&provedor.recebidos()[2])
        .into_iter()
        .last()
        .expect("o modelo recebeu o desfecho");
    assert!(resultado.contains("fim"), "{resultado}");

    let ficha = cenario.ficha(conversa);
    assert_eq!(ficha.status, RecordStatus::Closed);
    assert_eq!(ficha.fechada_por, Some(ClosedBy::Flow));
    let conversa_gravada = cenario
        .store
        .conversas()
        .obter(cenario.org, conversa)
        .expect("lê a conversa");
    assert_eq!(conversa_gravada.status, StatusConversa::Fechada);
    let execucao = cenario.execucao(conversa);
    assert_eq!(execucao.status, ExecutionStatus::Completed);
    assert_eq!(execucao.resultado.as_deref(), Some("Visita agendada"));
}

/// 10. Motor desligado: as três ferramentas não são registradas nem oferecidas.
#[tokio::test]
async fn motor_desligado_nao_registra_nem_oferece_as_ferramentas() {
    let cenario = Cenario::novo("desligado");
    cenario.fluxo();
    let conversa = cenario.conversa();
    let (motor, provedor) = cenario.motor(
        vec![texto("Oi! Como posso ajudar?")],
        None,
        Some(FlowsConfig {
            enabled: false,
            ..FlowsConfig::default()
        }),
    );
    let gatilho = cenario.falar(conversa, "oi");
    cenario.turno(&motor, conversa, gatilho).await;
    let nomes = oferecidas(&provedor.recebidos()[0]);
    for ferramenta in TODAS {
        assert!(
            !nomes.contains(&ferramenta.to_owned()),
            "`{ferramenta}` não é oferecida com o motor desligado: {nomes:?}"
        );
    }
    // Nem o registro as conhece: com o motor desligado, a chave `[fluxos] ativo` tira as
    // ferramentas do processo, não só do pedido ao modelo.
    let registro = poph_agent::turno::registrar_ferramentas_do_fluxo(
        poph_agent::turno::RegistroPadrao::novo(),
        Arc::clone(&cenario.store),
        &FlowsConfig {
            enabled: false,
            ..FlowsConfig::default()
        },
    );
    assert!(
        registro.nomes().is_empty(),
        "motor desligado não registra ferramenta de fluxo: {:?}",
        registro.nomes()
    );
    let ligado = poph_agent::turno::registrar_ferramentas_do_fluxo(
        poph_agent::turno::RegistroPadrao::novo(),
        Arc::clone(&cenario.store),
        &FlowsConfig::default(),
    );
    assert_eq!(ligado.nomes().len(), 3, "{:?}", ligado.nomes());

    // A Ficha continua existindo (o módulo de Fichas não depende do motor de fluxo).
    assert_eq!(cenario.ficha(conversa).status, RecordStatus::Open);
}

#[tokio::test]
async fn encerrar_pelo_agente_fecha_e_repeticao_e_idempotente() {
    let c = Cenario::novo("fx25-fechar");
    c.habilitar(&["atendimento_encerrar"]);
    c.fluxo();
    let conversa = c.conversa();
    let (provedor, resumo) = turno_com_fluxo(
        &c,
        vec![
            duas_chamadas(
                ("atendimento_encerrar", json!({"motivo":"resolvido"})),
                ("atendimento_encerrar", json!({"motivo":"resolvido"})),
            ),
            texto("Até logo!"),
        ],
        conversa,
    )
    .await;
    assert_eq!(resumo.estado, EstadoTurno::Concluido);
    assert!(oferecidas(&provedor.recebidos()[0]).contains(&"atendimento_encerrar".to_owned()));
    let respostas = resultados(&provedor.recebidos()[1]);
    assert!(
        respostas.iter().any(|s| s.contains("já encerrado")),
        "{respostas:?}"
    );
    assert_eq!(
        c.store.conversas().obter(c.org, conversa).unwrap().status,
        StatusConversa::Fechada
    );
    let ficha = c.ficha(conversa);
    assert_eq!(ficha.status, RecordStatus::Closed);
    assert_eq!(ficha.fechada_por, Some(ClosedBy::Agent));
    assert_eq!(ficha.fechada_motivo.as_deref(), Some("resolvido"));
    assert_eq!(c.execucao(conversa).status, ExecutionStatus::Abandoned);
    assert_eq!(
        c.eventos(conversa)
            .iter()
            .filter(|e| e.tipo == RecordEventKind::RecordClosed)
            .count(),
        1
    );
    assert_eq!(
        c.mensagens(conversa)
            .iter()
            .filter(|m| m.direcao == Direcao::Saida)
            .count(),
        1
    );
    let conn = rusqlite::Connection::open(c.pasta.join("harness.db")).unwrap();
    let n: i64 = conn
        .query_row(
            "SELECT count(*) FROM audit_log WHERE action='close' AND entity_id=?1",
            [conversa.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 1);
}

#[tokio::test]
async fn encerramento_desabilitado_e_recusado() {
    let c = Cenario::novo("fx25-desligada");
    c.habilitar(&[]);
    let conversa = c.conversa();
    let (provedor, _) = turno_com_fluxo(
        &c,
        vec![
            chamada("atendimento_encerrar", json!({"motivo":"resolvido"})),
            texto("Até logo!"),
        ],
        conversa,
    )
    .await;
    assert!(!oferecidas(&provedor.recebidos()[0]).contains(&"atendimento_encerrar".to_owned()));
    assert_eq!(c.chamadas(conversa)[0]["motivo"], "nao_habilitada");
    assert_eq!(c.ficha(conversa).status, RecordStatus::Open);
}
