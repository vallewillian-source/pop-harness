//! G4 com efeito: o departamento que recebe a transferência (HR-32).
//!
//! O que se prova aqui, no turno de verdade (banco, agente, departamentos e conversa reais):
//! - escolha válida do Jev manda a conversa para o departamento escolhido, com a origem registrada;
//! - departamento de outra org, apagado, inativo, com nome ambíguo, fora das opções ou com confiança
//!   baixa é **recusado pelo código** e a conversa vai para o departamento de transferência do
//!   agente, com o motivo gravado;
//! - sem resposta para o G4, vale o padrão (o comportamento de hoje);
//! - com o G4 desligado, o turno grava exatamente o que gravava antes: o padrão, e uma decisão só no
//!   registro (o G1);
//! - agente sem departamento de transferência continua transferindo, sem departamento;
//! - Jev fora do ar não trava o turno (e, sem o G1, não transfere: fallback é continuar).

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use poph_agent::config_quente::{ConfigQuente, OpcoesConfigQuente};
use poph_agent::limites::{Limitador, OpcoesLimites};
use poph_agent::metricas::{MetricasMemoria, MetricasTurno};
use poph_agent::turno::{Entrada, FuturoPausa, Motor, ObservadorNulo, OpcoesTurno, Pausa};
use poph_core::llm::{EventoLlm, MotivoFim, ProvedorFake, Roteiro, Uso};
use poph_jev::{
    G1_TRANSFERIR, IdGate, JevFalso, LoteResposta, MotorJev, Orientacao, Pedido, Resposta,
};
use poph_store::{
    Direcao, EstadoTurno, MudancaAgente, MudancaDepartamento, NovaConversa, NovaMensagem, NovaOrg,
    NovoAgente, NovoDepartamento, Pagina, Papel, Status, Store,
};
use poph_types::{AgentId, ConversationId, DepartmentId, JevGates, MessageId, OrgId};

const RESPOSTA_DO_MODELO: &str = "Olá! Como posso ajudar?";

struct SemEspera;

impl Pausa for SemEspera {
    fn esperar(&self, _duracao: Duration) -> FuturoPausa {
        Box::pin(async {})
    }
}

/// Banco, org, agente e conversa de verdade numa pasta temporária.
struct Cenario {
    pasta: PathBuf,
    store: Arc<Store>,
    org: OrgId,
    agente: AgentId,
    provedor: Arc<ProvedorFake>,
}

impl Cenario {
    fn novo(rotulo: &str) -> Self {
        let pasta = std::env::temp_dir().join(format!(
            "poph-hr32-{rotulo}-{}-{}",
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
                    ..NovoAgente::default()
                },
            )
            .expect("cria o agente")
            .id;
        store
            .instrucoes()
            .salvar(
                org,
                agente,
                0,
                "{}",
                "# Bia\n\nVocê é a atendente da FixProLar.",
                true,
            )
            .expect("grava as instruções");
        let atual = store.agentes().obter(org, agente).expect("lê o agente");
        store
            .agentes()
            .atualizar(
                org,
                agente,
                atual.versao,
                MudancaAgente {
                    handoff_ativo: Some(true),
                    ..MudancaAgente::default()
                },
            )
            .expect("liga o handoff");
        let provedor = Arc::new(ProvedorFake::com_roteiros(
            (0..8).map(|_| roteiro(RESPOSTA_DO_MODELO)).collect(),
        ));
        Self {
            pasta,
            store,
            org,
            agente,
            provedor,
        }
    }

    fn departamento(&self, nome: &str) -> DepartmentId {
        self.store
            .departamentos()
            .criar(
                self.org,
                NovoDepartamento {
                    nome: nome.to_owned(),
                    descricao: String::new(),
                },
            )
            .expect("cria o departamento")
            .id
    }

    fn departamento_em(&self, org: OrgId, nome: &str) -> DepartmentId {
        self.store
            .departamentos()
            .criar(
                org,
                NovoDepartamento {
                    nome: nome.to_owned(),
                    descricao: String::new(),
                },
            )
            .expect("cria o departamento")
            .id
    }

    fn outra_org(&self) -> OrgId {
        self.store
            .orgaos()
            .criar(NovaOrg {
                nome: "Outra org".to_owned(),
                slug: format!("outra-{}", AgentId::generate()),
                config_json: "{}".to_owned(),
            })
            .expect("cria a outra org")
            .id
    }

    fn desativar(&self, departamento: DepartmentId) {
        let atual = self
            .store
            .departamentos()
            .obter(self.org, departamento)
            .expect("lê o departamento");
        self.store
            .departamentos()
            .atualizar(
                self.org,
                departamento,
                atual.versao,
                MudancaDepartamento {
                    status: Some(Status::Inativo),
                    ..MudancaDepartamento::default()
                },
            )
            .expect("desativa o departamento");
    }

    fn apagar(&self, departamento: DepartmentId) {
        self.store
            .departamentos()
            .excluir(self.org, departamento)
            .expect("apaga o departamento");
    }

    fn definir_padrao(&self, departamento: DepartmentId) {
        let atual = self
            .store
            .agentes()
            .obter(self.org, self.agente)
            .expect("agente");
        self.store
            .agentes()
            .atualizar(
                self.org,
                self.agente,
                atual.versao,
                MudancaAgente {
                    handoff_departamento: Some(departamento),
                    ..MudancaAgente::default()
                },
            )
            .expect("define o departamento de transferência");
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
                    peer_externo: None,
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

    fn motor(&self, jev: Option<Arc<MotorJev>>) -> Motor {
        let config = ConfigQuente::nova(
            Arc::clone(&self.store),
            OpcoesConfigQuente::default(),
            MetricasMemoria::proprio(),
        );
        let limitador = Arc::new(Limitador::novo(
            OpcoesLimites::default(),
            MetricasMemoria::proprio(),
        ));
        let mut motor = Motor::novo(
            Arc::clone(&self.store),
            config,
            limitador,
            Arc::clone(&self.provedor) as Arc<dyn poph_core::llm::LlmProvider>,
            OpcoesTurno::default(),
            MetricasTurno::proprio(),
        )
        .com_pausa(Arc::new(SemEspera))
        .com_handoff(Orientacao::default());
        if let Some(jev) = jev {
            motor = motor.com_jev(jev);
        }
        motor
    }

    async fn turno(
        &self,
        motor: &Motor,
        conversa: ConversationId,
        gatilho: MessageId,
    ) -> poph_agent::turno::ResumoTurno {
        motor
            .executar(
                Entrada::nova(self.org, conversa, gatilho),
                &mut ObservadorNulo,
            )
            .await
            .expect("o turno roda")
    }

    fn conversa_gravada(&self, conversa: ConversationId) -> poph_store::Conversa {
        self.store
            .conversas()
            .obter(self.org, conversa)
            .expect("lê a conversa")
    }

    fn ultimo_turno(&self, conversa: ConversationId) -> poph_store::Turno {
        self.store
            .turnos()
            .listar(self.org, conversa, Pagina::com_limite(20))
            .expect("lista os turnos")
            .into_iter()
            .next_back()
            .expect("há um turno gravado")
    }

    fn auditoria_da_transferencia(&self) -> serde_json::Value {
        let registro = self
            .store
            .auditoria()
            .listar(self.org, Pagina::com_limite(50))
            .expect("lê a auditoria")
            .into_iter()
            .find(|registro| registro.acao == "transfer")
            .expect("a transferência é auditada");
        serde_json::from_str(&registro.detalhe_json).expect("detalhe é JSON")
    }
}

impl Drop for Cenario {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.pasta);
    }
}

fn roteiro(texto: &str) -> Roteiro {
    Roteiro::Eventos(vec![
        EventoLlm::Texto(texto.to_owned()),
        EventoLlm::Uso(Uso {
            entrada: 10,
            saida: 5,
        }),
        EventoLlm::Fim(MotivoFim::Concluido),
    ])
}

fn resposta(escolha: &str, confianca: f64) -> Resposta {
    let mut item = Resposta {
        escolha: Some(escolha.to_owned()),
        confianca,
        ..Resposta::default()
    };
    item.probabilidades.insert(escolha.to_owned(), confianca);
    item
}

/// Jev falso: o G1 transfere e o G4 responde o que o caso pedir.
///
/// `departamento` `None` = o lote volta **sem** resposta para o G4 (é o mesmo efeito de uma resposta
/// ausente/ilegível por gate; erro e prazo do lote atingem todos os gates juntos, e aí não há
/// transferência para rotear).
fn jev(departamento: Option<(&'static str, f64)>, veio_o_g4: bool) -> Arc<JevFalso> {
    Arc::new(JevFalso::novo(move |pedido: &Pedido| {
        let mut respostas = BTreeMap::new();
        if pedido.perguntas.contains_key(IdGate::G1.pergunta_id()) {
            respostas.insert(
                IdGate::G1.pergunta_id().to_owned(),
                resposta(G1_TRANSFERIR, 0.96),
            );
        }
        if veio_o_g4
            && pedido.perguntas.contains_key(IdGate::G4.pergunta_id())
            && let Some((nome, confianca)) = departamento
        {
            respostas.insert(
                IdGate::G4.pergunta_id().to_owned(),
                resposta(nome, confianca),
            );
        }
        Ok(LoteResposta {
            respostas,
            tokens: 11,
            ..LoteResposta::default()
        })
    }))
}

fn motor_jev(jev: Arc<JevFalso>, com_g4: bool) -> Arc<MotorJev> {
    let gates = JevGates {
        g1: true,
        g4: com_g4,
        ..JevGates::default()
    };
    Arc::new(MotorJev::novo(jev, gates, "jev-latest"))
}

// ───────────────────────────── escolha válida ─────────────────────────────

/// O Jev escolhe um departamento ativo e a conversa vai para ele — não para o padrão.
#[tokio::test]
async fn g4_manda_a_conversa_para_o_departamento_escolhido() {
    let cenario = Cenario::novo("escolha-valida");
    let padrao = cenario.departamento("Atendimento humano");
    let escolhido = cenario.departamento("Financeiro");
    cenario.definir_padrao(padrao);
    let conversa = cenario.conversa();
    let jev = jev(Some(("Financeiro", 0.93)), true);
    let motor = cenario.motor(Some(motor_jev(jev, true)));

    let gatilho = cenario.falar(conversa, "vocês cobraram duas vezes no meu cartão");
    let resumo = cenario.turno(&motor, conversa, gatilho).await;

    assert_eq!(resumo.estado, EstadoTurno::Concluido);
    assert!(
        cenario.provedor.recebidos().is_empty(),
        "transferida não chama o modelo"
    );
    let gravada = cenario.conversa_gravada(conversa);
    assert_eq!(gravada.status, poph_store::StatusConversa::Transferida);
    assert_eq!(gravada.departamento_alvo, Some(escolhido));

    // Auditoria: origem do departamento e a escolha do Jev registradas.
    let detalhe = cenario.auditoria_da_transferencia();
    assert_eq!(detalhe["departamento_origem"], "g4");
    assert_eq!(detalhe["departamento_escolhido_pelo_jev"], "Financeiro");
    assert_eq!(detalhe["departamento"], escolhido.to_string());
    assert_eq!(detalhe["departamento_motivo"], serde_json::Value::Null);

    // Registro do turno: a decisão do G4 e o efeito, juntos.
    let turno = cenario.ultimo_turno(conversa);
    let gravado: serde_json::Value = serde_json::from_str(&turno.jev_json).expect("JSON");
    assert_eq!(gravado["transferencia"]["origem"], "g4");
    assert_eq!(
        gravado["transferencia"]["departamento"],
        escolhido.to_string()
    );
    let g4 = gravado["decisoes"]
        .as_array()
        .expect("decisões")
        .iter()
        .find(|decisao| decisao["gate"] == "g4")
        .expect("o G4 decidiu no turno");
    assert_eq!(g4["escolha"], "Financeiro");
}

/// Escolha igual ao padrão: efeito igual, mas a origem registrada é a do Jev.
#[tokio::test]
async fn escolha_igual_ao_padrao_continua_vindo_do_g4() {
    let cenario = Cenario::novo("escolha-igual");
    let padrao = cenario.departamento("Suporte");
    cenario.departamento("Comercial");
    cenario.definir_padrao(padrao);
    let conversa = cenario.conversa();
    let motor = cenario.motor(Some(motor_jev(jev(Some(("Suporte", 0.9)), true), true)));
    let gatilho = cenario.falar(conversa, "o produto chegou quebrado");
    cenario.turno(&motor, conversa, gatilho).await;

    let detalhe = cenario.auditoria_da_transferencia();
    assert_eq!(detalhe["departamento_origem"], "g4");
    assert_eq!(detalhe["departamento"], padrao.to_string());
}

// ───────────────────────────── escolha recusada pelo código ─────────────────────────────

/// Departamento que só existe em outra org: o nome não está entre os candidatos daqui → padrão.
#[tokio::test]
async fn departamento_de_outra_org_cai_no_padrao() {
    let cenario = Cenario::novo("outra-org");
    let padrao = cenario.departamento("Atendimento humano");
    cenario.departamento("Comercial");
    cenario.definir_padrao(padrao);
    // "Jurídico" só existe na outra org.
    cenario.departamento_em(cenario.outra_org(), "Jurídico");

    let conversa = cenario.conversa();
    let motor = cenario.motor(Some(motor_jev(jev(Some(("Jurídico", 0.97)), true), true)));
    let gatilho = cenario.falar(conversa, "quero falar do contrato");
    cenario.turno(&motor, conversa, gatilho).await;

    let gravada = cenario.conversa_gravada(conversa);
    assert_eq!(gravada.status, poph_store::StatusConversa::Transferida);
    assert_eq!(gravada.departamento_alvo, Some(padrao));
    let detalhe = cenario.auditoria_da_transferencia();
    assert_eq!(detalhe["departamento_origem"], "padrao");
    assert!(
        detalhe["departamento_motivo"]
            .as_str()
            .unwrap_or_default()
            .contains("não é um departamento ativo"),
        "motivo: {detalhe}"
    );
}

/// Departamento desativado entre a pergunta e a decisão é recusado (a validação é no banco, agora).
#[tokio::test]
async fn departamento_inativo_cai_no_padrao() {
    let cenario = Cenario::novo("inativo");
    let padrao = cenario.departamento("Atendimento humano");
    let desativado = cenario.departamento("Fiscal");
    cenario.departamento("Comercial");
    cenario.definir_padrao(padrao);
    cenario.desativar(desativado);

    let conversa = cenario.conversa();
    let motor = cenario.motor(Some(motor_jev(jev(Some(("Fiscal", 0.95)), true), true)));
    let gatilho = cenario.falar(conversa, "preciso da nota fiscal da compra");
    cenario.turno(&motor, conversa, gatilho).await;

    assert_eq!(
        cenario.conversa_gravada(conversa).departamento_alvo,
        Some(padrao)
    );
}

/// Departamento apagado também.
#[tokio::test]
async fn departamento_apagado_cai_no_padrao() {
    let cenario = Cenario::novo("apagado");
    let padrao = cenario.departamento("Atendimento humano");
    let apagado = cenario.departamento("Logística");
    cenario.departamento("Comercial");
    cenario.definir_padrao(padrao);
    cenario.apagar(apagado);

    let conversa = cenario.conversa();
    let motor = cenario.motor(Some(motor_jev(jev(Some(("Logística", 0.95)), true), true)));
    let gatilho = cenario.falar(conversa, "meu pedido não chegou");
    cenario.turno(&motor, conversa, gatilho).await;

    assert_eq!(
        cenario.conversa_gravada(conversa).departamento_alvo,
        Some(padrao)
    );
}

/// Nome ambíguo não acontece no banco (o esquema tem índice único por `lower(name)` dentro da org),
/// então a recusa por ambiguidade do resolvedor é defesa em profundidade — provada no teste unitário
/// dele e no do `casar_candidato`. Aqui se prova o que o banco garante.
#[test]
fn o_banco_nao_deixa_dois_departamentos_com_o_mesmo_nome() {
    let cenario = Cenario::novo("nome-unico");
    cenario.departamento("Financeiro");
    let erro = cenario
        .store
        .departamentos()
        .criar(
            cenario.org,
            NovoDepartamento {
                nome: "FINANCEIRO".to_owned(),
                descricao: String::new(),
            },
        )
        .expect_err("nome repetido (sem diferenciar maiúsculas) devia ser recusado");
    assert!(
        erro.to_string().contains("já existe"),
        "erro inesperado: {erro}"
    );
}

// ───────────────────────────── abstenção ─────────────────────────────

/// Escolha fora das opções, sem resposta para o G4 e confiança baixa: todas caem no padrão, o turno
/// termina e nada fica preso.
#[tokio::test]
async fn abstencao_do_g4_cai_no_padrao_sem_travar_o_turno() {
    // (rótulo, resposta do G4, o Jev respondeu o G4?)
    type Caso = (&'static str, Option<(&'static str, f64)>, bool);
    let casos: [Caso; 3] = [
        ("fora-das-opcoes", Some(("Marketing", 0.99)), true),
        ("sem-resposta", None, false),
        ("confianca-baixa", Some(("Comercial", 0.41)), true),
    ];
    for (rotulo, departamento, veio) in casos {
        let cenario = Cenario::novo(rotulo);
        let padrao = cenario.departamento("Atendimento humano");
        cenario.departamento("Comercial");
        cenario.definir_padrao(padrao);
        let conversa = cenario.conversa();
        let motor = cenario.motor(Some(motor_jev(jev(departamento, veio), true)));
        let gatilho = cenario.falar(conversa, "quero falar com alguém sobre o pedido");
        let resumo = cenario.turno(&motor, conversa, gatilho).await;

        assert_eq!(resumo.estado, EstadoTurno::Concluido, "caso {rotulo}");
        assert_eq!(
            cenario.conversa_gravada(conversa).departamento_alvo,
            Some(padrao),
            "caso {rotulo}"
        );
        let detalhe = cenario.auditoria_da_transferencia();
        assert_eq!(detalhe["departamento_origem"], "padrao", "caso {rotulo}");
        assert!(
            detalhe["departamento_motivo"].as_str().is_some(),
            "caso {rotulo} precisa registrar o motivo da recusa"
        );
    }
}

/// Jev fora do ar (prazo estourado no lote): o turno termina normalmente e, sem o G1, não transfere
/// — o fallback do G1 é continuar. Nada trava por causa do G4.
#[tokio::test]
async fn jev_fora_do_ar_nao_trava_o_turno() {
    let cenario = Cenario::novo("jev-fora");
    let padrao = cenario.departamento("Atendimento humano");
    cenario.departamento("Comercial");
    cenario.definir_padrao(padrao);
    let conversa = cenario.conversa();

    let jev = Arc::new(
        JevFalso::com_escolhas(BTreeMap::from([(
            IdGate::G1.pergunta_id().to_owned(),
            G1_TRANSFERIR.to_owned(),
        )]))
        .com_atraso(Duration::from_millis(400)),
    );
    let avaliador = Arc::new(
        MotorJev::novo(
            jev,
            JevGates {
                g1: true,
                g4: true,
                ..JevGates::default()
            },
            "jev-latest",
        )
        .com_opcoes(poph_jev::OpcoesJev {
            tempo_maximo: Duration::from_millis(50),
            ..poph_jev::OpcoesJev::default()
        }),
    );
    let motor = cenario.motor(Some(avaliador));
    let gatilho = cenario.falar(conversa, "quero falar com um atendente");
    let resumo = cenario.turno(&motor, conversa, gatilho).await;

    assert_eq!(resumo.estado, EstadoTurno::Concluido);
    assert_eq!(
        cenario.conversa_gravada(conversa).status,
        poph_store::StatusConversa::Aberta
    );
    assert_eq!(resumo.texto, RESPOSTA_DO_MODELO);
}

// ───────────────────────────── desligado = como hoje ─────────────────────────────

/// Com o G4 desligado o turno faz exatamente o que fazia antes: grava o padrão do agente e o
/// registro traz só a decisão do G1 — nenhuma pergunta de departamento foi feita.
#[tokio::test]
async fn g4_desligado_e_o_comportamento_de_hoje() {
    let cenario = Cenario::novo("desligado");
    let padrao = cenario.departamento("Atendimento humano");
    cenario.departamento("Comercial");
    cenario.definir_padrao(padrao);
    let conversa = cenario.conversa();
    let jev = jev(Some(("Comercial", 0.99)), true);
    let avaliador = motor_jev(Arc::clone(&jev), false);
    let motor = cenario.motor(Some(avaliador));

    let gatilho = cenario.falar(conversa, "quero falar com um atendente");
    cenario.turno(&motor, conversa, gatilho).await;

    let gravada = cenario.conversa_gravada(conversa);
    assert_eq!(gravada.status, poph_store::StatusConversa::Transferida);
    assert_eq!(gravada.departamento_alvo, Some(padrao));

    // Nenhuma pergunta de departamento foi enviada ao Jev.
    let pedidos = jev.pedidos();
    assert!(
        pedidos
            .iter()
            .all(|pedido| !pedido.perguntas.contains_key(IdGate::G4.pergunta_id())),
        "G4 desligado não pode perguntar o departamento"
    );
    let detalhe = cenario.auditoria_da_transferencia();
    assert_eq!(detalhe["departamento"], padrao.to_string());
    assert_eq!(detalhe["departamento_origem"], "padrao");
    assert_eq!(
        detalhe["departamento_escolhido_pelo_jev"],
        serde_json::Value::Null
    );

    let turno = cenario.ultimo_turno(conversa);
    let gravado: serde_json::Value = serde_json::from_str(&turno.jev_json).expect("JSON");
    let gates: Vec<&str> = gravado["decisoes"]
        .as_array()
        .expect("decisões")
        .iter()
        .filter_map(|decisao| decisao["gate"].as_str())
        .collect();
    assert_eq!(gates, vec!["g1"], "só o G1 decide com o G4 desligado");
    assert_eq!(gravado["transferencia"]["origem"], "padrao");
    assert_eq!(gravado["transferencia"]["departamento"], padrao.to_string());
    assert_eq!(
        gravado["transferencia"]["motivo"],
        serde_json::Value::Null,
        "G4 desligado não inventa motivo de fallback"
    );
}

/// Agente sem departamento de transferência: a conversa transfere sem departamento, como hoje.
#[tokio::test]
async fn sem_padrao_configurado_transfere_sem_departamento() {
    let cenario = Cenario::novo("sem-padrao");
    cenario.departamento("Comercial");
    let conversa = cenario.conversa();
    // G4 desligado (comportamento de hoje) e ligado com escolha recusada: os dois terminam sem
    // departamento, porque não há padrão para onde cair.
    for (rotulo, com_g4) in [("desligado", false), ("ligado-recusado", true)] {
        let jev = jev(Some(("Marketing", 0.97)), true);
        let motor = cenario.motor(Some(motor_jev(jev, com_g4)));
        let gatilho = cenario.falar(conversa, "quero falar com uma pessoa");
        cenario.turno(&motor, conversa, gatilho).await;
        let gravada = cenario.conversa_gravada(conversa);
        assert_eq!(
            gravada.status,
            poph_store::StatusConversa::Transferida,
            "caso {rotulo}"
        );
        assert_eq!(gravada.departamento_alvo, None, "caso {rotulo}");
        // Reabre para o caso seguinte poder abrir turno.
        cenario
            .store
            .conversas()
            .reabrir_auditado(cenario.org, conversa, gravada.versao, "teste".to_owned())
            .expect("reabre");
    }
}

/// A escolha do G4 não muda o que o cliente recebe: a orientação e a mensagem automática são as
/// mesmas, com ou sem departamento escolhido.
#[tokio::test]
async fn a_escolha_do_departamento_nao_muda_o_texto_ao_cliente() {
    let cenario = Cenario::novo("texto");
    let padrao = cenario.departamento("Atendimento humano");
    cenario.departamento("Financeiro");
    cenario.definir_padrao(padrao);
    let conversa = cenario.conversa();
    let motor = cenario.motor(Some(motor_jev(jev(Some(("Financeiro", 0.95)), true), true)));
    let gatilho = cenario.falar(conversa, "quero falar com um atendente sobre a cobrança");
    let resumo = cenario.turno(&motor, conversa, gatilho).await;
    assert_eq!(resumo.texto, Orientacao::default().mensagem_automatica);
}
