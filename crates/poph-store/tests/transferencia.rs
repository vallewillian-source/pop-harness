//! Transferência da conversa para humano na camada de armazenamento (HR-29).
//!
//! O que o repositório promete: a situação vira `transferred`, a versão sobe (então o `If-Match` de
//! quem edita continua valendo) e a auditoria é gravada **na mesma transação**, com ator do sistema.
//! Transferir conversa de outra org não acha nada e não escreve auditoria.

use std::path::PathBuf;

use poph_store::{
    ErroStore, NovaConversa, NovaOrg, NovoAgente, NovoDepartamento, Pagina, StatusConversa, Store,
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
        let store = Store::abrir_em(&pasta.join("harness.db"), &pasta.join("rag.db"))
            .expect("abre o banco");
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
                NovaConversa {
                    agent_id: self.agente,
                    integration_id: None,
                    canal: "cli".to_owned(),
                    peer_externo: None,
                    departamento_alvo: None,
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
        .orgaos()
        .criar(NovaOrg {
            nome: format!("Org {slug}"),
            slug: slug.to_owned(),
            config_json: "{}".to_owned(),
        })
        .expect("cria a org")
        .id
}

fn criar_agente(store: &Store, org: OrgId, slug: &str) -> AgentId {
    store
        .agentes()
        .criar(
            org,
            NovoAgente {
                nome: format!("Agente {slug}"),
                slug: slug.to_owned(),
                ..NovoAgente::default()
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
        .obter(cenario.org, conversa)
        .expect("lê a conversa");
    assert_eq!(antes.status, StatusConversa::Aberta);

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

    assert_eq!(depois.status, StatusConversa::Transferida);
    assert_eq!(depois.versao, antes.versao + 1);
    assert!(depois.atualizado_em >= antes.atualizado_em);

    let auditoria = cenario
        .store
        .auditoria()
        .listar(cenario.org, Pagina::com_limite(10))
        .expect("lê a auditoria");
    let registro = auditoria
        .iter()
        .find(|registro| registro.acao == "transfer")
        .expect("a transferência é auditada");
    assert_eq!(registro.ator_tipo, "system");
    assert_eq!(registro.ator_id.as_deref(), Some("turno"));
    assert_eq!(registro.entidade, "conversation");
    assert_eq!(
        registro.entidade_id.as_deref(),
        Some(conversa.to_string().as_str())
    );
    assert_eq!(registro.detalhe_json, r#"{"gate":"g1"}"#);
}

/// O departamento alvo é guardado quando informado e mantido quando não é.
#[test]
fn transferir_guarda_o_departamento() {
    let cenario = Cenario::novo();
    let conversa = cenario.conversa();
    let departamento = cenario
        .store
        .departamentos()
        .criar(
            cenario.org,
            NovoDepartamento {
                nome: "Atendimento humano".to_owned(),
                descricao: String::new(),
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
    assert_eq!(depois.departamento_alvo, Some(departamento));

    // Sem departamento informado (ex.: G4 ainda não escolheu), o alvo não é apagado.
    let de_novo = cenario
        .store
        .conversas()
        .transferir(cenario.org, conversa, None, None, "{}".to_owned())
        .expect("transfere de novo");
    assert_eq!(de_novo.departamento_alvo, Some(departamento));
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
        ErroStore::Dominio(DomainError::NaoEncontrado { .. })
    ));

    // A conversa continua aberta e a org B não ganhou auditoria.
    let gravada = cenario
        .store
        .conversas()
        .obter(cenario.org, conversa)
        .expect("lê a conversa");
    assert_eq!(gravada.status, StatusConversa::Aberta);
    assert!(
        cenario
            .store
            .auditoria()
            .listar(cenario.outra_org, Pagina::com_limite(10))
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
        .departamentos()
        .criar(
            cenario.outra_org,
            NovoDepartamento {
                nome: "Departamento da outra org".to_owned(),
                descricao: String::new(),
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
        ErroStore::Dominio(DomainError::NaoEncontrado { .. })
    ));
    let gravada = cenario
        .store
        .conversas()
        .obter(cenario.org, conversa)
        .expect("lê a conversa");
    assert_eq!(gravada.status, StatusConversa::Aberta);
}
