//! Parser HTTP/1.1 incremental y serialización de respuestas (T11, C3).

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// Límite de bytes de la sección de cabeceras (incluye `\r\n\r\n`).
pub const MAX_HEADER_BYTES: usize = 16 * 1024;

/// Límite de bytes del cuerpo de la petición.
pub const MAX_BODY_BYTES: usize = 1024 * 1024;

/// Cabeceras HTTP ya parseadas; el cuerpo puede faltar aún.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeaderPeek {
    pub method: String,
    pub path: String,
    pub content_length: usize,
    pub header_end: usize,
}

/// Petición HTTP/1.1 completa (una por conexión).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpRequest {
    pub method: String,
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

/// Resultado de alimentar el parser.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FeedOutcome {
    NeedMore,
    Complete(HttpRequest),
}

/// Errores de framing HTTP.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HttpError {
    Parse,
    HeaderTooLarge,
    BodyTooLarge,
    AmbiguousContentLength,
    UnsupportedTransferEncoding,
    IncompleteBody,
    TrailingData,
    Overflow,
}

/// Lector incremental sin sockets ni reloj.
#[derive(Debug, Default)]
pub struct HttpReader {
    buf: Vec<u8>,
    finished: bool,
}

impl HttpReader {
    pub fn new() -> Self {
        Self::default()
    }

    /// Bytes acumulados (T17: comprobar Authorization antes del cuerpo).
    pub fn buffer(&self) -> &[u8] {
        &self.buf
    }

    pub fn feed(&mut self, chunk: &[u8]) -> Result<FeedOutcome, HttpError> {
        if self.finished {
            return Err(HttpError::TrailingData);
        }
        if chunk.is_empty() {
            return Ok(FeedOutcome::NeedMore);
        }
        self.buf.extend_from_slice(chunk);
        self.try_complete()
    }

    /// Cabeceras completas sin exigir el cuerpo (T17: responder busy antes del body).
    pub fn peek_headers(&self) -> Result<Option<HeaderPeek>, HttpError> {
        let header_end = match find_header_end(&self.buf) {
            Some(end) => end,
            None => {
                if self.buf.len() > MAX_HEADER_BYTES {
                    return Err(HttpError::HeaderTooLarge);
                }
                return Ok(None);
            }
        };
        if header_end > MAX_HEADER_BYTES {
            return Err(HttpError::HeaderTooLarge);
        }
        let headers_text = core::str::from_utf8(&self.buf[..header_end]).map_err(|_| HttpError::Parse)?;
        let (method, path, parsed_headers) = parse_header_block(headers_text)?;
        if has_transfer_encoding_chunked(&parsed_headers) {
            return Err(HttpError::UnsupportedTransferEncoding);
        }
        let content_length = resolve_content_length(&method, &parsed_headers)?;
        if content_length > MAX_BODY_BYTES {
            return Err(HttpError::BodyTooLarge);
        }
        Ok(Some(HeaderPeek {
            method,
            path,
            content_length,
            header_end,
        }))
    }

    pub fn signal_eof(&mut self) -> Result<FeedOutcome, HttpError> {
        if self.finished {
            return Err(HttpError::TrailingData);
        }
        if self.buf.is_empty() {
            return Err(HttpError::IncompleteBody);
        }
        match self.try_complete()? {
            FeedOutcome::Complete(req) => Ok(FeedOutcome::Complete(req)),
            FeedOutcome::NeedMore => Err(HttpError::IncompleteBody),
        }
    }

    fn try_complete(&mut self) -> Result<FeedOutcome, HttpError> {
        let header_end = match find_header_end(&self.buf) {
            Some(end) => end,
            None => {
                if self.buf.len() > MAX_HEADER_BYTES {
                    return Err(HttpError::HeaderTooLarge);
                }
                return Ok(FeedOutcome::NeedMore);
            }
        };

        if header_end > MAX_HEADER_BYTES {
            return Err(HttpError::HeaderTooLarge);
        }

        let header_bytes = &self.buf[..header_end];
        let headers_text = core::str::from_utf8(header_bytes).map_err(|_| HttpError::Parse)?;
        let (method, path, parsed_headers) = parse_header_block(headers_text)?;

        if has_transfer_encoding_chunked(&parsed_headers) {
            return Err(HttpError::UnsupportedTransferEncoding);
        }

        let content_length = resolve_content_length(&method, &parsed_headers)?;
        if content_length > MAX_BODY_BYTES {
            return Err(HttpError::BodyTooLarge);
        }

        let total = header_end
            .checked_add(content_length)
            .ok_or(HttpError::Overflow)?;
        if self.buf.len() < total {
            if total > MAX_HEADER_BYTES + MAX_BODY_BYTES {
                return Err(HttpError::BodyTooLarge);
            }
            return Ok(FeedOutcome::NeedMore);
        }
        if self.buf.len() > total {
            return Err(HttpError::TrailingData);
        }

        let body = self.buf[header_end..total].to_vec();
        self.buf.clear();
        self.finished = true;

        Ok(FeedOutcome::Complete(HttpRequest {
            method,
            path,
            headers: parsed_headers,
            body,
        }))
    }
}

fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n").map(|i| i + 4)
}

fn parse_header_block(headers_text: &str) -> Result<(String, String, Vec<(String, String)>), HttpError> {
    let mut lines = headers_text.split("\r\n");
    let request_line = lines.next().ok_or(HttpError::Parse)?;
    if request_line.is_empty() {
        return Err(HttpError::Parse);
    }
    let mut parts = request_line.split_whitespace();
    let method = parts.next().ok_or(HttpError::Parse)?.to_string();
    let path = parts.next().ok_or(HttpError::Parse)?.to_string();
    let version = parts.next().ok_or(HttpError::Parse)?;
    if !version.starts_with("HTTP/1.") {
        return Err(HttpError::Parse);
    }
    if parts.next().is_some() {
        return Err(HttpError::Parse);
    }

    let mut headers = Vec::new();
    for line in lines {
        if line.is_empty() {
            continue;
        }
        let (name, value) = line.split_once(':').ok_or(HttpError::Parse)?;
        headers.push((name.trim().to_string(), value.trim().to_string()));
    }
    Ok((method, path, headers))
}

fn header_values<'a>(headers: &'a [(String, String)], name: &str) -> Vec<&'a str> {
    headers
        .iter()
        .filter(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
        .collect()
}

fn has_transfer_encoding_chunked(headers: &[(String, String)]) -> bool {
    for v in header_values(headers, "Transfer-Encoding") {
        for part in v.split(',') {
            if part.trim().eq_ignore_ascii_case("chunked") {
                return true;
            }
        }
    }
    false
}

fn resolve_content_length(method: &str, headers: &[(String, String)]) -> Result<usize, HttpError> {
    let values = header_values(headers, "Content-Length");
    if values.is_empty() {
        return Ok(default_body_length(method));
    }
    let mut parsed: Option<usize> = None;
    for v in values {
        let n: usize = v.parse().map_err(|_| HttpError::Parse)?;
        match parsed {
            None => parsed = Some(n),
            Some(prev) if prev != n => return Err(HttpError::AmbiguousContentLength),
            Some(_) => {}
        }
    }
    Ok(parsed.unwrap_or(0))
}

fn default_body_length(method: &str) -> usize {
    if method.eq_ignore_ascii_case("GET") || method.eq_ignore_ascii_case("HEAD") {
        0
    } else {
        0
    }
}

/// Serializa una respuesta HTTP/1.1 con `Content-Length` exacto y `Connection: close`.
pub fn format_response(status: u16, extra_headers: &[(&str, &str)], body: &[u8]) -> Vec<u8> {
    let reason = status_reason(status);
    let mut out = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    for (k, v) in extra_headers {
        out.push_str(k);
        out.push_str(": ");
        out.push_str(v);
        out.push_str("\r\n");
    }
    out.push_str("\r\n");
    let mut bytes = out.into_bytes();
    bytes.extend_from_slice(body);
    bytes
}

/// Cabeceras de respuesta SSE: sin `Content-Length`; el cierre de conexión delimita el stream.
pub fn format_sse_response_headers(status: u16) -> Vec<u8> {
    let reason = status_reason(status);
    format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n"
    )
    .into_bytes()
}

fn status_reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        405 => "Method Not Allowed",
        413 => "Payload Too Large",
        415 => "Unsupported Media Type",
        422 => "Unprocessable Entity",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
        _ => "Unknown",
    }
}

#[cfg(test)]
mod unit {
    use super::*;

    #[test]
    fn complete_in_one_shot() {
        let raw = b"GET /health HTTP/1.1\r\nHost: x\r\n\r\n";
        let mut r = HttpReader::new();
        match r.feed(raw).unwrap() {
            FeedOutcome::Complete(req) => {
                assert_eq!(req.method, "GET");
                assert_eq!(req.path, "/health");
                assert!(req.body.is_empty());
            }
            FeedOutcome::NeedMore => panic!("expected complete"),
        }
    }
}
