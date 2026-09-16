//! Lógica de los logs nativos de soso (entrega U1 de `docs/PLAN-ACTUALIZACIONES.md`).
//!
//! Aquí no hay E/S: el kernel aporta el ring y las llamadas al VFS, y el banco
//! host prueba estas reglas —cursor, pérdidas, rotación y control de fallos—
//! que dentro del kernel no tienen forma de comprobarse.

#![cfg_attr(not(feature = "std"), no_std)]

pub mod flujo;
pub mod ring;
pub mod rotacion;
pub mod texto;

pub use flujo::{Estado, Flujo, Trabajo, LOTE_MAX, MAX_FALLOS, REINTENTO_MS};
pub use ring::{Lectura, Ring};
pub use rotacion::{Accion, Paso, MAX_ACTIVO, ROTACIONES, TECHO_POR_FLUJO};
pub use texto::{cabecera, marca_fallo, marca_perdida, Cabecera, EscritorSlice};

/// Directorio de los logs nativos dentro de sosofs.
pub const DIR_LOG: &str = "/var/log";

/// Nombre de cada flujo. Es también el nombre del fichero activo.
pub const FLUJO_KERNEL: &str = "kernel.log";
pub const FLUJO_APPS: &str = "aplicaciones.log";
pub const FLUJO_OTA: &str = "actualizaciones.log";
