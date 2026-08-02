//! Instalador dual-boot desde soso live: clona el USB de arranque a un NVMe.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;
use libsoso::abi::{
    DiskInfo, DISK_FLAG_BOOT, DISK_KIND_NVME, DISK_KIND_USB, O_RDONLY,
};
use libsoso::{print, println, sys};

libsoso::entry!(main);

const CHUNK: usize = 128 * 1024;

fn main(args: &str) -> u8 {
    let parts: Vec<&str> = args.split_whitespace().collect();
    if parts.is_empty() || parts[0] == "help" || parts[0] == "--help" {
        help();
        return 0;
    }
    if parts[0] == "list" {
        return cmd_list();
    }

    let mut yes = false;
    let mut target_id: Option<u32> = None;
    for p in &parts {
        if *p == "--yes" || *p == "-y" {
            yes = true;
        } else if let Ok(n) = p.parse::<u32>() {
            target_id = Some(n);
        } else if let Some(name) = p.strip_prefix("nvme") {
            if let Ok(slot) = name.parse::<u32>() {
                target_id = Some(if slot == 0 { 2 } else { 3 });
            }
        } else {
            println!("soso-install: argumento desconocido {p}");
            return 2;
        }
    }

    let Some(dst) = target_id else {
        println!("soso-install: falta disco destino (id o nvmeN)");
        help();
        return 2;
    };

    cmd_install(dst, yes)
}

fn help() {
    println!(
        "soso-install — instalar soso en un disco NVMe (desde live USB)\n\
         \n\
         Uso:\n\
           soso-install list\n\
           soso-install <id> [--yes]\n\
           soso-install nvme1 [--yes]\n\
         \n\
         Tras instalar, reinicia en Linux y ejecuta install-soso.sh --grub-only\n\
         (partición SOSO_INSTALL) o cargo xtask install-disk --no-grub + update-grub."
    );
}

fn cmd_list() -> u8 {
    let mut disks = [DiskInfo::default(); 8];
    let n = sys::disk_list(&mut disks);
    if n < 0 {
        println!("soso-install: disk_list errno {}", -n);
        return 1;
    }
    println!("id  nombre   sectores    flags");
    for d in &disks[..n as usize] {
        let name = disk_name(d);
        let boot = if d.flags & DISK_FLAG_BOOT != 0 { " boot" } else { "" };
        let ro = if d.kind == DISK_KIND_USB { " ro" } else { "" };
        println!(
            "{:2}  {:8}  {:10}  {}{}",
            d.id,
            name,
            d.sectors,
            ro,
            boot
        );
    }
    0
}

fn disk_name(d: &DiskInfo) -> String {
    let end = d.name.iter().position(|&b| b == 0).unwrap_or(d.name.len());
    String::from_utf8_lossy(&d.name[..end]).into_owned()
}

fn cmd_install(dst_id: u32, yes: bool) -> u8 {
    let mut disks = [DiskInfo::default(); 8];
    let n = sys::disk_list(&mut disks);
    if n < 0 {
        println!("soso-install: disk_list errno {}", -n);
        return 1;
    }
    let list = &disks[..n as usize];
    let Some(src) = list.iter().find(|d| d.flags & DISK_FLAG_BOOT != 0) else {
        println!("soso-install: no hay disco de arranque live (¿arrancaste desde USB?)");
        return 1;
    };
    let Some(dst) = list.iter().find(|d| d.id == dst_id) else {
        println!("soso-install: disco destino id {dst_id} no encontrado");
        let _ = cmd_list();
        return 1;
    };
    if dst.kind != DISK_KIND_NVME {
        println!("soso-install: destino debe ser NVMe (id {} es {})", dst.id, disk_name(dst));
        return 1;
    }
    if dst.id == src.id {
        println!("soso-install: destino es el disco de arranque");
        return 1;
    }

    let bytes = match read_live_bytes() {
        Some(b) => b,
        None => {
            println!("soso-install: falta /etc/soso-live.bytes");
            return 1;
        }
    };
    let sectors = (bytes + 511) / 512;
    if dst.sectors < sectors {
        println!(
            "soso-install: destino pequeño ({} sectores, hacen falta {})",
            dst.sectors, sectors
        );
        return 1;
    }

    println!(
        "soso-install: {} (id {}) → {} (id {}), {} MiB",
        disk_name(src),
        src.id,
        disk_name(dst),
        dst.id,
        bytes / (1024 * 1024)
    );

    if !yes {
        print!("¿Borrar destino e instalar? [y/N] ");
        let mut line = [0u8; 16];
        let nr = sys::read(0, &mut line);
        if nr <= 0 {
            println!("cancelado");
            return 1;
        }
        let ans = line[0] as char;
        if ans != 'y' && ans != 'Y' {
            println!("cancelado");
            return 1;
        }
    }

    let mut buf = [0u8; CHUNK];
    let mut lba = 0u64;
    let mut left = bytes as usize;
    while left > 0 {
        let chunk = left.min(CHUNK);
        let chunk = chunk / 512 * 512;
        if chunk == 0 {
            break;
        }
        if sys::disk_read(src.id, lba, &mut buf[..chunk]) < 0 {
            println!("soso-install: lectura falló en LBA {lba}");
            return 1;
        }
        if sys::disk_write(dst.id, lba, &buf[..chunk]) < 0 {
            println!("soso-install: escritura falló en LBA {lba}");
            return 1;
        }
        lba += (chunk / 512) as u64;
        left -= chunk;
        if lba % 2048 == 0 {
            let pct = (bytes as u64 - left as u64) * 100 / bytes;
            println!("  {pct}%");
        }
    }
    println!("soso-install: copia terminada");
    println!();
    print_grub_instructions(dst);
    0
}

fn read_live_bytes() -> Option<u64> {
    let fd = sys::open("/etc/soso-live.bytes", O_RDONLY);
    if fd < 0 {
        return None;
    }
    let mut buf = [0u8; 32];
    let n = sys::read(fd as u64, &mut buf);
    let _ = sys::close(fd as u64);
    if n <= 0 {
        return None;
    }
    let s = core::str::from_utf8(&buf[..n as usize]).ok()?;
    s.trim().parse().ok()
}

fn print_grub_instructions(dst: &DiskInfo) {
    println!("=== Siguiente paso (desde Linux) ===");
    println!(
        "Reinicia en Linux. Luego, con el USB conectado:\n\
         \n\
           sudo /media/$USER/SOSO_INSTALL/install-soso.sh --grub-only\n\
         \n\
         (detecta la ESP de soso en el disco destino y añade entrada GRUB.)\n\
         \n\
         Disco destino: {} (id {}, ESP suele ser {}p1)",
        disk_name(dst),
        dst.id,
        disk_name(dst)
    );
    if let Ok(text) = read_file("/etc/grub-linux.txt") {
        println!();
        print!("{text}");
    }
}

fn read_file(path: &str) -> Result<String, i64> {
    let fd = sys::open(path, O_RDONLY);
    if fd < 0 {
        return Err(fd);
    }
    let mut out = String::new();
    let mut buf = [0u8; 512];
    loop {
        let n = sys::read(fd as u64, &mut buf);
        if n < 0 {
            let _ = sys::close(fd as u64);
            return Err(n);
        }
        if n == 0 {
            break;
        }
        out.push_str(core::str::from_utf8(&buf[..n as usize]).unwrap_or(""));
    }
    let _ = sys::close(fd as u64);
    Ok(out)
}
