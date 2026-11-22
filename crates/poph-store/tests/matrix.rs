//! Testes das tabelas do módulo Matrix (MX-08).
//!
//! Todos usam banco de verdade em pasta temporária própria. Cobrem: a migração em banco novo e
//! sobre um banco que já tinha dados na versão anterior; o cascade da exclusão da org; o
//! isolamento por `org_id`; a idempotência das operações que o laço de espelhamento vai repetir; e
//! a revisão de que nenhuma coluna guarda texto de cliente, telefone ou token.

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};

use poph_store::migracoes::migrar;
use poph_store::{
    Agente, Conversa, Departamento, Direcao, ErroStore, EstadoSala, MIGRACOES_HARNESS,
    NovaConversa, NovaMensagem, NovaOrg, NovoAgente, NovoDepartamento, Org, Origem, OrigemEspaco,
    Papel, Store, versao_do_arquivo,
};
use poph_types::{ConversationId, DomainError, MessageId, OrgId};

/// Relógio controlado pelo teste.
#[derive(Clone)]
struct Relogio(Arc<AtomicI64>);

impl Relogio {
    fn novo(inicial: i64) -> Self {
        Self(Arc::new(AtomicI64::new(inicial)))
    }

    fn definir(&self, valor: i64) {
        self.0.store(valor, Ordering::SeqCst);
    }
}

/// Banco de verdade numa pasta temporária, apagada no fim.
struct Cenario {
    pasta: PathBuf,
    relogio: Relogio,
    store: Option<Arc<Store>>,
}

impl Cenario {
    fn novo(rotulo: &str) -> Self {
        Self::com_versao(rotulo, None)
    }

    /// `ate = Some(n)` cria o banco com as migrações 1..=n (simula instalação antiga).
    fn com_versao(rotulo: &str, ate: Option<usize>) -> Self {
        static CONTADOR: AtomicUsize = AtomicUsize::new(0);
        let n = CONTADOR.fetch_add(1, Ordering::Relaxed);
        let pasta =
            std::env::temp_dir().join(format!("poph-matrix-{rotulo}-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&pasta);
        fs::create_dir_all(&pasta).expect("cria a pasta do teste");
        let harness = pasta.join("harness.db");
        let rag = pasta.join("rag.db");
        if let Some(ate) = ate {
            let conn = rusqlite::Connection::open(&harness).expect("abre o banco antigo");
            migrar(&conn, "harness.db", &harness, &MIGRACOES_HARNESS[..ate], 1)
                .expect("migra até a versão antiga");
        }
        let relogio = Relogio::novo(1_000);
        let agora = Arc::clone(&relogio.0);
        let store = Store::abrir_em(&harness, &rag)
            .expect("abre o banco")
            .com_relogio(Arc::new(move || agora.load(Ordering::SeqCst)));
        Self {
            pasta,
            relogio,
            store: Some(Arc::new(store)),
        }
    }

    fn store(&self) -> &Store {
        self.store.as_deref().expect("store viva")
    }

    fn caminho(&self) -> PathBuf {
        self.pasta.join("harness.db")
    }
}

impl Drop for Cenario {
    fn drop(&mut self) {
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

fn criar_agente(store: &Store, org: &Org, slug: &str) -> Agente {
    store
        .agentes()
        .criar(
            org.id,
            NovoAgente {
                nome: format!("Agente {slug}"),
                slug: slug.to_owned(),
                ..NovoAgente::default()
            },
        )
        .expect("cria o agente")
}

fn criar_conversa(store: &Store, org: &Org, agente: &Agente) -> Conversa {
    store
        .conversas()
        .criar(
            org.id,
            NovaConversa {
                agent_id: agente.id,
                integration_id: None,
                canal: "cli".to_owned(),
                peer_externo: Some("teste".to_owned()),
                departamento_alvo: None,
            },
        )
        .expect("cria a conversa")
}

fn criar_departamento(store: &Store, org: &Org, nome: &str) -> Departamento {
    store
        .departamentos()
        .criar(
            org.id,
            NovoDepartamento {
                nome: nome.to_owned(),
                descricao: String::new(),
            },
        )
        .expect("cria o departamento")
}

fn gravar_mensagem(store: &Store, org: &Org, conversa: &Conversa, texto: &str) -> MessageId {
    store
        .mensagens()
        .gravar(
            org.id,
            NovaMensagem {
                conversation_id: conversa.id,
                direcao: Direcao::Entrada,
                papel: Papel::Usuario,
                texto: texto.to_owned(),
                anexos_json: "[]".to_owned(),
                id_externo: None,
                turn_id: None,
                origem: Origem::Agente,
                autor_nome: None,
            },
        )
        .expect("grava a mensagem")
        .id
}

fn linhas_da_tabela(caminho: &PathBuf, tabela: &str, org: OrgId) -> i64 {
    let conn = rusqlite::Connection::open(caminho).expect("abre para conferir");
    conn.query_row(
        &format!("SELECT count(*) FROM {tabela} WHERE org_id = ?1"),
        rusqlite::params![org.to_string()],
        |linha| linha.get(0),
    )
    .expect("conta as linhas")
}

#[test]
fn migracao_cria_as_cinco_tabelas_do_modulo() {
    let cenario = Cenario::novo("esquema");
    let store = cenario.store();

    let tabelas = store.tabelas().expect("lista tabelas");
    for esperada in [
        "matrix_orgs",
        "matrix_rooms",
        "matrix_spaces",
        "matrix_estado",
        "matrix_falhas",
    ] {
        assert!(
            tabelas.iter().any(|tabela| tabela == esperada),
            "tabela {esperada} não criada; existem: {tabelas:?}"
        );
    }
    assert_eq!(
        versao_do_arquivo(&cenario.caminho()).expect("lê versão"),
        Some(poph_store::versao_alvo_harness())
    );
    assert!(store.chaves_quebradas().expect("fk check").is_empty());
}

#[test]
fn migracao_aplica_em_banco_que_ja_tinha_dados() {
    // Banco na versão 4 (antes das tabelas do módulo), com org, agente, conversa e mensagem.
    let cenario = Cenario::com_versao("com-dados", Some(4));
    let org_id = uuid::Uuid::now_v7();
    let agente_id = uuid::Uuid::now_v7();
    let conversa_id = uuid::Uuid::now_v7();
    let mensagem_id = uuid::Uuid::now_v7();
    {
        let conn = rusqlite::Connection::open(cenario.caminho()).expect("abre o banco antigo");
        conn.execute_batch(&format!(
            "INSERT INTO orgs (id, name, slug, created_at, updated_at)
                VALUES ('{org_id}', 'Org antiga', 'org-antiga', 1, 1);
             INSERT INTO agents (id, org_id, name, slug, provider, model, created_at, updated_at)
                VALUES ('{agente_id}', '{org_id}', 'Agente', 'agente', 'antigravity2api',
                        'gemini', 1, 1);
             INSERT INTO conversations (id, org_id, agent_id, channel, created_at, updated_at)
                VALUES ('{conversa_id}', '{org_id}', '{agente_id}', 'cli', 1, 1);
             INSERT INTO messages (id, org_id, conversation_id, direction, role, text, created_at)
                VALUES ('{mensagem_id}', '{org_id}', '{conversa_id}', 'in', 'user',
                        'mensagem antiga', 1);"
        ))
        .expect("grava dados na versão anterior");
    }

    // A abertura do `Store` aplica a migração 5 sobre o banco com dados.
    let store = cenario.store();
    assert_eq!(
        versao_do_arquivo(&cenario.caminho()).expect("lê versão"),
        Some(poph_store::versao_alvo_harness())
    );
    assert!(store.chaves_quebradas().expect("fk check").is_empty());

    let org: OrgId = org_id.to_string().parse().expect("org válida");
    let conversa: ConversationId = conversa_id.to_string().parse().expect("conversa válida");
    store
        .conversas()
        .obter(org, conversa)
        .expect("a conversa antiga continua no banco");

    // As tabelas novas funcionam sobre os dados que já existiam.
    let estado = store.matrix().ligar(org).expect("liga o módulo");
    assert!(estado.ligado);
    let sala = store
        .matrix()
        .garantir_sala(org, conversa, "!antiga:teste", None)
        .expect("sala da conversa antiga");
    assert_eq!(sala.estado, EstadoSala::Criando);
    assert_eq!(sala.conversa, conversa);
}

#[test]
fn apagar_a_org_apaga_as_linhas_das_cinco_tabelas() {
    let cenario = Cenario::novo("cascade");
    let store = cenario.store();

    let mut orgs = Vec::new();
    for slug in ["org-a", "org-b"] {
        let org = criar_org(store, slug);
        let agente = criar_agente(store, &org, slug);
        let conversa = criar_conversa(store, &org, &agente);
        let departamento = criar_departamento(store, &org, "Comercial");
        let mensagem = gravar_mensagem(store, &org, &conversa, "oi");

        store.matrix().ligar(org.id).expect("liga");
        store
            .matrix()
            .garantir_sala(org.id, conversa.id, &format!("!{slug}:teste"), None)
            .expect("sala");
        store
            .matrix()
            .garantir_espaco(
                org.id,
                departamento.id,
                &format!("!espaco-{slug}:teste"),
                &format!("#espaco-{slug}:teste"),
                OrigemEspaco::Automatico,
            )
            .expect("espaço");
        store
            .matrix()
            .avancar_mensagens(org.id, 10)
            .expect("cursor de mensagens");
        store
            .matrix()
            .registrar_falha(org.id, mensagem, "M_UNKNOWN", 1)
            .expect("falha");
        orgs.push(org);
    }

    let (org_a, org_b) = (&orgs[0], &orgs[1]);
    let caminho = cenario.caminho();
    for tabela in [
        "matrix_orgs",
        "matrix_rooms",
        "matrix_spaces",
        "matrix_estado",
        "matrix_falhas",
    ] {
        assert_eq!(
            linhas_da_tabela(&caminho, tabela, org_a.id),
            1,
            "{tabela} da org A antes da exclusão"
        );
        assert_eq!(
            linhas_da_tabela(&caminho, tabela, org_b.id),
            1,
            "{tabela} da org B antes da exclusão"
        );
    }

    store
        .orgaos()
        .excluir_forcado(org_b.id)
        .expect("apaga a org B");

    for tabela in [
        "matrix_orgs",
        "matrix_rooms",
        "matrix_spaces",
        "matrix_estado",
        "matrix_falhas",
    ] {
        assert_eq!(
            linhas_da_tabela(&caminho, tabela, org_b.id),
            0,
            "{tabela} da org excluída ficou para trás"
        );
        assert_eq!(
            linhas_da_tabela(&caminho, tabela, org_a.id),
            1,
            "{tabela} da outra org não podia ser apagada"
        );
    }
    assert!(store.chaves_quebradas().expect("fk check").is_empty());
}

#[test]
fn isolamento_por_org_em_todas_as_tabelas_do_modulo() {
    let cenario = Cenario::novo("isolamento");
    let store = cenario.store();
    let org_a = criar_org(store, "org-a");
    let org_b = criar_org(store, "org-b");
    let agente_a = criar_agente(store, &org_a, "agente-a");
    let conversa_a = criar_conversa(store, &org_a, &agente_a);
    let departamento_a = criar_departamento(store, &org_a, "Comercial");
    let mensagem_a = gravar_mensagem(store, &org_a, &conversa_a, "oi");

    store.matrix().ligar(org_a.id).expect("liga A");
    store
        .matrix()
        .garantir_sala(org_a.id, conversa_a.id, "!sala-a:teste", None)
        .expect("sala A");
    store
        .matrix()
        .garantir_espaco(
            org_a.id,
            departamento_a.id,
            "!espaco-a:teste",
            "#espaco-a:teste",
            OrigemEspaco::Automatico,
        )
        .expect("espaço A");
    store
        .matrix()
        .avancar_mensagens(org_a.id, 7)
        .expect("cursor A");
    store
        .matrix()
        .avancar_conversas(org_a.id, 500, &conversa_a.id.to_string())
        .expect("cursor de conversas A");
    store
        .matrix()
        .registrar_falha(org_a.id, mensagem_a, "M_UNKNOWN", 1)
        .expect("falha A");

    // Nada da org A aparece para a org B.
    let estado_b = store.matrix().estado(org_b.id).expect("estado B");
    assert!(!estado_b.ligado);
    assert_eq!(estado_b.atualizado_em, 0);
    assert!(
        store
            .matrix()
            .orgs_ligadas()
            .expect("orgs ligadas")
            .iter()
            .all(|estado| estado.org_id != org_b.id)
    );

    assert!(
        store
            .matrix()
            .sala(org_b.id, conversa_a.id)
            .expect("sala B")
            .is_none(),
        "a sala da org A não pode ser lida pela org B"
    );
    assert!(
        store
            .matrix()
            .sala_por_room_id(org_b.id, "!sala-a:teste")
            .expect("sala por id B")
            .is_none(),
        "o id de sala da org A não resolve na org B"
    );
    assert!(
        store
            .matrix()
            .espaco(org_b.id, departamento_a.id)
            .expect("espaço B")
            .is_none()
    );
    assert!(
        store
            .matrix()
            .listar_espacos(org_b.id)
            .expect("espaços B")
            .is_empty()
    );
    let cursores_b = store.matrix().ler_cursores(org_b.id).expect("cursores B");
    assert_eq!(cursores_b.mensagens, 0);
    assert_eq!(cursores_b.conversas.atualizado_em, 0);
    assert_eq!(cursores_b.versao, 0);
    assert_eq!(store.matrix().contar_falhas(org_b.id).expect("falhas B"), 0);

    // E a org B não consegue criar sala para conversa da org A (a chave composta recusa).
    let erro = store
        .matrix()
        .garantir_sala(org_b.id, conversa_a.id, "!sala-invasora:teste", None)
        .expect_err("conversa de outra org tem que ser recusada");
    assert!(
        matches!(
            erro,
            ErroStore::Dominio(DomainError::Validacao { .. })
                | ErroStore::Dominio(DomainError::NaoEncontrado { .. })
        ),
        "erro inesperado: {erro:?}"
    );

    // A org A segue com tudo.
    let cursores_a = store.matrix().ler_cursores(org_a.id).expect("cursores A");
    assert_eq!(cursores_a.mensagens, 7);
    assert_eq!(cursores_a.conversas.atualizado_em, 500);
    assert_eq!(store.matrix().contar_falhas(org_a.id).expect("falhas A"), 1);
}

#[test]
fn garantir_sala_duas_vezes_e_idempotente_e_recusa_sala_de_outra_conversa() {
    let cenario = Cenario::novo("sala-idempotente");
    let store = cenario.store();
    let org = criar_org(store, "org");
    let agente = criar_agente(store, &org, "agente");
    let conversa = criar_conversa(store, &org, &agente);
    let outra = criar_conversa(store, &org, &agente);

    let primeira = store
        .matrix()
        .garantir_sala(org.id, conversa.id, "!sala:teste", Some("!espaco:teste"))
        .expect("primeira");
    let segunda = store
        .matrix()
        .garantir_sala(org.id, conversa.id, "!sala:teste", Some("!espaco:teste"))
        .expect("segunda");
    assert_eq!(primeira, segunda, "a segunda chamada devolve a mesma linha");
    assert_eq!(
        linhas_da_tabela(&cenario.caminho(), "matrix_rooms", org.id),
        1
    );

    // Um `room_id` diferente não troca a sala que já está gravada: a primeira ganha.
    let terceira = store
        .matrix()
        .garantir_sala(org.id, conversa.id, "!outra-sala:teste", None)
        .expect("terceira");
    assert_eq!(terceira.room_id, "!sala:teste");
    assert_eq!(terceira.espaco.as_deref(), Some("!espaco:teste"));

    // A mesma sala para outra conversa é conflito, não uma segunda linha.
    let erro = store
        .matrix()
        .garantir_sala(org.id, outra.id, "!sala:teste", None)
        .expect_err("sala de outra conversa tem que ser recusada");
    assert!(
        matches!(erro, ErroStore::Dominio(DomainError::Conflito { .. })),
        "erro inesperado: {erro:?}"
    );
    assert_eq!(
        linhas_da_tabela(&cenario.caminho(), "matrix_rooms", org.id),
        1
    );
}

#[test]
fn marcar_sala_pronta_e_idempotente() {
    let cenario = Cenario::novo("pronta");
    let store = cenario.store();
    let org = criar_org(store, "org");
    let agente = criar_agente(store, &org, "agente");
    let conversa = criar_conversa(store, &org, &agente);
    let fantasma = criar_conversa(store, &org, &agente);

    let sala = store
        .matrix()
        .garantir_sala(org.id, conversa.id, "!sala:teste", None)
        .expect("sala");
    assert_eq!(sala.estado, EstadoSala::Criando);

    cenario.relogio.definir(2_000);
    let pronta = store
        .matrix()
        .marcar_sala_pronta(org.id, conversa.id)
        .expect("pronta");
    assert_eq!(pronta.estado, EstadoSala::Pronta);
    assert_eq!(pronta.atualizada_em, 2_000);

    cenario.relogio.definir(3_000);
    let de_novo = store
        .matrix()
        .marcar_sala_pronta(org.id, conversa.id)
        .expect("pronta de novo");
    assert_eq!(de_novo.estado, EstadoSala::Pronta);
    assert_eq!(de_novo.atualizada_em, 2_000, "já pronta não é reescrita");

    let erro = store
        .matrix()
        .marcar_sala_pronta(org.id, fantasma.id)
        .expect_err("conversa sem sala");
    assert!(erro.eh_nao_encontrado(), "erro inesperado: {erro:?}");
}

#[test]
fn marcar_sala_pronta_grava_o_nome_do_cliente_que_a_sala_usa() {
    let cenario = Cenario::novo("pronta-com-nome");
    let store = cenario.store();
    let org = criar_org(store, "org");
    let agente = criar_agente(store, &org, "agente");
    let conversa = criar_conversa(store, &org, &agente);
    let sem_nome = criar_conversa(store, &org, &agente);

    store
        .matrix()
        .garantir_sala(org.id, conversa.id, "!sala:teste", None)
        .expect("sala");
    let criando = store
        .matrix()
        .sala(org.id, conversa.id)
        .expect("lê a sala")
        .expect("sala existe");
    assert_eq!(criando.nome_cliente_espelhado, None, "nasce sem nome");

    cenario.relogio.definir(2_000);
    let pronta = store
        .matrix()
        .marcar_sala_pronta_com_nome(org.id, conversa.id, Some("Maria"))
        .expect("pronta com nome");
    assert_eq!(pronta.estado, EstadoSala::Pronta);
    assert_eq!(pronta.nome_cliente_espelhado.as_deref(), Some("Maria"));
    assert_eq!(pronta.atualizada_em, 2_000);

    // Já pronta não é reescrita: o nome que vale é o da criação.
    cenario.relogio.definir(3_000);
    let de_novo = store
        .matrix()
        .marcar_sala_pronta_com_nome(org.id, conversa.id, Some("Outro"))
        .expect("pronta de novo");
    assert_eq!(de_novo.nome_cliente_espelhado.as_deref(), Some("Maria"));
    assert_eq!(de_novo.atualizada_em, 2_000);

    // Sala que nasceu com o identificador (cliente sem nome) fica com a coluna vazia.
    store
        .matrix()
        .garantir_sala(org.id, sem_nome.id, "!sala-sem-nome:teste", None)
        .expect("sala sem nome");
    let pronta = store
        .matrix()
        .marcar_sala_pronta(org.id, sem_nome.id)
        .expect("pronta sem nome");
    assert_eq!(pronta.nome_cliente_espelhado, None);
}

#[test]
fn definir_nome_cliente_espelhado_e_idempotente_e_filtrado_por_org() {
    let cenario = Cenario::novo("nome-espelhado");
    let store = cenario.store();
    let org = criar_org(store, "org");
    let outra_org = criar_org(store, "outra-org");
    let agente = criar_agente(store, &org, "agente");
    let conversa = criar_conversa(store, &org, &agente);
    store
        .matrix()
        .garantir_sala(org.id, conversa.id, "!sala:teste", None)
        .expect("sala");
    store
        .matrix()
        .marcar_sala_pronta(org.id, conversa.id)
        .expect("pronta");

    cenario.relogio.definir(2_000);
    let com_nome = store
        .matrix()
        .definir_nome_cliente_espelhado(org.id, conversa.id, "Ana")
        .expect("grava o nome");
    assert_eq!(com_nome.nome_cliente_espelhado.as_deref(), Some("Ana"));
    assert_eq!(com_nome.atualizada_em, 2_000);

    cenario.relogio.definir(3_000);
    let de_novo = store
        .matrix()
        .definir_nome_cliente_espelhado(org.id, conversa.id, "Ana")
        .expect("grava de novo");
    assert_eq!(de_novo.atualizada_em, 2_000, "o mesmo nome não reescreve");

    let trocado = store
        .matrix()
        .definir_nome_cliente_espelhado(org.id, conversa.id, "Ana Maria")
        .expect("nome novo");
    assert_eq!(trocado.nome_cliente_espelhado.as_deref(), Some("Ana Maria"));

    // Isolamento: a outra org não lê nem altera a sala.
    let erro = store
        .matrix()
        .definir_nome_cliente_espelhado(outra_org.id, conversa.id, "Invasora")
        .expect_err("sala de outra org");
    assert!(erro.eh_nao_encontrado(), "erro inesperado: {erro:?}");
    assert!(
        store
            .matrix()
            .sala(outra_org.id, conversa.id)
            .expect("lê pela outra org")
            .is_none()
    );

    // Nome vazio é recusado: o nome nunca volta a vazio.
    let vazio = store
        .matrix()
        .definir_nome_cliente_espelhado(org.id, conversa.id, "   ")
        .expect_err("nome vazio");
    assert!(
        matches!(vazio, ErroStore::Dominio(DomainError::Validacao { .. })),
        "erro inesperado: {vazio:?}"
    );
}

#[test]
fn migracao_do_nome_espelhado_preserva_sala_existente_com_a_coluna_vazia() {
    // Banco na versão 9 (antes da coluna do MX-33), com uma sala já criada.
    let cenario = Cenario::com_versao("nome-espelhado-antigo", Some(9));
    let org_id = uuid::Uuid::now_v7();
    let agente_id = uuid::Uuid::now_v7();
    let conversa_id = uuid::Uuid::now_v7();
    {
        let conn = rusqlite::Connection::open(cenario.caminho()).expect("abre o banco antigo");
        conn.execute_batch(&format!(
            "INSERT INTO orgs (id, name, slug, created_at, updated_at)
                VALUES ('{org_id}', 'Org antiga', 'org-antiga', 1, 1);
             INSERT INTO agents (id, org_id, name, slug, provider, model, created_at, updated_at)
                VALUES ('{agente_id}', '{org_id}', 'Agente', 'agente', 'antigravity2api',
                        'gemini', 1, 1);
             INSERT INTO conversations (id, org_id, agent_id, channel, created_at, updated_at)
                VALUES ('{conversa_id}', '{org_id}', '{agente_id}', 'whatsapp', 1, 1);
             INSERT INTO matrix_rooms
                (conversation_id, org_id, room_id, state, created_at, updated_at)
                VALUES ('{conversa_id}', '{org_id}', '!sala-antiga:teste', 'ready', 1, 1);"
        ))
        .expect("grava a sala na versão anterior");
    }

    let store = cenario.store();
    assert_eq!(
        versao_do_arquivo(&cenario.caminho()).expect("lê versão"),
        Some(poph_store::versao_alvo_harness())
    );
    let org: OrgId = org_id.to_string().parse().expect("org válida");
    let conversa: ConversationId = conversa_id.to_string().parse().expect("conversa válida");
    let sala = store
        .matrix()
        .sala(org, conversa)
        .expect("lê a sala")
        .expect("a sala antiga continua no banco");
    assert_eq!(sala.room_id, "!sala-antiga:teste");
    assert_eq!(sala.estado, EstadoSala::Pronta);
    // Sala antiga nasceu com o telefone: a coluna fica vazia e o laço de estado a renomeia quando
    // o nome do cliente chegar.
    assert_eq!(sala.nome_cliente_espelhado, None);
}

#[test]
fn definir_espaco_da_sala_e_idempotente_e_aceita_tirar_a_sala_do_espaco() {
    let cenario = Cenario::novo("espaco-da-sala");
    let store = cenario.store();
    let org = criar_org(store, "org");
    let agente = criar_agente(store, &org, "agente");
    let conversa = criar_conversa(store, &org, &agente);
    let outra_org = criar_org(store, "outra-org");

    store
        .matrix()
        .garantir_sala(org.id, conversa.id, "!sala:teste", None)
        .expect("sala");

    cenario.relogio.definir(2_000);
    let posta = store
        .matrix()
        .definir_espaco_da_sala(org.id, conversa.id, Some("!espaco:teste"))
        .expect("põe no espaço");
    assert_eq!(posta.espaco.as_deref(), Some("!espaco:teste"));
    assert_eq!(posta.atualizada_em, 2_000);

    cenario.relogio.definir(3_000);
    let de_novo = store
        .matrix()
        .definir_espaco_da_sala(org.id, conversa.id, Some("!espaco:teste"))
        .expect("põe de novo");
    assert_eq!(de_novo.espaco.as_deref(), Some("!espaco:teste"));
    assert_eq!(de_novo.atualizada_em, 2_000, "o mesmo espaço não reescreve");

    // Mudar de espaço é admitido (é o que a transferência de departamento faz).
    let movida = store
        .matrix()
        .definir_espaco_da_sala(org.id, conversa.id, Some("!outro-espaco:teste"))
        .expect("move de espaço");
    assert_eq!(movida.espaco.as_deref(), Some("!outro-espaco:teste"));
    assert_eq!(movida.atualizada_em, 3_000);

    // `None` tira a sala do espaço.
    let sem_espaco = store
        .matrix()
        .definir_espaco_da_sala(org.id, conversa.id, None)
        .expect("tira do espaço");
    assert_eq!(sem_espaco.espaco, None);

    // Sala de outra org (ou inexistente) não é encontrada — o isolamento vale aqui também.
    let erro = store
        .matrix()
        .definir_espaco_da_sala(outra_org.id, conversa.id, Some("!espaco:teste"))
        .expect_err("sala de outra org");
    assert!(erro.eh_nao_encontrado(), "erro inesperado: {erro:?}");
    assert_eq!(
        store
            .matrix()
            .sala(org.id, conversa.id)
            .expect("sala")
            .expect("existe")
            .espaco,
        None,
        "a tentativa de outra org não pode mexer na sala"
    );
}

#[test]
fn ligar_e_desligar_sao_idempotentes_e_o_corte_do_backfill_segue_a_ultima_ligacao() {
    let cenario = Cenario::novo("org");
    let store = cenario.store();
    let org = criar_org(store, "org");

    let inicial = store.matrix().estado(org.id).expect("estado inicial");
    assert!(!inicial.ligado);
    assert_eq!(inicial.ligado_em, None);
    assert_eq!(inicial.atualizado_em, 0);

    cenario.relogio.definir(1_000);
    let ligada = store.matrix().ligar(org.id).expect("liga");
    assert!(ligada.ligado);
    assert_eq!(ligada.ligado_em, Some(1_000));
    assert_eq!(store.matrix().orgs_ligadas().expect("ligadas").len(), 1);

    // Repetir não falha nem move o corte do backfill.
    cenario.relogio.definir(2_000);
    let repetida = store.matrix().ligar(org.id).expect("liga de novo");
    assert_eq!(repetida, ligada, "a segunda ligação é um no-op");

    cenario.relogio.definir(3_000);
    let desligada = store.matrix().desligar(org.id).expect("desliga");
    assert!(!desligada.ligado);
    assert_eq!(desligada.ligado_em, Some(1_000), "o corte não é apagado");
    assert_eq!(desligada.atualizado_em, 3_000);

    // Repetir o desligar também é no-op.
    cenario.relogio.definir(4_000);
    let desligada_de_novo = store.matrix().desligar(org.id).expect("desliga de novo");
    assert_eq!(desligada_de_novo, desligada);
    assert!(
        store.matrix().orgs_ligadas().expect("ligadas").is_empty(),
        "org desligada não aparece para o laço"
    );

    // Religar move o corte para o instante da nova ligação.
    cenario.relogio.definir(5_000);
    let religada = store.matrix().ligar(org.id).expect("religa");
    assert!(religada.ligado);
    assert_eq!(religada.ligado_em, Some(5_000));
}

#[test]
fn cursores_so_avancam() {
    let cenario = Cenario::novo("cursores");
    let store = cenario.store();
    let org = criar_org(store, "org");

    let zero = store.matrix().ler_cursores(org.id).expect("cursores");
    assert_eq!(zero.mensagens, 0);
    assert_eq!(zero.versao, 0);

    let andou = store.matrix().avancar_mensagens(org.id, 10).expect("10");
    assert_eq!(andou.mensagens, 10);
    assert_eq!(andou.versao, 1);

    let voltou = store.matrix().avancar_mensagens(org.id, 5).expect("5");
    assert_eq!(voltou.mensagens, 10, "o cursor não volta");
    assert_eq!(voltou.versao, 1, "nem a versão sobe à toa");

    let mesmo = store
        .matrix()
        .avancar_mensagens(org.id, 10)
        .expect("10 de novo");
    assert_eq!(mesmo, voltou, "repetir o mesmo lote é no-op");

    let frente = store.matrix().avancar_mensagens(org.id, 12).expect("12");
    assert_eq!(frente.mensagens, 12);
    assert_eq!(frente.versao, 2);

    let conversas = store
        .matrix()
        .avancar_conversas(org.id, 500, "conv-b")
        .expect("conversas");
    assert_eq!(conversas.conversas.atualizado_em, 500);
    assert_eq!(conversas.conversas.id, "conv-b");

    // Mesmo instante, id menor: não anda.
    let atras = store
        .matrix()
        .avancar_conversas(org.id, 500, "conv-a")
        .expect("conversas atrás");
    assert_eq!(atras.conversas.id, "conv-b");
    assert_eq!(atras.conversas.atualizado_em, 500);

    // Instante maior anda, mesmo com id menor.
    let avancou = store
        .matrix()
        .avancar_conversas(org.id, 501, "conv-a")
        .expect("conversas à frente");
    assert_eq!(avancou.conversas.atualizado_em, 501);
    assert_eq!(avancou.conversas.id, "conv-a");
}

#[test]
fn falha_da_mesma_mensagem_atualiza_em_vez_de_duplicar() {
    let cenario = Cenario::novo("falhas");
    let store = cenario.store();
    let org = criar_org(store, "org");
    let mensagem = MessageId::generate();

    cenario.relogio.definir(1_000);
    let primeira = store
        .matrix()
        .registrar_falha(org.id, mensagem, "M_UNKNOWN", 1)
        .expect("primeira falha");
    assert_eq!(primeira.motivo, "M_UNKNOWN");
    assert_eq!(primeira.tentativas, 1);
    assert_eq!(primeira.criada_em, 1_000);

    cenario.relogio.definir(2_000);
    let segunda = store
        .matrix()
        .registrar_falha(org.id, mensagem, "timeout", 3)
        .expect("segunda falha");
    assert_eq!(segunda.id, primeira.id, "é a mesma linha");
    assert_eq!(segunda.motivo, "timeout");
    assert_eq!(segunda.tentativas, 3);
    assert_eq!(segunda.criada_em, 1_000, "a criação fica");
    assert_eq!(segunda.atualizada_em, 2_000);
    assert_eq!(store.matrix().contar_falhas(org.id).expect("conta"), 1);

    let listadas = store.matrix().listar_falhas(org.id, 50).expect("lista");
    assert_eq!(listadas.len(), 1);

    assert!(
        store
            .matrix()
            .remover_falha(org.id, mensagem)
            .expect("remove")
    );
    assert!(
        !store
            .matrix()
            .remover_falha(org.id, mensagem)
            .expect("remove de novo")
    );
    assert_eq!(store.matrix().contar_falhas(org.id).expect("conta"), 0);

    // Motivo vazio ou longo demais é recusado antes de chegar ao banco.
    let vazio = store
        .matrix()
        .registrar_falha(org.id, mensagem, "  ", 1)
        .expect_err("motivo vazio");
    assert!(matches!(
        vazio,
        ErroStore::Dominio(DomainError::Validacao { .. })
    ));
    let longo = store
        .matrix()
        .registrar_falha(org.id, mensagem, &"x".repeat(65), 1)
        .expect_err("motivo longo");
    assert!(matches!(
        longo,
        ErroStore::Dominio(DomainError::Validacao { .. })
    ));
}

#[test]
fn garantir_espaco_e_idempotente_por_departamento_e_recusa_espaco_de_outro() {
    let cenario = Cenario::novo("espacos");
    let store = cenario.store();
    let org = criar_org(store, "org");
    let comercial = criar_departamento(store, &org, "Comercial");
    let suporte = criar_departamento(store, &org, "Suporte");

    let primeiro = store
        .matrix()
        .garantir_espaco(
            org.id,
            comercial.id,
            "!espaco:teste",
            "#espaco:teste",
            OrigemEspaco::Automatico,
        )
        .expect("primeiro");
    let segundo = store
        .matrix()
        .garantir_espaco(
            org.id,
            comercial.id,
            "!espaco:teste",
            "#espaco:teste",
            OrigemEspaco::Ligado,
        )
        .expect("segundo");
    assert_eq!(
        primeiro, segundo,
        "a segunda chamada devolve o mesmo espaço"
    );
    assert_eq!(segundo.origem, OrigemEspaco::Automatico, "a primeira ganha");
    assert_eq!(
        store.matrix().listar_espacos(org.id).expect("lista").len(),
        1
    );

    let erro = store
        .matrix()
        .garantir_espaco(
            org.id,
            suporte.id,
            "!espaco:teste",
            "#outro-alias:teste",
            OrigemEspaco::Ligado,
        )
        .expect_err("espaço de outro departamento");
    assert!(
        matches!(erro, ErroStore::Dominio(DomainError::Conflito { .. })),
        "erro inesperado: {erro:?}"
    );

    let outro = store
        .matrix()
        .garantir_espaco(
            org.id,
            suporte.id,
            "!espaco-suporte:teste",
            "#espaco-suporte:teste",
            OrigemEspaco::Ligado,
        )
        .expect("espaço do suporte");
    assert_eq!(outro.departamento, suporte.id);
    assert_eq!(
        store.matrix().listar_espacos(org.id).expect("lista").len(),
        2
    );
}

#[test]
fn nenhuma_coluna_das_tabelas_do_modulo_guarda_texto_de_cliente_telefone_ou_token() {
    let cenario = Cenario::novo("colunas");
    let store = cenario.store();

    // Revisão das colunas: só ids, estados, cursores, carimbos e o código curto da falha.
    let esperado: [(&str, &[&str]); 5] = [
        (
            "matrix_orgs",
            &["org_id", "enabled", "enabled_at", "updated_at"],
        ),
        (
            "matrix_rooms",
            &[
                "conversation_id",
                "org_id",
                "room_id",
                "space_room_id",
                "last_status",
                // Única coluna do módulo com dado de cliente, e de propósito: é o nome de exibição
                // já enviado à sala (MX-33), que o laço de estado compara para não renomear de novo.
                "client_name_espelhado",
                "state",
                "created_at",
                "updated_at",
            ],
        ),
        (
            "matrix_spaces",
            &[
                "id",
                "org_id",
                "department_id",
                "space_room_id",
                "alias",
                "origin",
                "created_at",
                "updated_at",
            ],
        ),
        (
            "matrix_estado",
            &[
                "org_id",
                "messages_cursor",
                "messages_cursor_id",
                "conversations_cursor_at",
                "conversations_cursor_id",
                "version",
                "updated_at",
            ],
        ),
        (
            "matrix_falhas",
            &[
                "id",
                "org_id",
                "message_id",
                "motivo",
                "tentativas",
                "created_at",
                "updated_at",
            ],
        ),
    ];

    for (tabela, colunas_esperadas) in esperado {
        let colunas = store.colunas(tabela).expect("lê colunas");
        let mut colunas_ordenadas = colunas.clone();
        colunas_ordenadas.sort();
        let mut esperadas_ordenadas = colunas_esperadas.to_vec();
        esperadas_ordenadas.sort();
        assert_eq!(
            colunas_ordenadas, esperadas_ordenadas,
            "colunas de {tabela} mudaram: fora `client_name_espelhado` (MX-33), nenhuma coluna do \
             módulo guarda texto de cliente"
        );
        assert!(
            colunas.iter().any(|coluna| coluna == "org_id"),
            "tabela {tabela} sem org_id"
        );
    }
}

#[test]
fn atraso_das_mensagens_conta_so_o_que_falta_espelhar() {
    let cenario = Cenario::novo("atraso");
    let store = cenario.store();
    let org = criar_org(store, "org");
    let agente = criar_agente(store, &org, "agente");
    let conversa = criar_conversa(store, &org, &agente);
    let outra_org = criar_org(store, "outra-org");
    let outra_agente = criar_agente(store, &outra_org, "agente");
    let outra_conversa = criar_conversa(store, &outra_org, &outra_agente);

    assert_eq!(
        store
            .matrix()
            .atraso_das_mensagens(org.id, 0)
            .expect("vazio"),
        poph_store::AtrasoMatrix::default(),
        "sem mensagem, o atraso é zero e não há a mais antiga"
    );

    cenario.relogio.definir(5_000);
    let primeira = gravar_mensagem(store, &org, &conversa, "primeira");
    cenario.relogio.definir(6_000);
    gravar_mensagem(store, &org, &conversa, "segunda");
    // Mensagem de ferramenta: o espelho nunca a envia, então ela não conta como atraso.
    store
        .mensagens()
        .gravar(
            org.id,
            NovaMensagem {
                conversation_id: conversa.id,
                direcao: Direcao::Saida,
                papel: Papel::Ferramenta,
                texto: "chamada de ferramenta".to_owned(),
                anexos_json: "[]".to_owned(),
                id_externo: None,
                turn_id: None,
                origem: Origem::Agente,
                autor_nome: None,
            },
        )
        .expect("mensagem de ferramenta");
    // Mensagem da outra org: não entra no atraso desta.
    gravar_mensagem(store, &outra_org, &outra_conversa, "da outra org");

    let atraso = store
        .matrix()
        .atraso_das_mensagens(org.id, 0)
        .expect("atraso da org");
    assert_eq!(atraso.pendentes, 2, "só as duas mensagens espelháveis");
    assert_eq!(atraso.mais_antiga_em, Some(5_000));

    // Depois de espelhar a primeira, o atraso cai e a mais antiga passa a ser a segunda.
    let rowid = store
        .mensagens()
        .desde(0, 10)
        .expect("lê as mensagens")
        .into_iter()
        .find(|mensagem| mensagem.id == primeira)
        .expect("primeira mensagem na leitura incremental")
        .rowid;
    let depois = store
        .matrix()
        .atraso_das_mensagens(org.id, rowid)
        .expect("atraso depois do cursor");
    assert_eq!(depois.pendentes, 1);
    assert_eq!(depois.mais_antiga_em, Some(6_000));

    // A outra org continua com o próprio atraso, isolada.
    assert_eq!(
        store
            .matrix()
            .atraso_das_mensagens(outra_org.id, 0)
            .expect("atraso da outra org")
            .pendentes,
        1
    );
}

#[test]
fn desligar_espaco_apaga_a_linha_e_esquece_as_salas_sem_tocar_no_resto() {
    let cenario = Cenario::novo("desliga-espaco");
    let store = cenario.store();
    let org = criar_org(store, "org");
    let agente = criar_agente(store, &org, "agente");
    let conversa = criar_conversa(store, &org, &agente);
    let outra_conversa = criar_conversa(store, &org, &agente);
    let comercial = criar_departamento(store, &org, "Comercial");
    let suporte = criar_departamento(store, &org, "Suporte");
    let outra_org = criar_org(store, "outra-org");
    let outra_agente = criar_agente(store, &outra_org, "agente");
    let conversa_da_outra = criar_conversa(store, &outra_org, &outra_agente);
    let departamento_da_outra = criar_departamento(store, &outra_org, "Comercial");

    // Dois espaços: um com sala dentro, outro sem — só o primeiro tem vínculo para esquecer.
    store
        .matrix()
        .garantir_espaco(
            org.id,
            comercial.id,
            "!espaco:teste",
            "#dep_comercial:teste",
            OrigemEspaco::Automatico,
        )
        .expect("espaço do comercial");
    store
        .matrix()
        .garantir_espaco(
            org.id,
            suporte.id,
            "!suporte:teste",
            "#dep_suporte:teste",
            OrigemEspaco::Ligado,
        )
        .expect("espaço do suporte");
    store
        .matrix()
        .garantir_sala(org.id, conversa.id, "!sala:teste", Some("!espaco:teste"))
        .expect("sala no espaço");
    store
        .matrix()
        .garantir_sala(
            org.id,
            outra_conversa.id,
            "!sala-2:teste",
            Some("!suporte:teste"),
        )
        .expect("outra sala no espaço do suporte");
    store
        .matrix()
        .garantir_espaco(
            outra_org.id,
            departamento_da_outra.id,
            "!espaco-da-outra:teste",
            "#dep_da_outra:teste",
            OrigemEspaco::Automatico,
        )
        .expect("espaço da outra org");
    store
        .matrix()
        .garantir_sala(
            outra_org.id,
            conversa_da_outra.id,
            "!sala-da-outra:teste",
            Some("!espaco-da-outra:teste"),
        )
        .expect("sala da outra org");

    cenario.relogio.definir(9_000);
    let desligado = store
        .matrix()
        .desligar_espaco(org.id, comercial.id)
        .expect("desliga o espaço");
    assert!(desligado.removido, "a linha do espaço saiu");
    assert_eq!(desligado.salas_esquecidas, 1, "só a sala daquele espaço");

    assert_eq!(
        store.matrix().espaco(org.id, comercial.id).expect("lê"),
        None,
        "o departamento ficou sem espaço gravado"
    );
    assert_eq!(
        store
            .matrix()
            .sala(org.id, conversa.id)
            .expect("sala")
            .expect("existe")
            .espaco,
        None,
        "a sala deixou de estar registrada no espaço"
    );
    assert_eq!(
        store
            .matrix()
            .sala(org.id, outra_conversa.id)
            .expect("sala do suporte")
            .expect("existe")
            .espaco
            .as_deref(),
        Some("!suporte:teste"),
        "o outro espaço não foi tocado"
    );
    assert!(
        store
            .matrix()
            .espaco(org.id, suporte.id)
            .expect("lê")
            .is_some(),
        "o espaço do suporte continua gravado"
    );
    assert_eq!(
        store
            .matrix()
            .sala(outra_org.id, conversa_da_outra.id)
            .expect("sala da outra org")
            .expect("existe")
            .espaco
            .as_deref(),
        Some("!espaco-da-outra:teste"),
        "a outra org não é afetada"
    );
    // As salas continuam existindo: desligar o espaço não apaga sala nem conversa.
    assert_eq!(
        store
            .matrix()
            .sala(org.id, conversa.id)
            .expect("sala")
            .expect("existe")
            .estado,
        EstadoSala::Criando,
        "a linha da sala continua lá (a da MX-08 fica em `creating` até ficar pronta)"
    );

    // Repetir é inofensivo.
    let de_novo = store
        .matrix()
        .desligar_espaco(org.id, comercial.id)
        .expect("desliga de novo");
    assert_eq!(
        de_novo,
        poph_store::EspacoDesligado::default(),
        "sem espaço gravado, nada acontece"
    );
}
