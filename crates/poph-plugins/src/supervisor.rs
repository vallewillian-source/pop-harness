use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use poph_types::{
    CommandFuture, EventSink, Health, HealthFuture, HostMessage, Integration, IntegrationError,
    IntegrationEvent, IntegrationId, IntegrationState, Manifest, OrgId, OutboundCommand,
    PROTOCOL_VERSION, PluginMessage, Secret, SendFile, SendText, SetPresence,
};
use tokio::io::AsyncWriteExt;
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{mpsc, oneshot};
use tokio::time::{sleep, timeout};

use crate::reader::{LineReader, NextLine};

const CONSTRUCTOR_MARGIN: Duration = Duration::from_secs(1);

const KILL_DEADLINE: Duration = Duration::from_secs(2);

const LINE_QUEUE_CAPACITY: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Backoff {
    pub base: Duration,
    pub factor: u32,
    pub cap: Duration,
}

impl Default for Backoff {
    fn default() -> Self {
        Self {
            base: Duration::from_millis(500),
            factor: 2,
            cap: Duration::from_secs(60),
        }
    }
}

impl Backoff {
    pub fn delay(&self, failure: u32) -> Duration {
        let mut delay = self.base;
        for _ in 1..failure.max(1) {
            if delay >= self.cap {
                break;
            }
            delay = delay.saturating_mul(self.factor.max(1));
        }
        delay.min(self.cap)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SupervisorLimits {
    pub max_message_bytes: usize,
    pub timeout_handshake: Duration,
    pub command_timeout: Duration,
    pub health_timeout: Duration,
    pub backoff: Backoff,
    pub max_failures: u32,
}

impl Default for SupervisorLimits {
    fn default() -> Self {
        Self {
            max_message_bytes: 1024 * 1024,
            timeout_handshake: Duration::from_secs(5),
            command_timeout: Duration::from_secs(10),
            health_timeout: Duration::from_secs(3),
            backoff: Backoff::default(),
            max_failures: 5,
        }
    }
}

#[derive(Clone)]
pub struct PluginOptions {
    pub program: PathBuf,
    pub arguments: Vec<String>,
    pub environment: Vec<(String, String)>,
    pub directory: Option<PathBuf>,
    pub integration: IntegrationId,
    pub org: OrgId,
    pub kind: String,
    pub config: serde_json::Value,
    pub secrets: BTreeMap<String, Secret>,
    pub limits: SupervisorLimits,
}

impl PluginOptions {
    pub fn new(program: impl Into<PathBuf>, integration: IntegrationId, org: OrgId) -> Self {
        Self {
            program: program.into(),
            arguments: Vec::new(),
            environment: Vec::new(),
            directory: None,
            integration,
            org,
            kind: "plugin".to_owned(),
            config: serde_json::json!({}),
            secrets: BTreeMap::new(),
            limits: SupervisorLimits::default(),
        }
    }

    pub fn with_kind(mut self, kind: impl Into<String>) -> Self {
        self.kind = kind.into();
        self
    }

    pub fn with_config(mut self, config: serde_json::Value) -> Self {
        self.config = config;
        self
    }

    pub fn with_secret(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.secrets.insert(name.into(), Secret::new(value.into()));
        self
    }

    pub fn with_env(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.environment.push((name.into(), value.into()));
        self
    }

    pub fn with_limits(mut self, limits: SupervisorLimits) -> Self {
        self.limits = limits;
        self
    }
}

impl fmt::Debug for PluginOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PluginOptions")
            .field("program", &self.program)
            .field("arguments", &self.arguments)
            .field(
                "environment",
                &format_args!("{} variável(is)", self.environment.len()),
            )
            .field("directory", &self.directory)
            .field("integration", &self.integration)
            .field("org", &self.org)
            .field("kind", &self.kind)
            .field("config", &self.config)
            .field(
                "secrets",
                &format_args!("{} chave(s) [oculto]", self.secrets.len()),
            )
            .field("limits", &self.limits)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CounterReport {
    pub lines: u64,
    pub junk: u64,
    pub events: u64,
    pub invalid_events: u64,
    pub states: u64,
    pub commands: u64,
    pub health_checks: u64,
    pub handshakes: u64,
    pub failures: u64,
    pub restarts: u64,
    pub timed_out: u64,
    pub aborts: u64,
}

#[derive(Debug, Default)]
struct Counters {
    lines: AtomicU64,
    junk: AtomicU64,
    events: AtomicU64,
    invalid_events: AtomicU64,
    states: AtomicU64,
    commands: AtomicU64,
    health_checks: AtomicU64,
    handshakes: AtomicU64,
    failures: AtomicU64,
    restarts: AtomicU64,
    timed_out: AtomicU64,
    aborts: AtomicU64,
}

impl Counters {
    fn increment(&self, counter: &AtomicU64) {
        counter.fetch_add(1, Ordering::Relaxed);
    }

    fn report(&self) -> CounterReport {
        let load = |counter: &AtomicU64| counter.load(Ordering::Relaxed);
        CounterReport {
            lines: load(&self.lines),
            junk: load(&self.junk),
            events: load(&self.events),
            invalid_events: load(&self.invalid_events),
            states: load(&self.states),
            commands: load(&self.commands),
            health_checks: load(&self.health_checks),
            handshakes: load(&self.handshakes),
            failures: load(&self.failures),
            restarts: load(&self.restarts),
            timed_out: load(&self.timed_out),
            aborts: load(&self.aborts),
        }
    }
}

struct Shared {
    options: PluginOptions,
    state: RwLock<IntegrationState>,
    manifest: RwLock<Option<Manifest>>,
    counters: Counters,
    next_id: AtomicU64,
}

impl Shared {
    fn new(options: PluginOptions) -> Self {
        Self {
            options,
            state: RwLock::new(IntegrationState::Starting),
            manifest: RwLock::new(None),
            counters: Counters::default(),
            next_id: AtomicU64::new(1),
        }
    }

    fn set_state(&self, next: IntegrationState) {
        let mut state = self
            .state
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if *state != next {
            tracing::debug!(target: "poph_plugins", state = ?next, "estado da integração mudou");
        }
        *state = next;
    }

    fn state(&self) -> IntegrationState {
        self.state
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    fn manifest(&self) -> Option<Manifest> {
        self.manifest
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    fn set_manifest(&self, manifest: Manifest) {
        let mut guard = self
            .manifest
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *guard = Some(manifest);
    }

    fn next_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }
}

#[derive(Clone)]
pub struct Supervisor {
    control: mpsc::UnboundedSender<Control>,
    shared: Arc<Shared>,
}

impl fmt::Debug for Supervisor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Supervisor")
            .field("integration", &self.shared.options.integration)
            .field("kind", &self.shared.options.kind)
            .field("state", &self.shared.state())
            .finish_non_exhaustive()
    }
}

impl Supervisor {
    pub async fn start(
        options: PluginOptions,
        sink: Arc<dyn EventSink>,
    ) -> Result<Self, IntegrationError> {
        let shared = Arc::new(Shared::new(options));
        let (control_tx, control_rx) = mpsc::unbounded_channel();
        let (lines_tx, lines_rx) = mpsc::channel(LINE_QUEUE_CAPACITY);
        let actor = Actor {
            shared: Arc::clone(&shared),
            sink,
            control: control_rx,
            lines: lines_rx,
            lines_tx,
            process: None,
            commands: HashMap::new(),
            health: HashMap::new(),
            failures: 0,
            generation: 0,
            restart_at: None,
        };
        tokio::spawn(run_loop(actor));

        let (reply, answer) = oneshot::channel();
        if control_tx.send(Control::Start { reply }).is_err() {
            return Err(IntegrationError::Closed);
        }
        let deadline = shared.options.limits.timeout_handshake + CONSTRUCTOR_MARGIN;
        match timeout(deadline, answer).await {
            Ok(Ok(result)) => result.map(|()| Self {
                control: control_tx,
                shared,
            }),
            Ok(Err(_)) => Err(IntegrationError::Unavailable {
                reason: "o supervisor do plugin encerrou durante o handshake".to_owned(),
            }),
            Err(_) => Err(IntegrationError::Timeout {
                operation: "handshake".to_owned(),
                limit_ms: deadline.as_millis() as u64,
            }),
        }
    }

    pub fn manifest(&self) -> Option<Manifest> {
        self.shared.manifest()
    }

    pub fn state(&self) -> IntegrationState {
        self.shared.state()
    }

    pub fn counters(&self) -> CounterReport {
        self.shared.counters.report()
    }

    pub fn integration(&self) -> (IntegrationId, OrgId, String) {
        (
            self.shared.options.integration,
            self.shared.options.org,
            self.shared.options.kind.clone(),
        )
    }

    pub async fn wait_ready(&self, deadline: Duration) -> Result<(), IntegrationError> {
        let limit = Instant::now() + deadline;
        loop {
            let state = self.shared.state();
            if state.is_ready() {
                return Ok(());
            }
            if let IntegrationState::Down { failures, reason } = &state {
                return Err(IntegrationError::Down {
                    failures: *failures,
                    reason: reason.clone(),
                });
            }
            if state == IntegrationState::Closed {
                return Err(IntegrationError::Closed);
            }
            if Instant::now() >= limit {
                return Err(IntegrationError::Unavailable {
                    reason: format!("o plugin não ficou pronto em {} ms", deadline.as_millis()),
                });
            }
            sleep(Duration::from_millis(10)).await;
        }
    }

    pub async fn shutdown(&self) {
        let (reply, answer) = oneshot::channel();
        if self
            .control
            .send(Control::Close { reply: Some(reply) })
            .is_ok()
        {
            let deadline = KILL_DEADLINE + Duration::from_secs(1);
            let _ = timeout(deadline, answer).await;
        }
    }

    async fn execute(
        &self,
        command: OutboundCommand,
        operation: &'static str,
    ) -> Result<(), IntegrationError> {
        let id = self.shared.next_id();
        let (reply, answer) = oneshot::channel();
        if self
            .control
            .send(Control::Command { id, command, reply })
            .is_err()
        {
            return Err(IntegrationError::Closed);
        }
        let deadline = self.shared.options.limits.command_timeout;
        match timeout(deadline, answer).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(IntegrationError::Unavailable {
                reason: "o supervisor do plugin encerrou".to_owned(),
            }),
            Err(_) => {
                self.shared
                    .counters
                    .increment(&self.shared.counters.timed_out);
                let _ = self.control.send(Control::Abort {
                    reason: format!(
                        "o plugin não respondeu a `{operation}` em {} ms",
                        deadline.as_millis()
                    ),
                });
                Err(IntegrationError::Timeout {
                    operation: operation.to_owned(),
                    limit_ms: deadline.as_millis() as u64,
                })
            }
        }
    }
}

impl Integration for Supervisor {
    fn manifest(&self) -> Option<Manifest> {
        self.shared.manifest()
    }

    fn state(&self) -> IntegrationState {
        self.shared.state()
    }

    fn send_text(&self, command: SendText) -> CommandFuture {
        let this = self.clone();
        Box::pin(async move {
            this.execute(OutboundCommand::Text(command), "enviar_texto")
                .await
        })
    }

    fn send_file(&self, command: SendFile) -> CommandFuture {
        let this = self.clone();
        Box::pin(async move {
            this.execute(OutboundCommand::File(command), "enviar_arquivo")
                .await
        })
    }

    fn set_presence(&self, command: SetPresence) -> CommandFuture {
        let this = self.clone();
        Box::pin(async move {
            this.execute(OutboundCommand::Presence(command), "definir_presenca")
                .await
        })
    }

    fn health(&self) -> HealthFuture {
        let this = self.clone();
        Box::pin(async move {
            let id = this.shared.next_id();
            let (reply, answer) = oneshot::channel();
            if this.control.send(Control::Health { id, reply }).is_err() {
                return Err(IntegrationError::Closed);
            }
            this.shared
                .counters
                .increment(&this.shared.counters.health_checks);
            let deadline = this.shared.options.limits.health_timeout;
            match timeout(deadline, answer).await {
                Ok(Ok(result)) => result,
                Ok(Err(_)) => Err(IntegrationError::Unavailable {
                    reason: "o supervisor do plugin encerrou".to_owned(),
                }),
                Err(_) => {
                    this.shared
                        .counters
                        .increment(&this.shared.counters.timed_out);
                    let _ = this.control.send(Control::Abort {
                        reason: format!(
                            "o plugin não respondeu à saúde em {} ms",
                            deadline.as_millis()
                        ),
                    });
                    Err(IntegrationError::Timeout {
                        operation: "saude".to_owned(),
                        limit_ms: deadline.as_millis() as u64,
                    })
                }
            }
        })
    }
}

enum Control {
    Start {
        reply: oneshot::Sender<Result<(), IntegrationError>>,
    },
    Command {
        id: u64,
        command: OutboundCommand,
        reply: oneshot::Sender<Result<(), IntegrationError>>,
    },
    Health {
        id: u64,
        reply: oneshot::Sender<Result<Health, IntegrationError>>,
    },
    Abort {
        reason: String,
    },
    Close {
        reply: Option<oneshot::Sender<()>>,
    },
}

type PendingCommand = (&'static str, oneshot::Sender<Result<(), IntegrationError>>);

enum Input {
    Line { generation: u64, line: String },
    Discarded { generation: u64 },
    End { generation: u64, reason: String },
}

struct Process {
    generation: u64,
    child: Child,
    stdin: ChildStdin,
}

struct Actor {
    shared: Arc<Shared>,
    sink: Arc<dyn EventSink>,
    control: mpsc::UnboundedReceiver<Control>,
    lines: mpsc::Receiver<Input>,
    lines_tx: mpsc::Sender<Input>,
    process: Option<Process>,
    commands: HashMap<u64, PendingCommand>,
    health: HashMap<u64, oneshot::Sender<Result<Health, IntegrationError>>>,
    failures: u32,
    generation: u64,
    restart_at: Option<Instant>,
}

async fn run_loop(mut actor: Actor) {
    loop {
        let restart_at = actor.restart_at;
        tokio::select! {
            request = actor.control.recv() => {
                let Some(request) = request else { break };
                if !actor.handle_control(request).await {
                    break;
                }
            }
            input = actor.lines.recv() => {
                let Some(input) = input else { break };
                actor.handle_input(input).await;
            }
            () = wait_until(restart_at) => actor.restart().await,
        }
    }
    actor.shutdown().await;
}

async fn wait_until(deadline: Option<Instant>) {
    match deadline {
        Some(deadline) => {
            let now = Instant::now();
            if deadline > now {
                sleep(deadline - now).await;
            }
        }
        None => std::future::pending::<()>().await,
    }
}

impl Actor {
    async fn handle_control(&mut self, request: Control) -> bool {
        match request {
            Control::Start { reply } => {
                let result = self.start_process().await;
                if result.is_err() {
                    self.record_failure().await;
                }
                let _ = reply.send(result);
            }
            Control::Command { id, command, reply } => {
                let operation = command.protocol_name();
                let Some(process) = self.process.as_mut() else {
                    let _ = reply.send(Err(self.no_process_error()));
                    return true;
                };
                self.commands.insert(id, (operation, reply));
                self.shared
                    .counters
                    .increment(&self.shared.counters.commands);
                let message = HostMessage::Command { id, command };
                let cap = self.shared.options.limits.max_message_bytes;
                match write_message(&mut process.stdin, &message, cap).await {
                    Err(error @ IntegrationError::MessageTooLarge { .. }) => {
                        if let Some((_, reply)) = self.commands.remove(&id) {
                            let _ = reply.send(Err(error));
                        }
                    }
                    Err(error) => {
                        tracing::debug!(target: "poph_plugins", %error, "falha ao escrever comando");
                        self.record_failure().await;
                    }
                    Ok(()) => {}
                }
            }
            Control::Health { id, reply } => {
                let Some(process) = self.process.as_mut() else {
                    let _ = reply.send(Err(self.no_process_error()));
                    return true;
                };
                self.health.insert(id, reply);
                let message = HostMessage::Health { id };
                let cap = self.shared.options.limits.max_message_bytes;
                if let Err(error) = write_message(&mut process.stdin, &message, cap).await {
                    tracing::debug!(target: "poph_plugins", %error, "falha ao escrever pedido de saúde");
                    self.record_failure().await;
                }
            }
            Control::Abort { reason } => {
                self.shared.counters.increment(&self.shared.counters.aborts);
                tracing::warn!(target: "poph_plugins", %reason, "abortando o plugin");
                self.record_failure().await;
            }
            Control::Close { reply } => {
                self.shutdown().await;
                if let Some(reply) = reply {
                    let _ = reply.send(());
                }
                return false;
            }
        }
        true
    }

    async fn handle_input(&mut self, input: Input) {
        match input {
            Input::Line { generation, line } => {
                if Some(generation) != self.process.as_ref().map(|process| process.generation) {
                    return;
                }
                self.shared.counters.increment(&self.shared.counters.lines);
                let message: PluginMessage = match serde_json::from_str(&line) {
                    Ok(message) => message,
                    Err(error) => {
                        self.shared.counters.increment(&self.shared.counters.junk);
                        tracing::debug!(target: "poph_plugins", %error, "linha fora do protocolo ignorada");
                        return;
                    }
                };
                self.handle_message(message);
            }
            Input::Discarded { generation } => {
                if Some(generation) == self.process.as_ref().map(|process| process.generation) {
                    self.shared.counters.increment(&self.shared.counters.junk);
                    tracing::debug!(
                        target: "poph_plugins",
                        "linha acima do limite de tamanho ignorada"
                    );
                }
            }
            Input::End { generation, reason } => {
                if Some(generation) != self.process.as_ref().map(|process| process.generation) {
                    return;
                }
                tracing::warn!(target: "poph_plugins", %reason, "o plugin caiu");
                self.record_failure().await;
            }
        }
    }

    fn handle_message(&mut self, message: PluginMessage) {
        match message {
            PluginMessage::HandshakeOk { .. } => {
                self.shared.counters.increment(&self.shared.counters.junk);
            }
            PluginMessage::Result {
                id,
                ok,
                error: failure,
            } => {
                let Some((operation, reply)) = self.commands.remove(&id) else {
                    self.shared.counters.increment(&self.shared.counters.junk);
                    return;
                };
                let result = if ok {
                    self.failures = 0;
                    Ok(())
                } else {
                    let failure = failure.unwrap_or_else(|| poph_types::CommandError {
                        code: "sem_motivo".to_owned(),
                        message: "o plugin recusou sem dizer o motivo".to_owned(),
                        transient: false,
                    });
                    Err(IntegrationError::Refused {
                        operation: operation.to_owned(),
                        code: failure.code,
                        message: failure.message,
                        transient: failure.transient,
                    })
                };
                let _ = reply.send(result);
            }
            PluginMessage::Health { id, health } => {
                let Some(reply) = self.health.remove(&id) else {
                    self.shared.counters.increment(&self.shared.counters.junk);
                    return;
                };
                self.failures = 0;
                let _ = reply.send(Ok(health));
            }
            PluginMessage::Evento { evento: event } => match event.validar() {
                Ok(()) => {
                    self.shared.counters.increment(&self.shared.counters.events);
                    self.sink.receive(IntegrationEvent::Inbound(event));
                }
                Err(error) => {
                    self.shared
                        .counters
                        .increment(&self.shared.counters.invalid_events);
                    tracing::debug!(target: "poph_plugins", %error, "evento de entrada inválido ignorado");
                }
            },
            PluginMessage::State { estado: state } => {
                self.shared.counters.increment(&self.shared.counters.states);
                self.sink.receive(IntegrationEvent::Estado(state));
            }
            PluginMessage::Error {
                code,
                message,
                transient,
            } => {
                self.shared.counters.increment(&self.shared.counters.junk);
                tracing::debug!(target: "poph_plugins", %code, %message, transient, "erro reportado pelo plugin");
                self.sink
                    .receive(IntegrationEvent::Estado(poph_types::StateEvent::Error {
                        code,
                        message,
                        transient,
                    }));
            }
        }
    }

    fn no_process_error(&self) -> IntegrationError {
        match self.shared.state() {
            IntegrationState::Down { failures, reason } => {
                IntegrationError::Down { failures, reason }
            }
            IntegrationState::Closed => IntegrationError::Closed,
            IntegrationState::Restarting { wait_ms, .. } => IntegrationError::Unavailable {
                reason: format!("o plugin está reiniciando (faltam ~{wait_ms} ms)"),
            },
            IntegrationState::Starting | IntegrationState::Ready => IntegrationError::Unavailable {
                reason: "o plugin ainda não está pronto".to_owned(),
            },
        }
    }

    async fn start_process(&mut self) -> Result<(), IntegrationError> {
        if self.process.is_some() {
            return Ok(());
        }
        self.restart_at = None;
        self.shared.set_state(IntegrationState::Starting);
        self.generation = self.generation.saturating_add(1);
        let generation = self.generation;

        let options = &self.shared.options;
        let mut command = Command::new(&options.program);
        command
            .args(&options.arguments)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        if let Some(directory) = &options.directory {
            command.current_dir(directory);
        }
        command.env_clear();
        for name in ["PATH", "HOME", "LANG", "LC_ALL", "TZ", "TMPDIR"] {
            if let Some(value) = std::env::var_os(name) {
                command.env(name, value);
            }
        }
        for (name, value) in &options.environment {
            command.env(name, value);
        }
        let mut child = command
            .spawn()
            .map_err(|error| IntegrationError::StartFailed {
                program: options.program.display().to_string(),
                reason: error.to_string(),
            })?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| IntegrationError::StartFailed {
                program: options.program.display().to_string(),
                reason: "sem entrada padrão".to_owned(),
            })?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| IntegrationError::StartFailed {
                program: options.program.display().to_string(),
                reason: "sem saída padrão".to_owned(),
            })?;
        if let Some(stderr) = child.stderr.take() {
            tokio::spawn(drain_stderr(stderr));
        }
        tokio::spawn(read_output(
            stdout,
            self.lines_tx.clone(),
            generation,
            self.shared.options.limits.max_message_bytes,
        ));

        let mut process = Process {
            generation,
            child,
            stdin,
        };
        let handshake = HostMessage::Handshake(poph_types::Handshake {
            protocol: PROTOCOL_VERSION,
            integration: options.integration,
            org: options.org,
            kind: options.kind.clone(),
            config: options.config.clone(),
            secrets: options
                .secrets
                .iter()
                .map(|(name, value)| (name.clone(), value.expose().clone()))
                .collect(),
        });
        write_message(
            &mut process.stdin,
            &handshake,
            options.limits.max_message_bytes,
        )
        .await?;

        let deadline = Instant::now() + options.limits.timeout_handshake;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(IntegrationError::Timeout {
                    operation: "handshake".to_owned(),
                    limit_ms: options.limits.timeout_handshake.as_millis() as u64,
                });
            }
            match timeout(remaining, self.lines.recv()).await {
                Err(_) => {
                    return Err(IntegrationError::Timeout {
                        operation: "handshake".to_owned(),
                        limit_ms: options.limits.timeout_handshake.as_millis() as u64,
                    });
                }
                Ok(None) => {
                    return Err(IntegrationError::Unavailable {
                        reason: "o leitor do plugin encerrou durante o handshake".to_owned(),
                    });
                }
                Ok(Some(Input::End {
                    generation: origin,
                    reason,
                })) if origin == generation => {
                    return Err(IntegrationError::HandshakeRefused {
                        reason: format!("o plugin terminou antes do handshake: {reason}"),
                    });
                }
                Ok(Some(Input::Line {
                    generation: origin,
                    line,
                })) if origin == generation => match serde_json::from_str::<PluginMessage>(&line) {
                    Ok(PluginMessage::HandshakeOk {
                        protocolo: protocol,
                        manifest,
                    }) => {
                        if protocol != PROTOCOL_VERSION {
                            return Err(IntegrationError::IncompatibleProtocol {
                                expected: PROTOCOL_VERSION,
                                received: protocol,
                            });
                        }
                        manifest.validar()?;
                        self.shared.set_manifest(manifest);
                        self.shared
                            .counters
                            .increment(&self.shared.counters.handshakes);
                        self.process = Some(process);
                        self.shared.set_state(IntegrationState::Ready);
                        return Ok(());
                    }
                    _ => {
                        self.shared.counters.increment(&self.shared.counters.junk);
                    }
                },
                Ok(Some(_)) => {}
            }
        }
    }

    async fn restart(&mut self) {
        self.restart_at = None;
        if self.shared.state().is_closed() {
            return;
        }
        match self.start_process().await {
            Ok(()) => {
                self.shared
                    .counters
                    .increment(&self.shared.counters.restarts);
                tracing::info!(target: "poph_plugins", "plugin reiniciado");
            }
            Err(error) => {
                tracing::warn!(target: "poph_plugins", %error, "o plugin não subiu");
                self.record_failure().await;
            }
        }
    }

    async fn record_failure(&mut self) {
        self.failures = self.failures.saturating_add(1);
        self.shared
            .counters
            .increment(&self.shared.counters.failures);
        if let Some(process) = self.process.take() {
            kill_process(process).await;
        }
        let unavailable = IntegrationError::Unavailable {
            reason: "o plugin caiu; a mensagem pode ser retentada".to_owned(),
        };
        for (_, (_, reply)) in self.commands.drain() {
            let _ = reply.send(Err(unavailable.clone()));
        }
        for (_, reply) in self.health.drain() {
            let _ = reply.send(Err(unavailable.clone()));
        }
        let limits = &self.shared.options.limits;
        if self.failures >= limits.max_failures {
            let reason = format!(
                "{} falhas seguidas (teto {}); não reinicia mais sozinho",
                self.failures, limits.max_failures
            );
            tracing::error!(target: "poph_plugins", %reason, "integração marcada como caída");
            self.shared.set_state(IntegrationState::Down {
                failures: self.failures,
                reason,
            });
            self.restart_at = None;
        } else {
            let delay = limits.backoff.delay(self.failures);
            self.shared.set_state(IntegrationState::Restarting {
                failures: self.failures,
                wait_ms: delay.as_millis() as u64,
            });
            self.restart_at = Some(Instant::now() + delay);
        }
    }

    async fn shutdown(&mut self) {
        self.restart_at = None;
        if let Some(process) = self.process.take() {
            let Process {
                child,
                stdin,
                generation: _,
            } = process;
            let mut stdin = stdin;
            let cap = self.shared.options.limits.max_message_bytes;
            let _ = write_message(&mut stdin, &HostMessage::Shutdown, cap).await;
            drop(stdin);
            let mut child = child;
            if timeout(Duration::from_millis(500), child.wait())
                .await
                .is_err()
            {
                let _ = child.start_kill();
                let _ = timeout(KILL_DEADLINE, child.wait()).await;
            }
        }
        let closed = IntegrationError::Closed;
        for (_, (_, reply)) in self.commands.drain() {
            let _ = reply.send(Err(closed.clone()));
        }
        for (_, reply) in self.health.drain() {
            let _ = reply.send(Err(closed.clone()));
        }
        self.shared.set_state(IntegrationState::Closed);
    }
}

async fn write_message(
    stdin: &mut ChildStdin,
    message: &HostMessage,
    cap: usize,
) -> Result<(), IntegrationError> {
    let mut line = serde_json::to_string(message).map_err(|error| IntegrationError::Protocol {
        reason: format!("não consegui serializar a mensagem: {error}"),
    })?;
    if line.len() + 1 > cap {
        return Err(IntegrationError::MessageTooLarge {
            bytes: line.len() + 1,
            cap,
        });
    }
    line.push('\n');
    stdin
        .write_all(line.as_bytes())
        .await
        .map_err(|error| IntegrationError::Unavailable {
            reason: format!("o plugin não aceitou a mensagem: {error}"),
        })?;
    stdin
        .flush()
        .await
        .map_err(|error| IntegrationError::Unavailable {
            reason: format!("o plugin não aceitou a mensagem: {error}"),
        })
}

async fn kill_process(mut process: Process) {
    let _ = process.child.start_kill();
    let _ = timeout(KILL_DEADLINE, process.child.wait()).await;
}

async fn read_output(
    output: tokio::process::ChildStdout,
    tx: mpsc::Sender<Input>,
    generation: u64,
    limit: usize,
) {
    let mut reader = LineReader::new(output, limit);
    loop {
        let input = match reader.next_line().await {
            Ok(NextLine::Line(line)) => Input::Line { generation, line },
            Ok(NextLine::Discarded) => Input::Discarded { generation },
            Ok(NextLine::End) => Input::End {
                generation,
                reason: "o plugin fechou a saída padrão".to_owned(),
            },
            Err(error) => Input::End {
                generation,
                reason: format!("falha ao ler a saída do plugin: {error}"),
            },
        };
        let end = matches!(input, Input::End { .. });
        if tx.send(input).await.is_err() {
            return;
        }
        if end {
            return;
        }
    }
}

async fn drain_stderr(mut stderr: tokio::process::ChildStderr) {
    use tokio::io::AsyncReadExt;

    let mut chunk = [0u8; 4096];
    loop {
        match stderr.read(&mut chunk).await {
            Ok(0) | Err(_) => return,
            Ok(read) => {
                let text = String::from_utf8_lossy(&chunk[..read]);
                let text = text.trim();
                if !text.is_empty() {
                    tracing::debug!(target: "poph_plugins", "plugin (stderr): {text}");
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_grows_and_stops_at_the_cap() {
        let backoff = Backoff {
            base: Duration::from_millis(100),
            factor: 2,
            cap: Duration::from_millis(500),
        };
        assert_eq!(backoff.delay(0), Duration::from_millis(100));
        assert_eq!(backoff.delay(1), Duration::from_millis(100));
        assert_eq!(backoff.delay(2), Duration::from_millis(200));
        assert_eq!(backoff.delay(3), Duration::from_millis(400));
        assert_eq!(backoff.delay(4), Duration::from_millis(500));
        assert_eq!(backoff.delay(30), Duration::from_millis(500));
    }

    #[test]
    fn default_limits_are_the_documented_ones() {
        let limits = SupervisorLimits::default();
        assert_eq!(limits.max_message_bytes, 1024 * 1024);
        assert_eq!(limits.timeout_handshake, Duration::from_secs(5));
        assert_eq!(limits.command_timeout, Duration::from_secs(10));
        assert_eq!(limits.health_timeout, Duration::from_secs(3));
        assert_eq!(limits.max_failures, 5);
    }

    #[test]
    fn ready_is_the_only_state_that_accepts_commands() {
        assert!(IntegrationState::Ready.is_ready());
        for state in [
            IntegrationState::Starting,
            IntegrationState::Restarting {
                failures: 1,
                wait_ms: 10,
            },
            IntegrationState::Down {
                failures: 5,
                reason: "x".to_owned(),
            },
            IntegrationState::Closed,
        ] {
            assert!(!state.is_ready(), "{state:?}");
        }
        assert!(
            IntegrationState::Down {
                failures: 5,
                reason: "x".to_owned()
            }
            .is_closed()
        );
        assert!(
            !IntegrationState::Restarting {
                failures: 1,
                wait_ms: 10
            }
            .is_closed()
        );
    }
}
