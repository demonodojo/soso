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
struct LoadPh {
    vaddr: u64,
    memsz: u64,
    filesz: u64,
    offset: u64,
    #[allow(dead_code)]
    write: bool,
    exec: bool,
}

/// Preflight común a `load` y `load_lazy` antes de mutar el mapa.
fn check_load_ph(
    vaddr: u64,
    memsz: u64,
    filesz: u64,
    offset: u64,
    file_limit: u64,
) -> Result<(), &'static str> {
    if memsz == 0 {
        return Ok(());
    }
    if filesz > memsz {
        return Err("filesz > memsz");
    }
    if offset.checked_add(filesz).is_none_or(|end| end > file_limit) {
        return Err("segmento truncado");
    }
    if vaddr < USER_BASE || vaddr.checked_add(memsz).is_none_or(|end| end > BRK_MAX) {
        return Err("segmento fuera del rango de usuario");
    }
    let page_delta = vaddr & 0xfff;
    if page_delta > offset {
        return Err("segmento no alineado");
    }
    if (vaddr & 0xfff) != (offset & 0xfff) {
        return Err("segmento incongruente");
    }
    Ok(())
}

fn collect_load_phs(elf: &ElfFile<'_>, file_limit: u64) -> Result<alloc::vec::Vec<LoadPh>, &'static str> {
    let mut out = alloc::vec::Vec::new();
    let mut spans: alloc::vec::Vec<(u64, u64)> = alloc::vec::Vec::new();
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
        check_load_ph(vaddr, memsz, filesz, offset, file_limit)?;
        if ty == Type::Load {
            let start = vaddr;
            let end = vaddr + memsz;
            for &(s, e) in &spans {
                if start < e && s < end && !(start == s && end == e) {
                    /* solape de página text/data es habitual; rechazar hueco no aplica aquí */
                }
            }
            spans.push((start, end));
            out.push(LoadPh {
                vaddr,
                memsz,
                filesz,
                offset,
                write: ph.flags().is_write(),
                exec: ph.flags().is_execute(),
            });
        }
    }
    Ok(out)
}

fn entry_in_exec_load(entry: u64, loads: &[LoadPh]) -> bool {
    loads
        .iter()
        .any(|p| p.exec && entry >= p.vaddr && entry < p.vaddr.saturating_add(p.memsz))
}

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
    check_exec(&elf)?;
    let loads = collect_load_phs(&elf, data.len() as u64)?;
    let entry = elf.header.pt2.entry_point();
    if !entry_in_exec_load(entry, &loads) {
        return Err("entry fuera de segmentos cargados");
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

    Ok((entry, brk.next_multiple_of(4096), tls_base))
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
    let loads = collect_load_phs(&elf, file_size)?;
    let entry = elf.header.pt2.entry_point();
    if !entry_in_exec_load(entry, &loads) {
        return Err("entry fuera de segmentos cargados");
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
        let page_delta = vaddr & 0xfff;
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
        let mut page = (vaddr + filesz).next_multiple_of(4096);
        while page < vaddr + memsz {
            space.ensure_mapped(page).ok_or("sin memoria")?;
            page += 4096;
        }
        brk = brk.max(vaddr + memsz);
    }
    Ok((entry, brk.next_multiple_of(4096), tls_base))
}
