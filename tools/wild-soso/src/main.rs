//! Wrapper de **wild** para el target soso. Si `wild` está en PATH lo invoca;
//! si no, falla con instrucciones (el port completo vendrá en iteraciones).

use std::env;
use std::path::Path;
use std::process::{Command, ExitCode};

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

/// `__dso_handle` está en `crtbegin.o`. gcc lo mete; rustc no.
fn with_crtbegin(args: &[String], crtbegin: Option<&str>) -> Vec<String> {
    let Some(crtbegin) = crtbegin else {
        return args.to_vec();
    };
    if !args.iter().any(|arg| arg == "-lstdc++")
        || args.iter().any(|arg| arg.ends_with("crtbegin.o"))
    {
        return args.to_vec();
    }
    let mut out = Vec::with_capacity(args.len() + 1);
    let mut inserted = false;
    for arg in args {
        if !inserted && arg == "-lstdc++" {
            out.push(crtbegin.to_string());
            inserted = true;
        }
        out.push(arg.clone());
    }
    out
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
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
    // C-022: `__dso_handle` está en crtbegin.o.
    let args = with_crtbegin(&args, cc_file_path("crtbegin.o").as_deref());
    match Command::new("wild").args(&args).status() {
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
}
