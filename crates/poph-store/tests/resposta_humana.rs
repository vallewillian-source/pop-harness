//! Testes da MX-04: a operação `registrar_resposta_humana` e a migração 0004 (`audit_log` com
//! `actor_kind = 'matrix_user'`).
//!
//! Cobrem: gravação feliz (mensagem, fila de saída, passagem para `transferred` e auditoria na
//! mesma escrita); reentrega do mesmo evento devolve `Duplicada`; conversa `closed` não grava
//! nada; conversa de outra org devolve `ConversaInexistente`; falha no meio (auditoria recusada
//! por gatilho) não deixa nada gravado; conversa já `transferred` não sobe a versão de novo; e a
//! migração preserva a auditoria existente e aceita o novo `actor_kind`.
//!
//! Todos usam banco de verdade em pasta temporária própria.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use poph_store::migrations::{HARNESS_MIGRATIONS, migrate};
use poph_store::{
    Conversa, ConversationStatus, HumanRecording, HumanResponse, NewAgent, NewConversation,
    NewHumanResponse, NewOrg, Org, Origin, Store,
};
use poph_types::{ConversationId, OrgId};
use rusqlite::{Connection, params};

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
            std::env::temp_dir().join(format!("poph-resposta-{rotulo}-{}-{n}", std::process::id()));
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
}

impl Drop for Cenario {
    fn drop(&mut self) {
        // Fecha o escritor antes de apagar a pasta.
        let _ = self.store.take();
        let _ = fs::remove_dir_all(&self.pasta);
    }
}

fn criar_org(store: &Store, slug: &str) -> Org {
    store
        .orgs()
        .criar(NewOrg {
            name: format!("Org {slug}"),
            slug: slug.to_owned(),
            config_json: String::new(),
        })
        .expect("cria a org")
}

/// Uma org com um agente e uma conversa aberta, para o resto do teste.
fn org_e_conversa(store: &Store, slug: &str) -> (Org, Conversa) {
    let org = criar_org(store, slug);
    let agente = store
        .agents()
        .criar(
            org.id,
            NewAgent {
                name: "Bia".to_owned(),
                slug: "bia".to_owned(),
                ..NewAgent::default()
            },
        )
        .expect("cria o agente");
    let conversa = store
        .conversas()
        .criar(
            org.id,
            NewConversation {
                agent_id: agente.id,
                integration_id: None,
                canal: "matrix".to_owned(),
                peer_externo: Some("!sala:matrix-dev.local".to_owned()),
                target_department: None,
            },
        )
        .expect("cria a conversa");
    (org, conversa)
}

fn conexao(store: &Store) -> Connection {
    Connection::open(store.harness_path()).expect("abre o banco para conferir")
}

/// Quantas linhas uma consulta de contagem devolve, lidas direto da tabela. Parâmetro vazio
/// significa consulta sem parâmetro.
fn contar(store: &Store, sql: &str, parametro: &str) -> i64 {
    let conn = conexao(store);
    if parametro.is_empty() {
        conn.query_row(sql, [], |linha| linha.get(0))
            .expect("conta")
    } else {
        conn.query_row(sql, [parametro], |linha| linha.get(0))
            .expect("conta")
    }
}

fn gravar(
    store: &Store,
    org: OrgId,
    conversa: ConversationId,
    evento: &str,
    texto: &str,
) -> poph_store::error::Resultado<HumanResponse> {
    let enviado = format!("*[Ana]*:\n{texto}");
    store.messages().registrar_resposta_humana(
        org,
        conversa,
        NewHumanResponse {
            event_id: evento,
            raw_text: texto,
            sent_text: &enviado,
            author_id: "@ph_ana:matrix-dev.local",
            author_name: "Ana",
        },
    )
}

#[test]
fn gravacao_feliz_grava_mensagem_saida_passagem_e_auditoria() {
    let cenario = Cenario::novo("feliz");
    let store = cenario.store();
    let (org, conversa) = org_e_conversa(store, "feliz");
    assert_eq!(conversa.version, 1);

    let resposta = gravar(store, org.id, conversa.id, "$evento-1", "oi, sou a Ana")
        .expect("registra a resposta");
    let HumanResponse::Stored(gravacao) = resposta else {
        panic!("esperava Gravada, veio {resposta:?}");
    };
    let HumanRecording {
        mensagem,
        saida,
        just_transferred: transferiu_agora,
    } = *gravacao;

    // Mensagem: origem humana, autor e id do evento, e o texto que o atendente escreveu.
    assert_eq!(mensagem.origin, Origin::Human);
    assert_eq!(mensagem.author_name.as_deref(), Some("Ana"));
    assert_eq!(mensagem.id_externo.as_deref(), Some("$evento-1"));
    assert_eq!(mensagem.texto, "oi, sou a Ana");
    assert_eq!(
        mensagem.turn_id,
        Some(saida.turn_id),
        "mensagem e saída ligadas pelo turno"
    );

    // Fila de saída: um item de texto, seq 0, com o texto já formatado.
    assert_eq!(saida.seq, 0);
    assert_eq!(saida.tipo, "text");
    let conteudo: serde_json::Value =
        serde_json::from_str(&saida.content_json).expect("payload é JSON");
    assert_eq!(conteudo["texto"], "*[Ana]*:\noi, sou a Ana");

    // Passagem: a conversa virou transferida e a versão subiu uma vez.
    assert!(transferiu_agora);
    let depois = store
        .conversas()
        .get(org.id, conversa.id)
        .expect("lê a conversa");
    assert_eq!(depois.status, ConversationStatus::Transferred);
    assert_eq!(depois.version, 2);

    // Banco: a linha da mensagem, o item da fila e a auditoria existem.
    let (origem, autor): (String, Option<String>) = conexao(store)
        .query_row(
            "SELECT origin, author_name FROM messages WHERE id = ?1",
            [mensagem.id.to_string()],
            |linha| Ok((linha.get(0)?, linha.get(1)?)),
        )
        .expect("lê a mensagem");
    assert_eq!((origem.as_str(), autor.as_deref()), ("human", Some("Ana")));
    assert_eq!(
        contar(
            store,
            "SELECT count(*) FROM outbox WHERE conversation_id = ?1",
            &conversa.id.to_string()
        ),
        1
    );
    let (tipo_ator, id_ator, acao, entidade): (String, String, String, String) = conexao(store)
        .query_row(
            "SELECT actor_kind, actor_id, action, entity FROM audit_log
              WHERE action = 'human_takeover'",
            [],
            |linha| Ok((linha.get(0)?, linha.get(1)?, linha.get(2)?, linha.get(3)?)),
        )
        .expect("lê a auditoria");
    assert_eq!(tipo_ator, "matrix_user");
    assert_eq!(id_ator, "@ph_ana:matrix-dev.local");
    assert_eq!(acao, "human_takeover");
    assert_eq!(entidade, "conversation");
}

#[test]
fn evento_repetido_devolve_duplicada_sem_segunda_saida() {
    let cenario = Cenario::novo("duplicada");
    let store = cenario.store();
    let (org, conversa) = org_e_conversa(store, "duplicada");

    let primeira = gravar(store, org.id, conversa.id, "$evento-2", "primeira vez").expect("grava");
    assert!(matches!(primeira, HumanResponse::Stored(_)));

    // Reentrega do mesmo evento (texto diferente, só para provar que não é regravado).
    let repetida = gravar(store, org.id, conversa.id, "$evento-2", "outro texto").expect("repete");
    assert_eq!(repetida, HumanResponse::Duplicate);

    let conversa_id = conversa.id.to_string();
    assert_eq!(
        contar(
            store,
            "SELECT count(*) FROM outbox WHERE conversation_id = ?1",
            &conversa_id
        ),
        1,
        "a segunda chamada não cria segunda linha de saída"
    );
    assert_eq!(
        contar(
            store,
            "SELECT count(*) FROM messages WHERE conversation_id = ?1",
            &conversa_id
        ),
        1
    );
    assert_eq!(
        contar(
            store,
            "SELECT count(*) FROM audit_log WHERE action = 'human_takeover' AND entity_id = ?1",
            &conversa_id
        ),
        1,
        "a duplicada não grava auditoria de novo"
    );
}

#[test]
fn conversa_fechada_nao_grava_nada() {
    let cenario = Cenario::novo("fechada");
    let store = cenario.store();
    let (org, conversa) = org_e_conversa(store, "fechada");
    store
        .conversas()
        .mudar_status(
            org.id,
            conversa.id,
            conversa.version,
            ConversationStatus::Closed,
            None,
        )
        .expect("fecha a conversa");

    let resposta = gravar(store, org.id, conversa.id, "$evento-3", "tarde demais").expect("chama");
    assert_eq!(resposta, HumanResponse::ConversationClosed);

    let conversa_id = conversa.id.to_string();
    assert_eq!(
        store.messages().contar(org.id, conversa.id).expect("conta"),
        0
    );
    assert_eq!(
        contar(
            store,
            "SELECT count(*) FROM outbox WHERE conversation_id = ?1",
            &conversa_id
        ),
        0
    );
    assert_eq!(
        contar(
            store,
            "SELECT count(*) FROM audit_log WHERE action = 'human_takeover'",
            ""
        ),
        0
    );
}

#[test]
fn conversa_de_outra_org_devolve_inexistente() {
    let cenario = Cenario::novo("outra-org");
    let store = cenario.store();
    let (org_a, conversa_a) = org_e_conversa(store, "org-a");
    let org_b = criar_org(store, "org-b");

    // Id da conversa de A, chamado com a org B.
    let resposta =
        gravar(store, org_b.id, conversa_a.id, "$evento-4", "vazamento?").expect("chama");
    assert_eq!(resposta, HumanResponse::ConversationMissing);

    // Nada foi gravado na conversa da org A.
    let conversa_id = conversa_a.id.to_string();
    assert_eq!(
        store
            .messages()
            .contar(org_a.id, conversa_a.id)
            .expect("conta"),
        0
    );
    assert_eq!(
        contar(
            store,
            "SELECT count(*) FROM outbox WHERE conversation_id = ?1",
            &conversa_id
        ),
        0
    );
    let depois = store.conversas().get(org_a.id, conversa_a.id).expect("lê");
    assert_eq!(depois.status, ConversationStatus::Open);
    assert_eq!(depois.version, 1);
}

#[test]
fn falha_na_auditoria_desfaz_mensagem_saida_e_passagem() {
    let cenario = Cenario::novo("falha");
    let store = cenario.store();
    let (org, conversa) = org_e_conversa(store, "falha");

    // Gatilho que recusa a auditoria `human_takeover`: força a falha depois da mensagem e da
    // saída terem sido escritas, no meio da mesma operação.
    conexao(store)
        .execute_batch(
            "CREATE TRIGGER recusa_takeover BEFORE INSERT ON audit_log
               WHEN NEW.action = 'human_takeover'
             BEGIN
               SELECT RAISE(ABORT, 'falha forçada no teste');
             END;",
        )
        .expect("cria o gatilho");

    let resultado = gravar(store, org.id, conversa.id, "$evento-5", "vai falhar");
    assert!(resultado.is_err(), "a falha da auditoria precisa aparecer");

    let conversa_id = conversa.id.to_string();
    assert_eq!(
        contar(
            store,
            "SELECT count(*) FROM messages WHERE external_id = '$evento-5'",
            ""
        ),
        0,
        "sem mensagem gravada"
    );
    assert_eq!(
        contar(
            store,
            "SELECT count(*) FROM outbox WHERE conversation_id = ?1",
            &conversa_id
        ),
        0,
        "sem linha de saída"
    );
    let depois = store.conversas().get(org.id, conversa.id).expect("lê");
    assert_eq!(depois.status, ConversationStatus::Open, "status não trocou");
    assert_eq!(depois.version, 1, "versão não subiu");

    // Sem o gatilho, a mesma resposta passa: a falha era só a auditoria forçada.
    conexao(store)
        .execute_batch("DROP TRIGGER recusa_takeover")
        .expect("remove o gatilho");
    let depois_do_gatilho = gravar(store, org.id, conversa.id, "$evento-5", "agora vai");
    assert!(matches!(depois_do_gatilho, Ok(HumanResponse::Stored(_))));
}

#[test]
fn conversa_ja_transferida_continua_transferida_sem_subir_versao() {
    let cenario = Cenario::novo("transferida");
    let store = cenario.store();
    let (org, conversa) = org_e_conversa(store, "transferida");
    store
        .conversas()
        .transferir(
            org.id,
            conversa.id,
            None,
            Some("sistema".to_owned()),
            "{}".to_owned(),
        )
        .expect("transfere");
    let antes = store.conversas().get(org.id, conversa.id).expect("lê");
    assert_eq!(antes.status, ConversationStatus::Transferred);
    assert_eq!(antes.version, 2);

    let primeira = gravar(store, org.id, conversa.id, "$evento-6", "primeira").expect("grava");
    let HumanResponse::Stored(gravacao) = primeira else {
        panic!("esperava Gravada, veio {primeira:?}");
    };
    let transferiu_agora = gravacao.just_transferred;
    assert!(
        !transferiu_agora,
        "já estava transferida: esta resposta não transfere"
    );

    let segunda = gravar(store, org.id, conversa.id, "$evento-7", "segunda").expect("grava");
    assert!(matches!(segunda, HumanResponse::Stored(_)));

    let depois = store.conversas().get(org.id, conversa.id).expect("lê");
    assert_eq!(depois.status, ConversationStatus::Transferred);
    assert_eq!(depois.version, 2, "a versão não sobe a cada resposta");
    assert_eq!(
        contar(
            store,
            "SELECT count(*) FROM outbox WHERE conversation_id = ?1",
            &conversa.id.to_string()
        ),
        2,
        "cada resposta nova entra na fila"
    );
}

/// Migração 0004: a auditoria existente sobrevive à reconstrução da tabela, e o novo
/// `actor_kind` passa enquanto um valor fora da lista continua recusado.
#[test]
fn migracao_da_auditoria_preserva_linhas_e_aceita_matrix_user() {
    static CONTADOR: AtomicUsize = AtomicUsize::new(0);
    let n = CONTADOR.fetch_add(1, Ordering::Relaxed);
    let pasta = std::env::temp_dir().join(format!("poph-resposta-mig-{}-{n}", std::process::id()));
    let _ = fs::remove_dir_all(&pasta);
    fs::create_dir_all(&pasta).expect("cria a pasta");
    let caminho = pasta.join("harness.db");

    {
        let conn = Connection::open(&caminho).expect("abre");
        conn.pragma_update(None, "foreign_keys", "ON")
            .expect("pragma");
        migrate(&conn, "harness.db", &caminho, &HARNESS_MIGRATIONS[..3], 1).expect("migra até a 3");
        for indice in 0..3 {
            conn.execute(
                "INSERT INTO audit_log (id, org_id, actor_kind, actor_id, action, entity, entity_id,
                                        detail_json, created_at)
                 VALUES (?1, NULL, 'cli', NULL, 'antiga', 'agent', ?2, '{}', ?3)",
                params![format!("aud-{indice}"), format!("ent-{indice}"), 10 + indice],
            )
            .expect("insere auditoria antiga");
        }
    }

    let conn = Connection::open(&caminho).expect("reabre");
    let relatorio = migrate(&conn, "harness.db", &caminho, HARNESS_MIGRATIONS, 2).expect("migra");
    assert_eq!(
        relatorio.version_after,
        poph_store::harness_target_version()
    );

    let total: i64 = conn
        .query_row(
            "SELECT count(*) FROM audit_log WHERE action = 'antiga'",
            [],
            |l| l.get(0),
        )
        .expect("conta");
    assert_eq!(total, 3, "nenhuma linha de auditoria perdida");
    let integridade: String = conn
        .query_row("PRAGMA integrity_check", [], |l| l.get(0))
        .expect("integridade");
    assert_eq!(integridade, "ok");

    conn.execute(
        "INSERT INTO audit_log (id, org_id, actor_kind, actor_id, action, entity, entity_id,
                                detail_json, created_at)
         VALUES ('aud-novo', NULL, 'matrix_user', '@ph_ana:x', 'human_takeover', 'conversation',
                 'c', '{}', 20)",
        [],
    )
    .expect("matrix_user é aceito");

    let recusado = conn.execute(
        "INSERT INTO audit_log (id, org_id, actor_kind, actor_id, action, entity, entity_id,
                                detail_json, created_at)
         VALUES ('aud-ruim', NULL, 'engine', NULL, 'x', 'y', NULL, '{}', 21)",
        [],
    );
    assert!(
        recusado.is_err(),
        "actor_kind fora da lista continua recusado"
    );

    // Os dois índices voltaram.
    let indices: i64 = conn
        .query_row(
            "SELECT count(*) FROM sqlite_master
              WHERE type = 'index' AND name IN ('ix_audit_log_time', 'ix_audit_log_org')",
            [],
            |l| l.get(0),
        )
        .expect("índices");
    assert_eq!(indices, 2);

    drop(conn);
    let _ = fs::remove_dir_all(&pasta);
}

// ───────────────────────────── MX-44: resposta com arquivo ─────────────────────────────

/// Grava uma resposta com arquivo (MX-44), com o payload que o módulo Matrix monta.
fn gravar_arquivo(
    store: &Store,
    org: OrgId,
    conversa: ConversationId,
    evento: &str,
) -> poph_store::error::Resultado<HumanResponse> {
    store.messages().registrar_resposta_humana_com_arquivo(
        org,
        conversa,
        poph_store::HumanResponseWithFile {
            event_id: evento,
            author_id: "@ph_ana:matrix-dev.local",
            author_name: "Ana",
            file: poph_store::ResponseFile {
                name: "laudo.pdf",
                mime: "application/pdf",
                size_bytes: 1_024,
                sha256: &"a".repeat(64),
                blob_path: "org/sha",
                caption: "**[Ana]**",
                extraction_status: "skipped",
            },
        },
    )
}

#[test]
fn resposta_com_arquivo_grava_mensagem_anexo_fila_passagem_e_auditoria() {
    let cenario = Cenario::novo("arquivo-feliz");
    let store = cenario.store();
    let (org, conversa) = org_e_conversa(store, "arquivo");

    let resultado = gravar_arquivo(
        store,
        org.id,
        conversa.id,
        "$evento-arquivo:matrix-dev.local",
    )
    .expect("grava");
    let HumanResponse::Stored(gravacao) = resultado else {
        panic!("esperava gravada, veio {resultado:?}");
    };

    // A mensagem é de saída, humana, sem texto e com o id do evento.
    assert_eq!(gravacao.mensagem.direction, poph_store::Direction::Outbound);
    assert_eq!(gravacao.mensagem.origin, Origin::Human);
    assert_eq!(gravacao.mensagem.texto, "");
    assert_eq!(
        gravacao.mensagem.id_externo.as_deref(),
        Some("$evento-arquivo:matrix-dev.local")
    );
    assert_eq!(gravacao.mensagem.author_name.as_deref(), Some("Ana"));

    // O anexo existe, aponta para a mensagem e guarda o tipo real.
    let anexos = store
        .messages()
        .anexos(org.id, gravacao.mensagem.id)
        .expect("lista os anexos");
    assert_eq!(anexos.len(), 1);
    assert_eq!(anexos[0].name, "laudo.pdf");
    assert_eq!(anexos[0].mime, "application/pdf");
    assert_eq!(anexos[0].size_bytes, 1_024);
    assert_eq!(anexos[0].blob_path, "org/sha");
    assert_eq!(anexos[0].extraction_status, "skipped");
    assert_eq!(
        gravacao.mensagem.attachments_json,
        serde_json::json!([anexos[0].id.to_string()]).to_string()
    );

    // O item da fila é `file`, com o payload do HR-24 e a legenda.
    assert_eq!(gravacao.saida.tipo, "file");
    let payload: serde_json::Value =
        serde_json::from_str(&gravacao.saida.content_json).expect("payload em JSON");
    assert_eq!(payload["blob_ref"], "org/sha");
    assert_eq!(payload["nome"], "laudo.pdf");
    assert_eq!(payload["mime"], "application/pdf");
    assert_eq!(payload["tamanho_bytes"], 1_024);
    assert_eq!(payload["sha256"], "a".repeat(64));
    assert_eq!(payload["legenda"], "**[Ana]**");
    assert_eq!(payload["arquivo_id"], anexos[0].id.to_string());

    // A conversa passou a `transferred` e a auditoria foi gravada uma vez.
    assert!(gravacao.just_transferred);
    let conversa_lida = store.conversas().get(org.id, conversa.id).expect("lê");
    assert_eq!(conversa_lida.status, ConversationStatus::Transferred);
    assert_eq!(
        contar(
            store,
            "SELECT count(*) FROM audit_log WHERE action = 'human_takeover'",
            ""
        ),
        1
    );
}

#[test]
fn reentrega_do_mesmo_arquivo_devolve_duplicada_sem_segundo_anexo_nem_saida() {
    let cenario = Cenario::novo("arquivo-duplicado");
    let store = cenario.store();
    let (org, conversa) = org_e_conversa(store, "arquivo-dup");

    gravar_arquivo(store, org.id, conversa.id, "$evento:matrix-dev.local").expect("primeira");
    let segunda = gravar_arquivo(store, org.id, conversa.id, "$evento:matrix-dev.local")
        .expect("segunda não falha");
    assert_eq!(segunda, HumanResponse::Duplicate);

    assert_eq!(
        contar(store, "SELECT count(*) FROM messages", ""),
        1,
        "uma mensagem só"
    );
    assert_eq!(
        contar(store, "SELECT count(*) FROM attachments", ""),
        1,
        "um anexo só"
    );
    assert_eq!(
        contar(store, "SELECT count(*) FROM outbox WHERE kind = 'file'", ""),
        1,
        "um item de arquivo só"
    );
}

#[test]
fn conversa_fechada_nao_grava_arquivo_nem_anexo() {
    let cenario = Cenario::novo("arquivo-fechada");
    let store = cenario.store();
    let (org, conversa) = org_e_conversa(store, "arquivo-fechada");
    let versao = store
        .conversas()
        .get(org.id, conversa.id)
        .expect("lê")
        .version;
    store
        .conversas()
        .fechar_auditado(org.id, conversa.id, versao, "teste".to_owned())
        .expect("encerra");

    let resultado =
        gravar_arquivo(store, org.id, conversa.id, "$evento:matrix-dev.local").expect("responde");
    assert_eq!(resultado, HumanResponse::ConversationClosed);
    assert_eq!(contar(store, "SELECT count(*) FROM messages", ""), 0);
    assert_eq!(contar(store, "SELECT count(*) FROM attachments", ""), 0);
    assert_eq!(
        contar(store, "SELECT count(*) FROM outbox WHERE kind = 'file'", ""),
        0
    );
}
