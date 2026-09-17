//! Identidad del arranque: `live` o `installed` (`SOSOMODE.TXT` en la ESP).
//!
//! Entrega U2 de `docs/PLAN-ACTUALIZACIONES.md`, sobre el contrato de U0.
//! Se lee **antes de montar sosofs**, porque de ella depende a dónde van los
//! logs y, más adelante, el recuperador de actualizaciones.
//!
//! No se deduce del medio. Un live puede arrancar desde un disco interno y una
//! instalación puede conservar todavía su `SOSOLOG.TXT`: sólo el registro
//! explícito de esta ESP autoriza a apagar el log FAT.

use alloc::string::{String, ToString};
use spin::Once;

use soso_update_core::identity::{self, BootMode, Identidad};
use soso_update_core::UPD_MODE_SIZE;

use super::espfat;

static IDENT: Once<Identidad> = Once::new();

pub fn init() {
    if !crate::drivers::live_disk::esp_available() {
        return;
    }
    let guid = crate::drivers::live_disk::esp_guid()
        .map(|g| g.to_string())
        .unwrap_or_default();
    let ident = leer(&guid);
    match &ident {
        Identidad::Explicita(r) => crate::println!(
            "modo: {} (declarado en SOSOMODE.TXT{})",
            r.modo.as_str(),
            if r.fecha.is_empty() {
                String::new()
            } else {
                alloc::format!(", {}", r.fecha)
            }
        ),
        Identidad::Heredada => {
            crate::println!("modo: sin SOSOMODE.TXT; se trata como live (log en la ESP)")
        }
        Identidad::Rota(e) => {
            crate::println!("modo: SOSOMODE.TXT ilegible ({e:?}); se trata como live")
        }
        Identidad::Ajena(r) => crate::println!(
            "modo: SOSOMODE.TXT es de otra ESP ({}); clon sin finalizar, se trata como live",
            r.esp_guid
        ),
    }
    IDENT.call_once(|| ident);
}

fn leer(guid: &str) -> Identidad {
    let Some(slot) = espfat::locate(b"SOSOMODE", b"TXT", UPD_MODE_SIZE) else {
        // Sin hueco: imagen anterior a U2. Es exactamente el caso «heredada».
        return identity::resolver(None, guid);
    };
    let mut buf = alloc::vec![0u8; UPD_MODE_SIZE];
    for (i, trozo) in buf.chunks_mut(espfat::SECTOR).enumerate() {
        if espfat::read(slot.data_lba + i as u64, trozo).is_err() {
            return identity::resolver(None, guid);
        }
    }
    identity::resolver(Some(&buf), guid)
}

fn identidad() -> &'static Identidad {
    IDENT.get().unwrap_or(&Identidad::Heredada)
}

pub fn modo() -> BootMode {
    identidad().modo()
}

/// ¿Se puede apagar el log FAT y retirar `SOSOLOG.TXT`? Sólo con una identidad
/// explícita, propia y `installed`. Ante la duda se conserva el log: uno de más
/// no rompe nada, y quedarse sin el único canal de diagnóstico de una máquina
/// que no monta sosofs, sí.
pub fn instalado_declarado() -> bool {
    identidad().puede_retirar_fatlog()
}

/// Resumen para el kshell: qué identidad hay y si la ESP conserva el log FAT.
/// Es la comprobación rápida de que una instalación quedó bien finalizada.
pub fn resumen() {
    let ident = identidad();
    let etiqueta = match ident {
        Identidad::Explicita(_) => "declarada",
        Identidad::Heredada => "heredada (sin SOSOMODE.TXT)",
        Identidad::Rota(_) => "registro ilegible",
        Identidad::Ajena(_) => "de otra ESP (clon sin finalizar)",
    };
    crate::println!("modo: {} — {etiqueta}", modo().as_str());
    if crate::drivers::live_disk::esp_available() {
        let hay_log = espfat::existe(b"SOSOLOG ", b"TXT");
        crate::println!(
            "modo: SOSOLOG.TXT en la ESP: {}",
            if hay_log { "presente" } else { "ausente" }
        );
    }
}
