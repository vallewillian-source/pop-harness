//! Transferência da conversa para humano na camada de armazenamento (HR-29).
//!
//! O que o repositório promete: a situação vira `transferred`, a versão sobe (então o `If-Match` de
//! quem edita continua valendo) e a auditoria é gravada **na mesma transação**, com ator do sistema.
//! Transferir conversa de outra org não acha nada e não escreve auditoria.

use std::path::PathBuf;

use poph_store::{
    ConversationStatus, NewAgent, NewConversation, NewDepartment, NewOrg, Page, Store, StoreError,
};
use poph_types::{AgentId, ConversationId, DomainError, OrgId};

struct Cenario {
    pasta: PathBuf,
    store: Store,
    org: OrgId,
    outra_org: OrgId,
    agente: AgentId,
}

impl Cenario {
    fn novo() -> Self {
        let pasta = std::env::temp_dir().join(format!(
            "poph-transferencia-store-{}-{}",
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
        Self {
            pasta,
            store,
            org,
            outra_org,
            agente,
        }
    }

    fn conversa(&self) -> ConversationId {
        self.store
            .conversas()
            .criar(
                self.org,
                NewConversation {
                    agent_id: self.agente,
                    integration_id: None,
                    canal: "cli".to_owned(),
                    peer_externo: None,
                    target_department: None,
                },
            )
            .expect("abre a conversa")
            .id
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
            slug: slug.to_owned(),
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
                slug: slug.to_owned(),
                ..NewAgent::default()
            },
        )
        .expect("cria o agente")
        .id
}

/// Aceite: a conversa fica `transferred`, a versão sobe e a auditoria sai junto.
#[test]
fn transferir_muda_situacao_e_audita() {
    let cenario = Cenario::novo();
    let conversa = cenario.conversa();
    let antes = cenario
        .store
        .conversas()
        .get(cenario.org, conversa)
        .expect("lê a conversa");
    assert_eq!(antes.status, ConversationStatus::Open);

    let depois = cenario
        .store
        .conversas()
        .transferir(
            cenario.org,
            conversa,
            None,
            Some("turno".to_owned()),
            r#"{"gate":"g1"}"#.to_owned(),
        )
        .expect("transfere");

    assert_eq!(depois.status, ConversationStatus::Transferred);
    assert_eq!(depois.version, antes.version + 1);
    assert!(depois.updated_at >= antes.updated_at);

    let auditoria = cenario
        .store
        .audit()
        .listar(cenario.org, Page::with_limit(10))
        .expect("lê a auditoria");
    let registro = auditoria
        .iter()
        .find(|registro| registro.acao == "transfer")
        .expect("a transferência é auditada");
    assert_eq!(registro.ator_tipo, "system");
    assert_eq!(registro.actor_id.as_deref(), Some("turno"));
    assert_eq!(registro.entity, "conversation");
    assert_eq!(
        registro.entity_id.as_deref(),
        Some(conversa.to_string().as_str())
    );
    assert_eq!(registro.detail_json, r#"{"gate":"g1"}"#);
}

/// O departamento alvo é guardado quando informado e mantido quando não é.
#[test]
fn transferir_guarda_o_departamento() {
    let cenario = Cenario::novo();
    let conversa = cenario.conversa();
    let departamento = cenario
        .store
        .departments()
        .criar(
            cenario.org,
            NewDepartment {
                name: "Atendimento humano".to_owned(),
                description: String::new(),
            },
        )
        .expect("cria o departamento")
        .id;

    let depois = cenario
        .store
        .conversas()
        .transferir(
            cenario.org,
            conversa,
            Some(departamento),
            None,
            "{}".to_owned(),
        )
        .expect("transfere");
    assert_eq!(depois.target_department, Some(departamento));

    // Sem departamento informado (ex.: G4 ainda não escolheu), o alvo não é apagado.
    let de_novo = cenario
        .store
        .conversas()
        .transferir(cenario.org, conversa, None, None, "{}".to_owned())
        .expect("transfere de novo");
    assert_eq!(de_novo.target_department, Some(departamento));
}

/// Conversa de outra org: "não encontrado" e nenhuma auditoria na org de quem tentou.
#[test]
fn nao_transfere_conversa_de_outra_org() {
    let cenario = Cenario::novo();
    let conversa = cenario.conversa();

    let erro = cenario
        .store
        .conversas()
        .transferir(
            cenario.outra_org,
            conversa,
            None,
            Some("turno".to_owned()),
            "{}".to_owned(),
        )
        .expect_err("não pode transferir conversa de outra org");
    assert!(matches!(
        erro,
        StoreError::Domain(DomainError::NotFound { .. })
    ));

    // A conversa continua aberta e a org B não ganhou auditoria.
    let gravada = cenario
        .store
        .conversas()
        .get(cenario.org, conversa)
        .expect("lê a conversa");
    assert_eq!(gravada.status, ConversationStatus::Open);
    assert!(
        cenario
            .store
            .audit()
            .listar(cenario.outra_org, Page::with_limit(10))
            .expect("lê a auditoria")
            .is_empty()
    );
}

/// Departamento de outra org é recusado antes de qualquer escrita.
#[test]
fn departamento_de_outra_org_e_recusado() {
    let cenario = Cenario::novo();
    let conversa = cenario.conversa();
    let alheio = cenario
        .store
        .departments()
        .criar(
            cenario.outra_org,
            NewDepartment {
                name: "Departamento da outra org".to_owned(),
                description: String::new(),
            },
        )
        .expect("cria o departamento")
        .id;

    let erro = cenario
        .store
        .conversas()
        .transferir(cenario.org, conversa, Some(alheio), None, "{}".to_owned())
        .expect_err("departamento de outra org é recusado");
    assert!(matches!(
        erro,
        StoreError::Domain(DomainError::NotFound { .. })
    ));
    let gravada = cenario
        .store
        .conversas()
        .get(cenario.org, conversa)
        .expect("lê a conversa");
    assert_eq!(gravada.status, ConversationStatus::Open);
}

#[test]
fn transferencia_administrativa_recusa_conversa_encerrada() {
    let cenario = Cenario::novo();
    let conversa = cenario.conversa();
    cenario
        .store
        .conversas()
        .fechar_pelo_agente(cenario.org, conversa, cenario.agente, "fim".to_owned())
        .expect("fecha pelo agente");

    let erro = cenario
        .store
        .conversas()
        .transferir_auditado(
            cenario.org,
            conversa,
            None,
            Some("admin".to_owned()),
            "{}".to_owned(),
        )
        .expect_err("conversa encerrada não transfere");
    assert!(matches!(
        erro,
        StoreError::Domain(DomainError::ConversationClosed { .. })
    ));

    let gravada = cenario
        .store
        .conversas()
        .get(cenario.org, conversa)
        .expect("lê a conversa");
    assert_eq!(gravada.status, ConversationStatus::Closed);
    assert!(
        !cenario
            .store
            .audit()
            .listar(cenario.org, Page::with_limit(10))
            .expect("lê a auditoria")
            .iter()
            .any(|registro| registro.acao == "transfer"),
        "a recusa não escreve auditoria de transferência"
    );
}

#[test]
fn transferencia_administrativa_marca_transferida_e_audita_o_motivo() {
    let cenario = Cenario::novo();
    let conversa = cenario.conversa();
    let depois = cenario
        .store
        .conversas()
        .transferir_auditado(
            cenario.org,
            conversa,
            None,
            Some("admin".to_owned()),
            r#"{"motivo":"rotina"}"#.to_owned(),
        )
        .expect("transfere");
    assert_eq!(depois.status, ConversationStatus::Transferred);

    let auditoria = cenario
        .store
        .audit()
        .listar(cenario.org, Page::with_limit(10))
        .expect("lê a auditoria");
    let registro = auditoria
        .iter()
        .find(|registro| registro.acao == "transfer")
        .expect("a transferência é auditada");
    assert_eq!(registro.actor_id.as_deref(), Some("admin"));
    assert_eq!(registro.detail_json, r#"{"motivo":"rotina"}"#);
}
