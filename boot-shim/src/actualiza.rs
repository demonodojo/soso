//! Aplica o revierte actualizaciones de kernel en la ESP antes del chainload.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use soso_update_core::hash::decode_hex_sha256;
use soso_update_core::kernel_apply::verify_staged_kernel;
use soso_update_core::kernel_meta::{KernelMeta, KernelPhase, KERNEL_META_SIZE};
use soso_update_core::mailbox::{Mailbox, MailboxCmd};
use soso_update_core::UPD_KERNEL_SLOT_SIZE;
use uefi::boot;
use uefi::proto::media::file::{File, FileAttribute, FileInfo, FileMode, RegularFile};
use uefi::{cstr16, CStr16};

const BUZON: &CStr16 = cstr16!("SOSOUPD.TXT");
const SLOT: &CStr16 = cstr16!("SOSOKRN.BIN");
const META: &CStr16 = cstr16!("SOSOKRN.MET");
const KERNEL: &CStr16 = cstr16!("kernel-x86_64");

/// Atiende el buzón de actualización. Devuelve una línea para el log.
pub fn atender() -> Option<String> {
    if let Some(msg) = recuperar_interrumpido() {
        return Some(msg);
    }
    let texto = leer_fichero(BUZON)?;
    let mb = Mailbox::parse(&texto);
    match mb.cmd {
        MailboxCmd::Probando { ref version } => {
            let v = version.clone();
            match revertir_kernel(&v) {
                Ok(()) => Some(format!(
                    "actualiza: revertido (arranque anterior falló, v{v})"
                )),
                Err(e) => Some(format!("actualiza: ERROR revertir: {e}")),
            }
        }
        MailboxCmd::Revertir => match revertir_kernel("") {
            Ok(()) => Some("actualiza: kernel restaurado desde copia".into()),
            Err(e) => Some(format!("actualiza: ERROR revertir: {e}")),
        },
        MailboxCmd::Kernel {
            size,
            hash,
            version,
        } => match aplicar_kernel(size, &hash, &version) {
            Ok(()) => Some(format!("actualiza: kernel {version} aplicado; probando")),
            Err(e) => Some(format!("actualiza: ERROR aplicar: {e}")),
        },
        MailboxCmd::Ok { .. } => {
            let _ = escribir_buzon(&soso_update_core::Mailbox::format_idle());
            let _ = escribir_kernel_meta(&KernelMeta::idle());
            None
        }
        MailboxCmd::Idle | MailboxCmd::Revertido { .. } => None,
    }
}

fn recuperar_interrumpido() -> Option<String> {
    let meta = leer_kernel_meta()?;
    if !meta.needs_recovery() {
        return None;
    }
    match revertir_desde_meta(&meta) {
        Ok(()) => Some(format!(
            "actualiza: recuperado tras corte (fase {:?})",
            meta.phase
        )),
        Err(e) => Some(format!("actualiza: ERROR recuperar: {e}")),
    }
}

fn aplicar_kernel(size: u64, hash_hex: &str, version: &str) -> Result<(), String> {
    if size == 0 || size as usize > UPD_KERNEL_SLOT_SIZE {
        return Err(format!("tamaño de kernel inválido: {size}"));
    }
    let staged = leer_fichero_bytes(SLOT, UPD_KERNEL_SLOT_SIZE)
        .ok_or_else(|| "no pude leer SOSOKRN.BIN".to_string())?;
    verify_staged_kernel(&staged, size, hash_hex)
        .map_err(|_| "hash del kernel en el hueco no coincide".to_string())?;
    let nuevo = staged[..size as usize].to_vec();

    let viejo = leer_fichero_completo(KERNEL, UPD_KERNEL_SLOT_SIZE)
        .ok_or_else(|| "no pude leer kernel-x86_64".to_string())?;
    let (backup_size, backup_hash) = soso_update_core::backup_digest(&viejo);

    let mut meta = KernelMeta::staged(version, size, hash_hex);
    meta.phase = KernelPhase::Applying;
    meta.backup_size = backup_size;
    meta.backup_hash = backup_hash.clone();
    escribir_kernel_meta(&meta).map_err(|e| format!("meta applying: {e}"))?;

    escribir_fichero_exacto(SLOT, &viejo).map_err(|e| format!("copia backup en hueco: {e}"))?;

    meta.phase = KernelPhase::BackupReady;
    escribir_kernel_meta(&meta).map_err(|e| format!("meta backup: {e}"))?;

    meta.phase = KernelPhase::Applying;
    escribir_kernel_meta(&meta).map_err(|e| format!("meta applying2: {e}"))?;

    escribir_fichero_exacto(KERNEL, &nuevo).map_err(|e| format!("escribir kernel nuevo: {e}"))?;

    meta.phase = KernelPhase::Probando;
    escribir_kernel_meta(&meta).map_err(|e| format!("meta probando: {e}"))?;

    let payload = soso_update_core::Mailbox::format_probando(version);
    escribir_buzon(&payload).map_err(|e| format!("escribir PROBANDO: {e}"))
}

fn revertir_kernel(version: &str) -> Result<(), String> {
    let meta = leer_kernel_meta().unwrap_or_default();
    revertir_desde_meta(&meta).map_err(|e| e.to_string())?;
    let ver = if version.is_empty() {
        meta.version.as_str()
    } else {
        version
    };
    let ver = if ver.is_empty() { "?" } else { ver };
    let payload = soso_update_core::Mailbox::format_revertido(ver);
    escribir_buzon(&payload).map_err(|e| format!("escribir REVERTIDO: {e}"))
}

fn revertir_desde_meta(meta: &KernelMeta) -> Result<(), String> {
    if meta.backup_size > 0 && meta.backup_hash.len() == 64 {
        let slot = leer_fichero_bytes(SLOT, UPD_KERNEL_SLOT_SIZE)
            .ok_or_else(|| "no pude leer SOSOKRN.BIN".to_string())?;
        let backup = meta
            .verify_backup(&slot)
            .map_err(|e| format!("backup inválido: {e:?}"))?;
        escribir_fichero_exacto(KERNEL, backup)
            .map_err(|e| format!("restaurar kernel-x86_64: {e}"))?;
        let _ = escribir_kernel_meta(&KernelMeta::idle());
        return Ok(());
    }
    revertir_kernel_legacy()
}

fn revertir_kernel_legacy() -> Result<(), String> {
    let backup = leer_fichero_bytes(SLOT, UPD_KERNEL_SLOT_SIZE)
        .ok_or_else(|| "no pude leer copia en SOSOKRN.BIN".to_string())?;
    if backup.iter().all(|&b| b == 0) {
        return Err("hueco vacío; nada que restaurar".into());
    }
    let len = backup
        .iter()
        .rposition(|&b| b != 0)
        .map(|i| i + 1)
        .unwrap_or(0);
    escribir_fichero_exacto(KERNEL, &backup[..len])
        .map_err(|e| format!("restaurar kernel-x86_64 (legacy): {e}"))
}

fn leer_kernel_meta() -> Option<KernelMeta> {
    let raw = leer_fichero_bytes(META, KERNEL_META_SIZE)?;
    let text = core::str::from_utf8(&raw).ok()?;
    KernelMeta::parse(text).ok()
}

fn escribir_kernel_meta(meta: &KernelMeta) -> Result<(), &'static str> {
    escribir_fichero_exacto(META, &meta.format())
}

fn escribir_buzon(payload: &[u8]) -> Result<(), &'static str> {
    escribir_fichero_exacto(BUZON, payload)
}

fn leer_fichero(path: &CStr16) -> Option<String> {
    let data = leer_fichero_bytes(path, 4096)?;
    Some(String::from_utf8_lossy(&data).into_owned())
}

fn leer_fichero_completo(path: &CStr16, max: usize) -> Option<Vec<u8>> {
    let mut fs = boot::get_image_file_system(boot::image_handle()).ok()?;
    let mut root = fs.open_volume().ok()?;
    let handle = root.open(path, FileMode::Read, FileAttribute::empty()).ok()?;
    let mut f = handle.into_regular_file()?;
    let mut data = Vec::new();
    let mut buf = [0u8; 32 * 1024];
    loop {
        let n = f.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        if data.len() + n > max {
            data.extend_from_slice(&buf[..max.saturating_sub(data.len())]);
            break;
        }
        data.extend_from_slice(&buf[..n]);
    }
    Some(data)
}

fn leer_fichero_bytes(path: &CStr16, max: usize) -> Option<Vec<u8>> {
    leer_fichero_completo(path, max)
}

fn escribir_fichero_exacto(path: &CStr16, data: &[u8]) -> Result<(), &'static str> {
    let mut fs = boot::get_image_file_system(boot::image_handle())
        .map_err(|_| "sin filesystem")?;
    let mut root = fs.open_volume().map_err(|_| "open_volume")?;
    let handle = root
        .open(path, FileMode::CreateReadWrite, FileAttribute::empty())
        .map_err(|_| "open")?;
    let mut f: RegularFile = handle.into_regular_file().ok_or("not regular")?;
    let _ = f.set_position(0);
    f.write(data).map_err(|_| "write")?;
    f.flush().map_err(|_| "flush")?;
    truncar_fichero(&mut f, data.len() as u64)
}

fn truncar_fichero(f: &mut RegularFile, size: u64) -> Result<(), &'static str> {
    let info = f.get_boxed_info::<FileInfo>().map_err(|_| "get_info")?;
    if info.file_size() == size {
        return Ok(());
    }
    let mut buf = alloc::vec![0u8; 512];
    let new_info = FileInfo::new(
        &mut buf,
        size,
        info.physical_size(),
        *info.create_time(),
        *info.last_access_time(),
        *info.modification_time(),
        info.attribute(),
        info.file_name(),
    )
    .map_err(|_| "new_info")?;
    f.set_info(new_info).map_err(|_| "set_info")
}
