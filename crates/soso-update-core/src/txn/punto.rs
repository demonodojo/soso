//! Punto de recuperación **retenido** (entrega U5a, sección 3.6 del plan).
//!
//! No es lo mismo que el respaldo de una operación en curso. Aquel vive
//! mientras dura la transacción; un punto retenido es la versión A guardada
//! **mientras B sea la activa**, aunque B esté confirmada y se reinicie muchas
//! veces. Eso es lo que permite volver atrás cuando el fallo no aparece en el
//! primer arranque —una aplicación que no va, el WiFi que deja de asociar— que
//! es justo cuando la confirmación ya se dio por buena.
//!
//! Reglas que gobiernan este módulo:
//!
//! 1. **No caduca.** Ni por tiempo, ni por limpieza de caché, ni por falta de
//!    espacio. Si no cabe algo, lo que se rechaza es la actualización nueva.
//! 2. **Se verifica releyéndolo.** Un punto que no se puede releer entero no
//!    sirve para nada, y creerlo bueno es peor que no tenerlo: se descubre el
//!    día que hace falta.
//! 3. **Sólo se recoge lo no referenciado** por el sistema activo, por una
//!    transacción pendiente o por su recuperación.

use alloc::string::String;
use alloc::vec::Vec;

use crate::record::{self, RecordError};
use crate::txn::aplicador::{De, Fallo, Sistema};
use crate::txn::journal::{Accion, Contenido, Entrada};
use crate::txn::TxnId;

pub const PUNTO_MAGIC: &str = "SOSOPUNTO";
/// Formato 1 del punto retenido. La envoltura de `record` ya rechaza un formato
/// más nuevo del que entiende este recuperador, que es lo que impide aceptar a
/// medias un punto escrito por una versión posterior.
pub const PUNTO_FORMATO: u16 = 1;

/// Todo lo que hace falta para volver a una versión.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Punto {
    /// Transacción que lo creó: identifica el punto sin ambigüedad.
    pub id: TxnId,
    /// Versión a la que se vuelve.
    pub version: String,
    pub build: String,
    /// GUID del destino. Un punto de otra instalación no se aplica aquí.
    pub guid_destino: String,
    /// Kernel de esa versión.
    pub kernel: Contenido,
    /// Qué restaurar y qué retirar. `Crear` = lo añadió la versión nueva, así
    /// que volver atrás significa **quitarlo**.
    pub entradas: Vec<Entrada>,
    pub seq: u64,
    /// Formato con el que se leyó, para no tratar un registro antiguo como si
    /// trajera campos que no tiene.
    pub formato: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PuntoError {
    Registro(RecordError),
    IdInvalido,
    Campo(&'static str),
    /// Es de otra instalación: aplicarlo sería restaurar el disco de otro.
    OtroDestino { espera: String, hay: String },
}

impl From<RecordError> for PuntoError {
    fn from(e: RecordError) -> Self {
        PuntoError::Registro(e)
    }
}

fn contenido_fmt(c: &Contenido) -> String {
    alloc::format!("{} {}", c.size, c.hash)
}

fn contenido_parse(s: &str) -> Result<Contenido, PuntoError> {
    let (size, hash) = s.split_once(' ').ok_or(PuntoError::Campo("contenido"))?;
    Ok(Contenido {
        size: size.parse().map_err(|_| PuntoError::Campo("size"))?,
        hash: hash.into(),
    })
}

impl Punto {
    pub fn nuevo(
        id: TxnId,
        version: &str,
        build: &str,
        guid_destino: &str,
        kernel: Contenido,
        entradas: Vec<Entrada>,
    ) -> Self {
        Self {
            id,
            version: version.into(),
            build: build.into(),
            guid_destino: guid_destino.into(),
            kernel,
            entradas,
            seq: 1,
            formato: PUNTO_FORMATO,
        }
    }

    pub fn format(&self) -> Vec<u8> {
        use alloc::format;
        let mut body = String::new();
        body.push_str(&format!("id={}\n", self.id.to_hex()));
        body.push_str(&format!("version={}\n", self.version));
        body.push_str(&format!("build={}\n", self.build));
        body.push_str(&format!("guid={}\n", self.guid_destino));
        body.push_str(&format!("kernel={}\n", contenido_fmt(&self.kernel)));
        for e in &self.entradas {
            let accion = match e.accion {
                Accion::Crear => "quitar",
                Accion::Reemplazar => "restaurar",
                Accion::Borrar => "reponer",
            };
            let c = e
                .respaldo
                .as_ref()
                .map(contenido_fmt)
                .unwrap_or_else(|| "- -".into());
            body.push_str(&format!("p {accion} {c} {}\n", e.path));
        }
        record::frame(PUNTO_MAGIC, PUNTO_FORMATO, self.seq, &body)
    }

    pub fn parse(raw: &[u8]) -> Result<Self, PuntoError> {
        let f = record::parse(PUNTO_MAGIC, PUNTO_FORMATO, raw)?;
        let mut entradas = Vec::new();
        for linea in f.lineas("p ") {
            let partes: Vec<&str> = linea.splitn(5, ' ').collect();
            if partes.len() != 5 {
                return Err(PuntoError::Campo("entrada"));
            }
            let accion = match partes[1] {
                "quitar" => Accion::Crear,
                "restaurar" => Accion::Reemplazar,
                "reponer" => Accion::Borrar,
                _ => return Err(PuntoError::Campo("accion")),
            };
            let respaldo = if partes[2] == "-" {
                None
            } else {
                Some(contenido_parse(&alloc::format!("{} {}", partes[2], partes[3]))?)
            };
            entradas.push(Entrada {
                accion,
                progreso: crate::txn::journal::Progreso::Respaldado,
                path: partes[4].into(),
                nuevo: None,
                respaldo,
            });
        }
        Ok(Self {
            id: TxnId::from_hex(f.requerido("id")?).ok_or(PuntoError::IdInvalido)?,
            version: f.requerido("version")?.into(),
            build: f.requerido("build")?.into(),
            guid_destino: f.requerido("guid")?.into(),
            kernel: contenido_parse(f.requerido("kernel")?)?,
            entradas,
            seq: f.seq,
            formato: f.formato,
        })
    }

    /// ¿Es de **esta** instalación?
    pub fn comprobar_destino(&self, guid: &str) -> Result<(), PuntoError> {
        if self.guid_destino.eq_ignore_ascii_case(guid) {
            Ok(())
        } else {
            Err(PuntoError::OtroDestino {
                espera: self.guid_destino.clone(),
                hay: guid.into(),
            })
        }
    }

    /// Relee el punto entero y comprueba tamaños y hashes.
    ///
    /// Se hace **antes de armar** la actualización siguiente y antes de usarlo
    /// para volver atrás. Un punto que se da por bueno sin releer se descubre
    /// roto el día que hace falta, que es el peor día.
    pub fn verificar<S: Sistema>(&self, s: &mut S) -> Result<(), Fallo> {
        for e in &self.entradas {
            // Lo que la versión nueva añadió no tiene copia: volver atrás es
            // quitarlo, y para eso no hace falta leer nada.
            let Some(resp) = e.respaldo.as_ref() else {
                continue;
            };
            let datos = s
                .leer(De::Respaldo, &e.path)
                .ok_or_else(|| Fallo::FaltaRespaldo(e.path.clone()))?;
            if datos.len() as u64 != resp.size
                || crate::hash::hex_sha256(&datos) != resp.hash
            {
                return Err(Fallo::HashDistinto(e.path.clone()));
            }
        }
        Ok(())
    }

    /// Bytes que hay que poder escribir para restaurar este punto.
    pub fn bytes_a_restaurar(&self) -> u64 {
        self.entradas
            .iter()
            .filter_map(|e| e.respaldo.as_ref().map(|c| c.size))
            .sum::<u64>()
            + self.kernel.size
    }
}

/// Reserva **efectiva** para poder restaurar un punto.
///
/// La reserva fija de U0 es un mínimo, no una demostración de que quepa
/// cualquier restauración. Esto calcula el peor caso real: reescribir todo lo
/// del punto con copia en escritura —que necesita el bloque nuevo antes de
/// soltar el viejo—, más el diario y los logs.
pub fn reserva_efectiva(punto: &Punto) -> u64 {
    let datos = punto.bytes_a_restaurar();
    // CoW: durante la restauración conviven el bloque viejo y el nuevo.
    let cow = datos;
    datos
        .saturating_add(cow)
        .saturating_add(crate::txn::RESERVA_LOGS)
        .saturating_add(crate::txn::RESERVA_RECUPERACION)
}

/// ¿Se puede recoger este punto?
///
/// Sólo si **nadie** lo referencia: ni la versión activa como su vuelta atrás,
/// ni una transacción pendiente, ni la recuperación de esa transacción. Durante
/// un A→B→C conviven dos puntos a propósito, y el de A no se toca hasta que C
/// esté confirmada y el punto de B acreditado.
pub fn recogible(punto: &Punto, referencias: &[TxnId]) -> bool {
    !referencias.iter().any(|r| *r == punto.id)
}
