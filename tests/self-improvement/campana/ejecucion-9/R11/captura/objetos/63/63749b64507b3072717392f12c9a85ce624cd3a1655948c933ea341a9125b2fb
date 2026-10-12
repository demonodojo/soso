//! Tablas de particiones GPT: leer, reubicar y reescribir.
//!
//! Todo son funciones puras sobre búferes de sectores; el crate no hace E/S ni
//! reserva memoria, así que sirve igual en el instalador de userspace (con las
//! syscalls `disk_read`/`disk_write`) que en un test del host.
//!
//! El caso de uso es el instalador: `soso-install` clona el USB live sector a
//! sector sobre un NVMe más grande, y el clon hereda una GPT que describe el
//! pendrive — cabecera de respaldo a mitad del disco, última partición del
//! tamaño del USB y los mismos GUID que el original, que dejarían al firmware
//! sin forma de distinguir los dos discos. [`relayout`] y [`reseed_guids`]
//! arreglan justo eso.

#![cfg_attr(not(test), no_std)]

use core::fmt;

pub const SECTOR: usize = 512;
pub const SIGNATURE: &[u8; 8] = b"EFI PART";
/// Tamaño mínimo de cabecera que exige la UEFI spec (2.8, tabla 5-5).
pub const MIN_HEADER_SIZE: u32 = 92;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// El sector 1 no empieza por `EFI PART`.
    BadSignature,
    /// Cabecera con campos imposibles (tamaño, número de entradas…).
    BadHeader,
    /// El búfer de entradas no cubre `num_entries * entry_size`.
    ShortEntries,
    /// El disco destino no da para la tabla más las particiones que ya hay.
    TooSmall,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Error::BadSignature => "sin firma EFI PART",
            Error::BadHeader => "cabecera GPT inválida",
            Error::ShortEntries => "búfer de entradas incompleto",
            Error::TooSmall => "disco destino demasiado pequeño",
        };
        f.write_str(s)
    }
}

// ---------------------------------------------------------------- GUID

/// GUID de 16 bytes tal y como está en disco (los tres primeros campos en
/// little-endian, los dos últimos en big-endian: el famoso «mixed endian»).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Guid(pub [u8; 16]);

impl Guid {
    pub const ZERO: Guid = Guid([0u8; 16]);

    pub fn is_zero(&self) -> bool {
        self.0.iter().all(|&b| b == 0)
    }

    /// Parsea la forma canónica `xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx`.
    pub fn parse(s: &str) -> Option<Guid> {
        let b = s.as_bytes();
        if b.len() != 36 || b[8] != b'-' || b[13] != b'-' || b[18] != b'-' || b[23] != b'-' {
            return None;
        }
        let mut nib = [0u8; 32];
        let mut n = 0;
        for &c in b {
            if c == b'-' {
                continue;
            }
            nib[n] = hex_val(c)?;
            n += 1;
        }
        let byte = |i: usize| (nib[i * 2] << 4) | nib[i * 2 + 1];
        let mut g = [0u8; 16];
        // Campos 1-3 al revés, 4-5 tal cual.
        g[0] = byte(3);
        g[1] = byte(2);
        g[2] = byte(1);
        g[3] = byte(0);
        g[4] = byte(5);
        g[5] = byte(4);
        g[6] = byte(7);
        g[7] = byte(6);
        for i in 8..16 {
            g[i] = byte(i);
        }
        Some(Guid(g))
    }
}

fn hex_val(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

impl fmt::Display for Guid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let g = &self.0;
        write!(
            f,
            "{:02X}{:02X}{:02X}{:02X}-{:02X}{:02X}-{:02X}{:02X}-{:02X}{:02X}-",
            g[3], g[2], g[1], g[0], g[5], g[4], g[7], g[6], g[8], g[9]
        )?;
        for &b in &g[10..16] {
            write!(f, "{b:02X}")?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------- CRC32

/// CRC32 IEEE (polinomio reflejado 0xEDB88320). Ojo: GPT usa este, no el
/// crc32c de sosofs.
pub fn crc32_ieee(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

// ---------------------------------------------------------------- Cabecera

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    pub revision: u32,
    pub header_size: u32,
    pub my_lba: u64,
    pub alternate_lba: u64,
    pub first_usable: u64,
    pub last_usable: u64,
    pub disk_guid: Guid,
    pub entries_lba: u64,
    pub num_entries: u32,
    pub entry_size: u32,
}

impl Header {
    pub fn parse(sec: &[u8]) -> Result<Header, Error> {
        if sec.len() < SECTOR {
            return Err(Error::BadHeader);
        }
        if &sec[0..8] != SIGNATURE {
            return Err(Error::BadSignature);
        }
        let header_size = rd32(sec, 12);
        if !(MIN_HEADER_SIZE..=SECTOR as u32).contains(&header_size) {
            return Err(Error::BadHeader);
        }
        let num_entries = rd32(sec, 80);
        let entry_size = rd32(sec, 84);
        if num_entries == 0 || num_entries > 512 || entry_size < 128 || entry_size % 8 != 0 {
            return Err(Error::BadHeader);
        }
        let mut disk_guid = [0u8; 16];
        disk_guid.copy_from_slice(&sec[56..72]);
        Ok(Header {
            revision: rd32(sec, 8),
            header_size,
            my_lba: rd64(sec, 24),
            alternate_lba: rd64(sec, 32),
            first_usable: rd64(sec, 40),
            last_usable: rd64(sec, 48),
            disk_guid: Guid(disk_guid),
            entries_lba: rd64(sec, 72),
            num_entries,
            entry_size,
        })
    }

    /// Bytes que ocupa el array de entradas.
    pub fn entries_bytes(&self) -> usize {
        self.num_entries as usize * self.entry_size as usize
    }

    /// Sectores que ocupa el array de entradas, redondeando hacia arriba.
    pub fn entries_sectors(&self) -> u64 {
        self.entries_bytes().div_ceil(SECTOR) as u64
    }

    /// Serializa a un sector, con los dos CRC ya recalculados.
    pub fn render(&self, entries: &[u8]) -> [u8; SECTOR] {
        let mut sec = [0u8; SECTOR];
        sec[0..8].copy_from_slice(SIGNATURE);
        wr32(&mut sec, 8, self.revision);
        wr32(&mut sec, 12, self.header_size);
        // 16..20: CRC de la cabecera, a cero mientras se calcula.
        wr64(&mut sec, 24, self.my_lba);
        wr64(&mut sec, 32, self.alternate_lba);
        wr64(&mut sec, 40, self.first_usable);
        wr64(&mut sec, 48, self.last_usable);
        sec[56..72].copy_from_slice(&self.disk_guid.0);
        wr64(&mut sec, 72, self.entries_lba);
        wr32(&mut sec, 80, self.num_entries);
        wr32(&mut sec, 84, self.entry_size);
        let n = self.entries_bytes().min(entries.len());
        wr32(&mut sec, 88, crc32_ieee(&entries[..n]));
        let crc = crc32_ieee(&sec[..self.header_size as usize]);
        wr32(&mut sec, 16, crc);
        sec
    }
}

// ---------------------------------------------------------------- Entradas

/// Vista sobre la entrada `i` del array.
pub fn entry<'a>(entries: &'a [u8], hdr: &Header, i: usize) -> Option<&'a [u8]> {
    let sz = hdr.entry_size as usize;
    let off = i.checked_mul(sz)?;
    entries.get(off..off + sz)
}

pub fn set_entry_first_lba(e: &mut [u8], lba: u64) {
    wr64(e, 32, lba);
}

pub fn set_entry_last_lba(e: &mut [u8], lba: u64) {
    wr64(e, 40, lba);
}

/// Entrada mutable `i` del array GPT.
pub fn entry_mut<'a>(entries: &'a mut [u8], hdr: &Header, i: usize) -> Option<&'a mut [u8]> {
    let sz = hdr.entry_size as usize;
    let off = i.checked_mul(sz)?;
    entries.get_mut(off..off + sz)
}

pub fn entry_used(e: &[u8]) -> bool {
    e[0..16].iter().any(|&b| b != 0)
}

pub fn entry_type(e: &[u8]) -> Guid {
    let mut g = [0u8; 16];
    g.copy_from_slice(&e[0..16]);
    Guid(g)
}

pub fn entry_unique(e: &[u8]) -> Guid {
    let mut g = [0u8; 16];
    g.copy_from_slice(&e[16..32]);
    Guid(g)
}

pub fn entry_first_lba(e: &[u8]) -> u64 {
    rd64(e, 32)
}

pub fn entry_last_lba(e: &[u8]) -> u64 {
    rd64(e, 40)
}

/// Nombre de la partición (UTF-16LE, 36 code units) volcado a ASCII; los
/// caracteres fuera de ASCII se sustituyen por `?`. Devuelve cuántos bytes
/// escribió.
pub fn entry_name_ascii(e: &[u8], out: &mut [u8]) -> usize {
    let mut n = 0;
    for i in 0..36 {
        let off = 56 + i * 2;
        if off + 1 >= e.len() || n >= out.len() {
            break;
        }
        let c = u16::from_le_bytes([e[off], e[off + 1]]);
        if c == 0 {
            break;
        }
        out[n] = if (0x20..0x7F).contains(&c) { c as u8 } else { b'?' };
        n += 1;
    }
    n
}

/// Índice de la última entrada usada, o `None` si la tabla está vacía.
pub fn last_used(entries: &[u8], hdr: &Header) -> Option<usize> {
    let mut best: Option<(u64, usize)> = None;
    for i in 0..hdr.num_entries as usize {
        let Some(e) = entry(entries, hdr, i) else {
            break;
        };
        if !entry_used(e) {
            continue;
        }
        let last = entry_last_lba(e);
        if best.is_none_or(|(lba, _)| last >= lba) {
            best = Some((last, i));
        }
    }
    best.map(|(_, i)| i)
}

/// Número de particiones definidas.
pub fn used_count(entries: &[u8], hdr: &Header) -> usize {
    (0..hdr.num_entries as usize)
        .filter(|&i| entry(entries, hdr, i).is_some_and(entry_used))
        .count()
}

// ---------------------------------------------------------------- Relayout

/// Dónde va cada trozo de la tabla en el disco destino.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Plan {
    pub primary_header_lba: u64,
    pub primary_entries_lba: u64,
    pub backup_entries_lba: u64,
    pub backup_header_lba: u64,
    pub entries_sectors: u64,
}

/// Adapta una GPT clonada al tamaño real del disco: recoloca la cabecera de
/// respaldo al final, recalcula `last_usable` y estira la última partición
/// usada hasta llenar el disco.
///
/// No encoge nada: si alguna partición existente no cabe en `disk_sectors`
/// devuelve [`Error::TooSmall`] sin tocar la tabla.
pub fn relayout(hdr: &mut Header, entries: &mut [u8], disk_sectors: u64) -> Result<Plan, Error> {
    if entries.len() < hdr.entries_bytes() {
        return Err(Error::ShortEntries);
    }
    let esec = hdr.entries_sectors();
    // Falta sitio para MBR + cabecera + tabla en los dos extremos.
    if disk_sectors < 2 * esec + 4 {
        return Err(Error::TooSmall);
    }
    let backup_header_lba = disk_sectors - 1;
    let backup_entries_lba = backup_header_lba - esec;
    let last_usable = backup_entries_lba - 1;
    let first_usable = 2 + esec;
    if last_usable <= first_usable {
        return Err(Error::TooSmall);
    }

    for i in 0..hdr.num_entries as usize {
        let Some(e) = entry(entries, hdr, i) else {
            break;
        };
        if entry_used(e) && (entry_last_lba(e) > last_usable || entry_first_lba(e) < first_usable) {
            return Err(Error::TooSmall);
        }
    }

    if let Some(i) = last_used(entries, hdr)
        && let Some(e) = entry_mut(entries, hdr, i)
        && entry_last_lba(e) < last_usable
    {
        wr64(e, 40, last_usable);
    }

    hdr.my_lba = 1;
    hdr.alternate_lba = backup_header_lba;
    hdr.entries_lba = 2;
    hdr.first_usable = first_usable;
    hdr.last_usable = last_usable;

    Ok(Plan {
        primary_header_lba: 1,
        primary_entries_lba: 2,
        backup_entries_lba,
        backup_header_lba,
        entries_sectors: esec,
    })
}

/// Sector de la cabecera primaria.
pub fn render_primary(hdr: &Header, entries: &[u8], plan: &Plan) -> [u8; SECTOR] {
    let mut h = *hdr;
    h.my_lba = plan.primary_header_lba;
    h.alternate_lba = plan.backup_header_lba;
    h.entries_lba = plan.primary_entries_lba;
    h.render(entries)
}

/// Sector de la cabecera de respaldo (la del final del disco).
pub fn render_backup(hdr: &Header, entries: &[u8], plan: &Plan) -> [u8; SECTOR] {
    let mut h = *hdr;
    h.my_lba = plan.backup_header_lba;
    h.alternate_lba = plan.primary_header_lba;
    h.entries_lba = plan.backup_entries_lba;
    h.render(entries)
}

/// GUID nuevo para el disco y para cada partición usada. Sin esto el destino
/// sería indistinguible del USB del que se clonó y el device path UEFI podría
/// resolver al pendrive.
pub fn reseed_guids(hdr: &mut Header, entries: &mut [u8], rng: &mut Rng) {
    hdr.disk_guid = rng.guid();
    for i in 0..hdr.num_entries as usize {
        let Some(e) = entry_mut(entries, hdr, i) else {
            break;
        };
        if entry_used(e) {
            let g = rng.guid();
            e[16..32].copy_from_slice(&g.0);
        }
    }
}

/// Ajusta el MBR protector al tamaño del disco (campo de 32 bits, saturado a
/// 0xFFFFFFFF en discos de más de 2 TiB, como hace sgdisk).
pub fn protective_mbr_fix(mbr: &mut [u8; SECTOR], disk_sectors: u64) {
    let size = (disk_sectors - 1).min(0xFFFF_FFFF) as u32;
    mbr[446] = 0x00; // no arrancable
    mbr[446 + 4] = 0xEE; // tipo: GPT protective
    wr32(mbr, 446 + 8, 1); // primer LBA
    wr32(mbr, 446 + 12, size);
    mbr[510] = 0x55;
    mbr[511] = 0xAA;
}

// ---------------------------------------------------------------- Tipos

const T_ESP: &str = "C12A7328-F81F-11D2-BA4B-00A0C93EC93B";
const T_LINUX: &str = "0FC63DAF-8483-4772-8E79-3D69D8477DE4";
const T_SWAP: &str = "0657FD6D-A4AB-43C4-84E5-0933C84B4F4F";
const T_LVM: &str = "E6D6D379-F507-44C2-A23C-238F2A3DF928";
const T_RAID: &str = "A19D880F-05FC-4D3B-A006-743F0F84911E";
const T_MSDATA: &str = "EBD0A0A2-B9E5-4433-87C0-68B6B72699C7";
const T_MSRESERVED: &str = "E3C9E316-0B5C-4DB8-817D-F92DF00215AE";
const T_WINRE: &str = "DE94BBA4-06D1-4D40-A16A-BFD50179D6AC";
const T_LINUX_HOME: &str = "933AC7E1-2EB4-4F13-B844-0E14E2AEF915";
const T_LINUX_ROOT_X64: &str = "4F68BCE3-E8CD-4DB1-96E7-FBCAF984B709";
const T_BIOSBOOT: &str = "21686148-6449-6E6F-744E-656564454649";

/// Etiqueta legible del tipo de partición, para que el instalador pueda
/// enseñar qué hay en un disco antes de borrarlo.
pub fn type_label(t: &Guid) -> &'static str {
    let known = [
        (T_ESP, "ESP"),
        (T_LINUX, "linux"),
        (T_LINUX_ROOT_X64, "linux-root"),
        (T_LINUX_HOME, "linux-home"),
        (T_SWAP, "swap"),
        (T_LVM, "lvm"),
        (T_RAID, "raid"),
        (T_MSDATA, "windows"),
        (T_MSRESERVED, "ms-reserved"),
        (T_WINRE, "windows-recovery"),
        (T_BIOSBOOT, "bios-boot"),
    ];
    for (guid, label) in known {
        if Guid::parse(guid).is_some_and(|g| g == *t) {
            return label;
        }
    }
    "desconocido"
}

/// ¿Es esta partición de otro sistema operativo? El live de soso usa `linux`
/// (0x8300) para sus particiones 2 y 3, así que ese tipo por sí solo no
/// delata a nadie: quien decide es el contenido (magic sosofs) y esta función
/// solo marca lo que es inequívocamente ajeno.
pub fn is_foreign(t: &Guid) -> bool {
    matches!(
        type_label(t),
        "swap" | "lvm" | "raid" | "windows" | "ms-reserved" | "windows-recovery"
            | "linux-root" | "linux-home" | "bios-boot"
    )
}

// ---------------------------------------------------------------- RNG

/// xorshift128+ para generar GUID. No hace falta calidad criptográfica: solo
/// que el destino no repita los GUID del USB del que se clonó.
pub struct Rng {
    s0: u64,
    s1: u64,
}

impl Rng {
    pub fn new(seed: u64) -> Rng {
        // splitmix64 para que una semilla pobre (uptime en ms) no arranque
        // con un estado casi todo ceros.
        let mut z = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut next = || {
            z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut x = z;
            x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            x ^ (x >> 31)
        };
        Rng {
            s0: next(),
            s1: next(),
        }
    }

    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.s0;
        let y = self.s1;
        self.s0 = y;
        x ^= x << 23;
        self.s1 = x ^ y ^ (x >> 17) ^ (y >> 26);
        self.s1.wrapping_add(y)
    }

    /// GUID aleatorio con los bits de versión 4 y variante RFC 4122.
    pub fn guid(&mut self) -> Guid {
        let mut g = [0u8; 16];
        g[0..8].copy_from_slice(&self.next_u64().to_le_bytes());
        g[8..16].copy_from_slice(&self.next_u64().to_le_bytes());
        // time_hi_and_version va en little-endian dentro del GUID.
        g[7] = (g[7] & 0x0F) | 0x40;
        g[8] = (g[8] & 0x3F) | 0x80;
        Guid(g)
    }
}

// ---------------------------------------------------------------- Utilidades

fn rd32(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}

fn rd64(b: &[u8], off: usize) -> u64 {
    let mut v = [0u8; 8];
    v.copy_from_slice(&b[off..off + 8]);
    u64::from_le_bytes(v)
}

fn wr32(b: &mut [u8], off: usize, v: u32) {
    b[off..off + 4].copy_from_slice(&v.to_le_bytes());
}

fn wr64(b: &mut [u8], off: usize, v: u64) {
    b[off..off + 8].copy_from_slice(&v.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    const NUM: u32 = 128;
    const ESZ: u32 = 128;
    /// USB de 64 MiB: lo que el clon hereda.
    const USB_SECTORS: u64 = 64 * 1024 * 1024 / 512;

    fn base_header() -> Header {
        Header {
            revision: 0x0001_0000,
            header_size: 92,
            my_lba: 1,
            alternate_lba: USB_SECTORS - 1,
            first_usable: 34,
            last_usable: USB_SECTORS - 34,
            disk_guid: Guid::parse("12345678-1234-5678-1234-567812345678").unwrap(),
            entries_lba: 2,
            num_entries: NUM,
            entry_size: ESZ,
        }
    }

    /// Tabla del live: p1 ESP, p2 sosofs, p3 modelos hasta el final del USB.
    fn base_entries() -> Vec<u8> {
        let mut v = vec![0u8; (NUM * ESZ) as usize];
        let parts = [
            (T_ESP, 2048u64, 2048 + 45055, "EFI system partition"),
            (T_LINUX, 47104, 47104 + 65535, "sosofs"),
            (T_LINUX, 112640, USB_SECTORS - 34, "modelos"),
        ];
        for (i, (ty, first, last, name)) in parts.iter().enumerate() {
            let off = i * ESZ as usize;
            let e = &mut v[off..off + ESZ as usize];
            e[0..16].copy_from_slice(&Guid::parse(ty).unwrap().0);
            let uniq = Guid::parse("aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeee0").unwrap();
            e[16..32].copy_from_slice(&uniq.0);
            wr64(e, 32, *first);
            wr64(e, 40, *last);
            for (j, c) in name.encode_utf16().enumerate() {
                e[56 + j * 2..56 + j * 2 + 2].copy_from_slice(&c.to_le_bytes());
            }
        }
        v
    }

    #[test]
    fn crc32_vector_conocido() {
        assert_eq!(crc32_ieee(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32_ieee(b""), 0);
    }

    #[test]
    fn guid_ida_y_vuelta() {
        let s = "C12A7328-F81F-11D2-BA4B-00A0C93EC93B";
        let g = Guid::parse(s).unwrap();
        // Mixed endian: el primer byte en disco es el último del primer campo.
        assert_eq!(g.0[0], 0x28);
        assert_eq!(g.0[3], 0xC1);
        assert_eq!(g.0[15], 0x3B);
        assert_eq!(format!("{g}"), s);
        assert!(Guid::parse("no-es-un-guid").is_none());
    }

    #[test]
    fn cabecera_ida_y_vuelta() {
        let hdr = base_header();
        let entries = base_entries();
        let sec = hdr.render(&entries);
        let back = Header::parse(&sec).unwrap();
        assert_eq!(hdr, back);
        // El CRC de la cabecera cuadra con lo que hay escrito.
        let mut check = sec;
        let stored = rd32(&check, 16);
        wr32(&mut check, 16, 0);
        assert_eq!(crc32_ieee(&check[..92]), stored);
        assert_eq!(rd32(&sec, 88), crc32_ieee(&entries[..(NUM * ESZ) as usize]));
    }

    #[test]
    fn cabecera_rechaza_basura() {
        let mut sec = [0u8; SECTOR];
        assert_eq!(Header::parse(&sec), Err(Error::BadSignature));
        sec[0..8].copy_from_slice(SIGNATURE);
        assert_eq!(Header::parse(&sec), Err(Error::BadHeader));
    }

    #[test]
    fn relayout_estira_la_ultima_particion() {
        let mut hdr = base_header();
        let mut entries = base_entries();
        let disco = 40u64 * 1024 * 1024 * 1024 / 512; // 40 GiB

        let plan = relayout(&mut hdr, &mut entries, disco).unwrap();

        assert_eq!(plan.backup_header_lba, disco - 1);
        assert_eq!(plan.entries_sectors, 32);
        assert_eq!(plan.backup_entries_lba, disco - 33);
        assert_eq!(hdr.last_usable, disco - 34);
        assert_eq!(hdr.alternate_lba, disco - 1);
        assert_eq!(hdr.first_usable, 34);

        let p3 = entry(&entries, &hdr, 2).unwrap();
        assert_eq!(entry_last_lba(p3), disco - 34);
        // Las otras dos no se mueven.
        let p1 = entry(&entries, &hdr, 0).unwrap();
        assert_eq!(entry_first_lba(p1), 2048);
        assert_eq!(entry_last_lba(p1), 2048 + 45055);
    }

    #[test]
    fn primaria_y_respaldo_son_coherentes() {
        let mut hdr = base_header();
        let mut entries = base_entries();
        let disco = 40u64 * 1024 * 1024 * 1024 / 512;
        let plan = relayout(&mut hdr, &mut entries, disco).unwrap();

        let prim = Header::parse(&render_primary(&hdr, &entries, &plan)).unwrap();
        let back = Header::parse(&render_backup(&hdr, &entries, &plan)).unwrap();

        assert_eq!(prim.my_lba, 1);
        assert_eq!(prim.alternate_lba, back.my_lba);
        assert_eq!(back.alternate_lba, prim.my_lba);
        assert_eq!(back.my_lba, disco - 1);
        assert_eq!(prim.entries_lba, 2);
        assert_eq!(back.entries_lba, disco - 33);
        // Mismo contenido lógico salvo la ubicación.
        assert_eq!(prim.last_usable, back.last_usable);
        assert_eq!(prim.disk_guid, back.disk_guid);
    }

    #[test]
    fn relayout_rechaza_disco_pequeno() {
        let mut hdr = base_header();
        let mut entries = base_entries();
        // Justo un sector menos de lo que ocupa la última partición.
        let disco = USB_SECTORS - 1;
        assert_eq!(
            relayout(&mut hdr, &mut entries, disco),
            Err(Error::TooSmall)
        );
        // Y no ha tocado la tabla.
        let p3 = entry(&entries, &hdr, 2).unwrap();
        assert_eq!(entry_last_lba(p3), USB_SECTORS - 34);
    }

    #[test]
    fn reseed_cambia_todos_los_guid() {
        let mut hdr = base_header();
        let mut entries = base_entries();
        let antes_disco = hdr.disk_guid;
        let antes: Vec<Guid> = (0..3)
            .map(|i| entry_unique(entry(&entries, &hdr, i).unwrap()))
            .collect();

        let mut rng = Rng::new(0xDEAD_BEEF);
        reseed_guids(&mut hdr, &mut entries, &mut rng);

        assert_ne!(hdr.disk_guid, antes_disco);
        let despues: Vec<Guid> = (0..3)
            .map(|i| entry_unique(entry(&entries, &hdr, i).unwrap()))
            .collect();
        for (a, d) in antes.iter().zip(&despues) {
            assert_ne!(a, d);
        }
        // Y son distintos entre sí (antes los tres compartían GUID).
        assert_ne!(despues[0], despues[1]);
        assert_ne!(despues[1], despues[2]);
        // Versión 4, variante RFC 4122.
        for g in &despues {
            assert_eq!(g.0[7] & 0xF0, 0x40);
            assert_eq!(g.0[8] & 0xC0, 0x80);
        }
        // Las entradas vacías siguen vacías.
        assert!(!entry_used(entry(&entries, &hdr, 3).unwrap()));
    }

    #[test]
    fn mbr_protector_al_tamano_del_disco() {
        let mut mbr = [0u8; SECTOR];
        protective_mbr_fix(&mut mbr, 1000);
        assert_eq!(mbr[450], 0xEE);
        assert_eq!(rd32(&mbr, 454), 1);
        assert_eq!(rd32(&mbr, 458), 999);
        assert_eq!(&mbr[510..512], &[0x55, 0xAA]);

        // Disco de 4 TiB: el campo satura.
        protective_mbr_fix(&mut mbr, 8 * 1024 * 1024 * 1024);
        assert_eq!(rd32(&mbr, 458), 0xFFFF_FFFF);
    }

    #[test]
    fn etiquetas_y_ajenos() {
        let esp = Guid::parse(T_ESP).unwrap();
        let swap = Guid::parse(T_SWAP).unwrap();
        let linux = Guid::parse(T_LINUX).unwrap();
        assert_eq!(type_label(&esp), "ESP");
        assert_eq!(type_label(&swap), "swap");
        assert_eq!(type_label(&Guid::ZERO), "desconocido");
        assert!(is_foreign(&swap));
        // Las particiones del propio live son 0x8300: no cuentan como ajenas.
        assert!(!is_foreign(&linux));
        assert!(!is_foreign(&esp));
    }

    #[test]
    fn nombres_y_recuento() {
        let hdr = base_header();
        let entries = base_entries();
        assert_eq!(used_count(&entries, &hdr), 3);
        assert_eq!(last_used(&entries, &hdr), Some(2));
        let mut buf = [0u8; 36];
        let n = entry_name_ascii(entry(&entries, &hdr, 1).unwrap(), &mut buf);
        assert_eq!(&buf[..n], b"sosofs");
    }

    #[test]
    fn relayout_estira_la_de_mayor_lba_no_el_indice() {
        // Live con SOSOINSTALL (p4) entre rootfs y modelos: hay que estirar p3.
        let mut hdr = base_header();
        let mut entries = vec![0u8; (NUM * ESZ) as usize];
        let parts = [
            (T_ESP, 2048u64, 2048 + 45055, "EFI"),
            (T_LINUX, 47104, 47104 + 20479, "sosofs"),
            (T_LINUX, 69632, USB_SECTORS - 34, "modelos"),
            (T_MSDATA, 67584, 69631, "SOSOINSTALL"),
        ];
        for (i, (ty, first, last, name)) in parts.iter().enumerate() {
            let off = i * ESZ as usize;
            let e = &mut entries[off..off + ESZ as usize];
            e[0..16].copy_from_slice(&Guid::parse(ty).unwrap().0);
            e[16..32].copy_from_slice(&Guid::parse("aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeee0").unwrap().0);
            wr64(e, 32, *first);
            wr64(e, 40, *last);
            for (j, c) in name.encode_utf16().enumerate() {
                e[56 + j * 2..56 + j * 2 + 2].copy_from_slice(&c.to_le_bytes());
            }
        }
        assert_eq!(last_used(&entries, &hdr), Some(2));
        let disco = 40u64 * 1024 * 1024 * 1024 / 512;
        relayout(&mut hdr, &mut entries, disco).unwrap();
        let p3 = entry(&entries, &hdr, 2).unwrap();
        assert_eq!(entry_last_lba(p3), disco - 34);
        let p4 = entry(&entries, &hdr, 3).unwrap();
        assert_eq!(entry_first_lba(p4), 67584);
        assert_eq!(entry_last_lba(p4), 69631);
    }
}
