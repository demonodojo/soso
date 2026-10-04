//! Lista un directorio de una imagen sosofs: `listar <datos.img> </dir>` (nombre y tamaño).

use block_dev::FileBlockDevice;
use sosofs::Sosofs;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    if a.len() != 3 {
        eprintln!("uso: listar <datos.img> </dir>");
        std::process::exit(2);
    }
    let dev = FileBlockDevice::open(std::path::Path::new(&a[1])).expect("abrir imagen");
    let mut fs = Sosofs::mount(dev).expect("montar imagen");
    let dir = fs.resolve(&a[2]).expect("directorio");
    for (nombre, ino) in fs.read_dir(dir).unwrap() {
        let st = fs.stat_inode(ino).unwrap();
        println!("{nombre} {}", st.size.get());
    }
}
