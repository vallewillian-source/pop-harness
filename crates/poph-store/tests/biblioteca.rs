//! Biblioteca de arquivos do agente (HR-24) na camada de armazenamento.
//!
//! Cobre o que o repositório promete: `org_id` e `agent_id` obrigatórios, isolamento entre orgs e
//! entre agentes da mesma org, validação dos campos, contagem e exclusão.

use poph_store::{Biblioteca, NewAgent, NewLibraryFile, NewOrg, Page, Store, StoreError};
use poph_types::{AgentId, FileId, OrgId};

struct Cenario {
    pasta: std::path::PathBuf,
    store: Store,
    org: OrgId,
    outra_org: OrgId,
    agente: AgentId,
    outro_agente: AgentId,
}

impl Cenario {
    fn novo() -> Self {
        let pasta = std::env::temp_dir().join(format!(
            "poph-biblioteca-store-{}-{}",
            std::process::id(),
            AgentId::generate()
        ));
        let _ = std::fs::remove_dir_all(&pasta);
        std::fs::create_dir_all(&pasta).expect("cria a pasta");
        let store =
            Store::open_at(&pasta.join("harness.db"), &pasta.join("rag.db")).expect("abre o banco");
        let org = criar_org(&store, "org-a");
        let outra_org = criar_org(&store, "org-b");
        let agente = criar_agente(&store, org, "bia");
        let outro_agente = criar_agente(&store, org, "caio");
        Self {
            pasta,
            store,
            org,
            outra_org,
            agente,
            outro_agente,
        }
    }

    fn biblioteca(&self) -> Biblioteca<'_> {
        self.store.library()
    }

    fn novo_arquivo(&self, agente: AgentId, nome: &str) -> NewLibraryFile {
        NewLibraryFile {
            agent_id: agente,
            name: nome.to_owned(),
            description: "descrição".to_owned(),
            mime: "application/pdf".to_owned(),
            size_bytes: 1_024,
            sha256: "a".repeat(64),
            blob_path: format!("{}/{nome}", self.org),
        }
    }
}

impl Drop for Cenario {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.pasta);
    }
}

fn criar_org(store: &Store, slug: &str) -> OrgId {
    store
        .orgs()
        .criar(NewOrg {
            name: format!("Org {slug}"),
            slug: format!("{slug}-{}", AgentId::generate()),
            config_json: "{}".to_owned(),
        })
        .expect("cria a org")
        .id
}

fn criar_agente(store: &Store, org: OrgId, slug: &str) -> AgentId {
    store
        .agents()
        .criar(
            org,
            NewAgent {
                name: format!("Agente {slug}"),
                slug: format!("{slug}-{}", &AgentId::generate().to_string()[..8]),
                ..NewAgent::default()
            },
        )
        .expect("cria o agente")
        .id
}

#[test]
fn cria_lista_conta_e_exclui() {
    let cenario = Cenario::novo();
    let arquivo = cenario
        .biblioteca()
        .criar(cenario.org, cenario.novo_arquivo(cenario.agente, "a.pdf"))
        .expect("cria o arquivo");
    assert_eq!(arquivo.name, "a.pdf");
    assert_eq!(arquivo.size_bytes, 1_024);
    assert_eq!(
        cenario
            .biblioteca()
            .contar(cenario.org, cenario.agente)
            .expect("conta"),
        1
    );
    let lista = cenario
        .biblioteca()
        .listar(cenario.org, cenario.agente, Page::default())
        .expect("lista");
    assert_eq!(lista.len(), 1);
    assert_eq!(lista[0].id, arquivo.id);

    cenario
        .biblioteca()
        .excluir(cenario.org, cenario.agente, arquivo.id)
        .expect("exclui");
    assert_eq!(
        cenario
            .biblioteca()
            .contar(cenario.org, cenario.agente)
            .expect("conta"),
        0
    );
    let erro = cenario
        .biblioteca()
        .obter(cenario.org, cenario.agente, arquivo.id)
        .expect_err("não existe mais");
    assert!(matches!(
        erro,
        StoreError::Domain(poph_types::DomainError::NotFound { .. })
    ));
}

#[test]
fn outro_agente_da_mesma_org_nao_alcanca_o_arquivo() {
    let cenario = Cenario::novo();
    let arquivo = cenario
        .biblioteca()
        .criar(cenario.org, cenario.novo_arquivo(cenario.agente, "a.pdf"))
        .expect("cria");
    let erro = cenario
        .biblioteca()
        .obter(cenario.org, cenario.outro_agente, arquivo.id)
        .expect_err("outro agente não vê");
    assert!(matches!(
        erro,
        StoreError::Domain(poph_types::DomainError::NotFound { .. })
    ));
    let erro = cenario
        .biblioteca()
        .excluir(cenario.org, cenario.outro_agente, arquivo.id)
        .expect_err("outro agente não apaga");
    assert!(matches!(
        erro,
        StoreError::Domain(poph_types::DomainError::NotFound { .. })
    ));
    // E o dono continua com o arquivo.
    assert_eq!(
        cenario
            .biblioteca()
            .contar(cenario.org, cenario.agente)
            .expect("conta"),
        1
    );
}

#[test]
fn org_errada_nao_alcanca_o_arquivo() {
    let cenario = Cenario::novo();
    let arquivo = cenario
        .biblioteca()
        .criar(cenario.org, cenario.novo_arquivo(cenario.agente, "a.pdf"))
        .expect("cria");
    let erro = cenario
        .biblioteca()
        .obter(cenario.outra_org, cenario.agente, arquivo.id)
        .expect_err("org errada não vê");
    assert!(matches!(
        erro,
        StoreError::Domain(poph_types::DomainError::NotFound { .. })
    ));
    assert_eq!(
        cenario
            .biblioteca()
            .contar(cenario.outra_org, cenario.agente)
            .expect("conta"),
        0
    );
}

#[test]
fn agente_de_outra_org_nao_recebe_arquivo() {
    let cenario = Cenario::novo();
    // O agente é da org A: criar o arquivo apontando a org B não passa.
    let erro = cenario
        .biblioteca()
        .criar(
            cenario.outra_org,
            cenario.novo_arquivo(cenario.agente, "a.pdf"),
        )
        .expect_err("agente de outra org é recusado");
    assert!(
        matches!(
            erro,
            StoreError::Domain(poph_types::DomainError::NotFound { .. })
        ),
        "{erro:?}"
    );
}

#[test]
fn campos_invalidos_dao_erro_de_validacao() {
    let cenario = Cenario::novo();
    let mut novo = cenario.novo_arquivo(cenario.agente, " ");
    let erro = cenario
        .biblioteca()
        .criar(cenario.org, novo.clone())
        .expect_err("nome vazio");
    assert!(matches!(
        erro,
        StoreError::Domain(poph_types::DomainError::Validation { field: "nome", .. })
    ));

    novo = cenario.novo_arquivo(cenario.agente, "a.pdf");
    novo.sha256 = "xyz".to_owned();
    let erro = cenario
        .biblioteca()
        .criar(cenario.org, novo)
        .expect_err("sha inválido");
    assert!(matches!(
        erro,
        StoreError::Domain(poph_types::DomainError::Validation {
            field: "sha256",
            ..
        })
    ));

    let mut novo = cenario.novo_arquivo(cenario.agente, "a.pdf");
    novo.description = "d".repeat(poph_store::LIMITE_DESCRICAO_ARQUIVO + 1);
    let erro = cenario
        .biblioteca()
        .criar(cenario.org, novo)
        .expect_err("descrição longa");
    assert!(matches!(
        erro,
        StoreError::Domain(poph_types::DomainError::Validation {
            field: "descrição",
            ..
        })
    ));

    // Id inválido na leitura.
    let invalido: Result<FileId, _> = "nao-e-uuid".parse();
    assert!(invalido.is_err());
}
