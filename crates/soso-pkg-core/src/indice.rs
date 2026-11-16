//! Índice remoto `indice.txt`.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use soso_update_core::parse_semver;

use crate::rutas::{RutaError, nombre_valido};

pub const INDICE_MAGIC: &str = "SOSOPKG indice v1";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntradaIndice {
    pub nombre: String,
    pub version_raw: String,
    pub txt_hash: String,
    pub txt_size: u64,
    pub pack_hash: String,
    pub pack_size: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Indice {
    pub entradas: Vec<EntradaIndice>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ParseIndiceError {
    BadMagic,
    BadLine { line: usize },
    Nombre(RutaError),
    BadVersion(String),
}

impl core::fmt::Display for ParseIndiceError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::BadMagic => write!(f, "cabecera incorrecta: se esperaba {INDICE_MAGIC}"),
            Self::BadLine { line } => write!(f, "línea {line} inválida"),
            Self::Nombre(e) => write!(f, "{e}"),
            Self::BadVersion(v) => write!(f, "versión {v:?} inválida"),
        }
    }
}

impl Indice {
    pub fn parse(text: &str) -> Result<Self, ParseIndiceError> {
        let mut saw_magic = false;
        let mut entradas = Vec::new();
        for (index, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if line == INDICE_MAGIC {
                saw_magic = true;
                continue;
            }
            if !saw_magic {
                return Err(ParseIndiceError::BadMagic);
            }
            let rest = line
                .strip_prefix("p ")
                .ok_or(ParseIndiceError::BadLine { line: index + 1 })?;
            let parts: Vec<_> = rest.split_whitespace().collect();
            if parts.len() != 6 {
                return Err(ParseIndiceError::BadLine { line: index + 1 });
            }
            let nombre = parts[0].to_string();
            nombre_valido(&nombre).map_err(ParseIndiceError::Nombre)?;
            let version_raw = parts[1].to_string();
            if parse_semver(&version_raw).is_none() {
                return Err(ParseIndiceError::BadVersion(version_raw));
            }
            let txt_hash = parts[2].into();
            let txt_size: u64 = parts[3]
                .parse()
                .map_err(|_| ParseIndiceError::BadLine { line: index + 1 })?;
            let pack_hash = parts[4].into();
            let pack_size: u64 = parts[5]
                .parse()
                .map_err(|_| ParseIndiceError::BadLine { line: index + 1 })?;
            entradas.push(EntradaIndice {
                nombre,
                version_raw,
                txt_hash,
                txt_size,
                pack_hash,
                pack_size,
            });
        }
        if !saw_magic {
            return Err(ParseIndiceError::BadMagic);
        }
        Ok(Self { entradas })
    }

    pub fn buscar(&self, filtro: Option<&str>) -> Vec<&EntradaIndice> {
        self.entradas
            .iter()
            .filter(|e| {
                filtro
                    .map(|f| e.nombre.contains(f) || e.version_raw.contains(f))
                    .unwrap_or(true)
            })
            .collect()
    }

    pub fn mejor(&self, nombre: &str) -> Option<&EntradaIndice> {
        let mut best: Option<&EntradaIndice> = None;
        for e in &self.entradas {
            if e.nombre != nombre {
                continue;
            }
            let Some(cur) = parse_semver(&e.version_raw) else {
                continue;
            };
            match best {
                None => best = Some(e),
                Some(b) => {
                    let Some(bv) = parse_semver(&b.version_raw) else {
                        best = Some(e);
                        continue;
                    };
                    if soso_update_core::semver_cmp(&cur, &bv).is_gt() {
                        best = Some(e);
                    }
                }
            }
        }
        best
    }

    /// Versión publicada más nueva que cumple la restricción indicada.
    pub fn mejor_compatible(&self, nombre: &str, req: &str) -> Option<&EntradaIndice> {
        let mut best: Option<&EntradaIndice> = None;
        for e in &self.entradas {
            if e.nombre != nombre {
                continue;
            }
            let Some(version) = parse_semver(&e.version_raw) else {
                continue;
            };
            if !crate::version_req::satisface(&version, req) {
                continue;
            }
            if best.is_none_or(|b| {
                parse_semver(&b.version_raw)
                    .is_some_and(|v| soso_update_core::semver_cmp(&version, &v).is_gt())
            }) {
                best = Some(e);
            }
        }
        best
    }
}

/// Varios índices, uno por origen, en orden de prioridad. Elige entre todos:
/// gana la versión más alta y, si empatan, el origen que va antes.
#[derive(Clone, Debug, Default)]
pub struct Catalogo {
    pub indices: Vec<Indice>,
}

impl Catalogo {
    fn elegir<'a>(
        &'a self,
        candidata: impl Fn(&'a Indice) -> Option<&'a EntradaIndice>,
    ) -> Option<(usize, &'a EntradaIndice)> {
        let mut best: Option<(usize, &EntradaIndice)> = None;
        for (i, idx) in self.indices.iter().enumerate() {
            let Some(e) = candidata(idx) else {
                continue;
            };
            let Some(v) = parse_semver(&e.version_raw) else {
                continue;
            };
            let mejora = match best {
                None => true,
                Some((_, b)) => parse_semver(&b.version_raw)
                    .is_none_or(|bv| soso_update_core::semver_cmp(&v, &bv).is_gt()),
            };
            if mejora {
                best = Some((i, e));
            }
        }
        best
    }

    /// Versión más nueva de `nombre` en cualquier origen, con el índice del origen.
    pub fn mejor(&self, nombre: &str) -> Option<(usize, &EntradaIndice)> {
        self.elegir(|idx| idx.mejor(nombre))
    }

    /// Como [`Indice::mejor_compatible`], entre todos los orígenes.
    pub fn mejor_compatible(&self, nombre: &str, req: &str) -> Option<(usize, &EntradaIndice)> {
        self.elegir(|idx| idx.mejor_compatible(nombre, req))
    }

    /// Todas las entradas que encajan con el filtro, con su origen.
    pub fn buscar(&self, filtro: Option<&str>) -> Vec<(usize, &EntradaIndice)> {
        let mut v = Vec::new();
        for (i, idx) in self.indices.iter().enumerate() {
            for e in idx.buscar(filtro) {
                v.push((i, e));
            }
        }
        v
    }
}

#[cfg(test)]
mod tests_catalogo {
    use super::*;

    fn indice(filas: &[(&str, &str)]) -> Indice {
        let mut t = String::from("SOSOPKG indice v1\n");
        for (n, v) in filas {
            t.push_str(&alloc::format!("p {n} {v} aa 1 bb 2\n"));
        }
        Indice::parse(&t).unwrap()
    }

    #[test]
    fn gana_la_version_mas_alta_de_cualquier_origen() {
        let c = Catalogo {
            indices: alloc::vec![indice(&[("sal", "0.5.1"), ("soso-pkg", "0.4.0")]), indice(&[("sal", "0.5.2")])],
        };
        let (o, e) = c.mejor("sal").unwrap();
        assert_eq!((o, e.version_raw.as_str()), (1, "0.5.2"));
        assert_eq!(c.mejor("soso-pkg").unwrap().0, 0);
        assert!(c.mejor("nada").is_none());
    }

    #[test]
    fn en_empate_gana_el_primer_origen() {
        let c = Catalogo {
            indices: alloc::vec![indice(&[("x", "1.0.0")]), indice(&[("x", "1.0.0")])],
        };
        assert_eq!(c.mejor("x").unwrap().0, 0);
    }

    #[test]
    fn compatible_mira_todos_los_origenes() {
        let c = Catalogo {
            indices: alloc::vec![indice(&[("b", "2.0.0")]), indice(&[("b", "1.4.0")])],
        };
        let (o, e) = c.mejor_compatible("b", "^1.0.0").unwrap();
        assert_eq!((o, e.version_raw.as_str()), (1, "1.4.0"));
        assert_eq!(c.buscar(Some("b")).len(), 2);
    }
}
