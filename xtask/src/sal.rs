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

/// Binario `sal` del checkout hermano (`SOSO_SAL_SOURCE`, por defecto `../sal`).
fn sal_source(root: &Path) -> std::path::PathBuf {
    std::env::var_os("SOSO_SAL_SOURCE")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| root.join("../sal"))
}

fn sal_compilado(root: &Path) -> std::path::PathBuf {
    compilar_sal(root, &sal_source(root))
}

fn compilar_sal(root: &Path, source: &Path) -> std::path::PathBuf {
    let manifest = source.join("Cargo.toml");
    ejecutar(
        Command::new("cargo")
            .current_dir(root)
            .args([
                "build",
                "--locked",
                "--release",
                "--bin",
                "sal",
                "--manifest-path",
            ])
            .arg(&manifest),
    );
    let meta = Command::new("cargo")
        .current_dir(root)
        .args(["metadata", "--format-version", "1", "--manifest-path"])
        .arg(&manifest)
        .output()
        .expect("cargo metadata (sal)");
    assert!(meta.status.success(), "cargo metadata falló");
    let meta_json: serde_json::Value =
        serde_json::from_slice(&meta.stdout).expect("JSON metadata sal");
    let target_dir = meta_json["target_directory"]
        .as_str()
        .expect("target_directory en metadata sal");
    Path::new(target_dir).join("release/sal")
}

fn rust_lld() -> std::path::PathBuf {
    let sysroot = Command::new("rustc")
        .args(["--print", "sysroot"])
        .output()
        .expect("rustc");
    assert!(sysroot.status.success());
    Path::new(std::str::from_utf8(&sysroot.stdout).unwrap().trim())
        .join("lib/rustlib/x86_64-unknown-linux-gnu/bin/rust-lld")
}

/// Identificador del toolchain: los 12 primeros hex del sha256 del pin.
fn id_toolchain(lock: &str) -> String {
    let sha = lock
        .lines()
        .find_map(|l| l.strip_prefix("sha256="))
        .expect("sha256= en sal-source.lock");
    sha[..12].to_string()
}

fn nombre_paquete(id: &str) -> String {
    format!("sal-toolchain-{id}-x86_64-linux.tar.gz")
}

/// `cargo xtask sal-pack`: empaqueta el compilador host y el SDK freestanding
/// de la revisión `base=` de `config/sal-source.lock` (sin tocar el pin), para
/// que el CI no necesite el checkout `../sal`. Se compila en un worktree
/// desechable junto a `soso` (Sal depende de `../soso/crates/sosomodel`).
pub fn pack() {
    let root = super::project_root();
    let lock = fs::read_to_string(root.join("config/sal-source.lock")).unwrap();
    let base = lock
        .lines()
        .find_map(|l| l.strip_prefix("base="))
        .expect("base= en sal-source.lock")
        .to_string();
    let origen = sal_source(&root);
    let wt = root.join("..").join(format!("sal-pin-{}", &base[..12]));
    let _ = Command::new("git")
        .arg("-C")
        .arg(&origen)
        .args(["worktree", "remove", "--force"])
        .arg(&wt)
        .status();
    ejecutar(
        Command::new("git")
            .arg("-C")
            .arg(&origen)
            .args(["worktree", "add", "--detach"])
            .arg(&wt)
            .arg(&base),
    );
    let sal_bin = compilar_sal(&root, &wt);
    ejecutar(
        Command::new("bash")
            .current_dir(&root)
            .env("SOSO_SAL_SOURCE", &wt)
            .arg("scripts/sal-sdk.sh"),
    );
    let area = root.join("target/sal/paquete");
    let _ = fs::remove_dir_all(&area);
    fs::create_dir_all(area.join("sdk")).unwrap();
    fs::copy(&sal_bin, area.join("sal")).expect("copiar sal");
    for file in [
        "runtime.o",
        "kernels.o",
        "libsal-soso.a",
        "link.ld",
        "sal_runtime.h",
        "manifest.txt",
    ] {
        fs::copy(root.join("target/sal/sdk").join(file), area.join("sdk").join(file))
            .unwrap_or_else(|e| panic!("{file}: {e}"));
    }
    fs::write(area.join("source.lock"), &lock).unwrap();
    let out = root.join("target").join(nombre_paquete(&id_toolchain(&lock)));
    ejecutar(
        Command::new("tar")
            .arg("-czf")
            .arg(&out)
            .arg("-C")
            .arg(&area)
            .args(["sal", "sdk", "source.lock"]),
    );
    let _ = Command::new("git")
        .arg("-C")
        .arg(&origen)
        .args(["worktree", "remove", "--force"])
        .arg(&wt)
        .status();
    let id = id_toolchain(&lock);
    println!("Toolchain de Sal ({}): {}", &base[..12], out.display());
    println!(
        "Publicar: gh release create sal-toolchain-{id} {} -R demonodojo/soso --title \"Toolchain de Sal {id}\" --notes \"Compilador y SDK de Sal, base {}\"",
        out.display(),
        &base[..12],
    );
}

/// `cargo xtask sal-fetch`: baja de la release del repo el toolchain del pin y
/// construye con él.
pub fn fetch() {
    let root = super::project_root();
    let lock = fs::read_to_string(root.join("config/sal-source.lock")).unwrap();
    let id = id_toolchain(&lock);
    let dir = root.join("target/sal/descarga");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    ejecutar(
        Command::new("gh")
            .current_dir(&root)
            .args(["release", "download", &format!("sal-toolchain-{id}")])
            .args(["-R", "demonodojo/soso", "-p", &nombre_paquete(&id), "-D"])
            .arg(&dir),
    );
    build(&[
        "--toolchain".to_string(),
        dir.join(nombre_paquete(&id)).display().to_string(),
    ]);
}

/// Deja listo `target/sal` desde un toolchain empaquetado (`.tar.gz` o carpeta).
fn preparar_toolchain(root: &Path, ruta: &Path) -> std::path::PathBuf {
    let dir = if ruta.is_dir() {
        ruta.to_path_buf()
    } else {
        let dst = root.join("target/sal/toolchain");
        let _ = fs::remove_dir_all(&dst);
        fs::create_dir_all(&dst).unwrap();
        ejecutar(Command::new("tar").arg("-xzf").arg(ruta).arg("-C").arg(&dst));
        dst
    };
    let lock = fs::read(dir.join("source.lock")).expect("el paquete no trae source.lock");
    assert_eq!(
        lock,
        fs::read(root.join("config/sal-source.lock")).unwrap(),
        "el toolchain empaquetado no corresponde a config/sal-source.lock"
    );
    quitar(&root.join("target/sal/source.lock"));
    let sdk = root.join("target/sal/sdk");
    let _ = fs::remove_dir_all(&sdk);
    fs::create_dir_all(&sdk).unwrap();
    for entry in fs::read_dir(dir.join("sdk")).unwrap().flatten() {
        fs::copy(entry.path(), sdk.join(entry.file_name())).unwrap();
    }
    dir.join("sal")
}

fn compilar_apps(root: &Path, sal_bin: &Path, lld: &Path) {
    for (source_file, name) in [
        ("user/sal/sal-contar.sal", "sal-contar"),
        ("user/sal/escritorio/soso-wm.sal", "soso-wm"),
        ("user/sal/escritorio/soso-comp.sal", "soso-comp"),
        ("user/sal/escritorio/gprim-prueba.sal", "gprim-prueba"),
        ("user/sal/escritorio/demo-cliente.sal", "demo-cliente"),
        ("user/sal/escritorio/soso-mono.sal", "soso-mono"),
        ("user/sal/escritorio/soso-files.sal", "soso-files"),
        ("tests/sal/hello.sal", "hello"),
        ("tests/sal/runtime.sal", "runtime"),
    ] {
        ejecutar(
            Command::new(sal_bin)
                .current_dir(root)
                .env("SAL_LD", lld)
                .env("SAL_SDK", root.join("target/sal/sdk"))
                .args(["build", source_file, "--target", "soso", "-o"])
                .arg(root.join("target/sal").join(name)),
        );
    }
}

pub fn build(args: &[String]) {
    let root = super::project_root();
    let (cross, toolchain) = match args {
        [] => (false, None),
        [a] if a == "--cross-only" => (true, None),
        [a, ruta] if a == "--toolchain" => (true, Some(Path::new(ruta))),
        _ => {
            eprintln!("uso: cargo xtask sal-build [--cross-only | --toolchain <paquete.tar.gz|dir>]");
            exit(2);
        }
    };
    if let Some(ruta) = toolchain {
        let sal_bin = preparar_toolchain(&root, ruta);
        compilar_apps(&root, &sal_bin, &rust_lld());
        fs::copy(
            root.join("config/sal-source.lock"),
            root.join("target/sal/source.lock"),
        )
        .expect("procedencia Sal");
        instalar_guest(&root, false);
        return;
    }
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
    let sal_bin = sal_compilado(&root);
    compilar_apps(&root, &sal_bin, &rust_lld());
    // Empaqueta el escritorio en rootfs aunque falte el bootstrap nativo (sal-native.sh).
    instalar_guest(&root, true);
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
        let mut elf = Command::new("python3");
        elf.current_dir(&root).args([
            "scripts/sal-elf.py",
            "target/sal/native/sal",
            "target/sal/native/sal-llc",
        ]);
        ejecutar(&mut elf);
        fs::copy(
            root.join("config/sal-source.lock"),
            root.join("target/sal/source.lock"),
        )
        .expect("procedencia Sal");
        install(&root);
    }
}

fn instalar_guest(root: &Path, verificar_fuente: bool) {
    if verificar_fuente {
        ejecutar(
            Command::new("python3")
                .current_dir(root)
                .arg("scripts/sal-source.py"),
        );
    }
    ejecutar(
        Command::new("python3")
            .current_dir(root)
            .args([
                "scripts/sal-elf.py",
                "target/sal/sal-contar",
                "target/sal/soso-wm",
                "target/sal/soso-comp",
                "target/sal/gprim-prueba",
                "target/sal/demo-cliente",
                "target/sal/soso-mono",
                "target/sal/soso-files",
                "target/sal/hello",
                "target/sal/runtime",
            ]),
    );
    fs::copy(
        root.join("config/sal-source.lock"),
        root.join("target/sal/source.lock"),
    )
    .expect("procedencia Sal");
    install(root);
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
    changed |= copiar(
        &prepared.join("soso-wm"),
        &root.join("rootfs/bin/soso-wm"),
    );
    changed |= copiar(
        &prepared.join("soso-comp"),
        &root.join("rootfs/bin/soso-comp"),
    );
    changed |= copiar(
        &prepared.join("gprim-prueba"),
        &root.join("rootfs/bin/gprim-prueba"),
    );
    changed |= copiar(
        &prepared.join("demo-cliente"),
        &root.join("rootfs/bin/demo-cliente"),
    );
    changed |= copiar(
        &prepared.join("soso-mono"),
        &root.join("rootfs/bin/soso-mono"),
    );
    changed |= copiar(
        &prepared.join("soso-files"),
        &root.join("rootfs/bin/soso-files"),
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
    let share_esc = root.join("rootfs/usr/share/sal/escritorio");
    if let Ok(entries) = fs::read_dir(root.join("user/sal/escritorio")) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "sal") {
                changed |= copiar(&path, &share_esc.join(path.file_name().unwrap()));
            }
        }
    }
    let share_std = root.join("rootfs/usr/share/sal/std/io");
    let sal_std = root.join("../sal/std/io");
    if sal_std.is_dir() {
        if let Ok(entries) = fs::read_dir(&sal_std) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().is_some_and(|e| e == "sal") || path.file_name().is_some_and(|n| n == "Sal.toml") {
                    changed |= copiar(&path, &share_std.join(path.file_name().unwrap()));
                }
            }
        }
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
    ejecutar(
        Command::new("python3")
            .current_dir(&root)
            .arg("tests/sal/escritorio/probar_host.py"),
    );
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
