//! Volcado del ring `logbuf` a `SOSOLOG.TXT` pre-creado en la ESP (FAT).
//!
//! Solo sobrescribe los sectores de datos del fichero; no toca FAT ni directorio.

use block_dev::BlockError;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use spin::Once;

const SECTOR: usize = 512;
const FILE_SIZE: usize = 256 * 1024;
const CHUNK: usize = 128 * 1024;
const POLL_MS: u64 = 2000;

static SLOT: Once<Option<Slot>> = Once::new();
static ACTIVE: AtomicBool = AtomicBool::new(false);
static LAST_FLUSH_LEN: AtomicUsize = AtomicUsize::new(0);
static FLUSH_COUNT: AtomicU32 = AtomicU32::new(0);
static LAST_POLL_MS: AtomicUsize = AtomicUsize::new(0);

#[derive(Clone, Copy)]
struct Slot {
    /// LBA relativo al inicio de la ESP donde empiezan los datos del fichero.
    data_lba: u64,
}

struct FatParams {
    bytes_per_sector: u16,
    sectors_per_cluster: u8,
    reserved_sectors: u16,
    num_fats: u8,
    root_entry_count: u16,
    sectors_per_fat: u32,
    root_cluster: u32,
    is_fat32: bool,
}

pub fn init() {
    if !crate::drivers::live_disk::active() {
        return;
    }
    match locate_sosolog() {
        Some(slot) => {
            SLOT.call_once(|| Some(slot));
            ACTIVE.store(true, Ordering::Relaxed);
            crate::println!(
                "fatlog: SOSOLOG.TXT LBA {} ({} KiB)",
                slot.data_lba,
                FILE_SIZE / 1024
            );
            flush_or_warn();
        }
        None => crate::println!("fatlog: SOSOLOG.TXT no encontrado o no contiguo; desactivado"),
    }
}

fn flush_or_warn() {
    if flush().is_err() {
        crate::println!("fatlog: aviso: flush a SOSOLOG.TXT falló");
    }
}

/// Rate-limit ~2 s; solo escribe si el logbuf creció desde el último flush.
pub fn poll() {
    if !ACTIVE.load(Ordering::Relaxed) {
        return;
    }
    let now = crate::arch::pit::uptime_ms() as usize;
    let prev = LAST_POLL_MS.load(Ordering::Relaxed);
    if now.saturating_sub(prev) < POLL_MS as usize {
        return;
    }
    LAST_POLL_MS.store(now, Ordering::Relaxed);
    let cur = crate::drivers::logbuf::len();
    if cur > LAST_FLUSH_LEN.load(Ordering::Relaxed) {
        let _ = flush();
    }
}

pub fn flush() -> Result<(), ()> {
    if !ACTIVE.load(Ordering::Relaxed) {
        return Err(());
    }
    let slot = SLOT.get().and_then(|s| *s).ok_or(())?;
    let log_len = crate::drivers::logbuf::len();
    let n = FLUSH_COUNT.fetch_add(1, Ordering::Relaxed) + 1;
    let uptime = crate::arch::pit::uptime_ms();

    static mut BUF: [u8; FILE_SIZE] = [b'\n'; FILE_SIZE];
    // SAFETY: solo la BSP llama a flush (scheduler/kshell/panic); no reentrante.
    let buf = unsafe {
        core::slice::from_raw_parts_mut(core::ptr::addr_of_mut!(BUF).cast(), FILE_SIZE)
    };

    let mut hdr = [0u8; 128];
    let hlen = format_header(&mut hdr, n, uptime, log_len);
    let copy = hlen.min(buf.len());
    buf[..copy].copy_from_slice(&hdr[..copy]);
    let mut pos = copy;

    let mut off = 0usize;
    let mut tmp = [0u8; 4096];
    while off < log_len && pos < FILE_SIZE {
        let n = crate::drivers::logbuf::copy_from(off, &mut tmp);
        if n == 0 {
            break;
        }
        let take = n.min(FILE_SIZE - pos);
        buf[pos..pos + take].copy_from_slice(&tmp[..take]);
        pos += take;
        off += n;
    }

    let write_ok = crate::drivers::logbuf::run_without_capture(|| {
        let mut lba = slot.data_lba;
        let mut written = 0usize;
        while written < FILE_SIZE {
            let chunk = (FILE_SIZE - written).min(CHUNK);
            if esp_write(lba, &buf[written..written + chunk]).is_err() {
                return false;
            }
            lba += (chunk / SECTOR) as u64;
            written += chunk;
        }
        true
    });
    if !write_ok {
        return Err(());
    }

    LAST_FLUSH_LEN.store(log_len, Ordering::Relaxed);
    Ok(())
}

fn format_header(out: &mut [u8], flush_n: u32, uptime_ms: u64, log_len: usize) -> usize {
    use core::fmt::Write;
    struct W<'a> {
        buf: &'a mut [u8],
        pos: usize,
    }
    impl Write for W<'_> {
        fn write_str(&mut self, s: &str) -> core::fmt::Result {
            let n = s.len().min(self.buf.len().saturating_sub(self.pos));
            self.buf[self.pos..self.pos + n].copy_from_slice(&s.as_bytes()[..n]);
            self.pos += n;
            Ok(())
        }
    }
    let mut w = W { buf: out, pos: 0 };
    let _ = write!(
        w,
        "=== soso log flush #{flush_n} uptime={uptime_ms}ms bytes={log_len} ===\n"
    );
    w.pos
}

fn esp_read(lba: u64, buf: &mut [u8]) -> Result<(), BlockError> {
    crate::drivers::live_disk::esp_read_sectors(lba, buf)
}

fn esp_write(lba: u64, buf: &[u8]) -> Result<(), BlockError> {
    crate::drivers::live_disk::esp_write_sectors(lba, buf)
}

fn locate_sosolog() -> Option<Slot> {
    let mut bpb_sec = [0u8; SECTOR];
    esp_read(0, &mut bpb_sec).ok()?;
    if bpb_sec[510] != 0x55 || bpb_sec[511] != 0xAA {
        return None;
    }
    let fat = parse_bpb(&bpb_sec)?;
    let (cluster, size) = find_in_root(&fat, b"SOSOLOG ", b"TXT")?;
    if size as usize != FILE_SIZE {
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
        let root_sectors = ((fat.root_entry_count as u32 * 32) + SECTOR as u32 - 1) / SECTOR as u32;
        let root_lba = fat.reserved_sectors as u64 + (fat.num_fats as u64 * fat.sectors_per_fat as u64);
        for s in 0..root_sectors {
            let mut sec = [0u8; SECTOR];
            esp_read(root_lba + s as u64, &mut sec).ok()?;
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
            esp_read(lba + c as u64, &mut sec).ok()?;
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
    let clusters_needed = ((size as u64 + bps * spc - 1) / (bps * spc)) as u32;
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
        esp_read(fat_lba, &mut sec).ok()?;
        let entry = u32::from_le_bytes([
            sec[off],
            sec[off + 1],
            sec[off + 2],
            sec[off + 3],
        ]) & 0x0FFF_FFFF;
        Some(entry)
    } else {
        let fat_lba = fat_offset + (cluster as u64 * 2 / SECTOR as u64);
        let off = (cluster as u64 * 2 % SECTOR as u64) as usize;
        let mut sec = [0u8; SECTOR];
        esp_read(fat_lba, &mut sec).ok()?;
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
        let root_sectors =
            ((fat.root_entry_count as u64 * 32) + SECTOR as u64 - 1) / SECTOR as u64;
        fat.reserved_sectors as u64
            + fat.num_fats as u64 * fat.sectors_per_fat as u64
            + root_sectors
    };
    Some(data_start + (cluster - 2) as u64 * fat.sectors_per_cluster as u64)
}
