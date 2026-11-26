//! Testes da MX-14 no `poph-store`: a migração 0007 (id do cursor de mensagens) e as operações de
//! cursor e de leitura que o laço de espelhamento usa.
//!
//! Banco de verdade em pasta temporária, apagada no fim.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use poph_store::Store;
use poph_store::migrations::{HARNESS_MIGRATIONS, migrate};
use poph_types::{MessageId, OrgId};

fn pasta(rotulo: &str) -> PathBuf {
    static CONTADOR: AtomicUsize = AtomicUsize::new(0);
    let n = CONTADOR.fetch_add(1, Ordering::Relaxed);
    let pasta = std::env::temp_dir().join(format!("poph-mx14-{rotulo}-{}-{n}", std::process::id()));
    let _ = fs::remove_dir_all(&pasta);
    fs::create_dir_all(&pasta).expect("cria a pasta");
    pasta
}

#[test]
fn migracao_0007_preserva_cursor_existente_e_põe_id_vazio() {
    let pasta = pasta("migracao");
    let caminho = pasta.join("harness.db");
    let conn = rusqlite::Connection::open(&caminho).expect("abre o banco");

    // Estado anterior (migrações 1 a 6), com um cursor já gravado.
    migrate(&conn, "harness.db", &caminho, &HARNESS_MIGRATIONS[..6], 1).expect("migra até a 6");
    conn.execute_batch("PRAGMA foreign_keys = OFF;")
        .expect("desliga");
    conn.execute(
        "INSERT INTO matrix_estado (org_id, messages_cursor, updated_at)
         VALUES ('0190a000-0000-7000-8000-00000000a001', 42, 10)",
        [],
    )
    .expect("grava cursor no formato anterior");

    let relatorio = migrate(&conn, "harness.db", &caminho, HARNESS_MIGRATIONS, 2).expect("até a 7");
    assert_eq!(relatorio.version_before, 6);
    assert_eq!(
        relatorio.version_after,
        poph_store::harness_target_version()
    );

    let (cursor, id): (i64, String) = conn
        .query_row(
            "SELECT messages_cursor, messages_cursor_id FROM matrix_estado",
            [],
            |linha| Ok((linha.get(0)?, linha.get(1)?)),
        )
        .expect("lê o cursor");
    assert_eq!(cursor, 42, "o cursor anterior não muda");
    assert_eq!(id, "", "sem id guardado ainda");
    let integridade: String = conn
        .query_row("PRAGMA integrity_check", [], |l| l.get(0))
        .expect("integridade");
    assert_eq!(integridade, "ok");
    drop(conn);
    let _ = fs::remove_dir_all(&pasta);
}

#[test]
fn cursor_com_id_so_anda_para_frente_e_recuo_so_pelo_ponto_seguro() {
    let pasta = pasta("cursor");
    let store = Store::open_at(&pasta.join("harness.db"), &pasta.join("rag.db")).expect("abre");
    let org = store
        .orgs()
        .criar(poph_store::NewOrg {
            name: "Org".to_owned(),
            slug: "org-cursor".to_owned(),
            config_json: String::new(),
        })
        .expect("cria a org")
        .id;
    let id_a = MessageId::generate().to_string();
    let id_b = MessageId::generate().to_string();

    let primeiro = store
        .matrix()
        .avancar_mensagens_com_id(org, 10, &id_a)
        .expect("avança");
    assert_eq!(primeiro.mensagens, 10);
    assert_eq!(primeiro.mensagens_id, id_a);

    // Para trás não muda nada: nem o número nem o id.
    let parado = store
        .matrix()
        .avancar_mensagens_com_id(org, 5, &id_b)
        .expect("tenta recuar");
    assert_eq!(parado.mensagens, 10);
    assert_eq!(parado.mensagens_id, id_a);

    // Recuo explícito (R5) grava o ponto seguro.
    let recuado = store
        .matrix()
        .recuar_cursor_mensagens(org, 5, &id_b)
        .expect("recua");
    assert_eq!(recuado.mensagens, 5);
    assert_eq!(recuado.mensagens_id, id_b);

    let lido = store.matrix().ler_cursores(org).expect("lê");
    assert_eq!(lido.mensagens, 5);
    assert_eq!(lido.mensagens_id, id_b);
    drop(store);
    let _ = fs::remove_dir_all(&pasta);
}

#[test]
fn ultima_rowid_e_teve_mensagens_antes_em_banco_vazio() {
    let pasta = pasta("vazio");
    let store = Store::open_at(&pasta.join("harness.db"), &pasta.join("rag.db")).expect("abre");
    assert!(store.messages().ultima_rowid().expect("ultima").is_none());
    let org = OrgId::generate();
    let conversa = poph_types::ConversationId::generate();
    assert!(
        !store
            .messages()
            .teve_mensagens_antes(org, conversa, i64::MAX)
            .expect("teve")
    );
    drop(store);
    let _ = fs::remove_dir_all(&pasta);
}
