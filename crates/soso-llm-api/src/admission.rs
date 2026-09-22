//! Admisión HTTP durante generación: una activa, sin cola, auxiliares acotados (T17, C3–C4).

use alloc::string::String;
use alloc::vec::Vec;

use crate::http::{FeedOutcome, HttpError, HttpReader, HttpRequest};
use crate::MAX_HEADER_BYTES;

/// Conexiones HTTP parciales mientras hay generación en curso.
pub const MAX_AUX_CONNECTIONS: usize = 2;

/// Bytes máximos leídos en un solo sondeo sobre una conexión auxiliar.
pub const MAX_AUX_READ_PER_POLL: usize = 512;

/// Bytes acumulados máximos por conexión auxiliar.
pub const MAX_AUX_BYTES_TOTAL: usize = MAX_HEADER_BYTES + 4096;

/// Tiempo máximo de vida de una conexión auxiliar sin completar (ms).
pub const AUX_DEADLINE_MS: u64 = 30_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdmissionError {
    AlreadyGenerating,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdleRoute {
    Health,
    Models,
    ChatCompletion,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuxResponse {
    NeedMore,
    Close,
    Reply(Vec<u8>),
}

#[derive(Debug)]
pub struct AuxSlot {
    pub fd: u64,
    pub reader: HttpReader,
    pub started_ms: u64,
    pub bytes_read: usize,
}

impl AuxSlot {
    pub fn new(fd: u64, now_ms: u64) -> Self {
        Self {
            fd,
            reader: HttpReader::new(),
            started_ms: now_ms,
            bytes_read: 0,
        }
    }
}

#[derive(Debug, Default)]
pub struct AdmissionState {
    generating: bool,
    aux: [Option<AuxSlot>; MAX_AUX_CONNECTIONS],
}

impl AdmissionState {
    pub fn is_generating(&self) -> bool {
        self.generating
    }

    pub fn begin_generation(&mut self) -> Result<(), AdmissionError> {
        if self.generating {
            return Err(AdmissionError::AlreadyGenerating);
        }
        self.generating = true;
        Ok(())
    }

    pub fn end_generation(&mut self) {
        self.generating = false;
        for slot in &mut self.aux {
            *slot = None;
        }
    }

    pub fn aux_vacant_index(&self) -> Option<usize> {
        self.aux.iter().position(|s| s.is_none())
    }

    pub fn register_aux(&mut self, index: usize, fd: u64, now_ms: u64) {
        if index < MAX_AUX_CONNECTIONS {
            self.aux[index] = Some(AuxSlot::new(fd, now_ms));
        }
    }

    pub fn drop_aux(&mut self, index: usize) {
        if index < MAX_AUX_CONNECTIONS {
            self.aux[index] = None;
        }
    }

    pub fn aux_slots_mut(&mut self) -> &mut [Option<AuxSlot>] {
        &mut self.aux
    }

    pub fn aux_count(&self) -> usize {
        self.aux.iter().filter(|s| s.is_some()).count()
    }
}

pub fn classify_idle_request(method: &str, path: &str) -> IdleRoute {
    match (method, path) {
        ("GET", "/health") => IdleRoute::Health,
        ("GET", "/v1/models") => IdleRoute::Models,
        ("POST", "/v1/chat/completions") => IdleRoute::ChatCompletion,
        _ => IdleRoute::Other,
    }
}

/// Qué responder en una conexión auxiliar ya identificada (Bearer comprobado).
pub fn classify_while_generating(
    req: &HttpRequest,
    health_json: &[u8],
    busy_json: &[u8],
) -> AuxResponse {
    match (req.method.as_str(), req.path.as_str()) {
        ("GET", "/health") => AuxResponse::Reply(health_json.to_vec()),
        ("POST", "/v1/chat/completions") => AuxResponse::Reply(busy_json.to_vec()),
        _ => AuxResponse::Close,
    }
}

/// Alimenta bytes de una conexión auxiliar durante generación.
pub fn aux_feed_authenticated(
    slot: &mut AuxSlot,
    chunk: &[u8],
    now_ms: u64,
    token: &str,
    health_body: &[u8],
    busy_body: &[u8],
    unauthorized_body: &[u8],
) -> Result<AuxResponse, HttpError> {
    if now_ms.saturating_sub(slot.started_ms) > AUX_DEADLINE_MS {
        return Ok(AuxResponse::Close);
    }
    if chunk.len() > MAX_AUX_READ_PER_POLL {
        return Ok(AuxResponse::Close);
    }
    if slot.bytes_read.saturating_add(chunk.len()) > MAX_AUX_BYTES_TOTAL {
        return Ok(AuxResponse::Close);
    }
    slot.bytes_read = slot.bytes_read.saturating_add(chunk.len());

    match slot.reader.feed(chunk)? {
        FeedOutcome::Complete(req) => {
            if !check_bearer(&req, token) {
                return Ok(AuxResponse::Reply(unauthorized_body.to_vec()));
            }
            Ok(classify_while_generating(&req, health_body, busy_body))
        }
        FeedOutcome::NeedMore => {
            if let Some(peek) = slot.reader.peek_headers()? {
                if !check_bearer_buffer(slot.reader.buffer(), token) {
                    return Ok(AuxResponse::Reply(unauthorized_body.to_vec()));
                }
                if peek.method.eq_ignore_ascii_case("POST")
                    && peek.path == "/v1/chat/completions"
                {
                    return Ok(AuxResponse::Reply(busy_body.to_vec()));
                }
                if peek.method.eq_ignore_ascii_case("GET")
                    && peek.path == "/health"
                    && peek.content_length == 0
                {
                    let req = HttpRequest {
                        method: String::from("GET"),
                        path: String::from("/health"),
                        headers: Vec::new(),
                        body: Vec::new(),
                    };
                    return Ok(classify_while_generating(&req, health_body, busy_body));
                }
            }
            Ok(AuxResponse::NeedMore)
        }
    }
}

pub fn check_bearer(req: &HttpRequest, token: &str) -> bool {
    if token.is_empty() {
        return false;
    }
    for (k, v) in &req.headers {
        if k.eq_ignore_ascii_case("authorization") {
            let v = v.trim();
            if let Some(rest) = v.strip_prefix("Bearer ") {
                return rest.trim() == token;
            }
            if let Some(rest) = v.strip_prefix("bearer ") {
                return rest.trim() == token;
            }
        }
    }
    false
}

fn check_bearer_buffer(buf: &[u8], token: &str) -> bool {
    if token.is_empty() {
        return false;
    }
    let Ok(text) = core::str::from_utf8(buf) else {
        return false;
    };
    for line in text.split("\r\n") {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if !name.trim().eq_ignore_ascii_case("authorization") {
            continue;
        }
        let v = value.trim();
        if let Some(rest) = v.strip_prefix("Bearer ") {
            return rest.trim() == token;
        }
        if let Some(rest) = v.strip_prefix("bearer ") {
            return rest.trim() == token;
        }
    }
    false
}
