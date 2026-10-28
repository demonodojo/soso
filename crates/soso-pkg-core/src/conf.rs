//! `/etc/paquetes.conf`

use alloc::string::{String, ToString};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Conf {
    pub url: Option<String>,
}

impl Conf {
    pub fn parse(text: &str) -> Self {
        let mut c = Conf::default();
        for linea in text.lines() {
            let l = linea.trim();
            if l.is_empty() || l.starts_with('#') {
                continue;
            }
            let Some((clave, valor)) = l.split_once('=') else {
                continue;
            };
            let valor = valor.trim();
            if valor.is_empty() {
                continue;
            }
            if clave.trim() == "url" {
                c.url = Some(valor.to_string());
            }
        }
        c
    }
}
