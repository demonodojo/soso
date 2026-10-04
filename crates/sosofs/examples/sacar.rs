//! Copia un fichero de una imagen sosofs al host: `sacar <datos.img> </ruta/dentro> <salida>`
//! (T42: para inspeccionar con `readelf` lo que el guest enlazó).

use block_dev::FileBlockDevice;
use sosofs::Sosofs;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    if a.len() != 4 {
        eprintln!("uso: sacar <datos.img> </ruta/dentro> <salida>");
        std::process::exit(2);
    }
    let dev = FileBlockDevice::open(std::path::Path::new(&a[1])).expect("abrir imagen");
    let mut fs = Sosofs::mount(dev).expect("montar imagen");
    let ino = fs.resolve(&a[2]).expect("ruta dentro de la imagen");
    let bytes = fs.read_file(ino).expect("leer");
    std::fs::write(&a[3], &bytes).expect("escribir salida");
    println!("{} -> {} ({} bytes)", a[2], a[3], bytes.len());
}
