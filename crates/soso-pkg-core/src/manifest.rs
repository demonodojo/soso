//! Manifiesto `paquete.txt` (`SOSOPKG v1`).

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use soso_update_core::{hex_sha256, parse_semver, SemVer};

use crate::rutas::{nombre_valido, ruta_valida, RutaError};
use crate::MAX_PACK_SIZE;

pub const MANIFEST_MAGIC: &str = "SOSOPKG v1";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileEntry {
    pub path: String,
    pub size: u64,
    pub hash_hex: String,
    pub offset: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Manifest {
    pub nombre: String,
    pub version: SemVer,
    pub version_raw: String,
    pub abi: Option<u32>,
    pub files: Vec<FileEntry>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ParseManifestError {
    BadMagic,
    MissingField(&'static str),
    BadLine {
        line: usize,
        field: &'static str,
    },
    BadVersion(String),
    Ruta(RutaError),
    Nombre(RutaError),
}

impl core::fmt::Display for ParseManifestError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::BadMagic => write!(f, "cabecera incorrecta: se esperaba {MANIFEST_MAGIC}"),
            Self::MissingField(field) => write!(f, "falta el campo {field}"),
            Self::BadLine { line, field } => write!(f, "línea {line}: {field} inválido"),
            Self::BadVersion(v) => write!(f, "version={v:?}: se esperaba major.minor.patch"),
            Self::Ruta(e) | Self::Nombre(e) => write!(f, "{e}"),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum ValidateError {
    BadHash { path: String },
    EmptyFile { path: String },
    PackTooLarge { size: u64 },
    HashMismatch { path: String },
    SizeMismatch { path: String },
    MissingInPack { path: String },
    DuplicatePath { path: String },
    AbiIncompatible { esperado: u32, actual: u32 },
}

impl core::fmt::Display for ValidateError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::BadHash { path } => write!(f, "hash inválido en {path}"),
            Self::EmptyFile { path } => write!(f, "fichero vacío: {path}"),
            Self::PackTooLarge { size } => write!(f, "pack demasiado grande: {size} B"),
            Self::HashMismatch { path } => write!(f, "hash no coincide: {path}"),
            Self::SizeMismatch { path } => write!(f, "tamaño no coincide: {path}"),
            Self::MissingInPack { path } => write!(f, "falta en el pack: {path}"),
            Self::AbiIncompatible { esperado, actual } => {
                write!(f, "abi={esperado} incompatible con el sistema (abi={actual})")
            }
            Self::DuplicatePath { path } => write!(f, "ruta duplicada: {path}"),
        }
    }
}

impl Manifest {
    pub fn parse(text: &str) -> Result<Self, ParseManifestError> {
        let mut saw_magic = false;
        let mut nombre = String::new();
        let mut version_raw = String::new();
        let mut abi: Option<u32> = None;
        let mut files = Vec::new();

        for (index, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if line == MANIFEST_MAGIC {
                saw_magic = true;
                continue;
            }
            if !saw_magic {
                return Err(ParseManifestError::BadMagic);
            }
            if let Some(v) = line.strip_prefix("nombre=") {
                nombre = v.trim().into();
            } else if let Some(v) = line.strip_prefix("version=") {
                version_raw = v.trim().into();
            } else if let Some(v) = line.strip_prefix("abi=") {
                abi = Some(
                    v.trim()
                        .parse()
                        .map_err(|_| ParseManifestError::BadLine {
                            line: index + 1,
                            field: "abi",
                        })?,
                );
            } else if let Some(rest) = line.strip_prefix("f ") {
                let parts: Vec<_> = rest.split_whitespace().collect();
                if parts.len() < 3 {
                    return Err(ParseManifestError::BadLine {
                        line: index + 1,
                        field: "f",
                    });
                }
                let hash_hex = parts[0].into();
                let size: u64 = parts[1]
                    .parse()
                    .map_err(|_| ParseManifestError::BadLine {
                        line: index + 1,
                        field: "tamaño",
                    })?;
                let path = parts[2..].join(" ");
                ruta_valida(&path).map_err(ParseManifestError::Ruta)?;
                files.push(FileEntry {
                    path,
                    size,
                    hash_hex,
                    offset: 0,
                });
            }
        }
        if !saw_magic {
            return Err(ParseManifestError::BadMagic);
        }
        if nombre.is_empty() {
            return Err(ParseManifestError::MissingField("nombre"));
        }
        nombre_valido(&nombre).map_err(ParseManifestError::Nombre)?;
        if version_raw.is_empty() {
            return Err(ParseManifestError::MissingField("version"));
        }
        let version = parse_semver(&version_raw)
            .ok_or_else(|| ParseManifestError::BadVersion(version_raw.clone()))?;
        let mut m = Self {
            nombre,
            version,
            version_raw,
            abi,
            files,
        };
        m.assign_offsets();
        Ok(m)
    }

    /// Offsets consecutivos en el orden del manifiesto.
    pub fn assign_offsets(&mut self) {
        let mut off = 0u64;
        for f in &mut self.files {
            f.offset = off;
            off = off.saturating_add(f.size);
        }
    }

    pub fn pack_size(&self) -> u64 {
        self.files.iter().map(|f| f.size).sum()
    }

    pub fn format(&self) -> String {
        let mut out = format!("{MANIFEST_MAGIC}\n");
        out.push_str(&format!("nombre={}\n", self.nombre));
        out.push_str(&format!("version={}\n", self.version_raw));
        if let Some(a) = self.abi {
            out.push_str(&format!("abi={a}\n"));
        }
        for f in &self.files {
            out.push_str(&format!("f {} {} {}\n", f.hash_hex, f.size, f.path));
        }
        out
    }

    pub fn validate(&self, abi_sistema: u32) -> Result<(), ValidateError> {
        if let Some(esperado) = self.abi {
            if esperado != abi_sistema {
                return Err(ValidateError::AbiIncompatible {
                    esperado,
                    actual: abi_sistema,
                });
            }
        }
        let total = self.pack_size();
        if total > MAX_PACK_SIZE {
            return Err(ValidateError::PackTooLarge { size: total });
        }
        let mut seen = alloc::collections::BTreeSet::new();
        for f in &self.files {
            if f.hash_hex.len() != 64
                || !f.hash_hex
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit())
            {
                return Err(ValidateError::BadHash {
                    path: f.path.clone(),
                });
            }
            if f.size == 0 {
                return Err(ValidateError::EmptyFile {
                    path: f.path.clone(),
                });
            }
            if !seen.insert(f.path.clone()) {
                return Err(ValidateError::DuplicatePath {
                    path: f.path.clone(),
                });
            }
        }
        Ok(())
    }

    pub fn verify_pack(&self, pack: &[u8], abi_sistema: u32) -> Result<(), ValidateError> {
        self.validate(abi_sistema)?;
        if pack.len() as u64 != self.pack_size() {
            return Err(ValidateError::PackTooLarge {
                size: pack.len() as u64,
            });
        }
        for f in &self.files {
            let start = f.offset as usize;
            let end = start.checked_add(f.size as usize).ok_or_else(|| {
                ValidateError::MissingInPack {
                    path: f.path.clone(),
                }
            })?;
            let slice = pack.get(start..end).ok_or_else(|| ValidateError::MissingInPack {
                path: f.path.clone(),
            })?;
            if slice.len() as u64 != f.size {
                return Err(ValidateError::SizeMismatch {
                    path: f.path.clone(),
                });
            }
            if hex_sha256(slice) != f.hash_hex {
                return Err(ValidateError::HashMismatch {
                    path: f.path.clone(),
                });
            }
        }
        Ok(())
    }
}
