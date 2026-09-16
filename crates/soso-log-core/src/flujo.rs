//! Estado durable de un flujo de log: cursor, pérdidas, tamaño del activo y
//! control de fallos.
//!
//! Regla que manda sobre todas las demás: **el logger no puede impedir que la
//! máquina arranque o se recupere**. Un disco lleno, un FS de sólo lectura o un
//! error de escritura suspenden el flujo y se contabilizan; la consola y el
//! ring siguen funcionando como si nada.

use crate::ring::Lectura;
use crate::rotacion::{self, Accion};

/// Tope de bytes que un flujo escribe en una pasada. Acota el tiempo que el
/// volcado tiene ocupado el disco: un ring lleno se drena en varias pasadas en
/// vez de en una escritura de 256 KiB.
pub const LOTE_MAX: usize = 64 * 1024;
/// Fallos seguidos antes de suspender el flujo.
pub const MAX_FALLOS: u32 = 3;
/// Espera antes de volver a intentarlo tras suspender.
pub const REINTENTO_MS: u64 = 30_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Estado {
    /// Aún no se ha podido abrir el destino (sosofs sin montar).
    Inactivo,
    Activo,
    /// Demasiados fallos seguidos; se reintenta pasado `REINTENTO_MS`.
    Suspendido { desde_ms: u64 },
}

#[derive(Clone, Copy, Debug)]
pub struct Flujo {
    pub nombre: &'static str,
    pub estado: Estado,
    /// Cursor del ring: hasta dónde se ha persistido.
    pub cursor: u64,
    /// Bytes perdidos por sobrescritura desde el arranque.
    pub perdidos: u64,
    /// Tamaño del fichero activo.
    pub activo_bytes: u64,
    /// Bytes que este flujo ha escrito a disco desde el arranque.
    pub escritos: u64,
    pub fallos_seguidos: u32,
    pub fallos_totales: u32,
    /// Bytes que no llegaron a disco por estar suspendido.
    pub descartados: u64,
}

/// Lo que el escritor tiene que hacer con un lote.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Trabajo {
    pub accion: Accion,
    /// Bytes a leer del ring en esta pasada.
    pub bytes: usize,
    /// Pérdida que hay que marcar antes del lote.
    pub marcar_perdida: u64,
    /// Queda material para otra pasada.
    pub mas_pendiente: bool,
}

impl Flujo {
    pub const fn nuevo(nombre: &'static str) -> Self {
        Self {
            nombre,
            estado: Estado::Inactivo,
            cursor: 0,
            perdidos: 0,
            activo_bytes: 0,
            escritos: 0,
            fallos_seguidos: 0,
            fallos_totales: 0,
            descartados: 0,
        }
    }

    /// ¿Toca intentar escribir ahora?
    pub fn disponible(&self, ahora_ms: u64) -> bool {
        match self.estado {
            Estado::Inactivo => false,
            Estado::Activo => true,
            // Reintento acotado: ni se abandona para siempre ni se insiste en
            // cada vuelta del planificador contra un disco que ya dijo que no.
            Estado::Suspendido { desde_ms } => ahora_ms.saturating_sub(desde_ms) >= REINTENTO_MS,
        }
    }

    /// Planifica la siguiente pasada a partir de lo que hay en el ring.
    /// `ring_escritos` y `ring_minimo` vienen del ring bajo su propio lock; el
    /// cálculo se hace ya fuera, para no hacer E/S con el candado tomado.
    pub fn planificar(&self, ring_escritos: u64, ring_minimo: u64) -> Option<Trabajo> {
        let cursor = self.cursor.max(ring_minimo);
        let pendientes = ring_escritos.saturating_sub(cursor);
        let marcar_perdida = ring_minimo.saturating_sub(self.cursor);
        if pendientes == 0 && marcar_perdida == 0 {
            return None;
        }
        let bytes = pendientes.min(LOTE_MAX as u64) as usize;
        Some(Trabajo {
            accion: rotacion::decidir(self.activo_bytes, bytes),
            bytes,
            marcar_perdida,
            mas_pendiente: pendientes > bytes as u64,
        })
    }

    /// Contabiliza una pasada que sí llegó a disco.
    pub fn exito(&mut self, lectura: &Lectura, escritos_en_fichero: u64, roto: bool) {
        self.cursor = lectura.cursor;
        self.perdidos += lectura.perdidos;
        self.activo_bytes = if roto { escritos_en_fichero } else { self.activo_bytes + escritos_en_fichero };
        self.escritos += escritos_en_fichero;
        self.fallos_seguidos = 0;
        self.estado = Estado::Activo;
    }

    /// Contabiliza un fallo. Tras `MAX_FALLOS` seguidos el flujo se suspende y
    /// el ring sigue siendo la única copia: se descuenta lo que se pierda.
    pub fn fallo(&mut self, ahora_ms: u64, bytes_del_lote: usize) {
        self.fallos_seguidos += 1;
        self.fallos_totales += 1;
        self.descartados += bytes_del_lote as u64;
        if self.fallos_seguidos >= MAX_FALLOS {
            self.estado = Estado::Suspendido { desde_ms: ahora_ms };
            self.fallos_seguidos = 0;
        }
    }

    /// El destino existe y se puede escribir.
    pub fn activar(&mut self, activo_bytes: u64) {
        self.estado = Estado::Activo;
        self.activo_bytes = activo_bytes;
    }
}
