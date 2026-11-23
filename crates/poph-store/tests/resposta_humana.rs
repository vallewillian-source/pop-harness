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

use poph_store::migracoes::{MIGRACOES_HARNESS, migrar};
use poph_store::{
    Conversa, GravacaoHumana, NovaConversa, NovaOrg, NovoAgente, Org, Origem, RespostaHumana,
    RespostaHumanaNova, StatusConversa, Store,
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
        let store = Store::abrir_em(&pasta.join("harness.db"), &pasta.join("rag.db"))
            .expect("abre o banco");
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
        .orgaos()
        .criar(NovaOrg {
            nome: format!("Org {slug}"),
            slug: slug.to_owned(),
            config_json: String::new(),
        })
        .expect("cria a org")
}

/// Uma org com um agente e uma conversa aberta, para o resto do teste.
fn org_e_conversa(store: &Store, slug: &str) -> (Org, Conversa) {
    let org = criar_org(store, slug);
    let agente = store
        .agentes()
        .criar(
            org.id,
            NovoAgente {
                nome: "Bia".to_owned(),
                slug: "bia".to_owned(),
                ..NovoAgente::default()
            },
        )
        .expect("cria o agente");
    let conversa = store
        .conversas()
        .criar(
            org.id,
            NovaConversa {
                agent_id: agente.id,
                integration_id: None,
                canal: "matrix".to_owned(),
                peer_externo: Some("!sala:matrix-dev.local".to_owned()),
                departamento_alvo: None,
            },
        )
        .expect("cria a conversa");
    (org, conversa)
}

fn conexao(store: &Store) -> Connection {
    Connection::open(store.caminho_harness()).expect("abre o banco para conferir")
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
) -> poph_store::erro::Resultado<RespostaHumana> {
    let enviado = format!("*[Ana]*:\n{texto}");
    store.mensagens().registrar_resposta_humana(
        org,
        conversa,
        RespostaHumanaNova {
            evento_id: evento,
            texto_bruto: texto,
            texto_enviado: &enviado,
            autor_id: "@ph_ana:matrix-dev.local",
            autor_nome: "Ana",
        },
    )
}

#[test]
fn gravacao_feliz_grava_mensagem_saida_passagem_e_auditoria() {
    let cenario = Cenario::novo("feliz");
    let store = cenario.store();
    let (org, conversa) = org_e_conversa(store, "feliz");
    assert_eq!(conversa.versao, 1);

    let resposta = gravar(store, org.id, conversa.id, "$evento-1", "oi, sou a Ana")
        .expect("registra a resposta");
    let RespostaHumana::Gravada(gravacao) = resposta else {
        panic!("esperava Gravada, veio {resposta:?}");
    };
    let GravacaoHumana {
        mensagem,
        saida,
        transferiu_agora,
    } = *gravacao;

    // Mensagem: origem humana, autor e id do evento, e o texto que o atendente escreveu.
    assert_eq!(mensagem.origem, Origem::Humano);
    assert_eq!(mensagem.autor_nome.as_deref(), Some("Ana"));
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
        serde_json::from_str(&saida.conteudo_json).expect("payload é JSON");
    assert_eq!(conteudo["texto"], "*[Ana]*:\noi, sou a Ana");

    // Passagem: a conversa virou transferida e a versão subiu uma vez.
    assert!(transferiu_agora);
    let depois = store
        .conversas()
        .obter(org.id, conversa.id)
        .expect("lê a conversa");
    assert_eq!(depois.status, StatusConversa::Transferida);
    assert_eq!(depois.versao, 2);

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
    assert!(matches!(primeira, RespostaHumana::Gravada(_)));

    // Reentrega do mesmo evento (texto diferente, só para provar que não é regravado).
    let repetida = gravar(store, org.id, conversa.id, "$evento-2", "outro texto").expect("repete");
    assert_eq!(repetida, RespostaHumana::Duplicada);

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
            conversa.versao,
            StatusConversa::Fechada,
            None,
        )
        .expect("fecha a conversa");

    let resposta = gravar(store, org.id, conversa.id, "$evento-3", "tarde demais").expect("chama");
    assert_eq!(resposta, RespostaHumana::ConversaFechada);

    let conversa_id = conversa.id.to_string();
    assert_eq!(
        store
            .mensagens()
            .contar(org.id, conversa.id)
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
    assert_eq!(resposta, RespostaHumana::ConversaInexistente);

    // Nada foi gravado na conversa da org A.
    let conversa_id = conversa_a.id.to_string();
    assert_eq!(
        store
            .mensagens()
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
    let depois = store
        .conversas()
        .obter(org_a.id, conversa_a.id)
        .expect("lê");
    assert_eq!(depois.status, StatusConversa::Aberta);
    assert_eq!(depois.versao, 1);
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
    let depois = store.conversas().obter(org.id, conversa.id).expect("lê");
    assert_eq!(depois.status, StatusConversa::Aberta, "status não trocou");
    assert_eq!(depois.versao, 1, "versão não subiu");

    // Sem o gatilho, a mesma resposta passa: a falha era só a auditoria forçada.
    conexao(store)
        .execute_batch("DROP TRIGGER recusa_takeover")
        .expect("remove o gatilho");
    let depois_do_gatilho = gravar(store, org.id, conversa.id, "$evento-5", "agora vai");
    assert!(matches!(depois_do_gatilho, Ok(RespostaHumana::Gravada(_))));
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
    let antes = store.conversas().obter(org.id, conversa.id).expect("lê");
    assert_eq!(antes.status, StatusConversa::Transferida);
    assert_eq!(antes.versao, 2);

    let primeira = gravar(store, org.id, conversa.id, "$evento-6", "primeira").expect("grava");
    let RespostaHumana::Gravada(gravacao) = primeira else {
        panic!("esperava Gravada, veio {primeira:?}");
    };
    let transferiu_agora = gravacao.transferiu_agora;
    assert!(
        !transferiu_agora,
        "já estava transferida: esta resposta não transfere"
    );

    let segunda = gravar(store, org.id, conversa.id, "$evento-7", "segunda").expect("grava");
    assert!(matches!(segunda, RespostaHumana::Gravada(_)));

    let depois = store.conversas().obter(org.id, conversa.id).expect("lê");
    assert_eq!(depois.status, StatusConversa::Transferida);
    assert_eq!(depois.versao, 2, "a versão não sobe a cada resposta");
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
        migrar(&conn, "harness.db", &caminho, &MIGRACOES_HARNESS[..3], 1).expect("migra até a 3");
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
    let relatorio = migrar(&conn, "harness.db", &caminho, MIGRACOES_HARNESS, 2).expect("migra");
    assert_eq!(relatorio.versao_depois, poph_store::versao_alvo_harness());

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
) -> poph_store::erro::Resultado<RespostaHumana> {
    store.mensagens().registrar_resposta_humana_com_arquivo(
        org,
        conversa,
        poph_store::RespostaHumanaComArquivo {
            evento_id: evento,
            autor_id: "@ph_ana:matrix-dev.local",
            autor_nome: "Ana",
            arquivo: poph_store::ArquivoDaResposta {
                nome: "laudo.pdf",
                mime: "application/pdf",
                tamanho_bytes: 1_024,
                sha256: &"a".repeat(64),
                blob_path: "org/sha",
                legenda: "**[Ana]**",
                status_extracao: "skipped",
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
    let RespostaHumana::Gravada(gravacao) = resultado else {
        panic!("esperava gravada, veio {resultado:?}");
    };

    // A mensagem é de saída, humana, sem texto e com o id do evento.
    assert_eq!(gravacao.mensagem.direcao, poph_store::Direcao::Saida);
    assert_eq!(gravacao.mensagem.origem, Origem::Humano);
    assert_eq!(gravacao.mensagem.texto, "");
    assert_eq!(
        gravacao.mensagem.id_externo.as_deref(),
        Some("$evento-arquivo:matrix-dev.local")
    );
    assert_eq!(gravacao.mensagem.autor_nome.as_deref(), Some("Ana"));

    // O anexo existe, aponta para a mensagem e guarda o tipo real.
    let anexos = store
        .mensagens()
        .anexos(org.id, gravacao.mensagem.id)
        .expect("lista os anexos");
    assert_eq!(anexos.len(), 1);
    assert_eq!(anexos[0].nome, "laudo.pdf");
    assert_eq!(anexos[0].mime, "application/pdf");
    assert_eq!(anexos[0].tamanho_bytes, 1_024);
    assert_eq!(anexos[0].blob_path, "org/sha");
    assert_eq!(anexos[0].status_extracao, "skipped");
    assert_eq!(
        gravacao.mensagem.anexos_json,
        serde_json::json!([anexos[0].id.to_string()]).to_string()
    );

    // O item da fila é `file`, com o payload do HR-24 e a legenda.
    assert_eq!(gravacao.saida.tipo, "file");
    let payload: serde_json::Value =
        serde_json::from_str(&gravacao.saida.conteudo_json).expect("payload em JSON");
    assert_eq!(payload["blob_ref"], "org/sha");
    assert_eq!(payload["nome"], "laudo.pdf");
    assert_eq!(payload["mime"], "application/pdf");
    assert_eq!(payload["tamanho_bytes"], 1_024);
    assert_eq!(payload["sha256"], "a".repeat(64));
    assert_eq!(payload["legenda"], "**[Ana]**");
    assert_eq!(payload["arquivo_id"], anexos[0].id.to_string());

    // A conversa passou a `transferred` e a auditoria foi gravada uma vez.
    assert!(gravacao.transferiu_agora);
    let conversa_lida = store.conversas().obter(org.id, conversa.id).expect("lê");
    assert_eq!(conversa_lida.status, StatusConversa::Transferida);
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
    assert_eq!(segunda, RespostaHumana::Duplicada);

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
        .obter(org.id, conversa.id)
        .expect("lê")
        .versao;
    store
        .conversas()
        .fechar_auditado(org.id, conversa.id, versao, "teste".to_owned())
        .expect("encerra");

    let resultado =
        gravar_arquivo(store, org.id, conversa.id, "$evento:matrix-dev.local").expect("responde");
    assert_eq!(resultado, RespostaHumana::ConversaFechada);
    assert_eq!(contar(store, "SELECT count(*) FROM messages", ""), 0);
    assert_eq!(contar(store, "SELECT count(*) FROM attachments", ""), 0);
    assert_eq!(
        contar(store, "SELECT count(*) FROM outbox WHERE kind = 'file'", ""),
        0
    );
}
