//! Sustituye un fichero dentro de una imagen sosofs con el QEMU parado (T42).
//!
//! `SOSO_REUSE_DATA=1` arranca con la imagen de datos tal cual, así que un
//! binario nuevo del rootfs (p. ej. `wild-soso`) no llega al guest. Esto lo
//! escribe directamente: `poner <datos.img> </ruta/dentro> <fichero-local>`.

use block_dev::FileBlockDevice;
use sosofs::Sosofs;
use std::process::exit;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    if a.len() != 4 {
        eprintln!("uso: poner <datos.img> </ruta/dentro> <fichero-local>");
        exit(2);
    }
    let dev = FileBlockDevice::open(std::path::Path::new(&a[1])).expect("abrir imagen");
    let mut fs = Sosofs::mount(dev).expect("montar imagen");
    let (dir, nombre) = a[2].rsplit_once('/').expect("ruta absoluta");
    let dir = fs.resolve(if dir.is_empty() { "/" } else { dir }).expect("directorio");
    let datos = std::fs::read(&a[3]).expect("leer fichero local");
    fs.create_file(dir, nombre, &datos, 0).expect("escribir");
    println!("{} <- {} ({} bytes)", a[2], a[3], datos.len());
}
