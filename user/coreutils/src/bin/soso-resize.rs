//! Redimensionar sosofs robando espacio al final de la partición de modelos.
//!
//! ```text
//! soso-resize              # estado (equivalente a «estado»)
//! soso-resize estado
//! soso-resize rootfs +512M
//! soso-resize rootfs +8G
//! ```

#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec::Vec;
use libsoso::abi::{FsSpaceInfo, FS_RESIZE_GROW_ROOT, FS_RESIZE_QUERY};
use libsoso::{print, println, sys};

libsoso::entry!(main);

const BLOCK_SIZE: u64 = 4096;

fn main(args: &str) -> u8 {
    let parts: Vec<&str> = args.split_whitespace().collect();
    if parts.first() == Some(&"help") || parts.first() == Some(&"--help") {
        help();
        return 0;
    }
    if parts.is_empty() || parts[0] == "estado" {
        return cmd_estado();
    }
    if parts.len() >= 2 && parts[0] == "rootfs" {
        let delta = match parse_delta(parts[1]) {
            Some(b) => b,
            None => {
                println!("soso-resize: tamaño inválido «{}»", parts[1]);
                return 2;
            }
        };
        return cmd_grow(delta);
    }
    println!("soso-resize: uso: soso-resize [estado] | rootfs +<tamaño>");
    2
}

fn help() {
    println!(
        "soso-resize — ampliar sosofs desde el espacio libre de modelos\n\
         \n\
         Uso:\n\
           soso-resize              estado de particiones y sistemas de ficheros\n\
           soso-resize rootfs +512M ampliar rootfs (live USB o instalación GPT)\n\
         \n\
         El espacio sobrante del pendrive o del NVMe va a modelos al instalar;\n\
         este comando mueve parte de ese margen libre de modelos a sosofs.\n\
         En QEMU sin GPT el rootfs ya crece solo al arrancar (SOSO_ROOTFS_SIZE)."
    );
}

fn cmd_estado() -> u8 {
    let mut info = FsSpaceInfo::default();
    let n = sys::fs_resize(FS_RESIZE_QUERY, 0, &mut info);
    if n < 0 {
        if n == -38 {
            println!("soso-resize: no disponible (sin disco GPT live/instalado)");
            println!("  en QEMU el rootfs crece solo al montar si la imagen es mayor");
        } else {
            println!("soso-resize: consulta falló (errno {})", -n);
        }
        return 1;
    }
    imprimir_estado(&info);
    0
}

fn imprimir_estado(info: &FsSpaceInfo) {
    println!("sosofs (rootfs):");
    println!(
        "  volumen: {} MiB  libres: {} MiB  partición: {} MiB",
        blocks_to_mib(info.root_fs_blocks),
        blocks_to_mib(info.root_free_blocks),
        blocks_to_mib(info.root_part_blocks),
    );
    println!("sosomfs (modelos):");
    println!(
        "  volumen: {} MiB  usados: {} MiB  partición: {} MiB",
        blocks_to_mib(info.models_fs_blocks),
        blocks_to_mib(info.models_used_blocks),
        blocks_to_mib(info.models_part_blocks),
    );
    println!(
        "  máximo ampliable rootfs: {} MiB ({} bloques)",
        blocks_to_mib(info.max_grow_blocks),
        info.max_grow_blocks
    );
}

fn cmd_grow(delta_blocks: u64) -> u8 {
    let mut info = FsSpaceInfo::default();
    if sys::fs_resize(FS_RESIZE_QUERY, 0, &mut info) < 0 {
        println!("soso-resize: no disponible en este medio de arranque");
        return 1;
    }
    if delta_blocks > info.max_grow_blocks {
        println!(
            "soso-resize: sólo hay {} MiB libres al final de modelos (pediste {})",
            blocks_to_mib(info.max_grow_blocks),
            blocks_to_mib(delta_blocks),
        );
        println!("  borra modelos o pide menos espacio");
        return 1;
    }
    print!(
        "soso-resize: rootfs +{} MiB desde modelos… ",
        blocks_to_mib(delta_blocks)
    );
    let mut dummy = FsSpaceInfo::default();
    let r = sys::fs_resize(FS_RESIZE_GROW_ROOT, delta_blocks, &mut dummy);
    if r < 0 {
        println!("error {}", -r);
        return 1;
    }
    println!("hecho");
    let mut after = FsSpaceInfo::default();
    if sys::fs_resize(FS_RESIZE_QUERY, 0, &mut after) >= 0 {
        imprimir_estado(&after);
    }
    0
}

fn blocks_to_mib(blocks: u64) -> u64 {
    blocks * BLOCK_SIZE / (1024 * 1024)
}

fn parse_delta(raw: &str) -> Option<u64> {
    let s = raw.trim();
    if s.starts_with('-') {
        return None;
    }
    let s = s.strip_prefix('+').unwrap_or(s);
    let mut num_end = 0;
    for (i, c) in s.char_indices() {
        if c.is_ascii_digit() {
            num_end = i + c.len_utf8();
        } else {
            break;
        }
    }
    if num_end == 0 {
        return None;
    }
    let n: u64 = s[..num_end].parse().ok()?;
    let unit = s[num_end..].trim().to_ascii_uppercase();
    let mul: u64 = match unit.as_str() {
        "" | "B" => 1,
        "K" | "KB" | "KIB" => 1024,
        "M" | "MB" | "MIB" => 1024 * 1024,
        "G" | "GB" | "GIB" => 1024 * 1024 * 1024,
        _ => return None,
    };
    let bytes = n.checked_mul(mul)?;
    Some(bytes.div_ceil(BLOCK_SIZE))
}
