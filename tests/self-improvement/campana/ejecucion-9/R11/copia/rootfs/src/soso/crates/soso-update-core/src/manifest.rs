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
    pub files: Vec<FileEntry>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ParseError {
    BadMagic,
    MissingField,
    BadLine,
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

/// Tamaño máximo de un fichero en el pack (64 MiB).
pub const MAX_FILE_SIZE: u64 = 64 * 1024 * 1024;

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

        for line in text.lines() {
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
                    return Err(ParseError::BadLine);
                }
                kernel_hash = parts[1].into();
                kernel_size = parts[2].parse().map_err(|_| ParseError::BadLine)?;
            } else if line.starts_with("pack ") {
                let parts: Vec<_> = line.split_whitespace().collect();
                if parts.len() != 3 {
                    return Err(ParseError::BadLine);
                }
                pack_hash = parts[1].into();
                pack_size = parts[2].parse().map_err(|_| ParseError::BadLine)?;
            } else if line.starts_with("f ") {
                let parts: Vec<_> = line.split_whitespace().collect();
                if parts.len() < 5 {
                    return Err(ParseError::BadLine);
                }
                let hash_hex = parts[1].into();
                let offset = parts[2].parse().map_err(|_| ParseError::BadLine)?;
                let size = parts[3].parse().map_err(|_| ParseError::BadLine)?;
                let path = parts[4..].join(" ");
                files.push(FileEntry {
                    path,
                    offset,
                    size,
                    hash_hex,
                });
            }
        }
        if !saw_magic || version_raw.is_empty() {
            return Err(ParseError::MissingField);
        }
        let version = semver::parse(&version_raw).ok_or(ParseError::BadLine)?;
        Ok(Self {
            version,
            version_raw,
            build,
            fecha,
            kernel_hash,
            kernel_size,
            pack_hash,
            pack_size,
            files,
        })
    }

    pub fn format(&self) -> String {
        use alloc::format;
        let mut out = format!("{MANIFEST_MAGIC}\n");
        out.push_str(&format!("version={}\n", self.version_raw));
        out.push_str(&format!("build={}\n", self.build));
        out.push_str(&format!("fecha={}\n", self.fecha));
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
