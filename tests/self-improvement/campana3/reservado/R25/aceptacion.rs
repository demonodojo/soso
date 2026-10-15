//! Aceptación reservada de R25.

use soso_update_core::{hex_sha256, KernelMeta, KernelPhase, MetaError};

fn meta(data: &[u8], new_hash: String, backup_hash: String) -> KernelMeta {
    KernelMeta {
        phase: KernelPhase::BackupReady,
        version: "0.3.0".into(),
        new_size: data.len() as u64,
        new_hash,
        backup_size: data.len() as u64,
        backup_hash,
    }
}

fn mezclado(s: &str) -> String {
    s.chars()
        .enumerate()
        .map(|(i, c)| if i % 2 == 0 { c.to_ascii_uppercase() } else { c })
        .collect()
}

#[test]
fn hash_nuevo_en_mayusculas_o_mezclado_verifica() {
    let data = b"kernel nuevo de prueba, con bastantes bytes".to_vec();
    let h = hex_sha256(&data);
    for variante in [h.clone(), h.to_uppercase(), mezclado(&h)] {
        let m = meta(&data, variante.clone(), String::new());
        assert_eq!(m.verify_new_kernel(&data), Ok(()), "{variante}");
    }
}

#[test]
fn hash_de_backup_en_mayusculas_o_mezclado_verifica() {
    let mut slot = b"kernel viejo".to_vec();
    let h = hex_sha256(&slot);
    slot.extend_from_slice(&[0u8; 100]);
    let n = 12;
    for variante in [h.clone(), h.to_uppercase(), mezclado(&h)] {
        let mut m = meta(&slot[..n], String::new(), variante.clone());
        m.backup_size = n as u64;
        assert_eq!(m.verify_backup(&slot).map(|s| s.to_vec()), Ok(slot[..n].to_vec()), "{variante}");
    }
}

#[test]
fn hash_ajeno_sigue_rechazado_en_cualquier_caja() {
    let data = b"contenido".to_vec();
    let otro = hex_sha256(b"otro contenido");
    for variante in [otro.clone(), otro.to_uppercase(), mezclado(&otro)] {
        let m = meta(&data, variante.clone(), variante.clone());
        assert_eq!(m.verify_new_kernel(&data), Err(MetaError::BadHash), "{variante}");
        assert_eq!(m.verify_backup(&data), Err(MetaError::BackupHashMismatch), "{variante}");
    }
}

#[test]
fn hash_mal_formado_es_bad_hash() {
    let data = b"contenido".to_vec();
    let h = hex_sha256(&data);
    let malos = [
        String::new(),
        h[..63].to_string(),
        format!("{h}0"),
        format!("{}g", &h[..63]),
        format!(" {}", &h[..63]),
        "z".repeat(64),
    ];
    for malo in malos {
        let m = meta(&data, malo.clone(), malo.clone());
        assert_eq!(m.verify_new_kernel(&data), Err(MetaError::BadHash), "{malo:?}");
        assert_eq!(m.verify_backup(&data), Err(MetaError::BadHash), "{malo:?}");
    }
}

#[test]
fn los_tamanos_se_comprueban_antes_que_el_hash() {
    let data = b"contenido".to_vec();
    let h = hex_sha256(&data).to_uppercase();
    let mut m = meta(&data, h.clone(), h.clone());
    m.new_size = 0;
    m.backup_size = 0;
    assert_eq!(m.verify_new_kernel(&data), Err(MetaError::MissingField));
    assert_eq!(m.verify_backup(&data), Err(MetaError::BackupEmpty));
    m.new_size = data.len() as u64 + 1;
    m.backup_size = data.len() as u64 + 1;
    assert_eq!(m.verify_new_kernel(&data), Err(MetaError::MissingField));
    assert_eq!(m.verify_backup(&data), Err(MetaError::BackupTooShort));
}
