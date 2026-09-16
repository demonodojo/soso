//! Diario de la operación en sosofs: `/var/lib/soso-update/<id>/diario.{0,1}`.
//!
//! Es el registro de **contenido**: qué cambia, con qué hash, qué había antes
//! y hasta dónde se ha llegado. La decisión de quedarse con la versión nueva
//! vive en la ESP (`bootrec`); el diario nunca la toma por su cuenta.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::record::{self, Framed, RecordError};
use crate::txn::{TxnId, TxnState};

pub const JOURNAL_MAGIC: &str = "SOSOTXN diario";
pub const JOURNAL_FORMATO: u16 = 1;

/// Qué hay que hacerle a una ruta administrada por la release.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Accion {
    /// No existía en la versión anterior.
    Crear,
    /// Existía con otro contenido: hay respaldo.
    Reemplazar,
    /// La versión nueva ya no lo incluye: hay respaldo.
    Borrar,
}

impl Accion {
    fn as_str(self) -> &'static str {
        match self {
            Accion::Crear => "crear",
            Accion::Reemplazar => "reemplazar",
            Accion::Borrar => "borrar",
        }
    }
    fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "crear" => Accion::Crear,
            "reemplazar" => Accion::Reemplazar,
            "borrar" => Accion::Borrar,
            _ => return None,
        })
    }
    /// Sólo `Crear` puede no tener respaldo: no había nada que copiar.
    pub fn exige_respaldo(self) -> bool {
        !matches!(self, Accion::Crear)
    }
}

/// Avance durable de una entrada. Permite continuar sin repetir trabajo y sin
/// depender de haber leído el fichero destino.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Progreso {
    Pendiente,
    /// Respaldo escrito y verificado (o innecesario, en `Crear`).
    Respaldado,
    /// La acción ya está hecha sobre el sistema activo.
    Aplicado,
    /// Deshecha desde el respaldo.
    Restaurado,
}

impl Progreso {
    fn as_str(self) -> &'static str {
        match self {
            Progreso::Pendiente => "pendiente",
            Progreso::Respaldado => "respaldado",
            Progreso::Aplicado => "aplicado",
            Progreso::Restaurado => "restaurado",
        }
    }
    fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "pendiente" => Progreso::Pendiente,
            "respaldado" => Progreso::Respaldado,
            "aplicado" => Progreso::Aplicado,
            "restaurado" => Progreso::Restaurado,
            _ => return None,
        })
    }
}

/// Tamaño y hash de un contenido. `None` significa «no existía».
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Contenido {
    pub size: u64,
    pub hash: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entrada {
    pub accion: Accion,
    pub progreso: Progreso,
    pub path: String,
    /// Contenido que deja la versión nueva (`None` en `Borrar`).
    pub nuevo: Option<Contenido>,
    /// Contenido de la versión anterior, respaldado (`None` en `Crear`).
    pub respaldo: Option<Contenido>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Journal {
    pub id: TxnId,
    pub estado: TxnState,
    pub seq: u64,
    pub version_nueva: String,
    pub version_anterior: String,
    pub kernel_nuevo: Option<Contenido>,
    pub kernel_anterior: Option<Contenido>,
    pub entradas: Vec<Entrada>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JournalError {
    Registro(RecordError),
    IdInvalido,
    EstadoInvalido,
    Entrada(&'static str),
    RutaInvalida(String),
    RutaDuplicada(String),
    /// Una entrada que debe poder deshacerse no tiene respaldo declarado.
    SinRespaldo(String),
    /// Progreso imposible para la acción (p. ej. `Crear` marcado `restaurado`
    /// sin respaldo, o aplicado sin haber respaldado antes).
    ProgresoInconsistente(String),
    /// El diario dice haber armado sin kernel de la pareja.
    SinKernel,
}

impl From<RecordError> for JournalError {
    fn from(e: RecordError) -> Self {
        JournalError::Registro(e)
    }
}

fn contenido_fmt(c: &Option<Contenido>) -> String {
    match c {
        Some(c) => format!("{} {}", c.size, c.hash),
        None => "- -".to_string(),
    }
}

fn contenido_parse(size: &str, hash: &str) -> Result<Option<Contenido>, JournalError> {
    if size == "-" && hash == "-" {
        return Ok(None);
    }
    let size = size.parse().map_err(|_| JournalError::Entrada("size"))?;
    if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(JournalError::Entrada("hash"));
    }
    Ok(Some(Contenido { size, hash: hash.into() }))
}

/// Mismas reglas de ruta que el manifiesto: relativa, sin `..` ni absolutas.
pub fn ruta_valida(p: &str) -> bool {
    !p.is_empty()
        && !p.starts_with('/')
        && !p.contains('\\')
        && !p.split('/').any(|seg| seg == ".." || seg.is_empty())
}

impl Journal {
    pub fn nuevo(id: TxnId, version_nueva: &str, version_anterior: &str) -> Self {
        Self {
            id,
            estado: TxnState::Descargando,
            seq: 0,
            version_nueva: version_nueva.into(),
            version_anterior: version_anterior.into(),
            kernel_nuevo: None,
            kernel_anterior: None,
            entradas: Vec::new(),
        }
    }

    pub fn format(&self) -> Vec<u8> {
        let mut body = String::new();
        body.push_str(&format!("id={}\n", self.id.to_hex()));
        body.push_str(&format!("estado={}\n", self.estado.as_str()));
        body.push_str(&format!("version_nueva={}\n", self.version_nueva));
        body.push_str(&format!("version_anterior={}\n", self.version_anterior));
        body.push_str(&format!("kernel_nuevo={}\n", contenido_fmt(&self.kernel_nuevo)));
        body.push_str(&format!("kernel_anterior={}\n", contenido_fmt(&self.kernel_anterior)));
        for e in &self.entradas {
            body.push_str(&format!(
                "a {} {} {} {} {}\n",
                e.accion.as_str(),
                e.progreso.as_str(),
                contenido_fmt(&e.nuevo),
                contenido_fmt(&e.respaldo),
                e.path
            ));
        }
        record::frame(JOURNAL_MAGIC, JOURNAL_FORMATO, self.seq, &body)
    }

    pub fn parse(raw: &[u8]) -> Result<Self, JournalError> {
        let f = record::parse(JOURNAL_MAGIC, JOURNAL_FORMATO, raw)?;
        Self::from_framed(&f)
    }

    /// Elige la copia válida de mayor secuencia entre `diario.0` y `diario.1`.
    pub fn pick(copias: &[&[u8]]) -> Result<Self, JournalError> {
        let (_, f) = record::pick(JOURNAL_MAGIC, JOURNAL_FORMATO, copias)?;
        Self::from_framed(&f)
    }

    fn from_framed(f: &Framed) -> Result<Self, JournalError> {
        let id = TxnId::from_hex(f.requerido("id")?).ok_or(JournalError::IdInvalido)?;
        let estado = TxnState::parse(f.requerido("estado")?).ok_or(JournalError::EstadoInvalido)?;
        let kernel = |clave: &'static str| -> Result<Option<Contenido>, JournalError> {
            let v = f.requerido(clave)?;
            let (s, h) = v.split_once(' ').ok_or(JournalError::Entrada("kernel"))?;
            contenido_parse(s, h)
        };
        let mut entradas = Vec::new();
        for linea in f.lineas("a ") {
            // `a <accion> <progreso> <nuevo> <respaldo> <ruta>`, con cada
            // contenido en dos campos y la ruta al final (puede llevar espacios).
            let partes: Vec<&str> = linea.splitn(8, ' ').collect();
            if partes.len() != 8 {
                return Err(JournalError::Entrada("campos"));
            }
            entradas.push(Entrada {
                accion: Accion::parse(partes[1]).ok_or(JournalError::Entrada("accion"))?,
                progreso: Progreso::parse(partes[2]).ok_or(JournalError::Entrada("progreso"))?,
                nuevo: contenido_parse(partes[3], partes[4])?,
                respaldo: contenido_parse(partes[5], partes[6])?,
                path: partes[7].into(),
            });
        }
        Ok(Self {
            id,
            estado,
            seq: f.seq,
            version_nueva: f.requerido("version_nueva")?.into(),
            version_anterior: f.requerido("version_anterior")?.into(),
            kernel_nuevo: kernel("kernel_nuevo")?,
            kernel_anterior: kernel("kernel_anterior")?,
            entradas,
        })
    }

    /// Coherencia interna. Se comprueba **antes** de publicar el registro de
    /// arranque: a partir de ahí el diario es lo único que permite deshacer.
    pub fn validate(&self) -> Result<(), JournalError> {
        let mut vistas: alloc::collections::BTreeSet<&str> = Default::default();
        for e in &self.entradas {
            if !ruta_valida(&e.path) {
                return Err(JournalError::RutaInvalida(e.path.clone()));
            }
            if !vistas.insert(e.path.as_str()) {
                return Err(JournalError::RutaDuplicada(e.path.clone()));
            }
            match e.accion {
                Accion::Crear => {
                    if e.respaldo.is_some() || e.nuevo.is_none() {
                        return Err(JournalError::Entrada("crear"));
                    }
                }
                Accion::Reemplazar => {
                    if e.nuevo.is_none() {
                        return Err(JournalError::Entrada("reemplazar"));
                    }
                }
                Accion::Borrar => {
                    if e.nuevo.is_some() {
                        return Err(JournalError::Entrada("borrar"));
                    }
                }
            }
            if self.estado.exige_respaldos() && e.accion.exige_respaldo() && e.respaldo.is_none() {
                return Err(JournalError::SinRespaldo(e.path.clone()));
            }
            if e.progreso == Progreso::Restaurado && e.accion.exige_respaldo() && e.respaldo.is_none()
            {
                return Err(JournalError::ProgresoInconsistente(e.path.clone()));
            }
            if e.progreso == Progreso::Aplicado
                && e.accion.exige_respaldo()
                && e.respaldo.is_none()
            {
                return Err(JournalError::ProgresoInconsistente(e.path.clone()));
            }
        }
        if self.estado.exige_respaldos()
            && (self.kernel_nuevo.is_none() || self.kernel_anterior.is_none())
        {
            return Err(JournalError::SinKernel);
        }
        Ok(())
    }

    /// Entradas que aún hay que aplicar. Es la base de la idempotencia: repetir
    /// la aplicación tras un corte no vuelve a tocar lo ya hecho.
    pub fn pendientes_aplicar(&self) -> impl Iterator<Item = &Entrada> {
        self.entradas.iter().filter(|e| e.progreso != Progreso::Aplicado)
    }

    /// Entradas que aún hay que deshacer: las que llegaron a tocarse.
    pub fn pendientes_restaurar(&self) -> impl Iterator<Item = &Entrada> {
        self.entradas.iter().filter(|e| e.progreso == Progreso::Aplicado)
    }

    /// Avanza el estado y la secuencia: cada escritura del diario es una
    /// secuencia nueva, y la ranura se elige por turnos.
    pub fn avanzar(&mut self, ev: crate::txn::TxnEvent) -> Result<TxnState, crate::txn::TransicionInvalida> {
        let siguiente = self.estado.next(ev)?;
        self.estado = siguiente;
        self.seq += 1;
        Ok(siguiente)
    }
}
