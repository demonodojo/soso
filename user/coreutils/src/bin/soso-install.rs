//! Instalador nativo: clona el USB live a un NVMe y lo deja arrancable.
//!
//! Todo ocurre dentro de soso, sin volver a Linux. Tres pasos:
//!
//! 1. **Clonar** el disco de arranque sector a sector sobre el destino.
//! 2. **Reparar la GPT** del destino: el clon describe el pendrive (cabecera de
//!    respaldo a mitad de disco, partición de modelos del tamaño del USB y los
//!    mismos GUID que el original), así que se recoloca al tamaño real y se
//!    reparten GUID nuevos.
//! 3. **Pedir la entrada de arranque UEFI**: el kernel no puede llamar a
//!    `SetVariable` (no hay Runtime Services tras `ExitBootServices`), así que
//!    la petición se deja en `SOSOBOOT.TXT` de la ESP del USB y la ejecuta el
//!    shim en el siguiente arranque.
//!
//! El disco de Linux no se toca en ningún momento: el kernel rechaza escribir
//! en el disco de arranque y solo admite NVMe como destino, y aquí además se
//! rechaza cualquier disco con particiones de otro sistema. Sin destino,
//! lista los discos con su uso y pide al usuario cuál usar.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use gptdisk::{Guid, Header, Plan};
use libsoso::abi::{
    DISK_FLAG_BOOT, DISK_FLAG_EMPTY, DISK_FLAG_SOSO, DISK_KIND_NVME, DISK_KIND_USB, DiskInfo,
    O_RDONLY,
};
use libsoso::linea::Lector;
use libsoso::{print, println, sys};

libsoso::entry!(main);

/// Coincide con `raw_disk::MAX_XFER`: cada syscall es una transferencia al
/// dispositivo, ni troceada de más ni partida por el kernel.
const CHUNK: usize = 128 * 1024;
const SECTOR: usize = 512;

fn main(args: &str) -> u8 {
    let parts: Vec<&str> = args.split_whitespace().collect();
    if !parts.is_empty() && (parts[0] == "help" || parts[0] == "--help") {
        help();
        return 0;
    }
    if parts.first() == Some(&"list") {
        return cmd_list();
    }
    if parts.first() == Some(&"status") {
        return cmd_status();
    }

    let mut yes = false;
    let mut force = false;
    let mut target: Option<String> = None;
    for p in &parts {
        if *p == "--yes" || *p == "-y" {
            yes = true;
        } else if *p == "--force" {
            force = true;
        } else if p.starts_with('-') {
            println!("soso-install: argumento desconocido {p}");
            return 2;
        } else if target.is_none() {
            target = Some((*p).into());
        } else {
            println!("soso-install: argumento desconocido {p}");
            return 2;
        }
    }

    let disks = match cargar_discos() {
        Ok(d) => d,
        Err(c) => return c,
    };

    let dst = match target {
        Some(ref token) => match resolver_disco(token, &disks) {
            Some(id) => id,
            None => {
                println!("soso-install: disco '{token}' no encontrado");
                imprimir_discos(&disks);
                return 1;
            }
        },
        None => {
            imprimir_discos(&disks);
            if disks.is_empty() {
                println!("soso-install: no hay discos");
                return 1;
            }
            if yes {
                println!("soso-install: indica el disco destino (id o nombre)");
                println!("  ejemplo: soso-install nvme1 --yes");
                return 2;
            }
            println!("Elige en qué disco instalar soso.");
            match pedir_destino(&disks) {
                Some(id) => id,
                None => return 1,
            }
        }
    };

    cmd_install(dst, yes, force)
}

fn help() {
    println!(
        "soso-install — instalar soso en un disco NVMe (desde live USB)\n\
         \n\
         Uso:\n\
           soso-install                # lista discos y pide cuál\n\
           soso-install list           # discos, uso y particiones (sin instalar)\n\
           soso-install status         # estado de la entrada de arranque UEFI\n\
           soso-install <id|nombre> [--yes]\n\
           soso-install nvme1 [--yes]\n\
           soso-install <id|nombre> --force   # sobrescribir disco con otro SO\n\
         \n\
         El listado enseña para qué se usa cada disco (live, soso, Linux,\n\
         Windows, vacío) y si se puede elegir como destino. Nunca se toca el\n\
         pendrive de arranque; el destino tiene que ser NVMe.\n\
         \n\
         Al terminar, reinicia con el USB puesto: el shim UEFI registra la\n\
         entrada de arranque «soso». Después ya puedes quitar el USB.\n\
         \n\
         Plan B si tu firmware ignora entradas nuevas: desde Linux,\n\
         install-soso.sh --grub-only (partición SOSOINSTALL)."
    );
}

// ------------------------------------------------------------------ listado

fn cargar_discos() -> Result<Vec<DiskInfo>, u8> {
    let mut disks = [DiskInfo::default(); 8];
    let n = sys::disk_list(&mut disks);
    if n < 0 {
        println!("soso-install: disk_list errno {}", -n);
        return Err(1);
    }
    Ok(disks[..n as usize].to_vec())
}

fn cmd_list() -> u8 {
    match cargar_discos() {
        Ok(disks) => {
            imprimir_discos(&disks);
            println!("para instalar: soso-install  o  soso-install <id|nombre>");
            0
        }
        Err(c) => c,
    }
}

fn imprimir_discos(disks: &[DiskInfo]) {
    println!("id  nombre      tamaño  uso");
    if disks.is_empty() {
        println!("  (ningún disco)");
        return;
    }
    for d in disks {
        let name = disk_name(d);
        println!(
            "{:2}  {:8}  {:>10}  {}  [{}]",
            d.id,
            name,
            tamano_humano(d.sectors),
            uso_disco(d),
            nota_destino(d)
        );
        // Enseñar las particiones es la mitad del trabajo del instalador: es lo
        // que deja ver que un disco lleva un sistema ajeno antes de borrarlo.
        for line in partition_lines(d) {
            println!("      {line}");
        }
    }
    if !disks
        .iter()
        .any(|d| d.kind == DISK_KIND_NVME && d.flags & DISK_FLAG_BOOT == 0)
    {
        println!("no hay ningún NVMe donde instalar (soso solo escribe en NVMe)");
    }
}

fn leer_linea_tty() -> Option<String> {
    Lector::new().siguiente()
}

fn pedir_destino(disks: &[DiskInfo]) -> Option<u32> {
    print!("disco destino (id o nombre, q cancela): ");
    let typed: String = match leer_linea_tty() {
        Some(s) => s.trim().into(),
        None => {
            println!("cancelado");
            return None;
        }
    };
    if typed.is_empty() || typed == "q" || typed == "Q" {
        println!("cancelado");
        return None;
    }
    match resolver_disco(&typed, disks) {
        Some(id) => Some(id),
        None => {
            println!("soso-install: disco '{typed}' no encontrado");
            None
        }
    }
}

fn resolver_disco(token: &str, disks: &[DiskInfo]) -> Option<u32> {
    if let Ok(id) = token.parse::<u32>() {
        return disks.iter().find(|d| d.id == id).map(|d| d.id);
    }
    let lower = ascii_lower(token);
    disks
        .iter()
        .find(|d| ascii_lower(&disk_name(d)) == lower)
        .map(|d| d.id)
}

fn ascii_lower(s: &str) -> String {
    s.bytes().map(|b| b.to_ascii_lowercase() as char).collect()
}

fn tamano_humano(sectors: u64) -> String {
    let mib = sectors / 2048;
    if mib >= 1024 {
        let gib = mib / 1024;
        let dec = (mib % 1024) * 10 / 1024;
        if dec == 0 {
            alloc::format!("{gib} GiB")
        } else {
            alloc::format!("{gib}.{dec} GiB")
        }
    } else {
        alloc::format!("{mib} MiB")
    }
}

/// Una línea por partición: `p2  linux     64 MiB  sosofs`.
fn partition_lines(d: &DiskInfo) -> Vec<String> {
    let mut out = Vec::new();
    let Some((hdr, entries)) = read_gpt(d.id) else {
        return out;
    };
    for i in 0..hdr.num_entries as usize {
        let Some(e) = gptdisk::entry(&entries, &hdr, i) else {
            break;
        };
        if !gptdisk::entry_used(e) {
            continue;
        }
        let first = gptdisk::entry_first_lba(e);
        let last = gptdisk::entry_last_lba(e);
        let mib = last.saturating_sub(first).saturating_add(1) / 2048;
        let mut name = [0u8; 36];
        let n = gptdisk::entry_name_ascii(e, &mut name);
        let name = core::str::from_utf8(&name[..n]).unwrap_or("");
        out.push(alloc::format!(
            "p{}  {:<10} {:>7} MiB  {}",
            i + 1,
            gptdisk::type_label(&gptdisk::entry_type(e)),
            mib,
            name
        ));
    }
    out
}

fn disk_name(d: &DiskInfo) -> String {
    let end = d.name.iter().position(|&b| b == 0).unwrap_or(d.name.len());
    String::from_utf8_lossy(&d.name[..end]).into_owned()
}

/// Para qué se está usando el disco, según GPT y flags del kernel.
fn uso_disco(d: &DiskInfo) -> String {
    if d.flags & DISK_FLAG_BOOT != 0 {
        return "pendrive live (origen)".into();
    }
    if d.kind == DISK_KIND_USB {
        return "USB (solo lectura)".into();
    }
    if d.flags & DISK_FLAG_EMPTY != 0 {
        return "vacío".into();
    }
    if d.flags & DISK_FLAG_SOSO != 0 {
        return "soso instalado".into();
    }
    let tipos = tipos_particion(d);
    if tipos.is_empty() {
        return "sin GPT reconocible".into();
    }
    let win = tipos
        .iter()
        .any(|t| t.starts_with("windows") || *t == "ms-reserved");
    let linux = tipos.iter().any(|t| {
        matches!(
            *t,
            "linux" | "linux-root" | "linux-home" | "swap" | "lvm" | "raid"
        )
    });
    let lista = join_tipos(&tipos);
    if win && linux {
        alloc::format!("Linux y Windows ({lista})")
    } else if win {
        alloc::format!("Windows ({lista})")
    } else if linux {
        alloc::format!("Linux ({lista})")
    } else {
        alloc::format!("GPT ({lista})")
    }
}

fn tipos_particion(d: &DiskInfo) -> Vec<&'static str> {
    let mut out = Vec::new();
    let Some((hdr, entries)) = read_gpt(d.id) else {
        return out;
    };
    for i in 0..hdr.num_entries as usize {
        let Some(e) = gptdisk::entry(&entries, &hdr, i) else {
            break;
        };
        if !gptdisk::entry_used(e) {
            continue;
        }
        let label = gptdisk::type_label(&gptdisk::entry_type(e));
        if !out.iter().any(|x| *x == label) {
            out.push(label);
        }
    }
    out
}

fn join_tipos(tipos: &[&str]) -> String {
    let mut s = String::new();
    for (i, t) in tipos.iter().enumerate() {
        if i > 0 {
            s.push_str(", ");
        }
        s.push_str(t);
    }
    s
}

fn nota_destino(d: &DiskInfo) -> &'static str {
    if d.flags & DISK_FLAG_BOOT != 0 {
        return "no se toca";
    }
    if d.kind != DISK_KIND_NVME {
        return "no se puede instalar (solo NVMe)";
    }
    let ajenas = foreign_partitions(d);
    if !disk_safe(d) || !ajenas.is_empty() {
        return "ocupado — --force para borrar";
    }
    if d.flags & DISK_FLAG_SOSO != 0 {
        return "reinstalar";
    }
    "se puede instalar"
}

fn disk_safe(d: &DiskInfo) -> bool {
    d.flags & DISK_FLAG_SOSO != 0 || d.flags & DISK_FLAG_EMPTY != 0
}

/// Particiones que delatan otro sistema operativo (swap, LVM, Windows, raíces
/// Linux con GUID de tipo propio…). El live usa 0x8300 genérico, así que ese
/// tipo por sí solo no cuenta.
fn foreign_partitions(d: &DiskInfo) -> Vec<String> {
    let mut out = Vec::new();
    let Some((hdr, entries)) = read_gpt(d.id) else {
        return out;
    };
    for i in 0..hdr.num_entries as usize {
        let Some(e) = gptdisk::entry(&entries, &hdr, i) else {
            break;
        };
        if !gptdisk::entry_used(e) {
            continue;
        }
        let t = gptdisk::entry_type(e);
        if gptdisk::is_foreign(&t) {
            out.push(alloc::format!("p{} {}", i + 1, gptdisk::type_label(&t)));
        }
    }
    out
}

// ------------------------------------------------------------------ GPT

/// Lee cabecera primaria y array de entradas de un disco.
fn read_gpt(id: u32) -> Option<(Header, Vec<u8>)> {
    let mut sec = [0u8; SECTOR];
    if sys::disk_read(id, 1, &mut sec) < 0 {
        return None;
    }
    let hdr = Header::parse(&sec).ok()?;
    let bytes = hdr.entries_bytes();
    let padded = bytes.div_ceil(SECTOR) * SECTOR;
    let mut entries = vec![0u8; padded];
    if sys::disk_read(id, hdr.entries_lba, &mut entries) < 0 {
        return None;
    }
    entries.truncate(bytes);
    Some((hdr, entries))
}

/// Adapta la GPT recién clonada al disco destino y devuelve el GUID de su ESP,
/// que es lo que el shim necesita para localizarla en el siguiente arranque.
fn fix_gpt(id: u32, disk_sectors: u64, seed: u64) -> Result<Guid, &'static str> {
    let (mut hdr, mut entries) = read_gpt(id).ok_or("no pude leer la GPT clonada")?;
    let bytes = hdr.entries_bytes();
    entries.resize(bytes.div_ceil(SECTOR) * SECTOR, 0);

    let plan: Plan =
        gptdisk::relayout(&mut hdr, &mut entries, disk_sectors).map_err(|_| "relayout")?;
    let mut rng = gptdisk::Rng::new(seed);
    gptdisk::reseed_guids(&mut hdr, &mut entries, &mut rng);

    let esp = gptdisk::entry(&entries, &hdr, 0)
        .map(gptdisk::entry_unique)
        .ok_or("sin partición 1")?;

    // Respaldo primero y cabecera primaria al final: si se corta la corriente a
    // medias, el disco es un destino dedicado y se reinstala, pero al menos la
    // primaria vieja sigue describiendo algo coherente hasta el último paso.
    let backup = gptdisk::render_backup(&hdr, &entries, &plan);
    write_sectors(id, plan.backup_entries_lba, &entries)?;
    write_sectors(id, plan.backup_header_lba, &backup)?;

    let primary = gptdisk::render_primary(&hdr, &entries, &plan);
    write_sectors(id, plan.primary_entries_lba, &entries)?;

    let mut mbr = [0u8; SECTOR];
    if sys::disk_read(id, 0, &mut mbr) < 0 {
        return Err("no pude leer el MBR protector");
    }
    gptdisk::protective_mbr_fix(&mut mbr, disk_sectors);
    write_sectors(id, 0, &mbr)?;

    write_sectors(id, plan.primary_header_lba, &primary)?;
    Ok(esp)
}

fn write_sectors(id: u32, lba: u64, buf: &[u8]) -> Result<(), &'static str> {
    if sys::disk_write(id, lba, buf) < 0 {
        return Err("escritura de la GPT falló");
    }
    Ok(())
}

// ------------------------------------------------------------------ instalar

fn cmd_install(dst_id: u32, yes: bool, force: bool) -> u8 {
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
        imprimir_discos(list);
        return 1;
    };
    if dst.kind != DISK_KIND_NVME {
        println!(
            "soso-install: destino debe ser NVMe (id {} es {})",
            dst.id,
            disk_name(dst)
        );
        return 1;
    }
    if dst.id == src.id {
        println!("soso-install: destino es el disco de arranque");
        return 1;
    }

    let ajenas = foreign_partitions(dst);
    if !disk_safe(dst) || !ajenas.is_empty() {
        let name = disk_name(dst);
        if !ajenas.is_empty() {
            println!("soso-install: {name} tiene particiones de otro sistema:");
            for p in &ajenas {
                println!("    {p}");
            }
        } else {
            println!("soso-install: destino {name} tiene contenido ajeno a soso");
        }
        if !force {
            println!("  uso: {}", uso_disco(dst));
            println!("  para sobrescribirlo de todos modos: soso-install {name} --force");
            return 1;
        }
        print!("ATENCIÓN: sobrescribir {name}. Escribe \"{name}\" para confirmar: ");
        let typed: String = match leer_linea_tty() {
            Some(s) => s.trim().into(),
            None => {
                println!("cancelado");
                return 1;
            }
        };
        if typed != name {
            println!("cancelado (esperaba \"{name}\", recibí \"{typed}\")");
            return 1;
        }
    }

    let bytes = match read_live_bytes() {
        Some(b) => b,
        None => {
            println!("soso-install: falta /etc/soso-live.bytes");
            return 1;
        }
    };
    let sectors = bytes.div_ceil(512);
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
    println!("  destino: {} [{}]", uso_disco(dst), nota_destino(dst));

    if !yes {
        print!("¿Borrar destino e instalar? [y/N] ");
        let typed: String = match leer_linea_tty() {
            Some(s) => s.trim().into(),
            None => {
                println!("cancelado");
                return 1;
            }
        };
        let ans = typed.chars().next().unwrap_or('\0');
        if ans != 'y' && ans != 'Y' {
            println!("cancelado");
            return 1;
        }
    }

    if !clonar(src.id, dst.id, bytes) {
        return 1;
    }
    println!("soso-install: copia terminada");

    // Semilla de GUID: no hace falta calidad criptográfica, solo que el destino
    // no acabe con los mismos identificadores que el pendrive del que salió.
    let seed = (sys::uptime_ms() as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ dst.sectors
        ^ (dst.id as u64) << 32;
    let esp = match fix_gpt(dst.id, dst.sectors, seed) {
        Ok(g) => g,
        Err(e) => {
            println!("soso-install: no pude reparar la GPT del destino: {e}");
            println!("  el disco tiene los datos pero no arrancará; reinstala");
            return 1;
        }
    };
    println!(
        "soso-install: GPT ajustada al disco ({} MiB) — ESP {esp}",
        dst.sectors / 2048
    );

    println!();
    pedir_arranque(&esp, dst);
    0
}

fn clonar(src_id: u32, dst_id: u32, bytes: u64) -> bool {
    // En el heap, no en la pila: la pila de usuario son 64 KiB.
    let mut buf = vec![0u8; CHUNK];
    let mut lba = 0u64;
    let mut left = bytes as usize;
    let mut last_pct = u64::MAX;
    while left > 0 {
        let chunk = left.min(CHUNK) / SECTOR * SECTOR;
        if chunk == 0 {
            break;
        }
        if sys::disk_read(src_id, lba, &mut buf[..chunk]) < 0 {
            println!("soso-install: lectura falló en LBA {lba}");
            return false;
        }
        if sys::disk_write(dst_id, lba, &buf[..chunk]) < 0 {
            println!("soso-install: escritura falló en LBA {lba}");
            return false;
        }
        lba += (chunk / SECTOR) as u64;
        left -= chunk;
        let pct = (bytes - left as u64) * 100 / bytes;
        if pct != last_pct && pct % 5 == 0 {
            println!("  {pct}%");
            last_pct = pct;
        }
    }
    true
}

// ------------------------------------------------------------ arranque UEFI

const BOOTREQ_HEAD: &str = "SOSOBOOT v1";

/// Deja la petición en `SOSOBOOT.TXT` de la ESP del USB. Lo lee el shim en el
/// siguiente arranque, que sí está en contexto UEFI y puede tocar la NVRAM.
fn pedir_arranque(esp: &Guid, dst: &DiskInfo) {
    let payload = alloc::format!(
        "{BOOTREQ_HEAD}\nINSTALL {esp}\ndisco {} id {}\n",
        disk_name(dst),
        dst.id
    );
    let r = sys::bootreq_write(payload.as_bytes());
    if r < 0 {
        println!("=== Falta un paso: hacer arrancable el disco ===");
        println!(
            "No pude escribir SOSOBOOT.TXT en la ESP del USB (errno {}).\n\
             El disco está instalado, pero nadie ha registrado la entrada de\n\
             arranque. Opciones:\n\
           \x20 - elegir el disco directamente en el menú de arranque de la placa\n\
           \x20   (F12 / Boot menu): la ESP tiene EFI\\BOOT\\BOOTX64.EFI\n\
           \x20 - desde Linux: install-soso.sh --grub-only",
            -r
        );
        return;
    }
    println!("=== Último paso: reinicia con el USB puesto ===");
    println!(
        "Petición de arranque anotada en SOSOBOOT.TXT (ESP del USB).\n\
         \n\
         1) Reinicia SIN quitar el pendrive.\n\
         2) El shim UEFI registrará la entrada de arranque «soso» apuntando a\n\x20  \
            la ESP {esp} del disco {}.\n\
         3) Apaga, quita el USB y arranca: «soso» estará en el menú de la placa.\n\
         \n\
         Comprueba con: soso-install status",
        disk_name(dst)
    );
}

fn cmd_status() -> u8 {
    let mut buf = [0u8; 512];
    let n = sys::bootreq_read(&mut buf);
    if n < 0 {
        println!("soso-install: sin SOSOBOOT.TXT en la ESP (errno {})", -n);
        return 1;
    }
    let text = core::str::from_utf8(&buf[..n as usize]).unwrap_or("");
    let mut vacio = true;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        vacio = false;
        println!("{line}");
    }
    if vacio {
        println!("soso-install: sin petición de arranque pendiente");
    }
    0
}

// ------------------------------------------------------------------ varios

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
