//! Adaptador JSON del proveedor (Chat Completions) sobre el dominio de core (T10, C1).

#![no_std]

extern crate alloc;

pub mod request;

pub use request::{
    parse_chat_completions, prepare_chat_completion, validate_wire_policy, wire_to_chat_input,
    ApiError, PreparedChatCompletion, WireChatCompletion,
};
