//! Empaqueta un kernel ELF en imágenes BIOS y UEFI con `bootloader::DiskImageBuilder`
//! (T42, C-118): `imagen <kernel-elf> <bios.img> <uefi.img>`.
//!
//! Es lo que hace `xtask build_image` sin el shim ni los huecos de actualización;
//! con las mismas etapas del cargador (`blobs/`) debe dar los mismos bytes en el
//! host y en el guest.

use std::path::PathBuf;
use std::process::exit;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    if a.len() != 4 {
        eprintln!("uso: imagen <kernel-elf> <bios.img> <uefi.img>");
        exit(2);
    }
    let builder = bootloader::DiskImageBuilder::new(PathBuf::from(&a[1]));
    builder.create_bios_image(&PathBuf::from(&a[2])).expect("imagen BIOS");
    println!("imagen BIOS: {}", a[2]);
    builder.create_uefi_image(&PathBuf::from(&a[3])).expect("imagen UEFI");
    println!("imagen UEFI: {}", a[3]);
}
