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

// Desplazamientos de la cabecera ELF64 y de un section header. Están aquí con
// nombre porque el fallo que arregló [T67] fue exactamente escribir en el sitio
// de al lado: `e_type` iba a `e_machine`, `e_machine` a `e_version`, y desde
// `sh_addr` los section headers iban ocho bytes corridos. Con números pelados
// eso se lee igual de bien esté bien o mal.
//
// [T67]: ../../../docs/self-improvement/T67-sosoas-elf-desplazado.md
mod eh {
    pub const TYPE: usize = 0x10; // u16
    pub const MACHINE: usize = 0x12; // u16
    pub const VERSION: usize = 0x14; // u32
    pub const SHOFF: usize = 0x28; // u64
    pub const EHSIZE: usize = 0x34; // u16
    pub const PHENTSIZE: usize = 0x36; // u16
    pub const SHENTSIZE: usize = 0x3a; // u16
    pub const SHNUM: usize = 0x3c; // u16
    pub const SHSTRNDX: usize = 0x3e; // u16
    pub const SIZE: u16 = 64;
}

mod sh {
    pub const NAME: usize = 0x00; // u32
    pub const TYPE: usize = 0x04; // u32
    pub const FLAGS: usize = 0x08; // u64
    pub const ADDR: usize = 0x10; // u64
    pub const OFFSET: usize = 0x18; // u64
    pub const SIZE_: usize = 0x20; // u64
    pub const LINK: usize = 0x28; // u32
    pub const INFO: usize = 0x2c; // u32
    pub const ADDRALIGN: usize = 0x30; // u64
    pub const ENTSIZE: usize = 0x38; // u64
    pub const SIZE: u64 = 64;
}

const ET_REL: u16 = 1;
const EM_X86_64: u16 = 0x3e;
const EV_CURRENT: u32 = 1;
const SHT_PROGBITS: u32 = 1;
const SHT_STRTAB: u32 = 3;
const SHF_ALLOC: u64 = 0x2;
const SHF_EXECINSTR: u64 = 0x4;

fn put16(out: &mut [u8], off: usize, v: u16) {
    out[off..off + 2].copy_from_slice(&v.to_le_bytes());
}
fn put32(out: &mut [u8], off: usize, v: u32) {
    out[off..off + 4].copy_from_slice(&v.to_le_bytes());
}
fn put64(out: &mut [u8], off: usize, v: u64) {
    out[off..off + 8].copy_from_slice(&v.to_le_bytes());
}

fn emit_elf(text: &[u8]) -> Vec<u8> {
    // ELF64 ET_REL mínimo: cabecera + 3 section headers (null, .text,
    // .shstrtab) + los datos.
    //
    // **Sin tabla de símbolos**: `.globl` se ignora, así que el objeto es
    // válido pero no exporta nada. Es un límite declarado, no un descuido —
    // leer símbolos es trabajo de ensamblador y `sosoas` no lo es todavía
    // (ver `docs/self-improvement/native/toolchain-deps.md`).
    let e_shoff = u64::from(eh::SIZE);
    let shnum: u16 = 3;
    let shstrndx: u16 = 2;
    let shstr: &[u8] = b"\0.text\0.shstrtab\0";
    const NAME_TEXT: u32 = 1; // índice de ".text" dentro de shstr
    const NAME_SHSTRTAB: u32 = 7; // índice de ".shstrtab"
    let text_off = e_shoff + u64::from(shnum) * sh::SIZE;
    let shstr_off = text_off + text.len() as u64;
    let file_size = shstr_off + shstr.len() as u64;

    let mut out = vec![0u8; file_size as usize];
    out[0..4].copy_from_slice(b"\x7fELF");
    out[4] = 2; // ELFCLASS64
    out[5] = 1; // ELFDATA2LSB
    out[6] = 1; // EV_CURRENT en e_ident
    put16(&mut out, eh::TYPE, ET_REL);
    put16(&mut out, eh::MACHINE, EM_X86_64);
    put32(&mut out, eh::VERSION, EV_CURRENT);
    put64(&mut out, eh::SHOFF, e_shoff);
    put16(&mut out, eh::EHSIZE, eh::SIZE);
    put16(&mut out, eh::PHENTSIZE, 0);
    put16(&mut out, eh::SHENTSIZE, sh::SIZE as u16);
    put16(&mut out, eh::SHNUM, shnum);
    put16(&mut out, eh::SHSTRNDX, shstrndx);

    // El section header 0 es la entrada nula: se queda a ceros a propósito.

    // .text (índice 1)
    let o = (e_shoff + sh::SIZE) as usize;
    put32(&mut out, o + sh::NAME, NAME_TEXT);
    put32(&mut out, o + sh::TYPE, SHT_PROGBITS);
    put64(&mut out, o + sh::FLAGS, SHF_ALLOC | SHF_EXECINSTR);
    put64(&mut out, o + sh::ADDR, 0);
    put64(&mut out, o + sh::OFFSET, text_off);
    put64(&mut out, o + sh::SIZE_, text.len() as u64);
    put32(&mut out, o + sh::LINK, 0);
    put32(&mut out, o + sh::INFO, 0);
    put64(&mut out, o + sh::ADDRALIGN, 1);
    put64(&mut out, o + sh::ENTSIZE, 0);

    // .shstrtab (índice 2)
    let o = (e_shoff + 2 * sh::SIZE) as usize;
    put32(&mut out, o + sh::NAME, NAME_SHSTRTAB);
    put32(&mut out, o + sh::TYPE, SHT_STRTAB);
    put64(&mut out, o + sh::FLAGS, 0);
    put64(&mut out, o + sh::OFFSET, shstr_off);
    put64(&mut out, o + sh::SIZE_, shstr.len() as u64);
    put64(&mut out, o + sh::ADDRALIGN, 1);

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

    fn u16_en(e: &[u8], off: usize) -> u16 {
        u16::from_le_bytes(e[off..off + 2].try_into().unwrap())
    }
    fn u32_en(e: &[u8], off: usize) -> u32 {
        u32::from_le_bytes(e[off..off + 4].try_into().unwrap())
    }
    fn u64_en(e: &[u8], off: usize) -> u64 {
        u64::from_le_bytes(e[off..off + 8].try_into().unwrap())
    }

    /// La cabecera dice lo que es, en el sitio donde ELF64 lo espera.
    ///
    /// Esto es [T67]: antes `ET_REL` acababa en `e_machine` y `EM_X86_64` en
    /// `e_version`, y cuatro campos no se escribían nunca. `readelf` lo
    /// delataba, pero el criterio de esta prueba **no puede depender de
    /// binutils**: la toolchain nativa tiene que poder comprobarse a sí misma.
    ///
    /// [T67]: ../../../docs/self-improvement/T67-sosoas-elf-desplazado.md
    #[test]
    fn la_cabecera_elf_tiene_cada_campo_en_su_sitio() {
        let e = emit_elf(&[0x90]);
        assert_eq!(u16_en(&e, eh::TYPE), ET_REL, "e_type");
        assert_eq!(u16_en(&e, eh::MACHINE), EM_X86_64, "e_machine");
        assert_eq!(u32_en(&e, eh::VERSION), EV_CURRENT, "e_version");
        assert_eq!(u64_en(&e, eh::SHOFF), u64::from(eh::SIZE), "e_shoff");
        assert_eq!(u16_en(&e, eh::EHSIZE), eh::SIZE, "e_ehsize");
        assert_eq!(u16_en(&e, eh::SHENTSIZE), sh::SIZE as u16, "e_shentsize");
        assert_eq!(u16_en(&e, eh::SHNUM), 3, "e_shnum");
        assert_eq!(u16_en(&e, eh::SHSTRNDX), 2, "e_shstrndx");
    }

    /// Los section headers también iban desplazados —ocho bytes desde
    /// `sh_addr`—, y `sh_name`/`sh_type` no se escribían: las secciones salían
    /// sin nombre y de tipo NULL.
    #[test]
    fn las_secciones_se_llaman_y_apuntan_a_donde_deben() {
        let text = [0x89u8, 0xf8, 0xc3];
        let e = emit_elf(&text);
        let shoff = u64_en(&e, eh::SHOFF) as usize;
        let tam = sh::SIZE as usize;

        // La entrada 0 es la nula: entera a ceros.
        assert!(e[shoff..shoff + tam].iter().all(|b| *b == 0), "shdr nula");

        let t = shoff + tam;
        assert_eq!(u32_en(&e, t + sh::TYPE), SHT_PROGBITS, ".text es PROGBITS");
        assert_eq!(
            u64_en(&e, t + sh::FLAGS),
            SHF_ALLOC | SHF_EXECINSTR,
            ".text es alojable y ejecutable"
        );
        assert_eq!(u64_en(&e, t + sh::SIZE_), text.len() as u64, ".text sh_size");

        // Y el nombre apunta a una cadena de verdad dentro de .shstrtab.
        let st = shoff + 2 * tam;
        assert_eq!(u32_en(&e, st + sh::TYPE), SHT_STRTAB, ".shstrtab es STRTAB");
        let str_off = u64_en(&e, st + sh::OFFSET) as usize;
        let str_len = u64_en(&e, st + sh::SIZE_) as usize;
        let tabla = &e[str_off..str_off + str_len];
        for (shdr, esperado) in [(t, ".text"), (st, ".shstrtab")] {
            let n = u32_en(&e, shdr + sh::NAME) as usize;
            let fin = tabla[n..].iter().position(|b| *b == 0).unwrap() + n;
            assert_eq!(&tabla[n..fin], esperado.as_bytes(), "nombre de sección");
        }
    }

    /// Los bytes del `.text` están donde el section header dice, y son los que
    /// entraron. Sin esto, la cabecera podría estar perfecta y el contenido en
    /// otro sitio.
    #[test]
    fn el_texto_esta_donde_dice_el_section_header() {
        let text = [0x89u8, 0xf8, 0x01, 0xf0, 0xc3];
        let e = emit_elf(&text);
        let shoff = u64_en(&e, eh::SHOFF) as usize;
        let t = shoff + sh::SIZE as usize;
        let off = u64_en(&e, t + sh::OFFSET) as usize;
        let len = u64_en(&e, t + sh::SIZE_) as usize;
        assert_eq!(&e[off..off + len], &text);
    }
}
