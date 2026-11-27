use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use poph_types::{MatrixConfig, Secret};
use reqwest::Method;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderValue};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::config::MatrixSecrets;
use crate::error::{AssetError, CallError};

const API: &str = "/_matrix/client/v3";

pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);

pub const MAX_WAIT_429: Duration = Duration::from_secs(60);

const OLD_UPLOAD_PATH: &str = "/_matrix/media/v3/upload";

const NEW_UPLOAD_PATH: &str = "/_matrix/client/v1/media/upload";

const OLD_DOWNLOAD_PATH: &str = "/_matrix/media/v3/download";

const NEW_DOWNLOAD_PATH: &str = "/_matrix/client/v1/media/download";

const ADMIN_ASSET: &str = "/_synapse/admin/v1/media";

const ERRCODE_NOT_RECOGNIZED: &str = "M_UNRECOGNIZED";

const BODY_CAP: u64 = 1024 * 1024;

const MESSAGE_SIZE: usize = 160;

#[derive(Clone, Debug)]
pub struct ClientConfig {
    pub url_base: String,
    pub server_name: String,
    pub timeout: Duration,
    as_token: Secret,
}

impl ClientConfig {
    pub fn new(
        url_base: impl Into<String>,
        server_name: impl Into<String>,
        as_token: Secret,
    ) -> Result<Self, CallError> {
        let url_base = url_base.into().trim().trim_end_matches('/').to_owned();
        if url_base.is_empty() {
            return Err(CallError::Configuration(
                "`matrix.homeserver_url` está vazio".to_owned(),
            ));
        }
        if !(url_base.starts_with("http://") || url_base.starts_with("https://")) {
            return Err(CallError::Configuration(
                "`matrix.homeserver_url` deve começar com http:// ou https://".to_owned(),
            ));
        }
        let server_name = server_name.into().trim().to_owned();
        if server_name.is_empty() {
            return Err(CallError::Configuration(
                "`matrix.server_name` está vazio".to_owned(),
            ));
        }
        if as_token.expose().trim().is_empty() {
            return Err(CallError::Configuration(
                "o token do Application Service (`POPH_MATRIX_AS_TOKEN`) está vazio".to_owned(),
            ));
        }
        Ok(Self {
            url_base,
            server_name,
            timeout: DEFAULT_TIMEOUT,
            as_token,
        })
    }

    pub fn from_parts(config: &MatrixConfig, secrets: &MatrixSecrets) -> Result<Self, CallError> {
        Self::new(
            config.homeserver_url.clone(),
            config.server_name.clone(),
            secrets.as_token.clone(),
        )
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }
}

#[derive(Clone, Copy)]
enum Body<'a> {
    None,
    Json(&'a Value),
    Bytes { mime: &'a str, bytes: &'a [u8] },
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct CreateRoom {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub room_alias_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topic: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visibility: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub creation_content: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub initial_state: Option<Vec<Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub invite: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preset: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub power_level_content_override: Option<Value>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
pub struct RoomMember {
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
pub struct Profile {
    pub displayname: Option<String>,
    pub avatar_url: Option<String>,
}

#[derive(Deserialize)]
struct JoinedResponse {
    joined: BTreeMap<String, RoomMember>,
}

#[derive(Clone, Copy)]
struct Request<'a> {
    method: &'a Method,
    path: &'a str,
    fixed_query: &'a [(&'a str, &'a str)],
    user: Option<&'a str>,
    body: Body<'a>,
    operation: &'static str,
}

struct Internal {
    http: reqwest::Client,
    http_without_redirect: reqwest::Client,
    config: ClientConfig,
}

#[derive(Clone)]
pub struct MatrixClient(Arc<Internal>);

impl fmt::Debug for MatrixClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ClienteMatrix")
            .field("url_base", &self.0.config.url_base)
            .field("server_name", &self.0.config.server_name)
            .field("timeout", &self.0.config.timeout)
            .field("as_token", &self.0.config.as_token)
            .finish()
    }
}

impl MatrixClient {
    pub fn new(config: ClientConfig) -> Result<Self, CallError> {
        let http = reqwest::Client::builder()
            .timeout(config.timeout)
            .user_agent(format!("poph-harness/{}", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|error| {
                CallError::Configuration(format!("cliente HTTP do Matrix: {error}"))
            })?;
        let http_without_redirect = reqwest::Client::builder()
            .timeout(config.timeout)
            .user_agent(format!("poph-harness/{}", env!("CARGO_PKG_VERSION")))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|error| {
                CallError::Configuration(format!("cliente HTTP de mídia do Matrix: {error}"))
            })?;
        Ok(Self(Arc::new(Internal {
            http,
            http_without_redirect,
            config,
        })))
    }

    pub fn from_parts(config: &MatrixConfig, secrets: &MatrixSecrets) -> Result<Self, CallError> {
        Self::new(ClientConfig::from_parts(config, secrets)?)
    }

    pub fn with_timeout(&self, timeout: Duration) -> Result<Self, CallError> {
        let mut config = self.0.config.clone();
        config.timeout = timeout;
        Self::new(config)
    }

    pub fn url_base(&self) -> &str {
        &self.0.config.url_base
    }

    pub fn server_name(&self) -> &str {
        &self.0.config.server_name
    }

    pub async fn create_room(&self, user: &str, room: &CreateRoom) -> Result<String, CallError> {
        let body = serde_json::to_value(room).map_err(|error| {
            CallError::Configuration(format!("corpo do createRoom não serializou: {error}"))
        })?;
        let value = self
            .call(
                Method::POST,
                format!("{API}/createRoom"),
                Some(user),
                Body::Json(&body),
                "createRoom",
            )
            .await?;
        required_text(&value, "room_id", "createRoom")
    }

    pub async fn invite(&self, user: &str, room: &str, guest: &str) -> Result<(), CallError> {
        self.call(
            Method::POST,
            format!("{API}/rooms/{}/invite", encode(room)),
            Some(user),
            Body::Json(&json!({ "user_id": guest })),
            "invite",
        )
        .await
        .map(|_| ())
    }

    pub async fn enter(&self, user: &str, room_or_alias: &str) -> Result<String, CallError> {
        let value = self
            .call(
                Method::POST,
                format!("{API}/join/{}", encode(room_or_alias)),
                Some(user),
                Body::Json(&json!({})),
                "join",
            )
            .await?;
        required_text(&value, "room_id", "join")
    }

    pub async fn resolver_alias(&self, alias: &str) -> Result<String, CallError> {
        let value = self
            .call(
                Method::GET,
                format!("{API}/directory/room/{}", encode(alias)),
                None,
                Body::None,
                "directory/room",
            )
            .await?;
        required_text(&value, "room_id", "directory/room")
    }

    pub async fn members(
        &self,
        user: &str,
        room: &str,
    ) -> Result<BTreeMap<String, RoomMember>, CallError> {
        let value = self
            .call(
                Method::GET,
                format!("{API}/rooms/{}/joined_members", encode(room)),
                Some(user),
                Body::None,
                "joined_members",
            )
            .await?;
        serde_json::from_value::<JoinedResponse>(value)
            .map(|junta| junta.joined)
            .map_err(|error| CallError::InvalidResponse {
                operation: "joined_members",
                reason: format!("corpo fora do contrato: {error}"),
            })
    }

    pub async fn send_event(
        &self,
        user: &str,
        room: &str,
        kind: &str,
        txn_id: &str,
        body: &Value,
    ) -> Result<String, CallError> {
        let value = self
            .call(
                Method::PUT,
                format!(
                    "{API}/rooms/{}/send/{}/{}",
                    encode(room),
                    encode(kind),
                    encode(txn_id)
                ),
                Some(user),
                Body::Json(body),
                "send",
            )
            .await?;
        required_text(&value, "event_id", "send")
    }

    pub async fn send_state(
        &self,
        user: &str,
        room: &str,
        kind: &str,
        key: Option<&str>,
        body: &Value,
    ) -> Result<String, CallError> {
        let value = self
            .call(
                Method::PUT,
                state_path(room, kind, key),
                Some(user),
                Body::Json(body),
                "state",
            )
            .await?;
        required_text(&value, "event_id", "state")
    }

    pub async fn read_state(
        &self,
        user: &str,
        room: &str,
        kind: &str,
        key: Option<&str>,
    ) -> Result<Value, CallError> {
        self.call(
            Method::GET,
            state_path(room, kind, key),
            Some(user),
            Body::None,
            "state (leitura)",
        )
        .await
    }

    pub async fn room_state(&self, user: &str, room: &str) -> Result<Vec<Value>, CallError> {
        let value = self
            .call(
                Method::GET,
                format!("{API}/rooms/{}/state", encode(room)),
                Some(user),
                Body::None,
                "state (sala inteira)",
            )
            .await?;
        match value {
            Value::Array(events) => Ok(events),
            _ => Err(CallError::InvalidResponse {
                operation: "state (sala inteira)",
                reason: "a resposta não é uma lista de eventos".to_owned(),
            }),
        }
    }

    pub async fn space_child(
        &self,
        user: &str,
        space: &str,
        child_room: &str,
        order: Option<&str>,
    ) -> Result<String, CallError> {
        let mut body = json!({ "via": [self.0.config.server_name] });
        if let Some(order) = order {
            body["order"] = Value::String(order.to_owned());
        }
        self.send_state(user, space, "m.space.child", Some(child_room), &body)
            .await
    }

    pub async fn hierarquia(&self, user: &str, space: &str) -> Result<Vec<String>, CallError> {
        let value = self
            .call_with_query(
                Method::GET,
                format!("/_matrix/client/v1/rooms/{}/hierarchy", encode(space)),
                &[("limit", "100")],
                Some(user),
                Body::None,
                "hierarchy",
            )
            .await?;
        let rooms = value
            .get("rooms")
            .and_then(Value::as_array)
            .ok_or_else(|| CallError::InvalidResponse {
                operation: "hierarchy",
                reason: "campo `rooms` ausente".to_owned(),
            })?;
        Ok(rooms
            .iter()
            .filter_map(|room| room.get("room_id").and_then(Value::as_str))
            .map(str::to_owned)
            .collect())
    }

    pub async fn profile(&self, user_id: &str) -> Result<Profile, CallError> {
        let value = self
            .call(
                Method::GET,
                format!("{API}/profile/{}", encode(user_id)),
                None,
                Body::None,
                "profile",
            )
            .await?;
        serde_json::from_value::<Profile>(value).map_err(|error| CallError::InvalidResponse {
            operation: "profile",
            reason: format!("corpo fora do contrato: {error}"),
        })
    }

    pub async fn set_display_name(
        &self,
        user: &str,
        user_id: &str,
        name: &str,
    ) -> Result<(), CallError> {
        self.call(
            Method::PUT,
            format!("{API}/profile/{}/displayname", encode(user_id)),
            Some(user),
            Body::Json(&json!({ "displayname": name })),
            "profile (nome)",
        )
        .await
        .map(|_| ())
    }

    pub async fn register_user(&self, localpart: &str) -> Result<String, CallError> {
        let value = self
            .call_with_query(
                Method::POST,
                format!("{API}/register"),
                &[("kind", "user")],
                None,
                Body::Json(&json!({
                    "type": "m.login.application_service",
                    "username": localpart,
                    "inhibit_login": true,
                })),
                "register",
            )
            .await?;
        required_text(&value, "user_id", "register")
    }

    pub async fn send_asset(
        &self,
        user: &str,
        name: &str,
        mime: &str,
        bytes: &[u8],
    ) -> Result<String, CallError> {
        let value = match self
            .upload_file(OLD_UPLOAD_PATH, user, name, mime, bytes)
            .await
        {
            Ok(value) => value,
            Err(CallError::Response {
                status: 404,
                errcode,
                ..
            }) if errcode
                .as_deref()
                .is_none_or(|code| code == ERRCODE_NOT_RECOGNIZED) =>
            {
                self.upload_file(NEW_UPLOAD_PATH, user, name, mime, bytes)
                    .await?
            }
            Err(error) => return Err(error),
        };
        required_text(&value, "content_uri", "upload")
    }

    async fn upload_file(
        &self,
        path: &'static str,
        user: &str,
        name: &str,
        mime: &str,
        bytes: &[u8],
    ) -> Result<Value, CallError> {
        self.call_with_query(
            Method::POST,
            path.to_owned(),
            &[("filename", name)],
            Some(user),
            Body::Bytes { mime, bytes },
            "upload",
        )
        .await
    }

    pub async fn download_asset(
        &self,
        user: &str,
        mxc: &str,
        cap: u64,
    ) -> Result<Vec<u8>, AssetError> {
        let (server, id) = mxc_parts(mxc)?;
        let new_path = format!("{NEW_DOWNLOAD_PATH}/{}/{}", encode(&server), encode(&id));
        let first = self.download(&new_path, user, cap).await;
        match first {
            Err(AssetError::Call(CallError::Response { status: 404, .. })) => {
                let old_path = format!("{OLD_DOWNLOAD_PATH}/{}/{}", encode(&server), encode(&id));
                match self.download(&old_path, user, cap).await {
                    Ok(bytes) => Ok(bytes),
                    Err(_) => first,
                }
            }
            other => other,
        }
    }

    async fn download(&self, path: &str, user: &str, cap: u64) -> Result<Vec<u8>, AssetError> {
        let operation = "download";
        let url = format!("{}{}", self.0.config.url_base, path);
        let authorization = token_authorization(&self.0.config)?;
        let response = self
            .0
            .http_without_redirect
            .get(&url)
            .header(AUTHORIZATION, authorization)
            .query(&[("user_id", user), ("allow_redirect", "false")])
            .send()
            .await
            .map_err(|error| transport_failure(error, operation, self.0.config.timeout))?;

        let status = response.status();
        if status.is_redirection() {
            return Err(AssetError::Call(CallError::InvalidResponse {
                operation,
                reason: "o Synapse respondeu com redirecionamento, que não é seguido".to_owned(),
            }));
        }
        if !status.is_success() {
            let raw = read_body(response, operation).await?;
            let value: Value = serde_json::from_slice(&raw).unwrap_or(Value::Null);
            let (errcode, message, retry_after_ms) = error_detail(&value);
            return Err(AssetError::Call(CallError::Response {
                operation,
                status: status.as_u16(),
                errcode,
                message,
                retry_after_ms,
            }));
        }

        if let Some(declared) = response.content_length()
            && declared > cap
        {
            return Err(AssetError::Large {
                bytes: declared,
                cap,
            });
        }

        let mut accumulated = Vec::new();
        let mut response = response;
        while let Some(piece) = response
            .chunk()
            .await
            .map_err(|error| transport_failure(error, operation, self.0.config.timeout))?
        {
            let total = accumulated.len() as u64 + piece.len() as u64;
            if total > cap {
                return Err(AssetError::Large { bytes: total, cap });
            }
            accumulated.extend_from_slice(&piece);
        }
        if accumulated.is_empty() {
            return Err(AssetError::Empty);
        }
        Ok(accumulated)
    }

    pub async fn read_event(
        &self,
        user: &str,
        room: &str,
        event: &str,
    ) -> Result<Value, CallError> {
        self.call(
            Method::GET,
            format!("{API}/rooms/{}/event/{}", encode(room), encode(event)),
            Some(user),
            Body::None,
            "event (leitura)",
        )
        .await
    }

    pub async fn redact(
        &self,
        user: &str,
        room: &str,
        event: &str,
        txn_id: &str,
        content: &Value,
    ) -> Result<String, CallError> {
        let value = self
            .call(
                Method::PUT,
                format!(
                    "{API}/rooms/{}/redact/{}/{}",
                    encode(room),
                    encode(event),
                    encode(txn_id)
                ),
                Some(user),
                Body::Json(content),
                "redact",
            )
            .await?;
        required_text(&value, "event_id", "redact")
    }

    pub async fn delete_asset(&self, token_admin: &Secret, mxc: &str) -> Result<bool, AssetError> {
        let (server, id) = mxc_parts(mxc)?;
        let path = format!("{ADMIN_ASSET}/{}/{}", encode(&server), encode(&id));
        let authorization = authorization_of(token_admin, "de administrador do Synapse")
            .map_err(AssetError::Call)?;
        let request = Request {
            method: &Method::DELETE,
            path: &path,
            fixed_query: &[],
            user: None,
            body: Body::None,
            operation: "media (administração)",
        };
        match self.try_with(request, authorization).await {
            Ok(_) => Ok(true),
            Err(CallError::Response { status: 404, .. }) => Ok(false),
            Err(error) => Err(AssetError::Call(error)),
        }
    }

    pub async fn check_connection(&self) -> Result<(), CallError> {
        self.call(
            Method::GET,
            "/_matrix/client/versions".to_owned(),
            None,
            Body::None,
            "versions",
        )
        .await
        .map(|_| ())
    }

    async fn call(
        &self,
        method: Method,
        path: String,
        user: Option<&str>,
        body: Body<'_>,
        operation: &'static str,
    ) -> Result<Value, CallError> {
        self.call_with_query(method, path, &[], user, body, operation)
            .await
    }

    async fn call_with_query(
        &self,
        method: Method,
        path: String,
        fixed_query: &[(&str, &str)],
        user: Option<&str>,
        body: Body<'_>,
        operation: &'static str,
    ) -> Result<Value, CallError> {
        let request = Request {
            method: &method,
            path: &path,
            fixed_query,
            user,
            body,
            operation,
        };
        let result = self.attempt(request).await;
        match result {
            Ok(value) => Ok(value),
            Err(error) => {
                if let CallError::Response {
                    status: 429,
                    retry_after_ms: Some(ms),
                    ..
                } = &error
                    && Duration::from_millis(*ms) <= MAX_WAIT_429
                {
                    tokio::time::sleep(Duration::from_millis(*ms)).await;
                    self.attempt(request).await
                } else {
                    Err(error)
                }
            }
        }
    }

    async fn attempt(&self, request: Request<'_>) -> Result<Value, CallError> {
        let authorization = token_authorization(&self.0.config)?;
        self.try_with(request, authorization).await
    }

    async fn try_with(
        &self,
        request: Request<'_>,
        authorization: HeaderValue,
    ) -> Result<Value, CallError> {
        let Request {
            method,
            path,
            fixed_query,
            user,
            body,
            operation,
        } = request;
        let url = format!("{}{}", self.0.config.url_base, path);

        let mut request = self
            .0
            .http
            .request(method.clone(), &url)
            .header(AUTHORIZATION, authorization);
        for (name, value) in fixed_query {
            request = request.query(&[(name, value)]);
        }
        if let Some(user) = user {
            request = request.query(&[("user_id", user)]);
        }
        match body {
            Body::None => {}
            Body::Json(value) => request = request.json(value),
            Body::Bytes { mime, bytes } => {
                request = request
                    .header(
                        CONTENT_TYPE,
                        HeaderValue::from_str(mime).map_err(|_| {
                            CallError::Configuration(
                                "o tipo do arquivo não cabe num cabeçalho HTTP".to_owned(),
                            )
                        })?,
                    )
                    .body(bytes.to_vec());
            }
        }

        let response = request
            .send()
            .await
            .map_err(|error| transport_failure(error, operation, self.0.config.timeout))?;

        let status = response.status();
        let raw = read_body(response, operation).await?;
        let text = String::from_utf8_lossy(&raw);
        let value: Value = if text.trim().is_empty() {
            Value::Null
        } else {
            match serde_json::from_str(&text) {
                Ok(value) => value,
                Err(_) if status.is_success() => {
                    return Err(CallError::InvalidResponse {
                        operation,
                        reason: "o corpo da resposta não é JSON".to_owned(),
                    });
                }
                Err(_) => Value::Null,
            }
        };

        if status.is_success() {
            return Ok(value);
        }

        let (errcode, message, retry_after_ms) = error_detail(&value);
        Err(CallError::Response {
            operation,
            status: status.as_u16(),
            errcode,
            message,
            retry_after_ms,
        })
    }
}

fn token_authorization(config: &ClientConfig) -> Result<HeaderValue, CallError> {
    authorization_of(&config.as_token, "do Application Service")
}

fn authorization_of(token: &Secret, label: &'static str) -> Result<HeaderValue, CallError> {
    HeaderValue::from_str(&format!("Bearer {}", token.expose().trim())).map_err(|_| {
        CallError::Configuration(format!(
            "o token {label} tem caractere que não cabe em cabeçalho HTTP"
        ))
    })
}

fn transport_failure(
    error: reqwest::Error,
    operation: &'static str,
    timeout: Duration,
) -> CallError {
    if error.is_timeout() {
        CallError::Deadline {
            operation,
            ms: u64::try_from(timeout.as_millis()).unwrap_or(u64::MAX),
        }
    } else if error.is_connect() {
        CallError::Network {
            operation,
            reason: "não consegui conectar".to_owned(),
        }
    } else {
        CallError::Network {
            operation,
            reason: error.without_url().to_string(),
        }
    }
}

fn mxc_parts(mxc: &str) -> Result<(String, String), AssetError> {
    let rest = mxc
        .trim()
        .strip_prefix("mxc://")
        .ok_or_else(|| AssetError::Address("o endereço não começa com `mxc://`".to_owned()))?;
    let (server, id) = rest.split_once('/').ok_or_else(|| {
        AssetError::Address("o endereço não tem servidor e identificador".to_owned())
    })?;
    if server.is_empty() || id.is_empty() {
        return Err(AssetError::Address(
            "o endereço tem servidor ou identificador vazio".to_owned(),
        ));
    }
    if id.contains('/') {
        return Err(AssetError::Address(
            "o identificador da mídia tem barra".to_owned(),
        ));
    }
    Ok((server.to_owned(), id.to_owned()))
}

async fn read_body(
    mut response: reqwest::Response,
    operation: &'static str,
) -> Result<Vec<u8>, CallError> {
    let mut accumulated = Vec::new();
    while let Some(piece) = response.chunk().await.map_err(|error| CallError::Network {
        operation,
        reason: error.without_url().to_string(),
    })? {
        if accumulated.len() as u64 + piece.len() as u64 > BODY_CAP {
            return Err(CallError::InvalidResponse {
                operation,
                reason: format!("a resposta passou de {BODY_CAP} bytes"),
            });
        }
        accumulated.extend_from_slice(&piece);
    }
    Ok(accumulated)
}

fn error_detail(value: &Value) -> (Option<String>, String, Option<u64>) {
    let errcode = value.get("errcode").and_then(Value::as_str).map(recortar);
    let message = value
        .get("error")
        .and_then(Value::as_str)
        .map(recortar)
        .unwrap_or_else(|| "sem mensagem do Synapse".to_owned());
    let retry_after_ms = value.get("retry_after_ms").and_then(Value::as_u64);
    (errcode, message, retry_after_ms)
}

fn recortar(text: &str) -> String {
    let clean = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if clean.chars().count() <= MESSAGE_SIZE {
        return clean;
    }
    let mut recorte: String = clean.chars().take(MESSAGE_SIZE).collect();
    recorte.push('…');
    recorte
}

fn required_text(value: &Value, field: &str, operation: &'static str) -> Result<String, CallError> {
    value
        .get(field)
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| CallError::InvalidResponse {
            operation,
            reason: format!("campo `{field}` ausente ou vazio"),
        })
}

fn state_path(room: &str, kind: &str, key: Option<&str>) -> String {
    let mut path = format!("{API}/rooms/{}/state/{}", encode(room), encode(kind));
    if let Some(key) = key.filter(|key| !key.is_empty()) {
        path.push('/');
        path.push_str(&encode(key));
    }
    path
}

fn encode(segment: &str) -> String {
    let mut output = String::with_capacity(segment.len());
    for byte in segment.bytes() {
        match byte {
            b'A'..=b'Z'
            | b'a'..=b'z'
            | b'0'..=b'9'
            | b'-'
            | b'.'
            | b'_'
            | b'~'
            | b'!'
            | b'$'
            | b'&'
            | b'\''
            | b'('
            | b')'
            | b'*'
            | b'+'
            | b','
            | b';'
            | b'='
            | b':'
            | b'@' => output.push(char::from(byte)),
            other => output.push_str(&format!("%{other:02X}")),
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_path_encodes_room_kind_and_key() {
        assert_eq!(
            state_path(
                "!abc:matrix-dev.local",
                "m.space.child",
                Some("!filha:matrix-dev.local")
            ),
            "/_matrix/client/v3/rooms/!abc:matrix-dev.local/state/m.space.child/!filha:matrix-dev.local"
        );
    }

    #[test]
    fn state_path_without_key_leaves_no_trailing_slash() {
        for key in [None, Some("")] {
            assert_eq!(
                state_path("!abc:dev", "m.room.topic", key),
                "/_matrix/client/v3/rooms/!abc:dev/state/m.room.topic"
            );
        }
    }

    #[test]
    fn encoding_keeps_hash_from_breaking_path_and_does_not_escape_room_id() {
        assert_eq!(
            encode("#ph_c_1:matrix-dev.local"),
            "%23ph_c_1:matrix-dev.local"
        );
        assert_eq!(encode("!sala:a/b"), "!sala:a%2Fb");
        assert_eq!(encode("a b"), "a%20b");
    }

    #[test]
    fn trim_keeps_one_line_and_limits_size() {
        let long = "erro\ncom   quebras ".repeat(30);
        let recorte = recortar(&long);
        assert!(!recorte.contains('\n'), "{recorte}");
        assert!(recorte.chars().count() <= MESSAGE_SIZE + 1, "{recorte}");
    }

    #[test]
    fn mxc_parts_accept_contract_and_refuse_rest() {
        assert_eq!(
            mxc_parts("mxc://matrix-dev.local/abc123").expect("endereço válido"),
            ("matrix-dev.local".to_owned(), "abc123".to_owned())
        );
        assert_eq!(
            mxc_parts("  mxc://s/id  ").expect("endereço válido"),
            ("s".to_owned(), "id".to_owned())
        );
        for invalid in [
            "",
            "https://exemplo/mxc",
            "mxc://",
            "mxc://servidor",
            "mxc:///id",
            "mxc://servidor/",
            "mxc://servidor/a/b",
        ] {
            assert!(
                matches!(mxc_parts(invalid), Err(AssetError::Address(_))),
                "deveria recusar: {invalid}"
            );
        }
    }

    #[test]
    fn error_detail_reads_only_contract_fields() {
        let body = json!({
            "errcode": "M_FORBIDDEN",
            "error": "You are not allowed",
            "retry_after_ms": 700,
            "extra": "ignorado",
        });
        let (errcode, message, retry) = error_detail(&body);
        assert_eq!(errcode.as_deref(), Some("M_FORBIDDEN"));
        assert_eq!(message, "You are not allowed");
        assert_eq!(retry, Some(700));
    }
}
