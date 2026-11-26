//! Testes de ponta a ponta de `poph retention run|show` (HR-51).
//!
//! Processo de verdade e banco de verdade numa pasta temporária. A varredura em si (o que sai,
//! o que fica, a simulação) é testada em `crates/poph-retention/tests/database.rs`; aqui se confere
//! a linha de comando: códigos de saída, formato do JSON e mensagens de erro.

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};

use serde_json::Value;

/// Pasta temporária com a pasta de dados. O banco nasce na primeira execução da CLI.
struct Cenario {
    pasta: PathBuf,
}

impl Cenario {
    fn novo(rotulo: &str) -> Self {
        let pasta =
            std::env::temp_dir().join(format!("poph-cli-retencao-{rotulo}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&pasta);
        fs::create_dir_all(pasta.join("dados")).expect("cria a pasta de dados");
        Self { pasta }
    }

    fn dados(&self) -> PathBuf {
        self.pasta.join("dados")
    }

    /// Roda `poph` com ambiente limpo, pasta atual isolada e a pasta de dados do teste.
    fn poph(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_poph"))
            .args(args)
            .current_dir(&self.pasta)
            .env_clear()
            .env("POPH_DADOS_DIRETORIO", self.dados())
            .output()
            .expect("executa o poph")
    }

    /// Cria uma org pela CLI (versão 1) e devolve o id lido do JSON.
    fn criar_org(&self, slug: &str) -> String {
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
        assert_eq!(codigo(&saida), 0, "{}", texto(&saida.stderr));
        json(&saida)["org"]["id"]
            .as_str()
            .expect("id da org")
            .to_owned()
    }

    /// Troca a configuração da org (a versão é 1, a da criação).
    fn configurar(&self, slug: &str, config: &str) {
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
        assert_eq!(codigo(&saida), 0, "{}", texto(&saida.stderr));
    }
}

impl Drop for Cenario {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.pasta);
    }
}

fn codigo(saida: &Output) -> i32 {
    saida.status.code().unwrap_or(-1)
}

fn texto(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn json(saida: &Output) -> Value {
    serde_json::from_str(&texto(&saida.stdout))
        .unwrap_or_else(|erro| panic!("saída não é JSON ({erro}): {}", texto(&saida.stdout)))
}

#[test]
fn run_com_dry_run_em_json_tem_forma_estavel_e_nao_apaga() {
    let cenario = Cenario::novo("dry-json");
    cenario.criar_org("clinica-norte");

    let saida = cenario.poph(&["retention", "run", "--dry-run", "--json"]);

    assert_eq!(codigo(&saida), 0, "{}", texto(&saida.stderr));
    let valor = json(&saida);
    assert_eq!(valor["versao_relatorio"], 1);
    assert_eq!(valor["simulacao"], true);
    let orgs = valor["orgs"].as_array().expect("lista de orgs");
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
fn run_sem_orgs_sai_com_0_e_avisa() {
    let cenario = Cenario::novo("sem-orgs");

    let saida = cenario.poph(&["retention", "run"]);

    assert_eq!(codigo(&saida), 0, "{}", texto(&saida.stderr));
    assert!(texto(&saida.stdout).contains("Nenhuma org para varrer"));
}

#[test]
fn org_com_configuracao_invalida_sai_com_1_e_avisa_que_nada_foi_apagado() {
    let cenario = Cenario::novo("config-invalida");
    cenario.criar_org("ruim");
    cenario.configurar("ruim", r#"{"retencao":{"dias":-3}}"#);

    let saida = cenario.poph(&["retention", "run"]);

    assert_eq!(codigo(&saida), 1, "erro de configuração tem de sair com 1");
    let texto_saida = texto(&saida.stdout);
    assert!(texto_saida.contains("Org ruim: erro"), "{texto_saida}");
    assert!(texto_saida.contains("Nada foi apagado"), "{texto_saida}");
}

#[test]
fn org_inexistente_sai_com_1_com_mensagem_clara() {
    let cenario = Cenario::novo("inexistente");
    cenario.criar_org("existe");

    let saida = cenario.poph(&["--org", "nao-existe", "retention", "run"]);

    assert_eq!(codigo(&saida), 1);
    assert!(
        texto(&saida.stderr).contains("não existe org com o slug `nao-existe`"),
        "{}",
        texto(&saida.stderr)
    );
}

#[test]
fn show_mostra_padrao_e_retencao_zero_da_org() {
    let cenario = Cenario::novo("show");
    cenario.criar_org("guarda-tudo");
    cenario.criar_org("padrao");
    cenario.configurar("guarda-tudo", r#"{"retencao":{"dias":0}}"#);

    let saida = cenario.poph(&["retention", "show"]);

    assert_eq!(codigo(&saida), 0, "{}", texto(&saida.stderr));
    let texto_saida = texto(&saida.stdout);
    assert!(
        texto_saida.contains("Org guarda-tudo: nunca apagar (definida na org)"),
        "{texto_saida}"
    );
    assert!(
        texto_saida.contains("Org padrao: 30 dias (padrão da instância)"),
        "{texto_saida}"
    );
}

#[test]
fn show_em_json_com_org_invalida_sai_com_1() {
    let cenario = Cenario::novo("show-json");
    cenario.criar_org("ruim");
    cenario.configurar("ruim", r#"{"retencao":{"dias":"trinta"}}"#);

    let saida = cenario.poph(&["retention", "show", "--json"]);

    assert_eq!(codigo(&saida), 1);
    let valor = json(&saida);
    assert_eq!(valor["versao_relatorio"], 1);
    assert_eq!(valor["padrao_dias"], 30);
    assert_eq!(valor["orgs"][0]["slug"], "ruim");
    assert!(valor["orgs"][0]["dias"].is_null());
    assert!(
        valor["orgs"][0]["erro"]
            .as_str()
            .expect("mensagem de erro")
            .contains("Nada foi apagado")
    );
}

// ───── O que nunca sai: Ficha, execução, etapa, evento e efeito (FX-34) ─────

/// Tabelas do módulo de Fluxos e Fichas: a varredura não pode perder linha de nenhuma.
const TABELAS_DO_MODULO: &[&str] = &[
    "fluxos",
    "fluxo_revisoes",
    "agente_fluxos",
    "fluxo_execucoes",
    "fluxo_execucao_etapas",
    "ficha_eventos",
    "fluxo_efeitos",
    "fichas",
];

const DIA_MS: i64 = 86_400_000;

fn agora_ms() -> i64 {
    let desde = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("relógio depois de 1970");
    i64::try_from(desde.as_millis()).expect("instante em ms")
}

/// Retrato das tabelas do módulo, linha a linha, para comparar antes e depois.
fn foto(conexao: &rusqlite::Connection, tabelas: &[&str]) -> Vec<(String, Vec<String>)> {
    let mut retrato = Vec::new();
    for tabela in tabelas {
        let mut consulta = conexao
            .prepare(&format!("SELECT * FROM {tabela} ORDER BY rowid"))
            .expect("consulta da foto");
        let colunas = consulta.column_count();
        let linhas = consulta
            .query_map([], |linha| {
                let mut partes = Vec::with_capacity(colunas);
                for i in 0..colunas {
                    let valor: rusqlite::types::Value = linha.get(i)?;
                    partes.push(format!("{valor:?}"));
                }
                Ok(partes.join("|"))
            })
            .expect("linhas da foto")
            .collect::<Result<Vec<_>, _>>()
            .expect("leitura da foto");
        retrato.push(((*tabela).to_owned(), linhas));
    }
    retrato
}

fn conexao(dados: &std::path::Path) -> rusqlite::Connection {
    let conexao = rusqlite::Connection::open(dados.join("harness.db")).expect("abre o banco");
    conexao
        .execute_batch("PRAGMA foreign_keys = ON;")
        .expect("chaves estrangeiras");
    conexao
}

/// `poph retention run` (e o `--dry-run`) apaga a conversa vazia e antiga, mas não toca em nada do
/// atendimento registrado. Depois da varredura, a Ficha fica com `conversation_id` nulo.
#[test]
fn run_nao_apaga_ficha_execucao_evento_nem_efeito() {
    use poph_store::{
        ExecutionStart, ExecutionState, NewConversation, NewFlow, RevisionPublication, Store,
        TransitionEffect,
    };
    use poph_types::{EventActor, ExecutionStatus};

    let cenario = Cenario::novo("fichas");
    let slug = "clinica-fichas";
    cenario.criar_org(slug);
    let dados = cenario.dados();

    // Semeia o atendimento pelo repositório, no mesmo banco que a CLI vai varrer.
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
        let revisao = store
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
                    revision: revisao.id,
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
        // Envelhece a conversa: é a conversa vazia e antiga que a varredura apaga.
        let conexao = conexao(&dados);
        conexao
            .execute(
                "UPDATE conversations SET created_at = ?1, updated_at = ?1",
                rusqlite::params![agora_ms() - 400 * DIA_MS],
            )
            .expect("envelhece a conversa");
    }

    let antes = {
        let conexao = conexao(&dados);
        foto(&conexao, TABELAS_DO_MODULO)
    };

    // `--dry-run` não apaga nada.
    let simulacao = cenario.poph(&["--org", slug, "retention", "run", "--dry-run", "--json"]);
    assert_eq!(codigo(&simulacao), 0, "{}", texto(&simulacao.stderr));
    assert_eq!(json(&simulacao)["orgs"][0]["banco"]["conversas"], 1);
    {
        let conexao = conexao(&dados);
        assert_eq!(
            foto(&conexao, TABELAS_DO_MODULO),
            antes,
            "a simulação não pode mexer no módulo de Fluxos e Fichas"
        );
        let conversas: i64 = conexao
            .query_row("SELECT COUNT(*) FROM conversations", [], |linha| {
                linha.get(0)
            })
            .expect("conta conversas");
        assert_eq!(conversas, 1, "a simulação não apaga a conversa");
    }

    // A varredura de verdade apaga a conversa e preserva o atendimento registrado.
    let real = cenario.poph(&["--org", slug, "retention", "run", "--json"]);
    assert_eq!(codigo(&real), 0, "{}", texto(&real.stderr));
    assert_eq!(json(&real)["orgs"][0]["banco"]["conversas"], 1);
    {
        let conexao = conexao(&dados);
        let conversas: i64 = conexao
            .query_row("SELECT COUNT(*) FROM conversations", [], |linha| {
                linha.get(0)
            })
            .expect("conta conversas");
        assert_eq!(conversas, 0, "a conversa vazia e antiga sai");
        let (situacao, conversa): (String, Option<String>) = conexao
            .query_row("SELECT status, conversation_id FROM fichas", [], |linha| {
                Ok((linha.get(0)?, linha.get(1)?))
            })
            .expect("Ficha depois da varredura");
        assert_eq!(situacao, "open");
        assert_eq!(conversa, None, "a Ficha sobrevive sem o vínculo");
        // Único campo que muda de propósito: o vínculo da Ficha com a conversa apagada.
        let sem_vinculo: Vec<(String, Vec<String>)> = foto(&conexao, TABELAS_DO_MODULO)
            .into_iter()
            .filter(|(tabela, _)| tabela != "fichas")
            .collect();
        assert_eq!(
            sem_vinculo,
            antes
                .into_iter()
                .filter(|(tabela, _)| tabela != "fichas")
                .collect::<Vec<_>>(),
            "Ficha, execução, etapa, evento e efeito não podem perder linha"
        );
    }
}
