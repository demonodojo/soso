//! Herramienta de construcción y ejecución de soso.
//!
//! Uso: `cargo xtask <build|run|gdb>`
//!
//! - `build`: compila el kernel y genera la imagen de disco BIOS.
//! - `run`:   build + lanza QEMU (q35, consola por serie en stdio).
//! - `gdb`:   como `run` pero congelado en el arranque con stub GDB en :1234.

use std::path::{Path, PathBuf};
use std::process::{Command, exit};

fn main() {
    let cmd = std::env::args().nth(1).unwrap_or_else(|| "run".into());
    match cmd.as_str() {
        "build" => {
            build_image();
        }
        "run" => {
            let img = build_image();
            run_qemu(&img, false);
        }
        "gdb" => {
            let img = build_image();
            run_qemu(&img, true);
        }
        "mkfs" => {
            build_user();
            mkfs(true);
        }
        "test" => {
            test::run();
        }
        "convert-gguf" => {
            let args: Vec<String> = std::env::args().skip(2).collect();
            convert_gguf(&args);
        }
        other => {
            eprintln!("comando desconocido: {other} (usa build | run | gdb | mkfs | test | convert-gguf)");
            exit(2);
        }
    }
}

mod test;

fn convert_gguf(args: &[String]) {
    let root = project_root();
    let mut cmd = Command::new("cargo");
    cmd.current_dir(&root)
        .args(["run", "-q", "-p", "convert-gguf", "--"]);
    for a in args {
        cmd.arg(a);
    }
    let status = cmd.status().expect("convert-gguf");
    if !status.success() {
        exit(status.code().unwrap_or(1));
    }
}

fn project_root() -> PathBuf {
    // xtask vive en <root>/xtask
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf()
}

/// Compila el kernel para x86_64-soso y devuelve la ruta de la imagen.
pub(crate) fn build_image() -> PathBuf {
    let root = project_root();
    let target = root.join("kernel/x86_64-soso.json");
    let status = Command::new("cargo")
        .current_dir(root.join("kernel"))
        .args([
            "build",
            "--target",
            target.to_str().unwrap(),
            "--target-dir",
            root.join("target/kernel").to_str().unwrap(),
        ])
        .status()
        .expect("no se pudo ejecutar cargo");
    if !status.success() {
        exit(status.code().unwrap_or(1));
    }

    let kernel_elf = root.join("target/kernel/x86_64-soso/debug/kernel");
    let img = root.join("target/soso-bios.img");
    bootloader::DiskImageBuilder::new(kernel_elf)
        .create_bios_image(&img)
        .expect("fallo creando la imagen de disco");
    println!("imagen: {}", img.display());
    img
}

/// Compila el workspace user/ (release) y copia los ELF a rootfs/bin.
pub(crate) fn build_user() -> bool {
    let root = project_root();
    let status = Command::new("cargo")
        .current_dir(root.join("user"))
        .args(["build", "--release", "--target-dir"])
        .arg(root.join("target/user"))
        .status()
        .expect("no se pudo compilar user/");
    if !status.success() {
        exit(status.code().unwrap_or(1));
    }
    let out = root.join("target/user/x86_64-soso-user/release");
    let bin = root.join("rootfs/bin");
    std::fs::create_dir_all(&bin).expect("no se pudo crear rootfs/bin");
    let mut cambiado = false;
    for prog in ["init", "sosh", "ls", "cat", "echo", "mkdir", "rm", "hexdump", "halt", "soso-llm"] {
        let src = out.join(prog);
        let dst = bin.join(prog);
        let igual = std::fs::read(&src).ok() == std::fs::read(&dst).ok();
        if !igual {
            std::fs::copy(&src, &dst).expect("no se pudo copiar el binario");
            cambiado = true;
        }
    }
    cambiado
}

/// mtime más reciente de un árbol de directorios.
fn newest_mtime(dir: &Path) -> std::time::SystemTime {
    let mut newest = std::time::SystemTime::UNIX_EPOCH;
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            let t = if e.path().is_dir() {
                newest_mtime(&e.path())
            } else {
                e.metadata().and_then(|m| m.modified()).unwrap_or(newest)
            };
            newest = newest.max(t);
        }
    }
    newest
}

/// Disco de datos persistente (virtio-blk 0) con sosofs desde rootfs/.
pub(crate) fn mkfs_rootfs(force: bool) -> PathBuf {
    let root = project_root();
    let path = root.join("target/soso-data.img");
    let vieja = path
        .metadata()
        .and_then(|m| m.modified())
        .map(|img| newest_mtime(&root.join("rootfs")) > img)
        .unwrap_or(true);
    if path.exists() && !force && !vieja {
        return path;
    }
    let pubkey = client_pubkey();
    let status = Command::new("cargo")
        .current_dir(&root)
        .args(["run", "-q", "-p", "mkfs-soso", "--"])
        .arg(root.join("rootfs"))
        .arg(&path)
        .arg("64")
        .arg(&pubkey)
        .status()
        .expect("no se pudo ejecutar mkfs-soso");
    if !status.success() {
        exit(status.code().unwrap_or(1));
    }
    path
}

/// Disco de modelos (virtio-blk 1) con sosomfs. Por defecto empaqueta el
/// modelo sintético tiny; `SOSO_MODELS_DIR=<dir>` usa un modelo propio
/// (p. ej. el resultado de `cargo xtask convert-gguf`).
pub(crate) fn mkfs_models(force: bool) -> PathBuf {
    let root = project_root();
    let path = root.join("target/soso-models.img");
    let custom = std::env::var_os("SOSO_MODELS_DIR").map(PathBuf::from);
    let model_src = custom
        .clone()
        .unwrap_or_else(|| root.join("target/tiny-model"));
    if custom.is_some() {
        if !model_src.join("manifest.som").exists() {
            eprintln!(
                "xtask: SOSO_MODELS_DIR={} no contiene manifest.som",
                model_src.display()
            );
            exit(1);
        }
    } else {
        // regenerar siempre: un tiny-model viejo puede tener un formato
        // .som anterior y es barato de reconstruir
        let status = Command::new("cargo")
            .current_dir(&root)
            .args(["run", "-q", "--release", "-p", "mkmodel-soso", "--"])
            .arg(&model_src)
            .status()
            .expect("mkmodel-soso");
        if !status.success() {
            exit(status.code().unwrap_or(1));
        }
    }
    let vieja = path
        .metadata()
        .and_then(|m| m.modified())
        .map(|img| newest_mtime(&model_src) > img)
        .unwrap_or(true);
    // con modelo propio se reconstruye siempre: la imagen puede venir de otro dir
    if path.exists() && !force && !vieja && custom.is_none() {
        return path;
    }
    let size = std::env::var("SOSO_MODELS_SIZE").unwrap_or_else(|_| "8G".into());
    let status = Command::new("cargo")
        .current_dir(&root)
        .args(["run", "-q", "--release", "-p", "mkfs-sosomfs", "--"])
        .arg(&model_src)
        .arg(&path)
        .arg("--size")
        .arg(&size)
        .status()
        .expect("mkfs-sosomfs");
    if !status.success() {
        exit(status.code().unwrap_or(1));
    }
    path
}

/// Regenera ambos discos (rootfs + modelos).
fn mkfs(force: bool) -> (PathBuf, PathBuf) {
    (mkfs_rootfs(force), mkfs_models(force))
}

/// Clave pública ed25519 a autorizar en el FS. Usa ~/.ssh/id_ed25519.pub
/// si existe; si no, genera un par de test dedicado en target/ con
/// ssh-keygen (para que `cargo xtask test` pueda conectar).
fn client_pubkey() -> PathBuf {
    if let Some(home) = std::env::var_os("HOME") {
        let user = PathBuf::from(&home).join(".ssh/id_ed25519.pub");
        if user.exists() {
            return user;
        }
    }
    let key = project_root().join("target/soso_test_key");
    let pubk = project_root().join("target/soso_test_key.pub");
    if !pubk.exists() {
        let _ = std::fs::remove_file(&key);
        let status = Command::new("ssh-keygen")
            .args(["-t", "ed25519", "-N", "", "-C", "soso-test", "-f"])
            .arg(&key)
            .status()
            .expect("no se pudo ejecutar ssh-keygen para la clave de test");
        if !status.success() {
            exit(status.code().unwrap_or(1));
        }
        println!("xtask: clave de test generada en {}", key.display());
    }
    pubk
}

/// Memoria y nº de CPUs de QEMU, configurables por entorno:
/// `SOSO_QEMU_MEM=64G SOSO_QEMU_SMP=8 cargo xtask run`.
pub(crate) fn qemu_mem() -> String {
    std::env::var("SOSO_QEMU_MEM").unwrap_or_else(|_| "2G".into())
}

pub(crate) fn qemu_smp() -> String {
    std::env::var("SOSO_QEMU_SMP").unwrap_or_else(|_| "1".into())
}

fn run_qemu(img: &Path, gdb: bool) {
    build_user();
    let (data, models) = mkfs(false);
    let mut qemu = Command::new("qemu-system-x86_64");
    qemu.args(["-machine", "q35"])
        // -cpu max: expone RDRAND, que la cripto de sunset (getrandom con
        // backend rdrand) necesita; la CPU por defecto de QEMU no lo trae.
        .args(["-cpu", "max"])
        .args(["-m", &qemu_mem()])
        .args(["-smp", &qemu_smp()])
        .args(["-drive", &format!("format=raw,file={}", img.display())])
        .args(["-drive", &format!("file={},format=raw,if=none,id=data0", data.display())])
        .args(["-device", "virtio-blk-pci,drive=data0"])
        .args(["-drive", &format!("file={},format=raw,if=none,id=data1", models.display())])
        .args(["-device", "virtio-blk-pci,drive=data1"])
        // Red de usuario (slirp): 10.0.2.0/24, host 2222→22 (SSH futuro)
        // y 7777→7 (echo).
        .args(["-netdev", "user,id=net0,hostfwd=tcp::7777-:7,hostfwd=tcp::2222-:22"])
        .args(["-device", "virtio-net-pci,netdev=net0"])
        // mon:stdio multiplexa monitor y serie: Ctrl-A X sale, Ctrl-A C monitor
        .args(["-serial", "mon:stdio"])
        .args(["-display", "none"])
        // Dispositivo de salida para tests: out 0xf4 -> exit((valor << 1) | 1)
        .args(["-device", "isa-debug-exit,iobase=0xf4,iosize=0x04"])
        .arg("-no-reboot");
    if gdb {
        // Congelado en arranque; conectar con: gdb -ex 'target remote :1234'
        qemu.args(["-s", "-S"]);
    }
    let status = qemu.status().expect("no se pudo ejecutar qemu-system-x86_64 (¿está instalado?)");
    exit(status.code().unwrap_or(0));
}
