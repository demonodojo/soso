//! Lógica de automejora: captura de base, banco de casos y verificadores.
//!
//! El objetivo declarado del plan es que **todo esto pueda correr dentro de
//! soso**, no solo en el host. Por eso el crate es `no_std + alloc` y no habla
//! nunca con el sistema directamente: leer un árbol o lanzar un proceso entran
//! por los traits de [`entorno`], que el host implementa con `std` y el guest
//! implementará con `libsoso` (`spawn_io` + `wait`, que ya existen).
//!
//! Es el mismo patrón que usa `soso-http` con `TcpTransport`: la política vive
//! en el crate portable y el mecanismo lo pone quien lo hospeda.
//!
//! Módulos:
//!
//! - [`entorno`] — traits de archivos y procesos, y sus tipos.
//! - [`captura`] — base reproducible: inventario, almacén por contenido, sonda
//!   de estabilidad y reconstrucción. **No usa git para reconstruir**: guarda
//!   los contenidos, así que no hace falta `clone` ni `apply` (soso no los
//!   tiene).
//! - [`ignorar`] — subconjunto de `.gitignore`, para saber qué no es fuente
//!   sin llamar a git.
//! - [`caso`] — casos del banco, partición reproducible y sellado.
//! - [`protocolo`] — aserciones sobre una respuesta HTTP/SSE grabada.
//! - [`programa`] — comparación de un candidato contra vectores.
//! - [`referencia`] — comparación de fixtures del modelo original contra el
//!   tokenizer de soso (T03).
//! - [`repo`] — verificación de un caso de repo sobre un árbol.

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

use alloc::string::{String, ToString};

pub mod captura;
pub mod caso;
pub mod entorno;
pub mod ignorar;
pub mod programa;
pub mod referencia;
pub mod protocolo;
pub mod repo;

/// Versión del formato de manifiestos y casos (C5: `schema_version`).
pub const ESQUEMA: u32 = 1;

/// Error único del crate: en `no_std` no hay `std::error::Error` que valga.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// Uso incorrecto: ruta fuera de sitio, destino ocupado, caso desconocido.
    Uso(String),
    /// El entorno falló: no se pudo leer, escribir o ejecutar.
    Entorno(String),
    /// Un JSON no se pudo interpretar o no cumple el esquema.
    Formato(String),
    /// El checkout cambió mientras se capturaba.
    Inestable(String),
}

impl Error {
    pub fn uso(m: impl ToString) -> Self {
        Error::Uso(m.to_string())
    }
    pub fn entorno(m: impl ToString) -> Self {
        Error::Entorno(m.to_string())
    }
    pub fn formato(m: impl ToString) -> Self {
        Error::Formato(m.to_string())
    }
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Error::Uso(m) => write!(f, "uso: {m}"),
            Error::Entorno(m) => write!(f, "entorno: {m}"),
            Error::Formato(m) => write!(f, "formato: {m}"),
            Error::Inestable(m) => write!(f, "captura inestable: {m}"),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for Error {}

pub type Resultado<T> = core::result::Result<T, Error>;

/// SHA-256 en hexadecimal minúsculo.
pub fn sha256_hex(datos: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let resumen = Sha256::digest(datos);
    let mut salida = String::with_capacity(64);
    for b in resumen.iter() {
        salida.push(char::from_digit((b >> 4) as u32, 16).unwrap());
        salida.push(char::from_digit((b & 0xf) as u32, 16).unwrap());
    }
    salida
}

/// Rechaza rutas absolutas, con `..` o vacías.
///
/// Todo lo que se escribe desde una captura o un caso pasa por aquí: una ruta
/// con `..` en un manifiesto es una escritura fuera del destino.
pub fn ruta_segura(ruta: &str) -> Resultado<()> {
    if ruta.is_empty() {
        return Err(Error::uso("ruta vacía"));
    }
    if ruta.starts_with('/') || ruta.starts_with('\\') {
        return Err(Error::uso(alloc::format!("ruta absoluta: {ruta}")));
    }
    if ruta.split('/').any(|s| s == "..") {
        return Err(Error::uso(alloc::format!("ruta con '..': {ruta}")));
    }
    Ok(())
}

/// Une dos trozos de ruta con `/`, sin duplicar separadores.
pub fn unir(base: &str, resto: &str) -> String {
    if base.is_empty() {
        return resto.to_string();
    }
    let base = base.trim_end_matches('/');
    alloc::format!("{base}/{}", resto.trim_start_matches('/'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_conocido() {
        // Vector de SHA-256 de toda la vida.
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn rutas_rechazadas() {
        assert!(ruta_segura("a/b.txt").is_ok());
        assert!(ruta_segura("a..b/c").is_ok(), "'a..b' es un nombre normal");
        assert!(ruta_segura("/etc/passwd").is_err());
        assert!(ruta_segura("../fuera").is_err());
        assert!(ruta_segura("a/../../fuera").is_err());
        assert!(ruta_segura("").is_err());
    }

    #[test]
    fn union_de_rutas() {
        assert_eq!(unir("a", "b"), "a/b");
        assert_eq!(unir("a/", "/b"), "a/b");
        assert_eq!(unir("", "b"), "b");
    }
}
