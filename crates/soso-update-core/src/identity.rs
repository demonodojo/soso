//! Identidad del arranque: `live` o `installed`, en `SOSOMODE.TXT` (ESP).
//!
//! Se lee **antes** de montar sosofs, porque de ella dependen el log FAT y el
//! recuperador. No se deduce del medio (USB frente a NVMe) ni de que exista
//! `SOSOLOG.TXT`: un live arrancado desde un disco interno y una instalación
//! con su log FAT todavía sin migrar existen los dos.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::record::{self, RecordError, SLOTS, SLOT_SIZE};

pub const MODE_MAGIC: &str = "SOSOMODE";
pub const MODE_FORMATO: u16 = 1;
pub const MODE_SIZE: usize = SLOT_SIZE * SLOTS;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BootMode {
    /// Medio de arranque y diagnóstico: conserva `SOSOLOG.TXT` en su ESP.
    Live,
    /// Instalación nativa: logs en `/var/log` dentro de sosofs.
    Installed,
}

impl BootMode {
    pub fn as_str(self) -> &'static str {
        match self {
            BootMode::Live => "live",
            BootMode::Installed => "installed",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "live" => BootMode::Live,
            "installed" => BootMode::Installed,
            _ => return None,
        })
    }
    /// El log en FAT sólo sigue activo en modo live.
    pub fn usa_fatlog(self) -> bool {
        matches!(self, BootMode::Live)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModeRecord {
    pub modo: BootMode,
    pub seq: u64,
    /// Fecha de la instalación, o vacío en el live empaquetado.
    pub fecha: String,
    /// GUID de **esta** ESP. `soso-install` genera GUID nuevos al clonar; si
    /// el registro trae el del origen, es una copia sin finalizar.
    pub esp_guid: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModeError {
    Registro(RecordError),
    ModoInvalido,
}

/// Resultado de resolver la identidad.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Identidad {
    /// Declarada en la ESP.
    Explicita(ModeRecord),
    /// Sin registro: instalación o stick anterior a U2. Se trata como live
    /// —log FAT incluido— hasta que la migración U6 escriba el registro.
    Heredada,
    /// Registro presente pero ilegible: no se adivina.
    Rota(ModeError),
    /// Registro de otra ESP: clon sin finalizar.
    Ajena(ModeRecord),
}

impl Identidad {
    pub fn modo(&self) -> BootMode {
        match self {
            Identidad::Explicita(r) => r.modo,
            Identidad::Ajena(_) | Identidad::Heredada | Identidad::Rota(_) => BootMode::Live,
        }
    }
    /// Sólo una identidad explícita y propia autoriza a apagar el log FAT y a
    /// borrar `SOSOLOG.TXT` de la ESP.
    pub fn puede_retirar_fatlog(&self) -> bool {
        matches!(self, Identidad::Explicita(r) if r.modo == BootMode::Installed)
    }
}

impl ModeRecord {
    pub fn nuevo(modo: BootMode, fecha: &str, esp_guid: &str, seq: u64) -> Self {
        Self { modo, seq, fecha: fecha.into(), esp_guid: esp_guid.into() }
    }

    pub fn format(&self) -> Result<Vec<u8>, ModeError> {
        let body = format!(
            "modo={}\nfecha={}\nesp_guid={}\n",
            self.modo.as_str(),
            self.fecha,
            self.esp_guid
        );
        record::frame_slot(MODE_MAGIC, MODE_FORMATO, self.seq, &body, SLOT_SIZE)
            .map_err(ModeError::Registro)
    }

    pub fn ranura(&self) -> usize {
        record::ranura(self.seq, SLOTS)
    }
}

/// Resuelve la identidad a partir del fichero de la ESP y del GUID real de la
/// partición, que el arranque ya conoce por la GPT.
pub fn resolver(fichero: Option<&[u8]>, esp_guid: &str) -> Identidad {
    let Some(bytes) = fichero else {
        return Identidad::Heredada;
    };
    let slots: Vec<&[u8]> = bytes.chunks(SLOT_SIZE).take(SLOTS).collect();
    let f = match record::pick(MODE_MAGIC, MODE_FORMATO, &slots) {
        Ok((_, f)) => f,
        Err(RecordError::Vacia) => return Identidad::Heredada,
        Err(e) => return Identidad::Rota(ModeError::Registro(e)),
    };
    let modo = match f.campo("modo").and_then(BootMode::parse) {
        Some(m) => m,
        None => return Identidad::Rota(ModeError::ModoInvalido),
    };
    let rec = ModeRecord {
        modo,
        seq: f.seq,
        fecha: f.campo("fecha").unwrap_or("").into(),
        esp_guid: f.campo("esp_guid").unwrap_or("").into(),
    };
    if !rec.esp_guid.is_empty() && !rec.esp_guid.eq_ignore_ascii_case(esp_guid) {
        return Identidad::Ajena(rec);
    }
    Identidad::Explicita(rec)
}
