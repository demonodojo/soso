//! Lector independiente de una imagen sosofs construida dentro de soso (T42).
//!
//! Uso: `cargo run -p sosofs --features std --example verificar -- <datos.img>
//! <imagen-dentro> <directorio-fuente-dentro>`
//!
//! Monta `datos.img` (el disco de datos del guest, sosofs a pelo), saca de él la
//! imagen que `mkfs-soso` generó en el guest, la monta con **este** `sosofs`
//! (el del host, otro compilador y otro sistema) y comprueba que cada fichero
//! del directorio fuente aparece en ella con los mismos bytes. Sale con 0 si
//! todo coincide; imprime una línea por fichero (`ruta tamaño crc32c`).

use block_dev::{FileBlockDevice, MemBlockDevice, BLOCK_SIZE};
use sosofs::{crc32c, Sosofs};
use std::process::exit;

fn recorrer(
    fs: &mut Sosofs<FileBlockDevice>,
    dir: u64,
    ruta: &str,
    fuera: &mut Vec<(String, Vec<u8>)>,
) {
    for (nombre, ino) in fs.read_dir(dir).unwrap() {
        let r = format!("{ruta}/{nombre}");
        let st = fs.stat_inode(ino).unwrap();
        if st.file_type == sosofs::layout::FT_DIR {
            recorrer(fs, ino, &r, fuera);
        } else {
            fuera.push((r, fs.read_file(ino).unwrap()));
        }
    }
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    if a.len() != 4 {
        eprintln!("uso: verificar <datos.img> <imagen-dentro> <directorio-fuente-dentro>");
        exit(2);
    }
    let dev = FileBlockDevice::open(std::path::Path::new(&a[1])).expect("abrir datos.img");
    let mut datos = Sosofs::mount(dev).expect("montar datos.img");
    let ino = datos.resolve(&a[2]).expect("imagen dentro de datos.img");
    let bytes = datos.read_file(ino).expect("leer la imagen");
    let tmp = std::env::temp_dir().join("t42-imagen-extraida.sosofs");
    std::fs::write(&tmp, &bytes).unwrap();
    println!("imagen: {} bytes, crc32c {:08x}", bytes.len(), crc32c(&bytes));

    // Fuente: lo que había en el directorio dentro del guest.
    let src = datos.resolve(&a[3]).expect("directorio fuente");
    let mut fuente: Vec<(String, Vec<u8>)> = Vec::new();
    recorrer(&mut datos, src, "", &mut fuente);

    let dev2 = FileBlockDevice::open(&tmp).expect("abrir imagen extraída");
    let mut img = Sosofs::mount(dev2).expect("montar la imagen construida en soso");
    let raiz = img.resolve("/").unwrap();
    let mut dentro: Vec<(String, Vec<u8>)> = Vec::new();
    recorrer(&mut img, raiz, "", &mut dentro);

    let mut mal = 0;
    for (ruta, contenido) in &fuente {
        match dentro.iter().find(|(r, _)| r == ruta) {
            Some((_, c)) if c == contenido => {
                println!("ok   {ruta} {} {:08x}", c.len(), crc32c(c));
            }
            Some((_, c)) => {
                println!("DIFF {ruta}: fuente {} B, imagen {} B", contenido.len(), c.len());
                mal += 1;
            }
            None => {
                println!("FALTA {ruta}");
                mal += 1;
            }
        }
    }
    for (ruta, c) in &dentro {
        if !fuente.iter().any(|(r, _)| r == ruta) {
            println!("extra {ruta} {} {:08x}", c.len(), crc32c(c));
        }
    }
    let _ = (BLOCK_SIZE, MemBlockDevice::new(1));
    println!("{} ficheros fuente, {} discrepancias", fuente.len(), mal);
    exit(if mal == 0 { 0 } else { 1 });
}
