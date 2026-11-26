//! Testes da migração das tabelas de fluxos, revisões, permissão, execuções, etapas e efeitos
//! (FX-06).
//!
//! Provam o que o banco tem que garantir sozinho, sem depender do repositório (que é a FX-07):
//! as seis tabelas do PRD §7.1 com as colunas certas; **uma** execução ativa por Ficha; revisão
//! publicada que ninguém altera nem apaga solta (mas que vai junto quando o fluxo é apagado);
//! vínculo entre organizações diferentes recusado; apagar agente levando só a permissão; `slug`
//! único por org e `numero` único por fluxo; rodar a migração de novo sem efeito; e os textos das
//! colunas de situação sendo exatamente os do `poph-types` (FX-03).
//!
//! As asserções usam SQL direto de propósito: o repositório é de outra tarefa, e aqui o que está
//! sendo provado é o esquema.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use poph_store::{MIGRACOES_HARNESS, Store, versao_alvo_harness, versao_do_arquivo};
use poph_types::{EffectState, ExecutionStatus, StepStatus};
use rusqlite::Connection;

/// Nome da migração desta tarefa em `MIGRACOES_HARNESS`.
const MIGRACAO: &str = "0013_fluxos_revisoes_execucoes_e_efeitos";

fn pasta_nova(rotulo: &str) -> PathBuf {
    static CONTADOR: AtomicUsize = AtomicUsize::new(0);
    let n = CONTADOR.fetch_add(1, Ordering::Relaxed);
    let pasta =
        std::env::temp_dir().join(format!("poph-fluxos-{rotulo}-{}-{n}", std::process::id()));
    let _ = fs::remove_dir_all(&pasta);
    fs::create_dir_all(&pasta).expect("cria a pasta do teste");
    pasta
}

/// Conexão própria para conferir e semear o banco, com as chaves estrangeiras ligadas como no
/// `Store` (sem isso o SQLite ignora a FK silenciosamente e o teste passaria à toa).
fn conexao(caminho: &std::path::Path) -> Connection {
    let conn = Connection::open(caminho).expect("abre o banco para conferir");
    conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA busy_timeout = 2000;")
        .expect("pragmas");
    conn
}

fn contar(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |linha| linha.get(0))
        .expect("conta")
}

fn novo_id() -> String {
    uuid::Uuid::now_v7().to_string()
}

/// Banco novo já migrado, com conexão de conferência e o `Store` vivo (para ele não sumir da
/// pasta durante o teste).
struct Cenario {
    pasta: PathBuf,
    _store: Store,
    conn: Connection,
}

impl Cenario {
    fn novo(rotulo: &str) -> Self {
        let pasta = pasta_nova(rotulo);
        let harness = pasta.join("harness.db");
        let store = Store::abrir_em(&harness, &pasta.join("rag.db")).expect("abre e migra");
        let conn = conexao(&harness);
        Self {
            pasta,
            _store: store,
            conn,
        }
    }

    fn org(&self, rotulo: &str) -> String {
        let org = novo_id();
        self.conn
            .execute(
                "INSERT INTO orgs (id, name, slug, created_at, updated_at) VALUES (?1, ?2, ?3, 1, 1)",
                rusqlite::params![org, format!("Org {rotulo}"), format!("org-{rotulo}")],
            )
            .expect("org");
        org
    }

    fn agente(&self, org: &str, rotulo: &str) -> String {
        let agente = novo_id();
        self.conn
            .execute(
                "INSERT INTO agents (id, org_id, name, slug, provider, model, created_at,
                                     updated_at)
                 VALUES (?1, ?2, ?3, ?4, 'antigravity2api', 'gemini', 1, 1)",
                rusqlite::params![
                    agente,
                    org,
                    format!("Agente {rotulo}"),
                    format!("agente-{rotulo}")
                ],
            )
            .expect("agente");
        agente
    }

    /// Ficha solta (sem conversa: a coluna aceita nulo e a FX-05 já provou esse caminho).
    fn ficha(&self, org: &str, agente: &str) -> String {
        let ficha = novo_id();
        self.conn
            .execute(
                "INSERT INTO fichas (id, org_id, conversation_id, agent_id, canal, status,
                                     aberta_em)
                 VALUES (?1, ?2, NULL, ?3, 'whatsapp', 'open', 1)",
                rusqlite::params![ficha, org, agente],
            )
            .expect("ficha");
        ficha
    }

    fn fluxo(&self, org: &str, slug: &str) -> String {
        let fluxo = novo_id();
        self.conn
            .execute(
                "INSERT INTO fluxos (id, org_id, slug, nome, descricao) VALUES (?1, ?2, ?3, ?4, '')",
                rusqlite::params![fluxo, org, slug, format!("Fluxo {slug}")],
            )
            .expect("fluxo");
        fluxo
    }

    fn revisao(&self, org: &str, fluxo: &str, numero: i64) -> String {
        let revisao = novo_id();
        self.conn
            .execute(
                "INSERT INTO fluxo_revisoes
                    (id, org_id, fluxo_id, numero, fonte, fonte_sha256, definicao_json,
                     publicada_em, publicada_por)
                 VALUES (?1, ?2, ?3, ?4, 'fluxo(id = \"x\")', 'sha', '{}', 1, 'cli')",
                rusqlite::params![revisao, org, fluxo, numero],
            )
            .expect("revisão");
        revisao
    }

    fn execucao(&self, org: &str, ficha: &str, fluxo: &str, revisao: &str) -> String {
        let execucao = novo_id();
        self.conn
            .execute(
                "INSERT INTO fluxo_execucoes
                    (id, org_id, ficha_id, fluxo_id, revisao_id, status, no_atual, iniciada_em)
                 VALUES (?1, ?2, ?3, ?4, ?5, 'ativa', 'inicio', 1)",
                rusqlite::params![execucao, org, ficha, fluxo, revisao],
            )
            .expect("execução");
        execucao
    }
}

impl Drop for Cenario {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.pasta);
    }
}

#[test]
fn migracao_cria_as_seis_tabelas_com_colunas_e_indices() {
    let cenario = Cenario::novo("esquema");
    let harness = cenario.pasta.join("harness.db");
    assert_eq!(
        versao_do_arquivo(&harness).expect("versão"),
        Some(versao_alvo_harness())
    );
    assert!(
        versao_alvo_harness() >= 13,
        "a migração 0013 tem que estar registrada"
    );
    assert!(
        MIGRACOES_HARNESS.iter().any(|m| m.nome == MIGRACAO),
        "migração {MIGRACAO} não está em MIGRACOES_HARNESS"
    );

    // Tabelas e colunas do PRD §7.1, sem faltar nem trocar de nome.
    let esperado: &[(&str, &[&str])] = &[
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
    for (tabela, colunas) in esperado {
        let do_banco = cenario._store.colunas(tabela).expect("colunas da tabela");
        for coluna in *colunas {
            assert!(
                do_banco.iter().any(|c| c == coluna),
                "tabela {tabela} sem a coluna {coluna}; tem {do_banco:?}"
            );
        }
    }

    // Índices que carregam regra (não só desempenho) e os de leitura que a tarefa pede.
    for indice in [
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
        let existe = contar(
            &cenario.conn,
            &format!(
                "SELECT count(*) FROM sqlite_master WHERE type = 'index' AND name = '{indice}'"
            ),
        );
        assert_eq!(existe, 1, "índice {indice} não foi criado");
    }

    // O índice novo de `fichas` cobre exatamente (org_id, agent_id, aberta_em), na ordem.
    let mut colunas = cenario
        .conn
        .prepare("PRAGMA index_info('ix_fichas_org_agente')")
        .expect("pragma index_info")
        .query_map([], |linha| linha.get::<_, String>(2))
        .expect("lê o índice")
        .collect::<Result<Vec<_>, _>>()
        .expect("colunas do índice");
    colunas.sort_by_key(|nome| match nome.as_str() {
        "org_id" => 0,
        "agent_id" => 1,
        _ => 2,
    });
    assert_eq!(
        colunas,
        vec!["org_id", "agent_id", "aberta_em"],
        "o índice do filtro por agente precisa ter as três colunas"
    );

    // Os dois gatilhos de imutabilidade existem.
    for gatilho in [
        "tr_fluxo_revisoes_sem_update",
        "tr_fluxo_revisoes_sem_delete",
    ] {
        let existe = contar(
            &cenario.conn,
            &format!(
                "SELECT count(*) FROM sqlite_master WHERE type = 'trigger' AND name = '{gatilho}'"
            ),
        );
        assert_eq!(existe, 1, "gatilho {gatilho} não foi criado");
    }
}

#[test]
fn banco_recusa_duas_execucoes_ativas_na_mesma_ficha() {
    let cenario = Cenario::novo("ativa");
    let org = cenario.org("a");
    let agente = cenario.agente(&org, "a");
    let fluxo = cenario.fluxo(&org, "agendar");
    let revisao = cenario.revisao(&org, &fluxo, 1);
    let ficha = cenario.ficha(&org, &agente);

    let primeira = cenario.execucao(&org, &ficha, &fluxo, &revisao);

    let erro = cenario
        .conn
        .execute(
            "INSERT INTO fluxo_execucoes
                (id, org_id, ficha_id, fluxo_id, revisao_id, status, no_atual, iniciada_em)
             VALUES (?1, ?2, ?3, ?4, ?5, 'ativa', 'inicio', 2)",
            rusqlite::params![novo_id(), org, ficha, fluxo, revisao],
        )
        .expect_err("a segunda execução ativa da mesma Ficha tem que ser recusada");
    assert!(
        erro.to_string().contains("UNIQUE"),
        "erro inesperado: {erro}"
    );

    // Encerrada a primeira, a Ficha aceita outra: o histórico se acumula.
    cenario
        .conn
        .execute(
            "UPDATE fluxo_execucoes SET status = 'concluida', finalizada_em = 5 WHERE id = ?1",
            [&primeira],
        )
        .expect("conclui a execução");
    cenario.execucao(&org, &ficha, &fluxo, &revisao);

    // Outra Ficha tem a própria execução ativa, e execuções de fluxos diferentes também coexistem
    // entre Fichas diferentes.
    let outra_ficha = cenario.ficha(&org, &agente);
    cenario.execucao(&org, &outra_ficha, &fluxo, &revisao);

    // Execução ativa não carrega instante de fim, e encerrada exige um.
    let erro = cenario
        .conn
        .execute(
            "INSERT INTO fluxo_execucoes
                (id, org_id, ficha_id, fluxo_id, revisao_id, status, no_atual, iniciada_em,
                 finalizada_em)
             VALUES (?1, ?2, ?3, ?4, ?5, 'ativa', 'inicio', 3, 9)",
            rusqlite::params![novo_id(), org, outra_ficha, fluxo, revisao],
        )
        .expect_err("ativa com instante de fim tem que ser recusada");
    assert!(
        erro.to_string().contains("CHECK"),
        "erro inesperado: {erro}"
    );
    let erro = cenario
        .conn
        .execute(
            "INSERT INTO fluxo_execucoes
                (id, org_id, ficha_id, fluxo_id, revisao_id, status, no_atual, iniciada_em)
             VALUES (?1, ?2, ?3, ?4, ?5, 'concluida', 'inicio', 4)",
            rusqlite::params![novo_id(), org, outra_ficha, fluxo, revisao],
        )
        .expect_err("encerrada sem instante de fim tem que ser recusada");
    assert!(
        erro.to_string().contains("CHECK"),
        "erro inesperado: {erro}"
    );
}

#[test]
fn revisao_publicada_nao_pode_ser_alterada_nem_apagada() {
    let cenario = Cenario::novo("imutavel");
    let org = cenario.org("a");
    let fluxo = cenario.fluxo(&org, "agendar");
    let revisao = cenario.revisao(&org, &fluxo, 1);

    let erro = cenario
        .conn
        .execute(
            "UPDATE fluxo_revisoes SET fonte = 'outra coisa' WHERE id = ?1",
            [&revisao],
        )
        .expect_err("alterar revisão publicada tem que ser recusado pelo banco");
    assert!(
        erro.to_string().contains("imutável"),
        "erro inesperado: {erro}"
    );
    // Nem mudar o número, nem o texto, nem a IR: o gatilho pega qualquer UPDATE.
    assert!(
        cenario
            .conn
            .execute(
                "UPDATE fluxo_revisoes SET numero = 7 WHERE id = ?1",
                [&revisao]
            )
            .is_err(),
        "trocar o número da revisão tem que ser recusado"
    );

    let erro = cenario
        .conn
        .execute("DELETE FROM fluxo_revisoes WHERE id = ?1", [&revisao])
        .expect_err("apagar revisão publicada tem que ser recusado pelo banco");
    assert!(
        erro.to_string().contains("imutável"),
        "erro inesperado: {erro}"
    );

    // A revisão continua onde estava depois das duas recusas.
    let fonte: String = cenario
        .conn
        .query_row(
            "SELECT fonte FROM fluxo_revisoes WHERE id = ?1",
            [&revisao],
            |linha| linha.get(0),
        )
        .expect("a revisão continua no banco");
    assert_eq!(fonte, "fluxo(id = \"x\")");

    // Apagar o fluxo leva as revisões junto (é o `DELETE /flows/{id}?force=true`): a imutabilidade
    // não impede a exclusão do fluxo, porque aí a cascata roda com o fluxo já apagado.
    cenario
        .conn
        .execute("DELETE FROM fluxos WHERE id = ?1", [&fluxo])
        .expect("apagar o fluxo com force leva as revisões");
    assert_eq!(
        contar(&cenario.conn, "SELECT count(*) FROM fluxo_revisoes"),
        0,
        "as revisões do fluxo apagado ficaram órfãs"
    );
}

#[test]
fn banco_recusa_vinculo_entre_orgs_diferentes() {
    let cenario = Cenario::novo("orgs");
    let org_a = cenario.org("a");
    let org_b = cenario.org("b");
    let agente_a = cenario.agente(&org_a, "a");

    let fluxo_a = cenario.fluxo(&org_a, "da-a");
    let fluxo_b = cenario.fluxo(&org_b, "da-b");

    // Revisão da org A apontando fluxo da org B.
    let erro = cenario
        .conn
        .execute(
            "INSERT INTO fluxo_revisoes
                (id, org_id, fluxo_id, numero, fonte, fonte_sha256, definicao_json,
                 publicada_em, publicada_por)
             VALUES (?1, ?2, ?3, 1, 'x', 'sha', '{}', 1, 'cli')",
            rusqlite::params![novo_id(), org_a, fluxo_b],
        )
        .expect_err("revisão cruzando orgs tem que ser recusada pelo banco");
    assert!(
        erro.to_string().contains("FOREIGN KEY"),
        "erro inesperado: {erro}"
    );

    // Permissão da org A apontando fluxo da org B (e agente da org B).
    assert!(
        cenario
            .conn
            .execute(
                "INSERT INTO agente_fluxos (org_id, agent_id, fluxo_id, liberado_em)
                 VALUES (?1, ?2, ?3, 1)",
                rusqlite::params![org_a, agente_a, fluxo_b],
            )
            .is_err(),
        "permissão para fluxo de outra org tem que ser recusada"
    );

    // Execução da org B apontando Ficha da org A.
    let ficha_a = cenario.ficha(&org_a, &agente_a);
    let revisao_b = cenario.revisao(&org_b, &fluxo_b, 1);
    let erro = cenario
        .conn
        .execute(
            "INSERT INTO fluxo_execucoes
                (id, org_id, ficha_id, fluxo_id, revisao_id, status, no_atual, iniciada_em)
             VALUES (?1, ?2, ?3, ?4, ?5, 'ativa', 'inicio', 1)",
            rusqlite::params![novo_id(), org_b, ficha_a, fluxo_b, revisao_b],
        )
        .expect_err("execução apontando Ficha de outra org tem que ser recusada");
    assert!(
        erro.to_string().contains("FOREIGN KEY"),
        "erro inesperado: {erro}"
    );

    // Efeito da org B apontando execução da org A.
    let revisao_a = cenario.revisao(&org_a, &fluxo_a, 1);
    let execucao_a = cenario.execucao(&org_a, &ficha_a, &fluxo_a, &revisao_a);
    let erro = cenario
        .conn
        .execute(
            "INSERT INTO fluxo_efeitos
                (id, org_id, ficha_id, execucao_id, chave, tipo, payload_json, created_at)
             VALUES (?1, ?2, ?3, ?4, 'k', 'mensagem_ao_cliente', '{}', 1)",
            rusqlite::params![
                novo_id(),
                org_b,
                cenario.ficha(&org_b, &cenario.agente(&org_b, "b")),
                execucao_a
            ],
        )
        .expect_err("efeito cruzando orgs tem que ser recusado");
    assert!(
        erro.to_string().contains("FOREIGN KEY"),
        "erro inesperado: {erro}"
    );

    // O caminho certo, na mesma org, é aceito.
    cenario
        .conn
        .execute(
            "INSERT INTO agente_fluxos (org_id, agent_id, fluxo_id, liberado_em)
             VALUES (?1, ?2, ?3, 1)",
            rusqlite::params![org_a, agente_a, fluxo_a],
        )
        .expect("permissão na mesma org é aceita");
}

#[test]
fn apagar_agente_remove_so_a_permissao() {
    let cenario = Cenario::novo("permissao");
    let org = cenario.org("a");
    let agente = cenario.agente(&org, "a");
    let outro = cenario.agente(&org, "b");
    let fluxo = cenario.fluxo(&org, "agendar");
    let revisao = cenario.revisao(&org, &fluxo, 1);
    cenario
        .conn
        .execute(
            "INSERT INTO agente_fluxos (org_id, agent_id, fluxo_id, liberado_em)
             VALUES (?1, ?2, ?3, 10), (?1, ?4, ?3, 10)",
            rusqlite::params![org, agente, fluxo, outro],
        )
        .expect("duas permissões");
    assert_eq!(
        contar(&cenario.conn, "SELECT count(*) FROM agente_fluxos"),
        2
    );

    cenario
        .conn
        .execute("DELETE FROM agents WHERE id = ?1", [&agente])
        .expect("apaga o agente");

    // Só a permissão dele saiu: o fluxo, a revisão e a permissão do outro agente continuam.
    assert_eq!(
        contar(&cenario.conn, "SELECT count(*) FROM agente_fluxos"),
        1,
        "apagar o agente tem que levar só a permissão dele"
    );
    assert_eq!(contar(&cenario.conn, "SELECT count(*) FROM fluxos"), 1);
    assert_eq!(
        contar(&cenario.conn, "SELECT count(*) FROM fluxo_revisoes"),
        1
    );
    let restante: String = cenario
        .conn
        .query_row("SELECT agent_id FROM agente_fluxos", [], |linha| {
            linha.get(0)
        })
        .expect("permissão do outro agente");
    assert_eq!(restante, outro);

    // Apagar o fluxo leva a permissão junto (ela não faz sentido sem o fluxo).
    cenario
        .conn
        .execute("DELETE FROM fluxos WHERE id = ?1", [&fluxo])
        .expect("apaga o fluxo");
    assert_eq!(
        contar(&cenario.conn, "SELECT count(*) FROM agente_fluxos"),
        0,
        "a permissão de um fluxo apagado ficou no banco"
    );
    let _ = revisao;
}

#[test]
fn slug_e_numero_respeitam_a_unicidade_por_org_e_por_fluxo() {
    let cenario = Cenario::novo("unicidade");
    let org_a = cenario.org("a");
    let org_b = cenario.org("b");

    cenario.fluxo(&org_a, "agendar");
    let erro = cenario
        .conn
        .execute(
            "INSERT INTO fluxos (id, org_id, slug, nome) VALUES (?1, ?2, 'agendar', 'Outro')",
            rusqlite::params![novo_id(), org_a],
        )
        .expect_err("slug repetido na mesma org tem que ser recusado");
    assert!(
        erro.to_string().contains("UNIQUE"),
        "erro inesperado: {erro}"
    );
    // O mesmo slug em outra org é aceito: o fluxo é da organização.
    cenario.fluxo(&org_b, "agendar");

    let fluxo = cenario.fluxo(&org_a, "triagem");
    let outro_fluxo = cenario.fluxo(&org_a, "confirmar");
    cenario.revisao(&org_a, &fluxo, 1);
    cenario.revisao(&org_a, &fluxo, 2);
    let erro = cenario
        .conn
        .execute(
            "INSERT INTO fluxo_revisoes
                (id, org_id, fluxo_id, numero, fonte, fonte_sha256, definicao_json,
                 publicada_em, publicada_por)
             VALUES (?1, ?2, ?3, 1, 'x', 'sha', '{}', 1, 'cli')",
            rusqlite::params![novo_id(), org_a, fluxo],
        )
        .expect_err("número repetido no mesmo fluxo tem que ser recusado");
    assert!(
        erro.to_string().contains("UNIQUE"),
        "erro inesperado: {erro}"
    );
    // O número 1 de outro fluxo da mesma org convive.
    cenario.revisao(&org_a, &outro_fluxo, 1);
}

#[test]
fn rodar_a_migracao_de_novo_nao_faz_nada() {
    let mut cenario = Cenario::novo("idempotente");
    let org = cenario.org("a");
    cenario.fluxo(&org, "agendar");
    let antes = contar(&cenario.conn, "SELECT count(*) FROM fluxos");

    let relatorios = cenario._store.migrar().expect("migra de novo");
    assert!(relatorios.ja_estava_em_dia(), "{relatorios:?}");

    cenario.conn = conexao(&cenario.pasta.join("harness.db"));
    assert_eq!(contar(&cenario.conn, "SELECT count(*) FROM fluxos"), antes);
    assert_eq!(
        versao_do_arquivo(&cenario.pasta.join("harness.db")).expect("versão"),
        Some(versao_alvo_harness())
    );
}

#[test]
fn vocabulario_aceita_exatamente_os_textos_do_poph_types() {
    let cenario = Cenario::novo("vocabulario");
    let org = cenario.org("a");
    let agente = cenario.agente(&org, "a");
    let fluxo = cenario.fluxo(&org, "agendar");
    let revisao = cenario.revisao(&org, &fluxo, 1);

    // Todo texto de `StatusExecucao` passa pelo CHECK. Cada execução numa Ficha própria, porque só
    // uma ativa por Ficha.
    for status in ExecutionStatus::ALL {
        let ficha = cenario.ficha(&org, &agente);
        let finalizada_em = if *status == ExecutionStatus::Active {
            None
        } else {
            Some(9_i64)
        };
        cenario
            .conn
            .execute(
                "INSERT INTO fluxo_execucoes
                    (id, org_id, ficha_id, fluxo_id, revisao_id, status, no_atual, iniciada_em,
                     finalizada_em)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'inicio', 1, ?7)",
                rusqlite::params![
                    novo_id(),
                    org,
                    ficha,
                    fluxo,
                    revisao,
                    status.as_text(),
                    finalizada_em
                ],
            )
            .unwrap_or_else(|erro| panic!("status {status} recusado pelo banco: {erro}"));
    }

    let ficha = cenario.ficha(&org, &agente);
    let execucao = cenario.execucao(&org, &ficha, &fluxo, &revisao);

    // `StatusEtapa` na tabela de etapas (a primeira linha de cada execução tem que ter ordem única).
    for (indice, status) in StepStatus::ALL.iter().enumerate() {
        let concluiu_em = if *status == StepStatus::Completed {
            Some(7_i64)
        } else {
            None
        };
        cenario
            .conn
            .execute(
                "INSERT INTO fluxo_execucao_etapas
                    (id, org_id, execucao_id, no_id, ordem, status, entrou_em, concluiu_em)
                 VALUES (?1, ?2, ?3, 'coleta', ?4, ?5, 1, ?6)",
                rusqlite::params![
                    novo_id(),
                    org,
                    execucao,
                    indice as i64 + 1,
                    status.as_text(),
                    concluiu_em
                ],
            )
            .unwrap_or_else(|erro| panic!("status de etapa {status} recusado: {erro}"));
    }

    // `EstadoEfeito` e os três tipos de efeito da linguagem.
    for (indice, estado) in EffectState::ALL.iter().enumerate() {
        for (posicao, tipo) in ["mensagem_ao_cliente", "notificar_whatsapp", "encaminhar"]
            .iter()
            .enumerate()
        {
            let chave = indice * 3 + posicao + 1;
            let enviado_em = if *estado == EffectState::Sent {
                Some(3_i64)
            } else {
                None
            };
            cenario
                .conn
                .execute(
                    "INSERT INTO fluxo_efeitos
                        (id, org_id, ficha_id, execucao_id, chave, tipo, estado, enviado_em,
                         created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 1)",
                    rusqlite::params![
                        novo_id(),
                        org,
                        ficha,
                        execucao,
                        format!("efeito-{chave}"),
                        tipo,
                        estado.as_text(),
                        enviado_em
                    ],
                )
                .unwrap_or_else(|erro| panic!("efeito {tipo}/{estado} recusado: {erro}"));
        }
    }

    // Valor inventado em cada coluna de situação é recusado.
    for (sql, rotulo) in [
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
            cenario.conn.execute(sql, []).is_err(),
            "{rotulo} tem que ser recusado"
        );
    }
    // E o valor inventado de situação de efeito, com o resto válido.
    assert!(
        cenario
            .conn
            .execute(
                "INSERT INTO fluxo_efeitos
                    (id, org_id, ficha_id, execucao_id, chave, tipo, estado, created_at)
                 VALUES (?1, ?2, ?3, ?4, 'inventado', 'encaminhar', 'entregue', 1)",
                rusqlite::params![novo_id(), org, ficha, execucao],
            )
            .is_err(),
        "estado de efeito fora da lista tem que ser recusado"
    );
    // Efeito entregue sem instante de entrega é incoerente.
    assert!(
        cenario
            .conn
            .execute(
                "INSERT INTO fluxo_efeitos
                    (id, org_id, ficha_id, execucao_id, chave, tipo, estado, created_at)
                 VALUES (?1, ?2, ?3, ?4, 'sem-instante', 'encaminhar', 'enviado', 1)",
                rusqlite::params![novo_id(), org, ficha, execucao],
            )
            .is_err(),
        "efeito enviado sem `enviado_em` tem que ser recusado"
    );
}

#[test]
fn apagar_a_org_leva_tudo_sem_esbarrar_no_gatilho() {
    // Caminho de remoção de organização: o gatilho de imutabilidade não pode barrar a cascata.
    let cenario = Cenario::novo("org-apagada");
    let org = cenario.org("a");
    let agente = cenario.agente(&org, "a");
    let fluxo = cenario.fluxo(&org, "agendar");
    let revisao = cenario.revisao(&org, &fluxo, 1);
    let ficha = cenario.ficha(&org, &agente);
    let execucao = cenario.execucao(&org, &ficha, &fluxo, &revisao);
    cenario
        .conn
        .execute(
            "INSERT INTO agente_fluxos (org_id, agent_id, fluxo_id, liberado_em)
             VALUES (?1, ?2, ?3, 1)",
            rusqlite::params![org, agente, fluxo],
        )
        .expect("permissão");
    cenario
        .conn
        .execute(
            "INSERT INTO fluxo_execucao_etapas
                (id, org_id, execucao_id, no_id, ordem, status, entrou_em)
             VALUES (?1, ?2, ?3, 'coleta', 1, 'pendente', 1)",
            rusqlite::params![novo_id(), org, execucao],
        )
        .expect("etapa");
    cenario
        .conn
        .execute(
            "INSERT INTO fluxo_efeitos
                (id, org_id, ficha_id, execucao_id, chave, tipo, payload_json, created_at)
             VALUES (?1, ?2, ?3, ?4, 'k', 'encaminhar', '{}', 1)",
            rusqlite::params![novo_id(), org, ficha, execucao],
        )
        .expect("efeito");

    cenario
        .conn
        .execute("DELETE FROM orgs WHERE id = ?1", [&org])
        .expect("apagar a org não pode esbarrar no gatilho de imutabilidade");

    for tabela in [
        "fluxos",
        "fluxo_revisoes",
        "agente_fluxos",
        "fluxo_execucoes",
        "fluxo_execucao_etapas",
        "fluxo_efeitos",
    ] {
        assert_eq!(
            contar(&cenario.conn, &format!("SELECT count(*) FROM {tabela}")),
            0,
            "{tabela} ficou com linha órfã depois de apagar a org"
        );
    }
}

#[test]
fn etapas_e_efeitos_somem_com_a_execucao_e_execucoes_com_a_ficha() {
    let cenario = Cenario::novo("cascata");
    let org = cenario.org("a");
    let agente = cenario.agente(&org, "a");
    let fluxo = cenario.fluxo(&org, "agendar");
    let revisao = cenario.revisao(&org, &fluxo, 1);
    let ficha = cenario.ficha(&org, &agente);
    let execucao = cenario.execucao(&org, &ficha, &fluxo, &revisao);

    cenario
        .conn
        .execute(
            "INSERT INTO fluxo_execucao_etapas
                (id, org_id, execucao_id, no_id, ordem, status, entrou_em)
             VALUES (?1, ?2, ?3, 'coleta', 1, 'pendente', 1)",
            rusqlite::params![novo_id(), org, execucao],
        )
        .expect("etapa");
    cenario
        .conn
        .execute(
            "INSERT INTO fluxo_efeitos
                (id, org_id, ficha_id, execucao_id, chave, tipo, payload_json, created_at)
             VALUES (?1, ?2, ?3, ?4, 'k', 'notificar_whatsapp', '{}', 1)",
            rusqlite::params![novo_id(), org, ficha, execucao],
        )
        .expect("efeito");
    // A mesma chave na mesma execução é recusada (idempotência do efeito).
    assert!(
        cenario
            .conn
            .execute(
                "INSERT INTO fluxo_efeitos
                    (id, org_id, ficha_id, execucao_id, chave, tipo, payload_json, created_at)
                 VALUES (?1, ?2, ?3, ?4, 'k', 'encaminhar', '{}', 2)",
                rusqlite::params![novo_id(), org, ficha, execucao],
            )
            .is_err(),
        "chave de efeito repetida na mesma execução tem que ser recusada"
    );

    // `ordem` é única por execução (cursor da linha do tempo das etapas).
    assert!(
        cenario
            .conn
            .execute(
                "INSERT INTO fluxo_execucao_etapas
                    (id, org_id, execucao_id, no_id, ordem, status, entrou_em)
                 VALUES (?1, ?2, ?3, 'pagamento', 1, 'pendente', 2)",
                rusqlite::params![novo_id(), org, execucao],
            )
            .is_err(),
        "ordem repetida na mesma execução tem que ser recusada"
    );

    // Apagar a Ficha leva execução, etapa e efeito (são sub-itens do atendimento).
    cenario
        .conn
        .execute("DELETE FROM fichas WHERE id = ?1", [&ficha])
        .expect("apaga a Ficha");
    assert_eq!(
        contar(&cenario.conn, "SELECT count(*) FROM fluxo_execucoes"),
        0
    );
    assert_eq!(
        contar(&cenario.conn, "SELECT count(*) FROM fluxo_execucao_etapas"),
        0
    );
    assert_eq!(
        contar(&cenario.conn, "SELECT count(*) FROM fluxo_efeitos"),
        0
    );

    // O fluxo e as revisões continuam: o histórico do fluxo é da organização.
    assert_eq!(contar(&cenario.conn, "SELECT count(*) FROM fluxos"), 1);
    assert_eq!(
        contar(&cenario.conn, "SELECT count(*) FROM fluxo_revisoes"),
        1
    );
}
