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
        if addr + len > MMAP_LIMIT {
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
    let end = start + len;
    regions
        .iter()
        .filter(|r| {
            let rend = r.virt_start + r.len;
            start < rend && r.virt_start < end
        })
        .map(|r| r.virt_start + r.len)
        .max()
}

pub fn find_region(regions: &[MmapRegion], addr: u64) -> Option<&MmapRegion> {
    regions.iter().find(|r| addr >= r.virt_start && addr < r.virt_start + r.len)
}

pub fn remove_region(regions: &mut Vec<MmapRegion>, addr: u64, len: u64) -> bool {
    let end = addr + len;
    let before = regions.len();
    regions.retain(|r| {
        let rend = r.virt_start + r.len;
        !(r.virt_start < end && addr < rend)
    });
    regions.len() < before
}
