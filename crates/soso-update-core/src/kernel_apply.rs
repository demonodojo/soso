//! Lógica compartida de aplicación/reversión del kernel (tests + shim).

use crate::hash::decode_hex_sha256;
use crate::kernel_meta::{KernelMeta, KernelPhase, MetaError};
use crate::mailbox::{Mailbox, MailboxCmd};

/// Qué hacer al arrancar si el apply se cortó entre fases.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AfterInterrupt {
    RecoverBackup,
    /// Meta `Probando` pero el buzón sigue en KERNEL/Idle: no reaplicar.
    CompleteProbandoMailbox { version: alloc::string::String },
    ContinueMailbox,
}

/// Corte entre meta `Probando` y el buzón: completar el buzón, no volver a aplicar.
pub fn after_interrupt(meta: &KernelMeta, mb: &Mailbox) -> AfterInterrupt {
    if meta.needs_recovery() {
        return AfterInterrupt::RecoverBackup;
    }
    if meta.phase == KernelPhase::Probando {
        match &mb.cmd {
            MailboxCmd::Kernel { .. } | MailboxCmd::Idle => {
                return AfterInterrupt::CompleteProbandoMailbox {
                    version: meta.version.clone(),
                };
            }
            MailboxCmd::Probando { .. }
            | MailboxCmd::Ok { .. }
            | MailboxCmd::Revertir
            | MailboxCmd::Revertido { .. } => {}
        }
    }
    AfterInterrupt::ContinueMailbox
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplyError {
    InvalidSize,
    HashMismatch,
    Meta(MetaError),
}

/// Verifica que el hueco contiene el kernel nuevo esperado.
pub fn verify_staged_kernel(data: &[u8], size: u64, hash_hex: &str) -> Result<(), ApplyError> {
    if size == 0 || data.len() < size as usize {
        return Err(ApplyError::InvalidSize);
    }
    let expect = decode_hex_sha256(hash_hex).ok_or(ApplyError::HashMismatch)?;
    if sha256_eq(data, size, &expect) {
        Ok(())
    } else {
        Err(ApplyError::HashMismatch)
    }
}

fn sha256_eq(data: &[u8], size: u64, expect: &[u8; 32]) -> bool {
    let slice = &data[..size as usize];
    crate::hash::sha256(slice) == *expect
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyPlan {
    pub backup: alloc::vec::Vec<u8>,
    pub backup_size: u64,
    pub backup_hash: alloc::string::String,
    pub new_kernel: alloc::vec::Vec<u8>,
    pub meta_after_backup: KernelMeta,
    pub meta_probando: KernelMeta,
}

/// Calcula backup y metadatos antes de tocar la ESP (banco host / simulación).
pub fn plan_apply(
    old_kernel: &[u8],
    staged: &[u8],
    size: u64,
    hash_hex: &str,
    version: &str,
) -> Result<ApplyPlan, ApplyError> {
    verify_staged_kernel(staged, size, hash_hex)?;
    let (backup_size, backup_hash) = crate::kernel_meta::backup_digest(old_kernel);
    let mut meta = KernelMeta::staged(version, size, hash_hex);
    meta.phase = KernelPhase::BackupReady;
    meta.backup_size = backup_size;
    meta.backup_hash = backup_hash.clone();
    let mut probando = meta.clone();
    probando.phase = KernelPhase::Probando;
    Ok(ApplyPlan {
        backup: old_kernel.to_vec(),
        backup_size,
        backup_hash,
        new_kernel: staged[..size as usize].to_vec(),
        meta_after_backup: meta,
        meta_probando: probando,
    })
}

pub fn restore_from_slot(slot: &[u8], meta: &KernelMeta) -> Result<alloc::vec::Vec<u8>, ApplyError> {
    meta.verify_backup(slot).map(|s| s.to_vec()).map_err(ApplyError::Meta)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hash::hex_sha256;

    #[test]
    fn plan_and_restore_short_kernel_with_zeros() {
        let old = vec![0u8; 2048];
        let new = b"new-kernel-image".to_vec();
        let hash = hex_sha256(&new);
        let plan = plan_apply(&old, &new, new.len() as u64, &hash, "0.2.3").unwrap();
        assert_eq!(plan.backup_size, 2048);
        let mut slot = plan.backup.clone();
        slot.extend_from_slice(&[0u8; 1024]);
        let restored = restore_from_slot(&slot, &plan.meta_after_backup).unwrap();
        assert_eq!(restored, old);
    }

    #[test]
    fn reject_staged_hash_mismatch() {
        let old = b"old".to_vec();
        let new = b"new".to_vec();
        let err = plan_apply(&old, &new, new.len() as u64, &"f".repeat(64), "0.2.3");
        assert_eq!(err, Err(ApplyError::HashMismatch));
    }

    #[test]
    fn cut_probando_before_mailbox_completes_mailbox() {
        let mut meta = KernelMeta::staged("0.2.9", 10, &"a".repeat(64));
        meta.phase = KernelPhase::Probando;
        meta.backup_size = 8;
        meta.backup_hash = "b".repeat(64);
        let mb = Mailbox {
            cmd: MailboxCmd::Kernel {
                size: 10,
                hash: "a".repeat(64),
                version: "0.2.9".into(),
            },
        };
        assert_eq!(
            after_interrupt(&meta, &mb),
            AfterInterrupt::CompleteProbandoMailbox {
                version: "0.2.9".into()
            }
        );
        let idle = Mailbox::default();
        assert!(matches!(
            after_interrupt(&meta, &idle),
            AfterInterrupt::CompleteProbandoMailbox { .. }
        ));
        let ya = Mailbox {
            cmd: MailboxCmd::Probando {
                version: "0.2.9".into(),
            },
        };
        assert_eq!(after_interrupt(&meta, &ya), AfterInterrupt::ContinueMailbox);
    }

    #[test]
    fn applying_still_recovers_backup() {
        let mut meta = KernelMeta::staged("0.2.9", 10, &"a".repeat(64));
        meta.phase = KernelPhase::Applying;
        meta.backup_size = 8;
        meta.backup_hash = "b".repeat(64);
        assert_eq!(
            after_interrupt(&meta, &Mailbox::default()),
            AfterInterrupt::RecoverBackup
        );
    }
}
