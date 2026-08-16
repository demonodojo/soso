//! Escritura mínima en FAT16/FAT32 (sin mtools): un fichero contiguo en el directorio raíz.

use std::fs::OpenOptions;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

const SECTOR: u64 = 512;

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
    let vol = read_vol(&mut f, part_first_lba)?;

    let fat_bytes = (vol.spf * vol.bps) as usize;
    let mut fat = vec![0u8; fat_bytes];
    read_at(&mut f, vol.base + vol.reserved * vol.bps, &mut fat)?;

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
        let (cluster, _) = find_entry(&mut f, &offsets, comp, true)?
            .ok_or_else(|| format!("directorio {:?} no encontrado", ascii(comp)))?;
        dir = Some(cluster);
        in_root = false;
    }

    let offsets = dir_sector_offsets(&vol, &fat, if in_root { None } else { dir });
    let (cluster, size) = find_entry(&mut f, &offsets, file_name11, false)?
        .ok_or_else(|| format!("fichero {:?} no encontrado", ascii(file_name11)))?;

    if data.len() as u64 > size as u64 {
        return Err(format!(
            "los datos ({} B) no caben en el fichero original ({size} B)",
            data.len()
        ));
    }
    let cluster_bytes = vol.spc * vol.bps;
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

    let mut buf = vec![0u8; size as usize];
    buf[..data.len()].copy_from_slice(data);
    write_at(
        &mut f,
        vol.data_start + (cluster - 2) as u64 * cluster_bytes,
        &buf,
    )?;
    Ok(size)
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
