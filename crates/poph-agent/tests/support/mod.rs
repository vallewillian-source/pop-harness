use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use poph_agent::config_hot::{ConfigHot, HotConfigOptions};
use poph_agent::limites::{LimitOptions, Limiter};
use poph_agent::metrics::{MetricsMemory, MetricsTurn};
use poph_agent::turno::{
    FuturePause, NoopObserver, Pause, TurnEngine, TurnInput, TurnOptions, TurnSummary,
};
use poph_core::llm::{FakeProvider, FinishReason, LlmEvent, Script, Usage};
use poph_store::{
    Direction, NewAgent, NewChannelAttachment, NewConversation, NewMessage, Role, Store,
};
use poph_types::{ConversationId, MessageId, OrgId, StructuredInstructions};

pub struct NoWait;

impl Pause for NoWait {
    fn wait(&self, _duration: Duration) -> FuturePause {
        Box::pin(async {})
    }
}

pub struct Scenario {
    pub folder: PathBuf,
    pub store: Arc<Store>,
    pub org: OrgId,
    pub conversa: ConversationId,
    pub files: poph_files::FileConfig,
    pub metrics: MetricsTurn,
}

pub struct Attachment {
    pub nome: String,
    pub mime: String,
    pub bytes: Vec<u8>,
    pub without_blob: bool,
    pub size_declared: Option<u64>,
    pub text_extracted: Option<String>,
}

impl Attachment {
    pub fn image(nome: &str, bytes: &[u8]) -> Self {
        Self {
            nome: nome.to_owned(),
            mime: "image/png".to_owned(),
            bytes: bytes.to_vec(),
            without_blob: false,
            size_declared: None,
            text_extracted: None,
        }
    }

    pub fn document(nome: &str, texto: &str) -> Self {
        Self {
            nome: nome.to_owned(),
            mime: "application/pdf".to_owned(),
            bytes: b"%PDF-1.4 de teste".to_vec(),
            without_blob: false,
            size_declared: None,
            text_extracted: Some(texto.to_owned()),
        }
    }
}

impl Scenario {
    pub fn new(label: &str) -> Self {
        let folder = std::env::temp_dir().join(format!(
            "poph-hr23b-{label}-{}-{}",
            std::process::id(),
            fastrand()
        ));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).expect("pasta do teste");
        let store = Arc::new(
            Store::open_at(&folder.join("harness.db"), &folder.join("rag.db"))
                .expect("abre o banco"),
        );
        let org = store
            .orgs()
            .create(poph_store::NewOrg {
                name: format!("Org {label}"),
                slug: format!("org-{label}-{}-{}", std::process::id(), fastrand()),
                config_json: "{}".to_owned(),
            })
            .expect("cria a org")
            .id;
        let agente = store
            .agents()
            .create(
                org,
                NewAgent {
                    name: format!("Bia {label}"),
                    slug: format!("bia-{label}-{}", fastrand()),
                    grouping_window_ms: 0,
                    ..NewAgent::default()
                },
            )
            .expect("cria o agente");
        let structured = StructuredInstructions::default();
        store
            .instructions()
            .save(
                org,
                agente.id,
                0,
                &structured.to_json(),
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
        let files = poph_files::FileConfig::default(folder.join("blobs"));
        Self {
            folder,
            store,
            org,
            conversa,
            files,
            metrics: MetricsTurn::own(),
        }
    }

    pub fn speak_with_files(&self, texto: &str, anexos: Vec<Attachment>) -> MessageId {
        let mut lines: Vec<NewChannelAttachment> = Vec::with_capacity(anexos.len());
        for attachment in anexos {
            let sha256 = if attachment.without_blob {
                poph_files::blob::sha256_hex(&attachment.bytes)
            } else {
                poph_files::blob::write(&self.files.blob_folder, self.org, &attachment.bytes)
                    .expect("grava o blob")
                    .sha256
            };
            let status_extraction = if attachment.mime.starts_with("image/") {
                "skipped"
            } else {
                "ok"
            };
            lines.push(NewChannelAttachment {
                name: attachment.nome,
                mime: attachment.mime,
                size_bytes: attachment
                    .size_declared
                    .unwrap_or(attachment.bytes.len() as u64),
                sha256,
                blob_path: format!("orgs/{}/{}", self.org, "sha-de-teste"),
                extracted_text: attachment.text_extracted,
                extraction_status: status_extraction,
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
                &lines,
            )
            .expect("grava a mensagem com anexos")
            .id
    }

    pub fn engine(&self, with_files: bool) -> (TurnEngine, Arc<FakeProvider>) {
        let fake = Arc::new(FakeProvider::with_scripts(vec![script_text(
            "recebi o arquivo",
        )]));
        let config = ConfigHot::new(
            Arc::clone(&self.store),
            HotConfigOptions::default(),
            MetricsMemory::own(),
        );
        let limiter = Arc::new(Limiter::new(LimitOptions::default(), MetricsMemory::own()));
        let mut engine = TurnEngine::new(
            Arc::clone(&self.store),
            config,
            limiter,
            fake.clone(),
            TurnOptions::default(),
            self.metrics.clone(),
        )
        .with_pause(Arc::new(NoWait))
        .with_summary(poph_agent::summary::SummaryOptions {
            ativo: false,
            ..poph_agent::summary::SummaryOptions::default()
        });
        if with_files {
            engine = engine.with_files(self.files.clone());
        }
        (engine, fake)
    }

    pub async fn turno(&self, engine: &TurnEngine, gatilho: MessageId) -> TurnSummary {
        engine
            .run(
                TurnInput::new(self.org, self.conversa, gatilho),
                &mut NoopObserver,
            )
            .await
            .expect("o turno roda")
    }
}

impl Drop for Scenario {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.folder);
    }
}

fn fastrand() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|time| time.as_nanos() as u64)
        .unwrap_or(0)
}

fn script_text(texto: &str) -> Script {
    Script::Events(vec![
        LlmEvent::Texto(texto.to_owned()),
        LlmEvent::Usage(Usage {
            input: 100,
            saida: 10,
        }),
        LlmEvent::Fim(FinishReason::Completed),
    ])
}
