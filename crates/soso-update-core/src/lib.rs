//! Formato de release, manifest, pack y buzón de actualización de kernel.
//! Compartido entre xtask (std), userspace `soso-update` y boot-shim.

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

pub mod hash;
pub mod kernel_apply;
pub mod kernel_meta;
pub mod mailbox;
pub mod manifest;
pub mod pack;
pub mod plan;
pub mod semver;

pub use hash::{hex_sha256, sha256, Hash256, Hasher};
pub use kernel_apply::{
    after_interrupt, plan_apply, restore_from_slot, verify_staged_kernel, AfterInterrupt,
    ApplyError, ApplyPlan,
};
pub use kernel_meta::{backup_digest, KernelMeta, KernelPhase, MetaError, KERNEL_META_MAGIC, KERNEL_META_SIZE};
pub use mailbox::{Mailbox, MailboxCmd};
pub use manifest::{FileEntry, Manifest, MANIFEST_MAGIC, ValidateError};
pub use pack::PackReader;
pub use plan::{plan_bytes, plan_spans, Span, GAP_MAX, SPAN_MAX};
#[cfg(feature = "std")]
pub use pack::{pack_rootfs, pack_rootfs_con};
#[cfg(feature = "std")]
pub use pack::PackWriter;
pub use semver::{cmp as semver_cmp, parse as parse_semver, SemVer};

/// Tamaño del buzón `SOSOUPD.TXT` en la ESP.
pub const UPD_MAILBOX_SIZE: usize = 4096;
/// Hueco `SOSOKRN.BIN` para el kernel en la ESP.
pub const UPD_KERNEL_SLOT_SIZE: usize = 64 * 1024 * 1024;
/// Metadatos `SOSOKRN.MET` (backup durable del kernel).
pub const UPD_KERNEL_META_SIZE: usize = kernel_meta::KERNEL_META_SIZE;

/// Directorios cuyo contenido nunca va en un pack de release. `var/actualiza-prueba`
/// es el release que fabrica `cargo xtask test-update`: empaquetarlo metería una
/// copia del pack dentro del pack siguiente, y el rootfs crecería al doble en
/// cada pasada.
pub const PACK_SKIP_DIRS: &[&str] = &[
    "var/actualiza-prueba/",
    "var/forja-cache/",
    "var/forja-out/",
    "src/soso/",
    // `/models` es el volumen sosomfs, de sólo lectura: `create_file` devuelve
    // EIO ahí sin más. Los modelos no viajan en el pack del OS, vienen en su
    // propio volumen, así que incluirlos sólo servía para abortar la
    // actualización en la primera entrada que tocara escribir.
    "models/",
];

/// Canal de actualización para toolchain de desarrollo (Hito 3f).
pub const UPD_CHANNEL_DEV: &str = "dev";
/// Canal estable por defecto.
pub const UPD_CHANNEL_STABLE: &str = "stable";

/// Ficheros de rootfs que no van en el pack de release (config local / metadatos).
pub const PACK_SKIP: &[&str] = &[
    "etc/ssh_host_key",
    "etc/authorized_key",
    "etc/wifi.conf",
    "etc/llm.conf",
    "etc/voz.conf",
    "etc/actualiza.conf",
    "etc/soso-live.bytes",
    "etc/grub-linux.txt",
    "etc/soso-release",
    "etc/actualiza.estado",
    "etc/soso-hw",
];
