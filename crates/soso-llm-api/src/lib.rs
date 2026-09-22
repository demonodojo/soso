//! Adaptador JSON del proveedor (Chat Completions) sobre el dominio de core (T10, C1).

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

#[cfg(feature = "std")]
extern crate std;

pub mod admission;
pub mod http;
pub mod request;
pub mod response;
pub mod sse;

pub use admission::{
    aux_feed_authenticated, check_bearer, classify_idle_request, classify_while_generating,
    AdmissionError, AdmissionState, AuxResponse, AuxSlot, IdleRoute, MAX_AUX_CONNECTIONS,
    MAX_AUX_READ_PER_POLL,
};
pub use http::{
    format_response, format_sse_response_headers, FeedOutcome, HeaderPeek, HttpError, HttpReader,
    HttpRequest, MAX_BODY_BYTES, MAX_HEADER_BYTES,
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

#[cfg(feature = "std")]
pub mod service;

#[cfg(feature = "std")]
pub use service::{
    http_error_bytes, load_cpu_backend, run_accept_loop, CancelBridge, ChatBackend, CpuBackend,
    HostService, LoadError, OwnedMapper, ServiceError,
};
