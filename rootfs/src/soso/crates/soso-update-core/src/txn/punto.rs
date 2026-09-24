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

/// `- -` significa **no consta**, y hay que escribirlo así explícitamente.
///
/// Con un hash vacío salía `0 ` y, al releer, la línea se recorta y queda `0`:
/// un campo que ya no se puede partir en dos y que hace ilegible el registro
/// entero. Pasó con el kernel de una máquina recién instalada, donde nadie ha
/// anotado todavía su hash.
fn contenido_fmt(c: &Contenido) -> String {
    if c.hash.is_empty() {
        return String::from("- -");
    }
    alloc::format!("{} {}", c.size, c.hash)
}

fn contenido_parse(s: &str) -> Result<Contenido, PuntoError> {
    let s = s.trim();
    if s == "- -" || s == "-" {
        return Ok(Contenido { size: 0, hash: String::new() });
    }
    let (size, hash) = s.split_once(' ').ok_or(PuntoError::Campo("contenido"))?;
    Ok(Contenido {
        size: size.parse().map_err(|_| PuntoError::Campo("size"))?,
        hash: hash.trim().into(),
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

// ─────────────────────────── Creación y conservación (U5b) ───────────────

/// Lo que el creador de puntos necesita del sistema. El kernel y el cliente lo
/// implementan; el banco host lo simula para poder quedarse sin espacio o
/// dejar una copia a medias sin tener que provocarlo de verdad.
pub trait Almacen {
    /// Contenido actual de un fichero del sistema, o `None` si no existe.
    fn leer_sistema(&mut self, ruta: &str) -> Option<Vec<u8>>;
    /// Guarda una copia dentro del punto.
    fn guardar_copia(&mut self, punto: TxnId, ruta: &str, datos: &[u8]) -> Result<(), ()>;
    /// Relee una copia ya guardada. Tiene que ir **al medio**, no a una caché:
    /// la verificación existe para detectar lo que no llegó a disco.
    fn releer_copia(&mut self, punto: TxnId, ruta: &str) -> Option<Vec<u8>>;
    fn guardar_punto(&mut self, p: &Punto) -> Result<(), ()>;
    fn borrar_punto(&mut self, punto: TxnId) -> Result<(), ()>;
    fn espacio_libre(&mut self) -> u64;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CrearError {
    /// No cabe el punto **y** su restauración: no se arma.
    SinEspacio { necesita: u64, libre: u64 },
    /// No se pudo copiar un fichero al punto.
    Copia(String),
    /// La copia no se pudo releer, o no coincide. Un punto a medias es peor que
    /// no tenerlo: se descubre el día que hace falta.
    Incompleto(String),
    /// No se pudo dejar durable el registro del punto.
    Registro,
}

/// Qué le va a pasar a una ruta con la versión nueva.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Cambio {
    /// La versión nueva la trae (exista o no hoy).
    Trae(String),
    /// La versión nueva ya no la incluye: hay que poder reponerla al volver.
    Retira(String),
}

/// Crea el punto de la versión **actual** antes de armar la siguiente.
///
/// El orden importa: primero se comprueba que cabe, después se copia, y sólo
/// entonces se relee todo y se hace durable el registro. Si algo falla, se
/// devuelve error y **no se arma**: publicar el registro de arranque sin una
/// vuelta atrás verificada es justo lo que este contrato evita.
pub fn crear_punto<A: Almacen>(
    id: TxnId,
    version: &str,
    build: &str,
    guid_destino: &str,
    kernel: Contenido,
    cambios: &[Cambio],
    a: &mut A,
) -> Result<Punto, CrearError> {
    let mut entradas = Vec::new();
    let mut copias: Vec<(String, Vec<u8>)> = Vec::new();

    for c in cambios {
        let (ruta, retira) = match c {
            Cambio::Trae(r) => (r, false),
            Cambio::Retira(r) => (r, true),
        };
        match a.leer_sistema(ruta) {
            Some(datos) => {
                entradas.push(Entrada {
                    // Al volver: lo que la nueva reemplaza se restaura, y lo
                    // que retira se repone. Las dos cosas necesitan copia.
                    accion: if retira { Accion::Borrar } else { Accion::Reemplazar },
                    progreso: crate::txn::journal::Progreso::Respaldado,
                    path: ruta.clone(),
                    nuevo: None,
                    respaldo: Some(Contenido {
                        size: datos.len() as u64,
                        hash: crate::hash::hex_sha256(&datos),
                    }),
                });
                copias.push((ruta.clone(), datos));
            }
            None => {
                // Hoy no existe: lo añade la versión nueva, así que volver
                // atrás es quitarlo. No hay nada que copiar.
                entradas.push(Entrada {
                    accion: Accion::Crear,
                    progreso: crate::txn::journal::Progreso::Respaldado,
                    path: ruta.clone(),
                    nuevo: None,
                    respaldo: None,
                });
            }
        }
    }

    let punto = Punto::nuevo(id, version, build, guid_destino, kernel, entradas);

    // Cabe el punto **y** lo que costará restaurarlo. Comprobarlo después de
    // copiar sería tarde: ya habríamos gastado el espacio.
    let necesita = copias.iter().map(|(_, d)| d.len() as u64).sum::<u64>()
        + reserva_efectiva(&punto);
    let libre = a.espacio_libre();
    if necesita > libre {
        return Err(CrearError::SinEspacio { necesita, libre });
    }

    for (ruta, datos) in &copias {
        a.guardar_copia(id, ruta, datos)
            .map_err(|_| CrearError::Copia(ruta.clone()))?;
    }

    // Releer **todo** antes de darlo por bueno.
    for e in &punto.entradas {
        let Some(resp) = e.respaldo.as_ref() else {
            continue;
        };
        let leido = a
            .releer_copia(id, &e.path)
            .ok_or_else(|| CrearError::Incompleto(e.path.clone()))?;
        if leido.len() as u64 != resp.size || crate::hash::hex_sha256(&leido) != resp.hash {
            return Err(CrearError::Incompleto(e.path.clone()));
        }
    }

    a.guardar_punto(&punto).map_err(|_| CrearError::Registro)?;
    Ok(punto)
}

/// Qué puntos hay que conservar mientras haya una operación en vuelo.
///
/// Durante A→B→C conviven dos a propósito: el de A —que es la vuelta atrás de
/// la versión activa B— y el de B, que se crea al armar C. Recoger el de A al
/// preparar C dejaría la máquina sin camino de vuelta si C falla y B también.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Retencion {
    /// Punto con el que la versión **activa** puede volver atrás.
    pub activo: Option<TxnId>,
    /// Punto creado al armar la operación pendiente, aún sin confirmar.
    pub armado: Option<TxnId>,
}

impl Retencion {
    pub fn referencias(&self) -> Vec<TxnId> {
        let mut v = Vec::new();
        if let Some(a) = self.activo {
            v.push(a);
        }
        if let Some(b) = self.armado {
            if !v.contains(&b) {
                v.push(b);
            }
        }
        v
    }

    /// La operación se confirmó: el punto que se armó pasa a ser la vuelta
    /// atrás de la versión nueva y el anterior queda libre. Devuelve lo que se
    /// puede recoger.
    ///
    /// **Sólo si el punto nuevo está verificado.** Si no, se conservan los dos:
    /// quedarse sin ninguna copia recuperable por recoger una que no sabíamos
    /// si servía es precisamente lo que no puede pasar.
    pub fn al_confirmar(&mut self, armado_verificado: bool) -> Vec<TxnId> {
        let Some(nuevo) = self.armado.take() else {
            return Vec::new();
        };
        if !armado_verificado {
            self.armado = Some(nuevo);
            return Vec::new();
        }
        let anterior = self.activo.replace(nuevo);
        match anterior {
            Some(a) if a != nuevo => alloc::vec![a],
            _ => Vec::new(),
        }
    }

    /// La operación se deshizo: el punto que se armó para ella sobra, y el de
    /// la versión activa sigue donde estaba.
    pub fn al_revertir(&mut self) -> Vec<TxnId> {
        match self.armado.take() {
            Some(b) if Some(b) != self.activo => alloc::vec![b],
            _ => Vec::new(),
        }
    }
}

/// Puntos que se pueden recoger: los que **nadie** referencia.
///
/// La regla que no se puede romper es no quedarse sin ninguna copia
/// recuperable. Por eso, si no consta ninguna referencia, no se recoge nada:
/// sin saber cuál es el bueno, borrar es peor que ocupar sitio. Y la limpieza
/// nunca decide por tiempo ni por espacio escaso (sección 3.6).
pub fn a_recoger(todos: &[TxnId], referencias: &[TxnId]) -> Vec<TxnId> {
    if referencias.is_empty() {
        return Vec::new();
    }
    todos
        .iter()
        .copied()
        .filter(|p| !referencias.contains(p))
        .collect()
}

/// ¿Es esta release la que acaba de fallar y se deshizo?
///
/// Sirve para no reinstalar a ciegas la versión que la máquina ya rechazó: sin
/// esto, un `aplicar` repetido vuelve a armar lo mismo y se entra en el bucle
/// de aplicar, fallar y deshacer.
pub fn candidata_fallida(decision: crate::txn::bootrec::Decision, version_registro: &str, version: &str) -> bool {
    matches!(
        decision,
        crate::txn::bootrec::Decision::Revertido
            | crate::txn::bootrec::Decision::Rescatar
            | crate::txn::bootrec::Decision::RestauradoAPrueba
    ) && !version.is_empty()
        && version_registro == version
}
