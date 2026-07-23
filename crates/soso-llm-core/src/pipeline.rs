//! Protocolo binario para inferencia distribuida por pipeline (estrella v3).

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

pub const MAGIC: u32 = 0x5053_4F53; // "SOSO" LE
pub const VERSION: u8 = 3;
pub const HEADER_LEN: usize = 16;

pub const MSG_HELLO: u8 = 1;
pub const MSG_BEGIN: u8 = 2;
pub const MSG_STEP: u8 = 3;
pub const MSG_ACK: u8 = 4;
pub const MSG_TOKEN: u8 = 5;
pub const MSG_RESET: u8 = 6;
pub const MSG_ERROR: u8 = 7;
pub const MSG_STEP_REPLY: u8 = 8;
pub const MSG_PING: u8 = 9;
pub const MSG_PONG: u8 = 10;

pub const ROLE_HEAD: u8 = 1;
pub const ROLE_NODE: u8 = 2;
/// Alias histórico v1; los nodos v2 usan `ROLE_NODE`.
pub const ROLE_WORKER: u8 = ROLE_NODE;

pub const ERR_HANDSHAKE: u16 = 1;
pub const ERR_POS: u16 = 2;
pub const ERR_TIMEOUT: u16 = 3;
pub const ERR_DISCONNECT: u16 = 4;
pub const ERR_NODE_DOWN: u16 = 5;

pub const DEFAULT_STEP_TIMEOUT_MS: u64 = 120_000;
pub const DEFAULT_HANDSHAKE_TIMEOUT_MS: u64 = 60_000;

pub const MAX_NAME: usize = 32;
pub const MAX_ERROR: usize = 128;
pub const MAX_PAYLOAD: usize = 16 * 1024 * 1024;
pub const MAX_PIPELINE_NODES: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PipelineRole {
    Head,
    Node,
    Tail,
    Full,
}

impl PipelineRole {
    pub fn from_layer_range(layer_start: u32, layer_end: u32, num_layers: u32) -> Self {
        if layer_start == 0 && layer_end == num_layers {
            PipelineRole::Full
        } else if layer_start == 0 {
            PipelineRole::Head
        } else if layer_end == num_layers {
            PipelineRole::Tail
        } else {
            PipelineRole::Node
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PipelineSegment {
    pub layer_start: u32,
    pub layer_end: u32,
}

/// Plan de reparto: segmento 0 = head local; 1.. = remotos en orden.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PipelinePlan {
    pub segments: Vec<PipelineSegment>,
}

impl PipelinePlan {
    /// `splits` define fronteras entre segmentos consecutivos.
    /// Ej.: splits `[2,3]` y 4 capas → `[0,2)`, `[2,3)`, `[3,4)`.
    pub fn from_splits(splits: &[u32], num_layers: u32) -> Result<Self, ()> {
        if splits.is_empty() || splits.len() > MAX_PIPELINE_NODES {
            return Err(());
        }
        let mut prev = 0u32;
        let mut segments = Vec::with_capacity(splits.len() + 1);
        for &split in splits {
            if split <= prev || split >= num_layers {
                return Err(());
            }
            segments.push(PipelineSegment {
                layer_start: prev,
                layer_end: split,
            });
            prev = split;
        }
        segments.push(PipelineSegment {
            layer_start: prev,
            layer_end: num_layers,
        });
        Ok(Self { segments })
    }

    pub fn head_segment(&self) -> PipelineSegment {
        self.segments.first().copied().unwrap_or(PipelineSegment {
            layer_start: 0,
            layer_end: 0,
        })
    }

    pub fn remote_count(&self) -> usize {
        self.segments.len().saturating_sub(1)
    }

    pub fn remote_segment(&self, idx: usize) -> Option<PipelineSegment> {
        self.segments.get(idx + 1).copied()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HelloPayload {
    pub role: u8,
    pub layer_start: u32,
    pub layer_end: u32,
    pub num_layers: u32,
    pub hidden_dim: u32,
    pub vocab_size: u32,
    pub model_name: String,
    pub manifest_crc: u32,
    pub index_crc: u32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BeginPayload {
    pub temp: f32,
    pub top_p: f32,
    pub seed: u64,
    pub max_new: u32,
    /// Identifica la sesión; un head nuevo tras failover usa otro id.
    pub session_id: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StepPayload {
    pub pos: u32,
    pub want_token: u8,
    pub hidden: Vec<f32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StepReplyPayload {
    pub pos: u32,
    pub hidden: Vec<f32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AckPayload {
    pub pos: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TokenPayload {
    pub pos: u32,
    pub token: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ErrorPayload {
    pub code: u16,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Message {
    Hello(HelloPayload),
    Begin(BeginPayload),
    Step(StepPayload),
    StepReply(StepReplyPayload),
    Ack(AckPayload),
    Token(TokenPayload),
    Reset,
    Ping,
    Pong,
    Error(ErrorPayload),
}

#[derive(Debug, PartialEq, Eq)]
pub enum CodecError {
    Truncated,
    BadMagic,
    BadVersion,
    BadType,
    PayloadTooLarge,
    InvalidUtf8,
}

impl fmt::Display for CodecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CodecError::Truncated => write!(f, "trama truncada"),
            CodecError::BadMagic => write!(f, "magic inválido"),
            CodecError::BadVersion => write!(f, "versión inválida"),
            CodecError::BadType => write!(f, "tipo de mensaje inválido"),
            CodecError::PayloadTooLarge => write!(f, "payload demasiado grande"),
            CodecError::InvalidUtf8 => write!(f, "utf-8 inválido"),
        }
    }
}

pub fn crc32c(data: &[u8]) -> u32 {
    const POLY: u32 = 0x82F6_3B78;
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ POLY;
            } else {
                crc >>= 1;
            }
        }
    }
    !crc
}

fn write_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn write_u16(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn write_f32(out: &mut Vec<u8>, v: f32) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn write_u64(out: &mut Vec<u8>, v: u64) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn read_u32(data: &[u8], off: &mut usize) -> Result<u32, CodecError> {
    if *off + 4 > data.len() {
        return Err(CodecError::Truncated);
    }
    let v = u32::from_le_bytes(data[*off..*off + 4].try_into().unwrap());
    *off += 4;
    Ok(v)
}

fn read_u16(data: &[u8], off: &mut usize) -> Result<u16, CodecError> {
    if *off + 2 > data.len() {
        return Err(CodecError::Truncated);
    }
    let v = u16::from_le_bytes(data[*off..*off + 2].try_into().unwrap());
    *off += 2;
    Ok(v)
}

fn read_u8(data: &[u8], off: &mut usize) -> Result<u8, CodecError> {
    if *off >= data.len() {
        return Err(CodecError::Truncated);
    }
    let v = data[*off];
    *off += 1;
    Ok(v)
}

fn read_f32(data: &[u8], off: &mut usize) -> Result<f32, CodecError> {
    Ok(f32::from_le_bytes(read_u32(data, off)?.to_le_bytes()))
}

fn read_u64(data: &[u8], off: &mut usize) -> Result<u64, CodecError> {
    if *off + 8 > data.len() {
        return Err(CodecError::Truncated);
    }
    let v = u64::from_le_bytes(data[*off..*off + 8].try_into().unwrap());
    *off += 8;
    Ok(v)
}

fn write_name(out: &mut Vec<u8>, name: &str) {
    let mut buf = [0u8; MAX_NAME];
    let bytes = name.as_bytes();
    let n = bytes.len().min(MAX_NAME - 1);
    buf[..n].copy_from_slice(&bytes[..n]);
    out.extend_from_slice(&buf);
}

fn read_name(data: &[u8], off: &mut usize) -> Result<String, CodecError> {
    if *off + MAX_NAME > data.len() {
        return Err(CodecError::Truncated);
    }
    let raw = &data[*off..*off + MAX_NAME];
    *off += MAX_NAME;
    let end = raw.iter().position(|&b| b == 0).unwrap_or(MAX_NAME);
    core::str::from_utf8(&raw[..end])
        .map(String::from)
        .map_err(|_| CodecError::InvalidUtf8)
}

fn read_hidden(data: &[u8], off: &mut usize) -> Result<Vec<f32>, CodecError> {
    let hidden_len = read_u32(data, off)? as usize;
    let mut hidden = Vec::with_capacity(hidden_len);
    for _ in 0..hidden_len {
        hidden.push(read_f32(data, off)?);
    }
    Ok(hidden)
}

fn write_hidden(out: &mut Vec<u8>, hidden: &[f32]) {
    write_u32(out, hidden.len() as u32);
    for v in hidden {
        write_f32(out, *v);
    }
}

pub fn encode_payload(msg: &Message) -> Vec<u8> {
    let mut body = Vec::new();
    match msg {
        Message::Hello(h) => {
            body.push(h.role);
            write_u32(&mut body, h.layer_start);
            write_u32(&mut body, h.layer_end);
            write_u32(&mut body, h.num_layers);
            write_u32(&mut body, h.hidden_dim);
            write_u32(&mut body, h.vocab_size);
            write_u32(&mut body, h.manifest_crc);
            write_u32(&mut body, h.index_crc);
            write_name(&mut body, &h.model_name);
        }
        Message::Begin(b) => {
            write_f32(&mut body, b.temp);
            write_f32(&mut body, b.top_p);
            write_u64(&mut body, b.seed);
            write_u32(&mut body, b.max_new);
            write_u64(&mut body, b.session_id);
        }
        Message::Step(s) => {
            write_u32(&mut body, s.pos);
            body.push(s.want_token);
            write_hidden(&mut body, &s.hidden);
        }
        Message::StepReply(r) => {
            write_u32(&mut body, r.pos);
            write_hidden(&mut body, &r.hidden);
        }
        Message::Ack(a) => write_u32(&mut body, a.pos),
        Message::Token(t) => {
            write_u32(&mut body, t.pos);
            write_u32(&mut body, t.token);
        }
        Message::Reset => {}
        Message::Ping | Message::Pong => {}
        Message::Error(e) => {
            write_u16(&mut body, e.code);
            let bytes = e.message.as_bytes();
            let n = bytes.len().min(MAX_ERROR - 1);
            write_u16(&mut body, n as u16);
            body.extend_from_slice(&bytes[..n]);
        }
    }
    body
}

pub fn encode_message(seq: u32, msg: &Message) -> Vec<u8> {
    let payload = encode_payload(msg);
    let msg_type = match msg {
        Message::Hello(_) => MSG_HELLO,
        Message::Begin(_) => MSG_BEGIN,
        Message::Step(_) => MSG_STEP,
        Message::Ack(_) => MSG_ACK,
        Message::Token(_) => MSG_TOKEN,
        Message::Reset => MSG_RESET,
        Message::Error(_) => MSG_ERROR,
        Message::StepReply(_) => MSG_STEP_REPLY,
        Message::Ping => MSG_PING,
        Message::Pong => MSG_PONG,
    };
    let mut out = Vec::with_capacity(HEADER_LEN + payload.len());
    write_u32(&mut out, MAGIC);
    out.push(VERSION);
    out.push(msg_type);
    write_u32(&mut out, seq);
    write_u32(&mut out, payload.len() as u32);
    out.extend_from_slice(&[0u8; 2]);
    out.extend_from_slice(&payload);
    out
}

pub fn decode_header(data: &[u8]) -> Result<(u8, u32, usize), CodecError> {
    if data.len() < HEADER_LEN {
        return Err(CodecError::Truncated);
    }
    let magic = u32::from_le_bytes(data[0..4].try_into().unwrap());
    if magic != MAGIC {
        return Err(CodecError::BadMagic);
    }
    let version = data[4];
    if version != VERSION {
        return Err(CodecError::BadVersion);
    }
    let msg_type = data[5];
    let seq = u32::from_le_bytes(data[6..10].try_into().unwrap());
    let payload_len = u32::from_le_bytes(data[10..14].try_into().unwrap()) as usize;
    if payload_len > MAX_PAYLOAD {
        return Err(CodecError::PayloadTooLarge);
    }
    Ok((msg_type, seq, payload_len))
}

pub fn decode_payload(msg_type: u8, payload: &[u8]) -> Result<Message, CodecError> {
    let mut off = 0;
    match msg_type {
        MSG_HELLO => {
            let role = read_u8(payload, &mut off)?;
            let layer_start = read_u32(payload, &mut off)?;
            let layer_end = read_u32(payload, &mut off)?;
            let num_layers = read_u32(payload, &mut off)?;
            let hidden_dim = read_u32(payload, &mut off)?;
            let vocab_size = read_u32(payload, &mut off)?;
            let manifest_crc = read_u32(payload, &mut off)?;
            let index_crc = read_u32(payload, &mut off)?;
            let model_name = read_name(payload, &mut off)?;
            Ok(Message::Hello(HelloPayload {
                role,
                layer_start,
                layer_end,
                num_layers,
                hidden_dim,
                vocab_size,
                model_name,
                manifest_crc,
                index_crc,
            }))
        }
        MSG_BEGIN => Ok(Message::Begin(BeginPayload {
            temp: read_f32(payload, &mut off)?,
            top_p: read_f32(payload, &mut off)?,
            seed: read_u64(payload, &mut off)?,
            max_new: read_u32(payload, &mut off)?,
            session_id: read_u64(payload, &mut off)?,
        })),
        MSG_STEP => {
            let pos = read_u32(payload, &mut off)?;
            let want_token = read_u8(payload, &mut off)?;
            let hidden = read_hidden(payload, &mut off)?;
            Ok(Message::Step(StepPayload {
                pos,
                want_token,
                hidden,
            }))
        }
        MSG_STEP_REPLY => {
            let pos = read_u32(payload, &mut off)?;
            let hidden = read_hidden(payload, &mut off)?;
            Ok(Message::StepReply(StepReplyPayload { pos, hidden }))
        }
        MSG_ACK => Ok(Message::Ack(AckPayload {
            pos: read_u32(payload, &mut off)?,
        })),
        MSG_TOKEN => Ok(Message::Token(TokenPayload {
            pos: read_u32(payload, &mut off)?,
            token: read_u32(payload, &mut off)?,
        })),
        MSG_RESET => Ok(Message::Reset),
        MSG_PING => Ok(Message::Ping),
        MSG_PONG => Ok(Message::Pong),
        MSG_ERROR => {
            let code = read_u16(payload, &mut off)?;
            let n = read_u16(payload, &mut off)? as usize;
            if off + n > payload.len() {
                return Err(CodecError::Truncated);
            }
            let message = core::str::from_utf8(&payload[off..off + n])
                .map(String::from)
                .map_err(|_| CodecError::InvalidUtf8)?;
            Ok(Message::Error(ErrorPayload { code, message }))
        }
        _ => Err(CodecError::BadType),
    }
}

pub fn decode_message(data: &[u8]) -> Result<(u32, Message), CodecError> {
    let (msg_type, seq, payload_len) = decode_header(data)?;
    if data.len() < HEADER_LEN + payload_len {
        return Err(CodecError::Truncated);
    }
    let payload = &data[HEADER_LEN..HEADER_LEN + payload_len];
    let msg = decode_payload(msg_type, payload)?;
    Ok((seq, msg))
}

/// Reloj en milisegundos (p. ej. `libsoso::sys::uptime_ms` en userspace).
pub type NowMs = fn() -> u64;

#[derive(Debug, PartialEq, Eq)]
pub enum RecvError {
    Timeout,
    Disconnected,
    Protocol,
}

/// Transporte abstracto para el protocolo (TCP real o mock en tests).
pub trait Transport {
    fn send_all(&mut self, data: &[u8]) -> Result<(), ()>;
    fn recv_some(&mut self, buf: &mut [u8]) -> Result<usize, ()>;
}

pub struct FramedTransport<T: Transport> {
    pub inner: T,
    pub rx_buf: Vec<u8>,
    pub seq: u32,
}

impl<T: Transport> FramedTransport<T> {
    pub fn new(inner: T) -> Self {
        Self {
            inner,
            rx_buf: Vec::new(),
            seq: 0,
        }
    }

    pub fn send(&mut self, msg: &Message) -> Result<(), ()> {
        let frame = encode_message(self.seq, msg);
        self.seq = self.seq.wrapping_add(1);
        self.inner.send_all(&frame)
    }

    pub fn send_ping(&mut self) -> Result<(), ()> {
        self.send(&Message::Ping)
    }

    pub fn recv(&mut self) -> Result<Message, ()> {
        loop {
            if self.rx_buf.len() >= HEADER_LEN {
                if let Ok((_, _, payload_len)) = decode_header(&self.rx_buf) {
                    let total = HEADER_LEN + payload_len;
                    if self.rx_buf.len() >= total {
                        let frame = self.rx_buf.drain(..total).collect::<Vec<_>>();
                        let (_, msg) = decode_message(&frame).map_err(|_| ())?;
                        return Ok(msg);
                    }
                } else {
                    return Err(());
                }
            }
            let mut tmp = [0u8; 4096];
            let n = self.inner.recv_some(&mut tmp).map_err(|_| ())?;
            if n == 0 {
                return Err(());
            }
            self.rx_buf.extend_from_slice(&tmp[..n]);
        }
    }

    /// Espera un mensaje de aplicación; responde `PONG` a `PING` automáticamente.
    pub fn recv_timeout(&mut self, timeout_ms: u64, now_ms: NowMs) -> Result<Message, RecvError> {
        let deadline = now_ms().saturating_add(timeout_ms);
        loop {
            if let Some(msg) = self.try_take_message()? {
                match msg {
                    Message::Ping => {
                        self.send(&Message::Pong).map_err(|_| RecvError::Protocol)?;
                        continue;
                    }
                    Message::Pong => continue,
                    other => return Ok(other),
                }
            }
            if timeout_ms != u64::MAX && now_ms() >= deadline {
                return Err(RecvError::Timeout);
            }
            let mut tmp = [0u8; 4096];
            match self.inner.recv_some(&mut tmp) {
                Ok(0) => {}
                Ok(n) => self.rx_buf.extend_from_slice(&tmp[..n]),
                Err(()) => return Err(RecvError::Disconnected),
            }
        }
    }

    fn try_take_message(&mut self) -> Result<Option<Message>, RecvError> {
        if self.rx_buf.len() < HEADER_LEN {
            return Ok(None);
        }
        let (_, _, payload_len) = decode_header(&self.rx_buf).map_err(|_| RecvError::Protocol)?;
        let total = HEADER_LEN + payload_len;
        if self.rx_buf.len() < total {
            return Ok(None);
        }
        let frame = self.rx_buf.drain(..total).collect::<Vec<_>>();
        let (_, msg) = decode_message(&frame).map_err(|_| RecvError::Protocol)?;
        Ok(Some(msg))
    }
}

#[cfg(feature = "std")]
pub mod host {
    use super::*;
    use std::io::{Read, Write};

    pub struct TcpTransport {
        stream: std::net::TcpStream,
    }

    impl TcpTransport {
        pub fn connect(addr: &str) -> Result<Self, ()> {
            let stream = std::net::TcpStream::connect(addr).map_err(|_| ())?;
            stream.set_nodelay(true).ok();
            Ok(Self { stream })
        }

        pub fn from_stream(stream: std::net::TcpStream) -> Self {
            stream.set_nodelay(true).ok();
            Self { stream }
        }
    }

    impl Transport for TcpTransport {
        fn send_all(&mut self, data: &[u8]) -> Result<(), ()> {
            self.stream.write_all(data).map_err(|_| ())
        }

        fn recv_some(&mut self, buf: &mut [u8]) -> Result<usize, ()> {
            self.stream.read(buf).map_err(|_| ())
        }
    }
}
