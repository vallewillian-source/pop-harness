//! Testes do resolvedor de conversa do canal (HR-27): um contato, uma conversa; conversa que deixou
//! de estar aberta não é servida do cache.

use std::sync::Arc;

use poph_agent::fila_integracao::{ConversaDoCanal, ResolvedorConversa};
use poph_store::{NovaConversa, NovaOrg, NovoAgente, StatusConversa, Store};
use poph_types::{AgentId, InboundEvent, IntegrationId, OrgId};

struct Cenario {
    pasta: std::path::PathBuf,
    store: Arc<Store>,
    org: OrgId,
    agente: AgentId,
}

impl Cenario {
    fn novo(rotulo: &str) -> Self {
        let pasta = std::env::temp_dir().join(format!(
            "poph-fila-integracao-{rotulo}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&pasta);
        std::fs::create_dir_all(&pasta).expect("cria a pasta");
        let store = Arc::new(
            Store::abrir_em(&pasta.join("harness.db"), &pasta.join("rag.db"))
                .expect("abre o banco"),
        );
        let org = store
            .orgaos()
            .criar(NovaOrg {
                nome: format!("Org {rotulo}"),
                slug: format!("org-{rotulo}"),
                config_json: String::new(),
            })
            .expect("cria a org")
            .id;
        let agente = store
            .agentes()
            .criar(
                org,
                NovoAgente {
                    nome: format!("Agente {rotulo}"),
                    slug: format!("agente-{rotulo}"),
                    ..NovoAgente::default()
                },
            )
            .expect("cria o agente")
            .id;
        Self {
            pasta,
            store,
            org,
            agente,
        }
    }
}

impl Drop for Cenario {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.pasta);
    }
}

fn evento(peer: &str) -> InboundEvent {
    InboundEvent {
        integration: IntegrationId::generate(),
        id_externo: format!("msg-{peer}"),
        peer: peer.to_owned(),
        remetente: None,
        text: "oi".to_owned(),
        attachments: Vec::new(),
        occurred_at: 1_795_140_000_000,
        metadata: serde_json::Value::Null,
    }
}

#[test]
fn um_contato_uma_conversa_com_cache_por_contato() {
    let cenario = Cenario::novo("cache");
    let resolvedor = ConversaDoCanal::novo(Arc::clone(&cenario.store), cenario.agente, "eco", None);

    let primeira = resolvedor
        .resolver(cenario.org, &evento("5511911111111"))
        .expect("resolve o primeiro contato");
    // O mesmo contato, de novo: mesma conversa, e o cache não cresce.
    let repetida = resolvedor
        .resolver(cenario.org, &evento("5511911111111"))
        .expect("resolve de novo");
    assert_eq!(primeira, repetida);
    assert_eq!(resolvedor.em_cache(), 1);

    // Outro contato abre outra conversa.
    let outra = resolvedor
        .resolver(cenario.org, &evento("5511922222222"))
        .expect("resolve o segundo contato");
    assert_ne!(primeira, outra);
    assert_eq!(resolvedor.em_cache(), 2);

    let conversas = cenario
        .store
        .conversas()
        .listar(cenario.org, None, poph_store::Pagina::com_limite(10))
        .expect("lista as conversas");
    assert_eq!(conversas.len(), 2);
    assert!(
        conversas
            .iter()
            .all(|conversa| conversa.canal == "eco" && conversa.agent_id == cenario.agente)
    );
}

#[test]
fn conversa_transferida_nao_e_servida_do_cache() {
    let cenario = Cenario::novo("transferida");
    let resolvedor = ConversaDoCanal::novo(Arc::clone(&cenario.store), cenario.agente, "eco", None);
    let peer = "5511933333333";

    let conversa = resolvedor
        .resolver(cenario.org, &evento(peer))
        .expect("resolve o contato");
    // Transferida para humano (D12): o agente para de responder nela.
    cenario
        .store
        .conversas()
        .mudar_status(cenario.org, conversa, 1, StatusConversa::Transferida, None)
        .expect("transfere a conversa");

    // Uma conversa nova e aberta do mesmo contato passa a ser a escolhida (o cache não insiste na
    // conversa que deixou de estar aberta).
    let nova = cenario
        .store
        .conversas()
        .criar(
            cenario.org,
            NovaConversa {
                agent_id: cenario.agente,
                integration_id: None,
                canal: "eco".to_owned(),
                peer_externo: Some(peer.to_owned()),
                departamento_alvo: None,
            },
        )
        .expect("abre a conversa nova");
    let escolhida = resolvedor
        .resolver(cenario.org, &evento(peer))
        .expect("resolve de novo");
    assert_eq!(escolhida, nova.id);
}
