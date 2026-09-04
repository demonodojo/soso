//! Formato de release, manifest, pack y buzón de actualización de kernel.
//! Compartido entre xtask (std), userspace `soso-update` y boot-shim.

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

pub mod hash;
pub mod mailbox;
pub mod manifest;
pub mod pack;
pub mod semver;

pub use hash::{hex_sha256, sha256, Hash256};
pub use mailbox::{Mailbox, MailboxCmd};
pub use manifest::{FileEntry, Manifest, MANIFEST_MAGIC};
pub use pack::PackReader;
#[cfg(feature = "std")]
pub use pack::pack_rootfs;
#[cfg(feature = "std")]
pub use pack::PackWriter;
pub use semver::{cmp as semver_cmp, parse as parse_semver, SemVer};

/// Tamaño del buzón `SOSOUPD.TXT` en la ESP.
pub const UPD_MAILBOX_SIZE: usize = 4096;
/// Hueco `SOSOKRN.BIN` para el kernel en la ESP.
pub const UPD_KERNEL_SLOT_SIZE: usize = 64 * 1024 * 1024;

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
];
