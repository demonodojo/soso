//! Registro de arranque en la ESP: `SOSOTXN.BIN`, 4 KiB pre-creados, cuatro
//! ranuras de 1 KiB escritas por turnos.
//!
//! Es el registro de **decisión**, legible por el shim y por el kernel antes
//! de montar sosofs. Sólo un registro válido puede armar la operación, y sólo
//! una decisión durable identificada por ID la confirma o la deshace.
//! `/etc/soso-release` se reconcilia con esta decisión; nunca al revés.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::record::{self, Framed, RecordError, SLOTS, SLOT_SIZE};
use crate::txn::TxnId;

pub const BOOTREC_MAGIC: &str = "SOSOTXN arranque";
pub const BOOTREC_FORMATO: u16 = 1;
/// Tamaño del fichero completo en la ESP.
pub const BOOTREC_SIZE: usize = SLOT_SIZE * SLOTS;

/// Decisión durable sobre la pareja kernel+rootfs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Decision {
    /// No hay operación en curso.
    #[default]
    Idle,
    /// Preparada y respaldada: aplicar en el próximo arranque.
    Armado,
    /// Aplicada y sin acreditar. Verla **al arrancar** significa que el
    /// arranque anterior no llegó a confirmar: hay que revertir.
    Probando,
    /// La versión nueva es la buena.
    Confirmado,
    /// Reversión pedida a mano.
    Revertir,
    /// Reversión terminada.
    Revertido,
}

impl Decision {
    pub fn as_str(self) -> &'static str {
        match self {
            Decision::Idle => "idle",
            Decision::Armado => "armado",
            Decision::Probando => "probando",
            Decision::Confirmado => "confirmado",
            Decision::Revertir => "revertir",
            Decision::Revertido => "revertido",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "idle" => Decision::Idle,
            "armado" => Decision::Armado,
            "probando" => Decision::Probando,
            "confirmado" => Decision::Confirmado,
            "revertir" => Decision::Revertir,
            "revertido" => Decision::Revertido,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BootRecord {
    pub decision: Decision,
    pub id: TxnId,
    pub seq: u64,
    /// Versión que se está instalando.
    pub version_nueva: String,
    /// Versión a la que se vuelve si hay que deshacer.
    pub version_anterior: String,
    /// Directorio de la operación dentro de sosofs (`/var/lib/soso-update/`).
    pub dir: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BootRecError {
    Registro(RecordError),
    IdInvalido,
    DecisionInvalida,
}

impl From<RecordError> for BootRecError {
    fn from(e: RecordError) -> Self {
        BootRecError::Registro(e)
    }
}

impl BootRecord {
    pub fn nuevo(decision: Decision, id: TxnId, nueva: &str, anterior: &str, seq: u64) -> Self {
        Self {
            decision,
            id,
            seq,
            version_nueva: nueva.into(),
            version_anterior: anterior.into(),
            dir: id.dir(),
        }
    }

    pub fn format(&self) -> Result<Vec<u8>, BootRecError> {
        let body = format!(
            "decision={}\nid={}\nversion_nueva={}\nversion_anterior={}\ndir={}\n",
            self.decision.as_str(),
            self.id.to_hex(),
            self.version_nueva,
            self.version_anterior,
            self.dir
        );
        record::frame_slot(BOOTREC_MAGIC, BOOTREC_FORMATO, self.seq, &body, SLOT_SIZE)
            .map_err(BootRecError::Registro)
    }

    /// Ranura donde escribir este registro: nunca encima de la última válida.
    pub fn ranura(&self) -> usize {
        record::ranura(self.seq, SLOTS)
    }

    pub fn parse(raw: &[u8]) -> Result<Self, BootRecError> {
        let f = record::parse(BOOTREC_MAGIC, BOOTREC_FORMATO, raw)?;
        Self::from_framed(&f)
    }

    /// Divide los 4 KiB del fichero en ranuras y elige la buena.
    pub fn pick(fichero: &[u8]) -> Result<Self, BootRecError> {
        let slots: Vec<&[u8]> = fichero.chunks(SLOT_SIZE).take(SLOTS).collect();
        let (_, f) = record::pick(BOOTREC_MAGIC, BOOTREC_FORMATO, &slots)?;
        Self::from_framed(&f)
    }

    fn from_framed(f: &Framed) -> Result<Self, BootRecError> {
        let id = TxnId::from_hex(f.requerido("id")?).ok_or(BootRecError::IdInvalido)?;
        Ok(Self {
            decision: Decision::parse(f.requerido("decision")?)
                .ok_or(BootRecError::DecisionInvalida)?,
            id,
            seq: f.seq,
            version_nueva: f.requerido("version_nueva")?.into(),
            version_anterior: f.requerido("version_anterior")?.into(),
            dir: f.campo("dir").unwrap_or(&id.dir()).into(),
        })
    }

    /// Versión que debe anunciar `/etc/soso-release` según esta decisión.
    /// La reconciliación es en este sentido: el fichero no decide nada.
    pub fn version_efectiva(&self) -> &str {
        match self.decision {
            Decision::Confirmado | Decision::Probando => &self.version_nueva,
            _ => &self.version_anterior,
        }
    }
}
