//! Localizador de ficheros pre-creados en la ESP (FAT16/FAT32) del disco live.
//!
//! No sabe crear ficheros ni asignar clusters: solo encuentra dónde empiezan
//! los datos de un fichero 8.3 de la raíz **que ya existe y es contiguo**, para
//! que otros módulos puedan sobrescribirlo in situ sin tocar FAT ni directorio.
//! `xtask package-usb-live` los deja preparados (`SOSOLOG.TXT`, `SOSODRV.TXT`,
//! `SOSOBOOT.TXT`).
//!
//! Lo usan `fatlog` (log de consola), `drvlog` (informe hwscan) y `bootreq`
//! (petición de entrada de arranque para el shim UEFI).

use block_dev::BlockError;

pub const SECTOR: usize = 512;

#[derive(Clone, Copy)]
pub struct Slot {
    /// LBA relativo al inicio de la ESP donde empiezan los datos del fichero.
    pub data_lba: u64,
}

pub struct FatParams {
    bytes_per_sector: u16,
    sectors_per_cluster: u8,
    reserved_sectors: u16,
    num_fats: u8,
    root_entry_count: u16,
    sectors_per_fat: u32,
    root_cluster: u32,
    is_fat32: bool,
}

pub fn read(lba: u64, buf: &mut [u8]) -> Result<(), BlockError> {
    crate::drivers::live_disk::esp_read_sectors(lba, buf)
}

pub fn write(lba: u64, buf: &[u8]) -> Result<(), BlockError> {
    crate::drivers::live_disk::esp_write_sectors(lba, buf)
}

/// Busca `NAME.EXT` en la raíz de la ESP y exige que mida exactamente
/// `expect_size` y que sus clusters sean consecutivos: se va a sobrescribir por
/// LBA, así que un fichero fragmentado destrozaría lo que hubiera en medio.
pub fn locate(name: &[u8; 8], ext: &[u8; 3], expect_size: usize) -> Option<Slot> {
    let mut bpb_sec = [0u8; SECTOR];
    read(0, &mut bpb_sec).ok()?;
    if bpb_sec[510] != 0x55 || bpb_sec[511] != 0xAA {
        return None;
    }
    let fat = parse_bpb(&bpb_sec)?;
    let (cluster, size) = find_in_root(&fat, name, ext)?;
    if size as usize != expect_size {
        return None;
    }
    if !cluster_contiguous(&fat, cluster, size)? {
        return None;
    }
    let data_lba = cluster_to_lba(&fat, cluster)?;
    Some(Slot { data_lba })
}

fn parse_bpb(sec: &[u8; SECTOR]) -> Option<FatParams> {
    let bytes_per_sector = u16::from_le_bytes([sec[11], sec[12]]);
    if bytes_per_sector as usize != SECTOR {
        return None;
    }
    let sectors_per_cluster = sec[13];
    if sectors_per_cluster == 0 {
        return None;
    }
    let reserved_sectors = u16::from_le_bytes([sec[14], sec[15]]);
    let num_fats = sec[16];
    let root_entry_count = u16::from_le_bytes([sec[17], sec[18]]);
    let fat16_sectors = u16::from_le_bytes([sec[22], sec[23]]) as u32;
    let total_16 = u16::from_le_bytes([sec[19], sec[20]]) as u32;
    let total_32 = u32::from_le_bytes([sec[32], sec[33], sec[34], sec[35]]);
    let total = if total_16 != 0 { total_16 } else { total_32 };

    let (sectors_per_fat, root_cluster, is_fat32) = if root_entry_count == 0 {
        let spf = u32::from_le_bytes([sec[36], sec[37], sec[38], sec[39]]);
        let root = u32::from_le_bytes([sec[44], sec[45], sec[46], sec[47]]);
        if spf == 0 || root < 2 {
            return None;
        }
        (spf, root, true)
    } else {
        if fat16_sectors == 0 {
            return None;
        }
        (fat16_sectors, 0, false)
    };

    if num_fats == 0 || total == 0 {
        return None;
    }

    let _ = total;
    Some(FatParams {
        bytes_per_sector,
        sectors_per_cluster,
        reserved_sectors,
        num_fats,
        root_entry_count,
        sectors_per_fat,
        root_cluster,
        is_fat32,
    })
}

fn find_in_root(fat: &FatParams, name: &[u8; 8], ext: &[u8; 3]) -> Option<(u32, u32)> {
    if fat.is_fat32 {
        find_in_cluster_chain(fat, fat.root_cluster, name, ext)
    } else {
        let root_sectors = (fat.root_entry_count as u32 * 32).div_ceil(SECTOR as u32);
        let root_lba =
            fat.reserved_sectors as u64 + (fat.num_fats as u64 * fat.sectors_per_fat as u64);
        for s in 0..root_sectors {
            let mut sec = [0u8; SECTOR];
            read(root_lba + s as u64, &mut sec).ok()?;
            match scan_dir_sector(&sec, name, ext) {
                DirScan::Found(c, sz) => return Some((c, sz)),
                DirScan::End => return None,
                DirScan::Continue => {}
            }
        }
        None
    }
}

enum DirScan {
    Found(u32, u32),
    Continue,
    End,
}

fn find_in_cluster_chain(
    fat: &FatParams,
    start: u32,
    name: &[u8; 8],
    ext: &[u8; 3],
) -> Option<(u32, u32)> {
    let mut cluster = start;
    loop {
        let lba = cluster_to_lba(fat, cluster)?;
        for c in 0..fat.sectors_per_cluster {
            let mut sec = [0u8; SECTOR];
            read(lba + c as u64, &mut sec).ok()?;
            match scan_dir_sector(&sec, name, ext) {
                DirScan::Found(cl, sz) => return Some((cl, sz)),
                DirScan::End => return None,
                DirScan::Continue => {}
            }
        }
        let next = read_fat_entry(fat, cluster)?;
        if next >= 0x0FFF_FFF8 {
            break;
        }
        cluster = next;
    }
    None
}

fn scan_dir_sector(sec: &[u8; SECTOR], name: &[u8; 8], ext: &[u8; 3]) -> DirScan {
    for ent in sec.chunks(32) {
        if ent[0] == 0x00 {
            return DirScan::End;
        }
        if ent[0] == 0xE5 || ent[11] & 0x08 != 0 {
            continue;
        }
        if &ent[0..8] != name || &ent[8..11] != ext {
            continue;
        }
        let lo = u16::from_le_bytes([ent[26], ent[27]]) as u32;
        let hi = u16::from_le_bytes([ent[20], ent[21]]) as u32;
        let cluster = (hi << 16) | lo;
        let size = u32::from_le_bytes([ent[28], ent[29], ent[30], ent[31]]);
        if cluster < 2 {
            return DirScan::Continue;
        }
        return DirScan::Found(cluster, size);
    }
    DirScan::Continue
}

fn cluster_contiguous(fat: &FatParams, start: u32, size: u32) -> Option<bool> {
    let bps = fat.bytes_per_sector as u64;
    let spc = fat.sectors_per_cluster as u64;
    let clusters_needed = (size as u64).div_ceil(bps * spc) as u32;
    if clusters_needed == 0 {
        return Some(false);
    }
    let mut cluster = start;
    for i in 0..clusters_needed {
        if i + 1 < clusters_needed {
            let next = read_fat_entry(fat, cluster)?;
            if next != cluster + 1 {
                return Some(false);
            }
            cluster = next;
        } else {
            let next = read_fat_entry(fat, cluster)?;
            if fat.is_fat32 {
                if next < 0x0FFF_FFF8 {
                    return Some(false);
                }
            } else if next < 0xFFF8 {
                return Some(false);
            }
        }
    }
    Some(true)
}

fn read_fat_entry(fat: &FatParams, cluster: u32) -> Option<u32> {
    let fat_offset = fat.reserved_sectors as u64;
    if fat.is_fat32 {
        let fat_lba = fat_offset + (cluster as u64 * 4 / SECTOR as u64);
        let off = (cluster as u64 * 4 % SECTOR as u64) as usize;
        let mut sec = [0u8; SECTOR];
        read(fat_lba, &mut sec).ok()?;
        let entry =
            u32::from_le_bytes([sec[off], sec[off + 1], sec[off + 2], sec[off + 3]]) & 0x0FFF_FFFF;
        Some(entry)
    } else {
        let fat_lba = fat_offset + (cluster as u64 * 2 / SECTOR as u64);
        let off = (cluster as u64 * 2 % SECTOR as u64) as usize;
        let mut sec = [0u8; SECTOR];
        read(fat_lba, &mut sec).ok()?;
        let entry = u16::from_le_bytes([sec[off], sec[off + 1]]) as u32;
        Some(entry)
    }
}

fn cluster_to_lba(fat: &FatParams, cluster: u32) -> Option<u64> {
    if cluster < 2 {
        return None;
    }
    let data_start = if fat.is_fat32 {
        fat.reserved_sectors as u64 + fat.num_fats as u64 * fat.sectors_per_fat as u64
    } else {
        let root_sectors = (fat.root_entry_count as u64 * 32).div_ceil(SECTOR as u64);
        fat.reserved_sectors as u64
            + fat.num_fats as u64 * fat.sectors_per_fat as u64
            + root_sectors
    };
    Some(data_start + (cluster - 2) as u64 * fat.sectors_per_cluster as u64)
}
