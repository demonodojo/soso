//! Sustituye un fichero —o un directorio entero— dentro de una imagen sosofs con
//! el QEMU parado (T42).
//!
//! `SOSO_REUSE_DATA=1` arranca con la imagen de datos tal cual, así que un
//! binario nuevo del rootfs (p. ej. `wild-soso`) no llega al guest. Esto lo
//! escribe directamente: `poner <datos.img> </ruta/dentro> <fichero-o-dir-local>`.
//! Con un directorio crea los intermedios y reemplaza lo que ya haya.

use block_dev::FileBlockDevice;
use sosofs::Sosofs;
use std::path::Path;
use std::process::exit;

type Fs = Sosofs<FileBlockDevice>;

/// Directorio `nombre` dentro de `dir`, creándolo si no existe.
/// Segundos desde la época: con `mtime = 0` cargo toma lo escrito por viejo y no recompila.
fn ahora() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn asegurar_dir(fs: &mut Fs, dir: u64, nombre: &str) -> u64 {
    match fs.lookup(dir, nombre) {
        Ok(ino) => ino,
        Err(_) => fs.mkdir(dir, nombre, ahora()).expect("mkdir"),
    }
}

fn copiar(fs: &mut Fs, dir: u64, local: &Path, n: &mut usize) {
    for e in std::fs::read_dir(local).expect("leer directorio") {
        let e = e.unwrap();
        let nombre = e.file_name().to_string_lossy().into_owned();
        let ty = e.file_type().unwrap();
        if ty.is_dir() {
            let sub = asegurar_dir(fs, dir, &nombre);
            copiar(fs, sub, &e.path(), n);
        } else if ty.is_file() {
            let datos = std::fs::read(e.path()).expect("leer fichero local");
            fs.create_file(dir, &nombre, &datos, ahora()).expect("escribir");
            *n += 1;
        }
    }
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    if a.len() != 4 {
        eprintln!("uso: poner <datos.img> </ruta/dentro> <fichero-o-dir-local>");
        exit(2);
    }
    let dev = FileBlockDevice::open(Path::new(&a[1])).expect("abrir imagen");
    let mut fs = Sosofs::mount(dev).expect("montar imagen");
    let local = Path::new(&a[3]);
    let mut n = 0;
    if local.is_dir() {
        let mut dir = fs.resolve("/").expect("raíz");
        for comp in a[2].split('/').filter(|c| !c.is_empty()) {
            dir = asegurar_dir(&mut fs, dir, comp);
        }
        copiar(&mut fs, dir, local, &mut n);
    } else {
        let (dir, nombre) = a[2].rsplit_once('/').expect("ruta absoluta");
        // Crea los directorios intermedios que falten (`mkdir -p`).
        let mut d = fs.resolve("/").expect("raíz");
        for comp in dir.split('/').filter(|c| !c.is_empty()) {
            d = asegurar_dir(&mut fs, d, comp);
        }
        let dir = d;
        let datos = std::fs::read(local).expect("leer fichero local");
        fs.create_file(dir, nombre, &datos, ahora()).expect("escribir");
        n = 1;
    }
    println!("{} <- {} ({n} ficheros)", a[2], a[3]);
}
