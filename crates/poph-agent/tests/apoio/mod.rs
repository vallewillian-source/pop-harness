//! Cenário de turno com armazenamento real e provedor falso (HR-23b).
//!
//! Vive em `tests/apoio/` porque só assim é compartilhado entre binários de teste: um arquivo
//! `.rs` solto em `tests/` viraria um binário próprio. O mesmo desenho do `tests/apoio` de
//! `poph-core`.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use poph_agent::config_quente::{ConfigQuente, OpcoesConfigQuente};
use poph_agent::limites::{Limitador, OpcoesLimites};
use poph_agent::metricas::{MetricasMemoria, MetricasTurno};
use poph_agent::turno::{
    Entrada, FuturoPausa, Motor, ObservadorNulo, OpcoesTurno, Pausa, ResumoTurno,
};
use poph_core::llm::{EventoLlm, MotivoFim, ProvedorFake, Roteiro, Uso};
use poph_store::{
    Direction, NewAgent, NewChannelAttachment, NewConversation, NewMessage, Role, Store,
};
use poph_types::{ConversationId, MessageId, OrgId, StructuredInstructions};

/// Pausa que não dorme: a janela de agrupamento do turno não atrasa o teste.
pub struct SemEspera;

impl Pausa for SemEspera {
    fn esperar(&self, _duracao: Duration) -> FuturoPausa {
        Box::pin(async {})
    }
}

/// Um turno pronto para rodar: banco, org, agente, conversa e pasta de blobs.
pub struct Cenario {
    pub pasta: PathBuf,
    pub store: Arc<Store>,
    pub org: OrgId,
    pub conversa: ConversationId,
    pub arquivos: poph_files::ConfigArquivos,
    pub metricas: MetricasTurno,
}

/// Anexo pronto para gravar: bytes no blob (ou não) e a linha em `attachments`.
pub struct Anexo {
    pub nome: String,
    pub mime: String,
    pub bytes: Vec<u8>,
    /// `true` = grava só a linha, sem o blob no disco (blob ausente).
    pub sem_blob: bool,
    /// Tamanho declarado na linha, quando difere do real (prova o teto sem gravar 5 MB).
    pub tamanho_declarado: Option<u64>,
    pub texto_extraido: Option<String>,
}

impl Anexo {
    pub fn imagem(nome: &str, bytes: &[u8]) -> Self {
        Self {
            nome: nome.to_owned(),
            mime: "image/png".to_owned(),
            bytes: bytes.to_vec(),
            sem_blob: false,
            tamanho_declarado: None,
            texto_extraido: None,
        }
    }

    pub fn documento(nome: &str, texto: &str) -> Self {
        Self {
            nome: nome.to_owned(),
            mime: "application/pdf".to_owned(),
            bytes: b"%PDF-1.4 de teste".to_vec(),
            sem_blob: false,
            tamanho_declarado: None,
            texto_extraido: Some(texto.to_owned()),
        }
    }
}

impl Cenario {
    pub fn novo(rotulo: &str) -> Self {
        let pasta = std::env::temp_dir().join(format!(
            "poph-hr23b-{rotulo}-{}-{}",
            std::process::id(),
            fastrand()
        ));
        let _ = std::fs::remove_dir_all(&pasta);
        std::fs::create_dir_all(&pasta).expect("pasta do teste");
        let store = Arc::new(
            Store::open_at(&pasta.join("harness.db"), &pasta.join("rag.db")).expect("abre o banco"),
        );
        let org = store
            .orgs()
            .create(poph_store::NewOrg {
                name: format!("Org {rotulo}"),
                slug: format!("org-{rotulo}-{}-{}", std::process::id(), fastrand()),
                config_json: "{}".to_owned(),
            })
            .expect("cria a org")
            .id;
        let agente = store
            .agents()
            .create(
                org,
                NewAgent {
                    name: format!("Bia {rotulo}"),
                    slug: format!("bia-{rotulo}-{}", fastrand()),
                    grouping_window_ms: 0,
                    ..NewAgent::default()
                },
            )
            .expect("cria o agente");
        let estruturado = StructuredInstructions::default();
        store
            .instructions()
            .save(
                org,
                agente.id,
                0,
                &estruturado.to_json(),
                "# Bia\n\nVocê é a atendente da loja.",
                true,
            )
            .expect("grava as instruções");
        let conversa = store
            .conversations()
            .create(
                org,
                NewConversation {
                    agent_id: agente.id,
                    integration_id: None,
                    canal: "cli".to_owned(),
                    peer_externo: None,
                    target_department: None,
                },
            )
            .expect("abre a conversa")
            .id;
        let arquivos = poph_files::ConfigArquivos::padrao(pasta.join("blobs"));
        Self {
            pasta,
            store,
            org,
            conversa,
            arquivos,
            metricas: MetricasTurno::proprio(),
        }
    }

    /// Grava a mensagem do cliente com os anexos dados, pondo o blob no disco quando for o caso.
    pub fn falar_com_arquivos(&self, texto: &str, anexos: Vec<Anexo>) -> MessageId {
        let mut linhas: Vec<NewChannelAttachment> = Vec::with_capacity(anexos.len());
        for anexo in anexos {
            let sha256 = if anexo.sem_blob {
                poph_files::blob::sha256_hex(&anexo.bytes)
            } else {
                poph_files::blob::gravar(&self.arquivos.pasta_blobs, self.org, &anexo.bytes)
                    .expect("grava o blob")
                    .sha256
            };
            let status_extracao = if anexo.mime.starts_with("image/") {
                "skipped"
            } else {
                "ok"
            };
            linhas.push(NewChannelAttachment {
                name: anexo.nome,
                mime: anexo.mime,
                size_bytes: anexo.tamanho_declarado.unwrap_or(anexo.bytes.len() as u64),
                sha256,
                blob_path: format!("orgs/{}/{}", self.org, "sha-de-teste"),
                extracted_text: anexo.texto_extraido,
                extraction_status: status_extracao,
            });
        }
        self.store
            .messages()
            .write_with_attachments(
                self.org,
                NewMessage {
                    conversation_id: self.conversa,
                    direction: Direction::Inbound,
                    role: Role::User,
                    texto: texto.to_owned(),
                    attachments_json: "[]".to_owned(),
                    external_id: None,
                    turn_id: None,
                    origin: poph_store::Origin::Agente,
                    author_name: None,
                },
                &linhas,
            )
            .expect("grava a mensagem com anexos")
            .id
    }

    pub fn motor(&self, com_arquivos: bool) -> (Motor, Arc<ProvedorFake>) {
        let fake = Arc::new(ProvedorFake::com_roteiros(vec![roteiro_texto(
            "recebi o arquivo",
        )]));
        let config = ConfigQuente::nova(
            Arc::clone(&self.store),
            OpcoesConfigQuente::default(),
            MetricasMemoria::proprio(),
        );
        let limitador = Arc::new(Limitador::novo(
            OpcoesLimites::default(),
            MetricasMemoria::proprio(),
        ));
        let mut motor = Motor::novo(
            Arc::clone(&self.store),
            config,
            limitador,
            fake.clone(),
            OpcoesTurno::default(),
            self.metricas.clone(),
        )
        .com_pausa(Arc::new(SemEspera))
        .com_resumo(poph_agent::resumo::OpcoesResumo {
            ativo: false,
            ..poph_agent::resumo::OpcoesResumo::default()
        });
        if com_arquivos {
            motor = motor.com_arquivos(self.arquivos.clone());
        }
        (motor, fake)
    }

    pub async fn turno(&self, motor: &Motor, gatilho: MessageId) -> ResumoTurno {
        motor
            .executar(
                Entrada::nova(self.org, self.conversa, gatilho),
                &mut ObservadorNulo,
            )
            .await
            .expect("o turno roda")
    }
}

impl Drop for Cenario {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.pasta);
    }
}

fn fastrand() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|tempo| tempo.as_nanos() as u64)
        .unwrap_or(0)
}

fn roteiro_texto(texto: &str) -> Roteiro {
    Roteiro::Eventos(vec![
        EventoLlm::Texto(texto.to_owned()),
        EventoLlm::Uso(Uso {
            entrada: 100,
            saida: 10,
        }),
        EventoLlm::Fim(MotivoFim::Concluido),
    ])
}
