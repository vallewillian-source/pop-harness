//! Medição da MX-05: planos de consulta, latência de uma página e custo do índice de
//! `conversations` na escrita do turno. Não roda no `cargo test` normal (é lenta).
//!
//! ```text
//! cargo test --release -p poph-store --test leitura_incremental_medicao -- --ignored --nocapture
//! ```
//!
//! Variáveis opcionais: `POPH_MX05_MENSAGENS` (padrão 1.000.000) e `POPH_MX05_CONVERSAS`
//! (padrão 5.000). A carga é sintética: as mensagens entram por SQL direto numa transação, para
//! que a preparação leve segundos, e as medições passam pela API real (`desde`, `alteradas_desde`,
//! `gravar`, `tocar`).

use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use poph_store::{Direcao, NovaMensagem, NovaOrg, NovoAgente, Origem, Papel, Store};
use poph_types::{ConversationId, MessageId, OrgId};
use rusqlite::{Connection, params};

const CONSULTA_MENSAGENS: &str = "SELECT rowid, id, org_id, conversation_id, direction, role,
    origin, author_name, text, attachments_json, external_id, created_at
    FROM messages WHERE rowid > ?1 AND role IN ('user', 'assistant') ORDER BY rowid LIMIT ?2";

const CONSULTA_CONVERSAS: &str = "SELECT id, org_id, agent_id, integration_id, channel,
    external_peer, status, target_department_id, summary, version, created_at, updated_at,
    last_message_at FROM conversations WHERE (updated_at, id) > (?1, ?2)
    ORDER BY updated_at, id LIMIT ?3";

const INDICE_CONVERSAS: &str =
    "CREATE INDEX ix_conversations_atualizada ON conversations (updated_at, id)";

fn env_usize(nome: &str, padrao: usize) -> usize {
    std::env::var(nome)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(padrao)
}

fn mediana_e_p95(mut tempos: Vec<Duration>) -> (Duration, Duration) {
    tempos.sort();
    let meio = tempos[tempos.len() / 2];
    let p95 = tempos[(tempos.len() * 95 / 100).min(tempos.len() - 1)];
    (meio, p95)
}

fn micros(d: Duration) -> f64 {
    d.as_secs_f64() * 1e6
}

/// Preparação: org, agente, conversas e mensagens sintéticas. Devolve o caminho do banco, a org e
/// as conversas criadas.
fn preparar(
    pasta: &PathBuf,
    conversas_n: usize,
    mensagens_n: usize,
) -> (Store, OrgId, Vec<ConversationId>) {
    let _ = fs::remove_dir_all(pasta);
    fs::create_dir_all(pasta).expect("pasta");
    let store = Store::abrir_em(&pasta.join("harness.db"), &pasta.join("rag.db")).expect("abre");
    let org = store
        .orgaos()
        .criar(NovaOrg {
            nome: "Medição".to_owned(),
            slug: "medicao-mx05".to_owned(),
            config_json: String::new(),
        })
        .expect("org")
        .id;
    let agente = store
        .agentes()
        .criar(
            org,
            NovoAgente {
                nome: "Agente".to_owned(),
                slug: "agente-medicao".to_owned(),
                ..NovoAgente::default()
            },
        )
        .expect("agente")
        .id;

    let caminho = store.caminho_harness().to_path_buf();
    let mut con = Connection::open(&caminho).expect("conexão de carga");
    let agora: i64 = 1_800_000_000_000;
    let org_txt = org.to_string();
    let agente_txt = agente.to_string();
    let tx = con.transaction().expect("transação");
    let mut conversas = Vec::with_capacity(conversas_n);
    {
        let mut insere = tx
            .prepare(
                "INSERT INTO conversations (id, org_id, agent_id, channel, external_peer, status,
                    version, created_at, updated_at, last_message_at)
                 VALUES (?1, ?2, ?3, 'whatsapp', ?4, 'open', 1, ?5, ?5, ?5)",
            )
            .expect("prepara conversas");
        for i in 0..conversas_n {
            let id = ConversationId::generate();
            insere
                .execute(params![
                    id.to_string(),
                    org_txt,
                    agente_txt,
                    format!("peer-{i}"),
                    agora + i as i64
                ])
                .expect("conversa");
            conversas.push(id);
        }
    }
    {
        let mut insere = tx
            .prepare(
                "INSERT INTO messages (id, org_id, conversation_id, direction, role, text,
                    attachments_json, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, '[]', ?7)",
            )
            .expect("prepara mensagens");
        for i in 0..mensagens_n {
            let conversa = conversas[i % conversas_n].to_string();
            let (direcao, papel) = match i % 10 {
                0..=3 => ("in", "user"),
                4..=7 => ("out", "assistant"),
                8 => ("out", "tool"),
                _ => ("out", "system"),
            };
            insere
                .execute(params![
                    MessageId::generate().to_string(),
                    org_txt,
                    conversa,
                    direcao,
                    papel,
                    "texto sintético da medição da leitura incremental",
                    agora + i as i64
                ])
                .expect("mensagem");
        }
    }
    tx.commit().expect("commit da carga");
    (store, org, conversas)
}

fn imprime_plano(con: &Connection, rotulo: &str, sql: &str, parametros: &[&dyn rusqlite::ToSql]) {
    let mut consulta = con
        .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
        .expect("explain");
    let linhas = consulta
        .query_map(parametros, |linha| linha.get::<_, String>(3))
        .expect("plano");
    println!("plano · {rotulo}:");
    for linha in linhas {
        println!("    {}", linha.expect("linha do plano"));
    }
}

#[test]
#[ignore = "medição de minutos; rodar com --ignored --nocapture (de preferência com --release)"]
fn medicao_leitura_incremental_e_custo_do_indice() {
    let mensagens_n = env_usize("POPH_MX05_MENSAGENS", 1_000_000);
    let conversas_n = env_usize("POPH_MX05_CONVERSAS", 5_000);
    let pasta = std::env::temp_dir().join(format!("poph-mx05-medicao-{}", std::process::id()));

    let inicio = Instant::now();
    let (store, org, conversas) = preparar(&pasta, conversas_n, mensagens_n);
    println!(
        "carga: {mensagens_n} mensagens em {conversas_n} conversas, preparadas em {:.1} s",
        inicio.elapsed().as_secs_f64()
    );

    let caminho = store.caminho_harness().to_path_buf();
    let con = Connection::open(&caminho).expect("conexão de leitura");
    let meio = (mensagens_n as i64) / 2;
    imprime_plano(
        &con,
        "mensagens desde(rowid)",
        CONSULTA_MENSAGENS,
        &[&meio, &200_i64],
    );
    imprime_plano(
        &con,
        "conversas alteradas_desde",
        CONSULTA_CONVERSAS,
        &[&0_i64, &"", &200_i64],
    );

    // 1) Latência de uma página de 200 pela API (leitor), no meio da tabela e no início.
    let repeticoes = 300;
    for (rotulo, apos) in [("início", 0_u64), ("meio", meio as u64)] {
        let mut tempos = Vec::with_capacity(repeticoes);
        for _ in 0..repeticoes {
            let t = Instant::now();
            let pagina = store.mensagens().desde(apos, 200).expect("desde");
            tempos.push(t.elapsed());
            assert!(pagina.len() <= 200);
        }
        let (mediana, p95) = mediana_e_p95(tempos);
        println!(
            "página de 200 mensagens desde o {rotulo}: mediana {:.0} µs, p95 {:.0} µs",
            micros(mediana),
            micros(p95)
        );
    }
    let mut tempos = Vec::with_capacity(repeticoes);
    for _ in 0..repeticoes {
        let t = Instant::now();
        let pagina = store
            .conversas()
            .alteradas_desde(Some((1_800_000_000_000, conversas[0])), 200)
            .expect("alteradas");
        tempos.push(t.elapsed());
        assert!(pagina.len() <= 200);
    }
    let (mediana, p95) = mediana_e_p95(tempos);
    println!(
        "página de 200 conversas alteradas_desde: mediana {:.0} µs, p95 {:.0} µs",
        micros(mediana),
        micros(p95)
    );

    // 2) Escrita do turno: uma mensagem + o marcador de atividade da conversa (`tocar`), que é o
    //    que o turno faz. Medido com e sem o índice novo de `conversations`, em dois turnos cada.
    let turnos = 20_000_usize;
    let rodada = |rotulo: &str| {
        let mut tempos = Vec::with_capacity(turnos);
        let inicio = Instant::now();
        for i in 0..turnos {
            let conversa = conversas[(i * 7919) % conversas_n];
            let t = Instant::now();
            store
                .mensagens()
                .gravar(
                    org,
                    NovaMensagem {
                        conversation_id: conversa,
                        direcao: Direcao::Entrada,
                        papel: Papel::Usuario,
                        origem: Origem::Agente,
                        autor_nome: None,
                        texto: "turno de medição".to_owned(),
                        anexos_json: "[]".to_owned(),
                        id_externo: None,
                        turn_id: None,
                    },
                )
                .expect("grava");
            store
                .conversas()
                .tocar(org, conversa, 1_900_000_000_000 + i as i64)
                .expect("toca");
            tempos.push(t.elapsed());
        }
        let total = inicio.elapsed().as_secs_f64();
        let (mediana, p95) = mediana_e_p95(tempos);
        println!(
            "escrita do turno · {rotulo}: {:.0} turnos/s; mediana {:.0} µs, p95 {:.0} µs",
            turnos as f64 / total,
            micros(mediana),
            micros(p95)
        );
    };

    let ddl = Connection::open(&caminho).expect("conexão de DDL");
    // Alternadas (sem, com, sem, com) para que o aquecimento e o ruído pesem igual nos dois lados.
    for ciclo in 1..=3 {
        ddl.execute_batch("DROP INDEX IF EXISTS ix_conversations_atualizada;")
            .expect("sem índice");
        rodada(&format!("sem o índice ({ciclo}ª)"));
        ddl.execute_batch(INDICE_CONVERSAS).expect("com índice");
        rodada(&format!("com o índice ({ciclo}ª)"));
    }

    drop(con);
    drop(ddl);
    drop(store);
    let _ = fs::remove_dir_all(&pasta);
}
