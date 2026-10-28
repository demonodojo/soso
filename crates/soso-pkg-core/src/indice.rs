//! Índice remoto `indice.txt`.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use soso_update_core::parse_semver;

use crate::rutas::{nombre_valido, RutaError};

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
            let rest = line.strip_prefix("p ").ok_or(ParseIndiceError::BadLine {
                line: index + 1,
            })?;
            let parts: Vec<_> = rest.split_whitespace().collect();
            if parts.len() != 6 {
                return Err(ParseIndiceError::BadLine {
                    line: index + 1,
                });
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
                filtro.map(|f| {
                    e.nombre.contains(f)
                        || e.version_raw.contains(f)
                }).unwrap_or(true)
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
}
