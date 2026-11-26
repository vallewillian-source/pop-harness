use std::collections::HashSet;

use futures::future::BoxFuture;
use futures::stream::BoxStream;

use super::error::LlmError;
use super::types::{LlmEvent, Message, Part, Request};

pub type LlmStream = BoxStream<'static, Result<LlmEvent, LlmError>>;

pub trait LlmProvider: Send + Sync {
    fn stream<'a>(&'a self, request: Request) -> BoxFuture<'a, Result<LlmStream, LlmError>>;
}

pub const IMAGE_MIMES: [&str; 4] = ["image/jpeg", "image/png", "image/gif", "image/webp"];

pub fn validate_request(request: &Request, recusa_turno_modelo: bool) -> Result<(), LlmError> {
    let Some(last) = request.messages.last() else {
        return Err(LlmError::InvalidRequest("nenhuma mensagem".to_owned()));
    };
    if recusa_turno_modelo && let Message::Assistant { .. } = last {
        return Err(LlmError::InvalidRequest(
            "a última mensagem é do modelo e o provedor recusa esse formato (llm.recusa_turno_modelo = true). Termine com a instrução do usuário ou com o resultado de uma ferramenta".to_owned(),
        ));
    }

    let mut names = HashSet::new();
    for tool in &request.tools {
        if tool.nome.trim().is_empty() {
            return Err(LlmError::InvalidRequest("ferramenta sem nome".to_owned()));
        }
        if !names.insert(tool.nome.as_str()) {
            return Err(LlmError::InvalidRequest(format!(
                "ferramenta repetida: {}",
                tool.nome
            )));
        }
    }

    for mensagem in &request.messages {
        if let Message::User(parts) = mensagem {
            for part in parts {
                if let Part::Image { mime, bytes } = part {
                    if !IMAGE_MIMES.contains(&mime.as_str()) {
                        return Err(LlmError::InvalidRequest(format!(
                            "tipo de imagem não aceito: {mime}"
                        )));
                    }
                    if bytes.is_empty() {
                        return Err(LlmError::InvalidRequest("imagem vazia".to_owned()));
                    }
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use poph_types::OrgId;

    use super::*;
    use crate::llm::types::Tool;

    fn request(messages: Vec<Message>) -> Request {
        Request::new(OrgId::generate(), messages)
    }

    fn request_ending_with_model() -> Request {
        request(vec![
            Message::User(vec![Part::Texto("oi".into())]),
            Message::Assistant {
                texto: "olá".into(),
                calls: vec![],
            },
        ])
    }

    #[test]
    fn rejects_request_ending_in_model_turn() {
        assert!(matches!(
            validate_request(&request_ending_with_model(), true),
            Err(LlmError::InvalidRequest(_))
        ));
    }

    #[test]
    fn accepts_model_turn_when_rule_disabled() {
        assert!(validate_request(&request_ending_with_model(), false).is_ok());
        assert!(validate_request(&request_ending_with_model(), true).is_err());
    }

    #[test]
    fn accepts_request_ending_with_tool_result() {
        let p = request(vec![
            Message::User(vec![Part::Texto("preço?".into())]),
            Message::Assistant {
                texto: String::new(),
                calls: vec![crate::llm::types::ToolCall {
                    id: "c1".into(),
                    nome: "buscar".into(),
                    arguments: serde_json::json!({}),
                }],
            },
            Message::Resultado {
                id: "c1".into(),
                content: "R$ 10".into(),
            },
        ]);
        assert!(validate_request(&p, true).is_ok());
    }

    #[test]
    fn rejects_image_type_and_duplicate_tool() {
        let mut p = request(vec![Message::User(vec![Part::Image {
            mime: "image/bmp".into(),
            bytes: vec![1],
        }])]);
        assert!(validate_request(&p, true).is_err());

        p.messages = vec![Message::User(vec![Part::Texto("x".into())])];
        let f = Tool {
            nome: "a".into(),
            descricao: String::new(),
            parameters: serde_json::json!({"type": "object"}),
        };
        p.tools = vec![f.clone(), f];
        assert!(validate_request(&p, true).is_err());
    }
}
