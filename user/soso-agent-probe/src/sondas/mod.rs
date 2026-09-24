//! Una sonda por capacidad. Cada pase de T33 añade un módulo aquí.
//!
//! Orden de la ficha: primero archivos persistentes; después, en pases
//! separados, argv/env/cwd, stdout/stderr y exit status, tuberías con EOF,
//! hilos/join/TLS, temporizadores y TCP con cierre y reconexión.

pub mod archivos;
pub mod canales;
pub mod compartir;
pub mod ejecutable;
pub mod hilos;
pub mod salidas;
pub mod senales;
