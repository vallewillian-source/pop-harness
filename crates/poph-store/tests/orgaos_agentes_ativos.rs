//! Contagem de agentes ativos por org (CF-03a): a coluna "agentes ativos" da listagem de orgs.
//!
//! O que fica provado aqui:
//!
//! - org com 3 agentes ativos e 1 inativo conta **3** (inativo e arquivado ficam de fora);
//! - org sem agente não aparece no mapa — quem chama trata a ausência como zero;
//! - a contagem de uma página inteira sai de **uma** consulta, não de uma por org
//!   ([`Store::leituras_feitas`]);
//! - org de fora da lista pedida não entra no resultado.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use poph_store::{AgentChange, NewAgent, NewOrg, Org, Status, Store};
use poph_types::OrgId;

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
            std::env::temp_dir().join(format!("poph-orgaos-{rotulo}-{}-{n}", std::process::id()));
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

    fn org(&self, slug: &str) -> Org {
        self.store()
            .orgs()
            .criar(NewOrg {
                name: format!("Org {slug}"),
                slug: slug.to_owned(),
                config_json: String::new(),
            })
            .expect("cria a org")
    }

    fn agente(&self, org: &Org, slug: &str) -> poph_store::Agente {
        self.store()
            .agents()
            .criar(
                org.id,
                NewAgent {
                    name: format!("Agente {slug}"),
                    slug: slug.to_owned(),
                    ..NewAgent::default()
                },
            )
            .expect("cria o agente")
    }

    /// Muda a situação do agente pelo repositório (é o caminho da API, com `If-Match`).
    fn situacao(&self, org: &Org, agente: &poph_store::Agente, status: Status) {
        let atual = self
            .store()
            .agents()
            .get(org.id, agente.id)
            .expect("lê o agente");
        self.store()
            .agents()
            .atualizar(
                org.id,
                agente.id,
                atual.version,
                AgentChange {
                    status: Some(status),
                    ..AgentChange::default()
                },
            )
            .expect("muda a situação");
    }
}

impl Drop for Cenario {
    fn drop(&mut self) {
        self.store = None;
        let _ = fs::remove_dir_all(&self.pasta);
    }
}

/// 3 ativos + 1 inativo + 1 arquivado contam 3; org sem agente fica fora do mapa.
#[test]
fn conta_so_agente_ativo_e_omite_org_sem_agente() {
    let cenario = Cenario::novo("contagem");
    let com_agentes = cenario.org("com-agentes");
    let sem_agentes = cenario.org("sem-agentes");

    for slug in ["um", "dois", "tres"] {
        cenario.agente(&com_agentes, slug);
    }
    let inativo = cenario.agente(&com_agentes, "quatro");
    cenario.situacao(&com_agentes, &inativo, Status::Inactive);
    let arquivado = cenario.agente(&com_agentes, "cinco");
    cenario.situacao(&com_agentes, &arquivado, Status::Archived);

    let contagem = cenario
        .store()
        .agents()
        .contar_ativos_por_org(&[com_agentes.id, sem_agentes.id])
        .expect("conta os ativos");

    assert_eq!(
        contagem.get(&com_agentes.id).copied(),
        Some(3),
        "inativo e arquivado não podem entrar na conta: {contagem:?}"
    );
    assert_eq!(
        contagem.get(&sem_agentes.id),
        None,
        "org sem agente ativo não aparece no mapa (quem chama usa 0): {contagem:?}"
    );
    assert_eq!(contagem.len(), 1);
}

/// Uma página de orgs custa **uma** leitura, não uma por linha (a mesma regra do RP-01).
#[test]
fn contagem_da_pagina_e_uma_consulta_so() {
    let cenario = Cenario::novo("uma-consulta");
    let mut ids: Vec<OrgId> = Vec::new();
    for indice in 0..5 {
        let org = cenario.org(&format!("org-{indice}"));
        if indice % 2 == 0 {
            cenario.agente(&org, "a");
        }
        ids.push(org.id);
    }

    let antes = cenario.store().reads_done();
    let contagem = cenario
        .store()
        .agents()
        .contar_ativos_por_org(&ids)
        .expect("conta os ativos");
    let depois = cenario.store().reads_done();

    assert_eq!(contagem.len(), 3, "três orgs com um agente ativo cada");
    assert_eq!(
        depois - antes,
        1,
        "a contagem da página inteira tem de custar uma leitura, não uma por org"
    );

    // Lista vazia não vai ao banco.
    let antes = cenario.store().reads_done();
    let vazia = cenario
        .store()
        .agents()
        .contar_ativos_por_org(&[])
        .expect("lista vazia");
    assert!(vazia.is_empty());
    assert_eq!(cenario.store().reads_done(), antes);
}

/// Org fora da lista pedida não entra no resultado: a consulta é fechada na página.
#[test]
fn so_conta_as_orgs_pedidas() {
    let cenario = Cenario::novo("fechada-na-pagina");
    let pedida = cenario.org("pedida");
    let outra = cenario.org("outra");
    cenario.agente(&pedida, "a");
    cenario.agente(&outra, "b");
    cenario.agente(&outra, "c");

    let contagem = cenario
        .store()
        .agents()
        .contar_ativos_por_org(&[pedida.id])
        .expect("conta os ativos");

    assert_eq!(contagem.get(&pedida.id).copied(), Some(1));
    assert_eq!(
        contagem.get(&outra.id),
        None,
        "a org fora da página não pode aparecer: {contagem:?}"
    );
}
