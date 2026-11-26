//! Testes de integração do armazenamento: migração, isolamento entre orgs, conflito de versão,
//! idempotência e concorrência.
//!
//! Todos usam banco de verdade em pasta temporária própria (sem mock).

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Barrier};
use std::thread;

use poph_store::{
    AgentChange, Agente, CategoryChange, Conversa, ConversationStatus, DepartmentChange, Direction,
    ExecutionStart, ExecutionState, FlowChange, FlowFilter, FlowTransition, NewAgent,
    NewAuditEntry, NewCategory, NewConversation, NewDepartment, NewFlow, NewInboxItem, NewMessage,
    NewOrg, NewOutboxItem, NewRecord, NewRecordEvent, Org, OrgChange, OutboxState, Page, Received,
    RevisionPublication, Role, Status, Store, StoreError, TransitionEffect, TransitionExtras,
    file_version,
};
use poph_types::{
    DomainError, EventActor, ExecutionStatus, InboxId, IntegrationId, MessageId, OrgId,
    RecordEventKind,
};

/// Banco de verdade numa pasta temporária, apagada no fim.
struct Cenario {
    pasta: PathBuf,
    store: Option<Arc<Store>>,
}

impl Cenario {
    fn novo(rotulo: &str) -> Self {
        static CONTADOR: AtomicUsize = AtomicUsize::new(0);
        let n = CONTADOR.fetch_add(1, Ordering::Relaxed);
        let pasta =
            std::env::temp_dir().join(format!("poph-store-{rotulo}-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&pasta);
        fs::create_dir_all(&pasta).expect("cria a pasta do teste");
        let store =
            Store::open_at(&pasta.join("harness.db"), &pasta.join("rag.db")).expect("abre o banco");
        Self {
            pasta,
            store: Some(Arc::new(store)),
        }
    }

    fn store(&self) -> &Store {
        self.store.as_deref().expect("store viva")
    }

    fn compartilhado(&self) -> Arc<Store> {
        Arc::clone(self.store.as_ref().expect("store viva"))
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

fn criar_agente(store: &Store, org: &Org, slug: &str) -> Agente {
    let novo = NewAgent {
        name: format!("Agente {slug}"),
        slug: slug.to_owned(),
        ..NewAgent::default()
    };
    store.agents().criar(org.id, novo).expect("cria o agente")
}

fn criar_conversa(store: &Store, org: &Org, agente: &Agente) -> Conversa {
    store
        .conversas()
        .criar(
            org.id,
            NewConversation {
                agent_id: agente.id,
                integration_id: None,
                canal: "cli".to_owned(),
                peer_externo: Some("teste".to_owned()),
                target_department: None,
            },
        )
        .expect("cria a conversa")
}

fn gravar_mensagem(store: &Store, org: &Org, conversa: &Conversa, texto: &str) -> MessageId {
    store
        .messages()
        .gravar(
            org.id,
            NewMessage {
                conversation_id: conversa.id,
                direction: Direction::Inbound,
                role: Role::User,
                texto: texto.to_owned(),
                attachments_json: "[]".to_owned(),
                id_externo: None,
                turn_id: None,
                origin: poph_store::Origin::Agente,
                author_name: None,
            },
        )
        .expect("grava a mensagem")
        .id
}

#[test]
fn migracao_cria_o_esquema_do_prd_e_e_idempotente() {
    let cenario = Cenario::novo("migracao");
    let store = cenario.store();

    let tabelas = store.tables().expect("lista tabelas");
    for esperada in [
        "orgs",
        "admin_tokens",
        "product_categories",
        "products",
        "departments",
        "agents",
        "agent_product_categories",
        "agent_instructions",
        "knowledge_groups",
        "knowledge_items",
        "agent_knowledge_groups",
        "file_library",
        "integrations",
        "conversations",
        "messages",
        "attachments",
        "carts",
        "cart_items",
        "inbox",
        "outbox",
        "turns",
        "audit_log",
        "matrix_orgs",
        "matrix_rooms",
        "matrix_spaces",
        "matrix_estado",
        "matrix_falhas",
        "schema_migrations",
    ] {
        assert!(
            tabelas.iter().any(|tabela| tabela == esperada),
            "tabela {esperada} não criada; existem: {tabelas:?}"
        );
    }

    let caminho = store.harness_path().to_path_buf();
    assert_eq!(
        file_version(&caminho).expect("lê versão"),
        Some(poph_store::harness_target_version())
    );
    assert!(store.broken_foreign_keys().expect("fk check").is_empty());

    // Rodar de novo não aplica nada nem muda a versão.
    let relatorios = store.migrate().expect("migra de novo");
    assert!(relatorios.already_up_to_date(), "{relatorios:?}");
    assert_eq!(
        file_version(&caminho).expect("lê versão"),
        Some(poph_store::harness_target_version())
    );

    // `rag.db` existe e está na versão 0 (sem tabelas de trechos até a HR-16).
    let caminho_rag = store.rag_path().to_path_buf();
    assert_eq!(file_version(&caminho_rag).expect("lê versão"), Some(0));
}

#[test]
fn toda_tabela_de_negocio_tem_org_id() {
    let cenario = Cenario::novo("org-id");
    let store = cenario.store();
    // `orgs` é a própria org; `schema_migrations` é infraestrutura.
    let sem_org = ["orgs", "schema_migrations"];
    let mut verificadas = 0;
    for tabela in store.tables().expect("lista tabelas") {
        if sem_org.contains(&tabela.as_str()) {
            continue;
        }
        let colunas = store.columns(&tabela).expect("lê colunas");
        assert!(
            colunas.iter().any(|coluna| coluna == "org_id"),
            "tabela {tabela} não tem org_id; colunas: {colunas:?}"
        );
        verificadas += 1;
    }
    assert!(
        verificadas >= 20,
        "poucas tabelas verificadas: {verificadas}"
    );
}

#[test]
fn nenhum_repositorio_le_registro_de_outra_org() {
    let cenario = Cenario::novo("isolamento");
    let store = cenario.store();
    let org_a = criar_org(store, "org-a");
    let org_b = criar_org(store, "org-b");

    // Dados todos da org A.
    let categoria = store
        .categories()
        .criar(
            org_a.id,
            NewCategory {
                name: "Dedetização".to_owned(),
                description: String::new(),
            },
        )
        .expect("categoria");
    let departamento = store
        .departments()
        .criar(
            org_a.id,
            NewDepartment {
                name: "Comercial".to_owned(),
                description: String::new(),
            },
        )
        .expect("departamento");
    let agente = criar_agente(store, &org_a, "agente-a");
    store
        .agents()
        .definir_categorias(org_a.id, agente.id, agente.version, &[categoria.id])
        .expect("vincula categoria");
    store
        .instructions()
        .salvar(org_a.id, agente.id, 0, "{}", "# A", true)
        .expect("instruções");
    let conversa = criar_conversa(store, &org_a, &agente);
    let mensagem = gravar_mensagem(store, &org_a, &conversa, "oi");
    let turno = store
        .turns()
        .iniciar(org_a.id, conversa.id, Some(agente.id))
        .expect("turno");
    let entrada = store
        .inbox()
        .receber(
            org_a.id,
            NewInboxItem {
                integration_id: IntegrationId::generate(),
                id_externo: "wamid-1".to_owned(),
                event_json: "{}".to_owned(),
            },
        )
        .expect("entrada")
        .1;
    let saida = store
        .outbox()
        .enfileirar(
            org_a.id,
            NewOutboxItem {
                conversation_id: conversa.id,
                turn_id: turno.id,
                seq: 0,
                tipo: "text".to_owned(),
                content_json: "{}".to_owned(),
            },
        )
        .expect("saída")
        .1;
    let token = store
        .tokens()
        .criar(Some(org_a.id), "token A", "hash-a")
        .expect("token");
    store
        .audit()
        .registrar(
            Some(org_a.id),
            NewAuditEntry {
                ator_tipo: "system".to_owned(),
                actor_id: None,
                acao: "create".to_owned(),
                entity: "agent".to_owned(),
                entity_id: Some(agente.id.to_string()),
                detail_json: "{}".to_owned(),
            },
        )
        .expect("auditoria");

    // Tudo consultado com a org B dá "não encontrado" ou lista vazia.
    let nao_encontrado = |resultado: Result<(), poph_store::StoreError>| match resultado {
        Err(poph_store::StoreError::Domain(DomainError::NotFound { .. })) => true,
        outro => panic!("esperado não encontrado, veio {outro:?}"),
    };

    nao_encontrado(store.categories().obter(org_b.id, categoria.id).map(|_| ()));
    assert!(
        store
            .categories()
            .listar(org_b.id, Page::default())
            .expect("lista")
            .is_empty()
    );
    nao_encontrado(
        store
            .departments()
            .obter(org_b.id, departamento.id)
            .map(|_| ()),
    );
    assert!(
        store
            .departments()
            .listar(org_b.id, Page::default())
            .expect("lista")
            .is_empty()
    );
    nao_encontrado(store.agents().get(org_b.id, agente.id).map(|_| ()));
    assert!(
        store
            .agents()
            .listar(org_b.id, Page::default())
            .expect("lista")
            .is_empty()
    );
    assert!(
        store
            .agents()
            .categorias(org_b.id, agente.id)
            .expect("categorias")
            .is_empty()
    );
    nao_encontrado(store.instructions().obter(org_b.id, agente.id).map(|_| ()));
    nao_encontrado(store.conversas().get(org_b.id, conversa.id).map(|_| ()));
    assert!(
        store
            .conversas()
            .listar(org_b.id, None, Page::default())
            .expect("lista")
            .is_empty()
    );
    nao_encontrado(store.conversas().agente(org_b.id, conversa.id).map(|_| ()));
    assert!(
        store
            .messages()
            .listar(org_b.id, conversa.id, Page::default())
            .expect("lista")
            .is_empty()
    );
    nao_encontrado(
        store
            .messages()
            .obter(org_b.id, conversa.id, mensagem)
            .map(|_| ()),
    );
    assert_eq!(
        store
            .messages()
            .contar(org_b.id, conversa.id)
            .expect("conta"),
        0
    );
    nao_encontrado(store.turns().obter(org_b.id, turno.id).map(|_| ()));
    assert!(
        store
            .turns()
            .listar(org_b.id, conversa.id, Page::default())
            .expect("lista")
            .is_empty()
    );
    nao_encontrado(store.inbox().obter(org_b.id, entrada.id).map(|_| ()));
    assert!(
        store
            .inbox()
            .listar_pendentes(org_b.id, 100)
            .expect("lista")
            .is_empty()
    );
    nao_encontrado(store.outbox().obter(org_b.id, saida.id).map(|_| ()));
    assert!(
        store
            .outbox()
            .listar_pendentes(org_b.id, 100)
            .expect("lista")
            .is_empty()
    );
    assert!(
        store
            .tokens()
            .listar(org_b.id, Page::default())
            .expect("lista")
            .is_empty()
    );
    nao_encontrado(store.tokens().obter(org_b.id, token.id).map(|_| ()));
    assert!(
        store
            .audit()
            .listar(org_b.id, Page::default())
            .expect("lista")
            .is_empty()
    );

    // E a org A continua enxergando tudo.
    assert_eq!(
        store
            .messages()
            .contar(org_a.id, conversa.id)
            .expect("conta"),
        1
    );
    assert_eq!(
        store
            .inbox()
            .listar_pendentes(org_a.id, 100)
            .expect("lista")
            .len(),
        1
    );

    // Fluxos e permissão por agente (FX-08): a org B não vê o fluxo, a permissão nem quem o tem.
    let fluxo = store
        .fluxos()
        .criar(
            org_a.id,
            NewFlow {
                slug: "agendar_visita".to_owned(),
                name: "Agendar visita".to_owned(),
                description: String::new(),
                rascunho_fonte: String::new(),
            },
        )
        .expect("fluxo");
    store
        .fluxos()
        .liberar(org_a.id, agente.id, fluxo.id)
        .expect("libera ao agente da org A");
    nao_encontrado(store.fluxos().obter(org_b.id, fluxo.id).map(|_| ()));
    assert!(
        store
            .fluxos()
            .por_slug(org_b.id, "agendar_visita")
            .expect("por slug")
            .is_none()
    );
    assert!(
        store
            .fluxos()
            .listar(org_b.id, &FlowFilter::default(), Page::default())
            .expect("lista")
            .is_empty()
    );
    assert!(
        store
            .fluxos()
            .listar_do_agente(org_b.id, agente.id)
            .expect("lista")
            .is_empty()
    );
    nao_encontrado(
        store
            .fluxos()
            .listar_agentes_do_fluxo(org_b.id, fluxo.id)
            .map(|_| ()),
    );
    nao_encontrado(store.fluxos().liberar(org_b.id, agente.id, fluxo.id));
    nao_encontrado(
        store
            .fluxos()
            .atualizar(org_b.id, fluxo.id, 1, FlowChange::default())
            .map(|_| ()),
    );
    nao_encontrado(store.fluxos().excluir(org_b.id, fluxo.id, true).map(|_| ()));
    store
        .fluxos()
        .obter(org_a.id, fluxo.id)
        .expect("a org A continua vendo o seu fluxo");

    // ── FX-09: revisões publicadas ──
    // A revisão da org A não é lida, não é listada e não é publicável pela org B.
    let versao_do_fluxo = store
        .fluxos()
        .obter(org_a.id, fluxo.id)
        .expect("fluxo da org A")
        .version;
    let publicacao = |esperada: u64| RevisionPublication {
        expected: esperada,
        fonte: "fluxo(id = \"agendar_visita\")".to_owned(),
        definicao_json: "{}".to_owned(),
        schema_versao: Some(1),
        resolucoes_json: "{}".to_owned(),
        ator_tipo: "cli".to_owned(),
        actor_id: None,
    };
    let revisao = store
        .revisions()
        .publicar_revisao(org_a.id, fluxo.id, publicacao(versao_do_fluxo))
        .expect("publica a revisão 1 na org A");
    nao_encontrado(store.revisions().obter(org_b.id, fluxo.id, 1).map(|_| ()));
    nao_encontrado(
        store
            .revisions()
            .obter_por_id(org_b.id, revisao.id)
            .map(|_| ()),
    );
    nao_encontrado(store.revisions().atual(org_b.id, fluxo.id).map(|_| ()));
    nao_encontrado(
        store
            .revisions()
            .listar(org_b.id, fluxo.id, None, 10)
            .map(|_| ()),
    );
    nao_encontrado(
        store
            .revisions()
            .publicar_revisao(org_b.id, fluxo.id, publicacao(versao_do_fluxo + 1))
            .map(|_| ()),
    );
    assert_eq!(
        store
            .revisions()
            .listar(org_a.id, fluxo.id, None, 10)
            .expect("lista da org A")
            .len(),
        1,
        "a org A continua com a sua revisão"
    );

    // ── FX-07: Fichas e eventos ──
    let ficha = store
        .fichas()
        .criar(
            org_a.id,
            NewRecord {
                conversation_id: None,
                agent_id: agente.id,
                canal: "cli".to_owned(),
                peer_externo: Some("cliente-ficticio-a".to_owned()),
            },
        )
        .expect("Ficha da org A");
    nao_encontrado(store.fichas().obter(org_b.id, ficha.id).map(|_| ()));
    assert!(
        store
            .fichas()
            .por_conversa(org_b.id, conversa.id)
            .expect("por conversa")
            .is_none()
    );
    assert!(
        store
            .fichas()
            .listar(org_b.id, &Default::default(), Page::default())
            .expect("lista")
            .is_empty()
    );
    nao_encontrado(
        store
            .fichas()
            .eventos(org_b.id, ficha.id, None, 10)
            .map(|_| ()),
    );
    nao_encontrado(
        store
            .fichas()
            .registrar_evento(
                org_b.id,
                ficha.id,
                NewRecordEvent {
                    execucao_id: None,
                    tipo: RecordEventKind::RecordOpened,
                    ator_tipo: EventActor::System,
                    ator_ref: None,
                    payload_json: "{}".to_owned(),
                    chave_idempotencia: None,
                },
            )
            .map(|_| ()),
    );
    nao_encontrado(
        store
            .fichas()
            .fechar_sem_conversa(org_b.id, ficha.id, ficha.version, "invasão")
            .map(|_| ()),
    );

    // ── FX-20: execução, etapas, eventos e efeitos ──
    let estado_inicial = || ExecutionState {
        status: ExecutionStatus::Active,
        no_atual: "coleta".to_owned(),
        dados_json: "{}".to_owned(),
        acting_actors: Vec::new(),
        result: None,
        reason: None,
    };
    let abertura = || ExecutionStart {
        ficha: ficha.id,
        fluxo: fluxo.id,
        revision: revisao.id,
        actor: EventActor::Attendant,
        ator_ref: Some("token-ficticio"),
        jev_decision: None,
        initial_state: estado_inicial(),
        inbound_effects: vec![TransitionEffect {
            key: "entrada/0".to_owned(),
            tipo: "mensagem_ao_cliente".to_owned(),
            payload_json: "{\"texto\":\"olá\"}".to_owned(),
        }],
        chave_idempotencia: None,
    };
    let aberta = store
        .executions()
        .iniciar_execucao(org_a.id, abertura())
        .expect("abre a execução na org A");
    let execucao = aberta.execution.id;
    let efeito = aberta.effects[0].id;

    nao_encontrado(store.executions().obter(org_b.id, execucao).map(|_| ()));
    assert!(
        store
            .executions()
            .ativa_da_ficha(org_b.id, ficha.id)
            .expect("ativa")
            .is_none()
    );
    assert!(
        store
            .executions()
            .da_ficha(org_b.id, ficha.id, 10)
            .expect("da ficha")
            .is_empty()
    );
    nao_encontrado(store.executions().etapas(org_b.id, execucao).map(|_| ()));
    nao_encontrado(store.executions().efeitos(org_b.id, execucao).map(|_| ()));
    assert!(
        store
            .executions()
            .ativas_por_revisao(org_b.id, fluxo.id)
            .expect("ativas por revisão")
            .is_empty()
    );
    nao_encontrado(
        store
            .executions()
            .iniciar_execucao(org_b.id, abertura())
            .map(|_| ()),
    );
    nao_encontrado(
        store
            .executions()
            .aplicar_transicao(
                org_b.id,
                execucao,
                aberta.execution.version,
                &FlowTransition {
                    new_state: estado_inicial(),
                    events: Vec::new(),
                    effects: Vec::new(),
                    closes_service: false,
                    repeated: false,
                    rule_error: false,
                },
                "chave-invasora",
                TransitionExtras::default(),
            )
            .map(|_| ()),
    );
    assert!(
        store
            .executions()
            .abertura_ja_feita(org_b.id, ficha.id, fluxo.id, "chave-invasora")
            .expect("abertura já feita")
            .is_none()
    );
    assert!(
        !store
            .executions()
            .transicao_ja_aplicada(org_b.id, execucao, "chave-invasora")
            .expect("transição já aplicada")
    );

    // Efeitos: a org B não lê a fila, não a move e não confirma entrega da org A.
    nao_encontrado(store.effects().obter(org_b.id, efeito).map(|_| ()));
    assert!(
        store
            .effects()
            .listar_pendentes(org_b.id, 100)
            .expect("pendentes")
            .is_empty()
    );
    assert!(
        store.effects().iniciar_envio(org_b.id, efeito).is_err(),
        "a org B não pode começar a entrega de um efeito da org A"
    );
    store
        .effects()
        .confirmar_envio(org_b.id, efeito)
        .expect("confirmar sem linha não é erro de banco");
    store
        .effects()
        .falhar_envio(org_b.id, efeito, "invasão", true, None)
        .expect("falhar sem linha não é erro de banco");
    store
        .effects()
        .adiar(org_b.id, efeito, 1_700_000_000_000)
        .expect("adiar sem linha não é erro de banco");
    let intacto = store
        .effects()
        .obter(org_a.id, efeito)
        .expect("o efeito da org A continua lá");
    assert_eq!(intacto.tentativas, 0);
    assert_eq!(intacto.ultimo_erro, None);
    assert_eq!(intacto.proxima_tentativa, None);
    assert_eq!(intacto.enviado_em, None);

    // A org A continua enxergando tudo o que é dela.
    assert_eq!(
        store
            .executions()
            .da_ficha(org_a.id, ficha.id, 10)
            .expect("da ficha na org A")
            .len(),
        1
    );
    assert_eq!(
        store
            .fichas()
            .eventos(org_a.id, ficha.id, None, 50)
            .expect("eventos da org A")
            .len(),
        2,
        "ficha_aberta + fluxo_iniciado"
    );
    assert_eq!(
        store
            .effects()
            .listar_pendentes(org_a.id, 100)
            .expect("pendentes da org A")
            .len(),
        1
    );
}

#[test]
fn conflito_de_versao_devolve_erro_tipado_em_todas_as_entidades_editaveis() {
    let cenario = Cenario::novo("conflito");
    let store = cenario.store();
    let org = criar_org(store, "org-conflito");
    let agente = criar_agente(store, &org, "agente-conflito");
    let categoria = store
        .categories()
        .criar(
            org.id,
            NewCategory {
                name: "Categoria".to_owned(),
                description: String::new(),
            },
        )
        .expect("categoria");
    let departamento = store
        .departments()
        .criar(
            org.id,
            NewDepartment {
                name: "Departamento".to_owned(),
                description: String::new(),
            },
        )
        .expect("departamento");
    let conversa = criar_conversa(store, &org, &agente);
    store
        .instructions()
        .salvar(org.id, agente.id, 0, "{}", "# v1", true)
        .expect("instruções");

    let confere_conflito = |resultado: Result<(), poph_store::StoreError>| match resultado {
        Err(erro) if erro.is_version_conflict() => {}
        outro => panic!("esperado conflito de versão, veio {outro:?}"),
    };

    confere_conflito(
        store
            .orgs()
            .atualizar(
                org.id,
                org.version + 5,
                OrgChange {
                    name: Some("Outro nome".to_owned()),
                    ..OrgChange::default()
                },
            )
            .map(|_| ()),
    );
    confere_conflito(
        store
            .agents()
            .atualizar(
                org.id,
                agente.id,
                agente.version + 5,
                AgentChange {
                    modelo: Some("outro".to_owned()),
                    ..AgentChange::default()
                },
            )
            .map(|_| ()),
    );
    confere_conflito(
        store
            .categories()
            .atualizar(
                org.id,
                categoria.id,
                categoria.version + 5,
                CategoryChange {
                    name: Some("Outra".to_owned()),
                    ..CategoryChange::default()
                },
            )
            .map(|_| ()),
    );
    confere_conflito(
        store
            .departments()
            .atualizar(
                org.id,
                departamento.id,
                departamento.version + 5,
                DepartmentChange {
                    name: Some("Outro".to_owned()),
                    ..DepartmentChange::default()
                },
            )
            .map(|_| ()),
    );
    confere_conflito(
        store
            .conversas()
            .mudar_status(
                org.id,
                conversa.id,
                conversa.version + 5,
                ConversationStatus::Closed,
                None,
            )
            .map(|_| ()),
    );
    confere_conflito(
        store
            .instructions()
            .salvar(org.id, agente.id, 9, "{}", "# v2", true)
            .map(|_| ()),
    );

    // Com a versão certa, a escrita anda e incrementa.
    let atualizado = store
        .agents()
        .atualizar(
            org.id,
            agente.id,
            agente.version,
            AgentChange {
                modelo: Some("gemini-3.8-flash-tiered".to_owned()),
                status: Some(Status::Inactive),
                ..AgentChange::default()
            },
        )
        .expect("atualiza");
    assert_eq!(atualizado.version, agente.version + 1);
    assert_eq!(atualizado.status, Status::Inactive);
}

#[test]
fn mensagem_repetida_pelo_id_externo_nao_duplica() {
    let cenario = Cenario::novo("mensagem-idempotente");
    let store = cenario.store();
    let org = criar_org(store, "org-msg");
    let agente = criar_agente(store, &org, "agente-msg");
    let conversa = criar_conversa(store, &org, &agente);

    let nova = || NewMessage {
        conversation_id: conversa.id,
        direction: Direction::Inbound,
        role: Role::User,
        origin: poph_store::Origin::Agente,
        author_name: None,
        texto: "bom dia".to_owned(),
        attachments_json: "[]".to_owned(),
        id_externo: Some("wamid-42".to_owned()),
        turn_id: None,
    };
    let primeira = store.messages().gravar(org.id, nova()).expect("grava");
    let segunda = store.messages().gravar(org.id, nova()).expect("regrava");
    assert_eq!(
        primeira.id, segunda.id,
        "a segunda gravação devolve a mesma"
    );
    assert_eq!(
        store.messages().contar(org.id, conversa.id).expect("conta"),
        1
    );
}

#[test]
fn entrada_e_saida_sao_idempotentes_pela_chave_do_prd() {
    let cenario = Cenario::novo("caixas");
    let store = cenario.store();
    let org = criar_org(store, "org-caixas");
    let agente = criar_agente(store, &org, "agente-caixas");
    let conversa = criar_conversa(store, &org, &agente);
    let integracao = IntegrationId::generate();

    let entrada = || NewInboxItem {
        integration_id: integracao,
        id_externo: "wamid-7".to_owned(),
        event_json: "{\"texto\":\"oi\"}".to_owned(),
    };
    let (primeiro, item) = store.inbox().receber(org.id, entrada()).expect("entrada");
    assert_eq!(primeiro, Received::New);
    let (segundo, repetido) = store.inbox().receber(org.id, entrada()).expect("reentrega");
    assert_eq!(segundo, Received::Duplicate);
    assert_eq!(repetido.id, item.id);
    assert_eq!(
        store
            .inbox()
            .listar_pendentes(org.id, 100)
            .expect("lista")
            .len(),
        1
    );

    store
        .inbox()
        .marcar_processado(org.id, item.id, None)
        .expect("processa");
    assert!(
        store
            .inbox()
            .listar_pendentes(org.id, 100)
            .expect("lista")
            .is_empty()
    );

    let turno = store
        .turns()
        .iniciar(org.id, conversa.id, Some(agente.id))
        .expect("turno");
    let saida = || NewOutboxItem {
        conversation_id: conversa.id,
        turn_id: turno.id,
        seq: 0,
        tipo: "text".to_owned(),
        content_json: "{\"texto\":\"olá\"}".to_owned(),
    };
    let (novo, item_saida) = store.outbox().enfileirar(org.id, saida()).expect("saída");
    assert!(novo);
    let (novo_de_novo, repetido) = store
        .outbox()
        .enfileirar(org.id, saida())
        .expect("saída repetida");
    assert!(!novo_de_novo);
    assert_eq!(repetido.id, item_saida.id);

    store
        .outbox()
        .mudar_estado(org.id, item_saida.id, OutboxState::Sending, None)
        .expect("enviando");
    // `sending` continua na fila: é o reenvio da retomada (D11).
    assert_eq!(
        store
            .outbox()
            .listar_pendentes(org.id, 100)
            .expect("lista")
            .len(),
        1
    );
    store
        .outbox()
        .mudar_estado(org.id, item_saida.id, OutboxState::Sent, None)
        .expect("enviado");
    assert!(
        store
            .outbox()
            .listar_pendentes(org.id, 100)
            .expect("lista")
            .is_empty()
    );
}

#[test]
fn turno_guarda_o_porque_e_a_auditoria_guarda_quem() {
    let cenario = Cenario::novo("turno");
    let store = cenario.store();
    let org = criar_org(store, "org-turno");
    let agente = criar_agente(store, &org, "agente-turno");
    let conversa = criar_conversa(store, &org, &agente);

    let turno = store
        .turns()
        .iniciar(org.id, conversa.id, Some(agente.id))
        .expect("turno");
    let fim = poph_store::TurnFinalization {
        estado: Some(poph_store::TurnState::Completed),
        times_json: Some("{\"llm_ms\":6500}".to_owned()),
        input_tokens: Some(1200),
        output_tokens: Some(80),
        ..poph_store::TurnFinalization::default()
    };
    let fechado = store
        .turns()
        .finalizar(org.id, turno.id, fim)
        .expect("fecha");
    assert_eq!(fechado.estado, poph_store::TurnState::Completed);
    assert!(fechado.finished_at.is_some());

    let registro = store
        .audit()
        .registrar(
            Some(org.id),
            NewAuditEntry {
                ator_tipo: "admin_token".to_owned(),
                actor_id: Some("token-1".to_owned()),
                acao: "update".to_owned(),
                entity: "agent".to_owned(),
                entity_id: Some(agente.id.to_string()),
                detail_json: "{\"campo\":\"modelo\"}".to_owned(),
            },
        )
        .expect("auditoria");
    assert_eq!(registro.acao, "update");
    assert_eq!(
        store
            .audit()
            .listar(org.id, Page::default())
            .expect("lista")
            .len(),
        1
    );
}

#[test]
fn dez_mil_escritas_concorrentes_sem_busy_e_sem_perda() {
    let cenario = Cenario::novo("concorrencia");
    let store = cenario.store();
    let org = criar_org(store, "org-concorrencia");
    let agente = criar_agente(store, &org, "agente-concorrencia");
    let conversa = criar_conversa(store, &org, &agente);

    const THREADS: usize = 10;
    const POR_THREAD: usize = 1000;
    let barreira = Arc::new(Barrier::new(THREADS));
    let org_id: OrgId = org.id;
    let conversa_id = conversa.id;
    let mut tarefas = Vec::with_capacity(THREADS);
    for thread_id in 0..THREADS {
        let store = cenario.compartilhado();
        let barreira = Arc::clone(&barreira);
        tarefas.push(thread::spawn(move || {
            barreira.wait();
            for indice in 0..POR_THREAD {
                store
                    .messages()
                    .gravar(
                        org_id,
                        NewMessage {
                            conversation_id: conversa_id,
                            direction: Direction::Inbound,
                            role: Role::User,
                            texto: format!("mensagem {thread_id}-{indice}"),
                            attachments_json: "[]".to_owned(),
                            id_externo: None,
                            turn_id: None,
                            origin: poph_store::Origin::Agente,
                            author_name: None,
                        },
                    )
                    .expect("gravação concorrente não pode falhar");
            }
        }));
    }
    for tarefa in tarefas {
        tarefa.join().expect("thread concluiu");
    }

    assert_eq!(
        store.messages().contar(org.id, conversa.id).expect("conta"),
        (THREADS * POR_THREAD) as u64,
        "nenhuma escrita pode ser perdida"
    );
    assert!(store.broken_foreign_keys().expect("fk check").is_empty());
}

#[test]
fn entrada_invalida_nao_derruba_o_lote() {
    let cenario = Cenario::novo("erro-no-lote");
    let store = cenario.store();
    let org = criar_org(store, "org-erro");

    // Uma org com slug repetido falha; a escrita seguinte, válida, precisa passar.
    let erro = store
        .orgs()
        .criar(NewOrg {
            name: "Repetida".to_owned(),
            slug: "org-erro".to_owned(),
            config_json: String::new(),
        })
        .expect_err("slug repetido deve falhar");
    assert!(
        matches!(
            erro,
            poph_store::StoreError::Domain(DomainError::Validation { .. })
        ),
        "veio {erro:?}"
    );

    let outra = criar_org(store, "org-erro-2");
    assert_eq!(outra.status, Status::Active);
    assert_eq!(
        store.orgs().listar(Page::default()).expect("lista").len(),
        2
    );

    // Ids que não existem dão "não encontrado" com a entidade no texto.
    let inexistente = store
        .agents()
        .get(org.id, poph_types::AgentId::generate())
        .expect_err("agente inexistente");
    assert!(inexistente.is_not_found(), "veio {inexistente:?}");
}

#[test]
fn tokens_so_aparecem_uma_vez_e_revogar_tira_da_autenticacao() {
    let cenario = Cenario::novo("tokens");
    let store = cenario.store();
    let org = criar_org(store, "org-token");
    let token = store
        .tokens()
        .criar(Some(org.id), "CI", "hash-unico")
        .expect("token");
    assert!(
        store
            .tokens()
            .por_hash("hash-unico")
            .expect("busca")
            .is_some()
    );
    store.tokens().revogar(org.id, token.id).expect("revoga");
    assert!(
        store
            .tokens()
            .por_hash("hash-unico")
            .expect("busca")
            .is_none(),
        "revogado não autentica"
    );
    assert!(
        matches!(
            store.inbox().obter(org.id, InboxId::generate()).map(|_| ()),
            Err(poph_store::StoreError::Domain(DomainError::NotFound { .. }))
        ),
        "item de entrada inexistente precisa dar não encontrado"
    );
}

#[test]
fn vinculo_entre_orgs_e_recusado_e_nao_grava_nada() {
    let cenario = Cenario::novo("vinculo-entre-orgs");
    let store = cenario.store();
    let org_a = criar_org(store, "org-a");
    let org_b = criar_org(store, "org-b");
    let agente_a = criar_agente(store, &org_a, "agente-a");
    let agente_b = criar_agente(store, &org_b, "agente-b");
    let departamento_b = store
        .departments()
        .criar(
            org_b.id,
            NewDepartment {
                name: "Departamento B".to_owned(),
                description: String::new(),
            },
        )
        .expect("departamento de B");
    let categoria_b = store
        .categories()
        .criar(
            org_b.id,
            NewCategory {
                name: "Categoria B".to_owned(),
                description: String::new(),
            },
        )
        .expect("categoria de B");

    // Departamento de outra org num agente: recusado pelo repositório, sem gravar.
    let recusado = store
        .agents()
        .atualizar(
            org_a.id,
            agente_a.id,
            agente_a.version,
            AgentChange {
                default_department: Some(departamento_b.id),
                ..AgentChange::default()
            },
        )
        .expect_err("departamento de outra org");
    assert!(recusado.is_not_found(), "veio {recusado:?}");
    assert_eq!(
        store
            .agents()
            .get(org_a.id, agente_a.id)
            .expect("agente intacto")
            .version,
        agente_a.version,
        "a versão não pode andar numa escrita recusada"
    );

    // Categoria de outra org no agente: recusado, e nenhum vínculo gravado.
    let recusado = store
        .agents()
        .definir_categorias(org_a.id, agente_a.id, agente_a.version, &[categoria_b.id])
        .expect_err("categoria de outra org");
    assert!(recusado.is_not_found(), "veio {recusado:?}");
    assert!(
        store
            .agents()
            .categorias(org_a.id, agente_a.id)
            .expect("categorias")
            .is_empty()
    );

    // Conversa da org A com agente da org B: a chave estrangeira composta recusa.
    let recusado = store
        .conversas()
        .criar(
            org_a.id,
            NewConversation {
                agent_id: agente_b.id,
                integration_id: None,
                canal: "cli".to_owned(),
                peer_externo: None,
                target_department: None,
            },
        )
        .expect_err("agente de outra org");
    assert!(
        matches!(recusado, poph_store::StoreError::Sqlite(_)),
        "veio {recusado:?}"
    );
    assert!(
        store
            .conversas()
            .listar(org_a.id, None, Page::default())
            .expect("lista")
            .is_empty(),
        "conversa recusada não pode aparecer"
    );

    // Mensagem na org A apontando para conversa da org B: recusada e nada gravado.
    let conversa_b = criar_conversa(store, &org_b, &agente_b);
    let recusado = store
        .messages()
        .gravar(
            org_a.id,
            NewMessage {
                conversation_id: conversa_b.id,
                direction: Direction::Inbound,
                role: Role::User,
                texto: "invasão".to_owned(),
                attachments_json: "[]".to_owned(),
                id_externo: None,
                turn_id: None,
                origin: poph_store::Origin::Agente,
                author_name: None,
            },
        )
        .expect_err("conversa de outra org");
    assert!(
        matches!(recusado, poph_store::StoreError::Sqlite(_)),
        "veio {recusado:?}"
    );
    assert_eq!(
        store
            .messages()
            .contar(org_b.id, conversa_b.id)
            .expect("conta"),
        0
    );

    // Turno na org A com agente da org B: recusado pelo repositório.
    let conversa_a = criar_conversa(store, &org_a, &agente_a);
    let recusado = store
        .turns()
        .iniciar(org_a.id, conversa_a.id, Some(agente_b.id))
        .expect_err("agente de outra org");
    assert!(recusado.is_not_found(), "veio {recusado:?}");
}

#[test]
fn excluir_org_leva_junto_os_dados_dela() {
    let cenario = Cenario::novo("cascata");
    let store = cenario.store();
    let org = criar_org(store, "org-cascata");
    let outro = criar_org(store, "org-fica");
    store
        .departments()
        .criar(
            org.id,
            NewDepartment {
                name: "Dep".to_owned(),
                description: String::new(),
            },
        )
        .expect("departamento");
    let categoria = store
        .categories()
        .criar(
            org.id,
            NewCategory {
                name: "Cat".to_owned(),
                description: String::new(),
            },
        )
        .expect("categoria");
    let agente = criar_agente(store, &org, "agente-cascata");
    store
        .agents()
        .definir_categorias(org.id, agente.id, agente.version, &[categoria.id])
        .expect("vínculo");
    store
        .instructions()
        .salvar(org.id, agente.id, 0, "{}", "# A", true)
        .expect("instruções");
    let conversa = criar_conversa(store, &org, &agente);
    gravar_mensagem(store, &org, &conversa, "oi");
    let turno = store
        .turns()
        .iniciar(org.id, conversa.id, Some(agente.id))
        .expect("turno");

    // Sem `--force` o repositório recusa (conflito tipado) e não apaga nada.
    let recusa = store
        .orgs()
        .excluir(org.id)
        .expect_err("recusa com agentes");
    assert!(matches!(
        recusa,
        StoreError::Domain(DomainError::Conflict { .. })
    ));
    store.orgs().get(org.id).expect("a org continua");

    let agentes_apagados = store.orgs().excluir_forcado(org.id).expect("exclui a org");
    assert_eq!(agentes_apagados, 1);
    assert!(store.orgs().get(org.id).is_err());
    assert!(
        store
            .agents()
            .listar(org.id, Page::default())
            .expect("lista")
            .is_empty()
    );
    assert_eq!(
        store
            .conversas()
            .listar(org.id, None, Page::default())
            .expect("lista")
            .len(),
        0
    );
    assert!(store.turns().obter(org.id, turno.id).is_err());
    assert!(
        store
            .departments()
            .listar(org.id, Page::default())
            .expect("lista")
            .is_empty()
    );
    assert!(store.broken_foreign_keys().expect("fk check").is_empty());

    // A outra org continua inteira.
    store.orgs().get(outro.id).expect("a outra org fica");
}

// ───────────────────────── D-05 e escopo de org nas instruções (HR-09) ─────────────────────────

#[test]
fn agente_inativo_nao_abre_turno_mas_continua_listado_e_editavel() {
    let cenario = Cenario::novo("agente-inativo");
    let store = cenario.store();
    let org = criar_org(store, "org-inativa");
    let agente = criar_agente(store, &org, "agente-inativo");

    // Active abre turno.
    assert!(agente.can_open_turn());
    store
        .agents()
        .obter_para_turno(org.id, agente.id)
        .expect("agente ativo abre turno");

    // Inativado, o mesmo id deixa de abrir turno com erro tipado.
    let inativo = store
        .agents()
        .atualizar(
            org.id,
            agente.id,
            agente.version,
            AgentChange {
                status: Some(Status::Inactive),
                ..AgentChange::default()
            },
        )
        .expect("inativa o agente");
    assert!(!inativo.can_open_turn(), "D-05: inativo não abre turno");
    match store.agents().obter_para_turno(org.id, agente.id) {
        Err(StoreError::Domain(DomainError::AgentInactive { id, status })) => {
            assert_eq!(id, agente.id.to_string());
            assert_eq!(status, "inactive");
        }
        outro => panic!("esperava AgenteInativo, veio {outro:?}"),
    }

    // Continua visível na listagem (o Console precisa mostrá-lo) e sai da lista de ativos.
    assert!(
        store
            .agents()
            .listar(org.id, Page::default())
            .expect("lista")
            .iter()
            .any(|item| item.id == agente.id)
    );
    assert!(
        store
            .agents()
            .listar_ativos(org.id)
            .expect("lista ativos")
            .is_empty()
    );
}

#[test]
fn instrucoes_de_uma_org_nao_aparecem_nem_se_apagam_em_outra() {
    let cenario = Cenario::novo("instrucoes-org");
    let store = cenario.store();
    let org_a = criar_org(store, "org-a-instrucoes");
    let org_b = criar_org(store, "org-b-instrucoes");
    let agente_a = criar_agente(store, &org_a, "agente-a-instrucoes");
    let agente_b = criar_agente(store, &org_b, "agente-b-instrucoes");

    store
        .instructions()
        .salvar(org_a.id, agente_a.id, 0, "{}", "# A", true)
        .expect("instruções da org A");
    store
        .instructions()
        .salvar(org_b.id, agente_b.id, 0, "{}", "# B", true)
        .expect("instruções da org B");

    // Ler com a org errada não devolve o conteúdo da outra.
    assert!(store.instructions().obter(org_b.id, agente_a.id).is_err());
    assert!(store.instructions().obter(org_a.id, agente_b.id).is_err());

    let apenas_b = store
        .instructions()
        .listar(org_b.id, Page::default())
        .expect("lista B");
    assert_eq!(apenas_b.len(), 1);
    assert_eq!(apenas_b[0].agent_id, agente_b.id);
    assert_eq!(apenas_b[0].markdown, "# B");

    // Apagar com a org errada não apaga.
    assert!(store.instructions().apagar(org_b.id, agente_a.id).is_err());
    assert!(store.instructions().obter(org_a.id, agente_a.id).is_ok());

    // A versão sobe a cada gravação, e o `If-Match` velho é recusado.
    let primeira = store
        .instructions()
        .obter(org_a.id, agente_a.id)
        .expect("instruções da org A");
    let segunda = store
        .instructions()
        .salvar(org_a.id, agente_a.id, primeira.version, "{}", "# A2", true)
        .expect("atualiza");
    assert_eq!(segunda.version, primeira.version + 1);
    let conflito = store
        .instructions()
        .salvar(org_a.id, agente_a.id, primeira.version, "{}", "# A3", true)
        .expect_err("versão velha deve conflitar");
    assert!(conflito.is_version_conflict(), "{conflito:?}");
}

#[test]
fn instrucoes_versao_zero_cria_e_versao_antiga_nao_recria() {
    let cenario = Cenario::novo("instrucoes-versao-ausente");
    let store = cenario.store();
    let org = criar_org(store, "org-instrucoes-versao");
    let agente = criar_agente(store, &org, "agente-instrucoes-versao");
    let repo = store.instructions();
    let primeira = repo
        .salvar(org.id, agente.id, 0, "{}", "primeira", true)
        .unwrap();
    assert!(
        repo.salvar(org.id, agente.id, 0, "{}", "duplicada", true)
            .unwrap_err()
            .is_version_conflict()
    );
    repo.apagar(org.id, agente.id).unwrap();
    let erro = repo
        .salvar(org.id, agente.id, primeira.version, "{}", "antiga", true)
        .unwrap_err();
    assert!(matches!(
        erro,
        StoreError::Domain(DomainError::VersionConflict {
            expected: 1,
            current: 0,
            ..
        })
    ));
    assert!(repo.obter(org.id, agente.id).is_err());
    assert_eq!(
        repo.salvar(org.id, agente.id, 0, "{}", "nova", true)
            .unwrap()
            .markdown,
        "nova"
    );
}

// ─────────────────────── queda e recuperação (HR-57) ───────────────────────

#[test]
fn turno_orfaos_fecha_so_os_comecados_antes_da_subida() {
    let cenario = Cenario::novo("orfao");
    let store = cenario.store();
    let org = criar_org(store, "org-orfao");
    let agente = criar_agente(store, &org, "agente-orfao");
    let conversa = criar_conversa(store, &org, &agente);

    let antes = store
        .turns()
        .iniciar(org.id, conversa.id, Some(agente.id))
        .expect("turno do processo anterior");
    // Espera o milissegundo virar: o carimbo do turno seguinte é estritamente maior.
    let corte = store.now() + 1;
    std::thread::sleep(std::time::Duration::from_millis(5));
    let depois = store
        .turns()
        .iniciar(org.id, conversa.id, Some(agente.id))
        .expect("turno desta instância");

    let fechados = store
        .turns()
        .falhar_orfaos(org.id, "interrompido", corte)
        .expect("fecha os órfãos");
    assert_eq!(fechados, 1, "só o turno anterior à subida foi fechado");

    let orfao = store.turns().obter(org.id, antes.id).expect("órfão");
    assert_eq!(orfao.estado, poph_store::TurnState::Failed);
    assert_eq!(orfao.error.as_deref(), Some("interrompido"));
    assert!(orfao.finished_at.is_some());

    let vivo = store.turns().obter(org.id, depois.id).expect("turno vivo");
    assert_eq!(
        vivo.estado,
        poph_store::TurnState::Running,
        "turno começado depois da subida não é tocado"
    );

    // Outra org não é atingida.
    let outra = criar_org(store, "org-vizinha");
    let agente_outra = criar_agente(store, &outra, "agente-vizinho");
    let conversa_outra = criar_conversa(store, &outra, &agente_outra);
    let da_outra = store
        .turns()
        .iniciar(outra.id, conversa_outra.id, Some(agente_outra.id))
        .expect("turno da vizinha");
    assert_eq!(
        store
            .turns()
            .falhar_orfaos(org.id, "interrompido", i64::MAX)
            .expect("fecha de novo"),
        1,
        "a varredura de uma org não fecha turno de outra"
    );
    assert_eq!(
        store
            .turns()
            .obter(outra.id, da_outra.id)
            .expect("turno da vizinha")
            .estado,
        poph_store::TurnState::Running
    );
}

#[test]
fn resposta_e_item_de_saida_entram_na_mesma_escrita() {
    let cenario = Cenario::novo("saida-junta");
    let store = cenario.store();
    let org = criar_org(store, "org-saida-junta");
    let agente = criar_agente(store, &org, "agente-saida-junta");
    let conversa = criar_conversa(store, &org, &agente);
    let turno = store
        .turns()
        .iniciar(org.id, conversa.id, Some(agente.id))
        .expect("turno");

    let (mensagem, item) = store
        .messages()
        .gravar_com_saida(
            org.id,
            NewMessage {
                conversation_id: conversa.id,
                direction: Direction::Outbound,
                role: Role::Assistant,
                texto: "resposta do agente".to_owned(),
                attachments_json: "[]".to_owned(),
                id_externo: None,
                turn_id: Some(turno.id),
                origin: poph_store::Origin::Agente,
                author_name: None,
            },
            NewOutboxItem {
                conversation_id: conversa.id,
                turn_id: turno.id,
                seq: 0,
                tipo: "text".to_owned(),
                content_json: "{\"texto\":\"resposta do agente\"}".to_owned(),
            },
        )
        .expect("grava mensagem e saída juntas");

    assert_eq!(mensagem.turn_id, Some(turno.id));
    assert_eq!(item.turn_id, turno.id);
    assert_eq!(item.estado, OutboxState::Pending);
    assert_eq!(
        store
            .messages()
            .listar(org.id, conversa.id, Page::with_limit(10))
            .expect("mensagens")
            .len(),
        1
    );
    assert_eq!(
        store
            .outbox()
            .listar_pendentes(org.id, 10)
            .expect("fila")
            .len(),
        1
    );

    // Repetir o mesmo turno + seq não duplica o item, e a mensagem (sem id externo) é uma nova.
    let (_, repetido) = store
        .messages()
        .gravar_com_saida(
            org.id,
            NewMessage {
                conversation_id: conversa.id,
                direction: Direction::Outbound,
                role: Role::Assistant,
                texto: "resposta do agente".to_owned(),
                attachments_json: "[]".to_owned(),
                id_externo: None,
                turn_id: Some(turno.id),
                origin: poph_store::Origin::Agente,
                author_name: None,
            },
            NewOutboxItem {
                conversation_id: conversa.id,
                turn_id: turno.id,
                seq: 0,
                tipo: "text".to_owned(),
                content_json: "{\"texto\":\"resposta do agente\"}".to_owned(),
            },
        )
        .expect("repete");
    assert_eq!(
        repetido.id, item.id,
        "o item da fila é o mesmo (turno + seq)"
    );
    assert_eq!(
        store
            .outbox()
            .listar_pendentes(org.id, 10)
            .expect("fila")
            .len(),
        1
    );
}
