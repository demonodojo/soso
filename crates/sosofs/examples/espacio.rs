//! Bloques totales y libres de una imagen sosofs: `espacio <datos.img>`.

use block_dev::FileBlockDevice;
use sosofs::Sosofs;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let dev = FileBlockDevice::open(std::path::Path::new(&a[1])).expect("abrir imagen");
    let fs = Sosofs::mount(dev).expect("montar imagen");
    let (t, l) = (fs.block_count(), fs.free_blocks());
    println!("{} MiB en total, {} MiB libres ({:.1} %)", t * 4096 / 1048576, l * 4096 / 1048576, 100.0 * l as f64 / t as f64);
}
