//! URLs de artefactos remotos.

use alloc::format;
use alloc::string::String;

pub fn indice_url(base: &str) -> String {
    format!("{}/indice.txt", base.trim_end_matches('/'))
}

pub fn artefacto_remoto(base: &str, nombre: &str, version: &str, fichero: &str) -> String {
    format!(
        "{}/{nombre}/{version}/{fichero}",
        base.trim_end_matches('/')
    )
}
