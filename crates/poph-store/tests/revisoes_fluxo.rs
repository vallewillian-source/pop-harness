//! Testes da publicação de revisões (FX-09, PRD §6.2 e §7.1).
//!
//! O que fica provado aqui:
//!
//! - **publicação**: uma escrita cria a revisão numerada, atualiza `revisao_atual` e o rascunho,
//!   avança a `version` do fluxo e grava `flow_publish` no `audit_log` com número, id da revisão,
//!   `schema_versao`, hash do texto e ator;
//! - **corrida**: duas publicações com a mesma versão esperada — só uma vence, a outra recebe
//!   `ConflitoDeVersao` (412) e não fica revisão órfã nem auditoria da perdedora (inclusive com
//!   duas threads publicando ao mesmo tempo);
//! - **imutabilidade**: a revisão 1 continua idêntica byte a byte depois da 2 (texto, hash e IR), e
//!   alterar ou apagar revisão publicada é recusado pelo banco (gatilhos `tr_fluxo_revisoes_sem_update`
//!   e `tr_fluxo_revisoes_sem_delete`);
//! - **atomicidade**: falha injetada **depois** da auditoria (no `INSERT` da revisão) não deixa nem
//!   a revisão, nem a auditoria, nem a mudança no fluxo;
//! - **recusas**: IR sem `schema_versao`, `schema_versao` zero, IR que não é objeto JSON, IR com
//!   `schema_versao` diferente do informado, `fonte` em branco e ator fora do vocabulário de
//!   `audit_log.actor_kind` — todas sem gravar nada;
//! - **leitura**: revisão por número (e o `NaoEncontrado` da que não existe), revisão atual (nula
//!   antes da primeira publicação) e lista com cursor por número;
//! - **org**: fluxo de outra org não é lido nem publicado.

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use poph_store::{
    ACAO_PUBLICAR, FlowRevision, Fluxo, NewFlow, NewOrg, Org, RevisionPublication, Store,
    StoreError,
};
use poph_types::{DomainError, FlowId, OrgId};
use rusqlite::Connection;

/// Texto da revisão 1 e o sha256 dele (hexadecimal minúsculo), para conferir o hash gravado.
const FONTE_1: &str = "fluxo(id = \"agendar_visita\")\n";
const SHA_1: &str = "966b85be2ca12cf68d88a35fd4dbfc31f514abdbd61bd05aeeb9977441469f3c";
/// Texto da revisão 2 e o sha256 dele.
const FONTE_2: &str = "fluxo(id = \"agendar_visita\")\netapa(\"inicio\")";
const SHA_2: &str = "c12a81f87d880cd496796e09f68f93c803c1e0dd6d0ad341dac0a391694fa62f";
/// IR mínima, no formato que a API grava (o repositório não compila: recebe pronta).
const IR_1: &str = "{\"schema_versao\":1,\"etapas\":[{\"id\":\"inicio\"}]}";
/// IR da revisão 2.
const IR_2: &str = "{\"schema_versao\":1,\"etapas\":[{\"id\":\"inicio\"},{\"id\":\"fim\"}]}";

/// Banco de verdade numa pasta temporária, apagada no fim.
struct Cenario {
    pasta: PathBuf,
    store: Option<Store>,
}

impl Cenario {
    fn novo(rotulo: &str) -> Self {
        static CONTADOR: AtomicUsize = AtomicUsize::new(0);
        let n = CONTADOR.fetch_add(1, Ordering::Relaxed);
        let pasta =
            std::env::temp_dir().join(format!("poph-revisoes-{rotulo}-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&pasta);
        fs::create_dir_all(&pasta).expect("cria a pasta do teste");
        let store =
            Store::open_at(&pasta.join("harness.db"), &pasta.join("rag.db")).expect("abre o banco");
        Self {
            pasta,
            store: Some(store),
        }
    }

    fn store(&self) -> &Store {
        self.store.as_ref().expect("store viva")
    }

    /// Conexão própria, para conferir o banco por SQL e para injetar gatilho de falha.
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

    fn fluxo(&self, org: &Org, slug: &str) -> Fluxo {
        self.store()
            .fluxos()
            .criar(
                org.id,
                NewFlow {
                    slug: slug.to_owned(),
                    name: format!("Fluxo {slug}"),
                    description: String::new(),
                    rascunho_fonte: "rascunho ainda inválido".to_owned(),
                },
            )
            .expect("cria o fluxo")
    }

    /// Publicação com os valores de sempre; o teste troca o que interessa.
    fn publicacao(esperada: u64, fonte: &str, ir: &str) -> RevisionPublication {
        RevisionPublication {
            expected: esperada,
            fonte: fonte.to_owned(),
            definicao_json: ir.to_owned(),
            schema_versao: Some(1),
            resolucoes_json: "{\"departamento_comercial\":\"dep-1\"}".to_owned(),
            ator_tipo: "admin_token".to_owned(),
            actor_id: Some("tok_1".to_owned()),
        }
    }

    fn publicar(
        &self,
        org: &Org,
        fluxo: &Fluxo,
        esperada: u64,
        fonte: &str,
        ir: &str,
    ) -> Result<FlowRevision, StoreError> {
        self.store().revisions().publicar_revisao(
            org.id,
            fluxo.id,
            Self::publicacao(esperada, fonte, ir),
        )
    }

    /// Fluxo relido do banco (versão, revisão atual e rascunho).
    fn fluxo_atual(&self, org: &Org, fluxo: &Fluxo) -> Fluxo {
        self.store()
            .fluxos()
            .obter(org.id, fluxo.id)
            .expect("lê o fluxo")
    }

    fn contar(&self, conn: &Connection, sql: &str, org: OrgId, fluxo: FlowId) -> i64 {
        conn.query_row(
            sql,
            rusqlite::params![org.to_string(), fluxo.to_string()],
            |linha| linha.get(0),
        )
        .expect("conta")
    }

    /// Quantas revisões o fluxo tem.
    fn revisoes(&self, conn: &Connection, org: &Org, fluxo: &Fluxo) -> i64 {
        self.contar(
            conn,
            "SELECT COUNT(*) FROM fluxo_revisoes WHERE org_id = ?1 AND fluxo_id = ?2",
            org.id,
            fluxo.id,
        )
    }

    /// Quantas auditorias de publicação o fluxo tem.
    fn auditorias(&self, conn: &Connection, org: &Org, fluxo: &Fluxo) -> i64 {
        self.contar(
            conn,
            "SELECT COUNT(*) FROM audit_log
              WHERE org_id = ?1 AND entity_id = ?2 AND action = 'flow_publish'",
            org.id,
            fluxo.id,
        )
    }
}

impl Drop for Cenario {
    fn drop(&mut self) {
        self.store = None;
        let _ = fs::remove_dir_all(&self.pasta);
    }
}

#[test]
fn publicar_grava_revisao_um_e_atualiza_o_fluxo() {
    let cenario = Cenario::novo("primeira");
    let org = cenario.org("a");
    let fluxo = cenario.fluxo(&org, "agendar_visita");

    let revisao = cenario
        .publicar(&org, &fluxo, 1, FONTE_1, IR_1)
        .expect("publica a revisão 1");

    assert_eq!(revisao.numero, 1);
    assert_eq!(revisao.fonte, FONTE_1);
    assert_eq!(revisao.fonte_sha256, SHA_1, "o hash é do texto exato");
    assert_eq!(revisao.definicao_json, IR_1);
    assert_eq!(revisao.schema_versao, 1);
    assert_eq!(revisao.publicada_por, "admin_token");
    assert_eq!(
        revisao.resolucoes_json,
        "{\"departamento_comercial\":\"dep-1\"}"
    );
    assert!(revisao.publicada_em > 0);
    assert_eq!(revisao.org_id, org.id);
    assert_eq!(revisao.fluxo_id, fluxo.id);

    let depois = cenario.fluxo_atual(&org, &fluxo);
    assert_eq!(depois.revisao_atual, Some(1));
    assert_eq!(
        depois.rascunho_fonte, FONTE_1,
        "o rascunho vira o publicado"
    );
    assert_eq!(depois.version, 2, "publicar avança a versão do fluxo");

    // A revisão está no banco com os mesmos bytes que voltaram.
    let conn = cenario.conexao();
    let (id, fonte, sha, ir): (String, String, String, String) = conn
        .query_row(
            "SELECT id, fonte, fonte_sha256, definicao_json FROM fluxo_revisoes
              WHERE org_id = ?1 AND fluxo_id = ?2 AND numero = 1",
            rusqlite::params![org.id.to_string(), fluxo.id.to_string()],
            |linha| Ok((linha.get(0)?, linha.get(1)?, linha.get(2)?, linha.get(3)?)),
        )
        .expect("lê a revisão gravada");
    assert_eq!(id, revisao.id.to_string());
    assert_eq!(fonte, FONTE_1);
    assert_eq!(sha, SHA_1);
    assert_eq!(ir, IR_1);
}

#[test]
fn auditoria_da_publicacao_guarda_acao_entidade_e_detalhe() {
    let cenario = Cenario::novo("auditoria");
    let org = cenario.org("a");
    let fluxo = cenario.fluxo(&org, "agendar_visita");
    let revisao = cenario
        .publicar(&org, &fluxo, 1, FONTE_1, IR_1)
        .expect("publica");

    let conn = cenario.conexao();
    let (acao, entidade, ator_tipo, ator_id, detalhe): (
        String,
        String,
        String,
        Option<String>,
        String,
    ) = conn
        .query_row(
            "SELECT action, entity, actor_kind, actor_id, detail_json FROM audit_log
              WHERE org_id = ?1 AND entity_id = ?2",
            rusqlite::params![org.id.to_string(), fluxo.id.to_string()],
            |linha| {
                Ok((
                    linha.get(0)?,
                    linha.get(1)?,
                    linha.get(2)?,
                    linha.get(3)?,
                    linha.get(4)?,
                ))
            },
        )
        .expect("lê a auditoria");
    assert_eq!(acao, ACAO_PUBLICAR);
    assert_eq!(acao, "flow_publish");
    assert_eq!(entidade, "flow");
    assert_eq!(ator_tipo, "admin_token");
    assert_eq!(ator_id.as_deref(), Some("tok_1"));
    // O detalhe é JSON com o número, o id da revisão, a versão do esquema e o hash do texto.
    // (`serde_json::json!` grava as chaves em ordem alfabética, então o texto é estável.)
    for esperado in [
        "\"fonte_sha256\":\"966b85be2ca12cf68d88a35fd4dbfc31f514abdbd61bd05aeeb9977441469f3c\"",
        "\"revisao\":1",
        "\"schema_versao\":1",
    ] {
        assert!(
            detalhe.contains(esperado),
            "o detalhe da auditoria tem `{esperado}`: {detalhe}"
        );
    }
    assert!(
        detalhe.contains(&format!("\"revisao_id\":\"{}\"", revisao.id)),
        "o detalhe aponta a revisão publicada: {detalhe}"
    );
}

#[test]
fn duas_publicacoes_com_a_mesma_versao_esperada_so_uma_vence() {
    let cenario = Cenario::novo("corrida");
    let org = cenario.org("a");
    let fluxo = cenario.fluxo(&org, "agendar_visita");

    let primeira = cenario
        .publicar(&org, &fluxo, 1, FONTE_1, IR_1)
        .expect("a primeira vence");
    assert_eq!(primeira.numero, 1);

    // A segunda leu a versão 1 antes da primeira publicar: chega com o mesmo `If-Match`.
    let erro = cenario
        .publicar(&org, &fluxo, 1, FONTE_2, IR_2)
        .expect_err("a segunda perde");
    assert!(
        matches!(
            erro,
            StoreError::Domain(DomainError::VersionConflict {
                entity: "fluxo",
                expected: 1,
                current: 2,
                ..
            })
        ),
        "esperava conflito de versão em fluxo, veio {erro:?}"
    );

    let conn = cenario.conexao();
    assert_eq!(cenario.revisoes(&conn, &org, &fluxo), 1, "sem revisão órfã");
    assert_eq!(
        cenario.auditorias(&conn, &org, &fluxo),
        1,
        "sem auditoria da perdedora"
    );
    let depois = cenario.fluxo_atual(&org, &fluxo);
    assert_eq!(depois.revisao_atual, Some(1));
    assert_eq!(depois.version, 2, "a versão avançou uma vez só");
    assert_eq!(
        depois.rascunho_fonte, FONTE_1,
        "o rascunho é o da vencedora"
    );
}

#[test]
fn duas_threads_publicando_com_a_mesma_versao_so_uma_vence() {
    let cenario = Cenario::novo("corrida-threads");
    let org = cenario.org("a");
    let fluxo = cenario.fluxo(&org, "agendar_visita");
    let store = Arc::new(
        Store::open_at(
            &cenario.pasta.join("harness.db"),
            &cenario.pasta.join("rag.db"),
        )
        .expect("abre um segundo store no mesmo banco"),
    );

    // Cada thread publica pelo seu store, com a mesma versão esperada: a corrida é real, entre dois
    // escritores do mesmo banco.
    let mut handles = Vec::new();
    for (indice, fonte) in [(1, FONTE_1), (2, FONTE_2)] {
        let store = Arc::clone(&store);
        let org_id = org.id;
        let fluxo_id = fluxo.id;
        handles.push(std::thread::spawn(move || {
            store
                .revisions()
                .publicar_revisao(
                    org_id,
                    fluxo_id,
                    Cenario::publicacao(1, fonte, if indice == 1 { IR_1 } else { IR_2 }),
                )
                .map(|revisao| revisao.numero)
        }));
    }
    let resultados: Vec<Result<u64, StoreError>> = handles
        .into_iter()
        .map(|handle| handle.join().expect("a thread termina"))
        .collect();

    let vencedoras = resultados
        .iter()
        .filter(|resultado| resultado.is_ok())
        .count();
    let perdedoras = resultados
        .iter()
        .filter(|resultado| {
            matches!(
                resultado,
                Err(StoreError::Domain(DomainError::VersionConflict { .. }))
            )
        })
        .count();
    assert_eq!(vencedoras, 1, "só uma publicação vence: {resultados:?}");
    assert_eq!(perdedoras, 1, "a outra recebe 412: {resultados:?}");

    let conn = cenario.conexao();
    assert_eq!(cenario.revisoes(&conn, &org, &fluxo), 1);
    assert_eq!(cenario.auditorias(&conn, &org, &fluxo), 1);
}

#[test]
fn revisao_um_continua_identica_depois_da_dois() {
    let cenario = Cenario::novo("imutavel");
    let org = cenario.org("a");
    let fluxo = cenario.fluxo(&org, "agendar_visita");
    let primeira = cenario
        .publicar(&org, &fluxo, 1, FONTE_1, IR_1)
        .expect("publica a 1");
    let segunda = cenario
        .publicar(&org, &fluxo, 2, FONTE_2, IR_2)
        .expect("publica a 2");

    assert_eq!(primeira.numero, 1);
    assert_eq!(segunda.numero, 2, "as duas coexistem, numeradas em ordem");
    assert_eq!(segunda.fonte, FONTE_2);
    assert_eq!(segunda.fonte_sha256, SHA_2);
    assert_ne!(primeira.id, segunda.id);

    // A 1 é lida de novo: texto, hash e IR byte a byte iguais aos da publicação.
    let relida = cenario
        .store()
        .revisions()
        .obter(org.id, fluxo.id, 1)
        .expect("lê a revisão 1");
    assert_eq!(relida, primeira);
    assert_eq!(relida.fonte, FONTE_1);
    assert_eq!(relida.fonte_sha256, SHA_1);
    assert_eq!(relida.definicao_json, IR_1);

    // E no banco também (nem o texto, nem o hash, nem a IR mudaram).
    let conn = cenario.conexao();
    let (fonte, sha, ir): (String, String, String) = conn
        .query_row(
            "SELECT fonte, fonte_sha256, definicao_json FROM fluxo_revisoes
              WHERE org_id = ?1 AND fluxo_id = ?2 AND numero = 1",
            rusqlite::params![org.id.to_string(), fluxo.id.to_string()],
            |linha| Ok((linha.get(0)?, linha.get(1)?, linha.get(2)?)),
        )
        .expect("lê a revisão 1 gravada");
    assert_eq!(fonte, FONTE_1);
    assert_eq!(sha, SHA_1);
    assert_eq!(ir, IR_1);

    let depois = cenario.fluxo_atual(&org, &fluxo);
    assert_eq!(depois.revisao_atual, Some(2), "a atual passa a ser a 2");
    assert_eq!(depois.version, 3);
}

#[test]
fn alterar_ou_apagar_revisao_publicada_e_recusado_pelo_banco() {
    let cenario = Cenario::novo("protegida");
    let org = cenario.org("a");
    let fluxo = cenario.fluxo(&org, "agendar_visita");
    let revisao = cenario
        .publicar(&org, &fluxo, 1, FONTE_1, IR_1)
        .expect("publica");
    let conn = cenario.conexao();
    let texto_id = revisao.id.to_string();

    // O repositório não tem operação de alteração: a única escrita de revisão é o `INSERT` da
    // publicação. Quem tentar por SQL direto é barrado pelo gatilho do banco.
    let erro = conn
        .execute(
            "UPDATE fluxo_revisoes SET fonte = 'outra coisa' WHERE id = ?1",
            rusqlite::params![texto_id],
        )
        .expect_err("o gatilho recusa alterar revisão publicada");
    assert!(
        format!("{erro}").contains("tr_fluxo_revisoes_sem_update")
            || format!("{erro}").contains("imutável"),
        "o erro vem do gatilho de imutabilidade: {erro}"
    );

    let erro = conn
        .execute(
            "DELETE FROM fluxo_revisoes WHERE id = ?1",
            rusqlite::params![texto_id],
        )
        .expect_err("o gatilho recusa apagar revisão publicada");
    assert!(
        format!("{erro}").contains("tr_fluxo_revisoes_sem_delete")
            || format!("{erro}").contains("imutável"),
        "o erro vem do gatilho de imutabilidade: {erro}"
    );

    // A revisão continua como foi publicada.
    let relida = cenario
        .store()
        .revisions()
        .obter(org.id, fluxo.id, 1)
        .expect("a revisão continua lá");
    assert_eq!(relida, revisao);

    // Derrubados os dois gatilhos, o mesmo `UPDATE` passa: é o gatilho que barra, não outra coisa
    // (é a mutação que prova este teste).
    conn.execute_batch(
        "DROP TRIGGER tr_fluxo_revisoes_sem_update;
         DROP TRIGGER tr_fluxo_revisoes_sem_delete;",
    )
    .expect("derruba os gatilhos");
    let alteradas = conn
        .execute(
            "UPDATE fluxo_revisoes SET fonte = 'outra coisa' WHERE id = ?1",
            rusqlite::params![revisao.id.to_string()],
        )
        .expect("sem o gatilho, a alteração passa");
    assert_eq!(alteradas, 1);
}

#[test]
fn falha_depois_da_auditoria_nao_deixa_revisao_nem_auditoria() {
    let cenario = Cenario::novo("atomica");
    let org = cenario.org("a");
    let fluxo = cenario.fluxo(&org, "agendar_visita");

    // Falha no último passo da escrita (o `INSERT` da revisão), depois da auditoria e depois de o
    // fluxo já ter sido atualizado na transação.
    {
        let conn = cenario.conexao();
        conn.execute_batch(
            "CREATE TRIGGER falha_injetada BEFORE INSERT ON fluxo_revisoes
             BEGIN SELECT RAISE(ABORT, 'falha injetada no teste'); END;",
        )
        .expect("cria o gatilho da falha");
    }

    let erro = cenario
        .publicar(&org, &fluxo, 1, FONTE_1, IR_1)
        .expect_err("a publicação falha no meio");
    assert!(
        format!("{erro}").contains("falha injetada"),
        "o erro do gatilho aparece: {erro}"
    );

    let conn = cenario.conexao();
    conn.execute_batch("DROP TRIGGER falha_injetada;")
        .expect("remove o gatilho");

    // Nem a revisão, nem a auditoria, nem a mudança no fluxo ficaram.
    assert_eq!(cenario.revisoes(&conn, &org, &fluxo), 0, "sem revisão");
    assert_eq!(cenario.auditorias(&conn, &org, &fluxo), 0, "sem auditoria");
    let depois = cenario.fluxo_atual(&org, &fluxo);
    assert_eq!(depois.revisao_atual, None, "revisão atual continua nula");
    assert_eq!(depois.version, 1, "a versão não avançou");
    assert_eq!(
        depois.rascunho_fonte, "rascunho ainda inválido",
        "o rascunho não foi trocado"
    );

    // E a publicação seguinte, sem o gatilho, entra normalmente.
    let revisao = cenario
        .publicar(&org, &fluxo, 1, FONTE_1, IR_1)
        .expect("depois da falha, publica");
    assert_eq!(revisao.numero, 1);
}

#[test]
fn ir_sem_schema_versao_e_recusada_sem_gravar_nada() {
    let cenario = Cenario::novo("sem-schema");
    let org = cenario.org("a");
    let fluxo = cenario.fluxo(&org, "agendar_visita");

    let mut sem_versao = Cenario::publicacao(1, FONTE_1, IR_1);
    sem_versao.schema_versao = None;
    let erro = cenario
        .store()
        .revisions()
        .publicar_revisao(org.id, fluxo.id, sem_versao)
        .expect_err("sem schema_versao não publica");
    assert!(
        matches!(
            erro,
            StoreError::Domain(DomainError::Validation {
                field: "schema_versao",
                ..
            })
        ),
        "esperava validação de schema_versao, veio {erro:?}"
    );

    let mut versao_zero = Cenario::publicacao(1, FONTE_1, IR_1);
    versao_zero.schema_versao = Some(0);
    let erro = cenario
        .store()
        .revisions()
        .publicar_revisao(org.id, fluxo.id, versao_zero)
        .expect_err("schema_versao zero não publica");
    assert!(
        matches!(
            erro,
            StoreError::Domain(DomainError::Validation {
                field: "schema_versao",
                ..
            })
        ),
        "esperava validação de schema_versao, veio {erro:?}"
    );

    let conn = cenario.conexao();
    assert_eq!(cenario.revisoes(&conn, &org, &fluxo), 0);
    assert_eq!(cenario.auditorias(&conn, &org, &fluxo), 0);
    assert_eq!(cenario.fluxo_atual(&org, &fluxo).version, 1);
}

#[test]
fn ir_incoerente_fonte_vazia_e_ator_desconhecido_sao_recusados() {
    let cenario = Cenario::novo("recusas");
    let org = cenario.org("a");
    let fluxo = cenario.fluxo(&org, "agendar_visita");

    /// Uma publicação boa e o que cada caso troca nela.
    fn casos() -> Vec<(&'static str, &'static str, RevisionPublication)> {
        let mut ir_torta = Cenario::publicacao(1, FONTE_1, IR_1);
        ir_torta.definicao_json = "ir torta".to_owned();
        let mut ir_de_outra_versao = Cenario::publicacao(1, FONTE_1, IR_1);
        ir_de_outra_versao.definicao_json = "{\"schema_versao\":2}".to_owned();
        let mut sem_texto = Cenario::publicacao(1, FONTE_1, IR_1);
        sem_texto.fonte = "   ".to_owned();
        let mut ator_desconhecido = Cenario::publicacao(1, FONTE_1, IR_1);
        ator_desconhecido.ator_tipo = "robo".to_owned();
        let mut resolucoes_que_nao_sao_objeto = Cenario::publicacao(1, FONTE_1, IR_1);
        resolucoes_que_nao_sao_objeto.resolucoes_json = "[1, 2]".to_owned();
        vec![
            ("IR que não é JSON", "definicao_json", ir_torta),
            (
                "IR que declara outra versão de esquema",
                "schema_versao",
                ir_de_outra_versao,
            ),
            ("texto em branco", "fonte", sem_texto),
            ("ator fora do vocabulário", "ator_tipo", ator_desconhecido),
            (
                "resoluções que não são objeto",
                "resolucoes_json",
                resolucoes_que_nao_sao_objeto,
            ),
        ]
    }

    for (rotulo, campo, publicacao) in casos() {
        let erro = cenario
            .store()
            .revisions()
            .publicar_revisao(org.id, fluxo.id, publicacao)
            .expect_err("a publicação devia ser recusada");
        match erro {
            StoreError::Domain(DomainError::Validation {
                field: nomeado,
                reason: motivo,
            }) => {
                assert_eq!(nomeado, campo, "campo recusado em `{rotulo}`");
                assert!(!motivo.is_empty(), "`{rotulo}` diz o motivo");
                if campo == "ator_tipo" {
                    // A recusa cita os tipos aceitos: é a conferência do repositório, não só o
                    // `CHECK` de `audit_log` (que recusaria depois, com outra mensagem).
                    assert!(
                        motivo.contains("admin_token"),
                        "a recusa do ator lista o vocabulário: {motivo}"
                    );
                }
            }
            outro => panic!("`{rotulo}` devia ser validação de `{campo}`, veio {outro:?}"),
        }
    }

    let conn = cenario.conexao();
    assert_eq!(cenario.revisoes(&conn, &org, &fluxo), 0);
    assert_eq!(cenario.auditorias(&conn, &org, &fluxo), 0);
    assert_eq!(cenario.fluxo_atual(&org, &fluxo).version, 1);
}

#[test]
fn resolucoes_vazias_viram_objeto_vazio() {
    let cenario = Cenario::novo("resolucoes");
    let org = cenario.org("a");
    let fluxo = cenario.fluxo(&org, "agendar_visita");

    let mut publicacao = Cenario::publicacao(1, FONTE_1, IR_1);
    publicacao.resolucoes_json = "  ".to_owned();
    let revisao = cenario
        .store()
        .revisions()
        .publicar_revisao(org.id, fluxo.id, publicacao)
        .expect("publica");
    assert_eq!(revisao.resolucoes_json, "{}");
}

#[test]
fn le_revisao_por_numero_a_atual_e_a_lista() {
    let cenario = Cenario::novo("leitura");
    let org = cenario.org("a");
    let fluxo = cenario.fluxo(&org, "agendar_visita");

    // Antes da primeira publicação: sem revisão atual e a lista vazia.
    assert_eq!(
        cenario
            .store()
            .revisions()
            .atual(org.id, fluxo.id)
            .expect("consulta"),
        None
    );
    assert!(
        cenario
            .store()
            .revisions()
            .listar(org.id, fluxo.id, None, 10)
            .expect("lista")
            .is_empty()
    );

    let primeira = cenario.publicar(&org, &fluxo, 1, FONTE_1, IR_1).expect("1");
    let segunda = cenario.publicar(&org, &fluxo, 2, FONTE_2, IR_2).expect("2");

    assert_eq!(
        cenario
            .store()
            .revisions()
            .obter(org.id, fluxo.id, 1)
            .expect("por número"),
        primeira
    );
    assert_eq!(
        cenario
            .store()
            .revisions()
            .atual(org.id, fluxo.id)
            .expect("atual")
            .expect("tem atual"),
        segunda
    );
    let erro = cenario
        .store()
        .revisions()
        .obter(org.id, fluxo.id, 9)
        .expect_err("a 9 não existe");
    assert!(
        matches!(
            erro,
            StoreError::Domain(DomainError::NotFound {
                entity: "revisão de fluxo",
                ..
            })
        ),
        "esperava não encontrado, veio {erro:?}"
    );

    let todas = cenario
        .store()
        .revisions()
        .listar(org.id, fluxo.id, None, 10)
        .expect("lista");
    assert_eq!(todas, vec![primeira.clone(), segunda.clone()]);
    let depois_da_um = cenario
        .store()
        .revisions()
        .listar(org.id, fluxo.id, Some(1), 10)
        .expect("lista com cursor");
    assert_eq!(depois_da_um, vec![segunda]);
}

#[test]
fn fluxo_de_outra_org_nao_e_lido_nem_publicado() {
    let cenario = Cenario::novo("org");
    let org_a = cenario.org("a");
    let org_b = cenario.org("b");
    let fluxo = cenario.fluxo(&org_a, "agendar_visita");
    cenario
        .publicar(&org_a, &fluxo, 1, FONTE_1, IR_1)
        .expect("1");

    let erro = cenario
        .publicar(&org_b, &fluxo, 2, FONTE_2, IR_2)
        .expect_err("publicar em outra org é recusado");
    assert!(erro.is_not_found(), "esperava não encontrado: {erro:?}");

    for erro in [
        cenario
            .store()
            .revisions()
            .obter(org_b.id, fluxo.id, 1)
            .expect_err("obter em outra org"),
        cenario
            .store()
            .revisions()
            .atual(org_b.id, fluxo.id)
            .expect_err("atual em outra org"),
        cenario
            .store()
            .revisions()
            .listar(org_b.id, fluxo.id, None, 10)
            .expect_err("listar em outra org"),
    ] {
        // A recusa é do **fluxo** (não da revisão): quem chamou pediu um fluxo de outra org.
        match erro {
            StoreError::Domain(DomainError::NotFound {
                entity: entidade, ..
            }) => {
                assert_eq!(entidade, "fluxo", "a recusa é do fluxo");
            }
            outro => panic!("esperava não encontrado do fluxo, veio {outro:?}"),
        }
    }

    // A revisão da org A continua única e intacta.
    let conn = cenario.conexao();
    assert_eq!(cenario.revisoes(&conn, &org_a, &fluxo), 1);
    assert_eq!(cenario.fluxo_atual(&org_a, &fluxo).revisao_atual, Some(1));
}
