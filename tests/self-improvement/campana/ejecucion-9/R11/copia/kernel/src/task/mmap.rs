//! Regiones mmap por proceso y resolución de fallos de página.

use alloc::vec::Vec;
use soso_abi::{MMAP_BASE, MMAP_LIMIT};

/// Región de fichero mapeada con paginación bajo demanda.
#[derive(Clone)]
pub struct MmapRegion {
    pub virt_start: u64,
    pub len: u64,
    pub inode: u64,
    pub file_offset: u64,
    /// Tamaño del fichero respaldo (0 en regiones anónimas): delimita hasta
    /// dónde puede servirse un fault con página de 2 MiB completa.
    pub file_len: u64,
    pub writable: bool,
}

/// Alineación de mapeos grandes: permite servir faults con páginas de 2 MiB
/// (el offset de fichero 0 queda 2 MiB-alineado en VA).
const HUGE_ALIGN: u64 = 2 * 1024 * 1024;

/// Siguiente dirección libre para mmap en este proceso. First-fit saltando
/// al final de la región que colisiona (no de 4 KiB en 4 KiB: la ventana es
/// de cientos de GiB).
pub fn next_addr(regions: &[MmapRegion], hint: u64, len: u64) -> Option<u64> {
    let len = len.next_multiple_of(4096);
    let align = if len >= HUGE_ALIGN { HUGE_ALIGN } else { 4096 };
    let start = if hint == 0 { MMAP_BASE } else { hint };
    let mut addr = start.next_multiple_of(align);
    loop {
        if addr.checked_add(len).is_none_or(|end| end > MMAP_LIMIT) {
            return None;
        }
        match first_overlap(regions, addr, len) {
            None => return Some(addr),
            Some(region_end) => {
                addr = region_end.next_multiple_of(align);
            }
        }
    }
}

/// Devuelve el final de alguna región que se solape con [start, start+len).
fn first_overlap(regions: &[MmapRegion], start: u64, len: u64) -> Option<u64> {
    let end = start.checked_add(len)?;
    regions
        .iter()
        .filter(|r| {
            let rend = r.virt_start.saturating_add(r.len);
            start < rend && r.virt_start < end
        })
        .map(|r| r.virt_start.saturating_add(r.len))
        .max()
}

pub fn find_region(regions: &[MmapRegion], addr: u64) -> Option<&MmapRegion> {
    // El último mapeo gana: en un ELF la última página de texto y la primera
    // de datos se solapan; si gana el RX, el primer write a .data mata init.
    regions.iter().rev().find(|r| {
        addr >= r.virt_start && addr < r.virt_start.saturating_add(r.len)
    })
}

/// Parte regiones que solapan `[addr, addr+len)` y aplica `writable` solo al
/// subrango. Así un mprotect interior no deja el fault-in con el permiso viejo.
pub fn split_prot(regions: &mut Vec<MmapRegion>, addr: u64, len: u64, writable: bool) -> bool {
    let Some(end) = addr.checked_add(len) else {
        return false;
    };
    let mut out = Vec::with_capacity(regions.len() + 2);
    let mut hit = false;
    for r in regions.drain(..) {
        let rend = r.virt_start.saturating_add(r.len);
        if rend <= addr || r.virt_start >= end {
            out.push(r);
            continue;
        }
        hit = true;
        if r.virt_start < addr {
            out.push(MmapRegion {
                virt_start: r.virt_start,
                len: addr - r.virt_start,
                inode: r.inode,
                file_offset: r.file_offset,
                file_len: r.file_len,
                writable: r.writable,
            });
        }
        let mid_start = r.virt_start.max(addr);
        let mid_end = rend.min(end);
        if mid_end > mid_start {
            out.push(MmapRegion {
                virt_start: mid_start,
                len: mid_end - mid_start,
                inode: r.inode,
                file_offset: r.file_offset.saturating_add(mid_start - r.virt_start),
                file_len: r.file_len,
                writable,
            });
        }
        if rend > end {
            out.push(MmapRegion {
                virt_start: end,
                len: rend - end,
                inode: r.inode,
                file_offset: r.file_offset.saturating_add(end - r.virt_start),
                file_len: r.file_len,
                writable: r.writable,
            });
        }
    }
    *regions = out;
    hit
}

pub fn remove_region(regions: &mut Vec<MmapRegion>, addr: u64, len: u64) -> bool {
    let Some(end) = addr.checked_add(len) else {
        return false;
    };
    let before = regions.len();
    regions.retain(|r| {
        let rend = r.virt_start + r.len;
        !(r.virt_start < end && addr < rend)
    });
    regions.len() < before
}
