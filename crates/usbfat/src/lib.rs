//! FAT12/16/32 con nombres largos (VFAT), para montar pendrives USB de datos.
//!
//! Lo justo para que un pendrive formateado en otro sistema se pueda leer y
//! escribir desde soso: buscar el volumen (superfloppy, MBR o GPT), recorrer
//! directorios, leer ficheros por rangos, crearlos o reemplazarlos enteros,
//! añadir al final, truncar, crear y borrar directorios y renombrar.
//!
//! Cada operación que escribe deja la FAT y los directorios en disco antes de
//! volver (`flush`): un pendrive se desenchufa sin avisar, y entre dos
//! operaciones el volumen tiene que estar entero.
//!
//! Los nodos se identifican por la posición de su entrada corta: cluster del
//! directorio que la contiene + índice de la entrada. No hay inodos en FAT y
//! esa posición no cambia mientras el fichero exista (reemplazar el
//! contenido reutiliza la entrada), que es lo que necesita un descriptor
//! abierto. exFAT y NTFS se reconocen para poder decir qué son, nada más.

#![cfg_attr(not(test), no_std)]

extern crate alloc;

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

pub const SECTOR: usize = 512;
const ENTRADA: usize = 32;
const BORRADA: u8 = 0xE5;
const ATTR_RO: u8 = 0x01;
const ATTR_VOLUMEN: u8 = 0x08;
const ATTR_DIR: u8 = 0x10;
const ATTR_ARCHIVO: u8 = 0x20;
const ATTR_LFN: u8 = 0x0F;
/// Bits de minúsculas de Windows NT en el byte 12 de la entrada corta.
const NT_BASE_MIN: u8 = 0x08;
const NT_EXT_MIN: u8 = 0x10;
/// Sectores que se leen o escriben de una vez en los caminos de datos.
const TROZO_SECTORES: usize = 128;
/// Sectores de FAT en caché.
const CACHE_FAT: usize = 16;

/// Acceso por sectores de 512 bytes al disco entero (LBA absolutos).
pub trait Disco {
    /// Lee `buf.len() / 512` sectores desde `lba`.
    fn leer(&mut self, lba: u64, buf: &mut [u8]) -> bool;
    /// Escribe `buf.len() / 512` sectores desde `lba`.
    fn escribir(&mut self, lba: u64, buf: &[u8]) -> bool;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    Io,
    NoEncontrado,
    Existe,
    NoEsDir,
    EsDir,
    NoVacio,
    SinEspacio,
    /// La raíz de FAT12/16 tiene un número fijo de entradas.
    DirLleno,
    NombreInvalido,
    Corrupto,
    NoSoportado,
}

pub type Res<T> = Result<T, Error>;

/// Qué hay en un disco.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hallazgo {
    /// Volumen FAT que empieza en `inicio` y mide `sectores`.
    Fat { inicio: u64, sectores: u64 },
    /// Un sistema de ficheros que se reconoce pero no se sabe montar.
    Otro(&'static str),
    Nada,
}

fn u16le(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}
fn u32le(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}
fn u64le(b: &[u8], o: usize) -> u64 {
    let mut a = [0u8; 8];
    a.copy_from_slice(&b[o..o + 8]);
    u64::from_le_bytes(a)
}

/// ¿Es este sector el BPB de un volumen FAT que sabemos montar?
fn es_bpb_fat(s: &[u8]) -> bool {
    if s.len() < SECTOR || s[510] != 0x55 || s[511] != 0xAA {
        return false;
    }
    if s[0] != 0xEB && s[0] != 0xE9 {
        return false;
    }
    let bps = u16le(s, 11);
    let spc = s[13];
    let reservados = u16le(s, 14);
    let nfats = s[16];
    bps == 512 && spc != 0 && spc.is_power_of_two() && reservados != 0 && (1..=2).contains(&nfats)
}

fn otro_fs(s: &[u8]) -> Option<&'static str> {
    match &s[3..11] {
        b"EXFAT   " => Some("exFAT"),
        b"NTFS    " => Some("NTFS"),
        _ => None,
    }
}

/// Busca el primer volumen FAT del disco: superfloppy, MBR o GPT.
pub fn buscar_volumen<D: Disco>(d: &mut D, total: u64) -> Hallazgo {
    let mut s = [0u8; SECTOR];
    if !d.leer(0, &mut s) {
        return Hallazgo::Nada;
    }
    if es_bpb_fat(&s) {
        return Hallazgo::Fat {
            inicio: 0,
            sectores: total,
        };
    }
    if let Some(n) = otro_fs(&s) {
        return Hallazgo::Otro(n);
    }
    if s[510] != 0x55 || s[511] != 0xAA {
        return Hallazgo::Nada;
    }
    let mut candidatos: Vec<(u64, u64)> = Vec::new();
    let mut gpt = false;
    for i in 0..4 {
        let e = &s[446 + i * 16..446 + (i + 1) * 16];
        let tipo = e[4];
        let inicio = u32le(e, 8) as u64;
        let n = u32le(e, 12) as u64;
        if tipo == 0xEE {
            gpt = true;
        } else if tipo != 0 && n != 0 {
            candidatos.push((inicio, n));
        }
    }
    if gpt {
        candidatos.clear();
        let mut h = [0u8; SECTOR];
        if d.leer(1, &mut h) && &h[0..8] == b"EFI PART" {
            let lba = u64le(&h, 72);
            let num = u32le(&h, 80).min(256) as u64;
            let tam = u32le(&h, 84) as u64;
            if tam >= 128 && tam <= SECTOR as u64 {
                let por_sector = SECTOR as u64 / tam;
                let mut sec = [0u8; SECTOR];
                for k in 0..num {
                    if k % por_sector == 0 && !d.leer(lba + k / por_sector, &mut sec) {
                        break;
                    }
                    let o = ((k % por_sector) * tam) as usize;
                    let e = &sec[o..o + tam as usize];
                    if e[0..16].iter().all(|&b| b == 0) {
                        continue;
                    }
                    let primero = u64le(e, 32);
                    let ultimo = u64le(e, 40);
                    if ultimo >= primero {
                        candidatos.push((primero, ultimo - primero + 1));
                    }
                }
            }
        }
    }
    let mut otro = None;
    for (inicio, n) in candidatos {
        if inicio >= total {
            continue;
        }
        let mut b = [0u8; SECTOR];
        if !d.leer(inicio, &mut b) {
            continue;
        }
        if es_bpb_fat(&b) {
            return Hallazgo::Fat {
                inicio,
                sectores: n,
            };
        }
        if otro.is_none() {
            otro = otro_fs(&b);
        }
    }
    match otro {
        Some(n) => Hallazgo::Otro(n),
        None => Hallazgo::Nada,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tipo {
    Fat12,
    Fat16,
    Fat32,
}

/// Un nodo: la entrada corta de un fichero o directorio.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Nodo {
    /// Primer cluster del directorio que la contiene (0 = raíz).
    pub dir: u32,
    /// Índice de la entrada corta en ese directorio.
    pub indice: u32,
}

/// La raíz del volumen.
pub const RAIZ: Nodo = Nodo {
    dir: 0,
    indice: 0xFFFF,
};

impl Nodo {
    /// 48 bits: cluster del directorio y 16 de índice.
    pub fn codificar(self) -> u64 {
        ((self.dir as u64) << 16) | (self.indice as u64 & 0xFFFF)
    }

    pub fn decodificar(v: u64) -> Self {
        Self {
            dir: (v >> 16) as u32,
            indice: (v & 0xFFFF) as u32,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Info {
    pub es_dir: bool,
    pub tam: u64,
    /// Segundos Unix (la FAT guarda hora local sin zona: se toma como UTC).
    pub mtime: u64,
    pub solo_lectura: bool,
}

/// Una entrada ya interpretada.
#[derive(Debug, Clone)]
struct Entrada {
    /// Índice de la entrada corta.
    indice: u32,
    /// Índice de la primera entrada (la primera LFN, o la corta si no hay).
    primero: u32,
    nombre: String,
    corto: [u8; 11],
    rec: [u8; ENTRADA],
}

impl Entrada {
    fn attr(&self) -> u8 {
        self.rec[11]
    }
    fn es_dir(&self) -> bool {
        self.attr() & ATTR_DIR != 0
    }
}

#[derive(Clone, Copy)]
enum Dir {
    /// Raíz de FAT12/16: zona fija tras las FAT.
    Raiz16,
    Cadena(u32),
}

struct SectorFat {
    /// Sector dentro de la (primera) FAT.
    n: u32,
    datos: [u8; SECTOR],
    sucio: bool,
    uso: u64,
}

pub struct Fat<D: Disco> {
    d: D,
    tipo: Tipo,
    spc: u32,
    fat_lba: u64,
    fat_sectores: u32,
    nfats: u32,
    raiz_lba: u64,
    raiz_entradas: u32,
    datos_lba: u64,
    n_clusters: u32,
    raiz_cluster: u32,
    fsinfo_lba: Option<u64>,
    fsinfo_invalidado: bool,
    cache: Vec<SectorFat>,
    reloj: u64,
    siguiente_libre: u32,
    etiqueta: String,
}

// ---------- tiempo ----------

fn dias_desde_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

fn civil_desde_dias(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Segundos Unix → (fecha, hora) FAT. Antes de 1980 no cabe: 1980-01-01.
pub fn a_fat(secs: u64) -> (u16, u16) {
    let dias = (secs / 86400) as i64;
    let (y, m, d) = civil_desde_dias(dias);
    if y < 1980 {
        return ((1 << 5) | 1, 0);
    }
    let y = (y - 1980).min(127) as u16;
    let resto = secs % 86400;
    let (h, mi, s) = (resto / 3600, (resto / 60) % 60, resto % 60);
    (
        (y << 9) | ((m as u16) << 5) | d as u16,
        ((h as u16) << 11) | ((mi as u16) << 5) | (s as u16 / 2),
    )
}

pub fn desde_fat(fecha: u16, hora: u16) -> u64 {
    if fecha == 0 {
        return 0;
    }
    let y = 1980 + (fecha >> 9) as i64;
    let m = ((fecha >> 5) & 0xF).clamp(1, 12) as u32;
    let d = (fecha & 0x1F).clamp(1, 31) as u32;
    let dias = dias_desde_civil(y, m, d);
    if dias < 0 {
        return 0;
    }
    let h = (hora >> 11) as u64;
    let mi = ((hora >> 5) & 0x3F) as u64;
    let s = ((hora & 0x1F) * 2) as u64;
    dias as u64 * 86400 + h * 3600 + mi * 60 + s
}

// ---------- nombres ----------

fn checksum(corto: &[u8; 11]) -> u8 {
    let mut s: u8 = 0;
    for &c in corto {
        s = (if s & 1 != 0 { 0x80u8 } else { 0 })
            .wrapping_add(s >> 1)
            .wrapping_add(c);
    }
    s
}

fn caracter_corto_valido(c: u8) -> bool {
    c.is_ascii_uppercase()
        || c.is_ascii_digit()
        || b"!#$%&'()-@^_`{}~".contains(&c)
        || c >= 0x80
}

fn nombre_valido(n: &str) -> bool {
    if n.is_empty() || n == "." || n == ".." || n.encode_utf16().count() > 255 {
        return false;
    }
    if n.ends_with('.') || n.ends_with(' ') {
        return false;
    }
    !n.chars()
        .any(|c| (c as u32) < 0x20 || "\"*/:<>?\\|".contains(c))
}

/// Nombre corto para mostrar (con los bits de minúsculas de NT).
fn mostrar_corto(c: &[u8; 11], nt: u8) -> String {
    let mut base: Vec<u8> = c[..8].to_vec();
    if base[0] == 0x05 {
        base[0] = 0xE5;
    }
    while base.last() == Some(&b' ') {
        base.pop();
    }
    let mut ext: Vec<u8> = c[8..].to_vec();
    while ext.last() == Some(&b' ') {
        ext.pop();
    }
    if nt & NT_BASE_MIN != 0 {
        base.make_ascii_lowercase();
    }
    if nt & NT_EXT_MIN != 0 {
        ext.make_ascii_lowercase();
    }
    let mut s: String = base.iter().map(|&b| latin(b)).collect();
    if !ext.is_empty() {
        s.push('.');
        s.extend(ext.iter().map(|&b| latin(b)));
    }
    s
}

/// Los bytes ≥ 0x80 de un nombre corto dependen de la página de códigos de
/// quien lo escribió; se muestran como Latin-1, que al menos no se pierde.
fn latin(b: u8) -> char {
    b as char
}

/// ¿Cabe tal cual en 8.3? Devuelve el nombre corto y los bits NT si las
/// minúsculas son uniformes en cada parte.
fn como_8_3(n: &str) -> Option<([u8; 11], u8)> {
    if !n.is_ascii() {
        return None;
    }
    let (base, ext) = match n.rfind('.') {
        Some(0) => return None,
        Some(p) => (&n[..p], &n[p + 1..]),
        None => (n, ""),
    };
    if base.is_empty() || base.len() > 8 || ext.len() > 3 || base.contains('.') {
        return None;
    }
    let mut nt = 0u8;
    let parte = |p: &str, bit: u8, nt: &mut u8| -> bool {
        let hay_min = p.bytes().any(|b| b.is_ascii_lowercase());
        let hay_may = p.bytes().any(|b| b.is_ascii_uppercase());
        if hay_min && hay_may {
            return false;
        }
        if hay_min {
            *nt |= bit;
        }
        true
    };
    if !parte(base, NT_BASE_MIN, &mut nt) || !parte(ext, NT_EXT_MIN, &mut nt) {
        return None;
    }
    let mut c = [b' '; 11];
    for (i, b) in base.bytes().enumerate() {
        c[i] = b.to_ascii_uppercase();
    }
    for (i, b) in ext.bytes().enumerate() {
        c[8 + i] = b.to_ascii_uppercase();
    }
    if !c.iter().all(|&b| b == b' ' || caracter_corto_valido(b)) || c[0] == b' ' {
        return None;
    }
    if c[0] == 0xE5 {
        c[0] = 0x05;
    }
    Some((c, nt))
}

/// Nombre corto «BASE~N.EXT» único en el directorio (base 6 + cola numérica).
fn generar_corto(n: &str, existentes: &[[u8; 11]]) -> Res<[u8; 11]> {
    let limpio = |s: &str| -> Vec<u8> {
        s.chars()
            .filter(|&c| c != ' ' && c != '.')
            .map(|c| {
                let u = c.to_ascii_uppercase();
                if u.is_ascii() && caracter_corto_valido(u as u8) {
                    u as u8
                } else {
                    b'_'
                }
            })
            .collect()
    };
    let sin_puntos_inicio = n.trim_start_matches('.');
    let (base, ext) = match sin_puntos_inicio.rfind('.') {
        Some(p) => (&sin_puntos_inicio[..p], &sin_puntos_inicio[p + 1..]),
        None => (sin_puntos_inicio, ""),
    };
    let mut base = limpio(base);
    let ext = limpio(ext);
    if base.is_empty() {
        base.push(b'_');
    }
    for k in 1u32..1_000_000 {
        let cola = alloc::format!("~{k}");
        let cabe = 8 - cola.len();
        let mut c = [b' '; 11];
        let b = &base[..base.len().min(cabe)];
        c[..b.len()].copy_from_slice(b);
        c[b.len()..b.len() + cola.len()].copy_from_slice(cola.as_bytes());
        for (i, &e) in ext.iter().take(3).enumerate() {
            c[8 + i] = e;
        }
        if !existentes.contains(&c) {
            return Ok(c);
        }
    }
    Err(Error::DirLleno)
}

fn iguales(a: &str, b: &str) -> bool {
    a.chars()
        .flat_map(char::to_lowercase)
        .eq(b.chars().flat_map(char::to_lowercase))
}

impl<D: Disco> Fat<D> {
    /// Monta el volumen que empieza en `inicio`.
    pub fn montar(mut d: D, inicio: u64) -> Res<Self> {
        let mut s = [0u8; SECTOR];
        if !d.leer(inicio, &mut s) {
            return Err(Error::Io);
        }
        if !es_bpb_fat(&s) {
            return Err(Error::NoSoportado);
        }
        let spc = s[13] as u32;
        let reservados = u16le(&s, 14) as u32;
        let nfats = s[16] as u32;
        let raiz_entradas = u16le(&s, 17) as u32;
        let total16 = u16le(&s, 19) as u32;
        let fat16 = u16le(&s, 22) as u32;
        let total32 = u32le(&s, 32);
        let fat32 = u32le(&s, 36);
        let fat_sectores = if fat16 != 0 { fat16 } else { fat32 };
        let total = if total16 != 0 { total16 } else { total32 };
        if fat_sectores == 0 || total == 0 {
            return Err(Error::Corrupto);
        }
        let raiz_sectores = (raiz_entradas * ENTRADA as u32).div_ceil(SECTOR as u32);
        let meta = reservados + nfats * fat_sectores + raiz_sectores;
        if meta >= total {
            return Err(Error::Corrupto);
        }
        let n_clusters = (total - meta) / spc;
        // Microsoft: el tipo sale del número de clusters, no de la etiqueta.
        let tipo = if n_clusters < 4085 {
            Tipo::Fat12
        } else if n_clusters < 65525 {
            Tipo::Fat16
        } else {
            Tipo::Fat32
        };
        let (raiz_cluster, fsinfo_lba, etiqueta_off) = if tipo == Tipo::Fat32 {
            let fsi = u16le(&s, 48) as u64;
            (
                u32le(&s, 44),
                (fsi != 0 && fsi != 0xFFFF).then_some(inicio + fsi),
                71,
            )
        } else {
            (0, None, 43)
        };
        let etiqueta = String::from_utf8_lossy(&s[etiqueta_off..etiqueta_off + 11])
            .trim_end()
            .into();
        let fat_lba = inicio + reservados as u64;
        let raiz_lba = fat_lba + (nfats * fat_sectores) as u64;
        Ok(Self {
            d,
            tipo,
            spc,
            fat_lba,
            fat_sectores,
            nfats,
            raiz_lba,
            raiz_entradas,
            datos_lba: raiz_lba + raiz_sectores as u64,
            n_clusters,
            raiz_cluster,
            fsinfo_lba,
            fsinfo_invalidado: false,
            cache: Vec::new(),
            reloj: 0,
            siguiente_libre: 2,
            etiqueta,
        })
    }

    pub fn tipo(&self) -> Tipo {
        self.tipo
    }

    /// Etiqueta del BPB (la de la raíz, si existe, manda en Windows).
    pub fn etiqueta(&self) -> &str {
        &self.etiqueta
    }

    pub fn disco(&mut self) -> &mut D {
        &mut self.d
    }

    fn bytes_cluster(&self) -> usize {
        self.spc as usize * SECTOR
    }

    fn lba_cluster(&self, c: u32) -> Res<u64> {
        if c < 2 || c >= self.n_clusters + 2 {
            return Err(Error::Corrupto);
        }
        Ok(self.datos_lba + (c - 2) as u64 * self.spc as u64)
    }

    fn leer(&mut self, lba: u64, buf: &mut [u8]) -> Res<()> {
        if self.d.leer(lba, buf) {
            Ok(())
        } else {
            Err(Error::Io)
        }
    }

    fn escribir(&mut self, lba: u64, buf: &[u8]) -> Res<()> {
        if self.d.escribir(lba, buf) {
            Ok(())
        } else {
            Err(Error::Io)
        }
    }

    // ---------- FAT ----------

    fn sector_fat(&mut self, n: u32) -> Res<usize> {
        if n >= self.fat_sectores {
            return Err(Error::Corrupto);
        }
        self.reloj += 1;
        let reloj = self.reloj;
        if let Some(i) = self.cache.iter().position(|s| s.n == n) {
            self.cache[i].uso = reloj;
            return Ok(i);
        }
        if self.cache.len() >= CACHE_FAT {
            let i = (0..self.cache.len())
                .min_by_key(|&i| self.cache[i].uso)
                .unwrap_or(0);
            if self.cache[i].sucio {
                self.volcar_sector(i)?;
            }
            self.cache.swap_remove(i);
        }
        let mut datos = [0u8; SECTOR];
        let lba = self.fat_lba + n as u64;
        self.leer(lba, &mut datos)?;
        self.cache.push(SectorFat {
            n,
            datos,
            sucio: false,
            uso: reloj,
        });
        Ok(self.cache.len() - 1)
    }

    fn volcar_sector(&mut self, i: usize) -> Res<()> {
        let n = self.cache[i].n;
        let datos = self.cache[i].datos;
        for k in 0..self.nfats {
            let lba = self.fat_lba + (k * self.fat_sectores + n) as u64;
            self.escribir(lba, &datos)?;
        }
        self.cache[i].sucio = false;
        Ok(())
    }

    fn fat_byte(&mut self, off: u64) -> Res<u8> {
        let i = self.sector_fat((off / SECTOR as u64) as u32)?;
        Ok(self.cache[i].datos[(off % SECTOR as u64) as usize])
    }

    fn fat_poner_byte(&mut self, off: u64, v: u8) -> Res<()> {
        let i = self.sector_fat((off / SECTOR as u64) as u32)?;
        self.cache[i].datos[(off % SECTOR as u64) as usize] = v;
        self.cache[i].sucio = true;
        Ok(())
    }

    fn entrada_fat(&mut self, c: u32) -> Res<u32> {
        match self.tipo {
            Tipo::Fat12 => {
                let off = c as u64 + c as u64 / 2;
                let v = self.fat_byte(off)? as u32 | (self.fat_byte(off + 1)? as u32) << 8;
                Ok(if c & 1 != 0 { v >> 4 } else { v & 0xFFF })
            }
            Tipo::Fat16 => {
                let off = c as u64 * 2;
                Ok(self.fat_byte(off)? as u32 | (self.fat_byte(off + 1)? as u32) << 8)
            }
            Tipo::Fat32 => {
                let off = c as u64 * 4;
                let mut v = 0u32;
                for k in 0..4 {
                    v |= (self.fat_byte(off + k)? as u32) << (8 * k);
                }
                Ok(v & 0x0FFF_FFFF)
            }
        }
    }

    fn poner_entrada_fat(&mut self, c: u32, v: u32) -> Res<()> {
        match self.tipo {
            Tipo::Fat12 => {
                let off = c as u64 + c as u64 / 2;
                let lo = self.fat_byte(off)? as u32;
                let hi = self.fat_byte(off + 1)? as u32;
                let viejo = lo | hi << 8;
                let nuevo = if c & 1 != 0 {
                    (viejo & 0x000F) | (v & 0xFFF) << 4
                } else {
                    (viejo & 0xF000) | (v & 0xFFF)
                };
                self.fat_poner_byte(off, nuevo as u8)?;
                self.fat_poner_byte(off + 1, (nuevo >> 8) as u8)
            }
            Tipo::Fat16 => {
                let off = c as u64 * 2;
                self.fat_poner_byte(off, v as u8)?;
                self.fat_poner_byte(off + 1, (v >> 8) as u8)
            }
            Tipo::Fat32 => {
                let off = c as u64 * 4;
                let alto = self.fat_byte(off + 3)? as u32 & 0xF0;
                let v = (v & 0x0FFF_FFFF) | alto << 24;
                for k in 0..4 {
                    self.fat_poner_byte(off + k, (v >> (8 * k)) as u8)?;
                }
                Ok(())
            }
        }
    }

    fn fin_de_cadena(&self) -> u32 {
        match self.tipo {
            Tipo::Fat12 => 0xFFF,
            Tipo::Fat16 => 0xFFFF,
            Tipo::Fat32 => 0x0FFF_FFFF,
        }
    }

    fn es_fin(&self, v: u32) -> bool {
        match self.tipo {
            Tipo::Fat12 => v >= 0xFF8,
            Tipo::Fat16 => v >= 0xFFF8,
            Tipo::Fat32 => v >= 0x0FFF_FFF8,
        }
    }

    fn cadena(&mut self, inicio: u32) -> Res<Vec<u32>> {
        let mut v = Vec::new();
        if inicio == 0 {
            return Ok(v);
        }
        let mut c = inicio;
        loop {
            if c < 2 || c >= self.n_clusters + 2 || v.len() > self.n_clusters as usize {
                return Err(Error::Corrupto);
            }
            v.push(c);
            let sig = self.entrada_fat(c)?;
            if self.es_fin(sig) {
                return Ok(v);
            }
            c = sig;
        }
    }

    /// Un cluster libre, ya marcado fin de cadena y enlazado tras `previo`.
    fn asignar(&mut self, previo: Option<u32>) -> Res<u32> {
        let total = self.n_clusters;
        let inicio = self.siguiente_libre.clamp(2, total + 1);
        for k in 0..total {
            let c = 2 + (inicio - 2 + k) % total;
            if self.entrada_fat(c)? == 0 {
                let fin = self.fin_de_cadena();
                self.poner_entrada_fat(c, fin)?;
                if let Some(p) = previo {
                    self.poner_entrada_fat(p, c)?;
                }
                self.siguiente_libre = c + 1;
                return Ok(c);
            }
        }
        Err(Error::SinEspacio)
    }

    fn liberar_desde(&mut self, inicio: u32) -> Res<()> {
        for c in self.cadena(inicio)? {
            self.poner_entrada_fat(c, 0)?;
        }
        Ok(())
    }

    /// Lleva la cadena que empieza en `inicio` a `n` clusters (0 = vacía).
    /// Devuelve el primer cluster (0 si queda vacía) y la cadena.
    fn ajustar_cadena(&mut self, inicio: u32, n: usize) -> Res<(u32, Vec<u32>)> {
        let mut c = self.cadena(inicio)?;
        if c.len() > n {
            if n == 0 {
                self.liberar_desde(c[0])?;
                return Ok((0, Vec::new()));
            }
            let sobra = c[n];
            let fin = self.fin_de_cadena();
            self.poner_entrada_fat(c[n - 1], fin)?;
            self.liberar_desde(sobra)?;
            c.truncate(n);
        }
        while c.len() < n {
            match self.asignar(c.last().copied()) {
                Ok(nuevo) => c.push(nuevo),
                Err(e) => {
                    // No dejar la cadena a medias: se queda como estaba.
                    let original = self.cadena(inicio)?.len();
                    if original == 0 {
                        if let Some(&p) = c.first() {
                            self.liberar_desde(p)?;
                        }
                    } else if c.len() > original {
                        let fin = self.fin_de_cadena();
                        self.poner_entrada_fat(c[original - 1], fin)?;
                        self.liberar_desde(c[original])?;
                    }
                    return Err(e);
                }
            }
        }
        Ok((c.first().copied().unwrap_or(0), c))
    }

    /// Vuelca la FAT y, en FAT32, marca FSInfo como «desconocido» (que es
    /// válido y obliga a recalcular, en vez de dejar un recuento falso).
    pub fn flush(&mut self) -> Res<()> {
        for i in 0..self.cache.len() {
            if self.cache[i].sucio {
                self.volcar_sector(i)?;
            }
        }
        if let (Some(lba), false) = (self.fsinfo_lba, self.fsinfo_invalidado) {
            let mut s = [0u8; SECTOR];
            self.leer(lba, &mut s)?;
            if u32le(&s, 0) == 0x4161_5252 && u32le(&s, 484) == 0x6141_7272 {
                s[488..496].copy_from_slice(&[0xFF; 8]);
                self.escribir(lba, &s)?;
            }
            self.fsinfo_invalidado = true;
        }
        Ok(())
    }

    // ---------- datos ----------

    /// Lee o escribe `len` bytes desde `off` dentro de la cadena `c`.
    fn mover(&mut self, c: &[u32], off: u64, mut leer: Option<&mut [u8]>, escribir: Option<&[u8]>) -> Res<()> {
        let len = leer.as_ref().map(|b| b.len()).or(escribir.map(|b| b.len())).unwrap_or(0);
        let cb = self.bytes_cluster() as u64;
        let mut hecho = 0usize;
        while hecho < len {
            let pos = off + hecho as u64;
            let ci = (pos / cb) as usize;
            if ci >= c.len() {
                return Err(Error::Corrupto);
            }
            // Clusters consecutivos en disco: una sola E/S.
            let mut fin_run = ci + 1;
            while fin_run < c.len() && c[fin_run] == c[fin_run - 1] + 1 {
                fin_run += 1;
            }
            let dentro = pos % cb;
            let disp_run = (fin_run - ci) as u64 * cb - dentro;
            let lba_run = self.lba_cluster(c[ci])?;
            let primer_sector = dentro / SECTOR as u64;
            let desfase = (dentro % SECTOR as u64) as usize;
            let quiero = (len - hecho).min(disp_run as usize).min(TROZO_SECTORES * SECTOR - desfase);
            let n_sec = (desfase + quiero).div_ceil(SECTOR);
            let lba = lba_run + primer_sector;
            let mut tmp = vec![0u8; n_sec * SECTOR];
            if let Some(w) = escribir {
                // Sectores parciales: leer, parchear, escribir.
                if desfase != 0 || (desfase + quiero) % SECTOR != 0 {
                    if n_sec == 1 {
                        self.leer(lba, &mut tmp)?;
                    } else {
                        self.leer(lba, &mut tmp[..SECTOR])?;
                        let u = (n_sec - 1) * SECTOR;
                        self.leer(lba + n_sec as u64 - 1, &mut tmp[u..])?;
                    }
                }
                tmp[desfase..desfase + quiero].copy_from_slice(&w[hecho..hecho + quiero]);
                self.escribir(lba, &tmp)?;
            } else if let Some(r) = leer.as_deref_mut() {
                self.leer(lba, &mut tmp)?;
                r[hecho..hecho + quiero].copy_from_slice(&tmp[desfase..desfase + quiero]);
            }
            hecho += quiero;
        }
        Ok(())
    }

    fn poner_a_cero(&mut self, c: u32) -> Res<()> {
        let lba = self.lba_cluster(c)?;
        let ceros = vec![0u8; self.bytes_cluster()];
        self.escribir(lba, &ceros)
    }

    // ---------- directorios ----------

    fn dir_de(&self, cluster: u32) -> Dir {
        if cluster == 0 {
            if self.tipo == Tipo::Fat32 {
                Dir::Cadena(self.raiz_cluster)
            } else {
                Dir::Raiz16
            }
        } else {
            Dir::Cadena(cluster)
        }
    }

    /// Cluster «normalizado» de un directorio para guardarlo en un nodo:
    /// la raíz siempre es 0, también en FAT32.
    fn normalizar(&self, cluster: u32) -> u32 {
        if self.tipo == Tipo::Fat32 && cluster == self.raiz_cluster {
            0
        } else {
            cluster
        }
    }

    /// Bytes crudos del directorio y la cadena que los contiene.
    fn leer_dir(&mut self, dir: u32) -> Res<(Vec<u8>, Vec<u32>)> {
        match self.dir_de(dir) {
            Dir::Raiz16 => {
                let n = (self.raiz_entradas as usize * ENTRADA).div_ceil(SECTOR);
                let mut b = vec![0u8; n * SECTOR];
                let lba = self.raiz_lba;
                self.leer(lba, &mut b)?;
                Ok((b, Vec::new()))
            }
            Dir::Cadena(c) => {
                let cadena = self.cadena(c)?;
                let cb = self.bytes_cluster();
                let mut b = vec![0u8; cadena.len() * cb];
                self.mover(&cadena, 0, Some(&mut b), None)?;
                Ok((b, cadena))
            }
        }
    }

    /// Escribe entradas consecutivas desde `indice` (un registro de 32 B cada una).
    fn escribir_entradas(&mut self, dir: u32, cadena: &[u32], indice: u32, recs: &[[u8; ENTRADA]]) -> Res<()> {
        let mut plano = Vec::with_capacity(recs.len() * ENTRADA);
        for r in recs {
            plano.extend_from_slice(r);
        }
        let off = indice as u64 * ENTRADA as u64;
        match self.dir_de(dir) {
            Dir::Raiz16 => {
                if off + plano.len() as u64 > self.raiz_entradas as u64 * ENTRADA as u64 {
                    return Err(Error::DirLleno);
                }
                let primer = off / SECTOR as u64;
                let ultimo = (off + plano.len() as u64 - 1) / SECTOR as u64;
                let n = (ultimo - primer + 1) as usize;
                let mut tmp = vec![0u8; n * SECTOR];
                let lba = self.raiz_lba + primer;
                self.leer(lba, &mut tmp)?;
                let d = (off % SECTOR as u64) as usize;
                tmp[d..d + plano.len()].copy_from_slice(&plano);
                self.escribir(lba, &tmp)
            }
            Dir::Cadena(_) => self.mover(cadena, off, None, Some(&plano)),
        }
    }

    fn entradas(&mut self, dir: u32) -> Res<(Vec<Entrada>, Vec<u8>, Vec<u32>)> {
        let (crudo, cadena) = self.leer_dir(dir)?;
        let mut out = Vec::new();
        let mut lfn: Vec<u16> = Vec::new();
        let mut lfn_partes = 0u8;
        let mut lfn_vistas = 0u8;
        let mut lfn_chk = 0u8;
        let mut lfn_primero = 0u32;
        for (i, r) in crudo.chunks_exact(ENTRADA).enumerate() {
            let i = i as u32;
            if r[0] == 0 {
                break;
            }
            if r[0] == BORRADA {
                lfn_partes = 0;
                continue;
            }
            if r[11] & 0x3F == ATTR_LFN {
                let ord = r[0] & 0x1F;
                if r[0] & 0x40 != 0 {
                    lfn_partes = ord;
                    lfn_vistas = 0;
                    lfn_chk = r[13];
                    lfn_primero = i;
                    lfn = vec![0xFFFF; ord as usize * 13];
                } else if lfn_partes == 0 || r[13] != lfn_chk || ord != lfn_partes - lfn_vistas {
                    lfn_partes = 0;
                    continue;
                }
                if ord == 0 || ord as usize * 13 > lfn.len() {
                    lfn_partes = 0;
                    continue;
                }
                let base = (ord as usize - 1) * 13;
                let mut k = 0;
                for o in (1..11).step_by(2).chain((14..26).step_by(2)).chain((28..32).step_by(2)) {
                    lfn[base + k] = u16le(r, o);
                    k += 1;
                }
                lfn_vistas += 1;
                continue;
            }
            let mut rec = [0u8; ENTRADA];
            rec.copy_from_slice(r);
            let mut corto = [0u8; 11];
            corto.copy_from_slice(&r[..11]);
            let largo_ok = lfn_partes != 0 && lfn_vistas == lfn_partes && checksum(&corto) == lfn_chk;
            let (nombre, primero) = if largo_ok {
                let fin = lfn.iter().position(|&u| u == 0 || u == 0xFFFF).unwrap_or(lfn.len());
                (
                    char::decode_utf16(lfn[..fin].iter().copied())
                        .map(|c| c.unwrap_or('\u{FFFD}'))
                        .collect(),
                    lfn_primero,
                )
            } else {
                (mostrar_corto(&corto, r[12]), i)
            };
            lfn_partes = 0;
            if r[11] & ATTR_VOLUMEN != 0 && r[11] & ATTR_DIR == 0 {
                continue;
            }
            if &corto == b".          " || &corto == b"..         " {
                continue;
            }
            out.push(Entrada {
                indice: i,
                primero,
                nombre,
                corto,
                rec,
            });
        }
        Ok((out, crudo, cadena))
    }

    /// Cluster del contenido de un directorio-nodo (0 = raíz).
    fn contenido_dir(&mut self, n: Nodo) -> Res<u32> {
        if n == RAIZ {
            return Ok(0);
        }
        let rec = self.registro(n)?;
        if rec[11] & ATTR_DIR == 0 {
            return Err(Error::NoEsDir);
        }
        Ok(self.normalizar(cluster_de(&rec, self.tipo)))
    }

    /// Entrada corta de un nodo. Si ya no está (borrada), NoEncontrado.
    fn registro(&mut self, n: Nodo) -> Res<[u8; ENTRADA]> {
        let off = n.indice as u64 * ENTRADA as u64;
        let mut rec = [0u8; ENTRADA];
        match self.dir_de(n.dir) {
            Dir::Raiz16 => {
                if n.indice >= self.raiz_entradas {
                    return Err(Error::NoEncontrado);
                }
                let mut s = [0u8; SECTOR];
                let lba = self.raiz_lba + off / SECTOR as u64;
                self.leer(lba, &mut s)?;
                let d = (off % SECTOR as u64) as usize;
                rec.copy_from_slice(&s[d..d + ENTRADA]);
            }
            Dir::Cadena(c) => {
                let cadena = self.cadena(c)?;
                if off >= cadena.len() as u64 * self.bytes_cluster() as u64 {
                    return Err(Error::NoEncontrado);
                }
                self.mover(&cadena, off, Some(&mut rec), None)?;
            }
        }
        if rec[0] == 0 || rec[0] == BORRADA || rec[11] & 0x3F == ATTR_LFN {
            return Err(Error::NoEncontrado);
        }
        Ok(rec)
    }

    fn poner_registro(&mut self, n: Nodo, rec: &[u8; ENTRADA]) -> Res<()> {
        let cadena = match self.dir_de(n.dir) {
            Dir::Raiz16 => Vec::new(),
            Dir::Cadena(c) => self.cadena(c)?,
        };
        self.escribir_entradas(n.dir, &cadena, n.indice, core::slice::from_ref(rec))
    }

    fn buscar(&mut self, dir: u32, nombre: &str) -> Res<Option<Entrada>> {
        let (ents, _, _) = self.entradas(dir)?;
        Ok(ents.into_iter().find(|e| {
            iguales(&e.nombre, nombre) || iguales(&mostrar_corto(&e.corto, 0), nombre)
        }))
    }

    /// Crea una entrada (nombre largo si hace falta) y devuelve su nodo.
    fn crear_entrada(&mut self, dir: u32, nombre: &str, mut rec: [u8; ENTRADA]) -> Res<Nodo> {
        if !nombre_valido(nombre) {
            return Err(Error::NombreInvalido);
        }
        let (ents, crudo, _) = self.entradas(dir)?;
        if ents
            .iter()
            .any(|e| iguales(&e.nombre, nombre) || iguales(&mostrar_corto(&e.corto, 0), nombre))
        {
            return Err(Error::Existe);
        }
        let cortos: Vec<[u8; 11]> = crudo
            .chunks_exact(ENTRADA)
            .take_while(|r| r[0] != 0)
            .filter(|r| r[0] != BORRADA && r[11] & 0x3F != ATTR_LFN)
            .map(|r| {
                let mut c = [0u8; 11];
                c.copy_from_slice(&r[..11]);
                c
            })
            .collect();
        let (corto, nt, largo) = match como_8_3(nombre) {
            Some((c, nt)) if !cortos.contains(&c) => (c, nt, false),
            _ => (generar_corto(nombre, &cortos)?, 0, true),
        };
        rec[..11].copy_from_slice(&corto);
        rec[12] = nt;
        let utf16: Vec<u16> = nombre.encode_utf16().collect();
        let n_lfn = if largo { utf16.len().div_ceil(13) } else { 0 };
        let mut recs: Vec<[u8; ENTRADA]> = Vec::with_capacity(n_lfn + 1);
        let chk = checksum(&corto);
        for ord in (1..=n_lfn).rev() {
            let mut l = [0u8; ENTRADA];
            l[0] = ord as u8 | if ord == n_lfn { 0x40 } else { 0 };
            l[11] = ATTR_LFN;
            l[13] = chk;
            let mut k = 0;
            for o in (1..11).step_by(2).chain((14..26).step_by(2)).chain((28..32).step_by(2)) {
                let p = (ord - 1) * 13 + k;
                let u = match p.cmp(&utf16.len()) {
                    core::cmp::Ordering::Less => utf16[p],
                    core::cmp::Ordering::Equal => 0,
                    core::cmp::Ordering::Greater => 0xFFFF,
                };
                l[o..o + 2].copy_from_slice(&u.to_le_bytes());
                k += 1;
            }
            recs.push(l);
        }
        recs.push(rec);

        // Hueco de `recs.len()` entradas libres seguidas.
        let total = crudo.len() / ENTRADA;
        // Tras la primera entrada 0x00 todo es libre, aunque no esté a cero.
        let fin = crudo
            .chunks_exact(ENTRADA)
            .position(|r| r[0] == 0)
            .unwrap_or(total);
        let mut inicio = None;
        let mut run = 0usize;
        for (i, r) in crudo.chunks_exact(ENTRADA).enumerate() {
            if i >= fin || r[0] == BORRADA {
                run += 1;
                if run == recs.len() {
                    inicio = Some(i + 1 - run);
                    break;
                }
            } else {
                run = 0;
            }
        }
        let (inicio, cadena) = match inicio {
            Some(i) => {
                let cadena = match self.dir_de(dir) {
                    Dir::Raiz16 => Vec::new(),
                    Dir::Cadena(c) => self.cadena(c)?,
                };
                (i, cadena)
            }
            None => {
                let Dir::Cadena(c) = self.dir_de(dir) else {
                    return Err(Error::DirLleno);
                };
                // Crecer el directorio: lo que sobre de la cola libre actual
                // más clusters nuevos a cero.
                let cb = self.bytes_cluster() / ENTRADA;
                let faltan = recs.len() - run;
                let nuevos = faltan.div_ceil(cb);
                let actual = self.cadena(c)?.len();
                let (_, cadena) = self.ajustar_cadena(c, actual + nuevos)?;
                for &nc in &cadena[actual..] {
                    self.poner_a_cero(nc)?;
                }
                (total - run, cadena)
            }
        };
        if inicio + recs.len() - 1 >= 0xFFFF {
            return Err(Error::DirLleno);
        }
        let indice_corto = inicio + recs.len() - 1;
        // Si se escribe en la zona tras el fin, lo que siga debe seguir
        // siendo fin de directorio y no basura que pase por entradas.
        let siguiente = indice_corto + 1;
        if siguiente >= fin && siguiente < total && crudo[siguiente * ENTRADA] != 0 {
            recs.push([0u8; ENTRADA]);
        }
        self.escribir_entradas(dir, &cadena, inicio as u32, &recs)?;
        Ok(Nodo {
            dir,
            indice: indice_corto as u32,
        })
    }

    fn borrar_entradas(&mut self, dir: u32, e: &Entrada) -> Res<()> {
        let cadena = match self.dir_de(dir) {
            Dir::Raiz16 => Vec::new(),
            Dir::Cadena(c) => self.cadena(c)?,
        };
        let n = (e.indice - e.primero + 1) as usize;
        let (crudo, _) = self.leer_dir(dir)?;
        let mut recs = Vec::with_capacity(n);
        for k in 0..n {
            let o = (e.primero as usize + k) * ENTRADA;
            let mut r = [0u8; ENTRADA];
            r.copy_from_slice(&crudo[o..o + ENTRADA]);
            r[0] = BORRADA;
            recs.push(r);
        }
        self.escribir_entradas(dir, &cadena, e.primero, &recs)
    }

    // ---------- API ----------

    pub fn info(&mut self, n: Nodo) -> Res<Info> {
        if n == RAIZ {
            return Ok(Info {
                es_dir: true,
                tam: 0,
                mtime: 0,
                solo_lectura: false,
            });
        }
        let r = self.registro(n)?;
        Ok(info_de(&r))
    }

    pub fn buscar_en(&mut self, dir: Nodo, nombre: &str) -> Res<Nodo> {
        let d = self.contenido_dir(dir)?;
        let e = self.buscar(d, nombre)?.ok_or(Error::NoEncontrado)?;
        Ok(Nodo {
            dir: d,
            indice: e.indice,
        })
    }

    pub fn listar(&mut self, dir: Nodo) -> Res<Vec<(String, Nodo, Info)>> {
        let d = self.contenido_dir(dir)?;
        let (ents, _, _) = self.entradas(d)?;
        Ok(ents
            .into_iter()
            .map(|e| {
                let info = info_de(&e.rec);
                (
                    e.nombre,
                    Nodo {
                        dir: d,
                        indice: e.indice,
                    },
                    info,
                )
            })
            .collect())
    }

    /// Lee desde `off`; devuelve los bytes leídos (menos al final del fichero).
    pub fn leer_fichero(&mut self, n: Nodo, off: u64, buf: &mut [u8]) -> Res<usize> {
        let r = self.registro(n)?;
        if r[11] & ATTR_DIR != 0 {
            return Err(Error::EsDir);
        }
        let tam = u32le(&r, 28) as u64;
        if off >= tam {
            return Ok(0);
        }
        let len = (buf.len() as u64).min(tam - off) as usize;
        let cadena = self.cadena(cluster_de(&r, self.tipo))?;
        self.mover(&cadena, off, Some(&mut buf[..len]), None)?;
        Ok(len)
    }

    /// Crea `nombre` en `dir` con `datos`, o reemplaza su contenido si ya
    /// existe (el nodo no cambia).
    pub fn escribir_fichero(&mut self, dir: Nodo, nombre: &str, datos: &[u8], mtime: u64) -> Res<Nodo> {
        if datos.len() as u64 > u32::MAX as u64 {
            return Err(Error::SinEspacio);
        }
        let d = self.contenido_dir(dir)?;
        let cb = self.bytes_cluster();
        let necesarios = datos.len().div_ceil(cb);
        let (fecha, hora) = a_fat(mtime);
        let r = match self.buscar(d, nombre)? {
            Some(e) => {
                if e.es_dir() {
                    return Err(Error::EsDir);
                }
                let nodo = Nodo {
                    dir: d,
                    indice: e.indice,
                };
                let mut rec = e.rec;
                let (primero, cadena) = self.ajustar_cadena(cluster_de(&rec, self.tipo), necesarios)?;
                self.mover(&cadena, 0, None, Some(datos))?;
                poner_cluster(&mut rec, primero);
                rec[28..32].copy_from_slice(&(datos.len() as u32).to_le_bytes());
                poner_mtime(&mut rec, fecha, hora);
                self.poner_registro(nodo, &rec)?;
                Ok(nodo)
            }
            None => {
                let (primero, cadena) = self.ajustar_cadena(0, necesarios)?;
                let escrito = self.mover(&cadena, 0, None, Some(datos));
                let mut rec = [0u8; ENTRADA];
                rec[11] = ATTR_ARCHIVO;
                poner_cluster(&mut rec, primero);
                rec[28..32].copy_from_slice(&(datos.len() as u32).to_le_bytes());
                poner_mtime(&mut rec, fecha, hora);
                poner_ctime(&mut rec, fecha, hora);
                let res = escrito.and_then(|_| self.crear_entrada(d, nombre, rec));
                if res.is_err() && primero != 0 {
                    let _ = self.liberar_desde(primero);
                }
                res
            }
        };
        self.flush()?;
        r
    }

    /// Añade `datos` al final del fichero.
    pub fn anexar(&mut self, n: Nodo, datos: &[u8], mtime: u64) -> Res<()> {
        let mut rec = self.registro(n)?;
        if rec[11] & ATTR_DIR != 0 {
            return Err(Error::EsDir);
        }
        let tam = u32le(&rec, 28) as u64;
        let nuevo = tam + datos.len() as u64;
        if nuevo > u32::MAX as u64 {
            return Err(Error::SinEspacio);
        }
        let cb = self.bytes_cluster() as u64;
        let (primero, cadena) =
            self.ajustar_cadena(cluster_de(&rec, self.tipo), nuevo.div_ceil(cb) as usize)?;
        self.mover(&cadena, tam, None, Some(datos))?;
        poner_cluster(&mut rec, primero);
        rec[28..32].copy_from_slice(&(nuevo as u32).to_le_bytes());
        let (fecha, hora) = a_fat(mtime);
        poner_mtime(&mut rec, fecha, hora);
        self.poner_registro(n, &rec)?;
        self.flush()
    }

    /// Recorta o alarga (con ceros) el fichero.
    pub fn truncar(&mut self, n: Nodo, tam: u64, mtime: u64) -> Res<()> {
        let mut rec = self.registro(n)?;
        if rec[11] & ATTR_DIR != 0 {
            return Err(Error::EsDir);
        }
        if tam > u32::MAX as u64 {
            return Err(Error::SinEspacio);
        }
        let viejo = u32le(&rec, 28) as u64;
        let cb = self.bytes_cluster() as u64;
        let (primero, cadena) =
            self.ajustar_cadena(cluster_de(&rec, self.tipo), tam.div_ceil(cb) as usize)?;
        if tam > viejo {
            let ceros = vec![0u8; (TROZO_SECTORES * SECTOR).min((tam - viejo) as usize)];
            let mut pos = viejo;
            while pos < tam {
                let n = ((tam - pos) as usize).min(ceros.len());
                self.mover(&cadena, pos, None, Some(&ceros[..n]))?;
                pos += n as u64;
            }
        }
        poner_cluster(&mut rec, primero);
        rec[28..32].copy_from_slice(&(tam as u32).to_le_bytes());
        let (fecha, hora) = a_fat(mtime);
        poner_mtime(&mut rec, fecha, hora);
        self.poner_registro(n, &rec)?;
        self.flush()
    }

    pub fn poner_mtime_nodo(&mut self, n: Nodo, mtime: u64) -> Res<()> {
        if n == RAIZ {
            return Ok(());
        }
        let mut rec = self.registro(n)?;
        let (fecha, hora) = a_fat(mtime);
        poner_mtime(&mut rec, fecha, hora);
        self.poner_registro(n, &rec)?;
        self.flush()
    }

    pub fn crear_dir(&mut self, dir: Nodo, nombre: &str, mtime: u64) -> Res<Nodo> {
        let d = self.contenido_dir(dir)?;
        if self.buscar(d, nombre)?.is_some() {
            return Err(Error::Existe);
        }
        let c = self.asignar(None)?;
        let (fecha, hora) = a_fat(mtime);
        let r = (|| {
            self.poner_a_cero(c)?;
            let mut punto = [0u8; ENTRADA];
            punto[..11].copy_from_slice(b".          ");
            punto[11] = ATTR_DIR;
            poner_cluster(&mut punto, c);
            poner_mtime(&mut punto, fecha, hora);
            let mut punto2 = punto;
            punto2[..11].copy_from_slice(b"..         ");
            poner_cluster(&mut punto2, d);
            self.mover(&[c], 0, None, Some(&[punto, punto2].concat()))?;
            let mut rec = [0u8; ENTRADA];
            rec[11] = ATTR_DIR;
            poner_cluster(&mut rec, c);
            poner_mtime(&mut rec, fecha, hora);
            poner_ctime(&mut rec, fecha, hora);
            self.crear_entrada(d, nombre, rec)
        })();
        if r.is_err() {
            let _ = self.poner_entrada_fat(c, 0);
        }
        self.flush()?;
        r
    }

    /// Borra un fichero o un directorio vacío.
    pub fn borrar(&mut self, dir: Nodo, nombre: &str) -> Res<()> {
        let d = self.contenido_dir(dir)?;
        let e = self.buscar(d, nombre)?.ok_or(Error::NoEncontrado)?;
        let c = cluster_de(&e.rec, self.tipo);
        if e.es_dir() {
            if c == 0 || self.normalizar(c) == 0 {
                return Err(Error::Corrupto);
            }
            if !self.entradas(c)?.0.is_empty() {
                return Err(Error::NoVacio);
            }
        }
        self.borrar_entradas(d, &e)?;
        if c != 0 {
            self.liberar_desde(c)?;
        }
        self.flush()
    }

    /// Renombra o mueve. Un fichero de destino que ya exista se reemplaza;
    /// un directorio de destino sólo si está vacío.
    pub fn renombrar(&mut self, odir: Nodo, onombre: &str, ndir: Nodo, nnombre: &str) -> Res<Nodo> {
        if !nombre_valido(nnombre) {
            return Err(Error::NombreInvalido);
        }
        let od = self.contenido_dir(odir)?;
        let nd = self.contenido_dir(ndir)?;
        let viejo = self.buscar(od, onombre)?.ok_or(Error::NoEncontrado)?;
        let c = cluster_de(&viejo.rec, self.tipo);
        if viejo.es_dir() && nd != od {
            // No meter un directorio dentro de sí mismo.
            let mut p = nd;
            for _ in 0..self.n_clusters {
                if p == c {
                    return Err(Error::NombreInvalido);
                }
                if p == 0 {
                    break;
                }
                let (crudo, _) = self.leer_dir(p)?;
                if crudo.len() < 2 * ENTRADA || &crudo[ENTRADA..ENTRADA + 11] != b"..         " {
                    break;
                }
                let mut r = [0u8; ENTRADA];
                r.copy_from_slice(&crudo[ENTRADA..2 * ENTRADA]);
                p = self.normalizar(cluster_de(&r, self.tipo));
            }
        }
        let mismo = od == nd && iguales(&viejo.nombre, nnombre);
        if !mismo {
            if let Some(dest) = self.buscar(nd, nnombre)? {
                if dest.es_dir() != viejo.es_dir() {
                    return Err(if dest.es_dir() { Error::EsDir } else { Error::NoEsDir });
                }
                let dc = cluster_de(&dest.rec, self.tipo);
                if dest.es_dir() && !self.entradas(dc)?.0.is_empty() {
                    return Err(Error::NoVacio);
                }
                self.borrar_entradas(nd, &dest)?;
                if dc != 0 {
                    self.liberar_desde(dc)?;
                }
            }
        }
        // Primero quitar la vieja: así un cambio sólo de mayúsculas no choca
        // consigo mismo, y el hueco puede reutilizarse.
        self.borrar_entradas(od, &viejo)?;
        let mut rec = viejo.rec;
        rec[12] = 0;
        let nodo = match self.crear_entrada(nd, nnombre, rec) {
            Ok(n) => n,
            Err(e) => {
                // Devolverle su nombre: mejor que perder el fichero.
                let _ = self.crear_entrada(od, &viejo.nombre, viejo.rec);
                self.flush()?;
                return Err(e);
            }
        };
        if viejo.es_dir() && nd != od && c != 0 {
            let cadena = self.cadena(c)?;
            let mut rec2 = [0u8; ENTRADA];
            self.mover(&cadena, ENTRADA as u64, Some(&mut rec2), None)?;
            if &rec2[..11] == b"..         " {
                poner_cluster(&mut rec2, nd);
                self.mover(&cadena, ENTRADA as u64, None, Some(&rec2))?;
            }
        }
        self.flush()?;
        Ok(nodo)
    }

    /// Bytes libres (recorre la FAT: es caro en volúmenes grandes).
    pub fn libres(&mut self) -> Res<u64> {
        let mut n = 0u64;
        for c in 2..self.n_clusters + 2 {
            if self.entrada_fat(c)? == 0 {
                n += 1;
            }
        }
        Ok(n * self.bytes_cluster() as u64)
    }

    pub fn total(&self) -> u64 {
        self.n_clusters as u64 * self.bytes_cluster() as u64
    }
}

fn cluster_de(r: &[u8], tipo: Tipo) -> u32 {
    let lo = u16le(r, 26) as u32;
    if tipo == Tipo::Fat32 {
        lo | (u16le(r, 20) as u32) << 16
    } else {
        lo
    }
}

fn poner_cluster(r: &mut [u8; ENTRADA], c: u32) {
    r[20..22].copy_from_slice(&((c >> 16) as u16).to_le_bytes());
    r[26..28].copy_from_slice(&(c as u16).to_le_bytes());
}

fn poner_mtime(r: &mut [u8; ENTRADA], fecha: u16, hora: u16) {
    r[22..24].copy_from_slice(&hora.to_le_bytes());
    r[24..26].copy_from_slice(&fecha.to_le_bytes());
    r[18..20].copy_from_slice(&fecha.to_le_bytes());
}

fn poner_ctime(r: &mut [u8; ENTRADA], fecha: u16, hora: u16) {
    r[14..16].copy_from_slice(&hora.to_le_bytes());
    r[16..18].copy_from_slice(&fecha.to_le_bytes());
}

fn info_de(r: &[u8]) -> Info {
    Info {
        es_dir: r[11] & ATTR_DIR != 0,
        tam: u32le(r, 28) as u64,
        mtime: desde_fat(u16le(r, 24), u16le(r, 22)),
        solo_lectura: r[11] & ATTR_RO != 0,
    }
}

#[cfg(test)]
mod tests;
