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
    pub writable: bool,
}

/// Siguiente dirección libre para mmap en este proceso.
pub fn next_addr(regions: &[MmapRegion], hint: u64, len: u64) -> Option<u64> {
    let len = len.next_multiple_of(4096);
    let mut addr = if hint == 0 { MMAP_BASE } else { hint.next_multiple_of(4096) };
    while addr + len <= MMAP_LIMIT {
        if !overlaps(regions, addr, len) {
            return Some(addr);
        }
        addr += 4096;
    }
    None
}

fn overlaps(regions: &[MmapRegion], start: u64, len: u64) -> bool {
    let end = start + len;
    regions.iter().any(|r| {
        let rend = r.virt_start + r.len;
        start < rend && r.virt_start < end
    })
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
