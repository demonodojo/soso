//! Formato de release, manifest, pack y buzón de actualización de kernel.
//! Compartido entre xtask (std), userspace `soso-update` y boot-shim.

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

// Siempre disponibles: es lo que necesita el kernel para resolver la identidad
// del arranque antes de montar sosofs.
pub mod hash;
pub mod identity;
pub mod record;

#[cfg(feature = "full")]
pub mod canal;
#[cfg(feature = "full")]
pub mod compat;
#[cfg(feature = "full")]
pub mod descarga;
#[cfg(feature = "full")]
pub mod kernel_apply;
#[cfg(feature = "full")]
pub mod kernel_meta;
#[cfg(feature = "full")]
pub mod mailbox;
#[cfg(feature = "full")]
pub mod manifest;
#[cfg(feature = "full")]
pub mod pack;
#[cfg(feature = "full")]
pub mod plan;
#[cfg(feature = "full")]
pub mod semver;
#[cfg(feature = "txn")]
pub mod txn;

pub use hash::{hex_of, hex_sha256, sha256, Hash256, Hasher};
pub use identity::{resolver as resolver_identidad, BootMode, Identidad, ModeRecord};
pub use record::{RecordError, SLOTS, SLOT_SIZE};

/// Identidad live/instalado (`SOSOMODE.TXT`, U0).
pub const UPD_MODE_SIZE: usize = identity::MODE_SIZE;

#[cfg(feature = "full")]
pub use canal::{Conf, Origen};
#[cfg(feature = "full")]
pub use compat::{Compat, CompatError, Equipo};
#[cfg(feature = "full")]
pub use descarga::{Etapa, TROZO_MAX};
#[cfg(feature = "full")]
pub use kernel_apply::{
    after_interrupt, plan_apply, restore_from_slot, verify_staged_kernel, AfterInterrupt,
    ApplyError, ApplyPlan,
};
#[cfg(feature = "full")]
pub use kernel_meta::{backup_digest, KernelMeta, KernelPhase, MetaError, KERNEL_META_MAGIC, KERNEL_META_SIZE};
#[cfg(feature = "full")]
pub use mailbox::{Mailbox, MailboxCmd};
#[cfg(feature = "full")]
pub use manifest::{FileEntry, Manifest, MANIFEST_MAGIC, ValidateError};
#[cfg(feature = "full")]
pub use pack::PackReader;
#[cfg(feature = "full")]
pub use plan::{plan_bytes, plan_spans, Span, GAP_MAX, SPAN_MAX};
#[cfg(feature = "std")]
pub use pack::{pack_rootfs, pack_rootfs_con};
#[cfg(feature = "std")]
pub use pack::PackWriter;
#[cfg(feature = "full")]
pub use semver::{cmp as semver_cmp, parse as parse_semver, SemVer};
#[cfg(feature = "txn")]
pub use txn::bootrec::{BootRecord, Decision};
#[cfg(feature = "txn")]
pub use txn::journal::{Accion, Entrada, Journal, Progreso};
#[cfg(feature = "txn")]
pub use txn::reconcile::{reconcile, EstadoEsp, EstadoJournal, Motivo, Recuperacion};
#[cfg(feature = "txn")]
pub use txn::{preflight, Capacidad, Necesidad, TxnEvent, TxnId, TxnState};

/// Registro de arranque de la transacción (`SOSOTXN.BIN`, U0).
#[cfg(feature = "txn")]
pub const UPD_BOOTREC_SIZE: usize = txn::bootrec::BOOTREC_SIZE;

/// Tamaño del buzón `SOSOUPD.TXT` en la ESP.
pub const UPD_MAILBOX_SIZE: usize = 4096;
/// Hueco `SOSOKRN.BIN` para el kernel en la ESP.
pub const UPD_KERNEL_SLOT_SIZE: usize = 64 * 1024 * 1024;
/// Metadatos `SOSOKRN.MET` (backup durable del kernel).
#[cfg(feature = "full")]
pub const UPD_KERNEL_META_SIZE: usize = kernel_meta::KERNEL_META_SIZE;

/// Por qué una ruta del rootfs no viaja en una release.
///
/// Está enumerado, y no sólo listado, porque el criterio de cierre de U3 es que
/// **ninguna release contenga logs, claves, cachés ni estado de una OTA**: sin
/// nombrar el motivo, una lista de rutas envejece en silencio cada vez que
/// alguien añade un directorio nuevo.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Excluido {
    /// Registros de la máquina que empaqueta (U1: `/var/log`).
    Log,
    /// Estado de una actualización en vuelo: diario, área de preparación y
    /// respaldos. Copiarlo haría que el destino heredara una operación ajena.
    EstadoOta,
    /// Cachés y artefactos regenerables.
    Cache,
    /// Temporales de ejecución.
    Temporal,
    /// Claves y configuración local: se preservan, no se sobrescriben.
    ConfigLocal,
    /// Otro volumen (sosomfs), que no se escribe desde el pack.
    Volumen,
    /// Árbol de fuentes embebido, que se regenera.
    Fuente,
}

/// Directorios cuyo contenido nunca va en un pack, con su motivo.
pub const PACK_SKIP_DIRS: &[(&str, Excluido)] = &[
    ("var/log/", Excluido::Log),
    ("var/lib/soso-update/", Excluido::EstadoOta),
    ("var/actualiza-prueba/", Excluido::EstadoOta),
    ("var/forja-cache/", Excluido::Cache),
    ("var/forja-out/", Excluido::Cache),
    ("var/cache/", Excluido::Cache),
    ("var/self-improvement/", Excluido::Cache),
    ("tmp/", Excluido::Temporal),
    ("src/soso/", Excluido::Fuente),
    // `/models` es el volumen sosomfs, de sólo lectura: `create_file` devuelve
    // EIO ahí sin más. Los modelos no viajan en el pack del OS, vienen en su
    // propio volumen, así que incluirlos sólo servía para abortar la
    // actualización en la primera entrada que tocara escribir.
    ("models/", Excluido::Volumen),
];

/// Ficheros sueltos que no van en el pack: claves y configuración de la máquina.
pub const PACK_SKIP: &[(&str, Excluido)] = &[
    ("etc/ssh_host_key", Excluido::ConfigLocal),
    ("etc/authorized_key", Excluido::ConfigLocal),
    ("etc/wifi.conf", Excluido::ConfigLocal),
    ("etc/llm.conf", Excluido::ConfigLocal),
    ("etc/voz.conf", Excluido::ConfigLocal),
    ("etc/actualiza.conf", Excluido::ConfigLocal),
    ("etc/soso-live.bytes", Excluido::ConfigLocal),
    ("etc/grub-linux.txt", Excluido::ConfigLocal),
    ("etc/soso-release", Excluido::ConfigLocal),
    ("etc/actualiza.estado", Excluido::EstadoOta),
    ("etc/soso-hw", Excluido::ConfigLocal),
];

/// Sufijos que no viajan nunca, esté donde esté el fichero.
const PACK_SKIP_SUFIJOS: &[(&str, Excluido)] = &[
    (".log", Excluido::Log),
    ("_key", Excluido::ConfigLocal),
    (".key", Excluido::ConfigLocal),
];

/// Motivo por el que `rel` (ruta relativa a la raíz, sin `/` inicial) se
/// excluye del pack, o `None` si sí viaja.
pub fn por_que_se_excluye(rel: &str) -> Option<Excluido> {
    let rel = rel.trim_start_matches('/');
    if let Some((_, m)) = PACK_SKIP.iter().find(|(p, _)| *p == rel) {
        return Some(*m);
    }
    if let Some((_, m)) = PACK_SKIP_DIRS.iter().find(|(d, _)| rel.starts_with(d)) {
        return Some(*m);
    }
    if let Some((_, m)) = PACK_SKIP_SUFIJOS.iter().find(|(s, _)| rel.ends_with(s)) {
        return Some(*m);
    }
    None
}

#[cfg(feature = "full")]
pub const UPD_CHANNEL_DEV: &str = canal::CANAL_DEV;
/// Canal estable por defecto.
#[cfg(feature = "full")]
pub const UPD_CHANNEL_STABLE: &str = canal::CANAL_STABLE;

