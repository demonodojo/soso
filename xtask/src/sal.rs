//! Construcción explícita y empaquetado del SDK y compilador Sal.
use std::{
    fs,
    path::Path,
    process::{Command, exit},
};

fn ejecutar(cmd: &mut Command) {
    let status = cmd
        .status()
        .expect("no se pudo ejecutar la herramienta de Sal");
    if !status.success() {
        exit(status.code().unwrap_or(1));
    }
}

pub fn build(args: &[String]) {
    let root = super::project_root();
    let cross = match args {
        [] => false,
        [a] if a == "--cross-only" => true,
        _ => {
            eprintln!("uso: cargo xtask sal-build [--cross-only]");
            exit(2);
        }
    };
    ejecutar(
        Command::new("python3")
            .current_dir(&root)
            .arg("scripts/sal-source.py"),
    );
    // Un fallo posterior no debe dejar artefactos parciales marcados como listos.
    quitar(&root.join("target/sal/source.lock"));
    ejecutar(
        Command::new("bash")
            .current_dir(&root)
            .arg("scripts/sal-sdk.sh"),
    );
    let source = std::env::var_os("SOSO_SAL_SOURCE")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| root.join("../sal"));
    ejecutar(
        Command::new("cargo")
            .current_dir(&root)
            .args([
                "build",
                "--locked",
                "--release",
                "--bin",
                "sal",
                "--manifest-path",
            ])
            .arg(source.join("Cargo.toml")),
    );
    let sysroot = Command::new("rustc")
        .args(["--print", "sysroot"])
        .output()
        .expect("rustc");
    assert!(sysroot.status.success());
    let lld = Path::new(std::str::from_utf8(&sysroot.stdout).unwrap().trim())
        .join("lib/rustlib/x86_64-unknown-linux-gnu/bin/rust-lld");
    for (source_file, name) in [
        ("user/sal/sal-contar.sal", "sal-contar"),
        ("tests/sal/hello.sal", "hello"),
        ("tests/sal/runtime.sal", "runtime"),
    ] {
        ejecutar(
            Command::new(source.join("target/release/sal"))
                .current_dir(&root)
                .env("SAL_LD", &lld)
                .env("SAL_SDK", root.join("target/sal/sdk"))
                .args(["build", source_file, "--target", "soso", "-o"])
                .arg(root.join("target/sal").join(name)),
        );
    }
    if !cross {
        ejecutar(
            Command::new("bash")
                .current_dir(&root)
                .arg("scripts/sal-native.sh"),
        );
        fs::copy(
            root.join("config/sal-source.lock"),
            root.join("target/sal/native/source.lock"),
        )
        .expect("procedencia compilador Sal");
    }
    // Rechazar fuentes que hayan cambiado durante el build.
    ejecutar(
        Command::new("python3")
            .current_dir(&root)
            .arg("scripts/sal-source.py"),
    );
    let mut elf = Command::new("python3");
    elf.current_dir(&root).args([
        "scripts/sal-elf.py",
        "target/sal/sal-contar",
        "target/sal/hello",
        "target/sal/runtime",
    ]);
    if !cross {
        elf.args(["target/sal/native/sal", "target/sal/native/sal-llc"]);
    }
    ejecutar(&mut elf);
    fs::copy(
        root.join("config/sal-source.lock"),
        root.join("target/sal/source.lock"),
    )
    .expect("procedencia Sal");
    install(&root);
}

fn copiar(src: &Path, dst: &Path) -> bool {
    let bytes = fs::read(src).unwrap_or_else(|e| panic!("{}: {e}", src.display()));
    if fs::read(dst).ok().as_ref() == Some(&bytes) {
        return false;
    }
    fs::create_dir_all(dst.parent().unwrap()).expect("directorio Sal");
    fs::copy(src, dst).unwrap_or_else(|e| panic!("{}: {e}", dst.display()));
    true
}

fn quitar(path: &Path) -> bool {
    match fs::remove_file(path) {
        Ok(()) => true,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
        Err(e) => panic!("{}: {e}", path.display()),
    }
}

/// Las construcciones normales reutilizan un SDK preparado de forma explícita.
pub fn install(root: &Path) -> bool {
    let prepared = root.join("target/sal");
    if !prepared.join("source.lock").is_file() {
        return false;
    }
    assert_eq!(
        fs::read(prepared.join("source.lock")).unwrap(),
        fs::read(root.join("config/sal-source.lock")).unwrap(),
        "SDK de Sal desactualizado: ejecuta cargo xtask sal-build"
    );
    let mut changed = copiar(
        &prepared.join("sal-contar"),
        &root.join("rootfs/bin/sal-contar"),
    );
    for file in [
        "runtime.o",
        "kernels.o",
        "libsal-soso.a",
        "link.ld",
        "sal_runtime.h",
        "manifest.txt",
    ] {
        changed |= copiar(
            &prepared.join("sdk").join(file),
            &root.join("rootfs/lib/sal").join(file),
        );
    }
    changed |= copiar(
        &prepared.join("source.lock"),
        &root.join("rootfs/lib/sal/source.lock"),
    );
    changed |= copiar(
        &root.join("user/sal/sal-contar.sal"),
        &root.join("rootfs/usr/share/sal/sal-contar.sal"),
    );
    changed |= copiar(
        &root.join("tests/sal/hello.sal"),
        &root.join("rootfs/usr/share/sal/hello.sal"),
    );
    for file in ["imports.sal", "modulo.sal"] {
        changed |= copiar(
            &root.join("tests/sal").join(file),
            &root.join("rootfs/usr/share/sal").join(file),
        );
    }
    if fs::read(prepared.join("native/source.lock")).ok()
        == fs::read(prepared.join("source.lock")).ok()
    {
        changed |= copiar(&prepared.join("native/sal"), &root.join("rootfs/bin/sal"));
        changed |= copiar(
            &prepared.join("native/sal-llc"),
            &root.join("rootfs/lib/sal/bin/sal-llc"),
        );
    } else {
        changed |= quitar(&root.join("rootfs/bin/sal"));
        changed |= quitar(&root.join("rootfs/lib/sal/bin/sal-llc"));
    }
    changed
}

pub fn test() {
    let root = super::project_root();
    build(&[]);
    super::build_image();
    super::build_user();
    ejecutar(Command::new("cargo").current_dir(root.join("user")).args([
        "build",
        "--release",
        "-p",
        "sal-runtime",
        "--example",
        "sonda",
    ]));
    ejecutar(Command::new("cargo").current_dir(&root).args([
        "build",
        "--release",
        "-p",
        "mkfs-soso",
    ]));
    ejecutar(
        Command::new("python3")
            .current_dir(&root)
            .arg("tests/sal/probar.py"),
    );
}
