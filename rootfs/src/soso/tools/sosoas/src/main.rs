//! Ensamblador GAS x86_64 de subconjunto para el toolchain nativo de soso.
//!
//! Uso: `sosoas -o out.o in.s`
//!
//! Soporta `.text`, `.globl`, `.byte` (hex). Emite un ELF64 ET_REL mínimo.

use std::env;
use std::fs;
use std::process::exit;

fn parse_bytes(src: &str) -> Vec<u8> {
    let mut out = Vec::new();
    for line in src.lines() {
        let t = line.split('#').next().unwrap_or("").trim();
        if !t.starts_with(".byte") {
            continue;
        }
        let rest = t.strip_prefix(".byte").unwrap_or("").trim();
        for part in rest.split(',') {
            let p = part.trim();
            if p.is_empty() {
                continue;
            }
            let p = p.strip_prefix("0x").unwrap_or(p);
            if let Ok(v) = u8::from_str_radix(p, 16) {
                out.push(v);
            }
        }
    }
    out
}

fn emit_elf(text: &[u8]) -> Vec<u8> {
    // ELF64 ET_REL mínimo: header + 2 section headers (.null, .text) + .shstrtab
    let e_shoff = 64u64;
    let shdr_size = 64u64;
    let shnum = 3u64;
    let shstrndx = 2u64;
    let shstr: &[u8] = b"\0.text\0.shstrtab\0";
    let text_off = e_shoff + shnum * shdr_size;
    let shstr_off = text_off + text.len() as u64;
    let file_size = shstr_off + shstr.len() as u64;

    let mut out = vec![0u8; file_size as usize];
    out[0..4].copy_from_slice(b"\x7fELF");
    out[4] = 2; // ELFCLASS64
    out[5] = 1; // ELFDATA2LSB
    out[6] = 1; // EV_CURRENT
    out[0x12..0x14].copy_from_slice(&1u16.to_le_bytes()); // ET_REL
    out[0x14..0x16].copy_from_slice(&0x3eu16.to_le_bytes()); // EM_X86_64
    out[0x28..0x30].copy_from_slice(&e_shoff.to_le_bytes());
    out[0x3a..0x3c].copy_from_slice(&(shnum as u16).to_le_bytes());
    out[0x3c..0x3e].copy_from_slice(&(shstrndx as u16).to_le_bytes());

    // .text shdr (index 1)
    let mut o = (e_shoff + shdr_size) as usize;
    out[o + 0x08..o + 0x10].copy_from_slice(&1u64.to_le_bytes()); // sh_flags SHF_ALLOC|EXEC
    out[o + 0x10..o + 0x18].copy_from_slice(&text_off.to_le_bytes());
    out[o + 0x18..o + 0x20].copy_from_slice(&(text.len() as u64).to_le_bytes());
    out[o + 0x20..o + 0x24].copy_from_slice(&1u32.to_le_bytes()); // sh_link
    out[o + 0x28..o + 0x30].copy_from_slice(&1u64.to_le_bytes()); // sh_addralign

    // .shstrtab shdr (index 2)
    o = (e_shoff + 2 * shdr_size) as usize;
    out[o + 0x08..o + 0x10].copy_from_slice(&0u64.to_le_bytes());
    out[o + 0x10..o + 0x18].copy_from_slice(&shstr_off.to_le_bytes());
    out[o + 0x18..o + 0x20].copy_from_slice(&(shstr.len() as u64).to_le_bytes());

    out[text_off as usize..(text_off as usize + text.len())].copy_from_slice(text);
    out[shstr_off as usize..].copy_from_slice(shstr);
    out
}

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 4 || args[1] != "-o" {
        eprintln!("uso: sosoas -o SALIDA.o ENTRADA.s");
        exit(2);
    }
    let out = &args[2];
    let inp = &args[3];
    let src = fs::read_to_string(inp).unwrap_or_else(|e| {
        eprintln!("sosoas: {inp}: {e}");
        exit(1);
    });
    if !src.contains(".text") && !src.contains(".globl") {
        eprintln!("sosoas: {inp}: no parece GAS x86_64");
        exit(1);
    }
    let text = parse_bytes(&src);
    if text.is_empty() {
        eprintln!("sosoas: {inp}: sin bytes (.byte)");
        exit(1);
    }
    let elf = emit_elf(&text);
    fs::write(out, &elf).unwrap_or_else(|e| {
        eprintln!("sosoas: no se pudo escribir {out}: {e}");
        exit(1);
    });
    eprintln!("sosoas: {inp} → {out} ({} B .text)", text.len());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_byte_line() {
        let b = parse_bytes(".text\n.globl x\n.byte 0x48, 0x31\n");
        assert_eq!(b, vec![0x48, 0x31]);
    }

    #[test]
    fn emit_elf_magic() {
        let e = emit_elf(&[0x90]);
        assert_eq!(&e[0..4], b"\x7fELF");
    }
}
