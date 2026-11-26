use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use poph_store::{
    Agente, Conversa, ConversationStatus, NewAgent, NewChannelInboxItem, NewConversation, NewOrg,
    NewRecord, NewRecordEvent, Org, Page, REASON_ABANDONMENT_SERVICE, Received, Store,
};
use poph_types::{
    AgentId, ClosedBy, ConversationId, DomainError, EventActor, OrgId, RecordEventKind, RecordId,
    RecordStatus,
};
use rusqlite::Connection;

struct Scenario {
    folder: PathBuf,
    store: Option<Store>,
}

impl Scenario {
    fn new(label: &str) -> Self {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let folder =
            std::env::temp_dir().join(format!("poph-fichas-{label}-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&folder);
        fs::create_dir_all(&folder).expect("cria a pasta do teste");
        let store = Store::open_at(&folder.join("harness.db"), &folder.join("rag.db"))
            .expect("abre o banco");
        Self {
            folder,
            store: Some(store),
        }
    }

    fn store(&self) -> &Store {
        self.store.as_ref().expect("store viva")
    }

    fn connection(&self) -> Connection {
        let conn = Connection::open(self.folder.join("harness.db")).expect("abre para conferir");
        conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA busy_timeout = 5000;")
            .expect("pragmas");
        conn
    }

    fn org(&self, slug: &str) -> Org {
        self.store()
            .orgs()
            .create(NewOrg {
                name: format!("Org {slug}"),
                slug: slug.to_owned(),
                config_json: String::new(),
            })
            .expect("cria a org")
    }

    fn agent(&self, org: &Org, slug: &str) -> Agente {
        self.store()
            .agents()
            .create(
                org.id,
                NewAgent {
                    name: format!("Agente {slug}"),
                    slug: slug.to_owned(),
                    ..NewAgent::default()
                },
            )
            .expect("cria o agente")
    }

    fn conversation(&self, org: &Org, agent: &Agente) -> Conversa {
        self.store()
            .conversations()
            .create(
                org.id,
                NewConversation {
                    agent_id: agent.id,
                    integration_id: None,
                    canal: "console".to_owned(),
                    peer_externo: Some("cliente-1".to_owned()),
                    target_department: None,
                },
            )
            .expect("abre a conversa")
    }

    fn close(&self, org: &Org, conversation: &Conversa) -> Conversa {
        self.store()
            .conversations()
            .close_audited(
                org.id,
                conversation.id,
                conversation.version,
                "token-de-teste".to_owned(),
            )
            .expect("fecha a conversa")
    }

    fn execution_active(&self, org: &Org, case: RecordId, now: i64) -> String {
        let conn = self.connection();
        let flow = uuid::Uuid::now_v7().to_string();
        let revision = uuid::Uuid::now_v7().to_string();
        let execution = uuid::Uuid::now_v7().to_string();
        conn.execute(
            "INSERT INTO fluxos (id, org_id, slug, nome, descricao, status, rascunho_fonte,
                                 revisao_atual, version)
             VALUES (?1, ?2, 'agendar_visita', 'Agendar visita', '', 'active', '', 1, 1)",
            rusqlite::params![flow, org.id.to_string()],
        )
        .expect("fluxo");
        conn.execute(
            "INSERT INTO fluxo_revisoes (id, org_id, fluxo_id, numero, fonte, fonte_sha256,
                                        definicao_json, schema_versao, publicada_em, publicada_por,
                                        resolucoes_json)
             VALUES (?1, ?2, ?3, 1, '', 'sha', '{}', 1, ?4, 'admin_token', '{}')",
            rusqlite::params![revision, org.id.to_string(), flow, now],
        )
        .expect("revisão");
        conn.execute(
            "INSERT INTO fluxo_execucoes (id, org_id, ficha_id, fluxo_id, revisao_id, status,
                                         no_atual, dados_json, resultado, motivo_fim, iniciada_em,
                                         finalizada_em, version)
             VALUES (?1, ?2, ?3, ?4, ?5, 'ativa', 'pedir_endereco', '{}', NULL, NULL, ?6, NULL, 1)",
            rusqlite::params![
                execution,
                org.id.to_string(),
                case.to_string(),
                flow,
                revision,
                now
            ],
        )
        .expect("execução");
        execution
    }

    fn integration(&self, org: &Org, agent: &Agente) -> poph_types::IntegrationId {
        self.store()
            .create_integration(org.id, agent.id, "whatsapp".into(), "active".into())
            .expect("integração")
            .id
            .parse()
            .expect("id da integração")
    }
}

impl Drop for Scenario {
    fn drop(&mut self) {
        let _ = self.store.take();
        let _ = fs::remove_dir_all(&self.folder);
    }
}

fn entry(
    integration: poph_types::IntegrationId,
    agent: AgentId,
    external_id: &str,
) -> NewChannelInboxItem {
    NewChannelInboxItem {
        integration_id: integration,
        agent_id: agent,
        canal: "whatsapp".to_owned(),
        external_id: external_id.to_owned(),
        peer: "5521999990000".to_owned(),
        event_json: "{}".to_owned(),
        texto: "oi".to_owned(),
        attachments: Vec::new(),
        customer_name: None,
    }
}

#[test]
fn creating_conversation_creates_case_in_same_write() {
    let scenario = Scenario::new("criar");
    let org = scenario.org("a");
    let agent = scenario.agent(&org, "a");
    let conversation = scenario.conversation(&org, &agent);

    let case = scenario
        .store()
        .cases()
        .by_conversation(org.id, conversation.id)
        .expect("consulta")
        .expect("a conversa nova tem Ficha");

    assert_eq!(case.status, RecordStatus::Open);
    assert_eq!(case.agent_id, agent.id);
    assert_eq!(case.canal, "console");
    assert_eq!(case.peer_externo.as_deref(), Some("cliente-1"));
    assert_eq!(case.aberta_em, conversation.created_at);
    assert_eq!(case.fechada_em, None);
    assert_eq!(case.fechada_por, None);
    assert_eq!(case.version, 1);

    let events = scenario
        .store()
        .cases()
        .events(org.id, case.id, None, 50)
        .expect("eventos");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].tipo, RecordEventKind::RecordOpened);
    assert_eq!(events[0].ator_tipo, EventActor::System);
    assert_eq!(events[0].seq, 1);
}

#[test]
fn channel_reception_creates_one_case_per_conversation_and_not_per_message() {
    let scenario = Scenario::new("canal");
    let org = scenario.org("a");
    let agent = scenario.agent(&org, "a");
    let integration = scenario.integration(&org, &agent);

    let first = scenario
        .store()
        .inbox()
        .receive_from_channel(org.id, entry(integration, agent.id, "MSG-1"))
        .expect("recebe a primeira");
    assert!(matches!(first.situation, Received::New));
    let conversation = first.mensagem.conversation_id;

    let second = scenario
        .store()
        .inbox()
        .receive_from_channel(org.id, entry(integration, agent.id, "MSG-2"))
        .expect("recebe a segunda");
    assert_eq!(
        second.mensagem.conversation_id, conversation,
        "a segunda mensagem entra na mesma conversa"
    );

    let case = scenario
        .store()
        .cases()
        .by_conversation(org.id, conversation)
        .expect("consulta")
        .expect("a conversa do canal tem Ficha");
    assert_eq!(case.canal, "whatsapp");
    assert_eq!(case.peer_externo.as_deref(), Some("5521999990000"));
    assert_eq!(case.agent_id, agent.id);

    let events = scenario
        .store()
        .cases()
        .events(org.id, case.id, None, 50)
        .expect("eventos");
    assert_eq!(
        events
            .iter()
            .filter(|evento| evento.tipo == RecordEventKind::RecordOpened)
            .count(),
        1,
        "uma Ficha tem exatamente um evento de abertura"
    );

    let every = scenario
        .store()
        .cases()
        .list(org.id, &Default::default(), Page::default())
        .expect("lista");
    assert_eq!(every.len(), 1);
}

#[test]
fn conversation_from_repository_and_from_channel_coexist_each_with_its_case() {
    let scenario = Scenario::new("dois-caminhos");
    let org = scenario.org("a");
    let agent = scenario.agent(&org, "a");
    let integration = scenario.integration(&org, &agent);

    let do_repositorio = scenario.conversation(&org, &agent);
    let from_channel = scenario
        .store()
        .inbox()
        .receive_from_channel(org.id, entry(integration, agent.id, "MSG-1"))
        .expect("recebe")
        .mensagem
        .conversation_id;

    for conversation in [do_repositorio.id, from_channel] {
        assert!(
            scenario
                .store()
                .cases()
                .by_conversation(org.id, conversation)
                .expect("consulta")
                .is_some(),
            "toda conversa criada pelo sistema tem Ficha"
        );
    }
    assert_eq!(
        scenario
            .store()
            .cases()
            .list(org.id, &Default::default(), Page::default())
            .expect("lista")
            .len(),
        2
    );
}

#[test]
fn closing_conversation_closes_case_and_records_event() {
    let scenario = Scenario::new("fechar");
    let org = scenario.org("a");
    let agent = scenario.agent(&org, "a");
    let conversation = scenario.conversation(&org, &agent);
    let case = scenario
        .store()
        .cases()
        .by_conversation(org.id, conversation.id)
        .expect("consulta")
        .expect("Ficha");

    let closed = scenario.close(&org, &conversation);
    assert_eq!(closed.status, ConversationStatus::Closed);

    let later = scenario
        .store()
        .cases()
        .get(org.id, case.id)
        .expect("Ficha fechada continua legível");
    assert_eq!(later.status, RecordStatus::Closed);
    assert_eq!(later.fechada_por, Some(ClosedBy::Admin));
    assert_eq!(later.fechada_em, Some(closed.updated_at));
    assert!(later.fechada_motivo.is_some());
    assert_eq!(later.version, 2, "fechar a Ficha sobe a versão dela");

    let events = scenario
        .store()
        .cases()
        .events(org.id, case.id, None, 50)
        .expect("eventos");
    let last_value = events.last().expect("tem evento de fechamento");
    assert_eq!(last_value.tipo, RecordEventKind::RecordClosed);
    assert_eq!(last_value.ator_tipo, EventActor::Admin);
    assert_eq!(
        last_value.seq, 2,
        "o fechamento é o segundo evento da linha do tempo"
    );
}

#[test]
fn changing_status_to_closed_also_closes_case() {
    let scenario = Scenario::new("mudar-status");
    let org = scenario.org("a");
    let agent = scenario.agent(&org, "a");
    let conversation = scenario.conversation(&org, &agent);

    scenario
        .store()
        .conversations()
        .change_status(
            org.id,
            conversation.id,
            conversation.version,
            ConversationStatus::Closed,
            None,
        )
        .expect("fecha pela mudança de situação");

    let case = scenario
        .store()
        .cases()
        .by_conversation(org.id, conversation.id)
        .expect("consulta")
        .expect("Ficha");
    assert_eq!(case.status, RecordStatus::Closed);
    assert_eq!(case.fechada_por, Some(ClosedBy::System));
}

#[test]
fn closing_with_active_execution_abandons_execution_in_same_write() {
    let scenario = Scenario::new("abandona");
    let org = scenario.org("a");
    let agent = scenario.agent(&org, "a");
    let conversation = scenario.conversation(&org, &agent);
    let case = scenario
        .store()
        .cases()
        .by_conversation(org.id, conversation.id)
        .expect("consulta")
        .expect("Ficha");
    let execution = scenario.execution_active(&org, case.id, scenario.store().now());

    scenario.close(&org, &conversation);

    let conn = scenario.connection();
    let (status, motivo, finalised): (String, Option<String>, Option<i64>) = conn
        .query_row(
            "SELECT status, motivo_fim, finalizada_em FROM fluxo_execucoes WHERE id = ?1",
            rusqlite::params![execution],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("execução");
    assert_eq!(status, "abandonada");
    assert_eq!(motivo.as_deref(), Some(REASON_ABANDONMENT_SERVICE));
    assert!(
        finalised.is_some(),
        "execução encerrada tem instante de fim"
    );

    let events = scenario
        .store()
        .cases()
        .events(org.id, case.id, None, 50)
        .expect("eventos");
    let abandonos: Vec<_> = events
        .iter()
        .filter(|evento| evento.tipo == RecordEventKind::ExecutionAbandoned)
        .collect();
    assert_eq!(abandonos.len(), 1);
    assert_eq!(
        abandonos[0].execucao_id.map(|id| id.to_string()),
        Some(execution)
    );
    assert_eq!(
        events.last().map(|evento| evento.tipo),
        Some(RecordEventKind::RecordClosed),
        "o abandono vem antes do fechamento na linha do tempo"
    );
}

#[test]
fn failure_midway_through_closing_leaves_no_closed_conversation_with_open_case() {
    let scenario = Scenario::new("atomico");
    let org = scenario.org("a");
    let agent = scenario.agent(&org, "a");
    let conversation = scenario.conversation(&org, &agent);
    let case = scenario
        .store()
        .cases()
        .by_conversation(org.id, conversation.id)
        .expect("consulta")
        .expect("Ficha");

    {
        let conn = scenario.connection();
        conn.execute_batch(
            "CREATE TRIGGER falha_injetada BEFORE INSERT ON ficha_eventos
             WHEN NEW.tipo = 'ficha_fechada'
             BEGIN SELECT RAISE(ABORT, 'falha injetada no teste'); END;",
        )
        .expect("cria o gatilho da falha");
    }

    let error = scenario
        .store()
        .conversations()
        .close_audited(
            org.id,
            conversation.id,
            conversation.version,
            "token".to_owned(),
        )
        .expect_err("o fechamento falha no meio");
    assert!(
        format!("{error}").contains("falha injetada"),
        "o erro do gatilho aparece: {error}"
    );

    scenario
        .connection()
        .execute_batch("DROP TRIGGER falha_injetada;")
        .expect("remove o gatilho");

    let conversation_after = scenario
        .store()
        .conversations()
        .get(org.id, conversation.id)
        .expect("lê a conversa");
    assert_eq!(
        conversation_after.status,
        ConversationStatus::Open,
        "a conversa não pode ficar fechada se a Ficha não fechou"
    );
    assert_eq!(
        conversation_after.version, conversation.version,
        "nem a versão subiu"
    );

    let case_after = scenario
        .store()
        .cases()
        .get(org.id, case.id)
        .expect("lê a Ficha");
    assert_eq!(case_after.status, RecordStatus::Open);
    let events = scenario
        .store()
        .cases()
        .events(org.id, case.id, None, 50)
        .expect("eventos");
    assert!(
        events
            .iter()
            .all(|evento| evento.tipo != RecordEventKind::RecordClosed),
        "nenhum evento parcial do fechamento sobrou"
    );
}

#[test]
fn message_after_closing_opens_new_conversation_and_case() {
    let scenario = Scenario::new("ciclo");
    let org = scenario.org("a");
    let agent = scenario.agent(&org, "a");
    let integration = scenario.integration(&org, &agent);

    let first = scenario
        .store()
        .inbox()
        .receive_from_channel(org.id, entry(integration, agent.id, "MSG-1"))
        .expect("recebe");
    let conversation_1 = first.mensagem.conversation_id;
    let case_1 = scenario
        .store()
        .cases()
        .by_conversation(org.id, conversation_1)
        .expect("consulta")
        .expect("Ficha da primeira");

    let version = scenario
        .store()
        .conversations()
        .get(org.id, conversation_1)
        .expect("conversa")
        .version;
    scenario
        .store()
        .conversations()
        .close_audited(org.id, conversation_1, version, "token".to_owned())
        .expect("fecha a primeira");

    let second = scenario
        .store()
        .inbox()
        .receive_from_channel(org.id, entry(integration, agent.id, "MSG-2"))
        .expect("recebe depois do fechamento");
    let conversation_2 = second.mensagem.conversation_id;
    assert_ne!(
        conversation_2, conversation_1,
        "mensagem nova abre conversa nova"
    );

    let case_2 = scenario
        .store()
        .cases()
        .by_conversation(org.id, conversation_2)
        .expect("consulta")
        .expect("a conversa nova tem Ficha");
    assert_ne!(case_2.id, case_1.id);
    assert_eq!(case_2.status, RecordStatus::Open);

    let case_1_after = scenario
        .store()
        .cases()
        .get(org.id, case_1.id)
        .expect("lê a primeira");
    assert_eq!(case_1_after.status, RecordStatus::Closed);

    let error = scenario
        .store()
        .cases()
        .register_event(
            org.id,
            case_1.id,
            NewRecordEvent {
                execucao_id: None,
                tipo: RecordEventKind::StepEntered,
                ator_tipo: EventActor::Agent,
                ator_ref: None,
                payload_json: "{}".to_owned(),
                chave_idempotencia: None,
            },
        )
        .expect_err("Ficha fechada não recebe evento");
    assert!(matches!(
        error,
        poph_store::StoreError::Domain(DomainError::RecordClosed { .. })
    ));
}

#[test]
fn reopening_closed_conversation_is_refused_and_transferred_keeps_reopening() {
    let scenario = Scenario::new("reabrir");
    let org = scenario.org("a");
    let agent = scenario.agent(&org, "a");

    let transferred = scenario.conversation(&org, &agent);
    scenario
        .store()
        .conversations()
        .change_status(
            org.id,
            transferred.id,
            transferred.version,
            ConversationStatus::Transferred,
            None,
        )
        .expect("transfere");
    let version = scenario
        .store()
        .conversations()
        .get(org.id, transferred.id)
        .expect("conversa")
        .version;
    let reaberta = scenario
        .store()
        .conversations()
        .reopen_audited(org.id, transferred.id, version, "token".to_owned())
        .expect("conversa transferida reabre");
    assert_eq!(reaberta.status, ConversationStatus::Open);

    let closed = scenario.conversation(&org, &agent);
    let after_closing = scenario.close(&org, &closed);
    let error = scenario
        .store()
        .conversations()
        .reopen_audited(org.id, closed.id, after_closing.version, "token".to_owned())
        .expect_err("conversa encerrada não reabre");
    assert!(
        matches!(
            error,
            poph_store::StoreError::Domain(DomainError::ConversationClosed { .. })
        ),
        "veio {error:?}"
    );

    let error = scenario
        .store()
        .conversations()
        .change_status(
            org.id,
            closed.id,
            after_closing.version,
            ConversationStatus::Open,
            None,
        )
        .expect_err("mudar_status não reabre conversa encerrada");
    assert!(matches!(
        error,
        poph_store::StoreError::Domain(DomainError::ConversationClosed { .. })
    ));

    let conversation_after = scenario
        .store()
        .conversations()
        .get(org.id, closed.id)
        .expect("conversa");
    assert_eq!(conversation_after.status, ConversationStatus::Closed);
    assert_eq!(conversation_after.version, after_closing.version);
    let case = scenario
        .store()
        .cases()
        .by_conversation(org.id, closed.id)
        .expect("consulta")
        .expect("Ficha");
    assert_eq!(case.status, RecordStatus::Closed);
}

#[test]
fn reopening_conversation_with_old_version_returns_version_conflict() {
    let scenario = Scenario::new("reabrir-versao");
    let org = scenario.org("a");
    let agent = scenario.agent(&org, "a");
    let conversation = scenario.conversation(&org, &agent);

    let error = scenario
        .store()
        .conversations()
        .reopen_audited(
            org.id,
            conversation.id,
            conversation.version + 7,
            "token".to_owned(),
        )
        .expect_err("versão velha não reabre");
    assert!(matches!(
        error,
        poph_store::StoreError::Domain(DomainError::VersionConflict { .. })
    ));
}

#[test]
fn list_filters_by_situation_agent_channel_and_instant() {
    let scenario = Scenario::new("lista");
    let org = scenario.org("a");
    let agent_a = scenario.agent(&org, "a");
    let agent_b = scenario.agent(&org, "b");

    let conversation_a = scenario.conversation(&org, &agent_a);
    let conversation_b = scenario
        .store()
        .conversations()
        .create(
            org.id,
            NewConversation {
                agent_id: agent_b.id,
                integration_id: None,
                canal: "whatsapp".to_owned(),
                peer_externo: Some("5521988887777".to_owned()),
                target_department: None,
            },
        )
        .expect("segunda conversa");
    scenario.close(&org, &conversation_a);

    let opened: Vec<_> = scenario
        .store()
        .cases()
        .list(
            org.id,
            &poph_store::RecordFilter {
                status: Some(RecordStatus::Open),
                ..Default::default()
            },
            Page::default(),
        )
        .expect("lista abertas");
    assert_eq!(opened.len(), 1);
    assert_eq!(
        opened[0].conversation_id.map(|id| id.to_string()),
        Some(conversation_b.id.to_string())
    );

    let from_agent: Vec<_> = scenario
        .store()
        .cases()
        .list(
            org.id,
            &poph_store::RecordFilter {
                agent: Some(agent_b.id),
                ..Default::default()
            },
            Page::default(),
        )
        .expect("lista do agente");
    assert_eq!(from_agent.len(), 1);
    assert_eq!(from_agent[0].agent_id, agent_b.id);

    let from_channel: Vec<_> = scenario
        .store()
        .cases()
        .list(
            org.id,
            &poph_store::RecordFilter {
                canal: Some("console".to_owned()),
                ..Default::default()
            },
            Page::default(),
        )
        .expect("lista do canal");
    assert_eq!(from_channel.len(), 1);
    assert_eq!(from_channel[0].canal, "console");

    let after_everything = scenario.store().now() + 1;
    let none = scenario
        .store()
        .cases()
        .list(
            org.id,
            &poph_store::RecordFilter {
                since: Some(after_everything),
                ..Default::default()
            },
            Page::default(),
        )
        .expect("lista desde o futuro");
    assert!(none.is_empty());

    let first_page = scenario
        .store()
        .cases()
        .list(org.id, &Default::default(), Page::with_limit(1))
        .expect("primeira página");
    assert_eq!(first_page.len(), 1);
    let second_page = scenario
        .store()
        .cases()
        .list(
            org.id,
            &Default::default(),
            Page {
                after: Some(first_page[0].id.as_uuid()),
                limit: 1,
            },
        )
        .expect("segunda página");
    assert_eq!(second_page.len(), 1);
    assert_ne!(second_page[0].id, first_page[0].id);
}

#[test]
fn case_from_other_org_neither_appears_nor_accepts_event() {
    let scenario = Scenario::new("isolamento");
    let org_a = scenario.org("a");
    let org_b = scenario.org("b");
    let agent_a = scenario.agent(&org_a, "a");
    let agent_b = scenario.agent(&org_b, "b");
    let conversation_a = scenario.conversation(&org_a, &agent_a);
    scenario.conversation(&org_b, &agent_b);
    let case_a = scenario
        .store()
        .cases()
        .by_conversation(org_a.id, conversation_a.id)
        .expect("consulta")
        .expect("Ficha A");

    let error = scenario
        .store()
        .cases()
        .get(org_b.id, case_a.id)
        .expect_err("Ficha de outra org não é legível");
    assert!(
        matches!(
            error,
            poph_store::StoreError::Domain(DomainError::NotFound { .. })
        ),
        "veio {error:?}"
    );

    let error = scenario
        .store()
        .cases()
        .register_event(
            org_b.id,
            case_a.id,
            NewRecordEvent {
                execucao_id: None,
                tipo: RecordEventKind::StepEntered,
                ator_tipo: EventActor::Agent,
                ator_ref: None,
                payload_json: "{}".to_owned(),
                chave_idempotencia: None,
            },
        )
        .expect_err("evento em Ficha de outra org é recusado");
    assert!(matches!(
        error,
        poph_store::StoreError::Domain(DomainError::NotFound { .. })
    ));

    let of_org_b: Vec<_> = scenario
        .store()
        .cases()
        .list(org_b.id, &Default::default(), Page::default())
        .expect("lista da org B");
    assert_eq!(of_org_b.len(), 1);
    assert!(
        of_org_b.iter().all(|case| case.id != case_a.id),
        "a lista não cruza orgs"
    );

    let error = scenario
        .store()
        .cases()
        .events(org_b.id, case_a.id, None, 50)
        .expect_err("linha do tempo de outra org não é legível");
    assert!(matches!(
        error,
        poph_store::StoreError::Domain(DomainError::NotFound { .. })
    ));
}

#[test]
fn events_leave_in_order_and_paginate_by_seq() {
    let scenario = Scenario::new("eventos");
    let org = scenario.org("a");
    let agent = scenario.agent(&org, "a");
    let conversation = scenario.conversation(&org, &agent);
    let case = scenario
        .store()
        .cases()
        .by_conversation(org.id, conversation.id)
        .expect("consulta")
        .expect("Ficha");

    for tipo in [
        RecordEventKind::FlowStarted,
        RecordEventKind::StepEntered,
        RecordEventKind::FieldsFilled,
    ] {
        scenario
            .store()
            .cases()
            .register_event(
                org.id,
                case.id,
                NewRecordEvent {
                    execucao_id: None,
                    tipo,
                    ator_tipo: EventActor::Agent,
                    ator_ref: Some("agente-1"),
                    payload_json: "{}".to_owned(),
                    chave_idempotencia: None,
                },
            )
            .expect("registra evento");
    }

    let all_rows = scenario
        .store()
        .cases()
        .events(org.id, case.id, None, 50)
        .expect("eventos");
    let seqs: Vec<i64> = all_rows.iter().map(|evento| evento.seq).collect();
    assert_eq!(seqs, vec![1, 2, 3, 4], "abertura + três eventos, em ordem");

    let second_page = scenario
        .store()
        .cases()
        .events(org.id, case.id, Some(2), 50)
        .expect("eventos depois do seq 2");
    assert_eq!(
        second_page
            .iter()
            .map(|evento| evento.tipo)
            .collect::<Vec<_>>(),
        vec![RecordEventKind::FieldsFilled, RecordEventKind::StepEntered,]
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
    );

    let with_limit = scenario
        .store()
        .cases()
        .events(org.id, case.id, None, 2)
        .expect("eventos com limite");
    assert_eq!(with_limit.len(), 2);
}

#[test]
fn event_with_same_key_does_not_duplicate() {
    let scenario = Scenario::new("idempotencia");
    let org = scenario.org("a");
    let agent = scenario.agent(&org, "a");
    let conversation = scenario.conversation(&org, &agent);
    let case = scenario
        .store()
        .cases()
        .by_conversation(org.id, conversation.id)
        .expect("consulta")
        .expect("Ficha");

    let new = || NewRecordEvent {
        execucao_id: None,
        tipo: RecordEventKind::ActionExecuted,
        ator_tipo: EventActor::Attendant,
        ator_ref: Some("@atendente:matrix"),
        payload_json: "{\"acao\":\"informar_endereco\"}".to_owned(),
        chave_idempotencia: Some("acao-1"),
    };
    let first_value = scenario
        .store()
        .cases()
        .register_event(org.id, case.id, new())
        .expect("primeiro evento");
    let second_value = scenario
        .store()
        .cases()
        .register_event(org.id, case.id, new())
        .expect("repetição devolve o mesmo evento");

    assert_eq!(first_value.id, second_value.id);
    assert_eq!(first_value.seq, second_value.seq);
    assert_eq!(second_value.ator_ref.as_deref(), Some("@atendente:matrix"));

    let events = scenario
        .store()
        .cases()
        .events(org.id, case.id, None, 50)
        .expect("eventos");
    assert_eq!(events.len(), 2, "abertura + uma ação, sem duplicata");
}

#[test]
fn event_of_execution_of_other_org_is_refused() {
    let scenario = Scenario::new("evento-execucao");
    let org_a = scenario.org("a");
    let org_b = scenario.org("b");
    let agent_a = scenario.agent(&org_a, "a");
    let agent_b = scenario.agent(&org_b, "b");
    let conversation_b = scenario.conversation(&org_b, &agent_b);
    let case_b = scenario
        .store()
        .cases()
        .by_conversation(org_b.id, conversation_b.id)
        .expect("consulta")
        .expect("Ficha B");
    let execution_of_b = scenario.execution_active(&org_b, case_b.id, scenario.store().now());
    let execution_of_b: poph_types::ExecutionId = execution_of_b.parse().expect("id");

    let conversation_a = scenario.conversation(&org_a, &agent_a);
    let case_a = scenario
        .store()
        .cases()
        .by_conversation(org_a.id, conversation_a.id)
        .expect("consulta")
        .expect("Ficha A");

    let error = scenario
        .store()
        .cases()
        .register_event(
            org_a.id,
            case_a.id,
            NewRecordEvent {
                execucao_id: Some(execution_of_b),
                tipo: RecordEventKind::StepEntered,
                ator_tipo: EventActor::Agent,
                ator_ref: None,
                payload_json: "{}".to_owned(),
                chave_idempotencia: None,
            },
        )
        .expect_err("execução de outra org não entra no evento");
    assert!(
        matches!(
            error,
            poph_store::StoreError::Domain(DomainError::NotFound { .. })
        ),
        "veio {error:?}"
    );
}

#[test]
fn case_created_by_repository_starts_open_and_without_conversation() {
    let scenario = Scenario::new("ficha-solta");
    let org = scenario.org("a");
    let agent = scenario.agent(&org, "a");

    let case = scenario
        .store()
        .cases()
        .create(
            org.id,
            NewRecord {
                conversation_id: None,
                agent_id: agent.id,
                canal: "whatsapp".to_owned(),
                peer_externo: None,
            },
        )
        .expect("cria a Ficha sem conversa");
    assert_eq!(case.status, RecordStatus::Open);
    assert_eq!(case.conversation_id, None);
    assert!(
        scenario
            .store()
            .cases()
            .by_conversation(org.id, ConversationId::generate())
            .expect("consulta")
            .is_none()
    );
}

#[test]
fn case_survives_deleted_conversation() {
    let scenario = Scenario::new("retencao");
    let org = scenario.org("a");
    let agent = scenario.agent(&org, "a");
    let conversation = scenario.conversation(&org, &agent);
    let case = scenario
        .store()
        .cases()
        .by_conversation(org.id, conversation.id)
        .expect("consulta")
        .expect("Ficha");

    scenario
        .connection()
        .execute(
            "DELETE FROM conversations WHERE org_id = ?1 AND id = ?2",
            rusqlite::params![org.id.to_string(), conversation.id.to_string()],
        )
        .expect("apaga a conversa");

    let later = scenario
        .store()
        .cases()
        .get(org.id, case.id)
        .expect("a Ficha continua legível");
    assert_eq!(later.conversation_id, None);
    assert_eq!(later.agent_id, agent.id, "o retrato do atendimento fica");

    let list = scenario
        .store()
        .cases()
        .list(org.id, &Default::default(), Page::default())
        .expect("lista");
    assert_eq!(list.len(), 1, "Ficha órfã continua na lista");
}

#[test]
fn case_id_is_uuid_v7() {
    let scenario = Scenario::new("id");
    let org = scenario.org("a");
    let agent = scenario.agent(&org, "a");
    let conversation = scenario.conversation(&org, &agent);
    let case = scenario
        .store()
        .cases()
        .by_conversation(org.id, conversation.id)
        .expect("consulta")
        .expect("Ficha");

    let texto = case.id.to_string();
    let uuid = uuid::Uuid::parse_str(&texto).expect("id é UUID");
    assert_eq!(uuid.get_version_num(), 7);
    let _: OrgId = org.id;
    let _: AgentId = agent.id;
}

#[test]
fn event_of_execution_of_other_case_in_same_org_is_refused() {
    let scenario = Scenario::new("evento-execucao-ficha");
    let org = scenario.org("a");
    let agent = scenario.agent(&org, "a");

    let conversation_1 = scenario.conversation(&org, &agent);
    let case_1 = scenario
        .store()
        .cases()
        .by_conversation(org.id, conversation_1.id)
        .expect("consulta")
        .expect("Ficha 1");
    let execution = scenario.execution_active(&org, case_1.id, scenario.store().now());
    let execution: poph_types::ExecutionId = execution.parse().expect("id");

    let conversation_2 = scenario.conversation(&org, &agent);
    let case_2 = scenario
        .store()
        .cases()
        .by_conversation(org.id, conversation_2.id)
        .expect("consulta")
        .expect("Ficha 2");

    let same_service = scenario
        .store()
        .cases()
        .register_event(
            org.id,
            case_1.id,
            NewRecordEvent {
                execucao_id: Some(execution),
                tipo: RecordEventKind::StepEntered,
                ator_tipo: EventActor::Agent,
                ator_ref: None,
                payload_json: "{}".to_owned(),
                chave_idempotencia: None,
            },
        )
        .expect("evento na própria Ficha da execução");
    assert_eq!(
        same_service.execucao_id.map(|id| id.to_string()),
        Some(execution.to_string())
    );

    let error = scenario
        .store()
        .cases()
        .register_event(
            org.id,
            case_2.id,
            NewRecordEvent {
                execucao_id: Some(execution),
                tipo: RecordEventKind::StepEntered,
                ator_tipo: EventActor::Agent,
                ator_ref: None,
                payload_json: "{}".to_owned(),
                chave_idempotencia: None,
            },
        )
        .expect_err("execução de outro atendimento não entra no evento");
    assert!(
        matches!(
            error,
            poph_store::StoreError::Domain(DomainError::NotFound { .. })
        ),
        "veio {error:?}"
    );
}

#[test]
fn conversation_without_case_that_is_closed_gains_closed_case() {
    let scenario = Scenario::new("sem-ficha");
    let org = scenario.org("a");
    let agent = scenario.agent(&org, "a");
    let conversation = scenario.conversation(&org, &agent);
    let case_original = scenario
        .store()
        .cases()
        .by_conversation(org.id, conversation.id)
        .expect("consulta")
        .expect("Ficha");

    scenario
        .connection()
        .execute(
            "DELETE FROM fichas WHERE org_id = ?1 AND id = ?2",
            rusqlite::params![org.id.to_string(), case_original.id.to_string()],
        )
        .expect("remove a Ficha");

    let closed = scenario.close(&org, &conversation);
    assert_eq!(closed.status, ConversationStatus::Closed);

    let case = scenario
        .store()
        .cases()
        .by_conversation(org.id, conversation.id)
        .expect("consulta")
        .expect("a conversa fechada ganhou Ficha");
    assert_eq!(case.status, RecordStatus::Closed);
    assert_eq!(case.fechada_por, Some(ClosedBy::Admin));
    assert_eq!(case.aberta_em, conversation.created_at);

    let events = scenario
        .store()
        .cases()
        .events(org.id, case.id, None, 50)
        .expect("eventos");
    assert_eq!(
        events.last().map(|evento| evento.tipo),
        Some(RecordEventKind::RecordClosed)
    );
}

#[test]
fn closing_by_agent_is_atomic_with_audit() {
    let c = Scenario::new("fx25-atomicidade");
    let org = c.org("fx25");
    let agent = c.agent(&org, "fx25");
    let conversation = c.conversation(&org, &agent);
    let case = c
        .store()
        .cases()
        .by_conversation(org.id, conversation.id)
        .unwrap()
        .unwrap();
    let execution = c.execution_active(&org, case.id, 1);
    let conn = c.connection();
    conn.execute_batch("CREATE TRIGGER falha_auditoria_fx25 BEFORE INSERT ON audit_log WHEN NEW.action = 'close' BEGIN SELECT RAISE(ABORT, 'falha injetada'); END;").unwrap();
    assert!(
        c.store()
            .conversations()
            .close_by_agent(org.id, conversation.id, agent.id, "resolvido".into())
            .is_err()
    );
    assert_eq!(
        c.store()
            .conversations()
            .get(org.id, conversation.id)
            .unwrap()
            .status,
        ConversationStatus::Open
    );
    assert_eq!(
        c.store().cases().get(org.id, case.id).unwrap().status,
        RecordStatus::Open
    );
    let status: String = conn
        .query_row(
            "SELECT status FROM fluxo_execucoes WHERE id=?1",
            [&execution],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(status, "ativa");
    assert!(
        c.store()
            .cases()
            .events(org.id, case.id, None, 50)
            .unwrap()
            .iter()
            .all(|e| e.tipo != RecordEventKind::RecordClosed)
    );
    let n: i64 = conn
        .query_row(
            "SELECT count(*) FROM audit_log WHERE action='close' AND entity_id=?1",
            [conversation.id.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 0);
    conn.execute_batch("DROP TRIGGER falha_auditoria_fx25;")
        .unwrap();
    assert!(
        c.store()
            .conversations()
            .close_by_agent(org.id, conversation.id, agent.id, "resolvido".into())
            .unwrap()
    );
    assert!(
        !c.store()
            .conversations()
            .close_by_agent(org.id, conversation.id, agent.id, "resolvido".into())
            .unwrap()
    );
}

#[test]
fn message_after_closing_by_agent_opens_new_case() {
    let c = Scenario::new("fx25-ciclo");
    let org = c.org("fx25-ciclo");
    let agent = c.agent(&org, "fx25-ciclo");
    let integration = c.integration(&org, &agent);
    let first = c
        .store()
        .inbox()
        .receive_from_channel(org.id, entry(integration, agent.id, "FX25-1"))
        .unwrap();
    let conversation = first.mensagem.conversation_id;
    let case = c
        .store()
        .cases()
        .by_conversation(org.id, conversation)
        .unwrap()
        .unwrap();
    c.store()
        .conversations()
        .close_by_agent(org.id, conversation, agent.id, "resolvido".into())
        .unwrap();
    let second = c
        .store()
        .inbox()
        .receive_from_channel(org.id, entry(integration, agent.id, "FX25-2"))
        .unwrap();
    let other = second.mensagem.conversation_id;
    assert_ne!(conversation, other);
    assert_ne!(
        case.id,
        c.store()
            .cases()
            .by_conversation(org.id, other)
            .unwrap()
            .unwrap()
            .id
    );
}

#[test]
fn closing_by_agent_respects_org_and_identity() {
    let c = Scenario::new("fx25-isolamento");
    let a = c.org("fx25-org-a");
    let b = c.org("fx25-org-b");
    let agent_a = c.agent(&a, "fx25-a");
    let another = c.agent(&a, "fx25-outro");
    let conversation = c.conversation(&a, &agent_a);
    assert!(
        c.store()
            .conversations()
            .close_by_agent(b.id, conversation.id, agent_a.id, "fim".into())
            .is_err()
    );
    assert!(
        c.store()
            .conversations()
            .close_by_agent(a.id, conversation.id, another.id, "fim".into())
            .is_err()
    );
    assert_eq!(
        c.store()
            .conversations()
            .get(a.id, conversation.id)
            .unwrap()
            .status,
        ConversationStatus::Open
    );
}
