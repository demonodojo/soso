//! Escritura mínima en FAT16/FAT32 (sin mtools): un fichero contiguo en el directorio raíz.

use std::fs::OpenOptions;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

const SECTOR: u64 = 512;
/// Tamaño de I/O para huecos grandes (kernel / SOSOKRN): nunca un Vec del hueco entero.
const IO_CHUNK: usize = 256 * 1024;

struct Vol {
    base: u64,
    bps: u64,
    spc: u64,
    reserved: u64,
    num_fats: u64,
    spf: u64,
    fat16: bool,
    root_lba: u64,
    root_sectors: u64,
    data_start: u64,
}

pub fn find_root_entry(
    img: &Path,
    part_first_lba: u64,
    name11: &[u8; 11],
) -> Result<Option<(u32, u32)>, String> {
    let mut f = OpenOptions::new()
        .read(true)
        .open(img)
        .map_err(|e| format!("open: {e}"))?;
    let vol = read_vol(&mut f, part_first_lba)?;
    find_entry(&mut f, &[vol.root_lba], name11, false)
}

pub fn write_root_file(
    img: &Path,
    part_first_lba: u64,
    name: &[u8; 8],
    ext: &[u8; 3],
    data: &[u8],
) -> Result<(), String> {
    let mut f = OpenOptions::new()
        .read(true)
        .write(true)
        .open(img)
        .map_err(|e| format!("open: {e}"))?;
    let vol = read_vol(&mut f, part_first_lba)?;

    let mut name11 = [0u8; 11];
    name11[..8].copy_from_slice(name);
    name11[8..11].copy_from_slice(ext);

    if let Some((cluster, size)) = find_entry(&mut f, &[vol.root_lba], &name11, false)? {
        let clusters_needed =
            (data.len() as u64 + vol.bps * vol.spc - 1) / (vol.bps * vol.spc);
        let clusters_have = (size as u64 + vol.bps * vol.spc - 1) / (vol.bps * vol.spc);
        if size as usize == data.len() && clusters_have >= clusters_needed {
            write_at(
                &mut f,
                vol.data_start + (cluster as u64 - 2) * vol.spc * vol.bps,
                data,
            )?;
            return Ok(());
        }
        if size as usize != data.len() {
            return Err(format!(
                "{}.{} ya existe con tamaño {size} B (pedido {} B)",
                String::from_utf8_lossy(name).trim_end(),
                String::from_utf8_lossy(ext),
                data.len()
            ));
        }
    }

    let clusters_needed = (data.len() as u64 + vol.bps * vol.spc - 1) / (vol.bps * vol.spc);
    if clusters_needed == 0 {
        return Err("fichero vacío".into());
    }

    let fat_bytes = (vol.spf * vol.bps) as usize;
    let mut fat = vec![0u8; fat_bytes];
    read_at(&mut f, vol.base + vol.reserved * vol.bps, &mut fat)?;

    let start_cluster = if vol.fat16 {
        find_contiguous_run16(&fat, clusters_needed as u32)
    } else {
        find_contiguous_run32(&fat, clusters_needed as u32)
    }
    .ok_or_else(|| format!("sin {clusters_needed} clusters libres contiguos"))?;

    if vol.fat16 {
        mark_contiguous_chain16(&mut fat, start_cluster, clusters_needed as u32);
    } else {
        mark_contiguous_chain32(&mut fat, start_cluster, clusters_needed as u32);
    }
    let fat_off = vol.base + vol.reserved * vol.bps;
    write_at(&mut f, fat_off, &fat)?;
    if vol.num_fats > 1 {
        write_at(&mut f, fat_off + vol.spf * vol.bps, &fat)?;
    }

    write_at(
        &mut f,
        vol.data_start + (start_cluster - 2) as u64 * vol.spc * vol.bps,
        data,
    )?;

    install_dir_entry(
        &mut f,
        &vol,
        name,
        ext,
        start_cluster,
        data.len() as u32,
    )?;

    Ok(())
}

/// Crea (o rellena si ya tiene ese tamaño) un fichero raíz contiguo sin
/// materializar `size` bytes en RAM.
pub fn write_root_file_fill(
    img: &Path,
    part_first_lba: u64,
    name: &[u8; 8],
    ext: &[u8; 3],
    size: u32,
    fill: u8,
) -> Result<(), String> {
    let mut f = OpenOptions::new()
        .read(true)
        .write(true)
        .open(img)
        .map_err(|e| format!("open: {e}"))?;
    let vol = read_vol(&mut f, part_first_lba)?;

    let mut name11 = [0u8; 11];
    name11[..8].copy_from_slice(name);
    name11[8..11].copy_from_slice(ext);

    if let Some((cluster, existing)) = find_entry(&mut f, &[vol.root_lba], &name11, false)? {
        if existing == size {
            write_fill(
                &mut f,
                vol.data_start + (cluster as u64 - 2) * vol.spc * vol.bps,
                size as usize,
                fill,
            )?;
            return Ok(());
        }
        return Err(format!(
            "{}.{} ya existe con tamaño {existing} B (pedido {size} B)",
            String::from_utf8_lossy(name).trim_end(),
            String::from_utf8_lossy(ext)
        ));
    }

    let cluster_bytes = vol.bps * vol.spc;
    let n = clusters_needed(size as u64, cluster_bytes);
    if n == 0 {
        return Err("fichero vacío".into());
    }

    let fat_bytes = (vol.spf * vol.bps) as usize;
    let mut fat = vec![0u8; fat_bytes];
    read_at(&mut f, vol.base + vol.reserved * vol.bps, &mut fat)?;

    let start_cluster = find_run(&vol, &fat, n)
        .ok_or_else(|| format!("sin {n} clusters libres contiguos"))?;
    mark_run(&vol, &mut fat, start_cluster, n);
    write_fats(&mut f, &vol, &fat)?;
    write_fill(&mut f, cluster_off(&vol, start_cluster), size as usize, fill)?;
    install_dir_entry(&mut f, &vol, name, ext, start_cluster, size)?;
    Ok(())
}

/// Agranda un fichero de la raíz conservando el prefijo y rellenando con ceros.
/// Reubica a un run contiguo nuevo si no se puede extender in situ.
/// Copia y relleno van a trozos: un `Vec` del hueco (64–96 MiB) tumba el host.
pub fn grow_root_file(
    img: &Path,
    part_first_lba: u64,
    name11: &[u8; 11],
    new_size: u32,
) -> Result<u32, String> {
    let mut f = OpenOptions::new()
        .read(true)
        .write(true)
        .open(img)
        .map_err(|e| format!("open: {e}"))?;
    let vol = read_vol(&mut f, part_first_lba)?;
    let fat_bytes = (vol.spf * vol.bps) as usize;
    let mut fat = vec![0u8; fat_bytes];
    read_at(&mut f, vol.base + vol.reserved * vol.bps, &mut fat)?;

    let root_cluster = if vol.fat16 {
        None
    } else {
        Some(((vol.root_lba - vol.data_start) / (vol.spc * vol.bps)) as u32 + 2)
    };
    let offsets = dir_sector_offsets(&vol, &fat, root_cluster);
    let (old_cluster, old_size, dir_off, ent_off) =
        find_entry_loc(&mut f, &offsets, name11, false)?
            .ok_or_else(|| format!("fichero {:?} no encontrado", ascii(name11)))?;

    if old_size >= new_size {
        return Ok(old_size);
    }

    let cluster_bytes = vol.spc * vol.bps;
    let old_n = clusters_needed(old_size as u64, cluster_bytes);
    let new_n = clusters_needed(new_size as u64, cluster_bytes);
    if new_n == 0 {
        return Err("fichero vacío".into());
    }

    let old_off = if old_cluster >= 2 {
        Some(cluster_off(&vol, old_cluster))
    } else {
        None
    };

    let inplace = old_cluster >= 2
        && old_n > 0
        && (new_n <= old_n
            || run_is_free(&vol, &fat, old_cluster + old_n, new_n - old_n));

    let start = if inplace {
        extend_chain(&vol, &mut fat, old_cluster, old_n, new_n);
        write_fats(&mut f, &vol, &fat)?;
        old_cluster
    } else if let Some(start) = find_run(&vol, &fat, new_n) {
        mark_run(&vol, &mut fat, start, new_n);
        write_fats(&mut f, &vol, &fat)?;
        let dst = cluster_off(&vol, start);
        if let Some(src) = old_off {
            if old_size > 0 {
                copy_bytes(&mut f, src, dst, old_size as usize)?;
            }
            if old_cluster >= 2 {
                free_chain(&vol, &mut fat, old_cluster);
                write_fats(&mut f, &vol, &fat)?;
            }
        }
        start
    } else {
        if old_cluster >= 2 {
            free_chain(&vol, &mut fat, old_cluster);
        }
        let start = find_run(&vol, &fat, new_n)
            .ok_or_else(|| format!("sin {new_n} clusters libres contiguos"))?;
        mark_run(&vol, &mut fat, start, new_n);
        write_fats(&mut f, &vol, &fat)?;
        let dst = cluster_off(&vol, start);
        if let Some(src) = old_off {
            if old_size > 0 {
                copy_bytes(&mut f, src, dst, old_size as usize)?;
            }
        }
        start
    };

    let pad_off = cluster_off(&vol, start) + old_size as u64;
    write_fill(&mut f, pad_off, (new_size - old_size) as usize, 0)?;
    write_dirent_cluster_size(&mut f, dir_off, ent_off, start, new_size)?;
    Ok(new_size)
}

fn free_chain(vol: &Vol, fat: &mut [u8], mut c: u32) {
    for _ in 0..1_000_000 {
        if c < 2 {
            break;
        }
        let next = fat_entry(vol, fat, c);
        if vol.fat16 {
            set_fat16(fat, c, 0);
        } else {
            set_fat32(fat, c, 0);
        }
        if next < 2 || is_eoc(vol, next) || next == c {
            break;
        }
        c = next;
    }
}

fn find_entry_loc(
    f: &mut std::fs::File,
    sector_offsets: &[u64],
    name11: &[u8; 11],
    want_dir: bool,
) -> Result<Option<(u32, u32, u64, usize)>, String> {
    for &off in sector_offsets {
        let mut sec = [0u8; 512];
        read_at(f, off, &mut sec)?;
        for i in 0..16 {
            let e = &sec[i * 32..i * 32 + 32];
            if e[0] == 0x00 {
                return Ok(None);
            }
            if e[0] == 0xE5 || e[11] & 0x0F == 0x0F || e[11] & 0x08 != 0 {
                continue;
            }
            if &e[0..11] != name11 || want_dir != (e[11] & 0x10 != 0) {
                continue;
            }
            let hi = u16::from_le_bytes([e[20], e[21]]) as u32;
            let lo = u16::from_le_bytes([e[26], e[27]]) as u32;
            let size = u32::from_le_bytes([e[28], e[29], e[30], e[31]]);
            return Ok(Some(((hi << 16) | lo, size, off, i * 32)));
        }
    }
    Ok(None)
}

/// Nombre 8.3 que fatfs/bootloader deja para `kernel-x86_64`.
pub const KERNEL_8_3: [u8; 11] = *b"KERNEL~1   ";

/// Localiza el kernel en la raíz: `KERNEL~1` o el primer fichero ELF.
pub fn find_root_kernel_name(img: &Path, part_first_lba: u64) -> Result<[u8; 11], String> {
    let files = list_root_files(img, part_first_lba)?;
    if files.iter().any(|(n, _)| *n == KERNEL_8_3) {
        return Ok(KERNEL_8_3);
    }
    for (name, size) in files {
        if size < 4 {
            continue;
        }
        let head = read_in_dir_prefix(img, part_first_lba, &[], &name, 4)?;
        if head == b"\x7fELF" {
            return Ok(name);
        }
    }
    Err("sin kernel ELF (KERNEL~1 / kernel-x86_64) en la ESP".into())
}

/// Sectores totales anunciados en el BPB (FAT16 `total_16` o FAT32 `total_32`).
#[allow(dead_code)] // test `--only kernel` 31→96 MiB
pub fn bpb_total_sectors(img: &Path, part_first_lba: u64) -> Result<u32, String> {
    let mut f = OpenOptions::new()
        .read(true)
        .open(img)
        .map_err(|e| format!("open: {e}"))?;
    let mut boot = [0u8; 512];
    read_at(&mut f, part_first_lba * SECTOR, &mut boot)?;
    if boot[510] != 0x55 || boot[511] != 0xAA {
        return Err("sin firma FAT boot".into());
    }
    let total16 = u16::from_le_bytes([boot[19], boot[20]]) as u32;
    let total32 = u32::from_le_bytes([boot[32], boot[33], boot[34], boot[35]]);
    Ok(if total16 != 0 { total16 } else { total32 })
}

/// Lee un fichero 8.3 existente (`dir_path` vacío = raíz).
pub fn read_in_dir(
    img: &Path,
    part_first_lba: u64,
    dir_path: &[[u8; 11]],
    file_name11: &[u8; 11],
) -> Result<Vec<u8>, String> {
    let mut f = OpenOptions::new()
        .read(true)
        .open(img)
        .map_err(|e| format!("open: {e}"))?;
    let located = locate_in_dir(&mut f, part_first_lba, dir_path, file_name11)?;
    let mut buf = vec![0u8; located.size as usize];
    read_at(&mut f, located.data_off, &mut buf)?;
    Ok(buf)
}

pub(crate) fn read_in_dir_prefix(
    img: &Path,
    part_first_lba: u64,
    dir_path: &[[u8; 11]],
    file_name11: &[u8; 11],
    n: usize,
) -> Result<Vec<u8>, String> {
    let mut f = OpenOptions::new()
        .read(true)
        .open(img)
        .map_err(|e| format!("open: {e}"))?;
    let located = locate_in_dir(&mut f, part_first_lba, dir_path, file_name11)?;
    let take = n.min(located.size as usize);
    let mut buf = vec![0u8; take];
    read_at(&mut f, located.data_off, &mut buf)?;
    Ok(buf)
}

/// Sobrescribe **in situ** los datos de un fichero existente (`dir_path` son
/// componentes 8.3 de 11 bytes, p. ej. `EFI` → `b"EFI        "`), rellenando
/// con ceros hasta el tamaño original. No toca FAT ni directorios: exige que
/// `data` quepa en el fichero y que sus clusters sean contiguos (cierto en la
/// ESP recién generada por fatfs). Devuelve el tamaño original del fichero.
pub fn overwrite_in_dir(
    img: &Path,
    part_first_lba: u64,
    dir_path: &[[u8; 11]],
    file_name11: &[u8; 11],
    data: &[u8],
) -> Result<u32, String> {
    let mut f = OpenOptions::new()
        .read(true)
        .write(true)
        .open(img)
        .map_err(|e| format!("open: {e}"))?;
    let located = locate_in_dir(&mut f, part_first_lba, dir_path, file_name11)?;

    if data.len() as u64 > located.size as u64 {
        return Err(format!(
            "los datos ({} B) no caben en el fichero original ({} B)",
            data.len(),
            located.size
        ));
    }

    write_at(&mut f, located.data_off, data)?;
    write_fill(
        &mut f,
        located.data_off + data.len() as u64,
        located.size as usize - data.len(),
        0,
    )?;
    Ok(located.size)
}

/// Copia un fichero 8.3 de `src` al hueco de `dst` (relleno a ceros) sin
/// cargar el hueco ni el origen entero en RAM.
pub fn overwrite_in_dir_from(
    dst: &Path,
    dst_lba: u64,
    dst_dir: &[[u8; 11]],
    dst_name: &[u8; 11],
    src: &Path,
    src_lba: u64,
    src_dir: &[[u8; 11]],
    src_name: &[u8; 11],
) -> Result<(u32, u32), String> {
    let mut src_f = OpenOptions::new()
        .read(true)
        .open(src)
        .map_err(|e| format!("open {}: {e}", src.display()))?;
    let mut dst_f = OpenOptions::new()
        .read(true)
        .write(true)
        .open(dst)
        .map_err(|e| format!("open {}: {e}", dst.display()))?;
    let src_loc = locate_in_dir(&mut src_f, src_lba, src_dir, src_name)?;
    let dst_loc = locate_in_dir(&mut dst_f, dst_lba, dst_dir, dst_name)?;
    if src_loc.size as u64 > dst_loc.size as u64 {
        return Err(format!(
            "los datos ({} B) no caben en el fichero original ({} B)",
            src_loc.size, dst_loc.size
        ));
    }
    copy_between(
        &mut src_f,
        src_loc.data_off,
        &mut dst_f,
        dst_loc.data_off,
        src_loc.size as usize,
    )?;
    let rest = dst_loc.size as usize - src_loc.size as usize;
    // Hueco live (64 MiB): el grow ya lo rellenó. No volver a escribir
    // decenas de MiB de ceros al USB (page cache → OOM). Huecos pequeños
    // (tests, EFI) sí se recortan enteros.
    let pad = if rest > 1024 * 1024 {
        IO_CHUNK.min(rest)
    } else {
        rest
    };
    write_fill(
        &mut dst_f,
        dst_loc.data_off + src_loc.size as u64,
        pad,
        0,
    )?;
    drop_written_pages(&dst_f, dst_loc.data_off, src_loc.size as u64 + pad as u64);
    Ok((src_loc.size, dst_loc.size))
}

struct LocatedFile {
    data_off: u64,
    size: u32,
}

fn locate_in_dir(
    f: &mut std::fs::File,
    part_first_lba: u64,
    dir_path: &[[u8; 11]],
    file_name11: &[u8; 11],
) -> Result<LocatedFile, String> {
    let vol = read_vol(f, part_first_lba)?;

    let fat_bytes = (vol.spf * vol.bps) as usize;
    let mut fat = vec![0u8; fat_bytes];
    read_at(f, vol.base + vol.reserved * vol.bps, &mut fat)?;

    // Raíz: área fija en FAT16; primer cluster (recalculado) en FAT32.
    let root_cluster = if vol.fat16 {
        None
    } else {
        Some(((vol.root_lba - vol.data_start) / (vol.spc * vol.bps)) as u32 + 2)
    };

    let mut dir = root_cluster;
    let mut in_root = vol.fat16;
    for comp in dir_path {
        let offsets = dir_sector_offsets(&vol, &fat, if in_root { None } else { dir });
        let (cluster, _) = find_entry(f, &offsets, comp, true)?
            .ok_or_else(|| format!("directorio {:?} no encontrado", ascii(comp)))?;
        dir = Some(cluster);
        in_root = false;
    }

    let offsets = dir_sector_offsets(&vol, &fat, if in_root { None } else { dir });
    let (cluster, size) = find_entry(f, &offsets, file_name11, false)?
        .ok_or_else(|| format!("fichero {:?} no encontrado", ascii(file_name11)))?;

    let cluster_bytes = vol.spc * vol.bps;
    if size > 0 {
        let clusters = ((size as u64 + cluster_bytes - 1) / cluster_bytes) as u32;
        let mut c = cluster;
        for i in 0..clusters {
            let next = fat_entry(&vol, &fat, c);
            if i + 1 < clusters {
                if next != c + 1 {
                    return Err(format!("fichero no contiguo (cluster {c} → {next})"));
                }
                c = next;
            } else if !is_eoc(&vol, next) {
                return Err(format!("cadena FAT no termina donde debería (cluster {c})"));
            }
        }
    }

    Ok(LocatedFile {
        data_off: vol.data_start + (cluster - 2) as u64 * cluster_bytes,
        size,
    })
}

/// Lista ficheros 8.3 del directorio raíz (sin LFN): `(nombre 11 bytes, tamaño)`.
pub fn list_root_files(img: &Path, part_first_lba: u64) -> Result<Vec<([u8; 11], u32)>, String> {
    let mut f = OpenOptions::new()
        .read(true)
        .open(img)
        .map_err(|e| format!("open: {e}"))?;
    let vol = read_vol(&mut f, part_first_lba)?;

    let fat_bytes = (vol.spf * vol.bps) as usize;
    let mut fat = vec![0u8; fat_bytes];
    read_at(&mut f, vol.base + vol.reserved * vol.bps, &mut fat)?;

    let root_cluster = if vol.fat16 {
        None
    } else {
        Some(((vol.root_lba - vol.data_start) / (vol.spc * vol.bps)) as u32 + 2)
    };
    let offsets = dir_sector_offsets(&vol, &fat, root_cluster);
    let mut out = Vec::new();
    for &off in &offsets {
        let mut sec = [0u8; 512];
        read_at(&mut f, off, &mut sec)?;
        for i in 0..16 {
            let e = &sec[i * 32..i * 32 + 32];
            if e[0] == 0x00 {
                return Ok(out);
            }
            if e[0] == 0xE5 || e[11] & 0x0F == 0x0F || e[11] & 0x08 != 0 {
                continue;
            }
            if e[11] & 0x10 != 0 {
                continue;
            }
            let size = u32::from_le_bytes([e[28], e[29], e[30], e[31]]);
            let name11: [u8; 11] = e[0..11].try_into().map_err(|_| "entry name")?;
            out.push((name11, size));
        }
    }
    Ok(out)
}

/// Lee un fichero contiguo del directorio raíz. Sirve para inspeccionar desde
/// el host lo que el guest dejó en la ESP (`SOSOBOOT.TXT`, `SOSOLOG.TXT`).
pub fn read_root_file(
    img: &Path,
    part_first_lba: u64,
    file_name11: &[u8; 11],
) -> Result<Vec<u8>, String> {
    let mut f = OpenOptions::new()
        .read(true)
        .open(img)
        .map_err(|e| format!("open: {e}"))?;
    let vol = read_vol(&mut f, part_first_lba)?;

    let fat_bytes = (vol.spf * vol.bps) as usize;
    let mut fat = vec![0u8; fat_bytes];
    read_at(&mut f, vol.base + vol.reserved * vol.bps, &mut fat)?;

    let root_cluster = if vol.fat16 {
        None
    } else {
        Some(((vol.root_lba - vol.data_start) / (vol.spc * vol.bps)) as u32 + 2)
    };
    let offsets = dir_sector_offsets(&vol, &fat, root_cluster);
    let (cluster, size) = find_entry(&mut f, &offsets, file_name11, false)?
        .ok_or_else(|| format!("fichero {:?} no encontrado", ascii(file_name11)))?;

    let cluster_bytes = vol.spc * vol.bps;
    let mut buf = vec![0u8; size as usize];
    read_at(
        &mut f,
        vol.data_start + (cluster - 2) as u64 * cluster_bytes,
        &mut buf,
    )?;
    Ok(buf)
}

/// Offsets absolutos (bytes) de los sectores de un directorio: raíz FAT16
/// (`None`) o cadena de clusters.
fn dir_sector_offsets(vol: &Vol, fat: &[u8], start_cluster: Option<u32>) -> Vec<u64> {
    match start_cluster {
        None => (0..vol.root_sectors)
            .map(|s| vol.root_lba + s * SECTOR)
            .collect(),
        Some(mut c) => {
            let mut v = Vec::new();
            loop {
                let lba = vol.data_start + (c as u64 - 2) * vol.spc * vol.bps;
                for s in 0..vol.spc {
                    v.push(lba + s * SECTOR);
                }
                let next = fat_entry(vol, fat, c);
                if next < 2 || is_eoc(vol, next) || v.len() > 65536 {
                    break;
                }
                c = next;
            }
            v
        }
    }
}

/// Busca una entrada 8.3 (nombre+extensión, 11 bytes) en los sectores dados.
/// Devuelve (primer cluster, tamaño). Ignora entradas LFN y etiquetas.
fn find_entry(
    f: &mut std::fs::File,
    sector_offsets: &[u64],
    name11: &[u8; 11],
    want_dir: bool,
) -> Result<Option<(u32, u32)>, String> {
    for &off in sector_offsets {
        let mut sec = [0u8; 512];
        read_at(f, off, &mut sec)?;
        for i in 0..16 {
            let e = &sec[i * 32..i * 32 + 32];
            if e[0] == 0x00 {
                return Ok(None);
            }
            if e[0] == 0xE5 || e[11] & 0x0F == 0x0F || e[11] & 0x08 != 0 {
                continue;
            }
            if &e[0..11] != name11 || want_dir != (e[11] & 0x10 != 0) {
                continue;
            }
            let hi = u16::from_le_bytes([e[20], e[21]]) as u32;
            let lo = u16::from_le_bytes([e[26], e[27]]) as u32;
            let size = u32::from_le_bytes([e[28], e[29], e[30], e[31]]);
            return Ok(Some(((hi << 16) | lo, size)));
        }
    }
    Ok(None)
}

fn fat_entry(vol: &Vol, fat: &[u8], cluster: u32) -> u32 {
    if vol.fat16 {
        fat16_entry(fat, cluster) as u32
    } else {
        fat32_entry(fat, cluster)
    }
}

fn is_eoc(vol: &Vol, entry: u32) -> bool {
    if vol.fat16 {
        entry >= 0xFFF8
    } else {
        entry >= 0x0FFF_FFF8
    }
}

fn ascii(name: &[u8; 11]) -> String {
    String::from_utf8_lossy(name).trim().to_string()
}

fn read_vol(f: &mut std::fs::File, part_first_lba: u64) -> Result<Vol, String> {
    let base = part_first_lba * SECTOR;
    let mut boot = [0u8; 512];
    read_at(f, base, &mut boot)?;
    if boot[510] != 0x55 || boot[511] != 0xAA {
        return Err("sin firma FAT boot".into());
    }
    let bps = u16::from_le_bytes([boot[11], boot[12]]) as u64;
    if bps != SECTOR {
        return Err(format!("sector size {bps} != 512").into());
    }
    let spc = boot[13] as u64;
    if spc == 0 {
        return Err("sectors_per_cluster=0".into());
    }
    let reserved = u16::from_le_bytes([boot[14], boot[15]]) as u64;
    let num_fats = boot[16] as u64;
    let root_entries = u16::from_le_bytes([boot[17], boot[18]]);
    let spf16 = u16::from_le_bytes([boot[22], boot[23]]) as u64;
    let spf32 = u32::from_le_bytes([boot[36], boot[37], boot[38], boot[39]]) as u64;

    if root_entries != 0 {
        if spf16 == 0 {
            return Err("BPB FAT16 inválido".into());
        }
        let root_sectors = ((root_entries as u64 * 32) + bps - 1) / bps;
        let data_start = base + (reserved + num_fats * spf16 + root_sectors) * bps;
        Ok(Vol {
            base,
            bps,
            spc,
            reserved,
            num_fats,
            spf: spf16,
            fat16: true,
            root_lba: base + (reserved + num_fats * spf16) * bps,
            root_sectors,
            data_start,
        })
    } else {
        if spf32 == 0 {
            return Err("BPB FAT32 inválido".into());
        }
        let root_cluster = u32::from_le_bytes([boot[44], boot[45], boot[46], boot[47]]);
        if root_cluster < 2 {
            return Err("root_cluster inválido".into());
        }
        let data_start = base + (reserved + num_fats * spf32) * bps;
        Ok(Vol {
            base,
            bps,
            spc,
            reserved,
            num_fats,
            spf: spf32,
            fat16: false,
            root_lba: data_start + (root_cluster as u64 - 2) * spc * bps,
            root_sectors: spc,
            data_start,
        })
    }
}

fn read_at(f: &mut std::fs::File, off: u64, buf: &mut [u8]) -> Result<(), String> {
    f.seek(SeekFrom::Start(off)).map_err(|e| e.to_string())?;
    f.read_exact(buf).map_err(|e| e.to_string())
}

fn write_at(f: &mut std::fs::File, off: u64, buf: &[u8]) -> Result<(), String> {
    f.seek(SeekFrom::Start(off)).map_err(|e| e.to_string())?;
    f.write_all(buf).map_err(|e| e.to_string())
}

fn clusters_needed(size: u64, cluster_bytes: u64) -> u32 {
    if size == 0 {
        0
    } else {
        size.div_ceil(cluster_bytes) as u32
    }
}

fn cluster_off(vol: &Vol, cluster: u32) -> u64 {
    vol.data_start + (cluster as u64 - 2) * vol.spc * vol.bps
}

fn fat_cluster_max(vol: &Vol, fat: &[u8]) -> u32 {
    if vol.fat16 {
        (fat.len() / 2) as u32
    } else {
        (fat.len() / 4) as u32
    }
}

fn run_is_free(vol: &Vol, fat: &[u8], start: u32, count: u32) -> bool {
    if count == 0 {
        return true;
    }
    let max = fat_cluster_max(vol, fat);
    if start < 2 || start.saturating_add(count) > max {
        return false;
    }
    (0..count).all(|i| fat_entry(vol, fat, start + i) == 0)
}

fn find_run(vol: &Vol, fat: &[u8], count: u32) -> Option<u32> {
    if vol.fat16 {
        find_contiguous_run16(fat, count)
    } else {
        find_contiguous_run32(fat, count)
    }
}

fn mark_run(vol: &Vol, fat: &mut [u8], start: u32, count: u32) {
    if vol.fat16 {
        mark_contiguous_chain16(fat, start, count);
    } else {
        mark_contiguous_chain32(fat, start, count);
    }
}

fn extend_chain(vol: &Vol, fat: &mut [u8], start: u32, old_n: u32, new_n: u32) {
    if new_n <= old_n {
        return;
    }
    let extra = start + old_n;
    if old_n > 0 {
        if vol.fat16 {
            set_fat16(fat, start + old_n - 1, extra as u16);
        } else {
            set_fat32(fat, start + old_n - 1, extra);
        }
    }
    mark_run(vol, fat, extra, new_n - old_n);
}

fn write_fats(f: &mut std::fs::File, vol: &Vol, fat: &[u8]) -> Result<(), String> {
    let fat_off = vol.base + vol.reserved * vol.bps;
    write_at(f, fat_off, fat)?;
    if vol.num_fats > 1 {
        write_at(f, fat_off + vol.spf * vol.bps, fat)?;
    }
    Ok(())
}

fn write_dirent_cluster_size(
    f: &mut std::fs::File,
    dir_off: u64,
    ent_off: usize,
    cluster: u32,
    size: u32,
) -> Result<(), String> {
    let mut sec = [0u8; 512];
    read_at(f, dir_off, &mut sec)?;
    sec[ent_off + 20..ent_off + 22].copy_from_slice(&(cluster >> 16).to_le_bytes()[..2]);
    sec[ent_off + 26..ent_off + 28].copy_from_slice(&(cluster as u16).to_le_bytes());
    sec[ent_off + 28..ent_off + 32].copy_from_slice(&size.to_le_bytes());
    write_at(f, dir_off, &sec)
}

fn write_fill(f: &mut std::fs::File, off: u64, len: usize, byte: u8) -> Result<(), String> {
    if len == 0 {
        return Ok(());
    }
    let chunk = vec![byte; IO_CHUNK.min(len)];
    let mut done = 0usize;
    while done < len {
        let n = (len - done).min(chunk.len());
        write_at(f, off + done as u64, &chunk[..n])?;
        done += n;
        if len > IO_CHUNK {
            drop_written_pages(f, off + (done - n) as u64, n as u64);
        }
    }
    Ok(())
}

#[cfg(all(unix, target_os = "linux"))]
fn drop_written_pages(f: &std::fs::File, off: u64, len: u64) {
    use std::os::unix::io::AsRawFd;
    unsafe {
        posix_fadvise(f.as_raw_fd(), off as i64, len as i64, POSIX_FADV_DONTNEED);
    }
}

#[cfg(all(unix, target_os = "linux"))]
const POSIX_FADV_DONTNEED: i32 = 4;

#[cfg(all(unix, target_os = "linux"))]
unsafe extern "C" {
    fn posix_fadvise(fd: i32, offset: i64, len: i64, advice: i32) -> i32;
}

#[cfg(not(all(unix, target_os = "linux")))]
fn drop_written_pages(_f: &std::fs::File, _off: u64, _len: u64) {}

pub(crate) fn drop_path_cache(path: &Path) {
    if let Ok(f) = OpenOptions::new().read(true).open(path) {
        drop_written_pages(&f, 0, 0);
    }
}

fn copy_bytes(f: &mut std::fs::File, src: u64, dst: u64, len: usize) -> Result<(), String> {
    if len == 0 || src == dst {
        return Ok(());
    }
    let mut buf = vec![0u8; IO_CHUNK.min(len)];
    if dst > src && dst < src + len as u64 {
        let mut done = 0usize;
        while done < len {
            let n = (len - done).min(buf.len());
            let off = len - done - n;
            read_at(f, src + off as u64, &mut buf[..n])?;
            write_at(f, dst + off as u64, &buf[..n])?;
            done += n;
        }
    } else {
        let mut done = 0usize;
        while done < len {
            let n = (len - done).min(buf.len());
            read_at(f, src + done as u64, &mut buf[..n])?;
            write_at(f, dst + done as u64, &buf[..n])?;
            done += n;
        }
    }
    Ok(())
}

fn copy_between(
    src: &mut std::fs::File,
    src_off: u64,
    dst: &mut std::fs::File,
    dst_off: u64,
    len: usize,
) -> Result<(), String> {
    if len == 0 {
        return Ok(());
    }
    let mut buf = vec![0u8; IO_CHUNK.min(len)];
    let mut done = 0usize;
    while done < len {
        let n = (len - done).min(buf.len());
        read_at(src, src_off + done as u64, &mut buf[..n])?;
        write_at(dst, dst_off + done as u64, &buf[..n])?;
        if len > IO_CHUNK {
            drop_written_pages(dst, dst_off + done as u64, n as u64);
        }
        done += n;
    }
    Ok(())
}

fn find_contiguous_run16(fat: &[u8], count: u32) -> Option<u32> {
    let max = (fat.len() / 2) as u32;
    let mut cluster = 2u32;
    while cluster + count <= max {
        let mut ok = true;
        for i in 0..count {
            if fat16_entry(fat, cluster + i) != 0 {
                ok = false;
                break;
            }
        }
        if ok {
            return Some(cluster);
        }
        cluster += 1;
    }
    None
}

fn find_contiguous_run32(fat: &[u8], count: u32) -> Option<u32> {
    let max = (fat.len() / 4) as u32;
    let mut cluster = 2u32;
    while cluster + count <= max {
        let mut ok = true;
        for i in 0..count {
            if fat32_entry(fat, cluster + i) != 0 {
                ok = false;
                break;
            }
        }
        if ok {
            return Some(cluster);
        }
        cluster += 1;
    }
    None
}

fn fat16_entry(fat: &[u8], cluster: u32) -> u16 {
    let off = (cluster as usize) * 2;
    u16::from_le_bytes([fat[off], fat[off + 1]])
}

fn fat32_entry(fat: &[u8], cluster: u32) -> u32 {
    let off = (cluster as usize) * 4;
    u32::from_le_bytes([fat[off], fat[off + 1], fat[off + 2], fat[off + 3]]) & 0x0FFF_FFFF
}

fn set_fat16(fat: &mut [u8], cluster: u32, value: u16) {
    let off = (cluster as usize) * 2;
    fat[off..off + 2].copy_from_slice(&value.to_le_bytes());
}

fn set_fat32(fat: &mut [u8], cluster: u32, value: u32) {
    let off = (cluster as usize) * 4;
    let old = u32::from_le_bytes([fat[off], fat[off + 1], fat[off + 2], fat[off + 3]]);
    let new = (old & 0xF000_0000) | (value & 0x0FFF_FFFF);
    fat[off..off + 4].copy_from_slice(&new.to_le_bytes());
}

fn mark_contiguous_chain16(fat: &mut [u8], start: u32, count: u32) {
    for i in 0..count - 1 {
        set_fat16(fat, start + i, (start + i + 1) as u16);
    }
    set_fat16(fat, start + count - 1, 0xFFFF);
}

fn mark_contiguous_chain32(fat: &mut [u8], start: u32, count: u32) {
    for i in 0..count - 1 {
        set_fat32(fat, start + i, start + i + 1);
    }
    set_fat32(fat, start + count - 1, 0x0FFF_FFFF);
}

fn install_dir_entry(
    f: &mut std::fs::File,
    vol: &Vol,
    name: &[u8; 8],
    ext: &[u8; 3],
    cluster: u32,
    size: u32,
) -> Result<(), String> {
    for s in 0..vol.root_sectors {
        let mut sec = [0u8; 512];
        read_at(f, vol.root_lba + s * SECTOR, &mut sec)?;
        for i in 0..16 {
            let off = i * 32;
            if sec[off] == 0x00 || sec[off] == 0xE5 {
                sec[off..off + 8].copy_from_slice(name);
                sec[off + 8..off + 11].copy_from_slice(ext);
                sec[off + 11] = 0x20;
                sec[off + 20..off + 22]
                    .copy_from_slice(&(cluster >> 16).to_le_bytes()[..2]);
                sec[off + 26..off + 28].copy_from_slice(&(cluster as u16).to_le_bytes());
                sec[off + 28..off + 32].copy_from_slice(&size.to_le_bytes());
                write_at(f, vol.root_lba + s * SECTOR, &sec)?;
                return Ok(());
            }
        }
    }
    Err("directorio raíz lleno".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::process::Command;

    fn write_min_fat16(img: &mut std::fs::File) {
        let mut boot = [0u8; 512];
        boot[510] = 0x55;
        boot[511] = 0xAA;
        boot[11..13].copy_from_slice(&512u16.to_le_bytes());
        boot[13] = 1;
        boot[16] = 2;
        boot[17..19].copy_from_slice(&512u16.to_le_bytes());
        boot[22..24].copy_from_slice(&1u16.to_le_bytes());
        img.write_all(&boot).unwrap();
        img.write_all(&vec![0u8; 512 * 2]).unwrap();
        img.write_all(&vec![0u8; 512 * 32]).unwrap();
        img.write_all(&vec![0u8; 512 * 64]).unwrap();
    }

    #[test]
    fn write_root_file_twice_no_duplica() {
        let path = std::env::temp_dir().join("soso-fat32-write-test.img");
        {
            let mut f = std::fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&path)
                .unwrap();
            write_min_fat16(&mut f);
        }
        let name = *b"TESTFILE";
        let ext = *b"TXT";
        write_root_file(&path, 0, &name, &ext, b"uno").unwrap();
        write_root_file(&path, 0, &name, &ext, b"uno").unwrap();
        let mut name11 = [0u8; 11];
        name11[..8].copy_from_slice(&name);
        name11[8..11].copy_from_slice(&ext);
        let mut f = std::fs::OpenOptions::new().read(true).open(&path).unwrap();
        let vol = read_vol(&mut f, 0).unwrap();
        let mut count = 0;
        for s in 0..vol.root_sectors {
            let mut sec = [0u8; 512];
            read_at(&mut f, vol.root_lba + s * 512, &mut sec).unwrap();
            for i in 0..16 {
                let e = &sec[i * 32..i * 32 + 32];
                if e[0] != 0x00 && e[0] != 0xE5 && &e[0..11] == &name11 {
                    count += 1;
                }
            }
        }
        let _ = std::fs::remove_file(&path);
        assert_eq!(count, 1);
    }

    #[test]
    fn grow_root_file_relocate_contiguous() {
        let mkfs = ["mkfs.vfat", "/usr/sbin/mkfs.vfat"]
            .into_iter()
            .find(|p| Command::new(p).arg("-V").output().is_ok());
        let Some(mkfs) = mkfs else {
            eprintln!("grow_root_file: sin mkfs.vfat, omito");
            return;
        };
        let path = std::env::temp_dir().join(format!("soso-fat-grow-{}", std::process::id()));
        {
            let f = std::fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&path)
                .unwrap();
            f.set_len(8 * 1024 * 1024).unwrap();
        }
        let st = Command::new(mkfs)
            .args(["-F", "32"])
            .arg(&path)
            .status()
            .unwrap();
        assert!(st.success());
        let mut small = vec![0x7f, b'E', b'L', b'F'];
        small.resize(4096, 0x11);
        write_root_file(&path, 0, b"KERNEL~1", b"   ", &small).unwrap();
        grow_root_file(&path, 0, &KERNEL_8_3, 256 * 1024).unwrap();
        let got = read_root_file(&path, 0, &KERNEL_8_3).unwrap();
        assert_eq!(got.len(), 256 * 1024);
        assert_eq!(&got[..4], b"\x7fELF");
        assert_eq!(got[4], 0x11);
        assert!(got[4096..].iter().all(|&b| b == 0));

        write_root_file_fill(&path, 0, b"SOSOKRN ", b"BIN", 128 * 1024, b'\n').unwrap();
        let slot = read_root_file(&path, 0, b"SOSOKRN BIN").unwrap();
        assert_eq!(slot.len(), 128 * 1024);
        assert!(slot.iter().all(|&b| b == b'\n'));
        let _ = std::fs::remove_file(&path);
    }
}
