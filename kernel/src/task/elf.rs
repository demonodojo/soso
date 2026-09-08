//! Carga de ELFs estáticos (ET_EXEC, enlazados a USER_BASE) en un
//! espacio de direcciones de proceso.

use super::addrspace::{AddrSpace, BRK_MAX, USER_BASE};
use crate::mm;
use xmas_elf::ElfFile;
use xmas_elf::header;
use xmas_elf::program::Type;

/// ET_EXEC x86_64 con la tabla de program headers dentro de `elf.input`.
/// No exige la tabla de secciones: en un ELF típico vive al final del fichero
/// y `load_lazy` solo tiene los primeros 64 KiB.
fn check_exec(elf: &ElfFile<'_>) -> Result<(), &'static str> {
    if elf.header.pt1.magic != header::MAGIC {
        return Err("bad magic number");
    }
    if elf.header.pt2.type_().as_type() != header::Type::Executable {
        return Err("no es un ejecutable estático (ET_EXEC)");
    }
    if elf.header.pt2.machine().as_machine() != header::Machine::X86_64 {
        return Err("no es x86_64");
    }
    let pt2 = &elf.header.pt2;
    let ph_end = pt2
        .ph_offset()
        .saturating_add((pt2.ph_entry_size() as u64).saturating_mul(pt2.ph_count() as u64));
    if ph_end > elf.input.len() as u64 {
        return Err("program header table out of range");
    }
    Ok(())
}

/// Carga los segmentos PT_LOAD y PT_TLS. Devuelve (entry, brk, tls_base).
pub fn load(space: &AddrSpace, data: &[u8]) -> Result<(u64, u64, u64), &'static str> {
    let elf = ElfFile::new(data)?;
    header::sanity_check(&elf)?;
    if elf.header.pt2.type_().as_type() != header::Type::Executable {
        return Err("no es un ejecutable estático (ET_EXEC)");
    }
    if elf.header.pt2.machine().as_machine() != header::Machine::X86_64 {
        return Err("no es x86_64");
    }

    let mut brk = USER_BASE;
    let mut tls_base = 0u64;
    for ph in elf.program_iter() {
        let ty = ph.get_type()?;
        if ty != Type::Load && ty != Type::Tls {
            continue;
        }
        let vaddr = ph.virtual_addr();
        let memsz = ph.mem_size();
        let filesz = ph.file_size();
        let offset = ph.offset();
        if memsz == 0 {
            continue;
        }
        if vaddr < USER_BASE || vaddr.checked_add(memsz).is_none_or(|end| end > BRK_MAX) {
            return Err("segmento fuera del rango de usuario");
        }
        if (offset + filesz) as usize > data.len() {
            return Err("segmento truncado");
        }

        let first_page = vaddr & !0xfff;
        let mut page = first_page;
        while page < vaddr + memsz {
            let frame = space.ensure_mapped(page).ok_or("sin memoria")?;
            let seg_start = vaddr.max(page);
            let seg_end = (vaddr + filesz).min(page + 4096);
            if seg_start < seg_end {
                let src = &data[(offset + (seg_start - vaddr)) as usize
                    ..(offset + (seg_end - vaddr)) as usize];
                let dst = mm::phys_to_virt(frame.start_address().as_u64() + (seg_start - page));
                unsafe {
                    core::ptr::copy_nonoverlapping(src.as_ptr(), dst.as_mut_ptr(), src.len())
                };
            }
            page += 4096;
        }
        brk = brk.max(vaddr + memsz);
        if ty == Type::Tls {
            tls_base = vaddr;
        }
    }

    Ok((
        elf.header.pt2.entry_point(),
        brk.next_multiple_of(4096),
        tls_base,
    ))
}

/// Carga perezosa: registra regiones mmap sobre `inode` en lugar de copiar el ELF.
pub fn load_lazy(
    space: &AddrSpace,
    head: &[u8],
    inode: u64,
    file_size: u64,
) -> Result<(u64, u64, u64), &'static str> {
    let elf = ElfFile::new(head)?;
    check_exec(&elf)?;
    let mut brk = USER_BASE;
    let mut tls_base = 0u64;
    let mut load_lo = u64::MAX;
    let mut load_hi = 0u64;
    for ph in elf.program_iter() {
        let ty = ph.get_type()?;
        if ty == Type::Tls {
            tls_base = ph.virtual_addr();
            continue;
        }
        if ty != Type::Load {
            continue;
        }
        let vaddr = ph.virtual_addr();
        let memsz = ph.mem_size();
        let filesz = ph.file_size();
        let offset = ph.offset();
        if memsz == 0 {
            continue;
        }
        if filesz > memsz {
            return Err("filesz > memsz");
        }
        if offset.checked_add(filesz).is_none_or(|end| end > file_size) {
            return Err("segmento truncado");
        }
        if vaddr < USER_BASE || vaddr.checked_add(memsz).is_none_or(|end| end > BRK_MAX) {
            return Err("segmento fuera del rango de usuario");
        }
        // p_vaddr ≡ p_offset (mod página). La página de solape text/data es
        // habitual: find_region se queda con el último PT_LOAD (el RW).
        let page_delta = vaddr & 0xfff;
        if page_delta > offset {
            return Err("segmento no alineado");
        }
        let virt_start = vaddr - page_delta;
        let file_offset = offset - page_delta;
        let len = (vaddr + memsz - virt_start).next_multiple_of(4096);
        space.with_mmap_mut(|book| {
            book.regions.push(super::mmap::MmapRegion {
                virt_start,
                len,
                inode,
                file_offset,
                file_len: core::cmp::min(offset.saturating_add(filesz), file_size),
                writable: ph.flags().is_write(),
            });
        });
        // BSS: páginas más allá de filesz (ya a cero vía fault anónimo).
        let mut page = (vaddr + filesz).next_multiple_of(4096);
        while page < vaddr + memsz {
            let _ = space.ensure_mapped(page);
            page += 4096;
        }
        brk = brk.max(vaddr + memsz);
        load_lo = load_lo.min(vaddr);
        load_hi = load_hi.max(vaddr + memsz);
    }
    let entry = elf.header.pt2.entry_point();
    if load_lo == u64::MAX || entry < load_lo || entry >= load_hi {
        return Err("entry fuera de segmentos cargados");
    }
    Ok((
        entry,
        brk.next_multiple_of(4096),
        tls_base,
    ))
}
