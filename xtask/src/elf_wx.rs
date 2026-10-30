//! Comprueba que los ELF del rootfs respeten W^X (sin PT_LOAD W+X ni solape W con X).

use std::fs;
use std::path::{Path, PathBuf};

const PT_LOAD: u32 = 1;
const PF_X: u32 = 1;
const PF_W: u32 = 2;
const PF_R: u32 = 4;

#[derive(Clone, Copy)]
struct Load {
    vaddr: u64,
    memsz: u64,
    flags: u32,
}

fn u16_at(b: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(o..o + 2)?.try_into().ok()?))
}
fn u32_at(b: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(o..o + 4)?.try_into().ok()?))
}
fn u64_at(b: &[u8], o: usize) -> Option<u64> {
    Some(u64::from_le_bytes(b.get(o..o + 8)?.try_into().ok()?))
}

fn loads_from_elf(data: &[u8]) -> Result<Vec<Load>, String> {
    if data.get(..4) != Some(b"\x7fELF") {
        return Err("no ELF".into());
    }
    if u16_at(data, 18).ok_or("ELF truncado")? != 62 {
        return Err("no x86_64".into());
    }
    let phoff = u64_at(data, 32).ok_or("ELF truncado")? as usize;
    let phent = u16_at(data, 54).ok_or("ELF truncado")? as usize;
    let phnum = u16_at(data, 56).ok_or("ELF truncado")? as usize;
    let mut out = Vec::new();
    for i in 0..phnum {
        let o = phoff + i * phent;
        if u32_at(data, o).ok_or("ELF truncado")? != PT_LOAD {
            continue;
        }
        let flags = u32_at(data, o + 4).ok_or("ELF truncado")?;
        let vaddr = u64_at(data, o + 16).ok_or("ELF truncado")?;
        let memsz = u64_at(data, o + 40).ok_or("ELF truncado")?;
        if memsz == 0 {
            continue;
        }
        if flags & PF_W != 0 && flags & PF_X != 0 {
            return Err(format!("PT_LOAD W+X en vaddr={vaddr:#x}"));
        }
        out.push(Load { vaddr, memsz, flags });
    }
    Ok(out)
}

fn page_overlap_wx(loads: &[Load]) -> Option<(u64, u32, u32)> {
    for i in 0..loads.len() {
        for j in (i + 1)..loads.len() {
            let a = loads[i];
            let b = loads[j];
            let a_end = a.vaddr.saturating_add(a.memsz);
            let b_end = b.vaddr.saturating_add(b.memsz);
            let start = a.vaddr.max(b.vaddr);
            let end = a_end.min(b_end);
            if start >= end {
                continue;
            }
            let mut p = start & !0xfff;
            while p < end {
                let a_on = p >= a.vaddr && p < a_end;
                let b_on = p >= b.vaddr && p < b_end;
                if a_on && b_on {
                    let aw = a.flags & PF_W != 0;
                    let ax = a.flags & PF_X != 0;
                    let bw = b.flags & PF_W != 0;
                    let bx = b.flags & PF_X != 0;
                    if (aw && bx) || (ax && bw) {
                        return Some((p, a.flags, b.flags));
                    }
                }
                p += 4096;
            }
        }
    }
    None
}

fn scan_file(path: &Path) -> Result<(), String> {
    let data = fs::read(path).map_err(|e| e.to_string())?;
    let loads = loads_from_elf(&data)?;
    if let Some((page, fa, fb)) = page_overlap_wx(&loads) {
        return Err(format!(
            "solape W/X en página {page:#x} (flags {fa:#x} vs {fb:#x})"
        ));
    }
    Ok(())
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, out);
        } else if p.extension().is_none() || p.file_name().is_some_and(|n| !n.to_string_lossy().contains('.')) {
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name.is_empty() {
                continue;
            }
            if fs::read(&p)
                .ok()
                .is_some_and(|b| b.starts_with(b"\x7fELF"))
            {
                out.push(p);
            }
        } else if fs::read(&p)
            .ok()
            .is_some_and(|b| b.starts_with(b"\x7fELF"))
        {
            out.push(p);
        }
    }
}

/// Recorre `rootfs/bin` y `rootfs/lib` (árbol embebido en el disco).
pub fn verificar_rootfs(root: &Path) -> Result<(), String> {
    let mut files = Vec::new();
    for sub in ["rootfs/bin", "rootfs/lib", "target/user-root/bin", "target/user-root/lib"] {
        walk(&root.join(sub), &mut files);
    }
    files.sort();
    files.dedup();
    for f in files {
        scan_file(&f).map_err(|e| format!("{}: {e}", f.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rechaza_wx_mismo_segmento() {
        let mut b = vec![0u8; 128];
        b[0..4].copy_from_slice(b"\x7fELF");
        b[4] = 2;
        b[5] = 1;
        b[18..20].copy_from_slice(&62u16.to_le_bytes());
        b[32..40].copy_from_slice(&64u64.to_le_bytes()); // e_phoff
        b[54..56].copy_from_slice(&56u16.to_le_bytes());
        b[56..58].copy_from_slice(&1u16.to_le_bytes());
        b[64..68].copy_from_slice(&1u32.to_le_bytes()); // PT_LOAD
        b[68..72].copy_from_slice(&(PF_R | PF_W | PF_X).to_le_bytes());
        b[80..88].copy_from_slice(&0x400000u64.to_le_bytes());
        b[104..112].copy_from_slice(&0x1000u64.to_le_bytes());
        assert!(loads_from_elf(&b).is_err());
    }
}
