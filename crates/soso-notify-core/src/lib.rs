//! Informes de `notifybug`: config, recorte de logs y JSON para la API de issues.
//!
//! Sin red ni E/S: el binario de userspace lee `/var/log` y llama a `soso-http`.

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

pub mod conf;
pub mod informe;
pub mod issue;

pub use conf::{parse_conf, token_listo, Conf};
pub use informe::{
    cola_flujo, construir_cuerpo, presupuesto_flujo, FlujoEntrada, InformeEntrada,
};
pub use issue::{
    build_issue_json, issues_url, json_escape, mensaje_error_http, parse_issue_response,
    titulo_issue, CUERPO_GITHUB_MAX, TITULO_MAX,
};
