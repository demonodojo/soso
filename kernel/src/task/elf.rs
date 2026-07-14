//! Carga de ELFs estáticos (ET_EXEC, enlazados a USER_BASE) en un
//! espacio de direcciones de proceso.

use super::addrspace::{AddrSpace, BRK_MAX, USER_BASE};
use crate::mm;
use xmas_elf::ElfFile;
use xmas_elf::header;
use xmas_elf::program::Type;

/// Carga los segmentos PT_LOAD. Devuelve (entry point, brk inicial).
pub fn load(space: &mut AddrSpace, data: &[u8]) -> Result<(u64, u64), &'static str> {
    let elf = ElfFile::new(data)?;
    header::sanity_check(&elf)?;
    if elf.header.pt2.type_().as_type() != header::Type::Executable {
        return Err("no es un ejecutable estático (ET_EXEC)");
    }
    if elf.header.pt2.machine().as_machine() != header::Machine::X86_64 {
        return Err("no es x86_64");
    }

    let mut brk = USER_BASE;
    for ph in elf.program_iter() {
        if ph.get_type()? != Type::Load {
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
            // Parte de este page cubierta por datos del fichero.
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
            // El resto ya es cero (ensure_mapped limpia el frame).
            page += 4096;
        }
        brk = brk.max(vaddr + memsz);
    }

    Ok((elf.header.pt2.entry_point(), brk.next_multiple_of(4096)))
}
