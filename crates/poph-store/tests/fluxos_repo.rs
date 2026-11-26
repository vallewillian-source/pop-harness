//! Testes do repositório de fluxos e da permissão por agente (FX-08, PRD §6.2 e §7.1).
//!
//! O que fica provado aqui:
//!
//! - **CRUD** de fluxo, com `If-Match` (`esperada` = `version`) e recusa de situação `archived`;
//! - **slug** único por org, com o formato da linguagem, e repetido em outra org aceito;
//! - **teto de 20 fluxos por agente**: o 21º é recusado, repetir um já liberado não conta, e a
//!   troca em lote com 21 recusa sem apagar a lista antiga;
//! - **org**: permissão que cite fluxo ou agente de outra org é recusada, e a troca em lote não deixa
//!   meia lista quando um fluxo é de outra org;
//! - **exclusão**: sem execução ativa devolve `0`; com execução ativa recusa sem `force` e, com
//!   `force`, abandona a execução (motivo `fluxo_excluido`), registra o evento na Ficha, mantém o
//!   histórico e a Ficha, e leva junto só a permissão;
//! - **versão do agente** avança quando a permissão muda, e `substituir_lista` recusa `If-Match`
//!   velho;
//! - **apagar agente** leva só a permissão, nunca o fluxo.
//!
//! O isolamento entre orgs de todas as leituras do repositório também é conferido no teste
//! `nenhum_repositorio_le_registro_de_outra_org` (em `armazenamento.rs`).

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use poph_store::{
    Agente, Conversa, ErroStore, FiltroFluxos, Fluxo, MOTIVO_EXCLUSAO_FLUXO, MudancaFluxo,
    NovaConversa, NovaOrg, NovoAgente, NovoFluxo, Org, Pagina, Status, Store,
    TETO_FLUXOS_POR_AGENTE,
};
use poph_types::{AgentId, DomainError, EventActor, FlowId, OrgId, RecordEventKind, RecordId};
use rusqlite::Connection;

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
            std::env::temp_dir().join(format!("poph-fluxos-{rotulo}-{}-{n}", std::process::id()));
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

    /// Conexão própria, para o que o repositório não expõe (execuções são da FX-13) e para conferir o
    /// banco por SQL.
    fn conexao(&self) -> Connection {
        let conn = Connection::open(self.pasta.join("harness.db")).expect("abre para conferir");
        conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA busy_timeout = 5000;")
            .expect("pragmas");
        conn
    }

    fn org(&self, slug: &str) -> Org {
        self.store()
            .orgaos()
            .criar(NovaOrg {
                nome: format!("Org {slug}"),
                slug: slug.to_owned(),
                config_json: String::new(),
            })
            .expect("cria a org")
    }

    fn agente(&self, org: &Org, slug: &str) -> Agente {
        self.store()
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

    fn fluxo(&self, org: &Org, slug: &str) -> Fluxo {
        self.store()
            .fluxos()
            .criar(
                org.id,
                NovoFluxo {
                    slug: slug.to_owned(),
                    nome: format!("Fluxo {slug}"),
                    descricao: String::new(),
                    rascunho_fonte: String::new(),
                },
            )
            .expect("cria o fluxo")
    }

    fn versao_agente(&self, org: &Org, agente: &Agente) -> u64 {
        self.store()
            .agentes()
            .obter(org.id, agente.id)
            .expect("lê o agente")
            .versao
    }

    fn conversa(&self, org: &Org, agente: &Agente) -> Conversa {
        self.store()
            .conversas()
            .criar(
                org.id,
                NovaConversa {
                    agent_id: agente.id,
                    integration_id: None,
                    canal: "console".to_owned(),
                    peer_externo: Some("cliente-1".to_owned()),
                    departamento_alvo: None,
                },
            )
            .expect("abre a conversa")
    }

    /// Execução de fluxo ativa numa Ficha, gravada direto (quem escreve execução é a FX-13).
    fn execucao_ativa(&self, org: OrgId, ficha: RecordId, fluxo: FlowId) -> String {
        let execucao = uuid::Uuid::now_v7().to_string();
        let revisao = uuid::Uuid::now_v7().to_string();
        self.conexao()
            .execute(
                "INSERT INTO fluxo_execucoes
                    (id, org_id, ficha_id, fluxo_id, revisao_id, status, no_atual, iniciada_em)
                 VALUES (?1, ?2, ?3, ?4, ?5, 'ativa', 'inicio', 1)",
                rusqlite::params![
                    execucao,
                    org.to_string(),
                    ficha.to_string(),
                    fluxo.to_string(),
                    revisao
                ],
            )
            .expect("execução ativa");
        execucao
    }
}

impl Drop for Cenario {
    fn drop(&mut self) {
        // Fecha o escritor antes de apagar a pasta.
        let _ = self.store.take();
        let _ = fs::remove_dir_all(&self.pasta);
    }
}

/// Erro de domínio de um resultado que deve ter falhado; qualquer outra coisa quebra o teste.
fn dominio<T: std::fmt::Debug>(resultado: Result<T, ErroStore>) -> DomainError {
    match resultado {
        Err(ErroStore::Dominio(erro)) => erro,
        outro => panic!("esperado erro de domínio, veio {outro:?}"),
    }
}

// ───────────────────────────── CRUD e If-Match ─────────────────────────────

#[test]
fn crud_de_fluxo_com_listagem_filtro_e_cursor() {
    let cenario = Cenario::novo("crud");
    let org = cenario.org("a");
    let store = cenario.store();

    let primeiro = cenario.fluxo(&org, "agendar_visita");
    assert_eq!(primeiro.status, Status::Ativo);
    assert_eq!(primeiro.revisao_atual, None);
    assert_eq!(primeiro.versao, 1);

    let obtido = store.fluxos().obter(org.id, primeiro.id).expect("obtém");
    assert_eq!(obtido, primeiro);
    let por_slug = store
        .fluxos()
        .por_slug(org.id, "agendar_visita")
        .expect("por slug");
    assert_eq!(por_slug, Some(primeiro.clone()));
    assert_eq!(
        store
            .fluxos()
            .por_slug(org.id, "nao_existe")
            .expect("por slug"),
        None
    );

    let segundo = cenario.fluxo(&org, "triagem_suporte");
    cenario.fluxo(&org, "confirmar_entrega");
    store
        .fluxos()
        .atualizar(
            org.id,
            segundo.id,
            1,
            MudancaFluxo {
                status: Some(Status::Inativo),
                ..MudancaFluxo::default()
            },
        )
        .expect("desativa");

    let ativos = store
        .fluxos()
        .listar(
            org.id,
            &FiltroFluxos {
                status: Some(Status::Ativo),
            },
            Pagina::default(),
        )
        .expect("lista ativos");
    assert_eq!(ativos.len(), 2);
    assert!(ativos.iter().all(|fluxo| fluxo.status == Status::Ativo));

    let todos = store
        .fluxos()
        .listar(org.id, &FiltroFluxos::default(), Pagina::default())
        .expect("lista todos");
    assert_eq!(todos.len(), 3);

    // Cursor: duas páginas de dois, sem repetir nem pular.
    let pagina_1 = store
        .fluxos()
        .listar(org.id, &FiltroFluxos::default(), Pagina::com_limite(2))
        .expect("página 1");
    assert_eq!(pagina_1.len(), 2);
    let apos: uuid::Uuid = pagina_1[1].id.to_string().parse().expect("uuid");
    let pagina_2 = store
        .fluxos()
        .listar(
            org.id,
            &FiltroFluxos::default(),
            Pagina {
                apos: Some(apos),
                limite: 2,
            },
        )
        .expect("página 2");
    assert_eq!(pagina_2.len(), 1);
    assert_eq!(pagina_2[0].id, todos[2].id);
}

#[test]
fn atualizar_usa_if_match_e_recusa_versao_velha() {
    let cenario = Cenario::novo("if-match");
    let org = cenario.org("a");
    let fluxo = cenario.fluxo(&org, "agendar_visita");
    let store = cenario.store();

    let atualizado = store
        .fluxos()
        .atualizar(
            org.id,
            fluxo.id,
            1,
            MudancaFluxo {
                nome: Some("Agendar visita técnica".to_owned()),
                rascunho_fonte: Some("fluxo(id = \"agendar_visita\")".to_owned()),
                ..MudancaFluxo::default()
            },
        )
        .expect("com a versão certa atualiza");
    assert_eq!(atualizado.versao, 2);
    assert_eq!(atualizado.nome, "Agendar visita técnica");
    assert_eq!(atualizado.rascunho_fonte, "fluxo(id = \"agendar_visita\")");

    // Versão 1 já foi substituída: a segunda escrita com ela não grava nada.
    let erro = dominio(store.fluxos().atualizar(
        org.id,
        fluxo.id,
        1,
        MudancaFluxo {
            nome: Some("Outro nome".to_owned()),
            ..MudancaFluxo::default()
        },
    ));
    assert!(
        matches!(
            erro,
            DomainError::VersionConflict {
                expected: 1,
                current: 2,
                ..
            }
        ),
        "erro inesperado: {erro:?}"
    );
    let depois = store.fluxos().obter(org.id, fluxo.id).expect("obtém");
    assert_eq!(
        depois.nome, "Agendar visita técnica",
        "a escrita recusada não gravou"
    );
    assert_eq!(depois.versao, 2);
}

#[test]
fn situacao_arquivada_e_recusada_no_fluxo() {
    let cenario = Cenario::novo("arquivado");
    let org = cenario.org("a");
    let fluxo = cenario.fluxo(&org, "agendar_visita");

    let erro = dominio(cenario.store().fluxos().atualizar(
        org.id,
        fluxo.id,
        1,
        MudancaFluxo {
            status: Some(Status::Arquivado),
            ..MudancaFluxo::default()
        },
    ));
    assert!(
        matches!(
            &erro,
            DomainError::Validation {
                field: "status",
                ..
            }
        ),
        "erro inesperado: {erro:?}"
    );
}

#[test]
fn slug_repetido_na_org_e_recusado_e_em_outra_org_e_aceito() {
    let cenario = Cenario::novo("slug");
    let org_a = cenario.org("a");
    let org_b = cenario.org("b");
    cenario.fluxo(&org_a, "agendar_visita");

    let erro = dominio(cenario.store().fluxos().criar(
        org_a.id,
        NovoFluxo {
            slug: "agendar_visita".to_owned(),
            nome: "Repetido".to_owned(),
            ..NovoFluxo::default()
        },
    ));
    assert!(
        matches!(&erro, DomainError::Validation { field: "slug", .. }),
        "erro inesperado: {erro:?}"
    );

    // Mesmo slug em outra org é outro fluxo.
    let outro = cenario.fluxo(&org_b, "agendar_visita");
    assert_eq!(outro.org_id, org_b.id);
}

#[test]
fn slug_fora_do_formato_da_linguagem_e_recusado() {
    let cenario = Cenario::novo("formato");
    let org = cenario.org("a");
    for slug in [
        "",
        "a",
        "Agendar",
        "1visita",
        "agendar-visita",
        "agendar visita",
    ] {
        let erro = dominio(cenario.store().fluxos().criar(
            org.id,
            NovoFluxo {
                slug: slug.to_owned(),
                nome: "Nome".to_owned(),
                ..NovoFluxo::default()
            },
        ));
        assert!(
            matches!(&erro, DomainError::Validation { field: "slug", .. }),
            "slug `{slug}` devia ser recusado, veio {erro:?}"
        );
    }
}

// ───────────────────────────── Exclusão ─────────────────────────────

#[test]
fn excluir_sem_execucao_ativa_devolve_zero_e_leva_a_permissao() {
    let cenario = Cenario::novo("excluir-simples");
    let org = cenario.org("a");
    let agente = cenario.agente(&org, "a");
    let fluxo = cenario.fluxo(&org, "agendar_visita");
    let store = cenario.store();
    store
        .fluxos()
        .liberar(org.id, agente.id, fluxo.id)
        .expect("libera");

    let ativas = store
        .fluxos()
        .excluir(org.id, fluxo.id, false)
        .expect("exclui sem execução ativa");
    assert_eq!(ativas, 0);
    assert!(matches!(
        dominio(store.fluxos().obter(org.id, fluxo.id)),
        DomainError::NotFound {
            entity: "fluxo",
            ..
        }
    ));
    assert!(
        store
            .fluxos()
            .listar_do_agente(org.id, agente.id)
            .expect("lista")
            .is_empty(),
        "a permissão do fluxo excluído tem que sair junto"
    );
    assert!(
        store.agentes().obter(org.id, agente.id).is_ok(),
        "excluir fluxo não apaga agente"
    );
}

#[test]
fn excluir_com_execucao_ativa_exige_force_e_abandona_mantendo_o_historico() {
    let cenario = Cenario::novo("excluir-ativa");
    let org = cenario.org("a");
    let agente = cenario.agente(&org, "a");
    let fluxo = cenario.fluxo(&org, "agendar_visita");
    let store = cenario.store();
    store
        .fluxos()
        .liberar(org.id, agente.id, fluxo.id)
        .expect("libera");
    let conversa = cenario.conversa(&org, &agente);
    let ficha = store
        .fichas()
        .por_conversa(org.id, conversa.id)
        .expect("consulta")
        .expect("a conversa tem Ficha")
        .id;
    let execucao = cenario.execucao_ativa(org.id, ficha, fluxo.id);

    // Sem force: recusa com a contagem, e nada muda.
    let erro = dominio(store.fluxos().excluir(org.id, fluxo.id, false));
    assert!(
        matches!(&erro, DomainError::Conflict { entity: "fluxo", reason, .. } if reason.contains("1 execução")),
        "erro inesperado: {erro:?}"
    );
    store
        .fluxos()
        .obter(org.id, fluxo.id)
        .expect("o fluxo continua existindo depois da recusa");
    let situacao_antes: String = cenario
        .conexao()
        .query_row(
            "SELECT status FROM fluxo_execucoes WHERE id = ?1",
            [&execucao],
            |linha| linha.get(0),
        )
        .expect("lê a execução");
    assert_eq!(situacao_antes, "ativa");

    // Com force: devolve a contagem, abandona a execução e tira o fluxo.
    let ativas = store
        .fluxos()
        .excluir(org.id, fluxo.id, true)
        .expect("exclui com force");
    assert_eq!(ativas, 1, "devolve quantas execuções ativas havia");
    assert!(matches!(
        dominio(store.fluxos().obter(org.id, fluxo.id)),
        DomainError::NotFound { .. }
    ));

    let conn = cenario.conexao();
    let (status, motivo, finalizada): (String, Option<String>, Option<i64>) = conn
        .query_row(
            "SELECT status, motivo_fim, finalizada_em FROM fluxo_execucoes WHERE id = ?1",
            [&execucao],
            |linha| Ok((linha.get(0)?, linha.get(1)?, linha.get(2)?)),
        )
        .expect("a execução continua no banco: é histórico do atendimento");
    assert_eq!(status, "abandonada");
    assert_eq!(motivo.as_deref(), Some(MOTIVO_EXCLUSAO_FLUXO));
    assert!(
        finalizada.is_some(),
        "execução encerrada tem instante de fim"
    );

    // A Ficha e a linha do tempo ficam, com o abandono registrado.
    store
        .fichas()
        .obter(org.id, ficha)
        .expect("a Ficha permanece");
    let eventos = store
        .fichas()
        .eventos(org.id, ficha, None, 100)
        .expect("eventos");
    let abandono = eventos
        .iter()
        .find(|evento| evento.tipo == RecordEventKind::ExecutionAbandoned)
        .expect("o abandono entra na linha do tempo da Ficha");
    assert_eq!(abandono.ator_tipo, EventActor::System);
    assert_eq!(
        abandono.execucao_id.map(|id| id.to_string()),
        Some(execucao.clone())
    );
    assert!(abandono.payload_json.contains(MOTIVO_EXCLUSAO_FLUXO));

    assert!(
        store
            .fluxos()
            .listar_do_agente(org.id, agente.id)
            .expect("lista")
            .is_empty()
    );
}

#[test]
fn excluir_fluxo_inexistente_ou_de_outra_org_e_nao_encontrado() {
    let cenario = Cenario::novo("excluir-ausente");
    let org_a = cenario.org("a");
    let org_b = cenario.org("b");
    let fluxo_b = cenario.fluxo(&org_b, "agendar_visita");

    let erro = dominio(cenario.store().fluxos().excluir(org_a.id, fluxo_b.id, true));
    assert!(matches!(
        erro,
        DomainError::NotFound {
            entity: "fluxo",
            ..
        }
    ));
    // O fluxo da outra org continua lá.
    cenario
        .store()
        .fluxos()
        .obter(org_b.id, fluxo_b.id)
        .expect("intacto");
}

// ───────────────────────────── Permissão e teto ─────────────────────────────

#[test]
fn teto_de_fluxos_por_agente_recusa_o_21o_e_repetir_nao_conta() {
    let cenario = Cenario::novo("teto");
    let org = cenario.org("a");
    let agente = cenario.agente(&org, "a");
    let store = cenario.store();
    let fluxos: Vec<Fluxo> = (0..21)
        .map(|n| cenario.fluxo(&org, &format!("fluxo_{n:02}")))
        .collect();

    for fluxo in &fluxos[..20] {
        store
            .fluxos()
            .liberar(org.id, agente.id, fluxo.id)
            .expect("até o 20º libera");
    }
    // Repetir um já liberado é no-op e não estoura o teto.
    store
        .fluxos()
        .liberar(org.id, agente.id, fluxos[0].id)
        .expect("repetir não conta de novo");
    assert_eq!(
        store
            .fluxos()
            .listar_do_agente(org.id, agente.id)
            .expect("lista")
            .len(),
        20
    );

    let erro = dominio(store.fluxos().liberar(org.id, agente.id, fluxos[20].id));
    assert_eq!(
        erro,
        DomainError::FlowLimitPerAgent {
            agent_id: agente.id.to_string(),
            limit: TETO_FLUXOS_POR_AGENTE,
            requested: 21,
        }
    );
    assert_eq!(erro.api_code(), "teto_de_fluxos_por_agente");
    assert_eq!(erro.http_status(), 409);
    assert_eq!(
        store
            .fluxos()
            .listar_do_agente(org.id, agente.id)
            .expect("lista")
            .len(),
        20,
        "o 21º não entrou"
    );
}

#[test]
fn substituir_lista_com_21_recusa_e_mantem_a_lista_antiga() {
    let cenario = Cenario::novo("teto-lote");
    let org = cenario.org("a");
    let agente = cenario.agente(&org, "a");
    let store = cenario.store();
    let antigo = cenario.fluxo(&org, "fluxo_antigo");
    store
        .fluxos()
        .liberar(org.id, agente.id, antigo.id)
        .expect("libera o antigo");
    let novos: Vec<FlowId> = (0..21)
        .map(|n| cenario.fluxo(&org, &format!("fluxo_{n:02}")).id)
        .collect();
    let versao = cenario.versao_agente(&org, &agente);

    let erro = dominio(
        store
            .fluxos()
            .substituir_lista(org.id, agente.id, versao, &novos),
    );
    assert!(
        matches!(
            erro,
            DomainError::FlowLimitPerAgent {
                limit: 20,
                requested: 21,
                ..
            }
        ),
        "erro inesperado: {erro:?}"
    );
    let lista = store
        .fluxos()
        .listar_do_agente(org.id, agente.id)
        .expect("lista");
    assert_eq!(lista.len(), 1, "a lista antiga continua inteira");
    assert_eq!(lista[0].id, antigo.id);
    assert_eq!(
        cenario.versao_agente(&org, &agente),
        versao,
        "versão não avançou"
    );
}

#[test]
fn substituir_lista_e_atomica_e_respeita_if_match_do_agente() {
    let cenario = Cenario::novo("substituir");
    let org = cenario.org("a");
    let agente = cenario.agente(&org, "a");
    let store = cenario.store();
    let a = cenario.fluxo(&org, "fluxo_a");
    let b = cenario.fluxo(&org, "fluxo_b");
    let c = cenario.fluxo(&org, "fluxo_c");

    let v1 = cenario.versao_agente(&org, &agente);
    // Repetido na lista conta uma vez.
    store
        .fluxos()
        .substituir_lista(org.id, agente.id, v1, &[a.id, b.id, a.id])
        .expect("troca a lista");
    let v2 = cenario.versao_agente(&org, &agente);
    assert_eq!(v2, v1 + 1, "troca de permissão avança a versão do agente");
    let lista: Vec<FlowId> = store
        .fluxos()
        .listar_do_agente(org.id, agente.id)
        .expect("lista")
        .into_iter()
        .map(|fluxo| fluxo.id)
        .collect();
    assert_eq!(lista.len(), 2);
    assert!(lista.contains(&a.id) && lista.contains(&b.id));

    // Idempotente: a mesma lista de novo deixa o mesmo estado.
    store
        .fluxos()
        .substituir_lista(org.id, agente.id, v2, &[a.id, b.id])
        .expect("mesma lista de novo");
    let v3 = cenario.versao_agente(&org, &agente);

    // If-Match velho (v1) recusa e não troca nada.
    let erro = dominio(
        store
            .fluxos()
            .substituir_lista(org.id, agente.id, v1, &[c.id]),
    );
    assert!(
        matches!(erro, DomainError::VersionConflict { .. }),
        "erro inesperado: {erro:?}"
    );
    assert_eq!(cenario.versao_agente(&org, &agente), v3);
    assert_eq!(
        store
            .fluxos()
            .listar_do_agente(org.id, agente.id)
            .expect("lista")
            .len(),
        2
    );
}

#[test]
fn liberar_e_revogar_avancam_a_versao_do_agente_e_revogar_e_idempotente() {
    let cenario = Cenario::novo("versao-agente");
    let org = cenario.org("a");
    let agente = cenario.agente(&org, "a");
    let fluxo = cenario.fluxo(&org, "agendar_visita");
    let store = cenario.store();
    let v0 = cenario.versao_agente(&org, &agente);

    store
        .fluxos()
        .liberar(org.id, agente.id, fluxo.id)
        .expect("libera");
    let v1 = cenario.versao_agente(&org, &agente);
    assert_eq!(v1, v0 + 1);

    store
        .fluxos()
        .revogar(org.id, agente.id, fluxo.id)
        .expect("revoga");
    let v2 = cenario.versao_agente(&org, &agente);
    assert_eq!(v2, v1 + 1);

    // Revogar o que já não estava liberado não mexe em nada.
    store
        .fluxos()
        .revogar(org.id, agente.id, fluxo.id)
        .expect("revogar de novo não falha");
    assert_eq!(cenario.versao_agente(&org, &agente), v2);
}

#[test]
fn listar_agentes_do_fluxo_devolve_quem_tem_a_permissao() {
    let cenario = Cenario::novo("agentes-do-fluxo");
    let org = cenario.org("a");
    let agente_1 = cenario.agente(&org, "um");
    let agente_2 = cenario.agente(&org, "dois");
    let fluxo = cenario.fluxo(&org, "agendar_visita");
    let store = cenario.store();
    store
        .fluxos()
        .liberar(org.id, agente_1.id, fluxo.id)
        .expect("libera 1");
    store
        .fluxos()
        .liberar(org.id, agente_2.id, fluxo.id)
        .expect("libera 2");

    let mut quem: Vec<AgentId> = store
        .fluxos()
        .listar_agentes_do_fluxo(org.id, fluxo.id)
        .expect("lista");
    quem.sort();
    let mut esperados = vec![agente_1.id, agente_2.id];
    esperados.sort();
    assert_eq!(quem, esperados);
}

#[test]
fn permissao_com_fluxo_ou_agente_de_outra_org_e_recusada() {
    let cenario = Cenario::novo("org-errada");
    let org_a = cenario.org("a");
    let org_b = cenario.org("b");
    let agente_a = cenario.agente(&org_a, "a");
    let agente_b = cenario.agente(&org_b, "b");
    let fluxo_a = cenario.fluxo(&org_a, "fluxo_a");
    let fluxo_b = cenario.fluxo(&org_b, "fluxo_b");
    let store = cenario.store();

    // Fluxo de outra org.
    let erro = dominio(store.fluxos().liberar(org_a.id, agente_a.id, fluxo_b.id));
    assert!(
        matches!(
            &erro,
            DomainError::NotFound {
                entity: "fluxo",
                ..
            }
        ),
        "erro inesperado: {erro:?}"
    );
    // Agente de outra org.
    let erro = dominio(store.fluxos().liberar(org_a.id, agente_b.id, fluxo_a.id));
    assert!(
        matches!(
            &erro,
            DomainError::NotFound {
                entity: "agente",
                ..
            }
        ),
        "erro inesperado: {erro:?}"
    );
    // Fluxo inexistente.
    let erro = dominio(
        store
            .fluxos()
            .liberar(org_a.id, agente_a.id, FlowId::generate()),
    );
    assert!(matches!(
        erro,
        DomainError::NotFound {
            entity: "fluxo",
            ..
        }
    ));

    // Na troca em lote, um fluxo de outra org no meio derruba o lote inteiro: a lista antiga fica.
    store
        .fluxos()
        .liberar(org_a.id, agente_a.id, fluxo_a.id)
        .expect("libera o próprio");
    let versao = cenario.versao_agente(&org_a, &agente_a);
    let erro = dominio(store.fluxos().substituir_lista(
        org_a.id,
        agente_a.id,
        versao,
        &[fluxo_b.id],
    ));
    assert!(matches!(
        &erro,
        DomainError::NotFound {
            entity: "fluxo",
            ..
        }
    ));
    let lista = store
        .fluxos()
        .listar_do_agente(org_a.id, agente_a.id)
        .expect("lista");
    assert_eq!(lista.len(), 1, "a lista antiga não foi apagada");
    assert_eq!(lista[0].id, fluxo_a.id);
    assert_eq!(
        cenario.versao_agente(&org_a, &agente_a),
        versao,
        "a versão do agente também não avançou"
    );

    // Listar quem tem um fluxo de outra org não devolve nada: é como se não existisse.
    let erro = dominio(store.fluxos().listar_agentes_do_fluxo(org_a.id, fluxo_b.id));
    assert!(matches!(erro, DomainError::NotFound { .. }));
}

#[test]
fn apagar_agente_remove_so_a_permissao() {
    let cenario = Cenario::novo("apaga-agente");
    let org = cenario.org("a");
    let agente = cenario.agente(&org, "a");
    let fluxo = cenario.fluxo(&org, "agendar_visita");
    let store = cenario.store();
    store
        .fluxos()
        .liberar(org.id, agente.id, fluxo.id)
        .expect("libera");

    cenario
        .conexao()
        .execute(
            "DELETE FROM agents WHERE org_id = ?1 AND id = ?2",
            rusqlite::params![org.id.to_string(), agente.id.to_string()],
        )
        .expect("apaga o agente");

    let ainda = store
        .fluxos()
        .obter(org.id, fluxo.id)
        .expect("o fluxo continua existindo");
    assert_eq!(ainda.id, fluxo.id);
    let permissoes: i64 = cenario
        .conexao()
        .query_row(
            "SELECT COUNT(*) FROM agente_fluxos WHERE org_id = ?1 AND agent_id = ?2",
            rusqlite::params![org.id.to_string(), agente.id.to_string()],
            |linha| linha.get(0),
        )
        .expect("conta");
    assert_eq!(permissoes, 0, "apagar o agente leva a permissão");
}

#[test]
fn excluir_fluxo_leva_as_revisoes_junto() {
    // A revisão sai com o fluxo pela cascata que o gatilho de imutabilidade permite (a revisão só é
    // protegida quando o fluxo continua de pé).
    let cenario = Cenario::novo("revisao-cascata");
    let org = cenario.org("a");
    let fluxo = cenario.fluxo(&org, "agendar_visita");
    let conn = cenario.conexao();
    conn.execute(
        "INSERT INTO fluxo_revisoes
            (id, org_id, fluxo_id, numero, fonte, fonte_sha256, definicao_json, publicada_em,
             publicada_por)
         VALUES (?1, ?2, ?3, 1, 'fluxo(id = \"agendar_visita\")', 'sha', '{}', 1, 'cli')",
        rusqlite::params![
            uuid::Uuid::now_v7().to_string(),
            org.id.to_string(),
            fluxo.id.to_string()
        ],
    )
    .expect("revisão");

    cenario
        .store()
        .fluxos()
        .excluir(org.id, fluxo.id, false)
        .expect("exclui: a revisão sai junto com o fluxo");
    let restantes: i64 = conn
        .query_row("SELECT COUNT(*) FROM fluxo_revisoes", [], |linha| {
            linha.get(0)
        })
        .expect("conta");
    assert_eq!(restantes, 0);
}
