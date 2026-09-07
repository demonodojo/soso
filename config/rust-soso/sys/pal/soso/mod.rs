//! Platform abstraction layer for `x86_64-unknown-soso` (Hito 2/3).
//!
//! Copiar este árbol a `library/std/src/sys/pal/soso/` en el fork de
//! `rust-lang/rust` junto con el target JSON del repositorio.

pub mod dl;

pub fn init() {}
