//! Agranda un sosofs (con el QEMU parado): `crecer <datos.img> <MiB>`.
//! T42: el volumen de datos del guest era de 1280 MiB y se llenó (ENOSPC al crear ficheros).

use block_dev::FileBlockDevice;
use sosofs::Sosofs;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let mib: u64 = a[2].parse().expect("MiB");
    let dev = FileBlockDevice::open(std::path::Path::new(&a[1])).expect("abrir imagen");
    let mut fs = Sosofs::mount(dev).expect("montar imagen");
    fs.grow_to(mib * 1048576 / 4096).expect("grow_to");
    println!("{} MiB en total, {} MiB libres", fs.block_count() * 4096 / 1048576, fs.free_blocks() * 4096 / 1048576);
}
