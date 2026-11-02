//! Manifest `manifest.txt` de un release.

use alloc::string::String;
use alloc::vec::Vec;

use crate::hash::Hash256;
use crate::semver::{self, SemVer};

pub const MANIFEST_MAGIC: &str = "SOSOREL v1";

#[derive(Clone, Debug)]
pub struct FileEntry {
    pub path: String,
    pub offset: u64,
    pub size: u64,
    pub hash_hex: String,
}

#[derive(Clone, Debug)]
pub struct Manifest {
    pub version: SemVer,
    pub version_raw: String,
    pub build: String,
    pub fecha: String,
    pub kernel_hash: String,
    pub kernel_size: u64,
    pub pack_hash: String,
    pub pack_size: u64,
    /// Contrato de compatibilidad (U0). `None` en manifiestos anteriores a U3:
    /// se conservan legibles, pero el cliente los rechaza al comprobar.
    pub compat: Option<crate::compat::Compat>,
    pub files: Vec<FileEntry>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ParseError {
    BadMagic,
    MissingField(&'static str),
    BadLine {
        line: usize,
        field: &'static str,
        expected: &'static str,
    },
    BadVersion(String),
    Compatibility(crate::compat::CompatError),
}

impl core::fmt::Display for ParseError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::BadMagic => write!(f, "cabecera incorrecta: se esperaba {MANIFEST_MAGIC}"),
            Self::MissingField(field) => write!(f, "falta el campo {field}"),
            Self::BadLine {
                line,
                field,
                expected,
            } => write!(f, "línea {line}, {field}: se esperaba {expected}"),
            Self::BadVersion(v) => write!(f, "version={v:?}: se esperaba major.minor.patch"),
            Self::Compatibility(crate::compat::CompatError::CampoInvalido(field)) => write!(
                f,
                "compatibilidad: {field} debe ser un entero sin signo de 32 bits"
            ),
            Self::Compatibility(e) => write!(f, "compatibilidad: {e:?}"),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum ValidateError {
    BadHash { field: &'static str },
    BadPath { path: String },
    DuplicatePath { path: String },
    FileOutOfPack { path: String },
    EmptyFile { path: String },
    FileTooLarge { path: String, size: u64 },
    PackSizeMismatch,
    OverlappingFiles,
}

impl core::fmt::Display for ValidateError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::BadHash { field } => write!(
                f,
                "SHA-256 de {field}: se esperaban 64 dígitos hexadecimales"
            ),
            Self::BadPath { path } => write!(f, "ruta no permitida: {path:?}"),
            Self::DuplicatePath { path } => write!(f, "ruta duplicada: {path:?}"),
            Self::FileOutOfPack { path } => {
                write!(f, "el fichero {path:?} queda fuera de rootfs.pack")
            }
            Self::EmptyFile { path } => write!(f, "el fichero {path:?} está vacío"),
            Self::FileTooLarge { path, size } => write!(
                f,
                "el fichero {path:?} ocupa {size} B; límite del cliente: {MAX_FILE_SIZE} B"
            ),
            Self::PackSizeMismatch => {
                write!(f, "el tamaño de rootfs.pack no coincide con sus ficheros")
            }
            Self::OverlappingFiles => write!(f, "hay ficheros con tramos solapados en rootfs.pack"),
        }
    }
}

/// Tamaño máximo de un fichero en el pack (512 MiB). `soso-update` sostiene
/// el fichero en curso entero en su heap, que el kernel corta en `BRK_MAX`
/// (1,5 GiB): al crecer el `Vec` conviven el viejo y el nuevo, ~768 MiB.
/// `bin/rustc` ya pasa de 200 MiB.
pub const MAX_FILE_SIZE: u64 = 512 * 1024 * 1024;

impl Manifest {
    pub fn parse(text: &str) -> Result<Self, ParseError> {
        let mut version_raw = String::new();
        let mut build = String::new();
        let mut fecha = String::new();
        let mut kernel_hash = String::new();
        let mut kernel_size = 0u64;
        let mut pack_hash = String::new();
        let mut pack_size = 0u64;
        let mut files = Vec::new();
        let mut saw_magic = false;

        for (index, line) in text.lines().enumerate() {
            let bad = |field, expected| ParseError::BadLine {
                line: index + 1,
                field,
                expected,
            };
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if line == MANIFEST_MAGIC {
                saw_magic = true;
                continue;
            }
            if !saw_magic {
                return Err(ParseError::BadMagic);
            }
            if let Some(v) = line.strip_prefix("version=") {
                version_raw = v.into();
            } else if let Some(v) = line.strip_prefix("build=") {
                build = v.into();
            } else if let Some(v) = line.strip_prefix("fecha=") {
                fecha = v.into();
            } else if line.starts_with("kernel ") {
                let parts: Vec<_> = line.split_whitespace().collect();
                if parts.len() != 3 {
                    return Err(bad("kernel", "kernel <sha256> <tamaño>"));
                }
                kernel_hash = parts[1].into();
                kernel_size = parts[2]
                    .parse()
                    .map_err(|_| bad("tamaño de kernel", "un entero sin signo de 64 bits"))?;
            } else if line.starts_with("pack ") {
                let parts: Vec<_> = line.split_whitespace().collect();
                if parts.len() != 3 {
                    return Err(bad("pack", "pack <sha256> <tamaño>"));
                }
                pack_hash = parts[1].into();
                pack_size = parts[2]
                    .parse()
                    .map_err(|_| bad("tamaño de pack", "un entero sin signo de 64 bits"))?;
            } else if line.starts_with("f ") {
                let parts: Vec<_> = line.split_whitespace().collect();
                if parts.len() < 5 {
                    return Err(bad(
                        "fichero",
                        "f <sha256> <offset> <tamaño> <ruta> (posible manifiesto truncado)",
                    ));
                }
                let hash_hex = parts[1].into();
                let offset = parts[2]
                    .parse()
                    .map_err(|_| bad("offset de fichero", "un entero sin signo de 64 bits"))?;
                let size = parts[3]
                    .parse()
                    .map_err(|_| bad("tamaño de fichero", "un entero sin signo de 64 bits"))?;
                let path = parts[4..].join(" ");
                files.push(FileEntry {
                    path,
                    offset,
                    size,
                    hash_hex,
                });
            }
        }
        if !saw_magic {
            return Err(ParseError::BadMagic);
        }
        if version_raw.is_empty() {
            return Err(ParseError::MissingField("version"));
        }
        let version = semver::parse(&version_raw)
            .ok_or_else(|| ParseError::BadVersion(version_raw.clone()))?;
        let compat = match crate::compat::Compat::parse(text) {
            Some(Ok(c)) => Some(c),
            Some(Err(e)) => return Err(ParseError::Compatibility(e)),
            None => None,
        };
        Ok(Self {
            version,
            version_raw,
            build,
            fecha,
            kernel_hash,
            kernel_size,
            pack_hash,
            pack_size,
            compat,
            files,
        })
    }

    pub fn format(&self) -> String {
        use alloc::format;
        let mut out = format!("{MANIFEST_MAGIC}\n");
        out.push_str(&format!("version={}\n", self.version_raw));
        out.push_str(&format!("build={}\n", self.build));
        out.push_str(&format!("fecha={}\n", self.fecha));
        if let Some(c) = &self.compat {
            out.push_str(&c.format());
        }
        out.push_str(&format!(
            "kernel {} {}\n",
            self.kernel_hash, self.kernel_size
        ));
        out.push_str(&format!("pack {} {}\n", self.pack_hash, self.pack_size));
        for f in &self.files {
            out.push_str(&format!(
                "f {} {} {} {}\n",
                f.hash_hex, f.offset, f.size, f.path
            ));
        }
        out
    }

    pub fn find_file(&self, path: &str) -> Option<&FileEntry> {
        self.files.iter().find(|f| f.path == path)
    }

    /// Comprobaciones globales antes de mutar el sistema.
    pub fn validate(&self) -> Result<(), ValidateError> {
        fn ok_hash(field: &'static str, h: &str) -> Result<(), ValidateError> {
            if h.len() == 64
                && h.as_bytes()
                    .iter()
                    .all(|b| b.is_ascii_digit() || matches!(b, b'a'..=b'f' | b'A'..=b'F'))
            {
                Ok(())
            } else {
                Err(ValidateError::BadHash { field })
            }
        }

        ok_hash("kernel", &self.kernel_hash)?;
        ok_hash("pack", &self.pack_hash)?;

        let mut seen = alloc::collections::BTreeSet::new();
        let mut max_end = 0u64;
        for f in &self.files {
            if f.path.is_empty()
                || f.path.starts_with('/')
                || f.path.contains('\\')
                || f.path.split('/').any(|p| p == "..")
            {
                return Err(ValidateError::BadPath {
                    path: f.path.clone(),
                });
            }
            if !seen.insert(f.path.clone()) {
                return Err(ValidateError::DuplicatePath {
                    path: f.path.clone(),
                });
            }
            ok_hash("file", &f.hash_hex)?;
            if f.size == 0 {
                return Err(ValidateError::EmptyFile {
                    path: f.path.clone(),
                });
            }
            if f.size > MAX_FILE_SIZE {
                return Err(ValidateError::FileTooLarge {
                    path: f.path.clone(),
                    size: f.size,
                });
            }
            let end = f.offset.saturating_add(f.size);
            if end > self.pack_size {
                return Err(ValidateError::FileOutOfPack {
                    path: f.path.clone(),
                });
            }
            max_end = max_end.max(end);
        }
        if !self.files.is_empty() && max_end > self.pack_size {
            return Err(ValidateError::PackSizeMismatch);
        }
        if self.files.len() > 1 {
            let mut sorted = self.files.clone();
            sorted.sort_by_key(|f| f.offset);
            for w in sorted.windows(2) {
                let a = &w[0];
                let b = &w[1];
                if a.offset + a.size > b.offset {
                    return Err(ValidateError::OverlappingFiles);
                }
            }
        }
        Ok(())
    }
}

pub fn hash_matches_hex(data: &[u8], hex: &str) -> bool {
    crate::hash::hex_sha256(data) == hex
}

pub fn verify_hash(data: &[u8], hex: &str) -> bool {
    hash_matches_hex(data, hex)
}

pub fn hash_bytes(data: &[u8]) -> Hash256 {
    crate::hash::sha256(data)
}
