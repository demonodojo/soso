//! Abstracción de lectura de artefactos (tests host y clientes).

#[cfg(feature = "std")]
use alloc::string::String;
use alloc::vec::Vec;

/// Devuelve bytes de una ruta relativa (`indice.txt`, `demo/1.0.0/paquete.txt`, …).
pub trait Fuente {
    fn leer(&self, ruta: &str) -> Option<Vec<u8>>;
}

#[cfg(feature = "std")]
pub struct MemFuente {
    pub map: std::collections::HashMap<String, Vec<u8>>,
}

#[cfg(feature = "std")]
impl Fuente for MemFuente {
    fn leer(&self, ruta: &str) -> Option<Vec<u8>> {
        self.map.get(ruta).cloned()
    }
}
