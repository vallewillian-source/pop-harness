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
    Agente, Conversa, Direcao, ErroStore, EstadoSaida, MudancaAgente, MudancaCategoria,
    MudancaDepartamento, MudancaOrg, NovaCategoria, NovaConversa, NovaEntrada, NovaMensagem,
    NovaOrg, NovaSaida, NovoAgente, NovoDepartamento, NovoRegistroAuditoria, Org, Pagina, Papel,
    Recebido, Status, StatusConversa, Store, versao_do_arquivo,
};
use poph_types::{DomainError, InboxId, IntegrationId, MessageId, OrgId};

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
        let store = Store::abrir_em(&pasta.join("harness.db"), &pasta.join("rag.db"))
            .expect("abre o banco");
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
        .orgaos()
        .criar(NovaOrg {
            nome: format!("Org {slug}"),
            slug: slug.to_owned(),
            config_json: String::new(),
        })
        .expect("cria a org")
}

fn criar_agente(store: &Store, org: &Org, slug: &str) -> Agente {
    let novo = NovoAgente {
        nome: format!("Agente {slug}"),
        slug: slug.to_owned(),
        ..NovoAgente::default()
    };
    store.agentes().criar(org.id, novo).expect("cria o agente")
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
                origem: poph_store::Origem::Agente,
                autor_nome: None,
            },
        )
        .expect("grava a mensagem")
        .id
}

#[test]
fn migracao_cria_o_esquema_do_prd_e_e_idempotente() {
    let cenario = Cenario::novo("migracao");
    let store = cenario.store();

    let tabelas = store.tabelas().expect("lista tabelas");
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

    let caminho = store.caminho_harness().to_path_buf();
    assert_eq!(
        versao_do_arquivo(&caminho).expect("lê versão"),
        Some(poph_store::versao_alvo_harness())
    );
    assert!(store.chaves_quebradas().expect("fk check").is_empty());

    // Rodar de novo não aplica nada nem muda a versão.
    let relatorios = store.migrar().expect("migra de novo");
    assert!(relatorios.ja_estava_em_dia(), "{relatorios:?}");
    assert_eq!(
        versao_do_arquivo(&caminho).expect("lê versão"),
        Some(poph_store::versao_alvo_harness())
    );

    // `rag.db` existe e está na versão 0 (sem tabelas de trechos até a HR-16).
    let caminho_rag = store.caminho_rag().to_path_buf();
    assert_eq!(versao_do_arquivo(&caminho_rag).expect("lê versão"), Some(0));
}

#[test]
fn toda_tabela_de_negocio_tem_org_id() {
    let cenario = Cenario::novo("org-id");
    let store = cenario.store();
    // `orgs` é a própria org; `schema_migrations` é infraestrutura.
    let sem_org = ["orgs", "schema_migrations"];
    let mut verificadas = 0;
    for tabela in store.tabelas().expect("lista tabelas") {
        if sem_org.contains(&tabela.as_str()) {
            continue;
        }
        let colunas = store.colunas(&tabela).expect("lê colunas");
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
        .categorias()
        .criar(
            org_a.id,
            NovaCategoria {
                nome: "Dedetização".to_owned(),
                descricao: String::new(),
            },
        )
        .expect("categoria");
    let departamento = store
        .departamentos()
        .criar(
            org_a.id,
            NovoDepartamento {
                nome: "Comercial".to_owned(),
                descricao: String::new(),
            },
        )
        .expect("departamento");
    let agente = criar_agente(store, &org_a, "agente-a");
    store
        .agentes()
        .definir_categorias(org_a.id, agente.id, agente.versao, &[categoria.id])
        .expect("vincula categoria");
    store
        .instrucoes()
        .salvar(org_a.id, agente.id, 0, "{}", "# A", true)
        .expect("instruções");
    let conversa = criar_conversa(store, &org_a, &agente);
    let mensagem = gravar_mensagem(store, &org_a, &conversa, "oi");
    let turno = store
        .turnos()
        .iniciar(org_a.id, conversa.id, Some(agente.id))
        .expect("turno");
    let entrada = store
        .caixa_entrada()
        .receber(
            org_a.id,
            NovaEntrada {
                integration_id: IntegrationId::generate(),
                id_externo: "wamid-1".to_owned(),
                evento_json: "{}".to_owned(),
            },
        )
        .expect("entrada")
        .1;
    let saida = store
        .caixa_saida()
        .enfileirar(
            org_a.id,
            NovaSaida {
                conversation_id: conversa.id,
                turn_id: turno.id,
                seq: 0,
                tipo: "text".to_owned(),
                conteudo_json: "{}".to_owned(),
            },
        )
        .expect("saída")
        .1;
    let token = store
        .tokens()
        .criar(Some(org_a.id), "token A", "hash-a")
        .expect("token");
    store
        .auditoria()
        .registrar(
            Some(org_a.id),
            NovoRegistroAuditoria {
                ator_tipo: "system".to_owned(),
                ator_id: None,
                acao: "create".to_owned(),
                entidade: "agent".to_owned(),
                entidade_id: Some(agente.id.to_string()),
                detalhe_json: "{}".to_owned(),
            },
        )
        .expect("auditoria");

    // Tudo consultado com a org B dá "não encontrado" ou lista vazia.
    let nao_encontrado = |resultado: Result<(), poph_store::ErroStore>| match resultado {
        Err(poph_store::ErroStore::Dominio(DomainError::NaoEncontrado { .. })) => true,
        outro => panic!("esperado não encontrado, veio {outro:?}"),
    };

    nao_encontrado(store.categorias().obter(org_b.id, categoria.id).map(|_| ()));
    assert!(
        store
            .categorias()
            .listar(org_b.id, Pagina::default())
            .expect("lista")
            .is_empty()
    );
    nao_encontrado(
        store
            .departamentos()
            .obter(org_b.id, departamento.id)
            .map(|_| ()),
    );
    assert!(
        store
            .departamentos()
            .listar(org_b.id, Pagina::default())
            .expect("lista")
            .is_empty()
    );
    nao_encontrado(store.agentes().obter(org_b.id, agente.id).map(|_| ()));
    assert!(
        store
            .agentes()
            .listar(org_b.id, Pagina::default())
            .expect("lista")
            .is_empty()
    );
    assert!(
        store
            .agentes()
            .categorias(org_b.id, agente.id)
            .expect("categorias")
            .is_empty()
    );
    nao_encontrado(store.instrucoes().obter(org_b.id, agente.id).map(|_| ()));
    nao_encontrado(store.conversas().obter(org_b.id, conversa.id).map(|_| ()));
    assert!(
        store
            .conversas()
            .listar(org_b.id, None, Pagina::default())
            .expect("lista")
            .is_empty()
    );
    nao_encontrado(store.conversas().agente(org_b.id, conversa.id).map(|_| ()));
    assert!(
        store
            .mensagens()
            .listar(org_b.id, conversa.id, Pagina::default())
            .expect("lista")
            .is_empty()
    );
    nao_encontrado(
        store
            .mensagens()
            .obter(org_b.id, conversa.id, mensagem)
            .map(|_| ()),
    );
    assert_eq!(
        store
            .mensagens()
            .contar(org_b.id, conversa.id)
            .expect("conta"),
        0
    );
    nao_encontrado(store.turnos().obter(org_b.id, turno.id).map(|_| ()));
    assert!(
        store
            .turnos()
            .listar(org_b.id, conversa.id, Pagina::default())
            .expect("lista")
            .is_empty()
    );
    nao_encontrado(
        store
            .caixa_entrada()
            .obter(org_b.id, entrada.id)
            .map(|_| ()),
    );
    assert!(
        store
            .caixa_entrada()
            .listar_pendentes(org_b.id, 100)
            .expect("lista")
            .is_empty()
    );
    nao_encontrado(store.caixa_saida().obter(org_b.id, saida.id).map(|_| ()));
    assert!(
        store
            .caixa_saida()
            .listar_pendentes(org_b.id, 100)
            .expect("lista")
            .is_empty()
    );
    assert!(
        store
            .tokens()
            .listar(org_b.id, Pagina::default())
            .expect("lista")
            .is_empty()
    );
    nao_encontrado(store.tokens().obter(org_b.id, token.id).map(|_| ()));
    assert!(
        store
            .auditoria()
            .listar(org_b.id, Pagina::default())
            .expect("lista")
            .is_empty()
    );

    // E a org A continua enxergando tudo.
    assert_eq!(
        store
            .mensagens()
            .contar(org_a.id, conversa.id)
            .expect("conta"),
        1
    );
    assert_eq!(
        store
            .caixa_entrada()
            .listar_pendentes(org_a.id, 100)
            .expect("lista")
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
        .categorias()
        .criar(
            org.id,
            NovaCategoria {
                nome: "Categoria".to_owned(),
                descricao: String::new(),
            },
        )
        .expect("categoria");
    let departamento = store
        .departamentos()
        .criar(
            org.id,
            NovoDepartamento {
                nome: "Departamento".to_owned(),
                descricao: String::new(),
            },
        )
        .expect("departamento");
    let conversa = criar_conversa(store, &org, &agente);
    store
        .instrucoes()
        .salvar(org.id, agente.id, 0, "{}", "# v1", true)
        .expect("instruções");

    let confere_conflito = |resultado: Result<(), poph_store::ErroStore>| match resultado {
        Err(erro) if erro.eh_conflito_de_versao() => {}
        outro => panic!("esperado conflito de versão, veio {outro:?}"),
    };

    confere_conflito(
        store
            .orgaos()
            .atualizar(
                org.id,
                org.versao + 5,
                MudancaOrg {
                    nome: Some("Outro nome".to_owned()),
                    ..MudancaOrg::default()
                },
            )
            .map(|_| ()),
    );
    confere_conflito(
        store
            .agentes()
            .atualizar(
                org.id,
                agente.id,
                agente.versao + 5,
                MudancaAgente {
                    modelo: Some("outro".to_owned()),
                    ..MudancaAgente::default()
                },
            )
            .map(|_| ()),
    );
    confere_conflito(
        store
            .categorias()
            .atualizar(
                org.id,
                categoria.id,
                categoria.versao + 5,
                MudancaCategoria {
                    nome: Some("Outra".to_owned()),
                    ..MudancaCategoria::default()
                },
            )
            .map(|_| ()),
    );
    confere_conflito(
        store
            .departamentos()
            .atualizar(
                org.id,
                departamento.id,
                departamento.versao + 5,
                MudancaDepartamento {
                    nome: Some("Outro".to_owned()),
                    ..MudancaDepartamento::default()
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
                conversa.versao + 5,
                StatusConversa::Fechada,
                None,
            )
            .map(|_| ()),
    );
    confere_conflito(
        store
            .instrucoes()
            .salvar(org.id, agente.id, 9, "{}", "# v2", true)
            .map(|_| ()),
    );

    // Com a versão certa, a escrita anda e incrementa.
    let atualizado = store
        .agentes()
        .atualizar(
            org.id,
            agente.id,
            agente.versao,
            MudancaAgente {
                modelo: Some("gemini-3.8-flash-tiered".to_owned()),
                status: Some(Status::Inativo),
                ..MudancaAgente::default()
            },
        )
        .expect("atualiza");
    assert_eq!(atualizado.versao, agente.versao + 1);
    assert_eq!(atualizado.status, Status::Inativo);
}

#[test]
fn mensagem_repetida_pelo_id_externo_nao_duplica() {
    let cenario = Cenario::novo("mensagem-idempotente");
    let store = cenario.store();
    let org = criar_org(store, "org-msg");
    let agente = criar_agente(store, &org, "agente-msg");
    let conversa = criar_conversa(store, &org, &agente);

    let nova = || NovaMensagem {
        conversation_id: conversa.id,
        direcao: Direcao::Entrada,
        papel: Papel::Usuario,
        origem: poph_store::Origem::Agente,
        autor_nome: None,
        texto: "bom dia".to_owned(),
        anexos_json: "[]".to_owned(),
        id_externo: Some("wamid-42".to_owned()),
        turn_id: None,
    };
    let primeira = store.mensagens().gravar(org.id, nova()).expect("grava");
    let segunda = store.mensagens().gravar(org.id, nova()).expect("regrava");
    assert_eq!(
        primeira.id, segunda.id,
        "a segunda gravação devolve a mesma"
    );
    assert_eq!(
        store
            .mensagens()
            .contar(org.id, conversa.id)
            .expect("conta"),
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

    let entrada = || NovaEntrada {
        integration_id: integracao,
        id_externo: "wamid-7".to_owned(),
        evento_json: "{\"texto\":\"oi\"}".to_owned(),
    };
    let (primeiro, item) = store
        .caixa_entrada()
        .receber(org.id, entrada())
        .expect("entrada");
    assert_eq!(primeiro, Recebido::Novo);
    let (segundo, repetido) = store
        .caixa_entrada()
        .receber(org.id, entrada())
        .expect("reentrega");
    assert_eq!(segundo, Recebido::Duplicado);
    assert_eq!(repetido.id, item.id);
    assert_eq!(
        store
            .caixa_entrada()
            .listar_pendentes(org.id, 100)
            .expect("lista")
            .len(),
        1
    );

    store
        .caixa_entrada()
        .marcar_processado(org.id, item.id, None)
        .expect("processa");
    assert!(
        store
            .caixa_entrada()
            .listar_pendentes(org.id, 100)
            .expect("lista")
            .is_empty()
    );

    let turno = store
        .turnos()
        .iniciar(org.id, conversa.id, Some(agente.id))
        .expect("turno");
    let saida = || NovaSaida {
        conversation_id: conversa.id,
        turn_id: turno.id,
        seq: 0,
        tipo: "text".to_owned(),
        conteudo_json: "{\"texto\":\"olá\"}".to_owned(),
    };
    let (novo, item_saida) = store
        .caixa_saida()
        .enfileirar(org.id, saida())
        .expect("saída");
    assert!(novo);
    let (novo_de_novo, repetido) = store
        .caixa_saida()
        .enfileirar(org.id, saida())
        .expect("saída repetida");
    assert!(!novo_de_novo);
    assert_eq!(repetido.id, item_saida.id);

    store
        .caixa_saida()
        .mudar_estado(org.id, item_saida.id, EstadoSaida::Enviando, None)
        .expect("enviando");
    // `sending` continua na fila: é o reenvio da retomada (D11).
    assert_eq!(
        store
            .caixa_saida()
            .listar_pendentes(org.id, 100)
            .expect("lista")
            .len(),
        1
    );
    store
        .caixa_saida()
        .mudar_estado(org.id, item_saida.id, EstadoSaida::Enviado, None)
        .expect("enviado");
    assert!(
        store
            .caixa_saida()
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
        .turnos()
        .iniciar(org.id, conversa.id, Some(agente.id))
        .expect("turno");
    let fim = poph_store::FinalizacaoTurno {
        estado: Some(poph_store::EstadoTurno::Concluido),
        tempos_json: Some("{\"llm_ms\":6500}".to_owned()),
        tokens_entrada: Some(1200),
        tokens_saida: Some(80),
        ..poph_store::FinalizacaoTurno::default()
    };
    let fechado = store
        .turnos()
        .finalizar(org.id, turno.id, fim)
        .expect("fecha");
    assert_eq!(fechado.estado, poph_store::EstadoTurno::Concluido);
    assert!(fechado.finalizado_em.is_some());

    let registro = store
        .auditoria()
        .registrar(
            Some(org.id),
            NovoRegistroAuditoria {
                ator_tipo: "admin_token".to_owned(),
                ator_id: Some("token-1".to_owned()),
                acao: "update".to_owned(),
                entidade: "agent".to_owned(),
                entidade_id: Some(agente.id.to_string()),
                detalhe_json: "{\"campo\":\"modelo\"}".to_owned(),
            },
        )
        .expect("auditoria");
    assert_eq!(registro.acao, "update");
    assert_eq!(
        store
            .auditoria()
            .listar(org.id, Pagina::default())
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
                    .mensagens()
                    .gravar(
                        org_id,
                        NovaMensagem {
                            conversation_id: conversa_id,
                            direcao: Direcao::Entrada,
                            papel: Papel::Usuario,
                            texto: format!("mensagem {thread_id}-{indice}"),
                            anexos_json: "[]".to_owned(),
                            id_externo: None,
                            turn_id: None,
                            origem: poph_store::Origem::Agente,
                            autor_nome: None,
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
        store
            .mensagens()
            .contar(org.id, conversa.id)
            .expect("conta"),
        (THREADS * POR_THREAD) as u64,
        "nenhuma escrita pode ser perdida"
    );
    assert!(store.chaves_quebradas().expect("fk check").is_empty());
}

#[test]
fn entrada_invalida_nao_derruba_o_lote() {
    let cenario = Cenario::novo("erro-no-lote");
    let store = cenario.store();
    let org = criar_org(store, "org-erro");

    // Uma org com slug repetido falha; a escrita seguinte, válida, precisa passar.
    let erro = store
        .orgaos()
        .criar(NovaOrg {
            nome: "Repetida".to_owned(),
            slug: "org-erro".to_owned(),
            config_json: String::new(),
        })
        .expect_err("slug repetido deve falhar");
    assert!(
        matches!(
            erro,
            poph_store::ErroStore::Dominio(DomainError::Validacao { .. })
        ),
        "veio {erro:?}"
    );

    let outra = criar_org(store, "org-erro-2");
    assert_eq!(outra.status, Status::Ativo);
    assert_eq!(
        store
            .orgaos()
            .listar(Pagina::default())
            .expect("lista")
            .len(),
        2
    );

    // Ids que não existem dão "não encontrado" com a entidade no texto.
    let inexistente = store
        .agentes()
        .obter(org.id, poph_types::AgentId::generate())
        .expect_err("agente inexistente");
    assert!(inexistente.eh_nao_encontrado(), "veio {inexistente:?}");
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
            store
                .caixa_entrada()
                .obter(org.id, InboxId::generate())
                .map(|_| ()),
            Err(poph_store::ErroStore::Dominio(
                DomainError::NaoEncontrado { .. }
            ))
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
        .departamentos()
        .criar(
            org_b.id,
            NovoDepartamento {
                nome: "Departamento B".to_owned(),
                descricao: String::new(),
            },
        )
        .expect("departamento de B");
    let categoria_b = store
        .categorias()
        .criar(
            org_b.id,
            NovaCategoria {
                nome: "Categoria B".to_owned(),
                descricao: String::new(),
            },
        )
        .expect("categoria de B");

    // Departamento de outra org num agente: recusado pelo repositório, sem gravar.
    let recusado = store
        .agentes()
        .atualizar(
            org_a.id,
            agente_a.id,
            agente_a.versao,
            MudancaAgente {
                departamento_padrao: Some(departamento_b.id),
                ..MudancaAgente::default()
            },
        )
        .expect_err("departamento de outra org");
    assert!(recusado.eh_nao_encontrado(), "veio {recusado:?}");
    assert_eq!(
        store
            .agentes()
            .obter(org_a.id, agente_a.id)
            .expect("agente intacto")
            .versao,
        agente_a.versao,
        "a versão não pode andar numa escrita recusada"
    );

    // Categoria de outra org no agente: recusado, e nenhum vínculo gravado.
    let recusado = store
        .agentes()
        .definir_categorias(org_a.id, agente_a.id, agente_a.versao, &[categoria_b.id])
        .expect_err("categoria de outra org");
    assert!(recusado.eh_nao_encontrado(), "veio {recusado:?}");
    assert!(
        store
            .agentes()
            .categorias(org_a.id, agente_a.id)
            .expect("categorias")
            .is_empty()
    );

    // Conversa da org A com agente da org B: a chave estrangeira composta recusa.
    let recusado = store
        .conversas()
        .criar(
            org_a.id,
            NovaConversa {
                agent_id: agente_b.id,
                integration_id: None,
                canal: "cli".to_owned(),
                peer_externo: None,
                departamento_alvo: None,
            },
        )
        .expect_err("agente de outra org");
    assert!(
        matches!(recusado, poph_store::ErroStore::Sqlite(_)),
        "veio {recusado:?}"
    );
    assert!(
        store
            .conversas()
            .listar(org_a.id, None, Pagina::default())
            .expect("lista")
            .is_empty(),
        "conversa recusada não pode aparecer"
    );

    // Mensagem na org A apontando para conversa da org B: recusada e nada gravado.
    let conversa_b = criar_conversa(store, &org_b, &agente_b);
    let recusado = store
        .mensagens()
        .gravar(
            org_a.id,
            NovaMensagem {
                conversation_id: conversa_b.id,
                direcao: Direcao::Entrada,
                papel: Papel::Usuario,
                texto: "invasão".to_owned(),
                anexos_json: "[]".to_owned(),
                id_externo: None,
                turn_id: None,
                origem: poph_store::Origem::Agente,
                autor_nome: None,
            },
        )
        .expect_err("conversa de outra org");
    assert!(
        matches!(recusado, poph_store::ErroStore::Sqlite(_)),
        "veio {recusado:?}"
    );
    assert_eq!(
        store
            .mensagens()
            .contar(org_b.id, conversa_b.id)
            .expect("conta"),
        0
    );

    // Turno na org A com agente da org B: recusado pelo repositório.
    let conversa_a = criar_conversa(store, &org_a, &agente_a);
    let recusado = store
        .turnos()
        .iniciar(org_a.id, conversa_a.id, Some(agente_b.id))
        .expect_err("agente de outra org");
    assert!(recusado.eh_nao_encontrado(), "veio {recusado:?}");
}

#[test]
fn excluir_org_leva_junto_os_dados_dela() {
    let cenario = Cenario::novo("cascata");
    let store = cenario.store();
    let org = criar_org(store, "org-cascata");
    let outro = criar_org(store, "org-fica");
    store
        .departamentos()
        .criar(
            org.id,
            NovoDepartamento {
                nome: "Dep".to_owned(),
                descricao: String::new(),
            },
        )
        .expect("departamento");
    let categoria = store
        .categorias()
        .criar(
            org.id,
            NovaCategoria {
                nome: "Cat".to_owned(),
                descricao: String::new(),
            },
        )
        .expect("categoria");
    let agente = criar_agente(store, &org, "agente-cascata");
    store
        .agentes()
        .definir_categorias(org.id, agente.id, agente.versao, &[categoria.id])
        .expect("vínculo");
    store
        .instrucoes()
        .salvar(org.id, agente.id, 0, "{}", "# A", true)
        .expect("instruções");
    let conversa = criar_conversa(store, &org, &agente);
    gravar_mensagem(store, &org, &conversa, "oi");
    let turno = store
        .turnos()
        .iniciar(org.id, conversa.id, Some(agente.id))
        .expect("turno");

    // Sem `--force` o repositório recusa (conflito tipado) e não apaga nada.
    let recusa = store
        .orgaos()
        .excluir(org.id)
        .expect_err("recusa com agentes");
    assert!(matches!(
        recusa,
        ErroStore::Dominio(DomainError::Conflito { .. })
    ));
    store.orgaos().obter(org.id).expect("a org continua");

    let agentes_apagados = store
        .orgaos()
        .excluir_forcado(org.id)
        .expect("exclui a org");
    assert_eq!(agentes_apagados, 1);
    assert!(store.orgaos().obter(org.id).is_err());
    assert!(
        store
            .agentes()
            .listar(org.id, Pagina::default())
            .expect("lista")
            .is_empty()
    );
    assert_eq!(
        store
            .conversas()
            .listar(org.id, None, Pagina::default())
            .expect("lista")
            .len(),
        0
    );
    assert!(store.turnos().obter(org.id, turno.id).is_err());
    assert!(
        store
            .departamentos()
            .listar(org.id, Pagina::default())
            .expect("lista")
            .is_empty()
    );
    assert!(store.chaves_quebradas().expect("fk check").is_empty());

    // A outra org continua inteira.
    store.orgaos().obter(outro.id).expect("a outra org fica");
}

// ───────────────────────── D-05 e escopo de org nas instruções (HR-09) ─────────────────────────

#[test]
fn agente_inativo_nao_abre_turno_mas_continua_listado_e_editavel() {
    let cenario = Cenario::novo("agente-inativo");
    let store = cenario.store();
    let org = criar_org(store, "org-inativa");
    let agente = criar_agente(store, &org, "agente-inativo");

    // Ativo abre turno.
    assert!(agente.pode_abrir_turno());
    store
        .agentes()
        .obter_para_turno(org.id, agente.id)
        .expect("agente ativo abre turno");

    // Inativado, o mesmo id deixa de abrir turno com erro tipado.
    let inativo = store
        .agentes()
        .atualizar(
            org.id,
            agente.id,
            agente.versao,
            MudancaAgente {
                status: Some(Status::Inativo),
                ..MudancaAgente::default()
            },
        )
        .expect("inativa o agente");
    assert!(!inativo.pode_abrir_turno(), "D-05: inativo não abre turno");
    match store.agentes().obter_para_turno(org.id, agente.id) {
        Err(ErroStore::Dominio(DomainError::AgenteInativo { id, status })) => {
            assert_eq!(id, agente.id.to_string());
            assert_eq!(status, "inactive");
        }
        outro => panic!("esperava AgenteInativo, veio {outro:?}"),
    }

    // Continua visível na listagem (o Console precisa mostrá-lo) e sai da lista de ativos.
    assert!(
        store
            .agentes()
            .listar(org.id, Pagina::default())
            .expect("lista")
            .iter()
            .any(|item| item.id == agente.id)
    );
    assert!(
        store
            .agentes()
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
        .instrucoes()
        .salvar(org_a.id, agente_a.id, 0, "{}", "# A", true)
        .expect("instruções da org A");
    store
        .instrucoes()
        .salvar(org_b.id, agente_b.id, 0, "{}", "# B", true)
        .expect("instruções da org B");

    // Ler com a org errada não devolve o conteúdo da outra.
    assert!(store.instrucoes().obter(org_b.id, agente_a.id).is_err());
    assert!(store.instrucoes().obter(org_a.id, agente_b.id).is_err());

    let apenas_b = store
        .instrucoes()
        .listar(org_b.id, Pagina::default())
        .expect("lista B");
    assert_eq!(apenas_b.len(), 1);
    assert_eq!(apenas_b[0].agent_id, agente_b.id);
    assert_eq!(apenas_b[0].markdown, "# B");

    // Apagar com a org errada não apaga.
    assert!(store.instrucoes().apagar(org_b.id, agente_a.id).is_err());
    assert!(store.instrucoes().obter(org_a.id, agente_a.id).is_ok());

    // A versão sobe a cada gravação, e o `If-Match` velho é recusado.
    let primeira = store
        .instrucoes()
        .obter(org_a.id, agente_a.id)
        .expect("instruções da org A");
    let segunda = store
        .instrucoes()
        .salvar(org_a.id, agente_a.id, primeira.versao, "{}", "# A2", true)
        .expect("atualiza");
    assert_eq!(segunda.versao, primeira.versao + 1);
    let conflito = store
        .instrucoes()
        .salvar(org_a.id, agente_a.id, primeira.versao, "{}", "# A3", true)
        .expect_err("versão velha deve conflitar");
    assert!(conflito.eh_conflito_de_versao(), "{conflito:?}");
}

#[test]
fn instrucoes_versao_zero_cria_e_versao_antiga_nao_recria() {
    let cenario = Cenario::novo("instrucoes-versao-ausente");
    let store = cenario.store();
    let org = criar_org(store, "org-instrucoes-versao");
    let agente = criar_agente(store, &org, "agente-instrucoes-versao");
    let repo = store.instrucoes();
    let primeira = repo
        .salvar(org.id, agente.id, 0, "{}", "primeira", true)
        .unwrap();
    assert!(
        repo.salvar(org.id, agente.id, 0, "{}", "duplicada", true)
            .unwrap_err()
            .eh_conflito_de_versao()
    );
    repo.apagar(org.id, agente.id).unwrap();
    let erro = repo
        .salvar(org.id, agente.id, primeira.versao, "{}", "antiga", true)
        .unwrap_err();
    assert!(matches!(
        erro,
        ErroStore::Dominio(DomainError::ConflitoDeVersao {
            esperada: 1,
            atual: 0,
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
        .turnos()
        .iniciar(org.id, conversa.id, Some(agente.id))
        .expect("turno do processo anterior");
    // Espera o milissegundo virar: o carimbo do turno seguinte é estritamente maior.
    let corte = store.agora() + 1;
    std::thread::sleep(std::time::Duration::from_millis(5));
    let depois = store
        .turnos()
        .iniciar(org.id, conversa.id, Some(agente.id))
        .expect("turno desta instância");

    let fechados = store
        .turnos()
        .falhar_orfaos(org.id, "interrompido", corte)
        .expect("fecha os órfãos");
    assert_eq!(fechados, 1, "só o turno anterior à subida foi fechado");

    let orfao = store.turnos().obter(org.id, antes.id).expect("órfão");
    assert_eq!(orfao.estado, poph_store::EstadoTurno::Falhou);
    assert_eq!(orfao.erro.as_deref(), Some("interrompido"));
    assert!(orfao.finalizado_em.is_some());

    let vivo = store.turnos().obter(org.id, depois.id).expect("turno vivo");
    assert_eq!(
        vivo.estado,
        poph_store::EstadoTurno::Rodando,
        "turno começado depois da subida não é tocado"
    );

    // Outra org não é atingida.
    let outra = criar_org(store, "org-vizinha");
    let agente_outra = criar_agente(store, &outra, "agente-vizinho");
    let conversa_outra = criar_conversa(store, &outra, &agente_outra);
    let da_outra = store
        .turnos()
        .iniciar(outra.id, conversa_outra.id, Some(agente_outra.id))
        .expect("turno da vizinha");
    assert_eq!(
        store
            .turnos()
            .falhar_orfaos(org.id, "interrompido", i64::MAX)
            .expect("fecha de novo"),
        1,
        "a varredura de uma org não fecha turno de outra"
    );
    assert_eq!(
        store
            .turnos()
            .obter(outra.id, da_outra.id)
            .expect("turno da vizinha")
            .estado,
        poph_store::EstadoTurno::Rodando
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
        .turnos()
        .iniciar(org.id, conversa.id, Some(agente.id))
        .expect("turno");

    let (mensagem, item) = store
        .mensagens()
        .gravar_com_saida(
            org.id,
            NovaMensagem {
                conversation_id: conversa.id,
                direcao: Direcao::Saida,
                papel: Papel::Assistente,
                texto: "resposta do agente".to_owned(),
                anexos_json: "[]".to_owned(),
                id_externo: None,
                turn_id: Some(turno.id),
                origem: poph_store::Origem::Agente,
                autor_nome: None,
            },
            NovaSaida {
                conversation_id: conversa.id,
                turn_id: turno.id,
                seq: 0,
                tipo: "text".to_owned(),
                conteudo_json: "{\"texto\":\"resposta do agente\"}".to_owned(),
            },
        )
        .expect("grava mensagem e saída juntas");

    assert_eq!(mensagem.turn_id, Some(turno.id));
    assert_eq!(item.turn_id, turno.id);
    assert_eq!(item.estado, EstadoSaida::Pendente);
    assert_eq!(
        store
            .mensagens()
            .listar(org.id, conversa.id, Pagina::com_limite(10))
            .expect("mensagens")
            .len(),
        1
    );
    assert_eq!(
        store
            .caixa_saida()
            .listar_pendentes(org.id, 10)
            .expect("fila")
            .len(),
        1
    );

    // Repetir o mesmo turno + seq não duplica o item, e a mensagem (sem id externo) é uma nova.
    let (_, repetido) = store
        .mensagens()
        .gravar_com_saida(
            org.id,
            NovaMensagem {
                conversation_id: conversa.id,
                direcao: Direcao::Saida,
                papel: Papel::Assistente,
                texto: "resposta do agente".to_owned(),
                anexos_json: "[]".to_owned(),
                id_externo: None,
                turn_id: Some(turno.id),
                origem: poph_store::Origem::Agente,
                autor_nome: None,
            },
            NovaSaida {
                conversation_id: conversa.id,
                turn_id: turno.id,
                seq: 0,
                tipo: "text".to_owned(),
                conteudo_json: "{\"texto\":\"resposta do agente\"}".to_owned(),
            },
        )
        .expect("repete");
    assert_eq!(
        repetido.id, item.id,
        "o item da fila é o mesmo (turno + seq)"
    );
    assert_eq!(
        store
            .caixa_saida()
            .listar_pendentes(org.id, 10)
            .expect("fila")
            .len(),
        1
    );
}
