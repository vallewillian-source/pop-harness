//! Testes do nome do cliente em `conversations` (MX-32).
//!
//! Todos usam banco de verdade em pasta temporária própria. Cobrem: a normalização do nome; criar,
//! atualizar e "o último vence"; que a gravação não sobe `version` nem `last_message_at`; que
//! nome vazio não escreve; que uma org não lê nem altera o nome de outra; e a migração sobre um
//! banco que já tinha conversas (elas ficam com `NULL`).

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};

use poph_store::migrations::migrate;
use poph_store::{
    Agente, Conversa, HARNESS_MIGRATIONS, LIMITE_NOME_CLIENTE, NewAgent, NewChannelInboxItem,
    NewConversation, NewOrg, Org, Received, Store, file_version, harness_target_version,
    normalizar_nome_cliente,
};
use poph_types::{ConversationId, IntegrationId, OrgId};

/// Banco de verdade numa pasta temporária, apagada no fim, com relógio controlado.
struct Banco {
    pasta: PathBuf,
    relogio: Arc<AtomicI64>,
    store: Store,
}

impl Banco {
    fn abrir(rotulo: &str) -> Self {
        let pasta = pasta_nova(rotulo);
        let relogio = Arc::new(AtomicI64::new(1_000));
        let agora = Arc::clone(&relogio);
        let store = Store::open_at(&pasta.join("harness.db"), &pasta.join("rag.db"))
            .expect("abre o banco")
            .with_clock(Arc::new(move || agora.load(Ordering::SeqCst)));
        Self {
            pasta,
            relogio,
            store,
        }
    }

    fn definir_relogio(&self, valor: i64) {
        self.relogio.store(valor, Ordering::SeqCst);
    }
}

impl Drop for Banco {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.pasta);
    }
}

fn pasta_nova(rotulo: &str) -> PathBuf {
    static CONTADOR: AtomicUsize = AtomicUsize::new(0);
    let n = CONTADOR.fetch_add(1, Ordering::Relaxed);
    let pasta = std::env::temp_dir().join(format!(
        "poph-nome-cliente-{rotulo}-{}-{n}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&pasta);
    fs::create_dir_all(&pasta).expect("cria a pasta do teste");
    pasta
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

fn criar_agente(store: &Store, org: OrgId) -> Agente {
    store
        .agents()
        .criar(
            org,
            NewAgent {
                name: "Atendente".to_owned(),
                slug: "atendente".to_owned(),
                ..NewAgent::default()
            },
        )
        .expect("cria o agente")
}

fn criar_conversa(store: &Store, org: OrgId, agente: &Agente) -> Conversa {
    store
        .conversas()
        .criar(
            org,
            NewConversation {
                agent_id: agente.id,
                integration_id: None,
                canal: "whatsapp".to_owned(),
                peer_externo: Some("5511999990000".to_owned()),
                target_department: None,
            },
        )
        .expect("cria a conversa")
}

#[test]
fn normalizacao_limpa_espacos_controle_e_marcas_de_direcao() {
    assert_eq!(
        normalizar_nome_cliente("  Ana   Maria  ").as_deref(),
        Some("Ana Maria")
    );
    // Largura zero some sem separar as palavras.
    assert_eq!(
        normalizar_nome_cliente("Ana\u{200B}Maria").as_deref(),
        Some("AnaMaria")
    );
    // Marca de direção de texto some (o nome não pode virar texto invertido).
    assert_eq!(
        normalizar_nome_cliente("\u{202E}Ana\u{202C}").as_deref(),
        Some("Ana")
    );
    assert_eq!(
        normalizar_nome_cliente("\u{2066}Ana\u{2069}").as_deref(),
        Some("Ana")
    );
    // Quebra de linha e tabulação viram espaço; controle sem espaço some.
    assert_eq!(
        normalizar_nome_cliente("Ana\tSilva\nReis").as_deref(),
        Some("Ana Silva Reis")
    );
    assert_eq!(
        normalizar_nome_cliente("Ma\u{0007}ria").as_deref(),
        Some("Maria")
    );
}

#[test]
fn normalizacao_recusa_nome_vazio_e_corta_em_cem_caracteres() {
    assert_eq!(normalizar_nome_cliente(""), None);
    assert_eq!(normalizar_nome_cliente("   \n\t "), None);
    assert_eq!(normalizar_nome_cliente("\u{200B}\u{2066}\u{0007}"), None);

    let longo = "a".repeat(150);
    let cortado = normalizar_nome_cliente(&longo).expect("nome não vazio");
    assert_eq!(cortado.chars().count(), LIMITE_NOME_CLIENTE);

    // Corte por caractere, não por byte: `é` ocupa dois bytes.
    let acentuado = "é".repeat(120);
    let cortado = normalizar_nome_cliente(&acentuado).expect("nome não vazio");
    assert_eq!(cortado.chars().count(), LIMITE_NOME_CLIENTE);

    // Se o corte cair num espaço, ele não fica na ponta.
    let com_espaco = format!("{} b", "a".repeat(99));
    let cortado = normalizar_nome_cliente(&com_espaco).expect("nome não vazio");
    assert_eq!(cortado.chars().count(), 99);
    assert!(!cortado.ends_with(' '));
}

#[test]
fn definir_cria_o_nome_e_o_ultimo_que_chegou_vence() {
    let banco = Banco::abrir("ultimo-vence");
    let store = &banco.store;
    let org = criar_org(store, "a");
    let agente = criar_agente(store, org.id);
    let conversa = criar_conversa(store, org.id, &agente);
    assert_eq!(conversa.customer_name, None);

    assert!(
        store
            .conversas()
            .set_customer_name(org.id, conversa.id, "Ana")
            .expect("grava")
    );
    let lida = store.conversas().get(org.id, conversa.id).expect("lê");
    assert_eq!(lida.customer_name.as_deref(), Some("Ana"));

    // Mesmo nome de novo: nada a mudar.
    assert!(
        !store
            .conversas()
            .set_customer_name(org.id, conversa.id, "Ana")
            .expect("repete")
    );
    // Nome novo: o último que chegou vence.
    assert!(
        store
            .conversas()
            .set_customer_name(org.id, conversa.id, "Ana Maria")
            .expect("atualiza")
    );
    let lida = store.conversas().get(org.id, conversa.id).expect("lê");
    assert_eq!(lida.customer_name.as_deref(), Some("Ana Maria"));
}

#[test]
fn gravar_o_nome_sobe_updated_at_e_nao_sobe_version_nem_ultima_mensagem() {
    let banco = Banco::abrir("sem-version");
    let store = &banco.store;
    let org = criar_org(store, "a");
    let agente = criar_agente(store, org.id);
    let conversa = criar_conversa(store, org.id, &agente);
    store
        .conversas()
        .tocar(org.id, conversa.id, 2_000)
        .expect("marca a última mensagem");
    let antes = store.conversas().get(org.id, conversa.id).expect("lê");
    assert_eq!(antes.version, 1);
    assert_eq!(antes.last_message_at, Some(2_000));

    banco.definir_relogio(5_000);
    store
        .conversas()
        .set_customer_name(org.id, conversa.id, "Ana")
        .expect("grava");

    let depois = store.conversas().get(org.id, conversa.id).expect("lê");
    assert_eq!(
        depois.updated_at, 5_000,
        "o cursor do espelho enxerga a mudança"
    );
    assert_eq!(
        depois.version, 1,
        "não gera conflito de If-Match no Console"
    );
    assert_eq!(
        depois.last_message_at,
        Some(2_000),
        "não mexe na última mensagem"
    );
}

#[test]
fn nome_vazio_nao_escreve_e_o_mesmo_nome_nao_escreve_de_novo() {
    let banco = Banco::abrir("sem-escrita");
    let store = &banco.store;
    let org = criar_org(store, "a");
    let agente = criar_agente(store, org.id);
    let conversa = criar_conversa(store, org.id, &agente);

    assert!(
        !store
            .conversas()
            .set_customer_name(org.id, conversa.id, "  \u{200B} ")
            .expect("nome vazio")
    );
    let depois_do_vazio = store.conversas().get(org.id, conversa.id).expect("lê");
    assert_eq!(depois_do_vazio.customer_name, None);
    assert_eq!(depois_do_vazio.updated_at, 1_000, "vazio não escreve nada");

    banco.definir_relogio(3_000);
    assert!(
        store
            .conversas()
            .set_customer_name(org.id, conversa.id, "Ana")
            .expect("primeira vez")
    );
    banco.definir_relogio(7_000);
    assert!(
        !store
            .conversas()
            .set_customer_name(org.id, conversa.id, "Ana")
            .expect("repetição")
    );
    let depois = store.conversas().get(org.id, conversa.id).expect("lê");
    assert_eq!(
        depois.updated_at, 3_000,
        "repetir o mesmo nome não toca na linha"
    );
}

#[test]
fn outra_org_nao_le_nem_altera_o_nome() {
    let banco = Banco::abrir("isolamento");
    let store = &banco.store;
    let org_a = criar_org(store, "a");
    let org_b = criar_org(store, "b");
    let agente_a = criar_agente(store, org_a.id);
    let conversa_a = criar_conversa(store, org_a.id, &agente_a);
    store
        .conversas()
        .set_customer_name(org_a.id, conversa_a.id, "Ana")
        .expect("grava na org A");

    let alterou = store
        .conversas()
        .set_customer_name(org_b.id, conversa_a.id, "Intruso")
        .expect("tentativa da org B");
    assert!(!alterou, "a org B não atinge a conversa da org A");
    assert!(
        store.conversas().get(org_b.id, conversa_a.id).is_err(),
        "a org B não lê a conversa da org A"
    );
    let da_a = store.conversas().get(org_a.id, conversa_a.id).expect("lê");
    assert_eq!(da_a.customer_name.as_deref(), Some("Ana"));
}

#[test]
fn migracao_aplica_em_banco_com_conversas_e_deixa_nulo() {
    // Banco na versão anterior à 8, com org, agente, conversa e mensagem já gravados.
    let pasta = pasta_nova("migracao");
    let harness = pasta.join("harness.db");
    let rag = pasta.join("rag.db");
    let antes = HARNESS_MIGRATIONS.len() - 1;
    let org_id = uuid::Uuid::now_v7();
    let agente_id = uuid::Uuid::now_v7();
    let conversa_id = uuid::Uuid::now_v7();
    {
        let conn = rusqlite::Connection::open(&harness).expect("abre o banco antigo");
        migrate(
            &conn,
            "harness.db",
            &harness,
            &HARNESS_MIGRATIONS[..antes],
            1,
        )
        .expect("migra até a versão anterior");
        conn.execute_batch(&format!(
            "INSERT INTO orgs (id, name, slug, created_at, updated_at)
                VALUES ('{org_id}', 'Org antiga', 'org-antiga', 1, 1);
             INSERT INTO agents (id, org_id, name, slug, provider, model, created_at, updated_at)
                VALUES ('{agente_id}', '{org_id}', 'Agente', 'agente', 'antigravity2api',
                        'gemini', 1, 1);
             INSERT INTO conversations (id, org_id, agent_id, channel, created_at, updated_at)
                VALUES ('{conversa_id}', '{org_id}', '{agente_id}', 'whatsapp', 1, 1);"
        ))
        .expect("grava dados na versão anterior");
    }

    let store = Store::open_at(&harness, &rag).expect("abre e migra");
    assert_eq!(
        file_version(&harness).expect("lê versão"),
        Some(harness_target_version())
    );
    let org: OrgId = org_id.to_string().parse().expect("org válida");
    let conversa: ConversationId = conversa_id.to_string().parse().expect("conversa válida");
    let antiga = store
        .conversas()
        .get(org, conversa)
        .expect("a conversa antiga continua no banco");
    assert_eq!(antiga.customer_name, None, "conversa antiga fica sem nome");

    assert!(
        store
            .conversas()
            .set_customer_name(org, conversa, "Ana")
            .expect("grava sobre a conversa antiga")
    );
    let depois = store.conversas().get(org, conversa).expect("lê");
    assert_eq!(depois.customer_name.as_deref(), Some("Ana"));
    let _ = fs::remove_dir_all(&pasta);
}

// ───────────── nome gravado pela entrega do canal (caminho do webhook, MX-32) ─────────────

/// Integração de WhatsApp de verdade (a conversa tem chave estrangeira para ela).
fn criar_integracao(store: &Store, org: OrgId, agente: poph_types::AgentId) -> IntegrationId {
    store
        .create_integration(org, agente, "whatsapp".to_owned(), "active".to_owned())
        .expect("cria a integração")
        .id
        .parse()
        .expect("id da integração válido")
}

fn entrada_do_canal(
    integracao: IntegrationId,
    agente: &Agente,
    id_externo: &str,
    nome: Option<&str>,
) -> NewChannelInboxItem {
    NewChannelInboxItem {
        integration_id: integracao,
        agent_id: agente.id,
        canal: "whatsapp".to_owned(),
        id_externo: id_externo.to_owned(),
        peer: "5511999990000".to_owned(),
        event_json: "{}".to_owned(),
        texto: "oi".to_owned(),
        anexos: Vec::new(),
        customer_name: nome.map(str::to_owned),
    }
}

#[test]
fn entrega_do_canal_grava_o_nome_junto_com_a_mensagem() {
    let banco = Banco::abrir("entrega-nome");
    let org = criar_org(&banco.store, "entrega-nome");
    let agente = criar_agente(&banco.store, org.id);
    let integracao = criar_integracao(&banco.store, org.id, agente.id);

    let recebido = banco
        .store
        .inbox()
        .receber_do_canal(
            org.id,
            entrada_do_canal(integracao, &agente, "M-NOME-1", Some("  Ana   Maria ")),
        )
        .expect("entrega");
    assert!(matches!(recebido.situacao, Received::New));

    let conversa = banco
        .store
        .conversas()
        .get(org.id, recebido.mensagem.conversation_id)
        .expect("conversa");
    assert_eq!(conversa.customer_name.as_deref(), Some("Ana Maria"));
    assert_eq!(conversa.version, 1, "o nome não sobe a versão");
}

#[test]
fn falha_ao_gravar_o_nome_na_entrega_nao_derruba_a_mensagem() {
    let banco = Banco::abrir("entrega-falha");
    let org = criar_org(&banco.store, "entrega-falha");
    let agente = criar_agente(&banco.store, org.id);
    let integracao = criar_integracao(&banco.store, org.id, agente.id);
    {
        let conn = rusqlite::Connection::open(banco.store.harness_path()).expect("abre");
        conn.execute_batch(
            "CREATE TRIGGER falha_nome BEFORE UPDATE OF client_name ON conversations
             BEGIN SELECT RAISE(ABORT, 'falha simulada'); END;",
        )
        .expect("cria o gatilho");
    }

    let recebido = banco
        .store
        .inbox()
        .receber_do_canal(
            org.id,
            entrada_do_canal(integracao, &agente, "M-FALHA-1", Some("Ana")),
        )
        .expect("a entrega não pode falhar por causa do nome");
    assert!(matches!(recebido.situacao, Received::New));
    assert_eq!(
        banco.store.inbox().contar_pendentes(org.id).expect("conta"),
        1
    );

    let conversa = banco
        .store
        .conversas()
        .get(org.id, recebido.mensagem.conversation_id)
        .expect("a conversa existe mesmo sem o nome");
    assert_eq!(
        conversa.customer_name, None,
        "o nome ficou para a próxima mensagem"
    );
}

#[test]
fn reentrega_do_mesmo_evento_nao_reescreve_o_nome() {
    // Limite conhecido (MX-32): a reentrega devolve a mensagem já gravada e não grava de novo; o
    // próximo `pushName` do cliente, numa mensagem nova, é que atualiza o nome.
    let banco = Banco::abrir("entrega-reentrega");
    let org = criar_org(&banco.store, "entrega-reentrega");
    let agente = criar_agente(&banco.store, org.id);
    let integracao = criar_integracao(&banco.store, org.id, agente.id);
    let primeira = banco
        .store
        .inbox()
        .receber_do_canal(
            org.id,
            entrada_do_canal(integracao, &agente, "M-RE-1", Some("Ana")),
        )
        .expect("primeira");
    let repetida = banco
        .store
        .inbox()
        .receber_do_canal(
            org.id,
            entrada_do_canal(integracao, &agente, "M-RE-1", Some("Outra")),
        )
        .expect("repetida");
    assert!(matches!(repetida.situacao, Received::Duplicate));
    assert_eq!(repetida.mensagem.id, primeira.mensagem.id);

    let conversa = banco
        .store
        .conversas()
        .get(org.id, primeira.mensagem.conversation_id)
        .expect("conversa");
    assert_eq!(conversa.customer_name.as_deref(), Some("Ana"));
}
