//! Transición de instalaciones existentes (U6): qué tiene una máquina y qué le
//! falta para poder recibir una actualización **recuperable**.
//!
//! Una instalación hecha con un live antiguo no tiene los huecos de la ESP que
//! el contrato U0 necesita, ni la entrada UEFI de rescate. Puede actualizarse
//! —el OTA viejo funciona— pero **sin vuelta atrás**, que es justo lo que este
//! plan promete. Antes de prometer nada hay que saber qué hay delante, y eso
//! es lo que resuelve este módulo: no toca nada, sólo diagnostica.
//!
//! Aquí no hay E/S. Quien sepa mirar la ESP —el cliente, el live o el shim—
//! rellena el `Inventario` y esto dice qué falta y qué se puede prometer.

use alloc::string::String;
use alloc::vec::Vec;

/// Un fichero pre-creado en la ESP, con para qué sirve. El tamaño es exacto:
/// el kernel localiza estos huecos por LBA y exige que midan lo que dicen y que
/// sus clusters sean consecutivos, porque los sobrescribe en su sitio.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hueco {
    pub nombre: &'static str,
    pub tamano: usize,
    /// Sin él no hay actualización recuperable.
    pub imprescindible: bool,
    pub para: &'static str,
}

pub const HUECOS: &[Hueco] = &[
    Hueco {
        nombre: "SOSOTXN.BIN",
        tamano: crate::UPD_BOOTREC_SIZE,
        imprescindible: true,
        para: "la decisión durable de la pareja kernel+rootfs",
    },
    Hueco {
        nombre: "SOSOKRN.BIN",
        tamano: crate::UPD_KERNEL_SLOT_SIZE,
        imprescindible: true,
        para: "el kernel nuevo y, después, la copia del anterior",
    },
    Hueco {
        nombre: "SOSOKRN.MET",
        tamano: crate::kernel_meta::KERNEL_META_SIZE,
        imprescindible: true,
        para: "en qué fase se quedó una actualización de kernel cortada",
    },
    Hueco {
        nombre: "SOSOUPD.TXT",
        tamano: 4096,
        imprescindible: true,
        para: "el buzón entre el cliente, el shim y init",
    },
    Hueco {
        nombre: "SOSOMODE.TXT",
        tamano: crate::UPD_MODE_SIZE,
        imprescindible: false,
        para: "la identidad live/instalado que el kernel lee al arrancar",
    },
];

/// Estado de un hueco tal como lo ve quien mira la ESP.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EstadoHueco {
    /// Está y mide lo que debe: utilizable.
    Listo,
    /// No está en la raíz de la ESP.
    Ausente,
    /// Está, pero no se puede usar: otro tamaño, o fragmentado.
    Inservible,
}

/// Lo que se ha podido averiguar de la instalación. Lo rellena quien mira.
#[derive(Clone, Debug, Default)]
pub struct Inventario {
    /// Un estado por hueco, en el orden de `HUECOS`.
    pub huecos: Vec<EstadoHueco>,
    /// Formato del registro de arranque que hay, si se pudo leer.
    pub formato_registro: Option<u16>,
    /// La entrada UEFI de rescate consta registrada.
    pub entrada_rescate: bool,
    /// Queda el log en FAT (`SOSOLOG.TXT`), que U2 sustituye por `/var/log`.
    pub log_fat: bool,
    /// Esto es un live, no una instalación. Cambia qué tiene sentido exigir:
    /// un USB de arranque no necesita una entrada UEFI de rescate —él **es**
    /// el rescate— y su log en FAT no sobra, es lo que se lee cuando la máquina
    /// no llega a montar nada.
    pub es_live: bool,
}

/// Algo que falta, con el arreglo que le corresponde.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Falta {
    /// Un hueco imprescindible que no está o no sirve.
    Hueco {
        nombre: String,
        estado: EstadoHueco,
        para: &'static str,
    },
    /// Hay hueco, pero el registro es de un formato que no sabe de puntos.
    RegistroSinPuntos { formato: u16 },
    /// Falta la entrada UEFI de rescate.
    EntradaRescate,
    /// Sigue el log en la ESP.
    LogEnFat,
}

impl Falta {
    /// ¿Impide prometer una actualización **recuperable**?
    pub fn bloquea(&self) -> bool {
        match self {
            Falta::Hueco { .. } => true,
            // Un formato 1 se lee sin problema; lo que no trae es punto, y el
            // primer registro que escriba el cliente nuevo ya será formato 2.
            Falta::RegistroSinPuntos { .. } => false,
            // La vuelta atrás manual sigue estando; lo que falta es la vía para
            // cuando el sistema no arranca.
            Falta::EntradaRescate => false,
            Falta::LogEnFat => false,
        }
    }

    pub fn descripcion(&self) -> String {
        match self {
            Falta::Hueco {
                nombre,
                estado,
                para,
            } => {
                let que = match estado {
                    EstadoHueco::Ausente => "no está",
                    EstadoHueco::Inservible => "está pero no se puede usar",
                    EstadoHueco::Listo => "listo",
                };
                alloc::format!("{nombre} {que} — hace falta para {para}")
            }
            Falta::RegistroSinPuntos { formato } => alloc::format!(
                "el registro de arranque es de formato {formato}: no sabe de copias guardadas"
            ),
            Falta::EntradaRescate => {
                "no consta la entrada UEFI «soso — recuperar versión anterior»".into()
            }
            Falta::LogEnFat => "el log sigue en SOSOLOG.TXT, en la ESP".into(),
        }
    }
}

/// Qué le falta a esta instalación. Vacío = está al día.
pub fn diagnosticar(inv: &Inventario) -> Vec<Falta> {
    let mut faltas = Vec::new();
    for (i, h) in HUECOS.iter().enumerate() {
        let estado = inv.huecos.get(i).copied().unwrap_or(EstadoHueco::Ausente);
        if estado != EstadoHueco::Listo && h.imprescindible {
            faltas.push(Falta::Hueco {
                nombre: h.nombre.into(),
                estado,
                para: h.para,
            });
        }
    }
    if let Some(f) = inv.formato_registro {
        if f < crate::txn::bootrec::BOOTREC_FORMATO {
            faltas.push(Falta::RegistroSinPuntos { formato: f });
        }
    }
    if !inv.entrada_rescate && !inv.es_live {
        faltas.push(Falta::EntradaRescate);
    }
    if inv.log_fat && !inv.es_live {
        faltas.push(Falta::LogEnFat);
    }
    faltas
}

/// ¿Puede esta máquina recibir una actualización **recuperable**?
///
/// Es la pregunta que importa antes de prometerle nada a nadie: si la respuesta
/// es no, el cliente tiene que decirlo **antes** de descargar, no descubrirlo
/// cuando ya no hay dónde escribir la decisión.
pub fn admite_recuperable(faltas: &[Falta]) -> bool {
    !faltas.iter().any(Falta::bloquea)
}
