//! Eventos SSE `text/event-stream` (T12, C3).

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use serde::Serialize;
use serde_json::Value;
use soso_llm_core::conversation::ToolCall;

use crate::response::{CompletionPayload, CompletionUsage, EncodeError};

pub const SSE_DONE: &[u8] = b"data: [DONE]\n\n";

/// Envuelve JSON en un evento SSE (`data: …\n\n`).
pub fn wrap_sse_data(json: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(6 + json.len() + 2);
    out.extend_from_slice(b"data: ");
    out.extend_from_slice(json);
    out.extend_from_slice(b"\n\n");
    out
}

#[derive(Serialize)]
struct ChunkResponse<'a> {
    id: &'a str,
    object: &'static str,
    created: u64,
    model: &'a str,
    choices: [ChunkChoice<'a>; 1],
}

#[derive(Serialize)]
struct ChunkChoice<'a> {
    index: u32,
    delta: Delta<'a>,
    #[serde(skip_serializing_if = "Option::is_none")]
    finish_reason: Option<&'a str>,
}

#[derive(Serialize, Default)]
struct Delta<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    role: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<ToolCallDelta<'a>>>,
}

#[derive(Serialize)]
struct ToolCallDelta<'a> {
    index: u32,
    id: Option<&'a str>,
    #[serde(rename = "type")]
    kind: Option<&'static str>,
    function: Option<FunctionDelta<'a>>,
}

#[derive(Serialize)]
struct FunctionDelta<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    arguments: Option<&'a str>,
}

#[derive(Serialize)]
struct UsageChunk<'a> {
    id: &'a str,
    object: &'static str,
    created: u64,
    model: &'a str,
    choices: [EmptyChoice; 1],
    usage: CompletionUsage,
}

#[derive(Serialize)]
struct EmptyChoice {
    index: u32,
    delta: EmptyDelta,
    finish_reason: Option<&'static str>,
}

#[derive(Serialize)]
struct EmptyDelta {}

#[derive(Serialize)]
struct StreamErrorChunk<'a> {
    error: StreamErrorBody<'a>,
}

#[derive(Serialize)]
struct StreamErrorBody<'a> {
    message: &'a str,
    #[serde(rename = "type")]
    kind: &'static str,
    code: &'a str,
}

fn chunk_json(
    payload: &CompletionPayload,
    delta: Delta<'_>,
    finish_reason: Option<&str>,
) -> Result<Vec<u8>, EncodeError> {
    let body = ChunkResponse {
        id: &payload.id,
        object: "chat.completion.chunk",
        created: payload.created,
        model: &payload.model,
        choices: [ChunkChoice {
            index: 0,
            delta,
            finish_reason,
        }],
    };
    serde_json::to_vec(&body).map_err(|e| EncodeError::Json(e.to_string()))
}

fn tool_delta_calls(calls: &[ToolCall]) -> Vec<ToolCallDelta<'_>> {
    calls
        .iter()
        .enumerate()
        .map(|(i, c)| ToolCallDelta {
            index: i as u32,
            id: Some(&c.id),
            kind: Some("function"),
            function: Some(FunctionDelta {
                name: Some(&c.name),
                arguments: Some(&c.arguments),
            }),
        })
        .collect()
}

/// Trocea `content` según `split_lens` (suma = longitud UTF-8 de `content`).
pub fn text_delta_splits(content: &str, split_lens: &[usize]) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = content;
    for &n in split_lens {
        if n == 0 {
            continue;
        }
        let (head, tail) = split_at_char_boundary(rest, n);
        out.push(head.to_string());
        rest = tail;
    }
    if !rest.is_empty() {
        out.push(rest.to_string());
    }
    out
}

fn split_at_char_boundary(s: &str, max_bytes: usize) -> (&str, &str) {
    if max_bytes >= s.len() {
        return (s, "");
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s.split_at(end)
}

/// Eventos SSE de éxito (sin cabecera HTTP). Termina con `[DONE]` exactamente una vez.
pub fn encode_success_stream(
    payload: &CompletionPayload,
    text_deltas: &[String],
    include_usage: bool,
) -> Result<Vec<Vec<u8>>, EncodeError> {
    let finish = payload
        .finish_reason()
        .ok_or(EncodeError::CancelledNoJsonCompletion)?;

    let mut events = Vec::new();

    events.push(wrap_sse_data(
        &chunk_json(
            payload,
            Delta {
                role: Some("assistant"),
                ..Delta::default()
            },
            None,
        )?,
    ));

    if !payload.tool_calls.is_empty() {
        events.push(wrap_sse_data(
            &chunk_json(
                payload,
                Delta {
                    tool_calls: Some(tool_delta_calls(&payload.tool_calls)),
                    ..Delta::default()
                },
                None,
            )?,
        ));
    } else {
        for piece in text_deltas {
            if piece.is_empty() {
                continue;
            }
            events.push(wrap_sse_data(
                &chunk_json(
                    payload,
                    Delta {
                        content: Some(Value::String(piece.clone())),
                        ..Delta::default()
                    },
                    None,
                )?,
            ));
        }
    }

    events.push(wrap_sse_data(
        &chunk_json(payload, Delta::default(), Some(finish))?,
    ));

    if include_usage {
        let usage_body = UsageChunk {
            id: &payload.id,
            object: "chat.completion.chunk",
            created: payload.created,
            model: &payload.model,
            choices: [EmptyChoice {
                index: 0,
                delta: EmptyDelta {},
                finish_reason: None,
            }],
            usage: payload.usage(),
        };
        events.push(wrap_sse_data(
            &serde_json::to_vec(&usage_body).map_err(|e| EncodeError::Json(e.to_string()))?,
        ));
    }

    events.push(SSE_DONE.to_vec());
    Ok(events)
}

/// Error tras HTTP 200 en stream (C3): evento JSON de error y cierre.
pub fn encode_stream_failure(message: &str, code: &str) -> Vec<Vec<u8>> {
    let body = StreamErrorChunk {
        error: StreamErrorBody {
            message,
            kind: "server_error",
            code,
        },
    };
    let json = serde_json::to_vec(&body).unwrap_or_default();
    let mut out = Vec::with_capacity(2);
    out.push(wrap_sse_data(&json));
    out.push(SSE_DONE.to_vec());
    out
}

/// Extrae payloads JSON de líneas `data:` (tests y clientes).
pub fn parse_sse_json_events(raw: &[u8]) -> Vec<serde_json::Value> {
    let text = core::str::from_utf8(raw).unwrap_or("");
    let mut out = Vec::new();
    for line in text.split("\n\n") {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        for part in line.lines() {
            let Some(json) = part.strip_prefix("data: ") else {
                continue;
            };
            if json.trim() == "[DONE]" {
                continue;
            }
            if let Ok(v) = serde_json::from_str(json) {
                out.push(v);
            }
        }
    }
    out
}

/// Reconstruye texto emitido en deltas `content`.
pub fn reassemble_text_from_events(events: &[serde_json::Value]) -> String {
    let mut s = String::new();
    for ev in events {
        if let Some(delta) = ev
            .get("choices")
            .and_then(|c| c.get(0))
            .and_then(|c| c.get("delta"))
            .and_then(|d| d.get("content"))
            .and_then(|c| c.as_str())
        {
            s.push_str(delta);
        }
    }
    s
}

/// Reconstruye argumentos de la primera tool call en deltas.
pub fn reassemble_tool_arguments_from_events(events: &[serde_json::Value]) -> Option<String> {
    for ev in events {
        let Some(delta) = ev
            .get("choices")
            .and_then(|c| c.get(0))
            .and_then(|c| c.get("delta"))
        else {
            continue;
        };
        let Some(calls) = delta.get("tool_calls").and_then(|t| t.as_array()) else {
            continue;
        };
        let Some(args) = calls
            .first()
            .and_then(|c| c.get("function"))
            .and_then(|f| f.get("arguments"))
            .and_then(|a| a.as_str())
        else {
            continue;
        };
        return Some(args.to_string());
    }
    None
}

/// Cuenta eventos `[DONE]`.
pub fn count_done_markers(raw: &[u8]) -> usize {
    raw.windows(SSE_DONE.len())
        .filter(|w| *w == SSE_DONE)
        .count()
}
