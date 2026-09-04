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
