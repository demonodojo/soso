//! T12 — respuestas JSON completas y SSE.

use soso_llm_api::{
    count_done_markers, encode_chat_completion_json, encode_stream_failure, encode_success_stream,
    parse_sse_json_events, reassemble_text_from_events, reassemble_tool_arguments_from_events,
    text_delta_splits, ApiError, CompletionPayload,
};
use soso_llm_core::conversation::ToolCall;
use soso_llm_core::generation::{GenerationReport, StopReason};

fn sample_report(generated: u32, stop: StopReason) -> GenerationReport {
    GenerationReport {
        generated,
        prompt_tokens: 12,
        sampled_tokens: generated.saturating_add(1),
        stop,
    }
}

fn text_payload(content: &str, stop: StopReason) -> CompletionPayload {
    CompletionPayload {
        id: String::from("chatcmpl-test"),
        model: String::from("soso-coder"),
        created: 1_700_000_000,
        content: Some(content.to_string()),
        tool_calls: Vec::new(),
        report: sample_report(content.len() as u32, stop),
    }
}

fn tool_payload(args: &str) -> CompletionPayload {
    CompletionPayload {
        id: String::from("chatcmpl-tool"),
        model: String::from("soso-coder"),
        created: 1_700_000_000,
        content: None,
        tool_calls: vec![ToolCall::nueva(
            "call_1",
            "leer",
            args,
        )],
        report: sample_report(0, StopReason::StopToken(151645)),
    }
}

#[test]
fn json_and_sse_text_coinciden() {
    let text = "Hola \"mundo\" y café ☕";
    let payload = text_payload(text, StopReason::StopToken(99));
    let json = encode_chat_completion_json(&payload).unwrap();
    let json_val: serde_json::Value = serde_json::from_slice(&json).unwrap();
    let json_content = json_val["choices"][0]["message"]["content"]
        .as_str()
        .unwrap();
    assert_eq!(json_content, text);

    let splits = text_delta_splits(text, &[2, 3, 4, 20]);
    let events = encode_success_stream(&payload, &splits, false).unwrap();
    let mut raw = Vec::new();
    for e in &events {
        raw.extend_from_slice(e);
    }
    assert_eq!(count_done_markers(&raw), 1);
    let parsed = parse_sse_json_events(&raw);
    assert_eq!(reassemble_text_from_events(&parsed), text);
    assert_eq!(
        json_val["usage"]["completion_tokens"].as_u64().unwrap(),
        payload.report.generated as u64
    );
}

#[test]
fn content_null_con_tool_calls_en_json() {
    let payload = tool_payload("{\"ruta\":\"a.rs\"}");
    let json = encode_chat_completion_json(&payload).unwrap();
    let v: serde_json::Value = serde_json::from_slice(&json).unwrap();
    assert!(v["choices"][0]["message"]["content"].is_null());
    assert_eq!(v["choices"][0]["finish_reason"], "tool_calls");
}

#[test]
fn sse_tool_arguments_reconstruidos() {
    let args = "{\"comillas\":\"\\\"x\\\"\",\"u\":\"ñ\"}";
    let payload = tool_payload(args);
    let json = encode_chat_completion_json(&payload).unwrap();
    let json_val: serde_json::Value = serde_json::from_slice(&json).unwrap();
    let json_args = json_val["choices"][0]["message"]["tool_calls"][0]["function"]["arguments"]
        .as_str()
        .unwrap();

    let events = encode_success_stream(&payload, &[], false).unwrap();
    let mut raw = Vec::new();
    for e in &events {
        raw.extend_from_slice(e);
    }
    let parsed = parse_sse_json_events(&raw);
    let sse_args = reassemble_tool_arguments_from_events(&parsed).unwrap();
    assert_eq!(sse_args, json_args);
    assert_eq!(sse_args, args);
}

#[test]
fn finish_reason_length() {
    let payload = text_payload("x", StopReason::Limit);
    let json = encode_chat_completion_json(&payload).unwrap();
    let v: serde_json::Value = serde_json::from_slice(&json).unwrap();
    assert_eq!(v["choices"][0]["finish_reason"], "length");
}

#[test]
fn include_usage_evento_antes_de_done() {
    let payload = text_payload("ab", StopReason::StopToken(1));
    let events = encode_success_stream(&payload, &text_delta_splits("ab", &[2]), true).unwrap();
    let mut raw = Vec::new();
    for e in &events {
        raw.extend_from_slice(e);
    }
    let parsed = parse_sse_json_events(&raw);
    let usage_ev = parsed.iter().find(|v| v.get("usage").is_some()).expect("usage");
    assert!(usage_ev["choices"][0]["delta"].as_object().unwrap().is_empty());
    assert_eq!(usage_ev["usage"]["prompt_tokens"], 12);
    assert_eq!(count_done_markers(&raw), 1);
}

#[test]
fn stream_error_y_un_solo_done() {
    let events = encode_stream_failure("inferencia falló", "inference_failed");
    let mut raw = Vec::new();
    for e in &events {
        raw.extend_from_slice(e);
    }
    assert_eq!(count_done_markers(&raw), 1);
    let parsed = parse_sse_json_events(&raw);
    assert_eq!(parsed[0]["error"]["code"], "inference_failed");
}

#[test]
fn cancelled_no_json_completion() {
    let mut payload = text_payload("", StopReason::Cancelled);
    payload.content = None;
    assert!(encode_chat_completion_json(&payload).is_err());
    assert!(encode_success_stream(&payload, &[], false).is_err());
}

#[test]
fn encode_api_error_forma_openai() {
    let err = ApiError::ModeloDesconocido {
        model: String::from("otro"),
    };
    let body = soso_llm_api::encode_api_error_json(&err);
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["error"]["code"], "model_not_found");
}

#[test]
fn sse_multiples_deltas_fragmentados() {
    let text = "abcdef";
    let payload = text_payload(text, StopReason::StopToken(5));
    for split in [1usize, 2, 3] {
        let deltas = text_delta_splits(text, &vec![split; 6 / split.max(1)]);
        let events = encode_success_stream(&payload, &deltas, false).unwrap();
        let mut raw = Vec::new();
        for e in &events {
            raw.extend_from_slice(e);
        }
        let parsed = parse_sse_json_events(&raw);
        assert_eq!(reassemble_text_from_events(&parsed), text);
    }
}
