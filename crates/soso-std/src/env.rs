//! Entorno del proceso.

use alloc::string::String;
use alloc::vec::Vec;
use spin::Mutex;

static ARGS: Mutex<Option<Vec<String>>> = Mutex::new(None);
static ENVP: Mutex<Option<Vec<(String, String)>>> = Mutex::new(None);

pub fn init() {}

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
    ENVP.lock()
        .as_ref()?
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.clone())
}
