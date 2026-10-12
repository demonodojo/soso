//! Entorno del proceso.

use alloc::string::String;
use alloc::vec::Vec;
use spin::Mutex;

static ARGS: Mutex<Option<Vec<String>>> = Mutex::new(None);
static ENVP: Mutex<Option<Vec<(String, String)>>> = Mutex::new(None);

pub fn init() {}

pub fn init_from_args(args: &str) {
    let v: Vec<String> = args.split_whitespace().map(String::from).collect();
    set_args(v);
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
