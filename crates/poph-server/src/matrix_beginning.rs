use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use poph_matrix::config::{MatrixError, MatrixModule};
use poph_matrix::rooms::PortError;
use poph_matrix::spaces::{SpaceOrigin, SpacesPort, StoredSpace};
use poph_matrix::{GhostIds, MatrixClient, Spaces, VirtualUsers};
use poph_store::{SpaceOrigin as StoredOrigin, Store, StoreError};
use poph_types::{ConversationId, DepartmentId, OrgId};
use tokio::sync::watch;

use crate::ServerState;
use crate::api::matrix_as::AsReceiver;
use crate::matrix_expiry::AssetExpiry;
use crate::matrix_failure_notice::{
    DEFAULT_PERIOD, FailureNotice, HarnessFailuresPort, SynapseNotice,
};
use crate::matrix_mirror::{MatrixMirror, SynapseMirrorTarget};
use crate::matrix_response::AttendantResponse;
use crate::matrix_state::{ConversationStates, StateStorePort, SynapseTarget};

pub type ModuleSpaces = poph_matrix::Spaces<SpaceStorePort>;

pub const PROBE_PERIOD: Duration = Duration::from_secs(30);

pub const PROBE_DELAY_CAP: Duration = Duration::from_secs(60);

pub const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug)]
pub struct ConnectionSignal {
    connected: AtomicBool,
}

impl ConnectionSignal {
    fn fresh() -> Self {
        Self {
            connected: AtomicBool::new(false),
        }
    }

    pub fn connected(&self) -> bool {
        self.connected.load(Ordering::Relaxed)
    }

    pub fn degraded(&self) -> bool {
        !self.connected()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StartupSettings {
    pub probe_period: Duration,
    pub probe_delay_cap: Duration,
    pub shutdown_timeout: Duration,
}

impl Default for StartupSettings {
    fn default() -> Self {
        Self::defaults()
    }
}

impl StartupSettings {
    pub const fn defaults() -> Self {
        Self {
            probe_period: PROBE_PERIOD,
            probe_delay_cap: PROBE_DELAY_CAP,
            shutdown_timeout: SHUTDOWN_TIMEOUT,
        }
    }
}

pub struct RunningMatrix {
    mirror: Arc<MatrixMirror<SynapseMirrorTarget>>,
    conversation_states: Arc<ConversationStates<SynapseTarget<SpaceStorePort>, StateStorePort>>,
    failure_notice: Arc<FailureNotice>,
    expiration: Option<Arc<AssetExpiry>>,
    connection: Arc<ConnectionSignal>,
    stop_probe: watch::Sender<bool>,
    tasks: Vec<tokio::task::JoinHandle<()>>,
    shutdown_timeout: Duration,
}

impl RunningMatrix {
    pub fn connection(&self) -> Arc<ConnectionSignal> {
        Arc::clone(&self.connection)
    }

    pub fn connected(&self) -> bool {
        self.connection.connected()
    }

    pub fn with_shutdown_timeout(mut self, timeout: Duration) -> Self {
        self.shutdown_timeout = timeout;
        self
    }

    pub async fn stop(self) {
        self.mirror.request_stop();
        self.conversation_states.ask_stop();
        self.failure_notice.ask_stop();
        if let Some(expiration) = &self.expiration {
            expiration.ask_stop();
        }
        self.stop_probe.send_replace(true);
        for task in self.tasks {
            if tokio::time::timeout(self.shutdown_timeout, task)
                .await
                .is_err()
            {
                tracing::warn!(
                    target: "poph_server::matrix_inicio",
                    reason = "prazo_de_parada_estourado",
                    "laço do módulo Matrix não terminou no prazo; abortado"
                );
            }
        }
        tracing::info!(
            target: "poph_server::matrix_inicio",
            "módulo Matrix parado"
        );
    }
}

pub fn start(state: &mut ServerState) -> Option<RunningMatrix> {
    let module = match MatrixModule::prepare(state.config.matrix.clone()) {
        Ok(Some(module)) => module,
        Ok(None) => {
            return None;
        }
        Err(error) => {
            tracing::error!(
                target: "poph_server::matrix_inicio",
                reason = module_error_code(&error),
                "módulo Matrix ligado na configuração, mas não subiu; o servidor segue sem ele"
            );
            return None;
        }
    };
    start_with(state, module)
}

pub fn start_with(state: &mut ServerState, module: MatrixModule) -> Option<RunningMatrix> {
    start_with_settings(state, module, StartupSettings::defaults())
}

pub fn start_with_settings(
    state: &mut ServerState,
    module: MatrixModule,
    settings: StartupSettings,
) -> Option<RunningMatrix> {
    let store = Arc::clone(&state.store);
    let metrics = state.matrix_metrics().ok().flatten();

    let client = match MatrixClient::from_parts(&module.config, &module.secrets) {
        Ok(client) => client,
        Err(error) => {
            tracing::error!(
                target: "poph_server::matrix_inicio",
                reason = call_error_code(&error),
                "cliente do Matrix não foi montado; o módulo não sobe"
            );
            return None;
        }
    };
    let ids = match GhostIds::of_config(&module.config) {
        Ok(ids) => ids,
        Err(_) => {
            tracing::error!(
                target: "poph_server::matrix_inicio",
                reason = "fantasmas_invalidos",
                "ids dos usuários virtuais inválidos; o módulo não sobe"
            );
            return None;
        }
    };
    let users = Arc::new(VirtualUsers::new(
        client.clone(),
        ids.clone(),
        poph_matrix::DEFAULT_CACHE_CAP,
    ));

    let mut destination =
        match SynapseMirrorTarget::assemble(Arc::clone(&store), client.clone(), &module.config) {
            Ok(destination) => destination,
            Err(_) => {
                tracing::error!(
                    target: "poph_server::matrix_inicio",
                    reason = "espelho_nao_montado",
                    "provisão de sala não foi montada; o módulo não sobe"
                );
                return None;
            }
        };
    if let Some(metrics) = &metrics {
        destination = destination.with_metrics(Arc::clone(metrics));
    }
    match Spaces::of_config(
        client.clone(),
        SpaceStorePort(Arc::clone(&store)),
        &module.config,
    ) {
        Ok(mirror_spaces) => destination = destination.with_spaces(mirror_spaces),
        Err(_) => {
            tracing::warn!(
                target: "poph_server::matrix_inicio",
                reason = "espacos_do_espelho_nao_montados",
                "o espelho cria a sala sem espaço; o laço de estado posiciona na próxima alteração"
            );
        }
    }
    let mut mirror = match MatrixMirror::new(Arc::clone(&store), destination, &module.config) {
        Ok(mirror) => mirror,
        Err(_) => {
            tracing::error!(
                target: "poph_server::matrix_inicio",
                reason = "espelho_nao_montado",
                "laço do espelho não foi montado; o módulo não sobe"
            );
            return None;
        }
    };
    if let Some(metrics) = &metrics {
        mirror = mirror.with_metrics(Arc::clone(metrics));
    }
    mirror = mirror.with_blob_folder(state.matrix_blob_folder().to_owned());

    let spaces = match Spaces::of_config(
        client.clone(),
        SpaceStorePort(Arc::clone(&store)),
        &module.config,
    ) {
        Ok(spaces) => spaces,
        Err(_) => {
            tracing::error!(
                target: "poph_server::matrix_inicio",
                reason = "espacos_nao_montados",
                "espaços por departamento não foram montados; o módulo não sobe"
            );
            return None;
        }
    };
    match Spaces::of_config(
        client.clone(),
        SpaceStorePort(Arc::clone(&store)),
        &module.config,
    ) {
        Ok(api_spaces) => state.set_matrix_spaces(Arc::new(api_spaces)),
        Err(_) => {
            tracing::warn!(
                target: "poph_server::matrix_inicio",
                reason = "espacos_da_api_nao_montados",
                "API administrativa do módulo fica sem os espaços; o resto do módulo segue"
            );
        }
    }

    let mut failure_notice = FailureNotice::new(
        Arc::new(HarnessFailuresPort::fresh(Arc::clone(&store))),
        SynapseNotice::new(client.clone(), ids.clone()),
    );
    if let Some(metrics) = &metrics {
        failure_notice = failure_notice.with_metrics(Arc::clone(metrics));
    }

    let state_target = SynapseTarget::new(client.clone(), users, spaces);
    let mut conversation_states = match ConversationStates::new(
        StateStorePort::fresh(Arc::clone(&store)),
        state_target,
        &module.config,
    ) {
        Ok(worker) => worker,
        Err(_) => {
            tracing::error!(
                target: "poph_server::matrix_inicio",
                reason = "estado_nao_montado",
                "laço do estado da conversa não foi montado; o módulo não sobe"
            );
            return None;
        }
    };
    if let Some(metrics) = &metrics {
        conversation_states = conversation_states.with_metrics(Arc::clone(metrics));
    }

    let expiration = module_expiry(
        Arc::clone(&state.store),
        client.clone(),
        &module.config,
        metrics.clone(),
        poph_matrix::config::environment_admin_token(),
    )
    .map(Arc::new);

    let mut delivery = match AttendantResponse::from_module(Arc::clone(&store), &module) {
        Ok(delivery) => delivery,
        Err(error) => {
            tracing::error!(
                target: "poph_server::matrix_inicio",
                reason = call_error_code(&error),
                "entrega da resposta do atendente não foi montada; o módulo não sobe"
            );
            return None;
        }
    };
    if let Some(metrics) = &metrics {
        delivery = delivery.with_metrics(Arc::clone(metrics));
    }
    delivery = delivery.with_blob_folder(state.matrix_blob_folder().to_owned());
    let receiver = AsReceiver::of_module(module, Arc::new(delivery));
    state.set_matrix_as(Arc::new(receiver));

    let mirror = Arc::new(mirror);
    let conversation_states = Arc::new(conversation_states);
    let failure_notice = Arc::new(failure_notice);
    let connection = Arc::new(ConnectionSignal::fresh());
    state.set_matrix_connection_signal(Arc::clone(&connection));
    let (stop_probe, probe_signal) = watch::channel(false);
    let mut tasks = vec![
        tokio::spawn(Arc::clone(&mirror).run()),
        tokio::spawn(Arc::clone(&conversation_states).run()),
        tokio::spawn(Arc::clone(&failure_notice).run(DEFAULT_PERIOD)),
        tokio::spawn(probe(
            client,
            Arc::clone(&connection),
            settings.probe_period,
            settings.probe_delay_cap,
            probe_signal,
        )),
    ];
    if let Some(expiration) = &expiration {
        tasks.push(tokio::spawn(Arc::clone(expiration).run()));
    }
    tracing::info!(
        target: "poph_server::matrix_inicio",
        server = %state.config.matrix.as_ref().map(|matrix| matrix.server_name.as_str()).unwrap_or_default(),
        interval_ms = state
            .config
            .matrix
            .as_ref()
            .map(|matrix| matrix.interval_ms)
            .unwrap_or_default(),
        asset_expiry = expiration.is_some(),
        "módulo Matrix ligado: espelho, estado e receptor do Application Service no ar"
    );
    Some(RunningMatrix {
        mirror,
        conversation_states,
        failure_notice,
        expiration,
        connection,
        stop_probe,
        tasks,
        shutdown_timeout: settings.shutdown_timeout,
    })
}

pub fn module_expiry(
    store: Arc<Store>,
    client: MatrixClient,
    config: &poph_types::MatrixConfig,
    metrics: Option<Arc<crate::matrix_metrics::MetricsMatrix>>,
    token: Option<poph_types::Secret>,
) -> Option<AssetExpiry> {
    let Some(token) = token else {
        tracing::warn!(
            target: "poph_server::matrix_inicio",
            reason = "token_de_administrador_ausente",
            variable = poph_matrix::config::VAR_ADMIN_TOKEN,
            "expiração da mídia desligada: defina POPH_MATRIX_ADMIN_TOKEN para apagar do Synapse a mídia vencida (o resto do módulo Matrix segue normal)"
        );
        return None;
    };
    match AssetExpiry::new(Arc::clone(&store), client, token, config) {
        Ok(expiration) => {
            tracing::info!(
                target: "poph_server::matrix_inicio",
                retention_days = config.attachment_retention_days,
                "expiração da mídia ligada: o que passar do prazo é redigido na sala e apagado do Synapse"
            );
            Some(match metrics {
                Some(metrics) => expiration.with_metrics(metrics),
                None => expiration,
            })
        }
        Err(_) => {
            tracing::error!(
                target: "poph_server::matrix_inicio",
                reason = "expiracao_nao_montada",
                "expiração da mídia não foi montada; o módulo Matrix segue sem ela"
            );
            None
        }
    }
}

async fn probe(
    client: MatrixClient,
    connection: Arc<ConnectionSignal>,
    period: Duration,
    cap: Duration,
    mut stop: watch::Receiver<bool>,
) {
    let mut failures = 0u32;
    loop {
        if *stop.borrow() {
            break;
        }
        let result = client.check_connection().await;
        let wait = match result {
            Ok(()) => {
                if connection.degraded() {
                    tracing::info!(
                        target: "poph_server::matrix_inicio",
                        "Synapse respondeu: módulo Matrix conectado"
                    );
                }
                connection.connected.store(true, Ordering::Relaxed);
                failures = 0;
                period
            }
            Err(error) => {
                if connection.connected() {
                    tracing::warn!(
                        target: "poph_server::matrix_inicio",
                        reason = call_error_code(&error),
                        "Synapse parou de responder: módulo Matrix degradado, sem derrubar o servidor"
                    );
                }
                connection.connected.store(false, Ordering::Relaxed);
                failures = failures.saturating_add(1);
                let factor = 1u32.checked_shl(failures.min(10)).unwrap_or(u32::MAX);
                period.saturating_mul(factor).min(cap)
            }
        };
        tokio::select! {
            _ = tokio::time::sleep(wait) => {}
            _ = stop.changed() => break,
        }
    }
}

pub struct SpaceStorePort(pub Arc<Store>);

impl SpacesPort for SpaceStorePort {
    fn space(
        &self,
        org: OrgId,
        department: DepartmentId,
    ) -> impl std::future::Future<Output = Result<Option<StoredSpace>, PortError>> + Send {
        let store = Arc::clone(&self.0);
        async move {
            let space = store.matrix().space(org, department).map_err(port_error)?;
            Ok(space.map(|space| StoredSpace {
                room_id: space.room_id,
                alias: space.alias,
                origin: from_stored_origin(space.origin),
            }))
        }
    }

    fn ensure_space(
        &self,
        org: OrgId,
        department: DepartmentId,
        room_id: &str,
        alias: &str,
        origin: SpaceOrigin,
    ) -> impl std::future::Future<Output = Result<StoredSpace, PortError>> + Send {
        let store = Arc::clone(&self.0);
        let room_id = room_id.to_owned();
        let alias = alias.to_owned();
        async move {
            let stored = store
                .matrix()
                .ensure_space(org, department, &room_id, &alias, to_stored_origin(origin))
                .map_err(port_error)?;
            Ok(StoredSpace {
                room_id: stored.room_id,
                alias: stored.alias,
                origin: from_stored_origin(stored.origin),
            })
        }
    }

    fn room_space(
        &self,
        org: OrgId,
        conversation: ConversationId,
    ) -> impl std::future::Future<Output = Result<Option<String>, PortError>> + Send {
        let store = Arc::clone(&self.0);
        async move {
            let room = store.matrix().room(org, conversation).map_err(port_error)?;
            Ok(room.and_then(|room| room.space))
        }
    }

    fn set_room_space(
        &self,
        org: OrgId,
        conversation: ConversationId,
        space: Option<&str>,
    ) -> impl std::future::Future<Output = Result<(), PortError>> + Send {
        let store = Arc::clone(&self.0);
        let space = space.map(str::to_owned);
        async move {
            store
                .matrix()
                .set_space_of_room(org, conversation, space.as_deref())
                .map(|_| ())
                .map_err(port_error)
        }
    }
}

fn port_error(_: StoreError) -> PortError {
    PortError::fresh("armazenamento do módulo Matrix indisponível")
}

fn from_stored_origin(origin: StoredOrigin) -> SpaceOrigin {
    match origin {
        StoredOrigin::Automatic => SpaceOrigin::Automatic,
        StoredOrigin::Linked => SpaceOrigin::Linked,
    }
}

fn to_stored_origin(origin: SpaceOrigin) -> StoredOrigin {
    match origin {
        SpaceOrigin::Automatic => StoredOrigin::Automatic,
        SpaceOrigin::Linked => StoredOrigin::Linked,
    }
}

fn module_error_code(error: &MatrixError) -> &'static str {
    match error {
        MatrixError::Configuration(_) => "configuracao_invalida",
        MatrixError::TokenMissing { .. } => "token_ausente",
    }
}

fn call_error_code(error: &poph_matrix::CallError) -> &'static str {
    use poph_matrix::CallError;
    match error {
        CallError::Configuration(_) => "configuracao_invalida",
        CallError::Network { .. } => "rede",
        CallError::Deadline { .. } => "prazo",
        CallError::Response { .. } => "resposta_do_synapse",
        CallError::InvalidResponse { .. } => "resposta_invalida",
    }
}
