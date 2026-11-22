//! Mensagem de erro de escrita com o banco de verdade: cada restrição do SQLite vira o erro
//! certo para quem chamou (duplicidade, campo fora da faixa), e o campo é o da API.
//!
//! Sem servidor: o repositório é chamado direto, num banco em pasta temporária.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use poph_store::{ErroStore, MudancaAgente, NovaOrg, NovoAgente, Org, Store};
use poph_types::DomainError;

struct Cenario {
    pasta: PathBuf,
    store: Option<Store>,
}

impl Cenario {
    fn novo(rotulo: &str) -> Self {
        static CONTADOR: AtomicUsize = AtomicUsize::new(0);
        let n = CONTADOR.fetch_add(1, Ordering::Relaxed);
        let pasta =
            std::env::temp_dir().join(format!("poph-erro-{rotulo}-{}-{n}", std::process::id()));
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
        let _ = self.store.take();
        let _ = fs::remove_dir_all(&self.pasta);
    }
}

fn criar_org(store: &Store) -> Org {
    store
        .orgaos()
        .criar(NovaOrg {
            nome: "Org de erro".to_owned(),
            slug: "org-erro".to_owned(),
            config_json: String::new(),
        })
        .expect("cria a org")
}

fn novo_agente(slug: &str) -> NovoAgente {
    NovoAgente {
        nome: format!("Agente {slug}"),
        slug: slug.to_owned(),
        ..NovoAgente::default()
    }
}

/// Espera erro de validação de um campo e devolve (campo, motivo).
fn validacao(erro: ErroStore) -> (&'static str, String) {
    match erro {
        ErroStore::Dominio(DomainError::Validacao { campo, motivo }) => (campo, motivo),
        outro => panic!("esperava validação de campo, veio {outro:?}"),
    }
}

#[test]
fn slug_repetido_no_agente_continua_duplicidade() {
    let cenario = Cenario::novo("duplicado");
    let store = cenario.store();
    let org = criar_org(store);
    store
        .agentes()
        .criar(org.id, novo_agente("beatriz"))
        .expect("cria o primeiro agente");

    let (campo, motivo) = validacao(
        store
            .agentes()
            .criar(org.id, novo_agente("beatriz"))
            .expect_err("slug repetido devia ser recusado"),
    );
    assert_eq!(campo, "slug");
    assert_eq!(motivo, "já existe um registro com esse valor nesta org");
}

#[test]
fn contexto_fora_da_faixa_no_patch_do_agente_nomeia_o_campo_certo() {
    let cenario = Cenario::novo("faixa-contexto");
    let store = cenario.store();
    let org = criar_org(store);
    let agente = store
        .agentes()
        .criar(org.id, novo_agente("beatriz"))
        .expect("cria o agente");

    // O banco exige max_context_tokens >= 1000. Valor 1 não é duplicidade de slug.
    let (campo, motivo) = validacao(
        store
            .agentes()
            .atualizar(
                org.id,
                agente.id,
                agente.versao,
                MudancaAgente {
                    max_context_tokens: Some(1),
                    ..MudancaAgente::default()
                },
            )
            .expect_err("contexto abaixo da faixa devia ser recusado"),
    );
    assert_eq!(campo, "max_context_tokens");
    assert_eq!(
        motivo,
        "valor fora da faixa ou da lista permitida para este campo"
    );
}

#[test]
fn limite_de_ferramentas_fora_da_faixa_nomeia_o_campo_da_api() {
    let cenario = Cenario::novo("faixa-ferramentas");
    let store = cenario.store();
    let org = criar_org(store);
    let agente = store
        .agentes()
        .criar(org.id, novo_agente("beatriz"))
        .expect("cria o agente");

    // O banco aceita 0 a 100 chamadas de ferramenta por mensagem.
    let (campo, _) = validacao(
        store
            .agentes()
            .atualizar(
                org.id,
                agente.id,
                agente.versao,
                MudancaAgente {
                    max_ferramentas_por_mensagem: Some(101),
                    ..MudancaAgente::default()
                },
            )
            .expect_err("101 chamadas devia ser recusado"),
    );
    assert_eq!(campo, "max_tool_calls");
}

#[test]
fn faixa_recusada_nao_grava_nada() {
    let cenario = Cenario::novo("nada-gravado");
    let store = cenario.store();
    let org = criar_org(store);
    let agente = store
        .agentes()
        .criar(org.id, novo_agente("beatriz"))
        .expect("cria o agente");

    let _ = store.agentes().atualizar(
        org.id,
        agente.id,
        agente.versao,
        MudancaAgente {
            max_context_tokens: Some(1),
            ..MudancaAgente::default()
        },
    );
    let depois = store
        .agentes()
        .obter(org.id, agente.id)
        .expect("lê o agente");
    assert_eq!(depois.max_context_tokens, agente.max_context_tokens);
    assert_eq!(depois.versao, agente.versao);
}
