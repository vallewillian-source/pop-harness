//! Cenário comum aos testes do plugin de exemplo: banco de verdade em pasta temporária, motor de
//! entrada/saída, supervisor do plugin como processo filho e o canal pelo contrato [`Integration`].

#![allow(dead_code)]

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use poph_agent::atores::{Atores, Futuro, OpcoesAtores, Processador, Tarefa};
use poph_agent::entrada_saida::{Fluxo, OpcoesFluxo};
use poph_agent::fila_integracao::{ConversaDoCanal, EnvioPeloCanal, InboxDoCanal};
use poph_agent::metricas::MetricasMemoria;
use poph_plugins::{Espera, LimitesSupervisor, OpcoesPlugin, Supervisor};
use poph_store::{
    Conversa, EstadoSaida, ItemSaida, NovaConversa, NovaOrg, NovaSaida, NovoAgente, Pagina, Store,
};
use poph_types::{ConversationId, EventSink, Integration, IntegrationId, OrgId};

/// Prazos curtos: o comportamento é o mesmo do padrão, só a espera do teste é menor.
pub fn limites_rapidos() -> LimitesSupervisor {
    LimitesSupervisor {
        max_mensagem_bytes: 1024 * 1024,
        timeout_handshake: Duration::from_secs(5),
        timeout_comando: Duration::from_millis(500),
        timeout_saude: Duration::from_secs(2),
        espera: Espera {
            base: Duration::from_millis(30),
            fator: 2,
            teto: Duration::from_millis(200),
        },
        max_falhas: 5,
    }
}

/// Nenhum turno roda nestes testes: interessa a ida e a volta das filas.
pub struct SemTurno;

impl Processador for SemTurno {
    fn processar(&self, _tarefa: Tarefa) -> Futuro {
        Box::pin(async { Ok(()) })
    }
}

/// Pasta temporária própria do cenário.
pub fn pasta_temporaria(rotulo: &str) -> PathBuf {
    static CONTADOR: AtomicU64 = AtomicU64::new(0);
    let sequencia = CONTADOR.fetch_add(1, Ordering::Relaxed);
    let pasta = std::env::temp_dir().join(format!(
        "poph-eco-{rotulo}-{}-{sequencia}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&pasta);
    std::fs::create_dir_all(&pasta).expect("criar a pasta do cenário");
    pasta
}

/// Cenário montado: banco, conversa, fluxo, receptor da inbox e supervisor do plugin.
pub struct Cenario {
    pub pasta: PathBuf,
    pub store: Arc<Store>,
    pub org: OrgId,
    pub agente: poph_types::AgentId,
    pub conversa: Conversa,
    pub fluxo: Fluxo,
    pub entrada: Arc<InboxDoCanal>,
    pub supervisor: Supervisor,
    pub canal: Arc<dyn Integration>,
    pub integracao: IntegrationId,
}

impl Cenario {
    /// Sobe o plugin dado e abre conversa com (ou sem) contato.
    pub async fn novo(rotulo: &str, programa: &PathBuf, peer: Option<&str>) -> Self {
        let pasta = pasta_temporaria(rotulo);
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
        let conversa = store
            .conversas()
            .criar(
                org,
                NovaConversa {
                    agent_id: agente,
                    integration_id: None,
                    canal: "eco".to_owned(),
                    peer_externo: peer.map(str::to_owned),
                    departamento_alvo: None,
                },
            )
            .expect("abre a conversa");

        let atores = Atores::novo(
            Arc::new(SemTurno),
            OpcoesAtores::default(),
            MetricasMemoria::proprio(),
        );
        let fluxo = Fluxo::nova(Arc::clone(&store), atores, OpcoesFluxo::default());
        let resolvedor = Arc::new(ConversaDoCanal::novo(
            Arc::clone(&store),
            agente,
            "eco",
            None,
        ));
        let entrada = InboxDoCanal::iniciar(fluxo.clone(), org, resolvedor);

        let integracao = IntegrationId::generate();
        let opcoes = OpcoesPlugin::novo(programa, integracao, org)
            .com_kind("eco")
            .com_limites(limites_rapidos());
        let supervisor = Supervisor::iniciar(opcoes, Arc::clone(&entrada) as Arc<dyn EventSink>)
            .await
            .expect("sobe o plugin eco");
        let canal: Arc<dyn Integration> = Arc::new(supervisor.clone());
        Self {
            pasta,
            store,
            org,
            agente,
            conversa,
            fluxo,
            entrada,
            supervisor,
            canal,
            integracao,
        }
    }

    /// Transporte de saída ligado ao canal deste cenário.
    pub fn transporte(&self) -> EnvioPeloCanal {
        EnvioPeloCanal::novo(Arc::clone(&self.store), Arc::clone(&self.canal))
    }

    /// Enfileira um texto na `outbox` da conversa do cenário.
    pub fn enfileirar_texto(&self, turno: poph_types::TurnId, texto: &str) -> ItemSaida {
        self.store
            .caixa_saida()
            .enfileirar(
                self.org,
                NovaSaida {
                    conversation_id: self.conversa.id,
                    turn_id: turno,
                    seq: 0,
                    tipo: "text".to_owned(),
                    conteudo_json: serde_json::json!({"texto": texto}).to_string(),
                },
            )
            .expect("enfileira o texto")
            .1
    }

    /// Estado atual de um item da `outbox`.
    pub fn estado_do_item(&self, id: poph_types::OutboxId) -> EstadoSaida {
        self.store
            .caixa_saida()
            .obter(self.org, id)
            .expect("lê o item de saída")
            .estado
    }

    /// Mensagens gravadas na conversa do cenário.
    pub fn mensagens(&self) -> Vec<poph_store::Mensagem> {
        self.store
            .mensagens()
            .listar(self.org, self.conversa.id, Pagina::com_limite(50))
            .expect("lista as mensagens")
    }

    /// Espera uma condição virar verdadeira (até 5 s, conferindo a cada 10 ms).
    pub async fn esperar(&self, rotulo: &str, condicao: impl Fn() -> bool) {
        let limite = Instant::now() + Duration::from_secs(5);
        while Instant::now() < limite {
            if condicao() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("a condição `{rotulo}` não ficou verdadeira em 5 s");
    }

    /// Encerra o supervisor e apaga a pasta.
    pub async fn encerrar(self) {
        self.supervisor.encerrar().await;
    }

    /// Id da conversa (atalho para as asserções).
    pub fn conversa_id(&self) -> ConversationId {
        self.conversa.id
    }
}

impl Drop for Cenario {
    /// Apaga a pasta do cenário mesmo quando o teste falha no meio.
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.pasta);
    }
}
