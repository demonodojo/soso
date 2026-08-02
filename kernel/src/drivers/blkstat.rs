//! Contadores de E/S de bloque.
//!
//! POR QUÉ EXISTE. Las dos optimizaciones grandes de la carga de modelos
//! (`8c70707c2`, `a81054a92`) se midieron con instrumentación temporal que **no
//! quedó en el árbol**: las cifras que las justifican —«54 272 lecturas de 4 KiB
//! a ~177 us», «94 bloques leídos por cada bloque de modelo»— sólo sobreviven en
//! comentarios. La siguiente optimización volvía a empezar a ciegas, y el
//! `docs/L6-G1-gate.md` ya deja escrito el precio de eso: «las tres primeras
//! hipótesis por inspección del código eran plausibles y las tres estaban
//! equivocadas».
//!
//! Aquí se cuenta lo mínimo para no volver a adivinar: cuántas **peticiones** se
//! le hacen al dispositivo, cuántos **bloques** mueven, y cuánto **tiempo** se
//! pasa esperándolas. La distinción peticiones/bloques es justo la que importa
//! cuando el objetivo es agrupar: bajar bloques es leer menos, bajar peticiones
//! es leer lo mismo en menos viajes.

use core::sync::atomic::{AtomicU64, Ordering};

static PETICIONES: AtomicU64 = AtomicU64::new(0);
static BLOQUES: AtomicU64 = AtomicU64::new(0);
static NANOS: AtomicU64 = AtomicU64::new(0);
static ESCRITURAS: AtomicU64 = AtomicU64::new(0);

/// Cronómetro de una petición. Se crea antes de tocar el dispositivo y se
/// consume después; el `Drop` no vale porque hay que saber cuántos bloques
/// movió, y eso sólo se sabe en el sitio de la llamada.
pub struct Peticion(u64);

impl Peticion {
    #[inline]
    pub fn empieza() -> Self {
        Self(crate::arch::tsc::now_ns())
    }

    /// Cierra la petición: suma una petición, `bloques` bloques y el tiempo.
    #[inline]
    pub fn termina(self, bloques: u64) {
        let dt = crate::arch::tsc::now_ns().wrapping_sub(self.0);
        PETICIONES.fetch_add(1, Ordering::Relaxed);
        BLOQUES.fetch_add(bloques, Ordering::Relaxed);
        NANOS.fetch_add(dt, Ordering::Relaxed);
    }
}

#[inline]
pub fn escritura(bloques: u64) {
    ESCRITURAS.fetch_add(1, Ordering::Relaxed);
    BLOQUES.fetch_add(bloques, Ordering::Relaxed);
}

/// (peticiones de lectura, bloques, ns en disco, escrituras).
pub fn leer() -> (u64, u64, u64, u64) {
    (
        PETICIONES.load(Ordering::Relaxed),
        BLOQUES.load(Ordering::Relaxed),
        NANOS.load(Ordering::Relaxed),
        ESCRITURAS.load(Ordering::Relaxed),
    )
}

pub fn reiniciar() {
    PETICIONES.store(0, Ordering::Relaxed);
    BLOQUES.store(0, Ordering::Relaxed);
    NANOS.store(0, Ordering::Relaxed);
    ESCRITURAS.store(0, Ordering::Relaxed);
}
