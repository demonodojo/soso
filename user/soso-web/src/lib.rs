//! Motor de lectura y render de soso-web.

#![no_std]

extern crate alloc;

pub use soso_web_core::{decode, html};

pub mod gui;
// El decodificador vive en soso-web-core: lo comparte `fondo`.
pub use soso_web_core::png;
pub mod rutas;
