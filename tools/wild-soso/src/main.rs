//! Wrapper de **wild** para el target soso. Si `wild` está en PATH lo invoca;
//! si no, falla con instrucciones (el port completo vendrá en iteraciones).

use std::env;
use std::fs::OpenOptions;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

/// `mov $1, %eax; syscall` dentro de `__libc_write`. En soso el 1 es `read`.
const SYSCALL_WRITE_LINUX: [u8; 7] = [0xb8, 0x01, 0x00, 0x00, 0x00, 0x0f, 0x05];

/// Cambia el inmediato de esas syscall de 1 a 2 (`SYS_WRITE` de soso).
fn sustituir_syscall_write(codigo: &mut [u8]) -> usize {
    let mut n = 0;
    let mut i = 0;
    while i + SYSCALL_WRITE_LINUX.len() <= codigo.len() {
        if codigo[i..i + SYSCALL_WRITE_LINUX.len()] == SYSCALL_WRITE_LINUX {
            codigo[i + 1] = 0x02;
            n += 1;
            i += SYSCALL_WRITE_LINUX.len();
        } else {
            i += 1;
        }
    }
    n
}

/// `-Wl,a,b` es lo que gcc le pasaría al enlazador como dos argumentos, `a` y `b`.
fn expand_wl(args: &[String]) -> Vec<String> {
    let mut out = Vec::with_capacity(args.len());
    for arg in args {
        if let Some(rest) = arg.strip_prefix("-Wl,") {
            out.extend(rest.split(',').filter(|part| !part.is_empty()).map(str::to_string));
        } else {
            out.push(arg.clone());
        }
    }
    out
}

/// `cc -print-file-name=<file>` devuelve la ruta absoluta, o el nombre si no
/// la encuentra.
fn cc_file_path(file: &str) -> Option<String> {
    let output = Command::new("cc")
        .arg(format!("-print-file-name={file}"))
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let path = String::from_utf8(output.stdout).ok()?;
    let path = path.trim();
    if path.is_empty() || path == file || !path.starts_with('/') {
        return None;
    }
    Some(path.to_string())
}

fn cc_file_dir(file: &str) -> Option<String> {
    cc_file_path(file).and_then(|path| {
        Path::new(&path)
            .parent()
            .map(|dir| dir.to_string_lossy().into_owned())
    })
}

fn stdcxx_dir() -> Option<String> {
    cc_file_dir("libstdc++.so")
}

fn libc_dir() -> Option<String> {
    cc_file_dir("libc.so")
}

fn libz_dir() -> Option<String> {
    cc_file_dir("libz.so")
}

fn libm_dir() -> Option<String> {
    cc_file_dir("libm.so")
}

fn openssl_lib_dir() -> Option<String> {
    cc_file_dir("libssl.so").or_else(|| cc_file_dir("libssl.a"))
}

/// `libssh2-sys` / `libgit2-sys` enlazan OpenSSL; rustc no pasa `-lssl`/`-lcrypto`.
fn with_openssl_libs(args: &[String]) -> Vec<String> {
    let needs = args.iter().any(|a| {
        a.contains("libssh2_sys") || a.contains("libgit2_sys") || a.contains("libcurl_sys")
    });
    if !needs || args.iter().any(|a| a == "-lssl") {
        return args.to_vec();
    }
    let Some(dir) = openssl_lib_dir() else {
        return args.to_vec();
    };
    let mut out = args.to_vec();
    if out.last().map(String::as_str) != Some("-Bstatic") {
        out.push("-Bstatic".to_string());
    }
    out.push("-L".to_string());
    out.push(dir);
    out.push("-lssl".to_string());
    out.push("-lcrypto".to_string());
    out
}

/// rustc pide `-lstdc++` sin el directorio de gcc. wild no lo busca.
fn with_stdcxx_dir(args: &[String], dir: Option<&str>) -> Vec<String> {
    let Some(dir) = dir else {
        return args.to_vec();
    };
    if !args.iter().any(|arg| arg == "-lstdc++") {
        return args.to_vec();
    }
    let mut out = Vec::with_capacity(args.len() + 2);
    let mut inserted = false;
    for arg in args {
        if !inserted && arg == "-lstdc++" {
            out.push("-L".to_string());
            out.push(dir.to_string());
            inserted = true;
        }
        out.push(arg.clone());
    }
    out
}

/// LLVM pide `free` y rustc no pasa `-lc`. La libc va detrás de `-lstdc++`.
fn with_host_libc(args: &[String], dir: Option<&str>) -> Vec<String> {
    let Some(dir) = dir else {
        return args.to_vec();
    };
    if args.iter().any(|arg| arg == "-lc") || !args.iter().any(|arg| arg == "-lstdc++") {
        return args.to_vec();
    }
    let mut out = Vec::with_capacity(args.len() + 3);
    let mut inserted = false;
    for arg in args {
        out.push(arg.clone());
        if !inserted && arg == "-lstdc++" {
            out.push("-L".to_string());
            out.push(dir.to_string());
            out.push("-lc".to_string());
            inserted = true;
        }
    }
    out
}

/// rustc pasa `-lz`/`-lc`/… sin `-L` cuando no hay `-lstdc++` (p. ej. bin `cargo`).
fn with_lib_dir_before(args: &[String], lib_flag: &str, dir: Option<&str>) -> Vec<String> {
    let Some(dir) = dir else {
        return args.to_vec();
    };
    if !args.iter().any(|arg| arg == lib_flag) {
        return args.to_vec();
    }
    let mut out = Vec::with_capacity(args.len() + 2);
    let mut inserted = false;
    for arg in args {
        if !inserted && arg == lib_flag {
            out.push("-L".to_string());
            out.push(dir.to_string());
            inserted = true;
        }
        out.push(arg.clone());
    }
    out
}

/// Directorios del host para `-lz`, `-lc`, `-lm`, `-lrt` y `-lpthread` ya presentes.
fn with_host_lib_search_dirs(args: &[String]) -> Vec<String> {
    let mut out = args.to_vec();
    for (flag, dir) in [
        ("-lz", libz_dir()),
        ("-lc", libc_dir()),
        ("-lm", libm_dir()),
        ("-lrt", libc_dir()),
        ("-lpthread", libc_dir()),
    ] {
        out = with_lib_dir_before(&out, flag, dir.as_deref());
    }
    out
}

/// `crc32` está en libz. LLVM lo usa y rustc no pasa `-lz`.
fn with_host_libz(args: &[String], dir: Option<&str>) -> Vec<String> {
    let Some(dir) = dir else {
        return args.to_vec();
    };
    if args.iter().any(|arg| arg == "-lz") || !args.iter().any(|arg| arg == "-lstdc++") {
        return args.to_vec();
    }
    let mut out = Vec::with_capacity(args.len() + 3);
    let mut inserted = false;
    for arg in args {
        out.push(arg.clone());
        if !inserted && arg == "-lstdc++" {
            out.push("-L".to_string());
            out.push(dir.to_string());
            out.push("-lz".to_string());
            inserted = true;
        }
    }
    out
}

/// `logb` está en libm. LLVM lo usa y rustc no pasa `-lm`.
fn with_host_libm(args: &[String], dir: Option<&str>) -> Vec<String> {
    let Some(dir) = dir else {
        return args.to_vec();
    };
    if args.iter().any(|arg| arg == "-lm") || !args.iter().any(|arg| arg == "-lstdc++") {
        return args.to_vec();
    }
    let mut out = Vec::with_capacity(args.len() + 3);
    let mut inserted = false;
    for arg in args {
        out.push(arg.clone());
        if !inserted && arg == "-lstdc++" {
            out.push("-L".to_string());
            out.push(dir.to_string());
            out.push("-lm".to_string());
            inserted = true;
        }
    }
    out
}

/// Enlaces con bibliotecas C/C++ del host (con o sin `-lstdc++`) necesitan crt de gcc.
fn needs_gcc_crt(args: &[String]) -> bool {
    args.iter().any(|arg| arg == "-lstdc++")
        || args.iter().any(|a| a.contains("libssh2_sys"))
        || args
            .windows(2)
            .any(|w| w[0] == "-Bdynamic" && matches!(w[1].as_str(), "-lz" | "-lc"))
}

/// `_Unwind_Resume` y `__register_frame` están en `libgcc_eh.a`. rustc no lo pasa.
fn with_host_libgcc_eh(args: &[String], archive: Option<&str>) -> Vec<String> {
    let Some(archive) = archive else {
        return args.to_vec();
    };
    let need = args.iter().any(|arg| arg == "-lstdc++") || needs_gcc_crt(args);
    if !need || args.iter().any(|arg| arg == "-lgcc_eh" || arg.ends_with("libgcc_eh.a")) {
        return args.to_vec();
    }
    let anchor = if args.iter().any(|arg| arg == "-lstdc++") {
        "-lstdc++"
    } else if args.iter().any(|arg| arg == "-lpthread") {
        "-lpthread"
    } else if args.iter().any(|arg| arg == "-lrt") {
        "-lrt"
    } else {
        "-lc"
    };
    let mut out = Vec::with_capacity(args.len() + 1);
    let mut inserted = false;
    for arg in args {
        out.push(arg.clone());
        if !inserted && arg == anchor {
            out.push(archive.to_string());
            inserted = true;
        }
    }
    if !inserted {
        out.push(archive.to_string());
    }
    out
}

/// `__dso_handle` está en `crtbegin.o`. gcc lo mete; rustc no.
fn with_crtbegin(args: &[String], crtbegin: Option<&str>) -> Vec<String> {
    let Some(crtbegin) = crtbegin else {
        return args.to_vec();
    };
    if !needs_gcc_crt(args) || args.iter().any(|arg| arg.ends_with("crtbegin.o")) {
        return args.to_vec();
    }
    if args.iter().any(|arg| arg == "-lstdc++") {
        let mut out = Vec::with_capacity(args.len() + 1);
        let mut inserted = false;
        for arg in args {
            if !inserted && arg == "-lstdc++" {
                out.push(crtbegin.to_string());
                inserted = true;
            }
            out.push(arg.clone());
        }
        return out;
    }
    if let Some(pos) = args
        .windows(2)
        .position(|w| w[0] == "-flavor" && !w[1].starts_with('-'))
    {
        let mut out = Vec::with_capacity(args.len() + 1);
        let end = pos + 2;
        out.extend_from_slice(&args[..end]);
        out.push(crtbegin.to_string());
        out.extend_from_slice(&args[end..]);
        return out;
    }
    let mut out = Vec::with_capacity(args.len() + 1);
    out.push(crtbegin.to_string());
    out.extend(args.iter().cloned());
    out
}

/// Las bibliotecas del host que pide LLVM van estáticas: el guest no tiene
/// el cargador de Linux. rustc ya pasa un `-Bstatic` anterior para los rlib
/// y luego `-Bdynamic` delante de `-lstdc++` o de `-lz`/`-lc` (binarios sin C++).
fn with_static_host_libs(args: &[String]) -> Vec<String> {
    let want = args.iter().any(|arg| arg == "-lstdc++")
        || args
            .windows(2)
            .any(|w| w[0] == "-Bdynamic" && matches!(w[1].as_str(), "-lz" | "-lc"))
        || (needs_gcc_crt(args)
            && !args.iter().any(|arg| arg == "-lstdc++")
            && args.iter().any(|arg| matches!(arg.as_str(), "-lz" | "-lc")));
    if !want {
        return args.to_vec();
    }
    let mut out = Vec::with_capacity(args.len() + 1);
    let mut pushed = false;
    for arg in args {
        if !pushed
            && matches!(arg.as_str(), "-lstdc++" | "-lz" | "-lc")
            && out.last().map(String::as_str) != Some("-Bstatic")
        {
            out.push("-Bstatic".to_string());
            pushed = true;
        }
        out.push(arg.clone());
    }
    out
}

/// rustc mete `-Bdynamic` delante de `-lz`/`-lc`; wild no debe quedarse en
/// modo dinámico para ejecutables que corren en el guest.
fn without_bdynamic(args: &[String]) -> Vec<String> {
    // Con `-lstdc++` también: un binario C++ cuyo `-Bdynamic -lc -lm` queda dinámico
    // sale con `NEEDED libc.so.6` y las llamadas a libc van por GOT a cero (T42: un
    // `main` de C++ enlazado desde Rust, p. ej. clang, moría en `secure_getenv`).
    if !needs_gcc_crt(args) || std::env::var_os("SOSO_ENLACE_DINAMICO").is_some() {
        return args.to_vec();
    }
    args.iter()
        .filter(|arg| *arg != "-Bdynamic")
        .cloned()
        .collect()
}

/// Binarios sin cargador Linux (p. ej. `cargo` en soso): hace falta `--static`
/// (wild) para no dejar `NEEDED` en libz/libc.
fn with_static_executable(args: &[String]) -> Vec<String> {
    if !needs_gcc_crt(args)
        || args.iter().any(|arg| arg == "-lstdc++")
        || args.iter().any(|arg| arg == "--static" || arg == "-static")
    {
        return args.to_vec();
    }
    let mut out = args.to_vec();
    out.push("--static".to_string());
    out
}

/// `__TMC_END__` está en `crtend.o`. gcc lo pone al final; rustc no.
fn with_crtend(args: &[String], crtend: Option<&str>) -> Vec<String> {
    let Some(crtend) = crtend else {
        return args.to_vec();
    };
    if !needs_gcc_crt(args) || args.iter().any(|arg| arg.ends_with("crtend.o")) {
        return args.to_vec();
    }
    let mut out = Vec::with_capacity(args.len() + 1);
    out.extend(args.iter().cloned());
    out.push(crtend.to_string());
    out
}

/// C-037: `write` de glibc es la syscall 1 de Linux (`read` en soso).
fn with_soso_libc_write(args: &[String], obj: Option<&str>) -> Vec<String> {
    let Some(obj) = obj else {
        return args.to_vec();
    };
    if !args.iter().any(|arg| arg == "-lstdc++")
        || args.iter().any(|arg| arg == "--wrap=__libc_write")
    {
        return args.to_vec();
    }
    let mut out = Vec::with_capacity(args.len() + 3);
    let mut rest = args.iter().cloned();
    if args.first().map(String::as_str) == Some("-flavor") {
        out.push(rest.next().unwrap());
        if let Some(flavor) = rest.next() {
            out.push(flavor);
        }
    }
    out.push("--wrap=__libc_write".to_string());
    out.push("--wrap=write".to_string());
    out.push(obj.to_string());
    out.extend(rest);
    out
}

fn soso_libc_write_obj() -> Option<String> {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("soso_libc_write.s");
    let dst = env::current_exe()
        .ok()?
        .parent()?
        .join("soso_libc_write.o");
    let status = Command::new("cc")
        .args(["-c", "-o"])
        .arg(&dst)
        .arg(&src)
        .status()
        .ok()?;
    if status.success() {
        Some(dst.to_string_lossy().into_owned())
    } else {
        None
    }
}

/// C-038: `malloc` de glibc pide `brk` absoluto. En soso esa memoria sale de `mmap`.
const MALLOC_WRAPS: &[&str] = &[
    "--wrap=malloc",
    "--wrap=calloc",
    "--wrap=realloc",
    "--wrap=free",
    "--wrap=aligned_alloc",
    "--wrap=memalign",
    "--wrap=posix_memalign",
    "--wrap=__libc_malloc",
    "--wrap=__libc_calloc",
    "--wrap=__libc_realloc",
    "--wrap=__libc_free",
    "--wrap=__libc_memalign",
    // T42: `sysconf(_SC_SIGSTKSZ/_SC_MINSIGSTKSZ)` da cero sin la inicialización de la
    // libc estática y glibc aborta con una aserción (LLVM la llama al arrancar).
    "--wrap=sysconf",
    "--wrap=__sysconf",
];

fn with_soso_libc_alloc(args: &[String], obj: Option<&str>) -> Vec<String> {
    let Some(obj) = obj else {
        return args.to_vec();
    };
    if !args.iter().any(|arg| arg == "-lstdc++")
        || args.iter().any(|arg| arg == "--wrap=malloc")
    {
        return args.to_vec();
    }
    let mut out = Vec::with_capacity(args.len() + MALLOC_WRAPS.len() + 1);
    let mut rest = args.iter().cloned();
    if args.first().map(String::as_str) == Some("-flavor") {
        out.push(rest.next().unwrap());
        if let Some(flavor) = rest.next() {
            out.push(flavor);
        }
    }
    for wrap in MALLOC_WRAPS {
        out.push((*wrap).to_string());
    }
    out.push(obj.to_string());
    out.extend(rest);
    out
}

fn soso_libc_alloc_obj() -> Option<String> {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("soso_libc_alloc.c");
    let dst = env::current_exe()
        .ok()?
        .parent()?
        .join("soso_libc_alloc.o");
    let status = Command::new("cc")
        .args([
            "-c",
            "-ffreestanding",
            "-fno-builtin",
            "-fno-stack-protector",
            "-O2",
            "-o",
        ])
        .arg(&dst)
        .arg(&src)
        .status()
        .ok()?;
    if status.success() {
        Some(dst.to_string_lossy().into_owned())
    } else {
        None
    }
}

fn salida_enlazada(args: &[String]) -> Option<&str> {
    args.iter()
        .enumerate()
        .rev()
        .find(|(_, a)| a.as_str() == "-o")
        .and_then(|(i, _)| args.get(i + 1).map(String::as_str))
}

/// C-046: stdio (`_IO_new_file_write`) llama a `__libc_write` por dentro de
/// libc, y `--wrap` no reescribe esa referencia. El cuerpo sigue haciendo
/// la syscall 1. Se cambia el inmediato a 2 en el símbolo ya enlazado.
fn parchear_libc_write(path: &Path) -> std::io::Result<usize> {
    let mut f = OpenOptions::new().read(true).write(true).open(path)?;
    let mut hdr = [0u8; 64];
    if f.read(&mut hdr)? < 64 || &hdr[0..4] != b"\x7fELF" || hdr[4] != 2 || hdr[5] != 1 {
        return Ok(0);
    }
    let e_shoff = u64::from_le_bytes(hdr[40..48].try_into().unwrap());
    let e_shentsize = u16::from_le_bytes(hdr[58..60].try_into().unwrap()) as u64;
    let e_shnum = u16::from_le_bytes(hdr[60..62].try_into().unwrap()) as u64;
    if e_shoff == 0 || e_shentsize < 64 || e_shnum == 0 {
        return Ok(0);
    }
    let mut shdrs = vec![0u8; (e_shentsize * e_shnum) as usize];
    f.seek(SeekFrom::Start(e_shoff))?;
    f.read_exact(&mut shdrs)?;
    let sh = |i: u64| -> Option<&[u8]> {
        let off = (i * e_shentsize) as usize;
        shdrs.get(off..off + 64)
    };
    let symtab = (0..e_shnum).find_map(|i| {
        let h = sh(i)?;
        let sh_type = u32::from_le_bytes(h[4..8].try_into().unwrap());
        (sh_type == 2).then_some(i) // SHT_SYMTAB
    });
    let Some(sym_i) = symtab else {
        return Ok(0);
    };
    let sym_h = sh(sym_i).unwrap();
    let sym_off = u64::from_le_bytes(sym_h[24..32].try_into().unwrap());
    let sym_size = u64::from_le_bytes(sym_h[32..40].try_into().unwrap()) as usize;
    let str_i = u32::from_le_bytes(sym_h[40..44].try_into().unwrap()) as u64;
    let Some(str_h) = sh(str_i) else {
        return Ok(0);
    };
    let str_off = u64::from_le_bytes(str_h[24..32].try_into().unwrap());
    let str_size = u64::from_le_bytes(str_h[32..40].try_into().unwrap()) as usize;
    let mut nombres = vec![0u8; str_size];
    f.seek(SeekFrom::Start(str_off))?;
    f.read_exact(&mut nombres)?;
    let mut syms = vec![0u8; sym_size];
    f.seek(SeekFrom::Start(sym_off))?;
    f.read_exact(&mut syms)?;
    let mut encontrado: Option<(u64, u64, u16)> = None;
    let mut off = 0;
    while off + 24 <= syms.len() {
        let st_name = u32::from_le_bytes(syms[off..off + 4].try_into().unwrap()) as usize;
        let st_shndx = u16::from_le_bytes(syms[off + 6..off + 8].try_into().unwrap());
        let st_value = u64::from_le_bytes(syms[off + 8..off + 16].try_into().unwrap());
        let st_size = u64::from_le_bytes(syms[off + 16..off + 24].try_into().unwrap());
        let fin = nombres[st_name..].iter().position(|b| *b == 0).map(|n| st_name + n).unwrap_or(st_name);
        if &nombres[st_name..fin] == b"__libc_write" && st_size > 0 && st_shndx != 0 {
            encontrado = Some((st_value, st_size, st_shndx));
            break;
        }
        off += 24;
    }
    let Some((valor, tam, shndx)) = encontrado else {
        return Ok(0);
    };
    let Some(sec) = sh(shndx as u64) else {
        return Ok(0);
    };
    let sec_addr = u64::from_le_bytes(sec[16..24].try_into().unwrap());
    let sec_off = u64::from_le_bytes(sec[24..32].try_into().unwrap());
    if valor < sec_addr {
        return Ok(0);
    }
    let file_off = sec_off + (valor - sec_addr);
    let mut codigo = vec![0u8; tam as usize];
    f.seek(SeekFrom::Start(file_off))?;
    f.read_exact(&mut codigo)?;
    let n = sustituir_syscall_write(&mut codigo);
    if n > 0 {
        f.seek(SeekFrom::Start(file_off))?;
        f.write_all(&codigo)?;
    }
    Ok(n)
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--parche-libc-write") {
        let Some(path) = args.get(1) else {
            eprintln!("wild-soso: falta la ruta");
            return ExitCode::from(2);
        };
        return match parchear_libc_write(Path::new(path)) {
            Ok(n) => {
                eprintln!("wild-soso: __libc_write syscall 1 → 2 ({n})");
                ExitCode::from(0)
            }
            Err(e) => {
                eprintln!("wild-soso: {e}");
                ExitCode::from(1)
            }
        };
    }
    if args.is_empty() {
        eprintln!("wild-soso: passthrough a `wild` (instalar desde github.com/davidlattimore/wild)");
        return ExitCode::from(2);
    }
    // Validación mínima de link.ld del kernel (PIE en 0x10000000000).
    if args.iter().any(|a| a.contains("link.ld")) {
        eprintln!("wild-soso: link.ld detectado (PIE user/kernel soso)");
    }
    // C-018: rustc pasa `-Wl,…` (prefijo de gcc). wild quiere la opción del enlazador.
    let args = expand_wl(&args);
    // C-019: `-lstdc++` sin el directorio de gcc.
    let args = with_stdcxx_dir(&args, stdcxx_dir().as_deref());
    // C-021: `free` está en la libc del host; libstdc++ no la arrastra sola.
    let args = with_host_libc(&args, libc_dir().as_deref());
    // C-023: `crc32` está en libz.
    let args = with_host_libz(&args, libz_dir().as_deref());
    // C-024: `logb` está en libm.
    let args = with_host_libm(&args, libm_dir().as_deref());
    // C-027: `_Unwind_Resume` está en libgcc_eh.a.
    let args = with_host_libgcc_eh(&args, cc_file_path("libgcc_eh.a").as_deref());
    // C-022: `__dso_handle` está en crtbegin.o.
    let args = with_crtbegin(&args, cc_file_path("crtbegin.o").as_deref());
    // C-025: `__TMC_END__` está en crtend.o.
    let args = with_crtend(&args, cc_file_path("crtend.o").as_deref());
    // C-090: `-L` del host delante de `-lz`/`-lc`/… cuando rustc no pide `-lstdc++`.
    let args = with_host_lib_search_dirs(&args);
    // C-091: OpenSSL para libssh2/libgit2/curl en el bin `cargo` (sin `-lstdc++`).
    let args = with_openssl_libs(&args);
    let args = without_bdynamic(&args);
    // C-026: libstdc++, libm, libz y libc estáticas. El guest no carga .so del host.
    let args = with_static_host_libs(&args);
    let args = with_static_executable(&args);
    // C-037: write de glibc no puede ser la syscall 1 (read en soso).
    let write_obj = soso_libc_write_obj();
    let args = with_soso_libc_write(&args, write_obj.as_deref());
    // C-038: malloc de glibc no obtiene memoria con el brk de Linux.
    let alloc_obj = soso_libc_alloc_obj();
    let args = with_soso_libc_alloc(&args, alloc_obj.as_deref());
    match Command::new("wild").args(&args).status() {
        Ok(st) if st.success() => {
            if let Some(out) = salida_enlazada(&args) {
                if let Err(e) = parchear_libc_write(Path::new(out)) {
                    eprintln!("wild-soso: no pude parchear __libc_write en {out}: {e}");
                }
            }
            ExitCode::SUCCESS
        }
        Ok(st) => ExitCode::from(st.code().unwrap_or(1) as u8),
        Err(e) => {
            eprintln!("wild-soso: no se encontró `wild` en PATH: {e}");
            eprintln!("  cargo install wild-linker   # host");
            ExitCode::from(127)
        }
    }
}

#[cfg(test)]
mod tests {
    /// El nombre del binario tiene que ser el que el target pide como enlazador.
    ///
    /// Se separaron —el binario se llamaba `wild` y el target pedía
    /// `wild-soso`— y nadie se enteró porque **nada consume ese target
    /// todavía**. Este test los ata: si alguien cambia uno, falla.
    #[test]
    fn la_syscall_1_de_libc_write_pasa_a_2() {
        let mut codigo = super::SYSCALL_WRITE_LINUX.to_vec();
        codigo.extend_from_slice(&[0x90, 0xc3]);
        codigo.extend_from_slice(&super::SYSCALL_WRITE_LINUX);
        assert_eq!(super::sustituir_syscall_write(&mut codigo), 2);
        assert_eq!(codigo[1], 0x02);
        assert_eq!(codigo[7 + 2 + 1], 0x02);
        assert_eq!(super::sustituir_syscall_write(&mut codigo), 0);
    }

    #[test]
    fn write_de_glibc_usa_la_syscall_de_soso() {
        let args = ["-flavor".to_string(), "gnu".to_string(), "-lstdc++".to_string()];
        let got = super::with_soso_libc_write(&args, Some("soso_libc_write.o"));
        assert_eq!(
            got,
            [
                "-flavor",
                "gnu",
                "--wrap=__libc_write",
                "--wrap=write",
                "soso_libc_write.o",
                "-lstdc++",
            ]
        );
        let sin = super::with_soso_libc_write(&["a.o".to_string()], Some("soso_libc_write.o"));
        assert_eq!(sin, ["a.o"]);
    }

    #[test]
    fn malloc_de_glibc_usa_mmap_de_soso() {
        let args = ["-flavor".to_string(), "gnu".to_string(), "-lstdc++".to_string()];
        let got = super::with_soso_libc_alloc(&args, Some("soso_libc_alloc.o"));
        assert_eq!(
            got,
            [
                "-flavor",
                "gnu",
                "--wrap=malloc",
                "--wrap=calloc",
                "--wrap=realloc",
                "--wrap=free",
                "--wrap=aligned_alloc",
                "--wrap=memalign",
                "--wrap=posix_memalign",
                "--wrap=__libc_malloc",
                "--wrap=__libc_calloc",
                "--wrap=__libc_realloc",
                "--wrap=__libc_free",
                "--wrap=__libc_memalign",
                "--wrap=sysconf",
                "--wrap=__sysconf",
                "soso_libc_alloc.o",
                "-lstdc++",
            ]
        );
        let sin = super::with_soso_libc_alloc(&["a.o".to_string()], Some("soso_libc_alloc.o"));
        assert_eq!(sin, ["a.o"]);
    }

    #[test]
    fn el_binario_se_llama_como_el_target_lo_busca() {
        let spec = include_str!("../../../targets/x86_64-unknown-soso.json");
        let linker = spec
            .lines()
            .find_map(|l| l.trim().strip_prefix("\"linker\":"))
            .expect("el target declara un linker")
            .trim()
            .trim_end_matches(',')
            .trim_matches('"');
        assert_eq!(linker, env!("CARGO_BIN_NAME"));
    }

    #[test]
    fn wl_se_parte_en_opciones_de_enlazador() {
        let args = [
            "-flavor".to_string(),
            "-Wl,-z,origin".to_string(),
            "-Wl,-rpath,$ORIGIN/../lib".to_string(),
            "a.o".to_string(),
        ];
        let expanded = super::expand_wl(&args);
        let got: Vec<&str> = expanded.iter().map(String::as_str).collect();
        assert_eq!(
            got,
            [
                "-flavor",
                "-z",
                "origin",
                "-rpath",
                "$ORIGIN/../lib",
                "a.o",
            ]
        );
    }

    #[test]
    fn stdcxx_recibe_el_directorio_de_gcc() {
        let args = [
            "-Bdynamic".to_string(),
            "-lstdc++".to_string(),
            "-o".to_string(),
            "a".to_string(),
        ];
        let got = super::with_stdcxx_dir(&args, Some("/usr/lib/gcc/x86_64-linux-gnu/13"));
        let got: Vec<&str> = got.iter().map(String::as_str).collect();
        assert_eq!(
            got,
            [
                "-Bdynamic",
                "-L",
                "/usr/lib/gcc/x86_64-linux-gnu/13",
                "-lstdc++",
                "-o",
                "a",
            ]
        );
    }

    #[test]
    fn stdcxx_arrastra_la_libc_del_host() {
        let args = [
            "-Bdynamic".to_string(),
            "-lstdc++".to_string(),
            "-o".to_string(),
            "a".to_string(),
        ];
        let got = super::with_host_libc(&args, Some("/usr/lib/x86_64-linux-gnu"));
        let got: Vec<&str> = got.iter().map(String::as_str).collect();
        assert_eq!(
            got,
            [
                "-Bdynamic",
                "-lstdc++",
                "-L",
                "/usr/lib/x86_64-linux-gnu",
                "-lc",
                "-o",
                "a",
            ]
        );
    }

    #[test]
    fn stdcxx_recibe_crtbegin() {
        let args = ["-Bdynamic".to_string(), "-lstdc++".to_string()];
        let crt = "/usr/lib/gcc/x86_64-linux-gnu/13/crtbegin.o";
        let got = super::with_crtbegin(&args, Some(crt));
        let got: Vec<&str> = got.iter().map(String::as_str).collect();
        assert_eq!(got, ["-Bdynamic", crt, "-lstdc++"]);
    }

    #[test]
    fn stdcxx_arrastra_libz() {
        let args = ["-lstdc++".to_string(), "-o".to_string(), "a".to_string()];
        let got = super::with_host_libz(&args, Some("/usr/lib/x86_64-linux-gnu"));
        let got: Vec<&str> = got.iter().map(String::as_str).collect();
        assert_eq!(
            got,
            [
                "-lstdc++",
                "-L",
                "/usr/lib/x86_64-linux-gnu",
                "-lz",
                "-o",
                "a",
            ]
        );
    }

    #[test]
    fn stdcxx_arrastra_libm() {
        let args = ["-lstdc++".to_string(), "-o".to_string(), "a".to_string()];
        let got = super::with_host_libm(&args, Some("/usr/lib/x86_64-linux-gnu"));
        let got: Vec<&str> = got.iter().map(String::as_str).collect();
        assert_eq!(
            got,
            [
                "-lstdc++",
                "-L",
                "/usr/lib/x86_64-linux-gnu",
                "-lm",
                "-o",
                "a",
            ]
        );
    }

    #[test]
    fn stdcxx_recibe_crtend_al_final() {
        let args = ["-lstdc++".to_string(), "-lc".to_string()];
        let crt = "/usr/lib/gcc/x86_64-linux-gnu/13/crtend.o";
        let got = super::with_crtend(&args, Some(crt));
        let got: Vec<&str> = got.iter().map(String::as_str).collect();
        assert_eq!(got, ["-lstdc++", "-lc", crt]);
    }

    #[test]
    fn stdcxx_arrastra_libgcc_eh() {
        let args = ["-lstdc++".to_string(), "-lc".to_string()];
        let archive = "/usr/lib/gcc/x86_64-linux-gnu/13/libgcc_eh.a";
        let got = super::with_host_libgcc_eh(&args, Some(archive));
        let got: Vec<&str> = got.iter().map(String::as_str).collect();
        assert_eq!(got, ["-lstdc++", archive, "-lc"]);
    }

    #[test]
    fn cargo_sin_stdcxx_arrastra_libgcc_eh() {
        let args = [
            "-Bdynamic".to_string(),
            "-lz".to_string(),
            "-lc".to_string(),
            "-lpthread".to_string(),
        ];
        let archive = "/usr/lib/gcc/x86_64-linux-gnu/13/libgcc_eh.a";
        let got = super::with_host_libgcc_eh(&args, Some(archive));
        let got: Vec<&str> = got.iter().map(String::as_str).collect();
        assert_eq!(
            got,
            ["-Bdynamic", "-lz", "-lc", "-lpthread", archive]
        );
    }

    #[test]
    fn las_bibliotecas_del_host_van_estaticas() {
        let args = [
            "-Bstatic".to_string(),
            "lib.rlib".to_string(),
            "-Bdynamic".to_string(),
            "-lstdc++".to_string(),
            "-lm".to_string(),
            "-lc".to_string(),
        ];
        let got = super::with_static_host_libs(&args);
        let got: Vec<&str> = got.iter().map(String::as_str).collect();
        assert_eq!(
            got,
            [
                "-Bstatic",
                "lib.rlib",
                "-Bdynamic",
                "-Bstatic",
                "-lstdc++",
                "-lm",
                "-lc",
            ]
        );
    }

    #[test]
    fn cargo_recibe_directorio_antes_de_lz() {
        let args = [
            "-Bdynamic".to_string(),
            "-lz".to_string(),
            "-lc".to_string(),
        ];
        let got = super::with_lib_dir_before(&args, "-lz", Some("/usr/lib/x86_64-linux-gnu"));
        let got: Vec<&str> = got.iter().map(String::as_str).collect();
        assert_eq!(
            got,
            ["-Bdynamic", "-L", "/usr/lib/x86_64-linux-gnu", "-lz", "-lc"]
        );
    }

    #[test]
    fn sin_stdcxx_las_bibliotecas_del_host_van_estaticas() {
        let args = [
            "-Bdynamic".to_string(),
            "-lz".to_string(),
            "-lc".to_string(),
        ];
        let got = super::with_static_host_libs(&args);
        let got: Vec<&str> = got.iter().map(String::as_str).collect();
        assert_eq!(got, ["-Bdynamic", "-Bstatic", "-lz", "-lc"]);
    }

    #[test]
    fn cargo_sin_stdcxx_pide_enlace_estatico() {
        let args = ["-Bdynamic".to_string(), "-lz".to_string(), "-lc".to_string()];
        let got = super::with_static_executable(&args);
        let got: Vec<&str> = got.iter().map(String::as_str).collect();
        assert_eq!(got, ["-Bdynamic", "-lz", "-lc", "--static"]);
    }

    #[test]
    fn cargo_quita_bdynamic() {
        let args = [
            "-Bdynamic".to_string(),
            "-lz".to_string(),
            "-lc".to_string(),
        ];
        let got = super::without_bdynamic(&args);
        let got: Vec<&str> = got.iter().map(String::as_str).collect();
        assert_eq!(got, ["-lz", "-lc"]);
    }

    #[test]
    fn libssh2_arrastra_openssl() {
        let args = [
            "liblibssh2_sys-deadbeef.rlib".to_string(),
            "-o".to_string(),
            "cargo".to_string(),
        ];
        let got = super::with_openssl_libs(&args);
        assert!(got.iter().any(|a| a == "-lssl"));
        assert!(got.iter().any(|a| a == "-lcrypto"));
    }
}
