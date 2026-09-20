//! Respuesta normalizada Chat Completions (JSON completo, T12, C2–C3).

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use serde::Serialize;
use serde_json::Value;
use soso_llm_core::conversation::ToolCall;
use soso_llm_core::generation::{GenerationReport, StopReason};

/// Uso de tokens alineado con [`GenerationReport`] (no re-tokeniza salida).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct CompletionUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}

/// Salida del asistente lista para JSON o SSE.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionPayload {
    pub id: String,
    pub model: String,
    pub created: u64,
    pub content: Option<String>,
    pub tool_calls: Vec<ToolCall>,
    pub report: GenerationReport,
}

impl CompletionPayload {
    pub fn usage(&self) -> CompletionUsage {
        let completion_tokens = self.report.generated;
        CompletionUsage {
            prompt_tokens: self.report.prompt_tokens,
            completion_tokens,
            total_tokens: self
                .report
                .prompt_tokens
                .saturating_add(completion_tokens),
        }
    }

    /// `None` si la generación se canceló (canal de error SSE, no `stop` fingido).
    pub fn finish_reason(&self) -> Option<&'static str> {
        if self.report.stop == StopReason::Cancelled {
            return None;
        }
        if !self.tool_calls.is_empty() {
            return Some("tool_calls");
        }
        Some(match self.report.stop {
            StopReason::StopToken(_) => "stop",
            StopReason::Limit | StopReason::ContextLimit => "length",
            StopReason::Cancelled => unreachable!(),
        })
    }
}

#[derive(Serialize)]
struct ChatCompletionResponse<'a> {
    id: &'a str,
    object: &'static str,
    created: u64,
    model: &'a str,
    choices: [ChoiceComplete<'a>; 1],
    usage: CompletionUsage,
}

#[derive(Serialize)]
struct ChoiceComplete<'a> {
    index: u32,
    message: AssistantMessage<'a>,
    finish_reason: &'a str,
}

#[derive(Serialize)]
struct AssistantMessage<'a> {
    role: &'static str,
    content: Value,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tool_calls: Vec<ToolCallWire<'a>>,
}

fn content_field(content: &Option<String>) -> Value {
    match content {
        Some(s) => Value::String(s.clone()),
        None => Value::Null,
    }
}

#[derive(Serialize)]
struct ToolCallWire<'a> {
    id: &'a str,
    #[serde(rename = "type")]
    kind: &'static str,
    function: FunctionWire<'a>,
}

#[derive(Serialize)]
struct FunctionWire<'a> {
    name: &'a str,
    arguments: &'a str,
}

fn tool_calls_wire(calls: &[ToolCall]) -> Vec<ToolCallWire<'_>> {
    calls
        .iter()
        .map(|c| ToolCallWire {
            id: &c.id,
            kind: "function",
            function: FunctionWire {
                name: &c.name,
                arguments: &c.arguments,
            },
        })
        .collect()
}

/// Serializa `chat.completion` UTF-8.
pub fn encode_chat_completion_json(payload: &CompletionPayload) -> Result<Vec<u8>, EncodeError> {
    let finish = payload
        .finish_reason()
        .ok_or(EncodeError::CancelledNoJsonCompletion)?;
    let body = ChatCompletionResponse {
        id: &payload.id,
        object: "chat.completion",
        created: payload.created,
        model: &payload.model,
        choices: [ChoiceComplete {
            index: 0,
            message: AssistantMessage {
                role: "assistant",
                content: content_field(&payload.content),
                tool_calls: tool_calls_wire(&payload.tool_calls),
            },
            finish_reason: finish,
        }],
        usage: payload.usage(),
    };
    serde_json::to_vec(&body).map_err(|e| EncodeError::Json(e.to_string()))
}

/// Cuerpo JSON de error OpenAI (`error.message/type/code`).
pub fn encode_api_error_json(err: &crate::ApiError) -> Vec<u8> {
    #[derive(Serialize)]
    struct Root<'a> {
        error: Detail<'a>,
    }
    #[derive(Serialize)]
    struct Detail<'a> {
        message: String,
        #[serde(rename = "type")]
        kind: &'a str,
        code: &'a str,
    }
    let root = Root {
        error: Detail {
            message: err.message(),
            kind: err.error_type(),
            code: err.code(),
        },
    };
    serde_json::to_vec(&root).unwrap_or_else(|_| br#"{"error":{"message":"error interno","type":"internal_error","code":"internal"}}"#.to_vec())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EncodeError {
    Json(String),
    CancelledNoJsonCompletion,
}
