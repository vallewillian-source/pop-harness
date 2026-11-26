//! Testes da migração das Fichas e dos eventos (FX-05).
//!
//! Cobrem o que a tarefa exige: banco novo ganha as duas tabelas com os índices combinados; banco
//! que já tinha conversas (`open`, `transferred` e `closed`, com e sem mensagens) fica com
//! **exatamente uma** Ficha por conversa, com o status, o autor e os retratos certos; rodar a
//! migração de novo não faz nada; e as regras que o banco tem que garantir sozinho valem (uma Ficha
//! por conversa, `seq` única por Ficha, idempotência opcional, `org_id` na referência composta e os
//! textos aceitos sendo exatamente os do `poph-types`).
//!
//! As asserções usam SQL direto de propósito: o repositório de Fichas é a FX-07, e aqui o que está
//! sendo provado é o esquema e o backfill, não a API do armazenamento.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use poph_store::migracoes::migrar;
use poph_store::{MIGRACOES_HARNESS, Store, versao_alvo_harness, versao_do_arquivo};
use poph_types::{ClosedBy, EventActor, RecordEventKind, RecordId, RecordStatus};
use rusqlite::Connection;

/// Nome da migração desta tarefa em `MIGRACOES_HARNESS`.
const MIGRACAO: &str = "0012_fichas_e_eventos";

fn pasta_nova(rotulo: &str) -> PathBuf {
    static CONTADOR: AtomicUsize = AtomicUsize::new(0);
    let n = CONTADOR.fetch_add(1, Ordering::Relaxed);
    let pasta =
        std::env::temp_dir().join(format!("poph-fichas-{rotulo}-{}-{n}", std::process::id()));
    let _ = fs::remove_dir_all(&pasta);
    fs::create_dir_all(&pasta).expect("cria a pasta do teste");
    pasta
}

/// Posição da migração das Fichas na lista (o que já existia antes dela é o banco "antigo").
fn indice_da_migracao() -> usize {
    MIGRACOES_HARNESS
        .iter()
        .position(|migracao| migracao.nome == MIGRACAO)
        .expect("a migração das Fichas está registrada em MIGRACOES_HARNESS")
}

/// Conexão própria para conferir o banco por SQL, com as chaves estrangeiras ligadas como no
/// `Store` (sem isso o SQLite ignora a FK silenciosamente e o teste passaria à toa).
fn conexao(caminho: &std::path::Path) -> Connection {
    let conn = Connection::open(caminho).expect("abre o banco para conferir");
    conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA busy_timeout = 2000;")
        .expect("pragmas");
    conn
}

/// Uma org e um agente mínimos, gravados direto (o cenário é anterior à migração).
fn semear_org_e_agente(conn: &Connection, rotulo: &str) -> (String, String) {
    let org = uuid::Uuid::now_v7().to_string();
    let agente = uuid::Uuid::now_v7().to_string();
    conn.execute(
        "INSERT INTO orgs (id, name, slug, created_at, updated_at) VALUES (?1, ?2, ?3, 1, 1)",
        rusqlite::params![org, format!("Org {rotulo}"), format!("org-{rotulo}")],
    )
    .expect("org");
    conn.execute(
        "INSERT INTO agents (id, org_id, name, slug, provider, model, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, 'antigravity2api', 'gemini', 1, 1)",
        rusqlite::params![
            agente,
            org,
            format!("Agente {rotulo}"),
            format!("agente-{rotulo}")
        ],
    )
    .expect("agente");
    (org, agente)
}

/// Uma conversa no estado pedido, gravada direto.
fn semear_conversa(
    conn: &Connection,
    org: &str,
    agente: &str,
    status: &str,
    peer: Option<&str>,
    criada_em: i64,
    atualizada_em: i64,
) -> String {
    let conversa = uuid::Uuid::now_v7().to_string();
    conn.execute(
        "INSERT INTO conversations
            (id, org_id, agent_id, channel, external_peer, status, created_at, updated_at)
         VALUES (?1, ?2, ?3, 'whatsapp', ?4, ?5, ?6, ?7)",
        rusqlite::params![
            conversa,
            org,
            agente,
            peer,
            status,
            criada_em,
            atualizada_em
        ],
    )
    .expect("conversa");
    conversa
}

fn semear_mensagem(conn: &Connection, org: &str, conversa: &str, criada_em: i64) {
    conn.execute(
        "INSERT INTO messages (id, org_id, conversation_id, direction, role, text, created_at)
         VALUES (?1, ?2, ?3, 'in', 'user', 'oi', ?4)",
        rusqlite::params![uuid::Uuid::now_v7().to_string(), org, conversa, criada_em],
    )
    .expect("mensagem");
}

/// Banco na versão **anterior** à das Fichas, com o cenário que a migração precisa resolver.
///
/// Devolve o caminho do `harness.db` e as conversas semeadas por nome do cenário.
struct BancoAntigo {
    pasta: PathBuf,
    conversas: Vec<(String, String, String)>, // (cenário, id da conversa, org)
}

impl BancoAntigo {
    fn montar(rotulo: &str) -> Self {
        let pasta = pasta_nova(rotulo);
        let harness = pasta.join("harness.db");
        let antes = indice_da_migracao();
        let mut conversas = Vec::new();
        {
            let conn = Connection::open(&harness).expect("abre o banco antigo");
            migrar(
                &conn,
                "harness.db",
                &harness,
                &MIGRACOES_HARNESS[..antes],
                1,
            )
            .expect("migra até a versão anterior");
            conn.execute_batch("PRAGMA foreign_keys = ON;")
                .expect("pragma");

            let (org_a, agente_a) = semear_org_e_agente(&conn, "a");
            // Segunda org: prova que o backfill não cruza org nem mistura ids.
            let (org_b, agente_b) = semear_org_e_agente(&conn, "b");

            let aberta = semear_conversa(
                &conn,
                &org_a,
                &agente_a,
                "open",
                Some("5521900000001"),
                1_000,
                1_500,
            );
            semear_mensagem(&conn, &org_a, &aberta, 1_100);
            semear_mensagem(&conn, &org_a, &aberta, 1_500);

            let transferida = semear_conversa(
                &conn,
                &org_a,
                &agente_a,
                "transferred",
                Some("5521900000002"),
                2_000,
                2_200,
            );

            let fechada = semear_conversa(
                &conn,
                &org_a,
                &agente_a,
                "closed",
                Some("5521900000003"),
                3_000,
                3_900,
            );
            semear_mensagem(&conn, &org_a, &fechada, 3_100);

            // Fechada e sem mensagem nenhuma (caminho do chat de teste que abre e fecha).
            let fechada_vazia =
                semear_conversa(&conn, &org_a, &agente_a, "closed", None, 4_000, 4_050);

            let da_outra_org = semear_conversa(
                &conn,
                &org_b,
                &agente_b,
                "open",
                Some("5521900000004"),
                5_000,
                5_000,
            );

            for (cenario, id, org) in [
                ("aberta", &aberta, &org_a),
                ("transferida", &transferida, &org_a),
                ("fechada", &fechada, &org_a),
                ("fechada-sem-mensagem", &fechada_vazia, &org_a),
                ("outra-org", &da_outra_org, &org_b),
            ] {
                conversas.push((cenario.to_owned(), id.clone(), org.clone()));
            }
        }
        Self { pasta, conversas }
    }

    fn harness(&self) -> PathBuf {
        self.pasta.join("harness.db")
    }

    fn id(&self, cenario: &str) -> &str {
        &self
            .conversas
            .iter()
            .find(|(nome, _, _)| nome == cenario)
            .expect("cenário semeado")
            .1
    }
}

impl Drop for BancoAntigo {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.pasta);
    }
}

/// O que o backfill gravou para uma conversa.
struct FichaGravada {
    id: String,
    status: String,
    aberta_em: i64,
    fechada_em: Option<i64>,
    fechada_por: Option<String>,
    fechada_motivo: Option<String>,
    agent_id: String,
    canal: String,
    peer_externo: Option<String>,
    version: i64,
}

fn ficha_da_conversa(conn: &Connection, conversa: &str) -> FichaGravada {
    conn.query_row(
        "SELECT id, status, aberta_em, fechada_em, fechada_por, fechada_motivo, agent_id, canal,
                peer_externo, version
           FROM fichas WHERE conversation_id = ?1",
        [conversa],
        |linha| {
            Ok(FichaGravada {
                id: linha.get(0)?,
                status: linha.get(1)?,
                aberta_em: linha.get(2)?,
                fechada_em: linha.get(3)?,
                fechada_por: linha.get(4)?,
                fechada_motivo: linha.get(5)?,
                agent_id: linha.get(6)?,
                canal: linha.get(7)?,
                peer_externo: linha.get(8)?,
                version: linha.get(9)?,
            })
        },
    )
    .expect("a conversa tem Ficha")
}

fn contar(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |linha| linha.get(0))
        .expect("conta")
}

#[test]
fn migracao_cria_as_duas_tabelas_com_os_indices_combinados() {
    let pasta = pasta_nova("esquema");
    let harness = pasta.join("harness.db");
    let store = Store::abrir_em(&harness, &pasta.join("rag.db")).expect("abre e migra");

    for tabela in ["fichas", "ficha_eventos"] {
        assert!(
            store
                .tabelas()
                .expect("tabelas")
                .iter()
                .any(|t| t == tabela),
            "tabela {tabela} não foi criada"
        );
    }
    assert_eq!(
        versao_do_arquivo(&harness).expect("versão"),
        Some(versao_alvo_harness())
    );

    // Colunas do PRD §7.1, sem faltar nem sobrar troca de nome.
    let colunas = store.colunas("fichas").expect("colunas de fichas");
    for esperada in [
        "id",
        "org_id",
        "conversation_id",
        "agent_id",
        "canal",
        "peer_externo",
        "status",
        "aberta_em",
        "fechada_em",
        "fechada_por",
        "fechada_motivo",
        "version",
    ] {
        assert!(
            colunas.iter().any(|coluna| coluna == esperada),
            "fichas sem a coluna {esperada}; tem {colunas:?}"
        );
    }
    let colunas = store.colunas("ficha_eventos").expect("colunas de eventos");
    for esperada in [
        "id",
        "org_id",
        "ficha_id",
        "execucao_id",
        "seq",
        "tipo",
        "ator_tipo",
        "ator_ref",
        "payload_json",
        "chave_idempotencia",
        "created_at",
    ] {
        assert!(
            colunas.iter().any(|coluna| coluna == esperada),
            "ficha_eventos sem a coluna {esperada}; tem {colunas:?}"
        );
    }

    // Índices que carregam regra (não só desempenho).
    let conn = conexao(&harness);
    for indice in [
        "ux_fichas_conversa",
        "ix_fichas_org_status",
        "ux_fichas_org_id",
        "ux_ficha_eventos_seq",
        "ux_ficha_eventos_idem",
    ] {
        let existe = contar(
            &conn,
            &format!(
                "SELECT count(*) FROM sqlite_master WHERE type = 'index' AND name = '{indice}'"
            ),
        );
        assert_eq!(existe, 1, "índice {indice} não foi criado");
    }
}

#[test]
fn backfill_cria_exatamente_uma_ficha_por_conversa() {
    let banco = BancoAntigo::montar("backfill");
    let harness = banco.harness();
    let store = Store::abrir_em(&harness, &banco.pasta.join("rag.db")).expect("abre e migra");
    assert_eq!(
        versao_do_arquivo(&harness).expect("versão"),
        Some(versao_alvo_harness())
    );
    let conn = conexao(&harness);

    // Uma Ficha por conversa, nenhuma sobrando e nenhuma conversa sem Ficha.
    assert_eq!(
        contar(&conn, "SELECT count(*) FROM fichas"),
        contar(&conn, "SELECT count(*) FROM conversations"),
        "o backfill tem que criar uma Ficha para cada conversa"
    );
    assert_eq!(
        contar(
            &conn,
            "SELECT count(*) FROM conversations c
              WHERE NOT EXISTS (SELECT 1 FROM fichas f WHERE f.conversation_id = c.id)"
        ),
        0,
        "sobrou conversa sem Ficha"
    );
    assert_eq!(
        contar(
            &conn,
            "SELECT count(*) FROM (SELECT conversation_id FROM fichas
                WHERE conversation_id IS NOT NULL
                GROUP BY conversation_id HAVING count(*) > 1)"
        ),
        0,
        "alguma conversa ficou com duas Fichas"
    );

    // Conversa aberta e transferida viram Ficha aberta, com o retrato do atendimento.
    for (cenario, criada_em) in [
        ("aberta", 1_000),
        ("transferida", 2_000),
        ("outra-org", 5_000),
    ] {
        let ficha = ficha_da_conversa(&conn, banco.id(cenario));
        assert_eq!(
            ficha.status.parse::<RecordStatus>().expect("status válido"),
            RecordStatus::Open,
            "{cenario} deveria ficar aberta"
        );
        assert_eq!(
            ficha.aberta_em, criada_em,
            "{cenario}: aberta_em é a criação da conversa"
        );
        assert_eq!(ficha.fechada_em, None, "{cenario} não pode ter fechamento");
        assert_eq!(ficha.fechada_por, None);
        assert_eq!(ficha.canal, "whatsapp");
        assert_eq!(ficha.version, 1);
        assert!(
            !ficha.agent_id.is_empty(),
            "{cenario} sem o agente do atendimento"
        );
    }

    // Conversa fechada vira Ficha fechada com `fechada_por = 'migracao'`, no último instante dela.
    for cenario in ["fechada", "fechada-sem-mensagem"] {
        let ficha = ficha_da_conversa(&conn, banco.id(cenario));
        assert_eq!(
            ficha.status.parse::<RecordStatus>().expect("status válido"),
            RecordStatus::Closed,
            "{cenario} deveria ficar fechada"
        );
        assert_eq!(
            ficha.fechada_por.as_deref(),
            Some(ClosedBy::Migration.as_text())
        );
        assert_eq!(
            ficha.fechada_em,
            Some(if cenario == "fechada" { 3_900 } else { 4_050 }),
            "fechada_em é o último instante conhecido da conversa"
        );
        assert!(
            ficha
                .fechada_motivo
                .as_deref()
                .is_some_and(|motivo| !motivo.is_empty()),
            "{cenario} fechada sem motivo"
        );
    }

    // O cliente da conversa é copiado; a conversa sem telefone conhecido fica sem.
    assert_eq!(
        ficha_da_conversa(&conn, banco.id("aberta"))
            .peer_externo
            .as_deref(),
        Some("5521900000001")
    );
    assert_eq!(
        ficha_da_conversa(&conn, banco.id("fechada-sem-mensagem")).peer_externo,
        None
    );

    // O id da Ficha é um UUID v7 de verdade (recusa de versão errada é a mesma dos ids do código).
    for (_, conversa, _) in &banco.conversas {
        let ficha = ficha_da_conversa(&conn, conversa);
        ficha
            .id
            .parse::<RecordId>()
            .unwrap_or_else(|erro| panic!("id de Ficha inválido no backfill: {erro}"));
    }

    // As mensagens antigas continuam onde estavam: o backfill não mexe em `messages`.
    assert_eq!(
        contar(&conn, "SELECT count(*) FROM messages"),
        3,
        "o backfill não pode criar nem apagar mensagem"
    );

    drop(store);
}

#[test]
fn rodar_a_migracao_de_novo_nao_faz_nada() {
    let banco = BancoAntigo::montar("idempotente");
    let store = Store::abrir_em(&banco.harness(), &banco.pasta.join("rag.db")).expect("abre");
    let antes = {
        let conn = conexao(&banco.harness());
        contar(&conn, "SELECT count(*) FROM fichas")
    };

    let relatorios = store.migrar().expect("migra de novo");
    assert!(relatorios.ja_estava_em_dia(), "{relatorios:?}");

    let conn = conexao(&banco.harness());
    assert_eq!(contar(&conn, "SELECT count(*) FROM fichas"), antes);

    // E o próprio comando do backfill é idempotente (o `NOT EXISTS`), caso alguém o repita.
    let sql = MIGRACOES_HARNESS[indice_da_migracao()].sql;
    let inicio = sql
        .find("INSERT INTO fichas")
        .expect("o backfill está na migração");
    conn.execute_batch(&sql[inicio..])
        .expect("repetir o backfill não pode falhar");
    assert_eq!(contar(&conn, "SELECT count(*) FROM fichas"), antes);
}

#[test]
fn banco_recusa_duas_fichas_para_a_mesma_conversa() {
    let banco = BancoAntigo::montar("unica");
    let _store = Store::abrir_em(&banco.harness(), &banco.pasta.join("rag.db")).expect("abre");
    let conn = conexao(&banco.harness());
    let conversa = banco.id("aberta");
    let org: String = conn
        .query_row(
            "SELECT org_id FROM conversations WHERE id = ?1",
            [conversa],
            |linha| linha.get(0),
        )
        .expect("org da conversa");

    let erro = conn
        .execute(
            "INSERT INTO fichas (id, org_id, conversation_id, agent_id, canal, status, aberta_em)
             VALUES (?1, ?2, ?3, 'ag', 'whatsapp', 'open', 1)",
            rusqlite::params![uuid::Uuid::now_v7().to_string(), org, conversa],
        )
        .expect_err("a segunda Ficha da mesma conversa tem que ser recusada");
    assert!(
        erro.to_string().contains("UNIQUE"),
        "erro inesperado: {erro}"
    );

    // Ficha que sobreviveu à conversa apagada não colide com outra na mesma situação.
    conn.execute("DELETE FROM conversations WHERE id = ?1", [conversa])
        .expect("apaga a conversa");
    let sem_dono: i64 = conn
        .query_row(
            "SELECT count(*) FROM fichas WHERE conversation_id IS NULL",
            [],
            |linha| linha.get(0),
        )
        .expect("conta");
    assert_eq!(sem_dono, 1, "a Ficha sobrevive à conversa apagada");
    conn.execute(
        "INSERT INTO fichas (id, org_id, conversation_id, agent_id, canal, status, aberta_em)
         VALUES (?1, ?2, NULL, 'ag', 'whatsapp', 'open', 2)",
        rusqlite::params![uuid::Uuid::now_v7().to_string(), org],
    )
    .expect("duas Fichas sem conversa não colidem");
}

#[test]
fn banco_recusa_ficha_com_status_e_fechamento_incoerentes() {
    let banco = BancoAntigo::montar("coerencia");
    let _store = Store::abrir_em(&banco.harness(), &banco.pasta.join("rag.db")).expect("abre");
    let conn = conexao(&banco.harness());
    let org: String = conn
        .query_row("SELECT org_id FROM conversations LIMIT 1", [], |linha| {
            linha.get(0)
        })
        .expect("org da conversa semeada");

    let inserir = |status: &str, fechada_em: Option<i64>, fechada_por: Option<&str>| {
        conn.execute(
            "INSERT INTO fichas (id, org_id, conversation_id, agent_id, canal, status, aberta_em,
                                 fechada_em, fechada_por)
             VALUES (?1, ?2, NULL, 'ag', 'whatsapp', ?3, 1, ?4, ?5)",
            rusqlite::params![
                uuid::Uuid::now_v7().to_string(),
                org,
                status,
                fechada_em,
                fechada_por
            ],
        )
    };

    assert!(
        inserir("closed", None, Some("agente")).is_err(),
        "fechada sem instante de fechamento"
    );
    assert!(
        inserir("closed", Some(10), None).is_err(),
        "fechada sem autor do fechamento"
    );
    assert!(
        inserir("open", Some(10), Some("agente")).is_err(),
        "aberta com fechamento"
    );
    assert!(
        inserir("aberto", None, None).is_err(),
        "status fora da lista"
    );
    assert!(
        inserir("closed", Some(10), Some("cliente_qualquer")).is_err(),
        "autor fora da lista"
    );
    assert!(
        inserir("closed", Some(10), Some("migracao")).is_ok(),
        "o par coerente é aceito"
    );
}

#[test]
fn banco_recusa_evento_com_texto_fora_da_lista_e_cruzando_orgs() {
    let banco = BancoAntigo::montar("contexto");
    let _store = Store::abrir_em(&banco.harness(), &banco.pasta.join("rag.db")).expect("abre");
    let conn = conexao(&banco.harness());
    let org: String = conn
        .query_row("SELECT org_id FROM conversations LIMIT 1", [], |linha| {
            linha.get(0)
        })
        .expect("org da conversa semeada");

    let evento = |ficha: &str, seq: i64, tipo: &str, ator: &str, chave: Option<&str>| {
        conn.execute(
            "INSERT INTO ficha_eventos
                (id, org_id, ficha_id, seq, tipo, ator_tipo, chave_idempotencia, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 1)",
            rusqlite::params![
                uuid::Uuid::now_v7().to_string(),
                org,
                ficha,
                seq,
                tipo,
                ator,
                chave
            ],
        )
    };

    // A primeira Ficha de evento serve de pai.
    let ficha = banco.id("aberta").to_owned();
    let ficha = ficha_da_conversa(&conn, &ficha).id;

    // Todos os tipos e atores do vocabulário passam pelo CHECK do banco.
    let mut seq = 1;
    for tipo in RecordEventKind::ALL {
        for ator in EventActor::ALL {
            evento(&ficha, seq, tipo.as_text(), ator.as_text(), None)
                .unwrap_or_else(|erro| panic!("{tipo}/{ator} recusado pelo banco: {erro}"));
            seq += 1;
        }
    }
    assert!(
        evento(&ficha, seq, "tipo_inventado", "sistema", None).is_err(),
        "tipo fora da lista tem que ser recusado"
    );
    assert!(
        evento(&ficha, seq + 1, "ficha_aberta", "operador", None).is_err(),
        "ator fora da lista tem que ser recusado"
    );
    // O cliente não é ator de evento (decisão D5).
    assert!(
        evento(&ficha, seq + 2, "ficha_aberta", "cliente", None).is_err(),
        "o cliente não assina evento da Ficha"
    );

    // `seq` é única por Ficha: a linha do tempo não pode empatar.
    let repetido = seq - 1;
    assert!(
        evento(&ficha, repetido, "ficha_aberta", "sistema", None).is_err(),
        "seq repetida na mesma Ficha tem que ser recusada"
    );

    // Idempotência: a mesma chave na mesma Ficha é recusada; chave nula não colide.
    assert!(evento(&ficha, seq + 10, "ficha_aberta", "sistema", Some("k1")).is_ok());
    assert!(
        evento(&ficha, seq + 11, "ficha_aberta", "sistema", Some("k1")).is_err(),
        "chave de idempotência repetida tem que ser recusada"
    );
    assert!(evento(&ficha, seq + 12, "ficha_aberta", "sistema", None).is_ok());
    assert!(evento(&ficha, seq + 13, "ficha_aberta", "sistema", None).is_ok());

    // Evento de Ficha de outra org é recusado pela referência composta.
    let outra_org: String = conn
        .query_row(
            "SELECT id FROM orgs WHERE id <> ?1 LIMIT 1",
            [&org],
            |linha| linha.get(0),
        )
        .expect("segunda org");
    let erro = conn
        .execute(
            "INSERT INTO ficha_eventos (id, org_id, ficha_id, seq, tipo, ator_tipo, created_at)
             VALUES (?1, ?2, ?3, 9000, 'ficha_aberta', 'sistema', 1)",
            rusqlite::params![uuid::Uuid::now_v7().to_string(), outra_org, ficha],
        )
        .expect_err("evento cruzando orgs tem que ser recusado pelo banco");
    assert!(
        erro.to_string().contains("FOREIGN KEY"),
        "erro inesperado: {erro}"
    );
}

#[test]
fn eventos_sobrevivem_a_ficha_e_somem_com_ela() {
    // `ficha_eventos` é filha da Ficha: apagar a Ficha leva a linha do tempo junto.
    let banco = BancoAntigo::montar("cascata");
    let _store = Store::abrir_em(&banco.harness(), &banco.pasta.join("rag.db")).expect("abre");
    let conn = conexao(&banco.harness());
    let ficha = ficha_da_conversa(&conn, banco.id("aberta"));
    conn.execute(
        "INSERT INTO ficha_eventos (id, org_id, ficha_id, seq, tipo, ator_tipo, created_at)
         SELECT ?1, org_id, id, 1, 'ficha_aberta', 'sistema', 1 FROM fichas WHERE id = ?2",
        rusqlite::params![uuid::Uuid::now_v7().to_string(), ficha.id],
    )
    .expect("evento");
    conn.execute("DELETE FROM fichas WHERE id = ?1", [&ficha.id])
        .expect("apaga a Ficha");
    assert_eq!(
        contar(&conn, "SELECT count(*) FROM ficha_eventos"),
        0,
        "evento órfão ficou no banco"
    );
}
