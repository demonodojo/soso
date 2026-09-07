//! Metadatos durables del slot `SOSOKRN` en la ESP (`SOSOKRN.MET`).

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::hash::{decode_hex_sha256, hex_sha256};

pub const KERNEL_META_MAGIC: &str = "SOSOKRN v1";
pub const KERNEL_META_SIZE: usize = 512;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum KernelPhase {
    #[default]
    Idle,
    /// Nuevo kernel descargado al hueco; el shim aún no lo aplicó.
    Staged,
    /// Copia de seguridad del kernel anterior en el hueco (tamaño/hash válidos).
    BackupReady,
    /// Escritura de `kernel-x86_64` en curso o interrumpida.
    Applying,
    /// Kernel nuevo activo; esperando confirmación de init.
    Probando,
}

impl KernelPhase {
    fn as_str(&self) -> &'static str {
        match self {
            KernelPhase::Idle => "idle",
            KernelPhase::Staged => "staged",
            KernelPhase::BackupReady => "backup",
            KernelPhase::Applying => "applying",
            KernelPhase::Probando => "probando",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        match s {
            "idle" => Some(Self::Idle),
            "staged" => Some(Self::Staged),
            "backup" => Some(Self::BackupReady),
            "applying" => Some(Self::Applying),
            "probando" => Some(Self::Probando),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KernelMeta {
    pub phase: KernelPhase,
    pub version: String,
    pub new_size: u64,
    pub new_hash: String,
    pub backup_size: u64,
    pub backup_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MetaError {
    BadMagic,
    BadPhase,
    MissingField,
    BadHash,
    BackupTooShort,
    BackupHashMismatch,
    BackupEmpty,
}

impl KernelMeta {
    pub fn idle() -> Self {
        Self {
            phase: KernelPhase::Idle,
            ..Default::default()
        }
    }

    pub fn staged(version: &str, new_size: u64, new_hash: &str) -> Self {
        Self {
            phase: KernelPhase::Staged,
            version: version.into(),
            new_size,
            new_hash: new_hash.into(),
            ..Default::default()
        }
    }

    pub fn needs_recovery(&self) -> bool {
        matches!(
            self.phase,
            KernelPhase::Applying | KernelPhase::BackupReady
        ) && self.backup_size > 0 && self.backup_hash.len() == 64
    }

    pub fn parse(text: &str) -> Result<Self, MetaError> {
        let mut saw_magic = false;
        let mut phase = KernelPhase::Idle;
        let mut version = String::new();
        let mut new_size = 0u64;
        let mut new_hash = String::new();
        let mut backup_size = 0u64;
        let mut backup_hash = String::new();

        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if line == KERNEL_META_MAGIC {
                saw_magic = true;
                continue;
            }
            if !saw_magic {
                return Err(MetaError::BadMagic);
            }
            if let Some(v) = line.strip_prefix("phase=") {
                phase = KernelPhase::parse(v).ok_or(MetaError::BadPhase)?;
            } else if let Some(v) = line.strip_prefix("version=") {
                version = v.into();
            } else if let Some(v) = line.strip_prefix("new_size=") {
                new_size = v.parse().map_err(|_| MetaError::MissingField)?;
            } else if let Some(v) = line.strip_prefix("new_hash=") {
                new_hash = v.into();
            } else if let Some(v) = line.strip_prefix("backup_size=") {
                backup_size = v.parse().map_err(|_| MetaError::MissingField)?;
            } else if let Some(v) = line.strip_prefix("backup_hash=") {
                backup_hash = v.into();
            }
        }
        if !saw_magic {
            return Err(MetaError::BadMagic);
        }
        Ok(Self {
            phase,
            version,
            new_size,
            new_hash,
            backup_size,
            backup_hash,
        })
    }

    pub fn format(&self) -> Vec<u8> {
        let mut body = format!("{KERNEL_META_MAGIC}\n");
        body.push_str(&format!("phase={}\n", self.phase.as_str()));
        if !self.version.is_empty() {
            body.push_str(&format!("version={}\n", self.version));
        }
        if self.new_size > 0 {
            body.push_str(&format!("new_size={}\n", self.new_size));
        }
        if !self.new_hash.is_empty() {
            body.push_str(&format!("new_hash={}\n", self.new_hash));
        }
        if self.backup_size > 0 {
            body.push_str(&format!("backup_size={}\n", self.backup_size));
        }
        if !self.backup_hash.is_empty() {
            body.push_str(&format!("backup_hash={}\n", self.backup_hash));
        }
        let mut out = body.into_bytes();
        if out.len() > KERNEL_META_SIZE {
            out.truncate(KERNEL_META_SIZE);
        } else {
            out.resize(KERNEL_META_SIZE, b'\n');
        }
        out
    }

    pub fn verify_new_kernel(&self, data: &[u8]) -> Result<(), MetaError> {
        if self.new_size == 0 || data.len() < self.new_size as usize {
            return Err(MetaError::MissingField);
        }
        let slice = &data[..self.new_size as usize];
        if decode_hex_sha256(&self.new_hash).is_none() {
            return Err(MetaError::BadHash);
        }
        if hex_sha256(slice) != self.new_hash {
            return Err(MetaError::BadHash);
        }
        Ok(())
    }

    /// Comprueba el backup y devuelve los bytes exactos a restaurar.
    pub fn verify_backup<'a>(&self, slot: &'a [u8]) -> Result<&'a [u8], MetaError> {
        if self.backup_size == 0 {
            return Err(MetaError::BackupEmpty);
        }
        if slot.len() < self.backup_size as usize {
            return Err(MetaError::BackupTooShort);
        }
        if decode_hex_sha256(&self.backup_hash).is_none() {
            return Err(MetaError::BadHash);
        }
        let slice = &slot[..self.backup_size as usize];
        if hex_sha256(slice) != self.backup_hash {
            return Err(MetaError::BackupHashMismatch);
        }
        Ok(slice)
    }
}

/// Toma el contenido del kernel activo y calcula tamaño/hash del backup.
pub fn backup_digest(data: &[u8]) -> (u64, String) {
    let size = data.len() as u64;
    (size, hex_sha256(data))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meta_roundtrip() {
        let m = KernelMeta {
            phase: KernelPhase::BackupReady,
            version: String::from("0.2.3"),
            new_size: 1_000_000,
            new_hash: "a".repeat(64),
            backup_size: 900_000,
            backup_hash: "b".repeat(64),
        };
        let bytes = m.format();
        let text = core::str::from_utf8(&bytes).unwrap();
        let m2 = KernelMeta::parse(text).unwrap();
        assert_eq!(m2.phase, KernelPhase::BackupReady);
        assert_eq!(m2.backup_size, 900_000);
    }

    #[test]
    fn verify_backup_rejects_corrupt() {
        let data = b"kernel-bytes";
        let m = KernelMeta {
            phase: KernelPhase::BackupReady,
            version: String::from("0.2.2"),
            new_size: 0,
            new_hash: String::new(),
            backup_size: data.len() as u64,
            backup_hash: "c".repeat(64),
        };
        assert_eq!(m.verify_backup(data), Err(MetaError::BackupHashMismatch));
    }

    #[test]
    fn verify_backup_exact_with_trailing_zeros() {
        let mut data = vec![0u8; 4096];
        data[..8].copy_from_slice(&[b'E', b'L', b'F', 0, 0, 0, 0, 0]);
        let hash = hex_sha256(&data[..512]);
        let m = KernelMeta {
            phase: KernelPhase::BackupReady,
            version: String::from("0.2.2"),
            new_size: 0,
            new_hash: String::new(),
            backup_size: 512,
            backup_hash: hash,
        };
        assert_eq!(m.verify_backup(&data).unwrap().len(), 512);
    }
}
