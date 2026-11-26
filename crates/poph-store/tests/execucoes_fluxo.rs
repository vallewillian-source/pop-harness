//! Testes da operação atômica de transição e da abertura de execução (FX-20).
//!
//! O que fica provado aqui:
//!
//! - **abrir a execução** grava a linha da execução, a entrada na primeira etapa, o evento
//!   `fluxo_iniciado` e os efeitos do `ao_entrar` na mesma escrita;
//! - **a transição grava tudo ou nada**: falha injetada no meio (efeito com tipo que a coluna
//!   recusa) não deixa evento sem estado, nem estado sem evento;
//! - **`412` quando a `version` está velha**, e nada muda;
//! - **repetir a chave** devolve o desfecho sem duplicar evento nem efeito;
//! - **início recusado** em cada caso: Ficha fechada, fluxo não liberado, fluxo inativo e segunda
//!   execução ativa, cada um com o erro de domínio certo;
//! - **`fim` com `encerra_atendimento`** fecha conversa e Ficha na mesma transação;
//! - **duas threads** (agente × atendente) na mesma execução: só uma vence;
//! - **isolamento entre orgs** na leitura e na escrita.
//!
//! As tabelas de fluxo e revisão entram por SQL direto porque a publicação de revisões é a FX-09 e
//! não tem repositório ainda; a permissão por agente usa o repositório de verdade (`FX-08`).

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Barrier};

use poph_store::{
    Agente, ExecutionStart, ExecutionState, FlowTransition, NewAgent, NewConversation, NewOrg, Org,
    Store, StoredExecution, TransitionEffect, TransitionEvent, TransitionExtras,
};
use poph_types::{
    ActionActor, ClosedBy, DomainError, EffectState, EventActor, ExecutionStatus, FlowId,
    RecordEventKind, RecordId, RecordStatus, RevisionId, StepStatus,
};
use rusqlite::Connection;

/// Banco de verdade numa pasta temporária, apagada no fim.
struct Cenario {
    pasta: PathBuf,
    store: Option<Arc<Store>>,
}

impl Cenario {
    fn novo(rotulo: &str) -> Self {
        static CONTADOR: AtomicUsize = AtomicUsize::new(0);
        let n = CONTADOR.fetch_add(1, Ordering::Relaxed);
        let pasta = std::env::temp_dir().join(format!(
            "poph-execucoes-{rotulo}-{}-{n}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&pasta);
        fs::create_dir_all(&pasta).expect("cria a pasta do teste");
        let store =
            Store::open_at(&pasta.join("harness.db"), &pasta.join("rag.db")).expect("abre o banco");
        Self {
            pasta,
            store: Some(Arc::new(store)),
        }
    }

    fn store(&self) -> &Store {
        self.store.as_ref().expect("store viva")
    }

    /// O mesmo store para as threads (o escritor é um só; não se abre o banco duas vezes).
    fn compartilhado(&self) -> Arc<Store> {
        Arc::clone(self.store.as_ref().expect("store viva"))
    }

    fn conexao(&self) -> Connection {
        let conn = Connection::open(self.pasta.join("harness.db")).expect("abre para conferir");
        conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA busy_timeout = 5000;")
            .expect("pragmas");
        conn
    }

    fn org(&self, slug: &str) -> Org {
        self.store()
            .orgs()
            .criar(NewOrg {
                name: format!("Org {slug}"),
                slug: slug.to_owned(),
                config_json: String::new(),
            })
            .expect("cria a org")
    }

    fn agente(&self, org: &Org, slug: &str) -> Agente {
        self.store()
            .agents()
            .criar(
                org.id,
                NewAgent {
                    name: format!("Agente {slug}"),
                    slug: slug.to_owned(),
                    ..NewAgent::default()
                },
            )
            .expect("cria o agente")
    }

    /// Conversa (que já nasce com a Ficha) e o id da Ficha.
    fn ficha(&self, org: &Org, agente: &Agente) -> (poph_store::Conversa, RecordId) {
        let conversa = self
            .store()
            .conversas()
            .criar(
                org.id,
                NewConversation {
                    agent_id: agente.id,
                    integration_id: None,
                    canal: "console".to_owned(),
                    peer_externo: Some("cliente-1".to_owned()),
                    target_department: None,
                },
            )
            .expect("abre a conversa");
        let ficha = self
            .store()
            .fichas()
            .por_conversa(org.id, conversa.id)
            .expect("consulta")
            .expect("a conversa tem Ficha")
            .id;
        (conversa, ficha)
    }

    /// Fluxo publicado (revisão 1) na org, gravado direto: a publicação é a FX-09.
    fn fluxo(&self, org: &Org, slug: &str) -> (FlowId, RevisionId) {
        let conn = self.conexao();
        let fluxo: FlowId = uuid::Uuid::now_v7().to_string().parse().expect("fluxo");
        let revisao: RevisionId = uuid::Uuid::now_v7().to_string().parse().expect("revisão");
        conn.execute(
            "INSERT INTO fluxos (id, org_id, slug, nome, descricao, status, rascunho_fonte,
                                 revisao_atual, version)
             VALUES (?1, ?2, ?3, ?4, '', 'active', '', 1, 1)",
            rusqlite::params![
                fluxo.to_string(),
                org.id.to_string(),
                slug,
                format!("Fluxo {slug}")
            ],
        )
        .expect("fluxo");
        conn.execute(
            "INSERT INTO fluxo_revisoes (id, org_id, fluxo_id, numero, fonte, fonte_sha256,
                                        definicao_json, schema_versao, publicada_em, publicada_por,
                                        resolucoes_json)
             VALUES (?1, ?2, ?3, 1, 'fluxo()', 'sha', '{}', 1, 1, 'admin_token', '{}')",
            rusqlite::params![revisao.to_string(), org.id.to_string(), fluxo.to_string()],
        )
        .expect("revisão");
        (fluxo, revisao)
    }

    /// Abre a execução já com o fluxo liberado ao agente.
    fn abrir(
        &self,
        org: &Org,
        agente: &Agente,
        ficha: RecordId,
        fluxo: FlowId,
        revisao: RevisionId,
    ) -> StoredExecution {
        self.store()
            .fluxos()
            .liberar(org.id, agente.id, fluxo)
            .expect("libera o fluxo");
        self.store()
            .executions()
            .iniciar_execucao(
                org.id,
                ExecutionStart {
                    chave_idempotencia: None,
                    ficha,
                    fluxo,
                    revision: revisao,
                    actor: EventActor::Agent,
                    ator_ref: Some("agente-de-teste"),
                    jev_decision: None,
                    initial_state: estado_inicial("coleta"),
                    inbound_effects: Vec::new(),
                },
            )
            .expect("abre a execução")
    }
}

impl Drop for Cenario {
    fn drop(&mut self) {
        let _ = self.store.take();
        let _ = fs::remove_dir_all(&self.pasta);
    }
}

/// Estado de abertura que o motor devolve (`iniciar`): ativa, na etapa de início, sem dado.
fn estado_inicial(etapa: &str) -> ExecutionState {
    ExecutionState {
        status: ExecutionStatus::Active,
        no_atual: etapa.to_owned(),
        dados_json: "{}".to_owned(),
        acting_actors: Vec::new(),
        result: None,
        reason: None,
    }
}

/// Transição que fica na mesma etapa (o atendente ainda não agiu), com um campo já gravado.
fn transicao_parcial(etapa: &str) -> FlowTransition {
    FlowTransition {
        new_state: ExecutionState {
            status: ExecutionStatus::Active,
            no_atual: etapa.to_owned(),
            dados_json: format!("{{\"{etapa}\":{{\"endereco\":\"Rua das Flores, 10\"}}}}"),
            acting_actors: vec![ActionActor::Agent],
            result: None,
            reason: None,
        },
        events: vec![
            TransitionEvent {
                tipo: RecordEventKind::FieldsFilled,
                ator_tipo: EventActor::Agent,
                payload_json: format!(
                    "{{\"etapa\":\"{etapa}\",\"campos\":{{\"endereco\":\"x\"}}}}"
                ),
            },
            TransitionEvent {
                tipo: RecordEventKind::ActionExecuted,
                ator_tipo: EventActor::Agent,
                payload_json: format!(
                    "{{\"etapa\":\"{etapa}\",\"acao\":\"registrar\",\"parcial\":true}}"
                ),
            },
        ],
        effects: vec![TransitionEffect {
            key: format!("acao:{etapa}.registrar/0"),
            tipo: "mensagem_ao_cliente".to_owned(),
            payload_json: "{\"texto\":\"Recebi seu endereço\"}".to_owned(),
        }],
        closes_service: false,
        repeated: false,
        rule_error: false,
    }
}

/// Transição que conclui a etapa e entra na seguinte.
fn transicao_avanca(de: &str, para: &str) -> FlowTransition {
    FlowTransition {
        new_state: ExecutionState {
            status: ExecutionStatus::Active,
            no_atual: para.to_owned(),
            dados_json: format!(
                "{{\"{de}\":{{\"endereco\":\"Rua das Flores, 10\"}},\"{para}\":{{}}}}"
            ),
            acting_actors: Vec::new(),
            result: None,
            reason: None,
        },
        events: vec![
            TransitionEvent {
                tipo: RecordEventKind::ActionExecuted,
                ator_tipo: EventActor::Attendant,
                payload_json: format!("{{\"etapa\":\"{de}\",\"acao\":\"confirmar\"}}"),
            },
            TransitionEvent {
                tipo: RecordEventKind::StepCompleted,
                ator_tipo: EventActor::System,
                payload_json: format!("{{\"etapa\":\"{de}\",\"acao\":\"confirmar\"}}"),
            },
            TransitionEvent {
                tipo: RecordEventKind::StepEntered,
                ator_tipo: EventActor::System,
                payload_json: format!("{{\"etapa\":\"{para}\"}}"),
            },
        ],
        effects: Vec::new(),
        closes_service: false,
        repeated: false,
        rule_error: false,
    }
}

/// Transição que conclui a execução num `fim` que pede o encerramento do atendimento.
fn transicao_conclui(de: &str) -> FlowTransition {
    FlowTransition {
        new_state: ExecutionState {
            status: ExecutionStatus::Completed,
            no_atual: "concluida".to_owned(),
            dados_json: format!("{{\"{de}\":{{\"endereco\":\"Rua das Flores, 10\"}}}}"),
            acting_actors: Vec::new(),
            result: Some("Visita agendada".to_owned()),
            reason: None,
        },
        events: vec![
            TransitionEvent {
                tipo: RecordEventKind::StepCompleted,
                ator_tipo: EventActor::System,
                payload_json: format!("{{\"etapa\":\"{de}\"}}"),
            },
            TransitionEvent {
                tipo: RecordEventKind::ExecutionCompleted,
                ator_tipo: EventActor::System,
                payload_json: "{\"resultado\":\"Visita agendada\"}".to_owned(),
            },
        ],
        effects: Vec::new(),
        closes_service: true,
        repeated: false,
        rule_error: false,
    }
}

fn sem_extras() -> TransitionExtras<'static> {
    TransitionExtras::default()
}

/// Situação de um registro da tabela de Fichas/execuções, lida por SQL.
fn status_da_ficha(cenario: &Cenario, org: &Org, ficha: RecordId) -> RecordStatus {
    let conn = cenario.conexao();
    let texto: String = conn
        .query_row(
            "SELECT status FROM fichas WHERE org_id = ?1 AND id = ?2",
            rusqlite::params![org.id.to_string(), ficha.to_string()],
            |linha| linha.get(0),
        )
        .expect("status da ficha");
    RecordStatus::from_text(&texto).expect("situação conhecida")
}

fn total(cenario: &Cenario, tabela: &str, org: &Org) -> i64 {
    let conn = cenario.conexao();
    conn.query_row(
        &format!("SELECT COUNT(*) FROM {tabela} WHERE org_id = ?1"),
        rusqlite::params![org.id.to_string()],
        |linha| linha.get(0),
    )
    .expect("conta")
}

// ────────────────────────────── Abertura de execução ──────────────────────────────

#[test]
fn iniciar_grava_execucao_primeira_etapa_evento_e_efeitos_de_entrada() {
    let cenario = Cenario::novo("iniciar");
    let org = cenario.org("a");
    let agente = cenario.agente(&org, "a");
    let (_conversa, ficha) = cenario.ficha(&org, &agente);
    let (fluxo, revisao) = cenario.fluxo(&org, "agendar_visita");
    cenario
        .store()
        .fluxos()
        .liberar(org.id, agente.id, fluxo)
        .expect("libera");

    let gravada = cenario
        .store()
        .executions()
        .iniciar_execucao(
            org.id,
            ExecutionStart {
                chave_idempotencia: None,
                ficha,
                fluxo,
                revision: revisao,
                actor: EventActor::Jev,
                ator_ref: Some("jev"),
                jev_decision: Some(poph_store::JevDecision {
                    choice: "agendar_visita",
                    confidence: Some(0.93),
                }),
                initial_state: estado_inicial("coleta"),
                inbound_effects: vec![TransitionEffect {
                    key: "etapa:coleta/0".to_owned(),
                    tipo: "mensagem_ao_cliente".to_owned(),
                    payload_json: "{\"texto\":\"Vamos agendar sua visita\"}".to_owned(),
                }],
            },
        )
        .expect("inicia");

    assert_eq!(gravada.execution.status, ExecutionStatus::Active);
    assert_eq!(gravada.execution.no_atual, "coleta");
    assert_eq!(gravada.execution.ficha_id, ficha);
    assert_eq!(gravada.execution.revisao_id, revisao);
    assert_eq!(gravada.execution.version, 1);
    assert_eq!(gravada.execution.finalizada_em, None);
    assert!(!gravada.closed_service);
    assert!(!gravada.repeated);

    // Um evento só, o de abertura, com a decisão do Jev no detalhe.
    assert_eq!(gravada.events.len(), 1);
    assert_eq!(gravada.events[0].tipo, RecordEventKind::FlowStarted);
    assert_eq!(gravada.events[0].ator_tipo, EventActor::Jev);
    assert_eq!(gravada.events[0].ator_ref.as_deref(), Some("jev"));
    assert!(
        gravada.events[0].payload_json.contains("agendar_visita"),
        "o evento cita o fluxo: {}",
        gravada.events[0].payload_json
    );
    assert!(
        gravada.events[0].payload_json.contains("0.93"),
        "o evento cita a confiança do Jev: {}",
        gravada.events[0].payload_json
    );

    // A primeira entrada da linha do tempo por etapa.
    let etapas = cenario
        .store()
        .executions()
        .etapas(org.id, gravada.execution.id)
        .expect("etapas");
    assert_eq!(etapas.len(), 1);
    assert_eq!(etapas[0].no_id, "coleta");
    assert_eq!(etapas[0].ordem, 1);
    assert_eq!(etapas[0].status, StepStatus::Pending);
    assert_eq!(etapas[0].concluiu_em, None);

    // Efeito de entrada na fila, pendente.
    assert_eq!(gravada.effects.len(), 1);
    let efeitos = cenario
        .store()
        .executions()
        .efeitos(org.id, gravada.execution.id)
        .expect("efeitos");
    assert_eq!(efeitos.len(), 1);
    assert_eq!(efeitos[0].key, "etapa:coleta/0");
    assert_eq!(efeitos[0].estado, EffectState::Pending);
    assert_eq!(efeitos[0].tentativas, 0);
}

// ────────────────────────────── Transição ──────────────────────────────

#[test]
fn transicao_grava_estado_etapa_evento_e_efeito_numa_escrita() {
    let cenario = Cenario::novo("transicao");
    let org = cenario.org("a");
    let agente = cenario.agente(&org, "a");
    let (_conversa, ficha) = cenario.ficha(&org, &agente);
    let (fluxo, revisao) = cenario.fluxo(&org, "agendar_visita");
    let aberta = cenario.abrir(&org, &agente, ficha, fluxo, revisao);

    // 1. Etapa parcial: fica onde está, com campo e ator gravados.
    let parcial = cenario
        .store()
        .executions()
        .aplicar_transicao(
            org.id,
            aberta.execution.id,
            aberta.execution.version,
            &transicao_parcial("coleta"),
            "chave-parcial",
            sem_extras(),
        )
        .expect("aplica a parcial");
    assert_eq!(parcial.execution.no_atual, "coleta");
    assert_eq!(parcial.execution.version, 2);
    assert_eq!(parcial.execution.status, ExecutionStatus::Active);
    assert_eq!(parcial.events.len(), 2);
    assert!(!parcial.repeated);

    let etapas = cenario
        .store()
        .executions()
        .etapas(org.id, aberta.execution.id)
        .expect("etapas");
    assert_eq!(etapas.len(), 1, "não entrou em etapa nova");
    assert_eq!(etapas[0].status, StepStatus::Partial);
    assert_eq!(etapas[0].acting_actors, vec![ActionActor::Agent]);
    assert!(
        etapas[0].dados_json.contains("Rua das Flores"),
        "campos da etapa"
    );

    let efeitos = cenario
        .store()
        .executions()
        .efeitos(org.id, aberta.execution.id)
        .expect("efeitos");
    assert_eq!(efeitos.len(), 1);
    assert_eq!(efeitos[0].key, "acao:coleta.registrar/0");
    assert_eq!(efeitos[0].tipo, "mensagem_ao_cliente");
    assert_eq!(efeitos[0].estado, EffectState::Pending);

    // 2. Avanço: conclui "coleta" e entra em "confirmacao".
    let avancou = cenario
        .store()
        .executions()
        .aplicar_transicao(
            org.id,
            aberta.execution.id,
            parcial.execution.version,
            &transicao_avanca("coleta", "confirmacao"),
            "chave-avanca",
            sem_extras(),
        )
        .expect("aplica o avanço");
    assert_eq!(avancou.execution.no_atual, "confirmacao");
    assert_eq!(avancou.execution.version, 3);

    let etapas = cenario
        .store()
        .executions()
        .etapas(org.id, aberta.execution.id)
        .expect("etapas");
    assert_eq!(etapas.len(), 2, "a etapa nova abre uma entrada");
    assert_eq!(etapas[0].no_id, "coleta");
    assert_eq!(etapas[0].status, StepStatus::Completed);
    assert!(
        etapas[0].concluiu_em.is_some(),
        "etapa concluída tem carimbo"
    );
    assert_eq!(etapas[1].no_id, "confirmacao");
    assert_eq!(etapas[1].ordem, 2);
    assert_eq!(etapas[1].status, StepStatus::Pending);

    // A linha do tempo cresce em ordem e sem buraco de `seq`.
    let eventos = cenario
        .store()
        .fichas()
        .eventos(org.id, ficha, None, 100)
        .expect("eventos");
    let seqs: Vec<i64> = eventos.iter().map(|evento| evento.seq).collect();
    let esperado: Vec<i64> = (1..=seqs.len() as i64).collect();
    assert_eq!(seqs, esperado, "seq contínuo de 1 até o fim");
    assert_eq!(
        eventos
            .iter()
            .filter(|evento| evento.tipo == RecordEventKind::StepCompleted)
            .count(),
        1
    );
    assert_eq!(
        eventos
            .iter()
            .filter(|evento| evento.tipo == RecordEventKind::StepEntered)
            .count(),
        1
    );
}

#[test]
fn falha_no_meio_nao_deixa_evento_sem_estado() {
    let cenario = Cenario::novo("atômica");
    let org = cenario.org("a");
    let agente = cenario.agente(&org, "a");
    let (_conversa, ficha) = cenario.ficha(&org, &agente);
    let (fluxo, revisao) = cenario.fluxo(&org, "agendar_visita");
    let aberta = cenario.abrir(&org, &agente, ficha, fluxo, revisao);

    let eventos_antes = total(&cenario, "ficha_eventos", &org);
    let etapas_antes = total(&cenario, "fluxo_execucao_etapas", &org);
    let efeitos_antes = total(&cenario, "fluxo_efeitos", &org);

    // O `tipo` do efeito tem CHECK na coluna: o insert falha **depois** de o estado e os eventos
    // terem sido gravados na mesma transação, que é justamente o caso a cobrir.
    let mut quebrada = transicao_avanca("coleta", "confirmacao");
    quebrada.effects.push(TransitionEffect {
        key: "acao:coleta.confirmar/0".to_owned(),
        tipo: "tipo_que_a_coluna_recusa".to_owned(),
        payload_json: "{}".to_owned(),
    });
    let erro = cenario
        .store()
        .executions()
        .aplicar_transicao(
            org.id,
            aberta.execution.id,
            aberta.execution.version,
            &quebrada,
            "chave-quebrada",
            sem_extras(),
        )
        .expect_err("a transição falha no efeito");
    // A falha é do `CHECK` da coluna, traduzida pelo store em erro de validação — o que importa
    // aqui é que ela acontece **depois** de o estado e os eventos já terem sido escritos.
    assert!(
        matches!(
            erro,
            poph_store::StoreError::Domain(DomainError::Validation { field, .. })
                if field == "fluxo_efeitos"
        ),
        "erro do CHECK do tipo do efeito: {erro:?}"
    );

    // Nada sobrou: a execução continua na etapa e na versão de antes.
    let depois = cenario
        .store()
        .executions()
        .obter(org.id, aberta.execution.id)
        .expect("lê a execução");
    assert_eq!(depois.version, aberta.execution.version);
    assert_eq!(depois.no_atual, "coleta");
    assert_eq!(depois.status, ExecutionStatus::Active);
    assert_eq!(total(&cenario, "ficha_eventos", &org), eventos_antes);
    assert_eq!(total(&cenario, "fluxo_execucao_etapas", &org), etapas_antes);
    assert_eq!(total(&cenario, "fluxo_efeitos", &org), efeitos_antes);

    // E a chave não ficou marcada: refazer a transição, agora válida, funciona.
    let certa = cenario
        .store()
        .executions()
        .aplicar_transicao(
            org.id,
            aberta.execution.id,
            aberta.execution.version,
            &transicao_avanca("coleta", "confirmacao"),
            "chave-quebrada",
            sem_extras(),
        )
        .expect("a chave não foi marcada pela transição que falhou");
    assert_eq!(certa.execution.no_atual, "confirmacao");
    assert!(!certa.repeated);
}

#[test]
fn versao_velha_recebe_412_e_nao_muda_nada() {
    let cenario = Cenario::novo("versao");
    let org = cenario.org("a");
    let agente = cenario.agente(&org, "a");
    let (_conversa, ficha) = cenario.ficha(&org, &agente);
    let (fluxo, revisao) = cenario.fluxo(&org, "agendar_visita");
    let aberta = cenario.abrir(&org, &agente, ficha, fluxo, revisao);

    let primeira = cenario
        .store()
        .executions()
        .aplicar_transicao(
            org.id,
            aberta.execution.id,
            1,
            &transicao_parcial("coleta"),
            "chave-1",
            sem_extras(),
        )
        .expect("a primeira vence");
    assert_eq!(primeira.execution.version, 2);

    let erro = cenario
        .store()
        .executions()
        .aplicar_transicao(
            org.id,
            aberta.execution.id,
            1,
            &transicao_parcial("coleta"),
            "chave-2",
            sem_extras(),
        )
        .expect_err("versão velha é recusada");
    match erro {
        poph_store::StoreError::Domain(DomainError::VersionConflict {
            expected: esperada,
            current: atual,
            ..
        }) => {
            assert_eq!(esperada, 1);
            assert_eq!(atual, 2);
        }
        outro => panic!("esperado conflito de versão, veio {outro:?}"),
    }
    assert_eq!(
        DomainError::VersionConflict {
            entity: "execução de fluxo",
            id: aberta.execution.id.to_string(),
            expected: 1,
            current: 2,
        }
        .http_status(),
        412
    );

    // Nada mudou com a recusa.
    let depois = cenario
        .store()
        .executions()
        .obter(org.id, aberta.execution.id)
        .expect("lê");
    assert_eq!(depois.version, 2);
    let eventos = cenario
        .store()
        .fichas()
        .eventos(org.id, ficha, None, 100)
        .expect("eventos");
    assert_eq!(
        eventos
            .iter()
            .filter(|evento| evento.tipo == RecordEventKind::ActionExecuted)
            .count(),
        1,
        "a recusa não grava evento"
    );
}

#[test]
fn repetir_a_chave_devolve_o_desfecho_sem_duplicar_evento_nem_efeito() {
    let cenario = Cenario::novo("repetida");
    let org = cenario.org("a");
    let agente = cenario.agente(&org, "a");
    let (_conversa, ficha) = cenario.ficha(&org, &agente);
    let (fluxo, revisao) = cenario.fluxo(&org, "agendar_visita");
    let aberta = cenario.abrir(&org, &agente, ficha, fluxo, revisao);

    let primeira = cenario
        .store()
        .executions()
        .aplicar_transicao(
            org.id,
            aberta.execution.id,
            aberta.execution.version,
            &transicao_parcial("coleta"),
            "chave-repetida",
            sem_extras(),
        )
        .expect("a primeira aplica");
    assert!(!primeira.repeated);
    let eventos_antes = total(&cenario, "ficha_eventos", &org);
    let efeitos_antes = total(&cenario, "fluxo_efeitos", &org);

    // Reenvio com a mesma chave e a versão que o cliente tinha: devolve o desfecho, sem 412.
    let repetida = cenario
        .store()
        .executions()
        .aplicar_transicao(
            org.id,
            aberta.execution.id,
            aberta.execution.version,
            &transicao_parcial("coleta"),
            "chave-repetida",
            sem_extras(),
        )
        .expect("repetir não é erro");
    assert!(repetida.repeated, "marcada como repetida");
    assert_eq!(repetida.execution.version, primeira.execution.version);
    assert!(repetida.events.is_empty());
    assert!(repetida.effects.is_empty());
    assert_eq!(total(&cenario, "ficha_eventos", &org), eventos_antes);
    assert_eq!(total(&cenario, "fluxo_efeitos", &org), efeitos_antes);

    // O motor também sabe que a chave já foi aplicada (`repetida` no resultado dele).
    let do_motor = cenario
        .store()
        .executions()
        .aplicar_transicao(
            org.id,
            aberta.execution.id,
            primeira.execution.version,
            &FlowTransition {
                new_state: estado_inicial("coleta"),
                events: Vec::new(),
                effects: Vec::new(),
                closes_service: false,
                repeated: true,
                rule_error: false,
            },
            "outra-chave",
            sem_extras(),
        )
        .expect("repetida do motor");
    assert!(do_motor.repeated);
    assert_eq!(total(&cenario, "ficha_eventos", &org), eventos_antes);
}

// ────────────────────────── Recusas da abertura ──────────────────────────

#[test]
fn inicio_recusado_com_ficha_fechada() {
    let cenario = Cenario::novo("ficha-fechada");
    let org = cenario.org("a");
    let agente = cenario.agente(&org, "a");
    let (conversa, ficha) = cenario.ficha(&org, &agente);
    let (fluxo, revisao) = cenario.fluxo(&org, "agendar_visita");
    cenario
        .store()
        .fluxos()
        .liberar(org.id, agente.id, fluxo)
        .expect("libera");

    // Fecha a conversa pelo caminho do administrador: a Ficha fecha junto (FX-07).
    cenario
        .store()
        .conversas()
        .fechar_auditado(org.id, conversa.id, conversa.version, "token".to_owned())
        .expect("fecha");
    assert_eq!(status_da_ficha(&cenario, &org, ficha), RecordStatus::Closed);

    let erro = cenario
        .store()
        .executions()
        .iniciar_execucao(
            org.id,
            ExecutionStart {
                chave_idempotencia: None,
                ficha,
                fluxo,
                revision: revisao,
                actor: EventActor::Agent,
                ator_ref: None,
                jev_decision: None,
                initial_state: estado_inicial("coleta"),
                inbound_effects: Vec::new(),
            },
        )
        .expect_err("Ficha fechada recusa execução nova");
    assert!(
        matches!(
            erro,
            poph_store::StoreError::Domain(DomainError::RecordClosed { .. })
        ),
        "veio {erro:?}"
    );
}

#[test]
fn inicio_recusado_com_fluxo_nao_liberado() {
    let cenario = Cenario::novo("nao-liberado");
    let org = cenario.org("a");
    let agente = cenario.agente(&org, "a");
    let (_conversa, ficha) = cenario.ficha(&org, &agente);
    let (fluxo, revisao) = cenario.fluxo(&org, "agendar_visita");

    let erro = cenario
        .store()
        .executions()
        .iniciar_execucao(
            org.id,
            ExecutionStart {
                chave_idempotencia: None,
                ficha,
                fluxo,
                revision: revisao,
                actor: EventActor::Agent,
                ator_ref: None,
                jev_decision: None,
                initial_state: estado_inicial("coleta"),
                inbound_effects: Vec::new(),
            },
        )
        .expect_err("sem permissão não abre");
    match erro {
        poph_store::StoreError::Domain(DomainError::FlowNotAllowed { fluxo_id, agent_id }) => {
            assert_eq!(fluxo_id, fluxo.to_string());
            assert_eq!(agent_id, agente.id.to_string());
        }
        outro => panic!("esperado fluxo não liberado, veio {outro:?}"),
    }
}

#[test]
fn inicio_recusado_com_fluxo_inativo() {
    let cenario = Cenario::novo("inativo");
    let org = cenario.org("a");
    let agente = cenario.agente(&org, "a");
    let (_conversa, ficha) = cenario.ficha(&org, &agente);
    let (fluxo, revisao) = cenario.fluxo(&org, "agendar_visita");
    cenario
        .store()
        .fluxos()
        .liberar(org.id, agente.id, fluxo)
        .expect("libera");
    let conn = cenario.conexao();
    conn.execute(
        "UPDATE fluxos SET status = 'inactive' WHERE org_id = ?1 AND id = ?2",
        rusqlite::params![org.id.to_string(), fluxo.to_string()],
    )
    .expect("inativa o fluxo");

    let erro = cenario
        .store()
        .executions()
        .iniciar_execucao(
            org.id,
            ExecutionStart {
                chave_idempotencia: None,
                ficha,
                fluxo,
                revision: revisao,
                actor: EventActor::Agent,
                ator_ref: None,
                jev_decision: None,
                initial_state: estado_inicial("coleta"),
                inbound_effects: Vec::new(),
            },
        )
        .expect_err("fluxo inativo não abre");
    match erro {
        poph_store::StoreError::Domain(DomainError::Conflict {
            entity: entidade,
            reason,
            ..
        }) => {
            assert_eq!(entidade, "fluxo");
            assert!(reason.contains("inativo"), "motivo: {reason}");
        }
        outro => panic!("esperado conflito de fluxo inativo, veio {outro:?}"),
    }
}

#[test]
fn inicio_recusado_com_segunda_execucao_ativa() {
    let cenario = Cenario::novo("segunda");
    let org = cenario.org("a");
    let agente = cenario.agente(&org, "a");
    let (_conversa, ficha) = cenario.ficha(&org, &agente);
    let (fluxo, revisao) = cenario.fluxo(&org, "agendar_visita");
    let aberta = cenario.abrir(&org, &agente, ficha, fluxo, revisao);

    let erro = cenario
        .store()
        .executions()
        .iniciar_execucao(
            org.id,
            ExecutionStart {
                chave_idempotencia: None,
                ficha,
                fluxo,
                revision: revisao,
                actor: EventActor::Agent,
                ator_ref: None,
                jev_decision: None,
                initial_state: estado_inicial("coleta"),
                inbound_effects: Vec::new(),
            },
        )
        .expect_err("uma execução ativa por Ficha (PRD L6)");
    match erro {
        poph_store::StoreError::Domain(DomainError::ExecutionAlreadyActive {
            ficha_id,
            execucao_id,
        }) => {
            assert_eq!(ficha_id, ficha.to_string());
            assert_eq!(execucao_id, aberta.execution.id.to_string());
        }
        outro => panic!("esperado execução já ativa, veio {outro:?}"),
    }
    assert_eq!(
        cenario
            .store()
            .executions()
            .da_ficha(org.id, ficha, 10)
            .expect("histórico")
            .len(),
        1
    );
}

// ────────────────────────────── Fim e encerramento ──────────────────────────────

#[test]
fn fim_com_encerra_atendimento_fecha_conversa_e_ficha_na_mesma_escrita() {
    let cenario = Cenario::novo("encerra");
    let org = cenario.org("a");
    let agente = cenario.agente(&org, "a");
    let (conversa, ficha) = cenario.ficha(&org, &agente);
    let (fluxo, revisao) = cenario.fluxo(&org, "agendar_visita");
    let aberta = cenario.abrir(&org, &agente, ficha, fluxo, revisao);

    let gravada = cenario
        .store()
        .executions()
        .aplicar_transicao(
            org.id,
            aberta.execution.id,
            aberta.execution.version,
            &transicao_conclui("coleta"),
            "chave-fim",
            TransitionExtras {
                ator_ref: Some("atendente-1"),
                ..TransitionExtras::default()
            },
        )
        .expect("aplica o fim");

    assert_eq!(gravada.execution.status, ExecutionStatus::Completed);
    assert_eq!(gravada.execution.no_atual, "concluida");
    assert_eq!(gravada.execution.result.as_deref(), Some("Visita agendada"));
    assert!(gravada.execution.finalizada_em.is_some());
    assert!(gravada.closed_service);

    // Ficha fechada, com o autor e o motivo do fim.
    let ficha_gravada = cenario
        .store()
        .fichas()
        .obter(org.id, ficha)
        .expect("lê a Ficha");
    assert_eq!(ficha_gravada.status, RecordStatus::Closed);
    assert_eq!(ficha_gravada.fechada_por, Some(ClosedBy::Flow));
    assert_eq!(
        ficha_gravada.fechada_motivo.as_deref(),
        Some("Visita agendada")
    );

    // Conversa fechada na mesma transação.
    let fechada = cenario
        .store()
        .conversas()
        .get(org.id, conversa.id)
        .expect("lê a conversa");
    assert_eq!(
        fechada.status,
        poph_store::ConversationStatus::Closed,
        "a conversa fecha junto"
    );

    // A linha do tempo termina com a execução concluída e o fechamento da Ficha.
    let eventos = cenario
        .store()
        .fichas()
        .eventos(org.id, ficha, None, 100)
        .expect("eventos");
    let tipos: Vec<RecordEventKind> = eventos.iter().map(|evento| evento.tipo).collect();
    assert!(tipos.contains(&RecordEventKind::ExecutionCompleted));
    assert_eq!(
        tipos.last().copied(),
        Some(RecordEventKind::RecordClosed),
        "o fechamento é o último evento"
    );
    let fechamento = eventos.last().expect("último evento");
    assert_eq!(fechamento.tipo, RecordEventKind::RecordClosed);
    assert!(fechamento.payload_json.contains("Visita agendada"));
}

#[test]
fn chave_de_idempotencia_vazia_e_recusada() {
    let cenario = Cenario::novo("chave-vazia");
    let org = cenario.org("a");
    let agente = cenario.agente(&org, "a");
    let (_conversa, ficha) = cenario.ficha(&org, &agente);
    let (fluxo, revisao) = cenario.fluxo(&org, "agendar_visita");
    let aberta = cenario.abrir(&org, &agente, ficha, fluxo, revisao);

    // Sem chave, duas transições diferentes cairiam no mesmo marcador e a segunda seria lida como
    // repetição: o store recusa antes de gravar.
    for vazia in ["", "   "] {
        let erro = cenario
            .store()
            .executions()
            .aplicar_transicao(
                org.id,
                aberta.execution.id,
                aberta.execution.version,
                &transicao_parcial("coleta"),
                vazia,
                sem_extras(),
            )
            .expect_err("chave vazia é recusada");
        assert!(
            matches!(
                erro,
                poph_store::StoreError::Domain(DomainError::Validation {
                    field: "chave_idempotencia",
                    ..
                })
            ),
            "veio {erro:?}"
        );
    }
    assert_eq!(
        cenario
            .store()
            .executions()
            .obter(org.id, aberta.execution.id)
            .expect("lê")
            .version,
        1
    );
}

#[test]
fn transicao_incoerente_que_encerra_sem_concluir_e_recusada() {
    let cenario = Cenario::novo("incoerente");
    let org = cenario.org("a");
    let agente = cenario.agente(&org, "a");
    let (_conversa, ficha) = cenario.ficha(&org, &agente);
    let (fluxo, revisao) = cenario.fluxo(&org, "agendar_visita");
    let aberta = cenario.abrir(&org, &agente, ficha, fluxo, revisao);

    // Pedir o encerramento do atendimento sem concluir a execução num `fim` é defeito de quem
    // chamou: o store recusa antes de gravar, em vez de fechar a Ficha com o fluxo pela metade.
    let incoerente = FlowTransition {
        closes_service: true,
        ..transicao_parcial("coleta")
    };
    let erro = cenario
        .store()
        .executions()
        .aplicar_transicao(
            org.id,
            aberta.execution.id,
            aberta.execution.version,
            &incoerente,
            "chave-incoerente",
            sem_extras(),
        )
        .expect_err("encerrar sem concluir é recusado");
    assert!(
        matches!(
            erro,
            poph_store::StoreError::Domain(DomainError::Validation {
                field: "encerra_atendimento",
                ..
            })
        ),
        "veio {erro:?}"
    );
    assert_eq!(status_da_ficha(&cenario, &org, ficha), RecordStatus::Open);
    assert_eq!(
        cenario
            .store()
            .executions()
            .obter(org.id, aberta.execution.id)
            .expect("lê")
            .version,
        1
    );
}

#[test]
fn execucao_encerrada_nao_aceita_transicao() {
    let cenario = Cenario::novo("encerrada");
    let org = cenario.org("a");
    let agente = cenario.agente(&org, "a");
    let (_conversa, ficha) = cenario.ficha(&org, &agente);
    let (fluxo, revisao) = cenario.fluxo(&org, "agendar_visita");
    let aberta = cenario.abrir(&org, &agente, ficha, fluxo, revisao);

    // Fim que NÃO encerra o atendimento: a execução conclui e a Ficha continua aberta.
    let conclui = cenario
        .store()
        .executions()
        .aplicar_transicao(
            org.id,
            aberta.execution.id,
            aberta.execution.version,
            &FlowTransition {
                closes_service: false,
                ..transicao_conclui("coleta")
            },
            "chave-conclui",
            sem_extras(),
        )
        .expect("conclui sem encerrar");
    assert_eq!(conclui.execution.status, ExecutionStatus::Completed);
    assert!(!conclui.closed_service);
    assert_eq!(status_da_ficha(&cenario, &org, ficha), RecordStatus::Open);

    let erro = cenario
        .store()
        .executions()
        .aplicar_transicao(
            org.id,
            aberta.execution.id,
            conclui.execution.version,
            &transicao_parcial("coleta"),
            "chave-depois",
            sem_extras(),
        )
        .expect_err("execução concluída não aceita transição");
    match erro {
        poph_store::StoreError::Domain(DomainError::Conflict {
            entity: entidade,
            reason,
            ..
        }) => {
            assert_eq!(entidade, "execução de fluxo");
            assert!(reason.contains("concluida"), "motivo: {reason}");
        }
        outro => panic!("esperado conflito, veio {outro:?}"),
    }

    // Depois de concluída, a Ficha aceita um fluxo novo (o histórico se acumula).
    let (outro_fluxo, outra_revisao) = cenario.fluxo(&org, "confirmar_entrega");
    cenario
        .store()
        .fluxos()
        .liberar(org.id, agente.id, outro_fluxo)
        .expect("libera o segundo");
    let segunda = cenario
        .store()
        .executions()
        .iniciar_execucao(
            org.id,
            ExecutionStart {
                chave_idempotencia: None,
                ficha,
                fluxo: outro_fluxo,
                revision: outra_revisao,
                actor: EventActor::Agent,
                ator_ref: None,
                jev_decision: None,
                initial_state: estado_inicial("entrega"),
                inbound_effects: Vec::new(),
            },
        )
        .expect("fluxo novo na mesma Ficha");
    let historico = cenario
        .store()
        .executions()
        .da_ficha(org.id, ficha, 10)
        .expect("histórico");
    assert_eq!(historico.len(), 2);
    assert_eq!(historico[0].id, aberta.execution.id);
    assert_eq!(historico[1].id, segunda.execution.id);
}

// ────────────────────────────── Concorrência e isolamento ──────────────────────────────

#[test]
fn duas_threads_agente_e_atendente_so_uma_vence() {
    let cenario = Cenario::novo("threads");
    let org = cenario.org("a");
    let agente = cenario.agente(&org, "a");
    let (_conversa, ficha) = cenario.ficha(&org, &agente);
    let (fluxo, revisao) = cenario.fluxo(&org, "agendar_visita");
    let aberta = cenario.abrir(&org, &agente, ficha, fluxo, revisao);

    let store: Arc<Store> = cenario.compartilhado();
    let barreira = Arc::new(Barrier::new(2));
    let esperada = aberta.execution.version;
    let id = aberta.execution.id;

    let mut resultados = Vec::new();
    for (rotulo, ator) in [
        ("agente", EventActor::Agent),
        ("atendente", EventActor::Attendant),
    ] {
        let store = Arc::clone(&store);
        let barreira = Arc::clone(&barreira);
        let mut transicao = transicao_parcial("coleta");
        transicao.events[0].ator_tipo = ator;
        transicao.new_state.acting_actors = vec![match ator {
            EventActor::Agent => ActionActor::Agent,
            _ => ActionActor::Attendant,
        }];
        let chave = format!("chave-{rotulo}");
        resultados.push(std::thread::spawn(move || {
            barreira.wait();
            store.executions().aplicar_transicao(
                org.id,
                id,
                esperada,
                &transicao,
                &chave,
                TransitionExtras::default(),
            )
        }));
    }
    let saidas: Vec<_> = resultados
        .into_iter()
        .map(|thread| thread.join().expect("a thread não entra em pânico"))
        .collect();

    let vencedoras = saidas.iter().filter(|saida| saida.is_ok()).count();
    let perdedoras = saidas
        .iter()
        .filter(|saida| {
            matches!(
                saida,
                Err(poph_store::StoreError::Domain(
                    DomainError::VersionConflict { .. }
                ))
            )
        })
        .count();
    assert_eq!(vencedoras, 1, "só uma transição vence: {saidas:?}");
    assert_eq!(perdedoras, 1, "a outra recebe 412: {saidas:?}");

    // Estado consistente: uma versão a mais e um evento de ação a mais, sem duplicar.
    let final_ = store.executions().obter(org.id, id).expect("lê");
    assert_eq!(final_.version, aberta.execution.version + 1);
    let eventos = store
        .fichas()
        .eventos(org.id, ficha, None, 100)
        .expect("eventos");
    assert_eq!(
        eventos
            .iter()
            .filter(|evento| evento.tipo == RecordEventKind::ActionExecuted)
            .count(),
        1
    );
    assert_eq!(
        total(&cenario, "fluxo_efeitos", &org),
        1,
        "o efeito da transição vencedora entrou uma vez só"
    );
}

#[test]
fn execucao_de_outra_org_nao_e_vista_nem_escrita() {
    let cenario = Cenario::novo("isolamento");
    let org_a = cenario.org("a");
    let org_b = cenario.org("b");
    let agente_a = cenario.agente(&org_a, "a");
    let (_conversa, ficha) = cenario.ficha(&org_a, &agente_a);
    let (fluxo, revisao) = cenario.fluxo(&org_a, "agendar_visita");
    let aberta = cenario.abrir(&org_a, &agente_a, ficha, fluxo, revisao);

    let erro = cenario
        .store()
        .executions()
        .obter(org_b.id, aberta.execution.id)
        .expect_err("outra org não vê");
    assert!(
        matches!(
            erro,
            poph_store::StoreError::Domain(DomainError::NotFound { .. })
        ),
        "veio {erro:?}"
    );
    assert!(
        cenario
            .store()
            .executions()
            .da_ficha(org_b.id, ficha, 10)
            .expect("lista vazia")
            .is_empty()
    );
    assert!(
        cenario
            .store()
            .executions()
            .ativa_da_ficha(org_b.id, ficha)
            .expect("nada")
            .is_none()
    );
    let erro = cenario
        .store()
        .executions()
        .aplicar_transicao(
            org_b.id,
            aberta.execution.id,
            aberta.execution.version,
            &transicao_parcial("coleta"),
            "chave-b",
            sem_extras(),
        )
        .expect_err("outra org não escreve");
    assert!(
        matches!(
            erro,
            poph_store::StoreError::Domain(DomainError::NotFound { .. })
        ),
        "veio {erro:?}"
    );

    // A execução da org A segue intacta.
    let depois = cenario
        .store()
        .executions()
        .obter(org_a.id, aberta.execution.id)
        .expect("lê");
    assert_eq!(depois.version, 1);
    assert_eq!(depois.no_atual, "coleta");
    assert_eq!(total(&cenario, "fluxo_efeitos", &org_a), 0);
    assert_eq!(total(&cenario, "fluxo_efeitos", &org_b), 0);
}
