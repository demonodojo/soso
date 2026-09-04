//! Aplica o revierte actualizaciones de kernel en la ESP antes del chainload.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use sha2::{Digest, Sha256};
use soso_update_core::hash::decode_hex_sha256;
use soso_update_core::mailbox::{Mailbox, MailboxCmd};
use soso_update_core::UPD_KERNEL_SLOT_SIZE;
use uefi::boot;
use uefi::proto::media::file::{File, FileAttribute, FileMode, RegularFile};
use uefi::{cstr16, CStr16};

const BUZON: &CStr16 = cstr16!("SOSOUPD.TXT");
const SLOT: &CStr16 = cstr16!("SOSOKRN.BIN");
const KERNEL: &CStr16 = cstr16!("kernel-x86_64");

/// Atiende el buzón de actualización. Devuelve una línea para el log.
pub fn atender() -> Option<String> {
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
            None
        }
        MailboxCmd::Idle | MailboxCmd::Revertido { .. } => None,
    }
}

fn aplicar_kernel(size: u64, hash_hex: &str, version: &str) -> Result<(), String> {
    if size == 0 || size as usize > UPD_KERNEL_SLOT_SIZE {
        return Err(format!("tamaño de kernel inválido: {size}"));
    }
    let expect = decode_hex_sha256(hash_hex).ok_or_else(|| "hash inválido".to_string())?;
    let mut nuevo = leer_fichero_bytes(SLOT, size as usize)
        .ok_or_else(|| "no pude leer SOSOKRN.BIN".to_string())?;
    nuevo.truncate(size as usize);
    if sha256(&nuevo) != expect {
        return Err("hash del kernel en el hueco no coincide".into());
    }
    let viejo = leer_fichero_bytes(KERNEL, UPD_KERNEL_SLOT_SIZE)
        .ok_or_else(|| "no pude leer kernel-x86_64".to_string())?;
    let viejo_len = viejo.len();
    escribir_fichero(SLOT, &viejo).map_err(|e| format!("escribir copia en hueco: {e}"))?;
    escribir_fichero(KERNEL, &nuevo).map_err(|e| format!("escribir kernel nuevo: {e}"))?;
    let _ = viejo_len;
    let payload = soso_update_core::Mailbox::format_probando(version);
    escribir_buzon(&payload).map_err(|e| format!("escribir PROBANDO: {e}"))
}

fn revertir_kernel(version: &str) -> Result<(), String> {
    let backup = leer_fichero_bytes(SLOT, UPD_KERNEL_SLOT_SIZE)
        .ok_or_else(|| "no pude leer copia en SOSOKRN.BIN".to_string())?;
    if backup.iter().all(|&b| b == 0) {
        return Err("hueco vacío; nada que restaurar".into());
    }
    // El backup puede ser más corto que el slot; escribir solo bytes no nulos al final ELF.
    let len = backup
        .iter()
        .rposition(|&b| b != 0)
        .map(|i| i + 1)
        .unwrap_or(0);
    escribir_fichero(KERNEL, &backup[..len])
        .map_err(|e| format!("restaurar kernel-x86_64: {e}"))?;
    let ver = if version.is_empty() {
        "?"
    } else {
        version
    };
    let payload = soso_update_core::Mailbox::format_revertido(ver);
    escribir_buzon(&payload).map_err(|e| format!("escribir REVERTIDO: {e}"))
}

fn sha256(data: &[u8]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(data);
    h.finalize().into()
}

fn escribir_buzon(payload: &[u8]) -> Result<(), &'static str> {
    escribir_fichero(BUZON, payload)
}

fn leer_fichero(path: &CStr16) -> Option<String> {
    let data = leer_fichero_bytes(path, 4096)?;
    Some(String::from_utf8_lossy(&data).into_owned())
}

fn leer_fichero_bytes(path: &CStr16, max: usize) -> Option<Vec<u8>> {
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

fn escribir_fichero(path: &CStr16, data: &[u8]) -> Result<(), &'static str> {
    let mut fs = boot::get_image_file_system(boot::image_handle())
        .map_err(|_| "sin filesystem")?;
    let mut root = fs.open_volume().map_err(|_| "open_volume")?;
    let handle = root
        .open(path, FileMode::CreateReadWrite, FileAttribute::empty())
        .map_err(|_| "open")?;
    let mut f: RegularFile = handle.into_regular_file().ok_or("not regular")?;
    let _ = f.set_position(0);
    f.write(data).map_err(|_| "write")?;
    f.flush().map_err(|_| "flush")
}
