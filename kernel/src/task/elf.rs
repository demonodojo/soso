//! Carga de ELFs estáticos (ET_EXEC, enlazados a USER_BASE) en un
//! espacio de direcciones de proceso.

use super::addrspace::{AddrSpace, BRK_MAX, USER_BASE};
use crate::mm;
use xmas_elf::ElfFile;
use xmas_elf::header;
use xmas_elf::program::Type;

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
        if vaddr < USER_BASE || vaddr.checked_add(memsz).is_none_or(|end| end > BRK_MAX) {
            return Err("segmento fuera del rango de usuario");
        }
        let len = memsz.next_multiple_of(4096);
        space.with_mmap_mut(|book| {
            book.regions.push(super::mmap::MmapRegion {
                virt_start: vaddr,
                len,
                inode,
                file_offset: offset,
                file_len: file_size,
                writable: false,
            });
        });
        // BSS: páginas más allá de filesz (ya a cero vía fault anónimo).
        let mut page = (vaddr + filesz).next_multiple_of(4096);
        while page < vaddr + memsz {
            let _ = space.ensure_mapped(page);
            page += 4096;
        }
        brk = brk.max(vaddr + memsz);
    }
    Ok((
        elf.header.pt2.entry_point(),
        brk.next_multiple_of(4096),
        tls_base,
    ))
}
