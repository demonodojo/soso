use soso_update_core::kernel_apply::{plan_apply, restore_from_slot, verify_staged_kernel, ApplyError};
use soso_update_core::kernel_meta::{KernelMeta, KernelPhase, MetaError};
use soso_update_core::manifest::{Manifest, ValidateError, MANIFEST_MAGIC};

#[test]
fn manifest_rejects_absolute_path() {
    let text = format!(
        "{MANIFEST_MAGIC}\nversion=1.0.0\nbuild=a\nfecha=2026-01-01\n\
         kernel {} 10\npack {} 20\nf {} 0 5 /etc/passwd\n",
        "a".repeat(64),
        "b".repeat(64),
        "c".repeat(64),
    );
    let m = Manifest::parse(&text).unwrap();
    assert!(matches!(
        m.validate(),
        Err(ValidateError::BadPath { .. })
    ));
}

#[test]
fn manifest_rejects_duplicate_paths() {
    let text = format!(
        "{MANIFEST_MAGIC}\nversion=1.0.0\nbuild=a\nfecha=2026-01-01\n\
         kernel {} 10\npack {} 30\nf {} 0 10 bin/a\nf {} 10 10 bin/a\n",
        "a".repeat(64),
        "b".repeat(64),
        "c".repeat(64),
        "d".repeat(64),
    );
    let m = Manifest::parse(&text).unwrap();
    assert!(matches!(
        m.validate(),
        Err(ValidateError::DuplicatePath { .. })
    ));
}

#[test]
fn manifest_rejects_parent_path() {
    let text = format!(
        "{MANIFEST_MAGIC}\nversion=1.0.0\nbuild=a\nfecha=2026-01-01\n\
         kernel {} 10\npack {} 20\nf {} 0 5 ../etc/passwd\n",
        "a".repeat(64),
        "b".repeat(64),
        "c".repeat(64),
    );
    let m = Manifest::parse(&text).unwrap();
    assert!(matches!(
        m.validate(),
        Err(ValidateError::BadPath { .. })
    ));
}

#[test]
fn manifest_rejects_overlap() {
    let text = format!(
        "{MANIFEST_MAGIC}\nversion=1.0.0\nbuild=a\nfecha=2026-01-01\n\
         kernel {} 10\npack {} 100\n\
         f {} 0 20 a\nf {} 10 20 b\n",
        "a".repeat(64),
        "b".repeat(64),
        "c".repeat(64),
        "d".repeat(64),
    );
    let m = Manifest::parse(&text).unwrap();
    assert_eq!(m.validate(), Err(ValidateError::OverlappingFiles));
}

#[test]
fn fault_after_meta_applying_step() {
    let old = b"kernel-original-bytes".to_vec();
    let new = b"kernel-nuevo-staged".to_vec();
    let hash = soso_update_core::hex_sha256(&new);
    let plan = plan_apply(&old, &new, new.len() as u64, &hash, "0.2.6").unwrap();
    let mut meta = plan.meta_after_backup.clone();
    meta.phase = KernelPhase::Applying;
    let mut slot = plan.backup.clone();
    slot.resize(8192, 0);
    let restored = restore_from_slot(&slot, &meta).unwrap();
    assert_eq!(restored, old);
}

#[test]
fn staged_verify_rejects_truncated() {
    let data = b"abc".to_vec();
    let hash = soso_update_core::hex_sha256(&data);
    assert_eq!(
        verify_staged_kernel(&data, 10, &hash),
        Err(ApplyError::InvalidSize)
    );
}

#[test]
fn recovery_after_apply_plan() {
    let old = vec![1u8; 3000];
    let new = b"kernel-nuevo".to_vec();
    let hash = soso_update_core::hex_sha256(&new);
    let plan = plan_apply(&old, &new, new.len() as u64, &hash, "0.2.4").unwrap();
    let mut slot = plan.backup.clone();
    slot.resize(8192, 0);
    let restored = restore_from_slot(&slot, &plan.meta_after_backup).unwrap();
    assert_eq!(restored, old);
}

#[test]
fn recovery_rejects_corrupt_backup() {
    let meta = KernelMeta {
        phase: KernelPhase::BackupReady,
        version: String::from("0.2.2"),
        new_size: 0,
        new_hash: String::new(),
        backup_size: 4,
        backup_hash: "f".repeat(64),
    };
    assert_eq!(
        restore_from_slot(b"data", &meta),
        Err(ApplyError::Meta(MetaError::BackupHashMismatch))
    );
}

#[test]
fn fault_after_backup_step() {
    let old = vec![0u8; 512];
    let new = b"N".repeat(1024);
    let hash = soso_update_core::hex_sha256(&new);
    let plan = plan_apply(&old, &new, new.len() as u64, &hash, "0.2.5").unwrap();
    let mut slot = plan.backup.clone();
    slot.extend_from_slice(&[0u8; 4096]);
    let restored = restore_from_slot(&slot, &plan.meta_after_backup).unwrap();
    assert_eq!(restored.len(), 512);
    assert!(restored.iter().all(|&b| b == 0));
}
