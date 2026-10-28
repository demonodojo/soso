//! Formato de paquetes de aplicación (`SOSOPKG v1`), índice remoto y reglas
//! puras (sin E/S). Compartido entre `soso-pkg` (guest) y tests host.

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

pub mod comandos;
pub mod conf;
pub mod fuente;
pub mod indice;
pub mod manifest;
pub mod oferta;
pub mod pack;
pub mod rutas;
pub mod urls;

pub use comandos::{entrada_comando, EntradaComando, IndiceComandos};
pub use conf::Conf;
pub use fuente::Fuente;
#[cfg(feature = "std")]
pub use fuente::MemFuente;
pub use indice::{EntradaIndice, Indice, ParseIndiceError};
pub use manifest::{FileEntry, Manifest, ParseManifestError, ValidateError, MANIFEST_MAGIC};
pub use oferta::{decidir, parse_version, AccionInstalacion};
pub use pack::PackReader;
pub use rutas::{nombre_valido, ruta_valida, RutaError};
pub use urls::{artefacto_remoto, indice_url};

/// Tope del `.pack` (64 MiB).
pub const MAX_PACK_SIZE: u64 = 64 * 1024 * 1024;

pub const DIR_INSTALADOS: &str = "var/paquetes";
pub const FICHERO_COMANDOS: &str = "var/paquetes/comandos";
pub const CONF_RUTA: &str = "etc/paquetes.conf";
