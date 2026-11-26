//! `poph chat` só confirma a saída das conversas do console (HR-26f).
//!
//! A caixa de saída é da org inteira. Com WhatsApp configurado, há itens de conversa de WhatsApp
//! pendentes ao lado dos do console. O envio do chat não pode tocar nos de WhatsApp: se tocasse,
//! a mensagem seria confirmada ou marcada como falha sem sair para o cliente.
//!
//! Banco de verdade em pasta temporária, transporte falso que conta cada envio.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use poph_agent::atores::{Atores, Futuro, OpcoesAtores, Processador, Tarefa};
use poph_agent::entrada_saida::{Fluxo, FuturoEnvio, OpcoesFluxo, TransporteSaida};
use poph_agent::metricas::MetricasMemoria;
use poph_store::{
    NewAgent, NewConversation, NewOrg, NewOutboxItem, OutboxItem, OutboxState, Store,
};
use poph_types::{ConversationId, OrgId, TurnId};

/// Processador que não faz nada: este teste só mexe na saída.
struct SemProcessar;

impl Processador for SemProcessar {
    fn processar(&self, _tarefa: Tarefa) -> Futuro {
        Box::pin(async { Ok(()) })
    }
}

/// Transporte que conta os envios e guarda o que recebeu.
#[derive(Default)]
struct TransporteDeTeste {
    envios: AtomicUsize,
    recebidos: Mutex<Vec<ConversationId>>,
}

impl TransporteDeTeste {
    fn envios(&self) -> usize {
        self.envios.load(Ordering::SeqCst)
    }

    fn conversas_enviadas(&self) -> Vec<ConversationId> {
        self.recebidos.lock().expect("sem veneno").clone()
    }
}

impl TransporteSaida for TransporteDeTeste {
    fn enviar(&self, item: OutboxItem) -> FuturoEnvio {
        self.envios.fetch_add(1, Ordering::SeqCst);
        self.recebidos
            .lock()
            .expect("sem veneno")
            .push(item.conversation_id);
        Box::pin(async { Ok(()) })
    }
}

struct Cenario {
    pasta: PathBuf,
    store: Arc<Store>,
    org: OrgId,
    console: ConversationId,
    whatsapp: ConversationId,
}

impl Cenario {
    fn novo(rotulo: &str) -> Self {
        let pasta =
            std::env::temp_dir().join(format!("poph-hr26f-{rotulo}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&pasta);
        std::fs::create_dir_all(&pasta).expect("cria a pasta do teste");
        let store = Arc::new(
            Store::open_at(&pasta.join("harness.db"), &pasta.join("rag.db")).expect("abre o banco"),
        );
        let org = store
            .orgs()
            .create(NewOrg {
                name: "Org HR-26f".to_owned(),
                slug: format!("org-hr26f-{rotulo}"),
                config_json: String::new(),
            })
            .expect("cria a org")
            .id;
        let agente = store
            .agents()
            .create(
                org,
                NewAgent {
                    name: "Agente HR-26f".to_owned(),
                    slug: format!("agente-hr26f-{rotulo}"),
                    ..NewAgent::default()
                },
            )
            .expect("cria o agente")
            .id;
        let integracao = store
            .create_integration(org, agente, "whatsapp".to_owned(), "active".to_owned())
            .expect("cria a integração")
            .id
            .parse()
            .expect("id da integração");
        let console = store
            .conversations()
            .create(
                org,
                NewConversation {
                    agent_id: agente,
                    integration_id: None,
                    canal: "cli".to_owned(),
                    peer_externo: None,
                    target_department: None,
                },
            )
            .expect("abre a conversa do console")
            .id;
        let whatsapp = store
            .conversations()
            .create(
                org,
                NewConversation {
                    agent_id: agente,
                    integration_id: Some(integracao),
                    canal: "whatsapp".to_owned(),
                    peer_externo: Some("cliente-de-teste".to_owned()),
                    target_department: None,
                },
            )
            .expect("abre a conversa do WhatsApp")
            .id;
        Self {
            pasta,
            store,
            org,
            console,
            whatsapp,
        }
    }

    fn fluxo(&self) -> Fluxo {
        let atores = Atores::novo(
            Arc::new(SemProcessar),
            OpcoesAtores {
                ociosidade: Duration::from_secs(60),
                fila_por_conversa: 8,
                max_atores: 8,
            },
            MetricasMemoria::proprio(),
        );
        Fluxo::nova(Arc::clone(&self.store), atores, OpcoesFluxo::default())
    }

    /// Põe um texto na caixa de saída da conversa e devolve o item.
    fn enfileirar(&self, conversa: ConversationId, seq: u32) -> OutboxItem {
        let (_novo, item) = self
            .store
            .outbox()
            .enqueue(
                self.org,
                NewOutboxItem {
                    conversation_id: conversa,
                    turn_id: TurnId::generate(),
                    seq,
                    tipo: "text".to_owned(),
                    content_json: format!("{{\"texto\":\"resposta {seq}\"}}"),
                },
            )
            .expect("enfileira a saída");
        item
    }

    fn estado(&self, item: &OutboxItem) -> OutboxState {
        self.store
            .outbox()
            .get(self.org, item.id)
            .expect("lê o item")
            .state
    }
}

impl Drop for Cenario {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.pasta);
    }
}

#[tokio::test]
async fn chat_confirma_o_do_console_e_nao_toca_no_whatsapp() {
    let cenario = Cenario::novo("chat");
    let do_console = cenario.enfileirar(cenario.console, 0);
    let do_whatsapp = cenario.enfileirar(cenario.whatsapp, 0);
    let transporte = TransporteDeTeste::default();

    let relato = cenario
        .fluxo()
        .enviar_pendentes_do_console(cenario.org, &transporte)
        .await
        .expect("envia o que é do console");

    assert_eq!(relato.enviados, 1, "só o do console sai");
    assert_eq!(transporte.envios(), 1, "o transporte foi chamado uma vez");
    assert_eq!(
        transporte.conversas_enviadas(),
        vec![cenario.console],
        "a única chamada é da conversa do console"
    );
    assert_eq!(cenario.estado(&do_console), OutboxState::Sent);
    assert_eq!(
        cenario.estado(&do_whatsapp),
        OutboxState::Pending,
        "o item de WhatsApp continua pendente"
    );
    let gravado = cenario
        .store
        .outbox()
        .get(cenario.org, do_whatsapp.id)
        .expect("lê o item de WhatsApp");
    assert_eq!(gravado.attempts, 0, "nenhuma tentativa foi gasta nele");
    assert!(
        gravado.ultimo_erro.is_none(),
        "nenhum erro foi gravado nele"
    );
}

#[tokio::test]
async fn o_envio_geral_pegaria_o_whatsapp_e_por_isso_o_chat_nao_usa_ele() {
    // Prova do defeito: o método geral confirma tudo da org, inclusive o de WhatsApp.
    let cenario = Cenario::novo("geral");
    let do_whatsapp = cenario.enfileirar(cenario.whatsapp, 0);
    let transporte = TransporteDeTeste::default();

    cenario
        .fluxo()
        .enviar_pendentes(cenario.org, &transporte)
        .await
        .expect("envio geral");

    assert_eq!(transporte.envios(), 1, "o geral tocou no item de WhatsApp");
    assert_eq!(cenario.estado(&do_whatsapp), OutboxState::Sent);
}

#[tokio::test]
async fn so_whatsapp_pendente_nao_chama_o_transporte() {
    let cenario = Cenario::novo("so-whatsapp");
    cenario.enfileirar(cenario.whatsapp, 0);
    cenario.enfileirar(cenario.whatsapp, 1);
    let transporte = TransporteDeTeste::default();

    let relato = cenario
        .fluxo()
        .enviar_pendentes_do_console(cenario.org, &transporte)
        .await
        .expect("envio do console");

    assert_eq!(transporte.envios(), 0, "nenhuma chamada ao canal do chat");
    assert_eq!(relato.enviados, 0);
}
