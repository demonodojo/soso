//! Renombra dentro de una imagen sosofs (con el QEMU parado): `mover <datos.img> </dir> <viejo> <nuevo>`.
//! T42: apartar un árbol con bloques corruptos sin recorrerlo.

use block_dev::FileBlockDevice;
use sosofs::Sosofs;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    if a.len() != 5 {
        eprintln!("uso: mover <datos.img> </dir> <viejo> <nuevo>");
        std::process::exit(2);
    }
    let dev = FileBlockDevice::open(std::path::Path::new(&a[1])).expect("abrir imagen");
    let mut fs = Sosofs::mount(dev).expect("montar imagen");
    let dir = fs.resolve(&a[2]).expect("directorio");
    fs.rename(dir, &a[3], dir, &a[4], 0).expect("rename");
    println!("{}/{} -> {}", a[2], a[3], a[4]);
}
