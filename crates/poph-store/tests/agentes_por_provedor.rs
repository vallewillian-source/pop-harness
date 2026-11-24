use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use poph_store::{MudancaAgente, NovaOrg, NovoAgente, Store, agentes_por_provedor_em};
use poph_types::OrgId;

struct Cenario {
    pasta: PathBuf,
    store: Option<Store>,
}

impl Cenario {
    fn novo(rotulo: &str) -> Self {
        static CONTADOR: AtomicUsize = AtomicUsize::new(0);
        let n = CONTADOR.fetch_add(1, Ordering::Relaxed);
        let pasta = std::env::temp_dir().join(format!(
            "poph-agentes-provedor-{rotulo}-{}-{n}",
            std::process::id()
        ));
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
        self.store.as_ref().expect("banco aberto")
    }

    fn org(&self) -> OrgId {
        self.store()
            .orgaos()
            .criar(NovaOrg {
                nome: "Org provedores".to_owned(),
                slug: format!("org-provedores-{}", std::process::id()),
                config_json: "{}".to_owned(),
            })
            .expect("cria a org")
            .id
    }

    fn agente(&self, org: OrgId, nome: &str) -> poph_types::AgentId {
        self.store()
            .agentes()
            .criar(
                org,
                NovoAgente {
                    nome: nome.to_owned(),
                    slug: nome.to_lowercase().replace(' ', "-"),
                    ..NovoAgente::default()
                },
            )
            .expect("cria o agente")
            .id
    }

    fn trocar_provedor(&self, org: OrgId, id: poph_types::AgentId, provedor: &str) {
        let agente = self.store().agentes().obter(org, id).expect("lê o agente");
        self.store()
            .agentes()
            .atualizar(
                org,
                id,
                agente.versao,
                MudancaAgente {
                    provedor: Some(provedor.to_owned()),
                    ..MudancaAgente::default()
                },
            )
            .expect("troca o provedor");
    }
}

impl Drop for Cenario {
    fn drop(&mut self) {
        self.store = None;
        let _ = fs::remove_dir_all(&self.pasta);
    }
}

#[test]
fn conta_agentes_por_provedor_sem_criar_nada() {
    let cenario = Cenario::novo("contagem");
    let org = cenario.org();
    let primeiro = cenario.agente(org, "Ana");
    let segundo = cenario.agente(org, "Bruno");
    let terceiro = cenario.agente(org, "Carla");
    cenario.trocar_provedor(org, segundo, "openai-forte");
    cenario.trocar_provedor(org, terceiro, "openai-forte");

    let caminho = cenario.pasta.join("harness.db");
    let contagem = agentes_por_provedor_em(&caminho).expect("conta os agentes");
    assert_eq!(
        contagem,
        vec![
            ("antigravity2api".to_owned(), 1),
            ("openai-forte".to_owned(), 2),
        ]
    );
    assert_eq!(
        cenario
            .store()
            .agentes()
            .obter(org, primeiro)
            .expect("o agente continua lá")
            .provedor,
        "antigravity2api"
    );
}

#[test]
fn banco_ausente_e_erro_e_nao_cria_arquivo() {
    let pasta = std::env::temp_dir().join(format!(
        "poph-agentes-provedor-sem-banco-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&pasta);
    fs::create_dir_all(&pasta).expect("cria a pasta do teste");
    let caminho = pasta.join("harness.db");
    assert!(agentes_por_provedor_em(&caminho).is_err());
    assert!(!caminho.exists(), "não pode criar o banco");
    let _ = fs::remove_dir_all(&pasta);
}
