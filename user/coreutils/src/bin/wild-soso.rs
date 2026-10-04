//! `wild-soso`: enlazador estático que rustc invoca en el guest.
//!
//! Lee objetos ELF y `.rlib`, aplica las reubicaciones x86-64 que emite
//! rustc y escribe un `ET_EXEC` en `0x400000`. La entrada es `_start`
//! (la PAL de std). `__ehdr_start` queda en la cabecera para que el
//! runtime encuentre `PT_TLS`.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::string::String;
use alloc::vec::Vec;
use libsoso::{abi, println, sys};

libsoso::entry!(main);

const BASE_EXEC: u64 = 0x400000;
/// Base nominal de un objeto compartido (`-shared`, T80): la ventana baja entre
/// `BRK_MAX` y la pila, que cabe en los 31 bits que exigen las relocaciones
/// `R_X86_64_32S` del código no PIC de las rlibs. El cargador de `std::os::soso::dl`
/// lo mapea donde haya hueco y suma la diferencia a cada dirección absoluta
/// anotada en `.soso.rel`.
const BASE_COMPARTIDO: u64 = 0x6100_0000;
/// Hueco de TLS que el cargador reserva bajo el TCB de cada hilo para los
/// objetos compartidos (`PM_TLS_BIAS` en `sys/pal/soso/dl.rs`): su bloque TLS
/// termina a este desplazamiento por debajo del TP.
const PM_TLS_BIAS: u64 = 0x8000;

static BASE_V: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(BASE_EXEC);

fn base() -> u64 {
    BASE_V.load(core::sync::atomic::Ordering::Relaxed)
}
const PAGE: u64 = 0x1000;

const SHF_WRITE: u64 = 0x1;
const SHF_ALLOC: u64 = 0x2;
const SHF_EXECINSTR: u64 = 0x4;
const SHF_TLS: u64 = 0x400;

const SHT_PROGBITS: u32 = 1;
const SHT_SYMTAB: u32 = 2;
const SHT_RELA: u32 = 4;
const SHT_NOBITS: u32 = 8;
const SHT_INIT_ARRAY: u32 = 14;
const SHT_FINI_ARRAY: u32 = 15;
const SHT_PREINIT_ARRAY: u32 = 16;

const SHN_UNDEF: u16 = 0;
const SHN_ABS: u16 = 0xfff1;

const STB_LOCAL: u8 = 0;
const STB_GLOBAL: u8 = 1;
const STB_WEAK: u8 = 2;
const STT_SECTION: u8 = 3;
const STT_FILE: u8 = 4;
const STT_TLS: u8 = 6;

const R_X86_64_64: u32 = 1;
const R_X86_64_PC32: u32 = 2;
// GOT: la dirección del símbolo va a un hueco de la GOT y la instrucción lo
// lee con un desplazamiento relativo. `GOTPCRELX` / `REX_GOTPCRELX` son la misma
// reubicación con permiso para relajarla; aquí no se relaja.
const R_X86_64_GOTPCREL: u32 = 9;
const R_X86_64_GOTPCRELX: u32 = 41;
const R_X86_64_REX_GOTPCRELX: u32 = 42;
const R_X86_64_PLT32: u32 = 4;
const R_X86_64_32: u32 = 10;
const R_X86_64_32S: u32 = 11;
const R_X86_64_GOTTPOFF: u32 = 22;
const R_X86_64_TPOFF32: u32 = 23;

const PT_LOAD: u32 = 1;
const PT_DYNAMIC: u32 = 2;
const DT_NULL: u64 = 0;
const DT_RELA: u64 = 7;
const DT_RELASZ: u64 = 8;
const DT_RELAENT: u64 = 9;
const R_X86_64_RELATIVE: u64 = 8;
const PT_TLS: u32 = 7;
const PF_X: u32 = 1;
const PF_W: u32 = 2;
const PF_R: u32 = 4;

fn main(args: &[String]) -> u8 {
    match enlazar(args) {
        Ok(()) => 0,
        Err(e) => {
            println!("wild-soso: {e}");
            1
        }
    }
}

#[derive(Clone)]
struct Reloc {
    offset: u64,
    sym: u32,
    typ: u32,
    addend: i64,
}

struct Sec {
    name: String,
    ty: u32,
    flags: u64,
    align: u64,
    size: u64,
    file_off: u64,
    relocs: Vec<Reloc>,
    keep: bool,
    out_addr: u64,
}

struct Sym {
    name: String,
    shndx: u16,
    value: u64,
    info: u8,
}

struct Obj {
    bytes: Vec<u8>,
    secs: Vec<Sec>,
    syms: Vec<Sym>,
}

enum Def {
    Abs(u64),
    Sym { obj: u32, sym: u32 },
}

enum Clase {
    Text,
    Ro,
    Tdata,
    Tbss,
    Data,
    Bss,
}

fn enlazar(args: &[String]) -> Result<(), String> {
    let op = linea(args)?;
    let compartido = op.compartido;
    let pie = op.pie && !compartido;
    if compartido {
        BASE_V.store(BASE_COMPARTIDO, core::sync::atomic::Ordering::Relaxed);
    } else if pie {
        BASE_V.store(0, core::sync::atomic::Ordering::Relaxed);
    }
    let salida = op.salida;
    let mut objs = Vec::new();
    for path in &op.entradas {
        cargar(&mut objs, path)?;
    }
    if objs.is_empty() {
        return Err(String::from("no hay objetos"));
    }
    // `extern_weak` de `__register_frame` llega como indefinido fuerte.
    // El runtime lo llama sólo si el puntero no es nulo; un `ret` basta.
    objs.push(stub_ret("__register_frame"));
    let mut global = simbolos(&objs)?;
    global
        .entry(String::from("__ehdr_start"))
        .or_insert(Def::Abs(base()));

    marcar(&mut objs, &global, compartido, pie, &op.exportar);
    if !objs.iter().any(|o| o.secs.iter().any(|s| s.keep)) {
        return Err(String::from("nada que enlazar"));
    }

    let got_n = contar_got(&objs);
    let nkept = objs.iter().map(|o| o.secs.iter().filter(|s| s.keep).count()).sum::<usize>();
    let has_tls = objs.iter().any(|o| {
        o.secs.iter().any(|s| s.keep && s.flags & SHF_TLS != 0)
    });
    let nphdr = if has_tls { 3 } else { 2 } + pie as usize;
    // Objeto compartido: `.rustc` (metadatos que rustc lee del propio fichero),
    // `.soso.exp` (símbolos exportados) y `.soso.rel` (direcciones absolutas).
    let rustc_meta: Option<Vec<u8>> = if compartido {
        objs.iter()
            .flat_map(|o| o.secs.iter().map(move |s| (o, s)))
            .find(|(_, s)| s.name == ".rustc" && s.flags & SHF_ALLOC == 0 && s.size > 0)
            .map(|(o, s)| o.bytes[s.file_off as usize..(s.file_off + s.size) as usize].to_vec())
    } else {
        None
    };
    let nextra = if compartido { 2 + rustc_meta.is_some() as usize } else { 0 };
    let nshdr = 1 + nkept + nextra + 1;
    let hdr = (64 + nphdr * 56 + nshdr * 64) as u64;

    let mut cursor = align_up(hdr, 16);
    colocar(&mut objs, Clase::Text, &mut cursor);
    colocar(&mut objs, Clase::Ro, &mut cursor);
    let text_end = cursor;

    cursor = align_up(cursor, PAGE);
    let tls_file = cursor;
    let tls_va = base() + cursor;
    colocar(&mut objs, Clase::Tdata, &mut cursor);
    let tls_filesz = cursor - tls_file;
    let mut tls_va_end = base() + cursor;
    colocar_nobits(&mut objs, Clase::Tbss, &mut tls_va_end);
    let tls_memsz = tls_va_end.saturating_sub(tls_va);
    let tls_align = align_tls(&objs).max(8);

    let data_va = align_up(tls_va_end, PAGE);
    cursor = data_va - base();
    let data_file = cursor;
    colocar(&mut objs, Clase::Data, &mut cursor);
    let got_addr = if got_n == 0 {
        0
    } else {
        cursor = align_up(cursor, 8);
        let a = base() + cursor;
        cursor += (got_n as u64) * 8;
        a
    };
    // `-pie`: `.rela.dyn` (una `R_X86_64_RELATIVE` por dirección absoluta) y
    // `.dynamic`, dentro del segmento de datos.
    let (rela_off, nrela, dyn_off) = if pie {
        let n = contar_rels(&objs, &global)?;
        cursor = align_up(cursor, 8);
        let r = cursor;
        cursor += (n as u64) * 24;
        let d = cursor;
        cursor += 4 * 16;
        (r, n, d)
    } else {
        (0, 0, 0)
    };
    let data_filesz = cursor - data_file;
    let mut data_va_end = base() + cursor;
    colocar_nobits(&mut objs, Clase::Bss, &mut data_va_end);
    let data_memsz = data_va_end - data_va;
    let file_end = cursor;

    let mut extra_nombres: Vec<&str> = Vec::new();
    if rustc_meta.is_some() {
        extra_nombres.push(".rustc");
    }
    if compartido {
        extra_nombres.push(".soso.exp");
        extra_nombres.push(".soso.rel");
    }
    let shstr = shstrtab(&objs, &extra_nombres);
    let shstr_off = file_end;
    // `.soso.exp`: u32 n; n × { u64 desplazamiento, u16 largo, nombre }.
    let mut exp: Vec<u8> = Vec::new();
    if compartido {
        let mut entradas: Vec<(u64, &String)> = Vec::new();
        for n in &op.exportar {
            if let Some(Def::Sym { obj, sym }) = global.get(n) {
                let r = resolver(&objs, &global, *obj as usize, *sym)?;
                entradas.push((r.va - base(), n));
            }
        }
        exp.extend_from_slice(&(entradas.len() as u32).to_le_bytes());
        for (off, n) in entradas {
            exp.extend_from_slice(&off.to_le_bytes());
            exp.extend_from_slice(&(n.len() as u16).to_le_bytes());
            exp.extend_from_slice(n.as_bytes());
        }
    }
    let rustc_off = shstr_off + shstr.len() as u64;
    let exp_off = rustc_off + rustc_meta.as_ref().map_or(0, |m| m.len() as u64);
    let file_size = exp_off + exp.len() as u64;

    let mut image = alloc::vec![0u8; file_size as usize];
    volcar(&objs, &mut image)?;
    if let Some(m) = &rustc_meta {
        image[rustc_off as usize..rustc_off as usize + m.len()].copy_from_slice(m);
    }
    image[exp_off as usize..exp_off as usize + exp.len()].copy_from_slice(&exp);
    let tcb = if has_tls && tls_memsz > 0 {
        tls_va + align_up(tls_memsz, tls_align) + if compartido { PM_TLS_BIAS } else { 0 }
    } else {
        0
    };
    let rels = aplicar(&objs, &global, &mut image, got_addr, tcb, compartido || pie, compartido)?;
    let rel_off = image.len() as u64;
    if pie {
        if rels.len() != nrela || rels.iter().any(|r| r & 3 != 0) {
            return Err(String::from("-pie: reubicación de 32 bits o recuento distinto"));
        }
        for (i, r) in rels.iter().enumerate() {
            let at = (r >> 2) as usize;
            let v = u64_at(&image, at);
            let e = rela_off as usize + i * 24;
            put_u64(&mut image, e, at as u64)?;
            put_u64(&mut image, e + 8, R_X86_64_RELATIVE)?;
            put_u64(&mut image, e + 16, v)?;
        }
        let d = dyn_off as usize;
        for (i, (tag, val)) in [
            (DT_RELA, rela_off),
            (DT_RELASZ, nrela as u64 * 24),
            (DT_RELAENT, 24),
            (DT_NULL, 0),
        ]
        .iter()
        .enumerate()
        {
            put_u64(&mut image, d + i * 16, *tag)?;
            put_u64(&mut image, d + i * 16 + 8, *val)?;
        }
    } else {
        for r in &rels {
            image.extend_from_slice(&r.to_le_bytes());
        }
    }
    let mut extras: Vec<Extra> = Vec::new();
    if let Some(m) = &rustc_meta {
        extras.push(Extra { nombre: ".rustc", off: rustc_off, size: m.len() as u64 });
    }
    if compartido {
        extras.push(Extra { nombre: ".soso.exp", off: exp_off, size: exp.len() as u64 });
        extras.push(Extra { nombre: ".soso.rel", off: rel_off, size: (rels.len() * 8) as u64 });
    }

    let entry = if compartido { 0 } else { entrada(&objs, &global)? };
    escribir_cabecera(
        &mut image,
        entry,
        nphdr,
        nshdr,
        text_end,
        has_tls,
        tls_file,
        tls_va,
        tls_filesz,
        tls_memsz,
        tls_align,
        data_file,
        data_va,
        data_filesz,
        data_memsz,
        &objs,
        &shstr,
        shstr_off,
        &extras,
        compartido,
        if pie { Some((dyn_off, 4 * 16)) } else { None },
    )?;

    escribir(&salida, &image)
}

struct Opciones {
    salida: String,
    entradas: Vec<String>,
    compartido: bool,
    /// `-pie`: el kernel (`ET_DYN` a base 0 con `.rela.dyn` y `PT_DYNAMIC`).
    pie: bool,
    /// Símbolos de `--version-script` (sección `global:`).
    exportar: Vec<String>,
}

fn linea(args: &[String]) -> Result<Opciones, String> {
    let mut salida = None;
    let mut entradas = Vec::new();
    let mut compartido = false;
    let mut pie = false;
    let mut script = None;
    // `libsoso::entry!` ya quita argv[0]. El primer argumento es `-flavor`.
    let mut i = 0usize;
    while i < args.len() {
        let a = args[i].as_str();
        let come = matches!(
            a,
            "-o" | "-L" | "-u" | "-e" | "--entry" | "-m" | "-flavor" | "-z" | "-soname"
        );
        if a == "-o" {
            let p = args.get(i + 1).ok_or_else(|| String::from("falta -o"))?;
            salida = Some(p.clone());
            i += 2;
            continue;
        }
        if a == "-shared" {
            compartido = true;
            i += 1;
            continue;
        }
        if a == "-pie" || a == "--pie" {
            pie = true;
            i += 1;
            continue;
        }
        if let Some(p) = a.strip_prefix("--version-script=") {
            script = Some(String::from(p));
            i += 1;
            continue;
        }
        if come {
            i += 2;
            continue;
        }
        if a.starts_with('-') {
            i += 1;
            continue;
        }
        entradas.push(args[i].clone());
        i += 1;
    }
    let salida = salida.ok_or_else(|| String::from("falta -o"))?;
    let mut exportar = Vec::new();
    if let Some(path) = script {
        let texto = leer(&path)?;
        let texto = String::from_utf8_lossy(&texto).into_owned();
        let mut en_global = true;
        for tok in texto.split(|c: char| c.is_whitespace()) {
            match tok {
                "" | "{" | "}" | "}; " => {}
                "global:" => en_global = true,
                "local:" => en_global = false,
                t if t.ends_with(';') && en_global => {
                    let n = t.trim_end_matches(';');
                    if !n.is_empty() && n != "*" && !n.starts_with('}') {
                        exportar.push(String::from(n));
                    }
                }
                _ => {}
            }
        }
    }
    if compartido {
        // El cargador necesita `__soso_dl_init` (montón propio de la copia de libstd).
        exportar.push(String::from("__soso_dl_init"));
    }
    Ok(Opciones { salida, entradas, compartido, pie, exportar })
}

fn cargar(objs: &mut Vec<Obj>, path: &str) -> Result<(), String> {
    let bytes = leer(path)?;
    if bytes.starts_with(b"\x7fELF") {
        if let Some(o) = objeto(bytes)? {
            objs.push(o);
        }
        return Ok(());
    }
    if bytes.starts_with(b"!<arch>\n") {
        return archivo(objs, &bytes);
    }
    Err(alloc::format!("{path}: no es ELF ni archivo"))
}

fn archivo(objs: &mut Vec<Obj>, data: &[u8]) -> Result<(), String> {
    let mut i = 8usize;
    while i + 60 <= data.len() {
        let hdr = &data[i..i + 60];
        let size: usize = core::str::from_utf8(&hdr[48..58])
            .unwrap_or("0")
            .trim()
            .parse()
            .unwrap_or(0);
        let body_at = i + 60;
        if body_at + size > data.len() {
            break;
        }
        let body = &data[body_at..body_at + size];
        if body.starts_with(b"\x7fELF") {
            if let Some(o) = objeto(body.to_vec())? {
                objs.push(o);
            }
        }
        i = body_at + size;
        if i % 2 == 1 {
            i += 1;
        }
    }
    Ok(())
}

fn objeto(bytes: Vec<u8>) -> Result<Option<Obj>, String> {
    if bytes.len() < 64 || bytes[4] != 2 || bytes[5] != 1 {
        return Ok(None);
    }
    let e_shoff = u64_at(&bytes, 40) as usize;
    let e_shentsize = u16_at(&bytes, 58) as usize;
    let e_shnum = u16_at(&bytes, 60) as usize;
    let e_shstrndx = u16_at(&bytes, 62) as usize;
    if e_shentsize < 64 || e_shnum == 0 || e_shoff + e_shnum * e_shentsize > bytes.len() {
        return Ok(None);
    }
    let sh = |i: usize| e_shoff + i * e_shentsize;
    let str_off = u64_at(&bytes, sh(e_shstrndx) + 24) as usize;
    let str_sz = u64_at(&bytes, sh(e_shstrndx) + 32) as usize;
    if str_off + str_sz > bytes.len() {
        return Ok(None);
    }
    let strtab = &bytes[str_off..str_off + str_sz];
    let mut secs = Vec::with_capacity(e_shnum);
    let mut alguno = false;
    for i in 0..e_shnum {
        let o = sh(i);
        let name_off = u32_at(&bytes, o) as usize;
        let name = cstr(strtab, name_off);
        let flags = u64_at(&bytes, o + 8);
        let size = u64_at(&bytes, o + 32);
        // `rmeta.o` sólo trae `.rustc` (no ALLOC): es el que lleva los metadatos
        // que rustc lee del objeto compartido (T80), así que no se descarta.
        if (flags & SHF_ALLOC != 0 && size > 0) || (name == ".rustc" && size > 0) {
            alguno = true;
        }
        let align = u64_at(&bytes, o + 48).max(1);
        secs.push(Sec {
            name,
            ty: u32_at(&bytes, o + 4),
            flags,
            align,
            size,
            file_off: u64_at(&bytes, o + 24),
            relocs: Vec::new(),
            keep: false,
            out_addr: 0,
        });
    }
    if !alguno {
        return Ok(None);
    }
    let mut syms = Vec::new();
    for i in 0..e_shnum {
        if secs[i].ty != SHT_SYMTAB {
            continue;
        }
        let o = sh(i);
        let off = u64_at(&bytes, o + 24) as usize;
        let size = u64_at(&bytes, o + 32) as usize;
        let link = u32_at(&bytes, o + 40) as usize;
        if link >= secs.len() {
            continue;
        }
        let so = sh(link);
        let soff = u64_at(&bytes, so + 24) as usize;
        let ssz = u64_at(&bytes, so + 32) as usize;
        if soff + ssz > bytes.len() || off + size > bytes.len() {
            continue;
        }
        let symstr = &bytes[soff..soff + ssz];
        let n = size / 24;
        for k in 0..n {
            let so = off + k * 24;
            let bind = bytes[so + 4] >> 4;
            let name = if bind != STB_LOCAL || u16_at(&bytes, so + 6) == SHN_UNDEF {
                cstr(symstr, u32_at(&bytes, so) as usize)
            } else {
                String::new()
            };
            syms.push(Sym {
                name,
                shndx: u16_at(&bytes, so + 6),
                value: u64_at(&bytes, so + 8),
                info: bytes[so + 4],
            });
        }
    }
    for i in 0..e_shnum {
        if secs[i].ty != SHT_RELA {
            continue;
        }
        let o = sh(i);
        let info = u32_at(&bytes, o + 44) as usize;
        if info >= secs.len() {
            continue;
        }
        let off = u64_at(&bytes, o + 24) as usize;
        let size = u64_at(&bytes, o + 32) as usize;
        if off + size > bytes.len() {
            continue;
        }
        let n = size / 24;
        for k in 0..n {
            let ro = off + k * 24;
            let info_r = u64_at(&bytes, ro + 8);
            secs[info].relocs.push(Reloc {
                offset: u64_at(&bytes, ro),
                sym: (info_r >> 32) as u32,
                typ: info_r as u32,
                addend: i64_at(&bytes, ro + 16),
            });
        }
    }
    Ok(Some(Obj { bytes, secs, syms }))
}

fn stub_ret(nombre: &str) -> Obj {
    Obj {
        bytes: alloc::vec![0xc3],
        secs: alloc::vec![
            Sec {
                name: String::new(),
                ty: 0,
                flags: 0,
                align: 1,
                size: 0,
                file_off: 0,
                relocs: Vec::new(),
                keep: false,
                out_addr: 0,
            },
            Sec {
                name: alloc::format!(".text.{nombre}"),
                ty: SHT_PROGBITS,
                flags: SHF_ALLOC | SHF_EXECINSTR,
                align: 1,
                size: 1,
                file_off: 0,
                relocs: Vec::new(),
                keep: false,
                out_addr: 0,
            },
        ],
        syms: alloc::vec![Sym {
            name: String::from(nombre),
            shndx: 1,
            value: 0,
            info: (STB_GLOBAL << 4) | 2,
        }],
    }
}

fn simbolos(objs: &[Obj]) -> Result<BTreeMap<String, Def>, String> {
    let mut map = BTreeMap::new();
    for (oi, obj) in objs.iter().enumerate() {
        for (si, sym) in obj.syms.iter().enumerate() {
            let bind = sym.info >> 4;
            let typ = sym.info & 0xf;
            if bind == STB_LOCAL || typ == STT_FILE || typ == STT_SECTION || sym.name.is_empty() {
                continue;
            }
            if sym.shndx == SHN_UNDEF {
                continue;
            }
            if sym.shndx != SHN_ABS {
                let sec = obj.secs.get(sym.shndx as usize);
                if sec.is_none_or(|s| s.flags & SHF_ALLOC == 0) {
                    continue;
                }
            }
            let def = Def::Sym { obj: oi as u32, sym: si as u32 };
            if let Some(prev) = map.get(&sym.name) {
                let prev_bind = match prev {
                    Def::Abs(_) => STB_GLOBAL,
                    Def::Sym { obj, sym } => objs[*obj as usize].syms[*sym as usize].info >> 4,
                };
                if prev_bind == STB_GLOBAL && bind == STB_GLOBAL {
                    return Err(alloc::format!("símbolo duplicado {}", sym.name));
                }
                if prev_bind == STB_GLOBAL && bind == STB_WEAK {
                    continue;
                }
            }
            map.insert(sym.name.clone(), def);
        }
    }
    Ok(map)
}

fn marcar(
    objs: &mut [Obj],
    global: &BTreeMap<String, Def>,
    compartido: bool,
    pie: bool,
    exportar: &[String],
) {
    let mut cola = Vec::new();
    for (oi, obj) in objs.iter().enumerate() {
        for (si, sec) in obj.secs.iter().enumerate() {
            let raiz = sec.ty == SHT_INIT_ARRAY
                || sec.ty == SHT_FINI_ARRAY
                || sec.ty == SHT_PREINIT_ARRAY
                || (!compartido && sec.name == ".text._start")
                || (pie && sec.name == ".bootloader-config");
            if raiz && sec.flags & SHF_ALLOC != 0 && sec.size > 0 {
                cola.push((oi, si));
            }
        }
    }
    // Un objeto compartido no tiene `_start`: sus raíces son lo que exporta.
    let raices: Vec<&str> = if compartido {
        exportar.iter().map(|n| n.as_str()).collect()
    } else {
        alloc::vec!["_start", "main"]
    };
    for nombre in raices {
        if let Some(def) = global.get(nombre) {
            if let Some((oi, si)) = seccion_def(objs, def) {
                cola.push((oi, si));
            }
        }
    }
    let mut vistos = BTreeSet::new();
    while let Some((oi, si)) = cola.pop() {
        if !vistos.insert((oi, si)) {
            continue;
        }
        // Una sección vacía a la que apunta una reubicación (un `static` de
        // longitud 0) se conserva: sin ella la reubicación no tiene dirección.
        if objs[oi].secs[si].flags & SHF_ALLOC == 0 {
            continue;
        }
        objs[oi].secs[si].keep = true;
        let relocs = objs[oi].secs[si].relocs.clone();
        for rel in relocs {
            if let Some((to, ts)) = seccion_sym(objs, global, oi, rel.sym) {
                cola.push((to, ts));
            }
        }
    }
}

fn seccion_def(objs: &[Obj], def: &Def) -> Option<(usize, usize)> {
    match def {
        Def::Abs(_) => None,
        Def::Sym { obj, sym } => {
            let s = objs.get(*obj as usize)?.syms.get(*sym as usize)?;
            if s.shndx == SHN_UNDEF || s.shndx == SHN_ABS {
                return None;
            }
            Some((*obj as usize, s.shndx as usize))
        }
    }
}

fn seccion_sym(
    objs: &[Obj],
    global: &BTreeMap<String, Def>,
    oi: usize,
    sym_i: u32,
) -> Option<(usize, usize)> {
    let sym = objs.get(oi)?.syms.get(sym_i as usize)?;
    if sym.shndx == SHN_ABS {
        return None;
    }
    if sym.shndx != SHN_UNDEF {
        return Some((oi, sym.shndx as usize));
    }
    let def = global.get(&sym.name)?;
    seccion_def(objs, def)
}

fn es_got(typ: u32) -> bool {
    matches!(
        typ,
        R_X86_64_GOTTPOFF | R_X86_64_GOTPCREL | R_X86_64_GOTPCRELX | R_X86_64_REX_GOTPCRELX
    )
}

fn contar_got(objs: &[Obj]) -> usize {
    objs.iter()
        .flat_map(|o| o.secs.iter())
        .filter(|s| s.keep)
        .flat_map(|s| s.relocs.iter())
        .filter(|r| es_got(r.typ))
        .count()
}

fn clase(sec: &Sec) -> Option<Clase> {
    if !sec.keep || sec.flags & SHF_ALLOC == 0 || sec.size == 0 {
        return None;
    }
    if sec.flags & SHF_TLS != 0 {
        return Some(if sec.ty == SHT_NOBITS { Clase::Tbss } else { Clase::Tdata });
    }
    if sec.ty == SHT_NOBITS {
        return Some(Clase::Bss);
    }
    if sec.flags & SHF_EXECINSTR != 0 {
        return Some(Clase::Text);
    }
    if sec.flags & SHF_WRITE != 0 {
        return Some(Clase::Data);
    }
    Some(Clase::Ro)
}

fn misma(c: &Clase, s: &Sec) -> bool {
    match (c, clase(s)) {
        (Clase::Text, Some(Clase::Text)) => true,
        (Clase::Ro, Some(Clase::Ro)) => true,
        (Clase::Tdata, Some(Clase::Tdata)) => true,
        (Clase::Tbss, Some(Clase::Tbss)) => true,
        (Clase::Data, Some(Clase::Data)) => true,
        (Clase::Bss, Some(Clase::Bss)) => true,
        _ => false,
    }
}

fn colocar(objs: &mut [Obj], cual: Clase, cursor: &mut u64) {
    // `_start` primero, para que el segmento ejecutable lo cubra.
    if matches!(cual, Clase::Text) {
        for obj in objs.iter_mut() {
            for sec in obj.secs.iter_mut() {
                if sec.name == ".text._start" && misma(&cual, sec) {
                    *cursor = align_up(*cursor, sec.align);
                    sec.out_addr = base() + *cursor;
                    *cursor += sec.size;
                }
            }
        }
    }
    for obj in objs.iter_mut() {
        for sec in obj.secs.iter_mut() {
            if sec.name == ".text._start" && matches!(cual, Clase::Text) {
                continue;
            }
            if !misma(&cual, sec) {
                continue;
            }
            *cursor = align_up(*cursor, sec.align);
            sec.out_addr = base() + *cursor;
            *cursor += sec.size;
        }
    }
}

fn colocar_nobits(objs: &mut [Obj], cual: Clase, va_end: &mut u64) {
    for obj in objs.iter_mut() {
        for sec in obj.secs.iter_mut() {
            if !misma(&cual, sec) {
                continue;
            }
            let addr = align_up(*va_end, sec.align);
            sec.out_addr = addr;
            *va_end = addr + sec.size;
        }
    }
}

fn align_tls(objs: &[Obj]) -> u64 {
    objs.iter()
        .flat_map(|o| o.secs.iter())
        .filter(|s| s.keep && s.flags & SHF_TLS != 0)
        .map(|s| s.align)
        .max()
        .unwrap_or(8)
}

fn volcar(objs: &[Obj], image: &mut [u8]) -> Result<(), String> {
    for obj in objs {
        for sec in &obj.secs {
            if !sec.keep || sec.ty == SHT_NOBITS || sec.out_addr < base() {
                continue;
            }
            let off = (sec.out_addr - base()) as usize;
            let src = sec.file_off as usize;
            let n = sec.size as usize;
            if src + n > obj.bytes.len() || off + n > image.len() {
                return Err(alloc::format!("sección {} fuera de rango", sec.name));
            }
            image[off..off + n].copy_from_slice(&obj.bytes[src..src + n]);
        }
    }
    Ok(())
}

struct Resuelto {
    va: u64,
    tls: bool,
    /// Es una dirección de la imagen (no un `SHN_ABS` ni un débil indefinido):
    /// en un objeto compartido hay que anotarla para que el cargador la ajuste.
    reubicable: bool,
}

fn resolver(
    objs: &[Obj],
    global: &BTreeMap<String, Def>,
    oi: usize,
    sym_i: u32,
) -> Result<Resuelto, String> {
    let sym = objs
        .get(oi)
        .and_then(|o| o.syms.get(sym_i as usize))
        .ok_or_else(|| String::from("reubicación con símbolo inválido"))?;
    if sym.shndx == SHN_ABS {
        return Ok(Resuelto { va: sym.value, tls: false, reubicable: false });
    }
    if sym.shndx != SHN_UNDEF {
        let sec = objs[oi]
            .secs
            .get(sym.shndx as usize)
            .ok_or_else(|| String::from("sección de símbolo inválida"))?;
        if !sec.keep {
            return Err(alloc::format!(
                "reubicación a sección descartada {}",
                sec.name
            ));
        }
        let tls = sec.flags & SHF_TLS != 0 || (sym.info & 0xf) == STT_TLS;
        return Ok(Resuelto { va: sec.out_addr + sym.value, tls, reubicable: !tls });
    }
    let bind = sym.info >> 4;
    let Some(def) = global.get(&sym.name) else {
        if bind == STB_WEAK {
            return Ok(Resuelto { va: 0, tls: false, reubicable: false });
        }
        return Err(alloc::format!("indefinido: {}", sym.name));
    };
    match def {
        Def::Abs(v) => Ok(Resuelto { va: *v, tls: false, reubicable: true }),
        Def::Sym { obj, sym: si } => resolver(objs, global, *obj as usize, *si),
    }
}

fn aplicar(
    objs: &[Obj],
    global: &BTreeMap<String, Def>,
    image: &mut [u8],
    mut got_addr: u64,
    tcb: u64,
    compartido: bool,
    tpoff: bool,
) -> Result<Vec<u64>, String> {
    // Objeto compartido: cada dirección absoluta que se escribe se anota como
    // `(desplazamiento << 2) | tipo` (0 = u64, 1 = u32) para `.soso.rel`.
    let mut rels: Vec<u64> = Vec::new();
    for (oi, obj) in objs.iter().enumerate() {
        for sec in &obj.secs {
            if !sec.keep || sec.ty == SHT_NOBITS {
                continue;
            }
            for rel in &sec.relocs {
                let r = resolver(objs, global, oi, rel.sym)?;
                let p = sec.out_addr + rel.offset;
                let at = (p - base()) as usize;
                match rel.typ {
                    R_X86_64_64 => {
                        let v = (r.va as i64).wrapping_add(rel.addend) as u64;
                        write_u64(image, at, v)?;
                        if compartido && r.reubicable {
                            rels.push((at as u64) << 2);
                        }
                    }
                    R_X86_64_PC32 | R_X86_64_PLT32 => {
                        let v = (r.va as i64).wrapping_add(rel.addend).wrapping_sub(p as i64);
                        write_i32(image, at, v)?;
                    }
                    R_X86_64_32 => {
                        let v = (r.va as i64).wrapping_add(rel.addend) as u64;
                        if v > u32::MAX as u64 {
                            return Err(String::from("R_X86_64_32 no cabe"));
                        }
                        write_u32(image, at, v as u32)?;
                        if compartido && r.reubicable {
                            rels.push(((at as u64) << 2) | 1);
                        }
                    }
                    R_X86_64_32S => {
                        let v = (r.va as i64).wrapping_add(rel.addend);
                        write_i32(image, at, v)?;
                        if compartido && r.reubicable {
                            rels.push(((at as u64) << 2) | 1);
                        }
                    }
                    R_X86_64_TPOFF32 => {
                        if tcb == 0 {
                            return Err(String::from("TPOFF32 sin TLS"));
                        }
                        let v = (r.va as i64).wrapping_add(rel.addend).wrapping_sub(tcb as i64);
                        write_i32(image, at, v)?;
                        if tpoff {
                            rels.push(((at as u64) << 2) | 2);
                        }
                    }
                    R_X86_64_GOTTPOFF => {
                        if tcb == 0 || got_addr == 0 {
                            return Err(String::from("GOTTPOFF sin GOT"));
                        }
                        let slot = got_addr;
                        got_addr += 8;
                        let tp = (r.va as i64).wrapping_sub(tcb as i64);
                        write_u64(image, (slot - base()) as usize, tp as u64)?;
                        if tpoff {
                            rels.push(((slot - base()) << 2) | 3);
                        }
                        let v = (slot as i64).wrapping_add(rel.addend).wrapping_sub(p as i64);
                        write_i32(image, at, v)?;
                    }
                    R_X86_64_GOTPCREL | R_X86_64_GOTPCRELX | R_X86_64_REX_GOTPCRELX => {
                        if got_addr == 0 {
                            return Err(String::from("GOTPCREL sin GOT"));
                        }
                        let slot = got_addr;
                        got_addr += 8;
                        write_u64(image, (slot - base()) as usize, r.va)?;
                        if compartido && r.reubicable {
                            rels.push((slot - base()) << 2);
                        }
                        let v = (slot as i64).wrapping_add(rel.addend).wrapping_sub(p as i64);
                        write_i32(image, at, v)?;
                    }
                    otro => {
                        return Err(alloc::format!("reubicación {otro} no soportada"));
                    }
                }
            }
        }
    }
    Ok(rels)
}

/// Número de direcciones absolutas que `aplicar` anotará: `R_X86_64_64` y
/// ranuras de GOT que apuntan a la imagen (lo que no es TLS ni `SHN_ABS`).
fn contar_rels(objs: &[Obj], global: &BTreeMap<String, Def>) -> Result<usize, String> {
    let mut n = 0;
    for (oi, obj) in objs.iter().enumerate() {
        for sec in &obj.secs {
            if !sec.keep || sec.ty == SHT_NOBITS {
                continue;
            }
            for rel in &sec.relocs {
                let cuenta = matches!(
                    rel.typ,
                    R_X86_64_64
                        | R_X86_64_32
                        | R_X86_64_32S
                        | R_X86_64_GOTPCREL
                        | R_X86_64_GOTPCRELX
                        | R_X86_64_REX_GOTPCRELX
                );
                if cuenta && resolver(objs, global, oi, rel.sym)?.reubicable {
                    n += 1;
                }
            }
        }
    }
    Ok(n)
}

fn entrada(objs: &[Obj], global: &BTreeMap<String, Def>) -> Result<u64, String> {
    for nombre in ["_start", "main"] {
        if let Some(def) = global.get(nombre) {
            if let Def::Sym { obj, sym } = def {
                let s = &objs[*obj as usize].syms[*sym as usize];
                if s.shndx != SHN_UNDEF && s.shndx != SHN_ABS {
                    let sec = &objs[*obj as usize].secs[s.shndx as usize];
                    if sec.keep && sec.flags & SHF_EXECINSTR != 0 {
                        return Ok(sec.out_addr + s.value);
                    }
                }
            }
        }
    }
    Err(String::from("no está _start"))
}

struct Extra {
    nombre: &'static str,
    off: u64,
    size: u64,
}

fn escribir_cabecera(
    image: &mut [u8],
    entry: u64,
    nphdr: usize,
    nshdr: usize,
    text_end: u64,
    has_tls: bool,
    tls_file: u64,
    tls_va: u64,
    tls_filesz: u64,
    tls_memsz: u64,
    tls_align: u64,
    data_file: u64,
    data_va: u64,
    data_filesz: u64,
    data_memsz: u64,
    objs: &[Obj],
    shstr: &[u8],
    shstr_off: u64,
    extras: &[Extra],
    compartido: bool,
    dinamica: Option<(u64, u64)>,
) -> Result<(), String> {
    image[0..4].copy_from_slice(b"\x7fELF");
    image[4] = 2;
    image[5] = 1;
    image[6] = 1;
    put_u16(image, 16, if compartido || dinamica.is_some() { 3 } else { 2 });
    put_u16(image, 18, 62);
    put_u32(image, 20, 1);
    put_u64(image, 24, entry)?;
    put_u64(image, 32, 64)?;
    let shoff = 64 + nphdr * 56;
    put_u64(image, 40, shoff as u64)?;
    put_u16(image, 52, 64);
    put_u16(image, 54, 56);
    put_u16(image, 56, nphdr as u16);
    put_u16(image, 58, 64);
    put_u16(image, 60, nshdr as u16);
    put_u16(image, 62, (nshdr - 1) as u16);

    let mut ph = 64usize;
    // RX: cabecera + texto + rodata.
    phdr(image, &mut ph, PT_LOAD, PF_R | PF_X, 0, base(), text_end, text_end, PAGE)?;
    if has_tls {
        phdr(
            image,
            &mut ph,
            PT_TLS,
            PF_R,
            tls_file,
            tls_va,
            tls_filesz,
            tls_memsz,
            tls_align,
        )?;
    }
    if data_memsz > 0 {
        phdr(
            image,
            &mut ph,
            PT_LOAD,
            PF_R | PF_W,
            data_file,
            data_va,
            data_filesz,
            data_memsz,
            PAGE,
        )?;
    }

    if let Some((off, len)) = dinamica {
        phdr(image, &mut ph, PT_DYNAMIC, PF_R | PF_W, off, off, len, len, 8)?;
    }

    let mut name_off = BTreeMap::<String, u32>::new();
    let mut cursor_n = 1u32;
    for obj in objs {
        for sec in &obj.secs {
            if !sec.keep {
                continue;
            }
            name_off.entry(sec.name.clone()).or_insert_with(|| {
                let o = cursor_n;
                cursor_n += sec.name.len() as u32 + 1;
                o
            });
        }
    }

    let mut sh = shoff;
    // NULL
    sh += 64;
    for obj in objs {
        for sec in &obj.secs {
            if !sec.keep {
                continue;
            }
            let name = *name_off.get(&sec.name).unwrap_or(&0);
            put_u32(image, sh, name);
            put_u32(image, sh + 4, sec.ty);
            put_u64(image, sh + 8, sec.flags)?;
            put_u64(image, sh + 16, sec.out_addr)?;
            let foff = if sec.ty == SHT_NOBITS { 0 } else { sec.out_addr - base() };
            put_u64(image, sh + 24, foff)?;
            put_u64(image, sh + 32, sec.size)?;
            put_u64(image, sh + 48, sec.align)?;
            sh += 64;
        }
    }
    for e in extras {
        let nombre = buscar_nombre(shstr, e.nombre);
        put_u32(image, sh, nombre);
        put_u32(image, sh + 4, SHT_PROGBITS);
        put_u64(image, sh + 24, e.off)?;
        put_u64(image, sh + 32, e.size)?;
        put_u64(image, sh + 48, 1)?;
        sh += 64;
    }
    // .shstrtab
    put_u32(image, sh, 0);
    put_u32(image, sh + 4, 3);
    put_u64(image, sh + 24, shstr_off)?;
    put_u64(image, sh + 32, shstr.len() as u64)?;
    put_u64(image, sh + 48, 1)?;
    if shstr_off as usize + shstr.len() <= image.len() {
        image[shstr_off as usize..shstr_off as usize + shstr.len()].copy_from_slice(shstr);
    }
    Ok(())
}

/// Desplazamiento de `nombre` en la tabla de nombres de sección.
fn buscar_nombre(shstr: &[u8], nombre: &str) -> u32 {
    let n = nombre.as_bytes();
    let mut i = 1usize;
    while i < shstr.len() {
        let fin = shstr[i..].iter().position(|&b| b == 0).map_or(shstr.len(), |p| i + p);
        if &shstr[i..fin] == n {
            return i as u32;
        }
        i = fin + 1;
    }
    0
}

fn shstrtab(objs: &[Obj], extras: &[&str]) -> Vec<u8> {
    let mut out = Vec::new();
    out.push(0);
    let mut seen = BTreeSet::new();
    for obj in objs {
        for sec in &obj.secs {
            if !sec.keep || !seen.insert(sec.name.clone()) {
                continue;
            }
            out.extend_from_slice(sec.name.as_bytes());
            out.push(0);
        }
    }
    for e in extras {
        out.extend_from_slice(e.as_bytes());
        out.push(0);
    }
    out.extend_from_slice(b".shstrtab\0");
    out
}

fn phdr(
    image: &mut [u8],
    at: &mut usize,
    ty: u32,
    flags: u32,
    off: u64,
    va: u64,
    filesz: u64,
    memsz: u64,
    align: u64,
) -> Result<(), String> {
    let o = *at;
    put_u32(image, o, ty);
    put_u32(image, o + 4, flags);
    put_u64(image, o + 8, off)?;
    put_u64(image, o + 16, va)?;
    put_u64(image, o + 24, va)?;
    put_u64(image, o + 32, filesz)?;
    put_u64(image, o + 40, memsz)?;
    put_u64(image, o + 48, align)?;
    *at += 56;
    Ok(())
}

fn align_up(x: u64, a: u64) -> u64 {
    let a = a.max(1);
    x.wrapping_add(a - 1) & !(a - 1)
}

fn leer(path: &str) -> Result<Vec<u8>, String> {
    let mut st = abi::Stat::default();
    let rc = sys::stat(path, &mut st);
    if rc < 0 {
        return Err(alloc::format!("{path}: no se abre"));
    }
    let fd = sys::open(path, abi::O_RDONLY);
    if fd < 0 {
        return Err(alloc::format!("{path}: no se abre"));
    }
    let mut buf = alloc::vec![0u8; st.size as usize];
    let mut off = 0usize;
    let mut fallo = None;
    while off < buf.len() {
        let fin = (off + (4 << 20)).min(buf.len());
        let n = sys::read(fd as u64, &mut buf[off..fin]);
        if n <= 0 {
            fallo = Some(n);
            break;
        }
        off += n as usize;
    }
    sys::close(fd as u64);
    if let Some(rc) = fallo {
        return Err(alloc::format!(
            "{path}: lectura corta (rc={rc}, {off} de {} bytes)",
            buf.len()
        ));
    }
    Ok(buf)
}

fn escribir(path: &str, buf: &[u8]) -> Result<(), String> {
    let fd = sys::open(path, abi::O_WRONLY | abi::O_CREAT | abi::O_TRUNC);
    if fd < 0 {
        return Err(alloc::format!("{path}: no se crea"));
    }
    let r = sys::write_all(fd as u64, buf);
    sys::close(fd as u64);
    r.map_err(|_| alloc::format!("{path}: escritura corta"))?;
    Ok(())
}

fn cstr(buf: &[u8], off: usize) -> String {
    if off >= buf.len() {
        return String::new();
    }
    let end = buf[off..].iter().position(|b| *b == 0).unwrap_or(buf.len() - off);
    String::from_utf8_lossy(&buf[off..off + end]).into_owned()
}

fn u16_at(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes(b[o..o + 2].try_into().unwrap_or([0, 0]))
}
fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap_or([0; 4]))
}
fn u64_at(b: &[u8], o: usize) -> u64 {
    u64::from_le_bytes(b[o..o + 8].try_into().unwrap_or([0; 8]))
}
fn i64_at(b: &[u8], o: usize) -> i64 {
    i64::from_le_bytes(b[o..o + 8].try_into().unwrap_or([0; 8]))
}

fn put_u16(b: &mut [u8], o: usize, v: u16) {
    if o + 2 <= b.len() {
        b[o..o + 2].copy_from_slice(&v.to_le_bytes());
    }
}
fn put_u32(b: &mut [u8], o: usize, v: u32) {
    if o + 4 <= b.len() {
        b[o..o + 4].copy_from_slice(&v.to_le_bytes());
    }
}
fn put_u64(b: &mut [u8], o: usize, v: u64) -> Result<(), String> {
    if o + 8 > b.len() {
        return Err(String::from("cabecera fuera de la imagen"));
    }
    b[o..o + 8].copy_from_slice(&v.to_le_bytes());
    Ok(())
}
fn write_u32(b: &mut [u8], o: usize, v: u32) -> Result<(), String> {
    if o + 4 > b.len() {
        return Err(String::from("reubicación fuera de la imagen"));
    }
    b[o..o + 4].copy_from_slice(&v.to_le_bytes());
    Ok(())
}
fn write_u64(b: &mut [u8], o: usize, v: u64) -> Result<(), String> {
    if o + 8 > b.len() {
        return Err(String::from("reubicación fuera de la imagen"));
    }
    b[o..o + 8].copy_from_slice(&v.to_le_bytes());
    Ok(())
}
fn write_i32(b: &mut [u8], o: usize, v: i64) -> Result<(), String> {
    if v < i32::MIN as i64 || v > i32::MAX as i64 {
        return Err(String::from("reubicación de 32 bits no cabe"));
    }
    write_u32(b, o, v as u32)
}
