//! `/etc/paquetes.conf`
//!
//! ```text
//! url=https://…/soso/main/rootfs/share/paquetes     # el repositorio de soso
//! origen=https://…/sal/main/paquetes                # otros, en orden de prioridad
//! ```
//!
//! `url=` es el origen principal (el único que entendía `soso-pkg` < 0.4.0, que
//! ignora las líneas `origen=`). Cada `origen=` añade otro repositorio; si no
//! hay ninguno se usan los de [`ORIGENES_POR_DEFECTO`], y `origen=ninguno`
//! deja sólo el principal.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// Repositorio de soso: sus paquetes de sistema y aplicaciones.
pub const ORIGEN_SOSO: &str =
    "https://raw.githubusercontent.com/demonodojo/soso/main/rootfs/share/paquetes";
/// Repositorio de Sal: publica el paquete `sal` (compilador, LLVM y SDK).
pub const ORIGEN_SAL: &str = "https://raw.githubusercontent.com/demonodojo/sal/main/paquetes";
/// Orígenes extra cuando la configuración no dice nada.
pub const ORIGENES_POR_DEFECTO: &[&str] = &[ORIGEN_SAL];

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Conf {
    pub url: Option<String>,
    /// `None`: no había líneas `origen=` (se usan los de por defecto).
    pub origenes: Option<Vec<String>>,
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
            match clave.trim() {
                "url" => c.url = Some(valor.to_string()),
                "origen" => {
                    let lista = c.origenes.get_or_insert_with(Vec::new);
                    if valor != "ninguno" && !lista.iter().any(|o| o == valor) {
                        lista.push(valor.to_string());
                    }
                }
                _ => {}
            }
        }
        c
    }

    /// Orígenes en orden de prioridad: el principal y después los extra, sin
    /// repetidos. Ante dos versiones iguales de un paquete gana el primero.
    pub fn origenes_efectivos(&self) -> Vec<String> {
        let principal = self.url.clone().unwrap_or_else(|| ORIGEN_SOSO.to_string());
        let mut todos = alloc::vec![principal];
        let extra: Vec<String> = match &self.origenes {
            Some(lista) => lista.clone(),
            None => ORIGENES_POR_DEFECTO.iter().map(|o| o.to_string()).collect(),
        };
        for o in extra {
            if !todos.iter().any(|t| t.trim_end_matches('/') == o.trim_end_matches('/')) {
                todos.push(o);
            }
        }
        todos
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sin_origenes_usa_los_de_por_defecto() {
        let c = Conf::parse("url=https://a/paquetes\n");
        assert_eq!(c.origenes_efectivos(), ["https://a/paquetes", ORIGEN_SAL]);
    }

    #[test]
    fn conf_vacia_es_soso_mas_sal() {
        assert_eq!(Conf::parse("").origenes_efectivos(), [ORIGEN_SOSO, ORIGEN_SAL]);
    }

    #[test]
    fn origenes_explicitos_en_orden_y_sin_repetir() {
        let c = Conf::parse("url=https://a\norigen=https://b\n# x\norigen=https://c\norigen=https://b\norigen=https://a/\n");
        assert_eq!(c.origenes_efectivos(), ["https://a", "https://b", "https://c"]);
    }

    #[test]
    fn ninguno_deja_solo_el_principal() {
        let c = Conf::parse("url=https://a\norigen=ninguno\n");
        assert_eq!(c.origenes_efectivos(), ["https://a"]);
    }

    #[test]
    fn el_url_sigue_siendo_la_ultima_linea_url() {
        // Lo que lee un soso-pkg viejo no cambia.
        let c = Conf::parse("url=https://x\norigen=https://y\n");
        assert_eq!(c.url.as_deref(), Some("https://x"));
    }
}
