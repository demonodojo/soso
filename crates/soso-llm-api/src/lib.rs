//! Adaptador JSON del proveedor (Chat Completions) sobre el dominio de core (T10, C1).

#![no_std]

extern crate alloc;

pub mod http;
pub mod request;
pub mod response;
pub mod sse;

pub use http::{
    format_response, format_sse_response_headers, FeedOutcome, HttpError, HttpReader, HttpRequest,
    MAX_BODY_BYTES, MAX_HEADER_BYTES,
};
pub use request::{
    parse_chat_completions, prepare_chat_completion, validate_wire_policy, wire_to_chat_input,
    ApiError, PreparedChatCompletion, WireChatCompletion,
};
pub use response::{
    encode_api_error_json, encode_chat_completion_json, CompletionPayload, CompletionUsage,
    EncodeError,
};
pub use sse::{
    count_done_markers, encode_stream_failure, encode_success_stream, parse_sse_json_events,
    reassemble_text_from_events, reassemble_tool_arguments_from_events, text_delta_splits,
    wrap_sse_data, SSE_DONE,
};
