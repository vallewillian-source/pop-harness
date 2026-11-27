use std::borrow::Cow;
use std::collections::HashMap;
use std::time::Duration;

use poph_core::llm::{Tool, ToolCall};
use poph_types::config::{HttpToolConfig, HttpToolMethod, HttpToolsConfig};
use poph_types::instructions::REGISTER_LEAD;
use poph_types::{AgentId, ConversationId, MessageId, OrgId, Secret, TurnId};
use reqwest::header::{AUTHORIZATION, HeaderValue};
use reqwest::{Client, Method, Url};
use serde::Serialize;
use serde_json::{Map, Value};

const MAX_RESPONSE_BYTES: usize = 64 * 1024;

#[derive(Default)]
pub struct HttpToolRegistry {
    tools: HashMap<OrgId, HashMap<String, HttpTool>>,
}

impl HttpToolRegistry {
    pub fn from_config(
        config: &HttpToolsConfig,
        mut secret: impl FnMut(&str) -> Option<Secret>,
    ) -> Result<Self, String> {
        if config.is_empty() {
            return Ok(Self::default());
        }
        let client = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .no_proxy()
            .build()
            .map_err(|_| "não foi possível iniciar o cliente das ferramentas HTTP".to_owned())?;

        let mut tools = HashMap::new();
        for (&org, definitions) in config {
            let mut org_tools = HashMap::new();
            for (name, definition) in definitions {
                let authorization = secret(&definition.authorization_env)
                    .map(|value| {
                        let mut header = HeaderValue::from_str(value.expose())
                            .map_err(|_| "credencial inválida para ferramenta HTTP".to_owned())?;
                        header.set_sensitive(true);
                        Ok::<_, String>(header)
                    })
                    .transpose()?;
                let url = Url::parse(&definition.url)
                    .ok()
                    .filter(|url| {
                        matches!(url.scheme(), "http" | "https")
                            && url.has_host()
                            && url.username().is_empty()
                            && url.password().is_none()
                            && url.query().is_none()
                            && url.fragment().is_none()
                    })
                    .ok_or_else(|| "endereço inválido para ferramenta HTTP".to_owned())?;
                org_tools.insert(
                    name.clone(),
                    HttpTool {
                        client: client.clone(),
                        definition: definition.clone(),
                        authorization,
                        url,
                    },
                );
            }
            tools.insert(org, org_tools);
        }
        Ok(Self { tools })
    }

    pub(super) fn tool(&self, org: OrgId, name: &str) -> Option<&HttpTool> {
        self.tools.get(&org)?.get(name)
    }
}

pub(super) struct HttpTool {
    client: Client,
    definition: HttpToolConfig,
    authorization: Option<HeaderValue>,
    url: Url,
}

impl HttpTool {
    pub(super) fn definition(&self, name: &str) -> Tool {
        Tool {
            nome: name.to_owned(),
            descricao: self.definition.description.clone(),
            parameters: self.definition.parameters.clone(),
        }
    }

    pub(super) fn audit_result<'a>(&self, name: &str, result: &'a str) -> Cow<'a, str> {
        if name == REGISTER_LEAD {
            return Cow::Borrowed("");
        }
        match serde_json::from_str(result) {
            Ok(value) => Cow::Owned(super::tools::summary_of_arguments(&value)),
            Err(_) => Cow::Borrowed(result),
        }
    }

    async fn request(&self, context: HttpToolContext, call: &ToolCall) -> Result<String, String> {
        let authorization = self
            .authorization
            .as_ref()
            .ok_or("http_tool_not_configured: credencial ausente")?;
        let method = match self.definition.method {
            HttpToolMethod::Post => Method::POST,
            HttpToolMethod::Put => Method::PUT,
            HttpToolMethod::Patch => Method::PATCH,
            HttpToolMethod::Get => Method::GET,
            HttpToolMethod::Delete => Method::DELETE,
        };
        let mut response = self
            .client
            .request(method, self.url.clone())
            .header(AUTHORIZATION, authorization.clone())
            .header(
                "Idempotency-Key",
                format!("{}:{}", context.turn_id, call.id),
            )
            .timeout(Duration::from_millis(self.definition.timeout_ms))
            .json(&HttpToolRequest {
                context,
                call_id: &call.id,
                arguments: &call.arguments,
            })
            .send()
            .await
            .map_err(transport_error)?;
        if !response.status().is_success() {
            return Err("http_tool_http_error: serviço recusou a chamada".to_owned());
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(transport_error)? {
            if chunk.len() > MAX_RESPONSE_BYTES.saturating_sub(bytes.len()) {
                return Err(invalid_response());
            }
            bytes.extend_from_slice(&chunk);
        }
        let body: Value = serde_json::from_slice(&bytes).map_err(|_| invalid_response())?;
        let fields = self
            .definition
            .response_fields
            .iter()
            .filter_map(|(name, pointer)| Some((name.clone(), body.pointer(pointer)?.clone())))
            .collect::<Map<String, Value>>();
        if fields.is_empty() {
            return Err(invalid_response());
        }
        let result = Value::Object(fields).to_string();
        if authorization.to_str().ok().is_some_and(|value| {
            let credential = value.split_once(' ').map_or(value, |(_, token)| token);
            !credential.is_empty() && result.contains(credential)
        }) {
            return Err(invalid_response());
        }
        Ok(result)
    }
}

fn transport_error(error: reqwest::Error) -> String {
    if error.is_timeout() {
        "http_tool_timeout: serviço demorou para responder"
    } else {
        "http_tool_unavailable: serviço indisponível"
    }
    .to_owned()
}

fn invalid_response() -> String {
    "http_tool_invalid_response: resposta inválida do serviço".to_owned()
}

#[derive(Clone, Copy, Serialize)]
pub(super) struct HttpToolContext {
    pub org_id: OrgId,
    pub agent_id: AgentId,
    pub conversation_id: ConversationId,
    pub turn_id: TurnId,
    pub message_id: MessageId,
}

#[derive(Serialize)]
struct HttpToolRequest<'a> {
    context: HttpToolContext,
    call_id: &'a str,
    arguments: &'a Value,
}

#[derive(Default)]
pub(super) struct HttpToolCalls {
    calls: HashMap<String, RecordedHttpCall>,
}

struct RecordedHttpCall {
    name: String,
    arguments: Value,
    result: Result<String, String>,
}

impl HttpToolCalls {
    pub(super) async fn execute(
        &mut self,
        tool: &HttpTool,
        context: HttpToolContext,
        call: &ToolCall,
    ) -> Result<String, String> {
        if call.id.is_empty()
            || call.id.len() > 256
            || !call.id.bytes().all(|byte| byte.is_ascii_graphic())
            || !call.arguments.is_object()
        {
            return Err("http_tool_invalid_arguments: id ou argumentos inválidos".to_owned());
        }
        if let Some(previous) = self.calls.get(&call.id) {
            if previous.name == call.nome && previous.arguments == call.arguments {
                return previous.result.clone();
            }
            return Err(
                "http_tool_call_conflict: chamada repetida com outros argumentos".to_owned(),
            );
        }

        let result = tool.request(context, call).await;
        self.calls.insert(
            call.id.clone(),
            RecordedHttpCall {
                name: call.nome.clone(),
                arguments: call.arguments.clone(),
                result: result.clone(),
            },
        );
        result
    }
}
