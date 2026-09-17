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
/// Formato 2 (U5a): añade el punto retenido y la decisión de rescate. Los
/// registros de formato 1 **siguen leyéndose**; simplemente no traen punto, y
/// quien los lea tiene que saberlo en vez de suponer que hay vuelta atrás.
pub const BOOTREC_FORMATO: u16 = 2;
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
    /// Punto restaurado y **a prueba**: falta que ese arranque se acredite.
    ///
    /// Es la simétrica de `probando`, para la vuelta atrás. Verla al arrancar
    /// significa que la versión restaurada **tampoco** llegó a init, y entonces
    /// lo que toca es decirlo, no repetir la restauración en bucle.
    RestauradoAPrueba,
    /// Rescate pedido desde fuera del sistema actualizado (entrada UEFI o live):
    /// restaurar el **punto retenido**, no la operación en curso. Es la vía para
    /// cuando el fallo aparece después de confirmar, o cuando init/sosh no
    /// arrancan y no hay dónde escribir una petición normal.
    Rescatar,
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
            Decision::Rescatar => "rescatar",
            Decision::RestauradoAPrueba => "restaurado-a-prueba",
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
            "rescatar" => Decision::Rescatar,
            "restaurado-a-prueba" => Decision::RestauradoAPrueba,
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
    /// Punto retenido al que se vuelve (formato 2). `None` en registros de
    /// formato 1: no es que no haya vuelta atrás, es que ese registro no sabe
    /// de puntos, y confundirlo con «no hay» sería inventarse una garantía.
    pub punto: Option<TxnId>,
    /// Formato con el que se leyó.
    pub formato: u16,
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
            punto: None,
            formato: BOOTREC_FORMATO,
        }
    }

    /// Ata el registro a un punto retenido.
    pub fn con_punto(mut self, punto: TxnId) -> Self {
        self.punto = Some(punto);
        self
    }

    pub fn format(&self) -> Result<Vec<u8>, BootRecError> {
        let mut body = format!(
            "decision={}\nid={}\nversion_nueva={}\nversion_anterior={}\ndir={}\n",
            self.decision.as_str(),
            self.id.to_hex(),
            self.version_nueva,
            self.version_anterior,
            self.dir
        );
        if let Some(p) = self.punto {
            body.push_str(&format!("punto={}\n", p.to_hex()));
        }
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
            punto: f.campo("punto").and_then(TxnId::from_hex),
            formato: f.formato,
        })
    }

    /// Versión que debe anunciar `/etc/soso-release` según esta decisión.
    /// La reconciliación es en este sentido: el fichero no decide nada.
    pub fn version_efectiva(&self) -> &str {
        match self.decision {
            Decision::Confirmado | Decision::Probando => &self.version_nueva,
            // Ya se restauró: lo que corre es la anterior, aunque falte
            // acreditar que arranca.
            Decision::RestauradoAPrueba => &self.version_anterior,
            _ => &self.version_anterior,
        }
    }

    /// ¿Este registro sabe de puntos retenidos? Un formato 1 **no**, y hay que
    /// tratarlo como «no consta», no como «no hay».
    pub fn conoce_puntos(&self) -> bool {
        self.formato >= 2
    }
}
