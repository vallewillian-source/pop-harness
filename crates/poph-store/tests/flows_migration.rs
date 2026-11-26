use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use poph_store::{HARNESS_MIGRATIONS, Store, file_version, harness_target_version};
use poph_types::{EffectState, ExecutionStatus, StepStatus};
use rusqlite::Connection;

const MIGRATION: &str = "0013_fluxos_revisoes_execucoes_e_efeitos";

fn folder_new(label: &str) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let folder =
        std::env::temp_dir().join(format!("poph-fluxos-{label}-{}-{n}", std::process::id()));
    let _ = fs::remove_dir_all(&folder);
    fs::create_dir_all(&folder).expect("cria a pasta do teste");
    folder
}

fn connection(path: &std::path::Path) -> Connection {
    let conn = Connection::open(path).expect("abre o banco para conferir");
    conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA busy_timeout = 2000;")
        .expect("pragmas");
    conn
}

fn count_rows(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |row| row.get(0)).expect("conta")
}

fn new_id() -> String {
    uuid::Uuid::now_v7().to_string()
}

struct Scenario {
    folder: PathBuf,
    _store: Store,
    conn: Connection,
}

impl Scenario {
    fn new(label: &str) -> Self {
        let folder = folder_new(label);
        let harness = folder.join("harness.db");
        let store = Store::open_at(&harness, &folder.join("rag.db")).expect("abre e migra");
        let conn = connection(&harness);
        Self {
            folder,
            _store: store,
            conn,
        }
    }

    fn org(&self, label: &str) -> String {
        let org = new_id();
        self.conn
            .execute(
                "INSERT INTO orgs (id, name, slug, created_at, updated_at) VALUES (?1, ?2, ?3, 1, 1)",
                rusqlite::params![org, format!("Org {label}"), format!("org-{label}")],
            )
            .expect("org");
        org
    }

    fn agent(&self, org: &str, label: &str) -> String {
        let agent = new_id();
        self.conn
            .execute(
                "INSERT INTO agents (id, org_id, name, slug, provider, model, created_at,
                                     updated_at)
                 VALUES (?1, ?2, ?3, ?4, 'antigravity2api', 'gemini', 1, 1)",
                rusqlite::params![
                    agent,
                    org,
                    format!("Agente {label}"),
                    format!("agente-{label}")
                ],
            )
            .expect("agente");
        agent
    }

    fn case(&self, org: &str, agent: &str) -> String {
        let case = new_id();
        self.conn
            .execute(
                "INSERT INTO fichas (id, org_id, conversation_id, agent_id, canal, status,
                                     aberta_em)
                 VALUES (?1, ?2, NULL, ?3, 'whatsapp', 'open', 1)",
                rusqlite::params![case, org, agent],
            )
            .expect("ficha");
        case
    }

    fn flow(&self, org: &str, slug: &str) -> String {
        let flow = new_id();
        self.conn
            .execute(
                "INSERT INTO fluxos (id, org_id, slug, nome, descricao) VALUES (?1, ?2, ?3, ?4, '')",
                rusqlite::params![flow, org, slug, format!("Fluxo {slug}")],
            )
            .expect("fluxo");
        flow
    }

    fn revision(&self, org: &str, flow: &str, numero: i64) -> String {
        let revision = new_id();
        self.conn
            .execute(
                "INSERT INTO fluxo_revisoes
                    (id, org_id, fluxo_id, numero, fonte, fonte_sha256, definicao_json,
                     publicada_em, publicada_por)
                 VALUES (?1, ?2, ?3, ?4, 'fluxo(id = \"x\")', 'sha', '{}', 1, 'cli')",
                rusqlite::params![revision, org, flow, numero],
            )
            .expect("revisão");
        revision
    }

    fn execution(&self, org: &str, case: &str, flow: &str, revision: &str) -> String {
        let execution = new_id();
        self.conn
            .execute(
                "INSERT INTO fluxo_execucoes
                    (id, org_id, ficha_id, fluxo_id, revisao_id, status, no_atual, iniciada_em)
                 VALUES (?1, ?2, ?3, ?4, ?5, 'ativa', 'inicio', 1)",
                rusqlite::params![execution, org, case, flow, revision],
            )
            .expect("execução");
        execution
    }
}

impl Drop for Scenario {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.folder);
    }
}

#[test]
fn migration_creates_six_tables_with_columns_and_indexes() {
    let scenario = Scenario::new("esquema");
    let harness = scenario.folder.join("harness.db");
    assert_eq!(
        file_version(&harness).expect("versão"),
        Some(harness_target_version())
    );
    assert!(
        harness_target_version() >= 13,
        "a migração 0013 tem que estar registrada"
    );
    assert!(
        HARNESS_MIGRATIONS.iter().any(|m| m.name == MIGRATION),
        "migração {MIGRATION} não está em MIGRACOES_HARNESS"
    );

    let expected_value: &[(&str, &[&str])] = &[
        (
            "fluxos",
            &[
                "id",
                "org_id",
                "slug",
                "nome",
                "descricao",
                "status",
                "rascunho_fonte",
                "revisao_atual",
                "version",
            ],
        ),
        (
            "fluxo_revisoes",
            &[
                "id",
                "org_id",
                "fluxo_id",
                "numero",
                "fonte",
                "fonte_sha256",
                "definicao_json",
                "schema_versao",
                "publicada_em",
                "publicada_por",
                "resolucoes_json",
            ],
        ),
        (
            "agente_fluxos",
            &["org_id", "agent_id", "fluxo_id", "liberado_em"],
        ),
        (
            "fluxo_execucoes",
            &[
                "id",
                "org_id",
                "ficha_id",
                "fluxo_id",
                "revisao_id",
                "status",
                "no_atual",
                "dados_json",
                "resultado",
                "motivo_fim",
                "iniciada_em",
                "finalizada_em",
                "version",
            ],
        ),
        (
            "fluxo_execucao_etapas",
            &[
                "id",
                "org_id",
                "execucao_id",
                "no_id",
                "ordem",
                "status",
                "dados_json",
                "atores_que_agiram_json",
                "entrou_em",
                "concluiu_em",
            ],
        ),
        (
            "fluxo_efeitos",
            &[
                "id",
                "org_id",
                "ficha_id",
                "execucao_id",
                "chave",
                "tipo",
                "payload_json",
                "estado",
                "tentativas",
                "ultimo_erro",
                "proxima_tentativa",
                "enviado_em",
                "created_at",
            ],
        ),
    ];
    for (table, columns) in expected_value {
        let from_database = scenario._store.columns(table).expect("colunas da tabela");
        for column in *columns {
            assert!(
                from_database.iter().any(|c| c == column),
                "tabela {table} sem a coluna {column}; tem {from_database:?}"
            );
        }
    }

    for index in [
        "ux_fluxos_slug",
        "ux_fluxos_org_id",
        "ix_fluxos_org_status",
        "ux_fluxo_revisoes_numero",
        "ix_agente_fluxos_fluxo",
        "ux_fluxo_execucoes_ativa",
        "ux_fluxo_execucoes_org_id",
        "ix_fluxo_execucoes_ficha",
        "ix_fluxo_execucoes_revisao",
        "ux_fluxo_execucao_etapas_ordem",
        "ux_fluxo_efeitos_chave",
        "ix_fluxo_efeitos_fila",
        "ix_fluxo_efeitos_org",
        "ix_fichas_org_agente",
    ] {
        let exists = count_rows(
            &scenario.conn,
            &format!(
                "SELECT count(*) FROM sqlite_master WHERE type = 'index' AND name = '{index}'"
            ),
        );
        assert_eq!(exists, 1, "índice {index} não foi criado");
    }

    let mut columns = scenario
        .conn
        .prepare("PRAGMA index_info('ix_fichas_org_agente')")
        .expect("pragma index_info")
        .query_map([], |row| row.get::<_, String>(2))
        .expect("lê o índice")
        .collect::<Result<Vec<_>, _>>()
        .expect("colunas do índice");
    columns.sort_by_key(|nome| match nome.as_str() {
        "org_id" => 0,
        "agent_id" => 1,
        _ => 2,
    });
    assert_eq!(
        columns,
        vec!["org_id", "agent_id", "aberta_em"],
        "o índice do filtro por agente precisa ter as três colunas"
    );

    for gatilho in [
        "tr_fluxo_revisoes_sem_update",
        "tr_fluxo_revisoes_sem_delete",
    ] {
        let exists = count_rows(
            &scenario.conn,
            &format!(
                "SELECT count(*) FROM sqlite_master WHERE type = 'trigger' AND name = '{gatilho}'"
            ),
        );
        assert_eq!(exists, 1, "gatilho {gatilho} não foi criado");
    }
}

#[test]
fn database_refuses_two_active_executions_in_same_case() {
    let scenario = Scenario::new("ativa");
    let org = scenario.org("a");
    let agent = scenario.agent(&org, "a");
    let flow = scenario.flow(&org, "agendar");
    let revision = scenario.revision(&org, &flow, 1);
    let case = scenario.case(&org, &agent);

    let first = scenario.execution(&org, &case, &flow, &revision);

    let error = scenario
        .conn
        .execute(
            "INSERT INTO fluxo_execucoes
                (id, org_id, ficha_id, fluxo_id, revisao_id, status, no_atual, iniciada_em)
             VALUES (?1, ?2, ?3, ?4, ?5, 'ativa', 'inicio', 2)",
            rusqlite::params![new_id(), org, case, flow, revision],
        )
        .expect_err("a segunda execução ativa da mesma Ficha tem que ser recusada");
    assert!(
        error.to_string().contains("UNIQUE"),
        "erro inesperado: {error}"
    );

    scenario
        .conn
        .execute(
            "UPDATE fluxo_execucoes SET status = 'concluida', finalizada_em = 5 WHERE id = ?1",
            [&first],
        )
        .expect("conclui a execução");
    scenario.execution(&org, &case, &flow, &revision);

    let other_case = scenario.case(&org, &agent);
    scenario.execution(&org, &other_case, &flow, &revision);

    let error = scenario
        .conn
        .execute(
            "INSERT INTO fluxo_execucoes
                (id, org_id, ficha_id, fluxo_id, revisao_id, status, no_atual, iniciada_em,
                 finalizada_em)
             VALUES (?1, ?2, ?3, ?4, ?5, 'ativa', 'inicio', 3, 9)",
            rusqlite::params![new_id(), org, other_case, flow, revision],
        )
        .expect_err("ativa com instante de fim tem que ser recusada");
    assert!(
        error.to_string().contains("CHECK"),
        "erro inesperado: {error}"
    );
    let error = scenario
        .conn
        .execute(
            "INSERT INTO fluxo_execucoes
                (id, org_id, ficha_id, fluxo_id, revisao_id, status, no_atual, iniciada_em)
             VALUES (?1, ?2, ?3, ?4, ?5, 'concluida', 'inicio', 4)",
            rusqlite::params![new_id(), org, other_case, flow, revision],
        )
        .expect_err("encerrada sem instante de fim tem que ser recusada");
    assert!(
        error.to_string().contains("CHECK"),
        "erro inesperado: {error}"
    );
}

#[test]
fn published_revision_cannot_be_changed_nor_deleted() {
    let scenario = Scenario::new("imutavel");
    let org = scenario.org("a");
    let flow = scenario.flow(&org, "agendar");
    let revision = scenario.revision(&org, &flow, 1);

    let error = scenario
        .conn
        .execute(
            "UPDATE fluxo_revisoes SET fonte = 'outra coisa' WHERE id = ?1",
            [&revision],
        )
        .expect_err("alterar revisão publicada tem que ser recusado pelo banco");
    assert!(
        error.to_string().contains("imutável"),
        "erro inesperado: {error}"
    );
    assert!(
        scenario
            .conn
            .execute(
                "UPDATE fluxo_revisoes SET numero = 7 WHERE id = ?1",
                [&revision]
            )
            .is_err(),
        "trocar o número da revisão tem que ser recusado"
    );

    let error = scenario
        .conn
        .execute("DELETE FROM fluxo_revisoes WHERE id = ?1", [&revision])
        .expect_err("apagar revisão publicada tem que ser recusado pelo banco");
    assert!(
        error.to_string().contains("imutável"),
        "erro inesperado: {error}"
    );

    let fonte: String = scenario
        .conn
        .query_row(
            "SELECT fonte FROM fluxo_revisoes WHERE id = ?1",
            [&revision],
            |row| row.get(0),
        )
        .expect("a revisão continua no banco");
    assert_eq!(fonte, "fluxo(id = \"x\")");

    scenario
        .conn
        .execute("DELETE FROM fluxos WHERE id = ?1", [&flow])
        .expect("apagar o fluxo com force leva as revisões");
    assert_eq!(
        count_rows(&scenario.conn, "SELECT count(*) FROM fluxo_revisoes"),
        0,
        "as revisões do fluxo apagado ficaram órfãs"
    );
}

#[test]
fn database_refuses_link_between_different_orgs() {
    let scenario = Scenario::new("orgs");
    let org_a = scenario.org("a");
    let org_b = scenario.org("b");
    let agent_a = scenario.agent(&org_a, "a");

    let flow_a = scenario.flow(&org_a, "da-a");
    let flow_b = scenario.flow(&org_b, "da-b");

    let error = scenario
        .conn
        .execute(
            "INSERT INTO fluxo_revisoes
                (id, org_id, fluxo_id, numero, fonte, fonte_sha256, definicao_json,
                 publicada_em, publicada_por)
             VALUES (?1, ?2, ?3, 1, 'x', 'sha', '{}', 1, 'cli')",
            rusqlite::params![new_id(), org_a, flow_b],
        )
        .expect_err("revisão cruzando orgs tem que ser recusada pelo banco");
    assert!(
        error.to_string().contains("FOREIGN KEY"),
        "erro inesperado: {error}"
    );

    assert!(
        scenario
            .conn
            .execute(
                "INSERT INTO agente_fluxos (org_id, agent_id, fluxo_id, liberado_em)
                 VALUES (?1, ?2, ?3, 1)",
                rusqlite::params![org_a, agent_a, flow_b],
            )
            .is_err(),
        "permissão para fluxo de outra org tem que ser recusada"
    );

    let case_a = scenario.case(&org_a, &agent_a);
    let revision_b = scenario.revision(&org_b, &flow_b, 1);
    let error = scenario
        .conn
        .execute(
            "INSERT INTO fluxo_execucoes
                (id, org_id, ficha_id, fluxo_id, revisao_id, status, no_atual, iniciada_em)
             VALUES (?1, ?2, ?3, ?4, ?5, 'ativa', 'inicio', 1)",
            rusqlite::params![new_id(), org_b, case_a, flow_b, revision_b],
        )
        .expect_err("execução apontando Ficha de outra org tem que ser recusada");
    assert!(
        error.to_string().contains("FOREIGN KEY"),
        "erro inesperado: {error}"
    );

    let revision_a = scenario.revision(&org_a, &flow_a, 1);
    let execution_a = scenario.execution(&org_a, &case_a, &flow_a, &revision_a);
    let error = scenario
        .conn
        .execute(
            "INSERT INTO fluxo_efeitos
                (id, org_id, ficha_id, execucao_id, chave, tipo, payload_json, created_at)
             VALUES (?1, ?2, ?3, ?4, 'k', 'mensagem_ao_cliente', '{}', 1)",
            rusqlite::params![
                new_id(),
                org_b,
                scenario.case(&org_b, &scenario.agent(&org_b, "b")),
                execution_a
            ],
        )
        .expect_err("efeito cruzando orgs tem que ser recusado");
    assert!(
        error.to_string().contains("FOREIGN KEY"),
        "erro inesperado: {error}"
    );

    scenario
        .conn
        .execute(
            "INSERT INTO agente_fluxos (org_id, agent_id, fluxo_id, liberado_em)
             VALUES (?1, ?2, ?3, 1)",
            rusqlite::params![org_a, agent_a, flow_a],
        )
        .expect("permissão na mesma org é aceita");
}

#[test]
fn deleting_agent_removes_only_permission() {
    let scenario = Scenario::new("permissao");
    let org = scenario.org("a");
    let agent = scenario.agent(&org, "a");
    let another = scenario.agent(&org, "b");
    let flow = scenario.flow(&org, "agendar");
    let revision = scenario.revision(&org, &flow, 1);
    scenario
        .conn
        .execute(
            "INSERT INTO agente_fluxos (org_id, agent_id, fluxo_id, liberado_em)
             VALUES (?1, ?2, ?3, 10), (?1, ?4, ?3, 10)",
            rusqlite::params![org, agent, flow, another],
        )
        .expect("duas permissões");
    assert_eq!(
        count_rows(&scenario.conn, "SELECT count(*) FROM agente_fluxos"),
        2
    );

    scenario
        .conn
        .execute("DELETE FROM agents WHERE id = ?1", [&agent])
        .expect("apaga o agente");

    assert_eq!(
        count_rows(&scenario.conn, "SELECT count(*) FROM agente_fluxos"),
        1,
        "apagar o agente tem que levar só a permissão dele"
    );
    assert_eq!(count_rows(&scenario.conn, "SELECT count(*) FROM fluxos"), 1);
    assert_eq!(
        count_rows(&scenario.conn, "SELECT count(*) FROM fluxo_revisoes"),
        1
    );
    let restante: String = scenario
        .conn
        .query_row("SELECT agent_id FROM agente_fluxos", [], |row| row.get(0))
        .expect("permissão do outro agente");
    assert_eq!(restante, another);

    scenario
        .conn
        .execute("DELETE FROM fluxos WHERE id = ?1", [&flow])
        .expect("apaga o fluxo");
    assert_eq!(
        count_rows(&scenario.conn, "SELECT count(*) FROM agente_fluxos"),
        0,
        "a permissão de um fluxo apagado ficou no banco"
    );
    let _ = revision;
}

#[test]
fn slug_and_number_respect_uniqueness_by_org_and_by_flow() {
    let scenario = Scenario::new("unicidade");
    let org_a = scenario.org("a");
    let org_b = scenario.org("b");

    scenario.flow(&org_a, "agendar");
    let error = scenario
        .conn
        .execute(
            "INSERT INTO fluxos (id, org_id, slug, nome) VALUES (?1, ?2, 'agendar', 'Outro')",
            rusqlite::params![new_id(), org_a],
        )
        .expect_err("slug repetido na mesma org tem que ser recusado");
    assert!(
        error.to_string().contains("UNIQUE"),
        "erro inesperado: {error}"
    );
    scenario.flow(&org_b, "agendar");

    let flow = scenario.flow(&org_a, "triagem");
    let other_flow = scenario.flow(&org_a, "confirmar");
    scenario.revision(&org_a, &flow, 1);
    scenario.revision(&org_a, &flow, 2);
    let error = scenario
        .conn
        .execute(
            "INSERT INTO fluxo_revisoes
                (id, org_id, fluxo_id, numero, fonte, fonte_sha256, definicao_json,
                 publicada_em, publicada_por)
             VALUES (?1, ?2, ?3, 1, 'x', 'sha', '{}', 1, 'cli')",
            rusqlite::params![new_id(), org_a, flow],
        )
        .expect_err("número repetido no mesmo fluxo tem que ser recusado");
    assert!(
        error.to_string().contains("UNIQUE"),
        "erro inesperado: {error}"
    );
    scenario.revision(&org_a, &other_flow, 1);
}

#[test]
fn running_migration_again_does_nothing() {
    let mut scenario = Scenario::new("idempotente");
    let org = scenario.org("a");
    scenario.flow(&org, "agendar");
    let before = count_rows(&scenario.conn, "SELECT count(*) FROM fluxos");

    let reports = scenario._store.migrate().expect("migra de novo");
    assert!(reports.already_up_to_date(), "{reports:?}");

    scenario.conn = connection(&scenario.folder.join("harness.db"));
    assert_eq!(
        count_rows(&scenario.conn, "SELECT count(*) FROM fluxos"),
        before
    );
    assert_eq!(
        file_version(&scenario.folder.join("harness.db")).expect("versão"),
        Some(harness_target_version())
    );
}

#[test]
fn vocabulary_accepts_exactly_the_texts_of_poph_types() {
    let scenario = Scenario::new("vocabulario");
    let org = scenario.org("a");
    let agent = scenario.agent(&org, "a");
    let flow = scenario.flow(&org, "agendar");
    let revision = scenario.revision(&org, &flow, 1);

    for status in ExecutionStatus::ALL {
        let case = scenario.case(&org, &agent);
        let finalizada_em = if *status == ExecutionStatus::Active {
            None
        } else {
            Some(9_i64)
        };
        scenario
            .conn
            .execute(
                "INSERT INTO fluxo_execucoes
                    (id, org_id, ficha_id, fluxo_id, revisao_id, status, no_atual, iniciada_em,
                     finalizada_em)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'inicio', 1, ?7)",
                rusqlite::params![
                    new_id(),
                    org,
                    case,
                    flow,
                    revision,
                    status.as_text(),
                    finalizada_em
                ],
            )
            .unwrap_or_else(|error| panic!("status {status} recusado pelo banco: {error}"));
    }

    let case = scenario.case(&org, &agent);
    let execution = scenario.execution(&org, &case, &flow, &revision);

    for (index, status) in StepStatus::ALL.iter().enumerate() {
        let concluiu_em = if *status == StepStatus::Completed {
            Some(7_i64)
        } else {
            None
        };
        scenario
            .conn
            .execute(
                "INSERT INTO fluxo_execucao_etapas
                    (id, org_id, execucao_id, no_id, ordem, status, entrou_em, concluiu_em)
                 VALUES (?1, ?2, ?3, 'coleta', ?4, ?5, 1, ?6)",
                rusqlite::params![
                    new_id(),
                    org,
                    execution,
                    index as i64 + 1,
                    status.as_text(),
                    concluiu_em
                ],
            )
            .unwrap_or_else(|error| panic!("status de etapa {status} recusado: {error}"));
    }

    for (index, state) in EffectState::ALL.iter().enumerate() {
        for (position, tipo) in ["mensagem_ao_cliente", "notificar_whatsapp", "encaminhar"]
            .iter()
            .enumerate()
        {
            let chave = index * 3 + position + 1;
            let enviado_em = if *state == EffectState::Sent {
                Some(3_i64)
            } else {
                None
            };
            scenario
                .conn
                .execute(
                    "INSERT INTO fluxo_efeitos
                        (id, org_id, ficha_id, execucao_id, chave, tipo, estado, enviado_em,
                         created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 1)",
                    rusqlite::params![
                        new_id(),
                        org,
                        case,
                        execution,
                        format!("efeito-{chave}"),
                        tipo,
                        state.as_text(),
                        enviado_em
                    ],
                )
                .unwrap_or_else(|error| panic!("efeito {tipo}/{state} recusado: {error}"));
        }
    }

    for (sql, label) in [
        (
            "INSERT INTO fluxo_execucoes
                (id, org_id, ficha_id, fluxo_id, revisao_id, status, no_atual, iniciada_em)
             VALUES ('x', 'org', 'f', 'fl', 'r', 'pausada', 'inicio', 1)",
            "status de execução fora da lista",
        ),
        (
            "INSERT INTO fluxo_execucao_etapas
                (id, org_id, execucao_id, no_id, ordem, status, entrou_em)
             VALUES ('x', 'org', 'e', 'n', 99, 'feita', 1)",
            "status de etapa fora da lista",
        ),
        (
            "INSERT INTO fluxo_efeitos
                (id, org_id, ficha_id, execucao_id, chave, tipo, estado, created_at)
             VALUES ('x', 'org', 'f', 'e', 'k', 'ligar_para_cliente', 'pendente', 1)",
            "tipo de efeito fora da lista",
        ),
    ] {
        assert!(
            scenario.conn.execute(sql, []).is_err(),
            "{label} tem que ser recusado"
        );
    }
    assert!(
        scenario
            .conn
            .execute(
                "INSERT INTO fluxo_efeitos
                    (id, org_id, ficha_id, execucao_id, chave, tipo, estado, created_at)
                 VALUES (?1, ?2, ?3, ?4, 'inventado', 'encaminhar', 'entregue', 1)",
                rusqlite::params![new_id(), org, case, execution],
            )
            .is_err(),
        "estado de efeito fora da lista tem que ser recusado"
    );
    assert!(
        scenario
            .conn
            .execute(
                "INSERT INTO fluxo_efeitos
                    (id, org_id, ficha_id, execucao_id, chave, tipo, estado, created_at)
                 VALUES (?1, ?2, ?3, ?4, 'sem-instante', 'encaminhar', 'enviado', 1)",
                rusqlite::params![new_id(), org, case, execution],
            )
            .is_err(),
        "efeito enviado sem `enviado_em` tem que ser recusado"
    );
}

#[test]
fn deleting_org_takes_everything_without_hitting_trigger() {
    let scenario = Scenario::new("org-apagada");
    let org = scenario.org("a");
    let agent = scenario.agent(&org, "a");
    let flow = scenario.flow(&org, "agendar");
    let revision = scenario.revision(&org, &flow, 1);
    let case = scenario.case(&org, &agent);
    let execution = scenario.execution(&org, &case, &flow, &revision);
    scenario
        .conn
        .execute(
            "INSERT INTO agente_fluxos (org_id, agent_id, fluxo_id, liberado_em)
             VALUES (?1, ?2, ?3, 1)",
            rusqlite::params![org, agent, flow],
        )
        .expect("permissão");
    scenario
        .conn
        .execute(
            "INSERT INTO fluxo_execucao_etapas
                (id, org_id, execucao_id, no_id, ordem, status, entrou_em)
             VALUES (?1, ?2, ?3, 'coleta', 1, 'pendente', 1)",
            rusqlite::params![new_id(), org, execution],
        )
        .expect("etapa");
    scenario
        .conn
        .execute(
            "INSERT INTO fluxo_efeitos
                (id, org_id, ficha_id, execucao_id, chave, tipo, payload_json, created_at)
             VALUES (?1, ?2, ?3, ?4, 'k', 'encaminhar', '{}', 1)",
            rusqlite::params![new_id(), org, case, execution],
        )
        .expect("efeito");

    scenario
        .conn
        .execute("DELETE FROM orgs WHERE id = ?1", [&org])
        .expect("apagar a org não pode esbarrar no gatilho de imutabilidade");

    for table in [
        "fluxos",
        "fluxo_revisoes",
        "agente_fluxos",
        "fluxo_execucoes",
        "fluxo_execucao_etapas",
        "fluxo_efeitos",
    ] {
        assert_eq!(
            count_rows(&scenario.conn, &format!("SELECT count(*) FROM {table}")),
            0,
            "{table} ficou com linha órfã depois de apagar a org"
        );
    }
}

#[test]
fn steps_and_effects_disappear_with_execution_and_executions_with_case() {
    let scenario = Scenario::new("cascata");
    let org = scenario.org("a");
    let agent = scenario.agent(&org, "a");
    let flow = scenario.flow(&org, "agendar");
    let revision = scenario.revision(&org, &flow, 1);
    let case = scenario.case(&org, &agent);
    let execution = scenario.execution(&org, &case, &flow, &revision);

    scenario
        .conn
        .execute(
            "INSERT INTO fluxo_execucao_etapas
                (id, org_id, execucao_id, no_id, ordem, status, entrou_em)
             VALUES (?1, ?2, ?3, 'coleta', 1, 'pendente', 1)",
            rusqlite::params![new_id(), org, execution],
        )
        .expect("etapa");
    scenario
        .conn
        .execute(
            "INSERT INTO fluxo_efeitos
                (id, org_id, ficha_id, execucao_id, chave, tipo, payload_json, created_at)
             VALUES (?1, ?2, ?3, ?4, 'k', 'notificar_whatsapp', '{}', 1)",
            rusqlite::params![new_id(), org, case, execution],
        )
        .expect("efeito");
    assert!(
        scenario
            .conn
            .execute(
                "INSERT INTO fluxo_efeitos
                    (id, org_id, ficha_id, execucao_id, chave, tipo, payload_json, created_at)
                 VALUES (?1, ?2, ?3, ?4, 'k', 'encaminhar', '{}', 2)",
                rusqlite::params![new_id(), org, case, execution],
            )
            .is_err(),
        "chave de efeito repetida na mesma execução tem que ser recusada"
    );

    assert!(
        scenario
            .conn
            .execute(
                "INSERT INTO fluxo_execucao_etapas
                    (id, org_id, execucao_id, no_id, ordem, status, entrou_em)
                 VALUES (?1, ?2, ?3, 'pagamento', 1, 'pendente', 2)",
                rusqlite::params![new_id(), org, execution],
            )
            .is_err(),
        "ordem repetida na mesma execução tem que ser recusada"
    );

    scenario
        .conn
        .execute("DELETE FROM fichas WHERE id = ?1", [&case])
        .expect("apaga a Ficha");
    assert_eq!(
        count_rows(&scenario.conn, "SELECT count(*) FROM fluxo_execucoes"),
        0
    );
    assert_eq!(
        count_rows(&scenario.conn, "SELECT count(*) FROM fluxo_execucao_etapas"),
        0
    );
    assert_eq!(
        count_rows(&scenario.conn, "SELECT count(*) FROM fluxo_efeitos"),
        0
    );

    assert_eq!(count_rows(&scenario.conn, "SELECT count(*) FROM fluxos"), 1);
    assert_eq!(
        count_rows(&scenario.conn, "SELECT count(*) FROM fluxo_revisoes"),
        1
    );
}
