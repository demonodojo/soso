//! Entorno del proceso.

use alloc::string::String;
use alloc::vec::Vec;
use spin::Mutex;

static ARGS: Mutex<Option<Vec<String>>> = Mutex::new(None);
static ENVP: Mutex<Option<Vec<(String, String)>>> = Mutex::new(None);

pub fn init() {}

pub fn init_from_args(args: &str) {
    if args.as_bytes().starts_with(b"SOSA") {
        if let Some(v) = decode_sosa(args.as_bytes()) {
            set_args(v);
            return;
        }
    }
    let v: Vec<String> = args.split_whitespace().map(String::from).collect();
    set_args(v);
}

fn decode_sosa(raw: &[u8]) -> Option<Vec<String>> {
    if raw.len() < 8 || &raw[..4] != b"SOSA" {
        return None;
    }
    let count = u32::from_le_bytes(raw[4..8].try_into().ok()?) as usize;
    let mut out = Vec::with_capacity(count);
    let mut off = 8usize;
    for _ in 0..count {
        if off + 4 > raw.len() {
            return None;
        }
        let len = u32::from_le_bytes(raw[off..off + 4].try_into().ok()?) as usize;
        off += 4;
        if off + len > raw.len() {
            return None;
        }
        out.push(String::from(core::str::from_utf8(&raw[off..off + len]).ok()?));
        off += len;
    }
    Some(out)
}

pub fn set_args(v: Vec<String>) {
    *ARGS.lock() = Some(v);
}

pub fn args() -> impl Iterator<Item = String> {
    ARGS.lock().clone().unwrap_or_default().into_iter()
}

pub fn var(key: &str) -> Option<String> {
    get(key)
}

pub fn set_env(pairs: Vec<(String, String)>) {
    *ENVP.lock() = Some(pairs);
}

pub(crate) fn get(key: &str) -> Option<String> {
    if let Some(pairs) = ENVP.lock().as_ref() {
        if let Some((_, v)) = pairs.iter().find(|(k, _)| k == key) {
            return Some(v.clone());
        }
    }
    let mut buf = [0u8; 512];
    let n = libsoso::sys::getenv(key, &mut buf);
    if n <= 0 {
        return None;
    }
    core::str::from_utf8(&buf[..n as usize])
        .ok()
        .map(String::from)
}
