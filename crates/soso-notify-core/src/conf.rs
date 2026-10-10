//! Parseo de `/etc/notifybug.conf`.

use alloc::string::{String, ToString};

pub struct Conf {
    pub repo: String,
    pub token: String,
}

/// Valores por defecto si falta el fichero o una clave.
pub fn parse_conf(text: &str) -> Conf {
    let mut repo = String::from("demonodojo/soso");
    let mut token = String::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(v) = line.strip_prefix("repo=") {
            repo = v.trim().to_string();
        } else if let Some(v) = line.strip_prefix("token=") {
            token = v.trim().to_string();
        }
    }
    Conf { repo, token }
}

pub fn token_listo(conf: &Conf) -> bool {
    !conf.token.is_empty()
}
