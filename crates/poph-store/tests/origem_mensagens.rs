//! Testes da MX-03: a origem (`origin`) e o nome do autor (`author_name`) das mensagens.
//!
//! Cobrem: a mensagem do turno continua `agent`, sem autor, em todas as formas de gravação; a
//! resposta humana sobrevive ao backup (ida e volta); um backup antigo, sem as colunas, continua
//! importável; e um banco que já tinha mensagens migra sem perder linha.
//!
//! Todos usam banco de verdade em pasta temporária própria.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use poph_store::backup::ImportTarget;
use poph_store::migrations::{HARNESS_MIGRATIONS, migrate};
use poph_store::{
    Conversa, Direction, NewAgent, NewConversation, NewMessage, NewOrg, NewOutboxItem, Org, Origin,
    Page, Role, Store,
};
use poph_types::{ConversationId, MessageId, TurnId};
use serde_json::Value;

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
            std::env::temp_dir().join(format!("poph-origem-{rotulo}-{}-{n}", std::process::id()));
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

/// Uma org com um agente e uma conversa, para o resto do teste.
fn org_e_conversa(store: &Store, slug: &str) -> (Org, Conversa) {
    let org = store
        .orgs()
        .criar(NewOrg {
            name: format!("Org {slug}"),
            slug: slug.to_owned(),
            config_json: String::new(),
        })
        .expect("cria a org");
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
                canal: "cli".to_owned(),
                peer_externo: Some("cliente-1".to_owned()),
                target_department: None,
            },
        )
        .expect("cria a conversa");
    (org, conversa)
}

fn saida_do_agente(conversa: ConversationId, texto: &str) -> NewMessage {
    NewMessage {
        conversation_id: conversa,
        direction: Direction::Outbound,
        role: Role::Assistant,
        origin: Origin::Agente,
        author_name: None,
        texto: texto.to_owned(),
        attachments_json: "[]".to_owned(),
        id_externo: None,
        turn_id: None,
    }
}

/// Lê origem e autor direto da tabela, sem passar pelo repositório.
fn linha_bruta(store: &Store, id: MessageId) -> (String, Option<String>) {
    let conn = rusqlite::Connection::open(store.harness_path()).expect("abre o banco");
    conn.query_row(
        "SELECT origin, author_name FROM messages WHERE id = ?1",
        [id.to_string()],
        |linha| Ok((linha.get(0)?, linha.get(1)?)),
    )
    .expect("lê a linha")
}

#[test]
fn mensagem_do_turno_continua_agente_sem_autor() {
    let cenario = Cenario::novo("turno");
    let store = cenario.store();
    let (org, conversa) = org_e_conversa(store, "turno");

    // Gravação simples (o que o turno faz para a entrada e a resposta).
    let gravada = store
        .messages()
        .gravar(org.id, saida_do_agente(conversa.id, "olá"))
        .expect("grava");
    assert_eq!(gravada.origin, Origin::Agente);
    assert_eq!(gravada.author_name, None);
    assert_eq!(
        linha_bruta(store, gravada.id),
        ("agent".to_owned(), None),
        "o banco guarda `agent` e autor nulo"
    );

    // Resposta do agente com a fila de saída, na mesma transação.
    let turno = TurnId::generate();
    let (com_saida, _item) = store
        .messages()
        .gravar_com_saida(
            org.id,
            NewMessage {
                turn_id: Some(turno),
                ..saida_do_agente(conversa.id, "resposta do turno")
            },
            NewOutboxItem {
                conversation_id: conversa.id,
                turn_id: turno,
                seq: 0,
                tipo: "text".to_owned(),
                content_json: "{}".to_owned(),
            },
        )
        .expect("grava com saída");
    assert_eq!(com_saida.origin, Origin::Agente);
    assert_eq!(com_saida.author_name, None);
    assert_eq!(linha_bruta(store, com_saida.id), ("agent".to_owned(), None));

    // Leitura pelo repositório devolve o mesmo que o banco guardou.
    let lidas = store
        .messages()
        .listar(org.id, conversa.id, Page::default())
        .expect("lista");
    assert_eq!(lidas.len(), 2);
    assert!(
        lidas
            .iter()
            .all(|m| m.origin == Origin::Agente && m.author_name.is_none())
    );
}

#[test]
fn resposta_humana_vai_e_volta_no_backup_com_o_autor() {
    let origem = Cenario::novo("backup-origem");
    let (org, conversa) = org_e_conversa(origem.store(), "humano");
    let humana = origem
        .store()
        .messages()
        .gravar(
            org.id,
            NewMessage {
                origin: Origin::Human,
                author_name: Some("Ana Atendente".to_owned()),
                ..saida_do_agente(conversa.id, "*[Ana Atendente]*: posso ajudar")
            },
        )
        .expect("grava a resposta humana");
    assert_eq!(
        linha_bruta(origem.store(), humana.id),
        ("human".to_owned(), Some("Ana Atendente".to_owned()))
    );

    let retrato = origem
        .store()
        .org_snapshot(org.id, true)
        .expect("retrato da org");
    let linha = retrato.tables["messages"]
        .iter()
        .find(|linha| linha["id"] == Value::String(humana.id.to_string()))
        .expect("a mensagem humana está no arquivo");
    assert_eq!(linha["origin"], Value::String("human".to_owned()));
    assert_eq!(
        linha["author_name"],
        Value::String("Ana Atendente".to_owned())
    );

    let destino = Cenario::novo("backup-destino");
    destino
        .store()
        .import_snapshot(retrato, ImportTarget::Origin)
        .expect("importa o arquivo");

    let lidas = destino
        .store()
        .messages()
        .listar(org.id, conversa.id, Page::default())
        .expect("lista no destino");
    assert_eq!(lidas.len(), 1);
    assert_eq!(lidas[0].origin, Origin::Human);
    assert_eq!(lidas[0].author_name.as_deref(), Some("Ana Atendente"));
}

#[test]
fn backup_antigo_sem_as_colunas_importa_como_agente() {
    let origem = Cenario::novo("backup-antigo-origem");
    let (org, conversa) = org_e_conversa(origem.store(), "antigo");
    origem
        .store()
        .messages()
        .gravar(
            org.id,
            saida_do_agente(conversa.id, "mensagem de antes da MX-03"),
        )
        .expect("grava");

    // Simula um arquivo gerado antes da migração: sem `origin` e `author_name`.
    let mut retrato = origem
        .store()
        .org_snapshot(org.id, true)
        .expect("retrato da org");
    for linha in retrato.tables.get_mut("messages").expect("tem mensagens") {
        let objeto = linha.as_object_mut().expect("linha é objeto");
        objeto.remove("origin");
        objeto.remove("author_name");
    }

    let destino = Cenario::novo("backup-antigo-destino");
    destino
        .store()
        .import_snapshot(retrato, ImportTarget::Origin)
        .expect("importa o arquivo antigo");

    let lidas = destino
        .store()
        .messages()
        .listar(org.id, conversa.id, Page::default())
        .expect("lista no destino");
    assert_eq!(lidas.len(), 1);
    assert_eq!(lidas[0].origin, Origin::Agente);
    assert_eq!(lidas[0].author_name, None);
}

#[test]
fn migracao_da_origem_preserva_mensagens_que_ja_existiam() {
    static CONTADOR: AtomicUsize = AtomicUsize::new(0);
    let n = CONTADOR.fetch_add(1, Ordering::Relaxed);
    let pasta =
        std::env::temp_dir().join(format!("poph-origem-migracao-{}-{n}", std::process::id()));
    let _ = fs::remove_dir_all(&pasta);
    fs::create_dir_all(&pasta).expect("cria a pasta");
    let caminho = pasta.join("harness.db");
    let conn = rusqlite::Connection::open(&caminho).expect("abre o banco");

    // Banco no estado anterior: só as migrações 1 e 2, com duas mensagens gravadas.
    migrate(&conn, "harness.db", &caminho, &HARNESS_MIGRATIONS[..2], 1).expect("migra até a 2");
    // As mensagens de teste não têm org nem conversa reais: o teste cobre só as colunas novas,
    // então a checagem de chave estrangeira fica desligada nesta conexão.
    conn.execute_batch("PRAGMA foreign_keys = OFF;")
        .expect("desliga a checagem");
    conn.execute_batch(
        "INSERT INTO messages (id, org_id, conversation_id, direction, role, text, created_at)
            VALUES ('m-1', 'org-1', 'conv-1', 'out', 'assistant', 'primeira', 10),
                   ('m-2', 'org-1', 'conv-1', 'in', 'user', 'segunda', 20);",
    )
    .expect("grava mensagens no formato antigo");

    // Aplica a migração 3 sobre o banco com dados.
    let relatorio =
        migrate(&conn, "harness.db", &caminho, HARNESS_MIGRATIONS, 2).expect("migra até a 3");
    assert_eq!(relatorio.version_before, 2);
    assert_eq!(
        relatorio.version_after,
        poph_store::harness_target_version()
    );

    let total: i64 = conn
        .query_row("SELECT count(*) FROM messages", [], |linha| linha.get(0))
        .expect("conta");
    assert_eq!(total, 2, "nenhuma linha perdida");

    let linhas: Vec<(String, String, Option<String>)> = {
        let mut consulta = conn
            .prepare("SELECT id, origin, author_name FROM messages ORDER BY id")
            .expect("prepara");
        consulta
            .query_map([], |linha| {
                Ok((linha.get(0)?, linha.get(1)?, linha.get(2)?))
            })
            .expect("consulta")
            .map(|r| r.expect("linha"))
            .collect()
    };
    assert_eq!(
        linhas,
        vec![
            ("m-1".to_owned(), "agent".to_owned(), None),
            ("m-2".to_owned(), "agent".to_owned(), None),
        ],
        "as mensagens antigas ganham `agent` e autor nulo"
    );

    // O CHECK recusa valor fora do conjunto.
    let recusado = conn.execute(
        "INSERT INTO messages (id, org_id, conversation_id, direction, role, text, created_at, origin)
            VALUES ('m-3', 'org-1', 'conv-1', 'out', 'assistant', 'x', 30, 'robo')",
        [],
    );
    assert!(
        recusado.is_err(),
        "origem fora do conjunto deve ser recusada"
    );

    drop(conn);
    let _ = fs::remove_dir_all(&pasta);
}
