//! Borra de una imagen sosofs (con el QEMU parado) todos los ficheros cuyo
//! nombre termina en `<sufijo>` bajo un directorio: `borrar <datos.img> </dir> <sufijo>`.
//! T42: tirar los `.so` de proc-macro enlazados con un `wild-soso` anterior.

use block_dev::FileBlockDevice;
use sosofs::Sosofs;

fn recorrer(fs: &mut Sosofs<FileBlockDevice>, dir: u64, suf: &str, n: &mut usize) {
    for (nombre, ino) in fs.read_dir(dir).unwrap() {
        let st = fs.stat_inode(ino).unwrap();
        if st.file_type == sosofs::layout::FT_DIR {
            recorrer(fs, ino, suf, n);
        } else if nombre.ends_with(suf) {
            fs.unlink(dir, &nombre).unwrap();
            *n += 1;
        }
    }
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    if a.len() != 4 {
        eprintln!("uso: borrar <datos.img> </dir> <sufijo>");
        std::process::exit(2);
    }
    let dev = FileBlockDevice::open(std::path::Path::new(&a[1])).expect("abrir imagen");
    let mut fs = Sosofs::mount(dev).expect("montar imagen");
    let dir = fs.resolve(&a[2]).expect("directorio");
    let mut n = 0;
    recorrer(&mut fs, dir, &a[3], &mut n);
    println!("{n} ficheros borrados");
}
