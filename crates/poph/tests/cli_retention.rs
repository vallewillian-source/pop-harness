use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};

use serde_json::Value;

struct Scenario {
    dir: PathBuf,
}

impl Scenario {
    fn new(label: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("poph-cli-retencao-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("dados")).expect("cria a pasta de dados");
        Self { dir }
    }

    fn dados(&self) -> PathBuf {
        self.dir.join("dados")
    }

    fn poph(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_poph"))
            .args(args)
            .current_dir(&self.dir)
            .env_clear()
            .env("POPH_DADOS_DIRETORIO", self.dados())
            .output()
            .expect("executa o poph")
    }

    fn create_org(&self, slug: &str) -> String {
        let saida = self.poph(&[
            "--offline",
            "org",
            "create",
            "--name",
            &format!("Org {slug}"),
            "--slug",
            slug,
            "--json",
        ]);
        assert_eq!(exit_code(&saida), 0, "{}", texto(&saida.stderr));
        json(&saida)["org"]["id"]
            .as_str()
            .expect("id da org")
            .to_owned()
    }

    fn configure(&self, slug: &str, config: &str) {
        let saida = self.poph(&[
            "--offline",
            "org",
            "update",
            "--if-match",
            "1",
            slug,
            "--config-json",
            config,
        ]);
        assert_eq!(exit_code(&saida), 0, "{}", texto(&saida.stderr));
    }
}

impl Drop for Scenario {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn exit_code(saida: &Output) -> i32 {
    saida.status.code().unwrap_or(-1)
}

fn texto(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn json(saida: &Output) -> Value {
    serde_json::from_str(&texto(&saida.stdout))
        .unwrap_or_else(|error| panic!("saída não é JSON ({error}): {}", texto(&saida.stdout)))
}

#[test]
fn dry_run_json_has_stable_shape_and_deletes_nothing() {
    let scenario = Scenario::new("dry-json");
    scenario.create_org("clinica-norte");

    let saida = scenario.poph(&["retention", "run", "--dry-run", "--json"]);

    assert_eq!(exit_code(&saida), 0, "{}", texto(&saida.stderr));
    let value = json(&saida);
    assert_eq!(value["versao_relatorio"], 1);
    assert_eq!(value["simulacao"], true);
    let orgs = value["orgs"].as_array().expect("lista de orgs");
    assert_eq!(orgs.len(), 1);
    assert_eq!(orgs[0]["slug"], "clinica-norte");
    assert_eq!(orgs[0]["situacao"], "varrida");
    assert_eq!(orgs[0]["dias"], 30, "padrão da instância");
    assert_eq!(orgs[0]["banco"]["mensagens"], 0);
    assert!(
        orgs[0]["banco"]["ids_conversas"]
            .as_array()
            .expect("lista")
            .is_empty()
    );
}

#[test]
fn run_without_orgs_exits_zero_and_warns() {
    let scenario = Scenario::new("sem-orgs");

    let saida = scenario.poph(&["retention", "run"]);

    assert_eq!(exit_code(&saida), 0, "{}", texto(&saida.stderr));
    assert!(texto(&saida.stdout).contains("Nenhuma org para varrer"));
}

#[test]
fn org_with_invalid_config_exits_one_and_says_nothing_was_deleted() {
    let scenario = Scenario::new("config-invalida");
    scenario.create_org("ruim");
    scenario.configure("ruim", r#"{"retencao":{"dias":-3}}"#);

    let saida = scenario.poph(&["retention", "run"]);

    assert_eq!(
        exit_code(&saida),
        1,
        "erro de configuração tem de sair com 1"
    );
    let output_text = texto(&saida.stdout);
    assert!(output_text.contains("Org ruim: erro"), "{output_text}");
    assert!(output_text.contains("Nada foi apagado"), "{output_text}");
}

#[test]
fn missing_org_exits_one_with_clear_message() {
    let scenario = Scenario::new("inexistente");
    scenario.create_org("existe");

    let saida = scenario.poph(&["--org", "nao-existe", "retention", "run"]);

    assert_eq!(exit_code(&saida), 1);
    assert!(
        texto(&saida.stderr).contains("não existe org com o slug `nao-existe`"),
        "{}",
        texto(&saida.stderr)
    );
}

#[test]
fn show_reports_default_and_org_zero_retention() {
    let scenario = Scenario::new("show");
    scenario.create_org("guarda-tudo");
    scenario.create_org("padrao");
    scenario.configure("guarda-tudo", r#"{"retencao":{"dias":0}}"#);

    let saida = scenario.poph(&["retention", "show"]);

    assert_eq!(exit_code(&saida), 0, "{}", texto(&saida.stderr));
    let output_text = texto(&saida.stdout);
    assert!(
        output_text.contains("Org guarda-tudo: nunca apagar (definida na org)"),
        "{output_text}"
    );
    assert!(
        output_text.contains("Org padrao: 30 dias (padrão da instância)"),
        "{output_text}"
    );
}

#[test]
fn show_json_with_invalid_org_exits_one() {
    let scenario = Scenario::new("show-json");
    scenario.create_org("ruim");
    scenario.configure("ruim", r#"{"retencao":{"dias":"trinta"}}"#);

    let saida = scenario.poph(&["retention", "show", "--json"]);

    assert_eq!(exit_code(&saida), 1);
    let value = json(&saida);
    assert_eq!(value["versao_relatorio"], 1);
    assert_eq!(value["padrao_dias"], 30);
    assert_eq!(value["orgs"][0]["slug"], "ruim");
    assert!(value["orgs"][0]["dias"].is_null());
    assert!(
        value["orgs"][0]["erro"]
            .as_str()
            .expect("mensagem de erro")
            .contains("Nada foi apagado")
    );
}

const MODULE_TABLES: &[&str] = &[
    "fluxos",
    "fluxo_revisoes",
    "agente_fluxos",
    "fluxo_execucoes",
    "fluxo_execucao_etapas",
    "ficha_eventos",
    "fluxo_efeitos",
    "fichas",
];

const DAY_MS: i64 = 86_400_000;

fn now_ms() -> i64 {
    let desde = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("relógio depois de 1970");
    i64::try_from(desde.as_millis()).expect("instante em ms")
}

fn snapshot(connection: &rusqlite::Connection, tabelas: &[&str]) -> Vec<(String, Vec<String>)> {
    let mut snapshot_rows = Vec::new();
    for table in tabelas {
        let mut query = connection
            .prepare(&format!("SELECT * FROM {table} ORDER BY rowid"))
            .expect("consulta da foto");
        let columns = query.column_count();
        let rows = query
            .query_map([], |row| {
                let mut parts = Vec::with_capacity(columns);
                for i in 0..columns {
                    let value: rusqlite::types::Value = row.get(i)?;
                    parts.push(format!("{value:?}"));
                }
                Ok(parts.join("|"))
            })
            .expect("linhas da foto")
            .collect::<Result<Vec<_>, _>>()
            .expect("leitura da foto");
        snapshot_rows.push(((*table).to_owned(), rows));
    }
    snapshot_rows
}

fn connection(dados: &std::path::Path) -> rusqlite::Connection {
    let connection = rusqlite::Connection::open(dados.join("harness.db")).expect("abre o banco");
    connection
        .execute_batch("PRAGMA foreign_keys = ON;")
        .expect("chaves estrangeiras");
    connection
}

#[test]
fn run_keeps_case_execution_step_event_and_effect() {
    use poph_store::{
        ExecutionStart, ExecutionState, NewConversation, NewFlow, RevisionPublication, Store,
        TransitionEffect,
    };
    use poph_types::{EventActor, ExecutionStatus};

    let scenario = Scenario::new("fichas");
    let slug = "clinica-fichas";
    scenario.create_org(slug);
    let dados = scenario.dados();

    {
        let store = Store::open_at(&dados.join("harness.db"), &dados.join("rag.db"))
            .expect("abre o banco do teste");
        let org = store.orgs().by_slug(slug).expect("org criada pela CLI").id;
        let agente = store
            .agents()
            .create(
                org,
                poph_store::NewAgent {
                    name: "Agente fictício".to_owned(),
                    slug: "agente-ficticio".to_owned(),
                    ..poph_store::NewAgent::default()
                },
            )
            .expect("agente");
        let fluxo = store
            .flows()
            .create(
                org,
                NewFlow {
                    slug: "agendar_visita".to_owned(),
                    name: "Agendar visita".to_owned(),
                    description: String::new(),
                    rascunho_fonte: "fluxo()".to_owned(),
                },
            )
            .expect("fluxo");
        let revision = store
            .revisions()
            .publish_revision(
                org,
                fluxo.id,
                RevisionPublication {
                    expected: fluxo.version,
                    fonte: "fluxo()".to_owned(),
                    definicao_json: "{}".to_owned(),
                    schema_versao: Some(1),
                    resolucoes_json: "{}".to_owned(),
                    ator_tipo: "cli".to_owned(),
                    actor_id: None,
                },
            )
            .expect("revisão");
        store
            .flows()
            .release(org, agente.id, fluxo.id)
            .expect("libera o fluxo");
        let conversa = store
            .conversations()
            .create(
                org,
                NewConversation {
                    agent_id: agente.id,
                    integration_id: None,
                    canal: "cli".to_owned(),
                    peer_externo: Some("cliente-ficticio".to_owned()),
                    target_department: None,
                },
            )
            .expect("conversa com Ficha");
        let ficha = store
            .cases()
            .by_conversation(org, conversa.id)
            .expect("consulta")
            .expect("a conversa tem Ficha");
        store
            .executions()
            .start_execution(
                org,
                ExecutionStart {
                    case: ficha.id,
                    flow: fluxo.id,
                    revision: revision.id,
                    actor: EventActor::Agent,
                    ator_ref: Some("agente-ficticio"),
                    jev_decision: None,
                    initial_state: ExecutionState {
                        status: ExecutionStatus::Active,
                        no_atual: "coleta".to_owned(),
                        dados_json: "{}".to_owned(),
                        acting_actors: Vec::new(),
                        result: None,
                        reason: None,
                    },
                    inbound_effects: vec![TransitionEffect {
                        key: "entrada/0".to_owned(),
                        tipo: "mensagem_ao_cliente".to_owned(),
                        payload_json: "{\"texto\":\"olá\"}".to_owned(),
                    }],
                    chave_idempotencia: None,
                },
            )
            .expect("abre a execução");
        let connection = connection(&dados);
        connection
            .execute(
                "UPDATE conversations SET created_at = ?1, updated_at = ?1",
                rusqlite::params![now_ms() - 400 * DAY_MS],
            )
            .expect("envelhece a conversa");
    }

    let before = {
        let connection = connection(&dados);
        snapshot(&connection, MODULE_TABLES)
    };

    let simulation = scenario.poph(&["--org", slug, "retention", "run", "--dry-run", "--json"]);
    assert_eq!(exit_code(&simulation), 0, "{}", texto(&simulation.stderr));
    assert_eq!(json(&simulation)["orgs"][0]["banco"]["conversas"], 1);
    {
        let connection = connection(&dados);
        assert_eq!(
            snapshot(&connection, MODULE_TABLES),
            before,
            "a simulação não pode mexer no módulo de Fluxos e Fichas"
        );
        let conversas: i64 = connection
            .query_row("SELECT COUNT(*) FROM conversations", [], |row| row.get(0))
            .expect("conta conversas");
        assert_eq!(conversas, 1, "a simulação não apaga a conversa");
    }

    let real = scenario.poph(&["--org", slug, "retention", "run", "--json"]);
    assert_eq!(exit_code(&real), 0, "{}", texto(&real.stderr));
    assert_eq!(json(&real)["orgs"][0]["banco"]["conversas"], 1);
    {
        let connection = connection(&dados);
        let conversas: i64 = connection
            .query_row("SELECT COUNT(*) FROM conversations", [], |row| row.get(0))
            .expect("conta conversas");
        assert_eq!(conversas, 0, "a conversa vazia e antiga sai");
        let (situacao, conversa): (String, Option<String>) = connection
            .query_row("SELECT status, conversation_id FROM fichas", [], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .expect("Ficha depois da varredura");
        assert_eq!(situacao, "open");
        assert_eq!(conversa, None, "a Ficha sobrevive sem o vínculo");
        let without_link: Vec<(String, Vec<String>)> = snapshot(&connection, MODULE_TABLES)
            .into_iter()
            .filter(|(table, _)| table != "fichas")
            .collect();
        assert_eq!(
            without_link,
            before
                .into_iter()
                .filter(|(table, _)| table != "fichas")
                .collect::<Vec<_>>(),
            "Ficha, execução, etapa, evento e efeito não podem perder linha"
        );
    }
}
