//! Lee entero cada fichero de una imagen sosofs y cuenta los que fallan
//! (T42: ¿está sana la imagen de datos tras las escrituras desde el host?).
//! `escanear <datos.img> [/dir]`

use block_dev::FileBlockDevice;
use sosofs::Sosofs;

fn recorrer(fs: &mut Sosofs<FileBlockDevice>, dir: u64, ruta: &str, ok: &mut usize, mal: &mut Vec<String>) {
    let entradas = match fs.read_dir(dir) {
        Ok(e) => e,
        Err(e) => {
            mal.push(format!("{ruta}/: read_dir {e:?}"));
            return;
        }
    };
    for (nombre, ino) in entradas {
        let r = format!("{ruta}/{nombre}");
        let st = match fs.stat_inode(ino) {
            Ok(s) => s,
            Err(e) => {
                mal.push(format!("{r}: stat {e:?}"));
                continue;
            }
        };
        if st.file_type == sosofs::layout::FT_DIR {
            recorrer(fs, ino, &r, ok, mal);
        } else {
            match fs.read_file(ino) {
                Ok(_) => *ok += 1,
                Err(e) => mal.push(format!("{r}: {e:?}")),
            }
        }
    }
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let dev = FileBlockDevice::open(std::path::Path::new(&a[1])).expect("abrir imagen");
    let mut fs = Sosofs::mount(dev).expect("montar imagen");
    let raiz = a.get(2).map_or("/", |s| s.as_str());
    let dir = fs.resolve(raiz).expect("directorio");
    let (mut ok, mut mal) = (0, Vec::new());
    recorrer(&mut fs, dir, if raiz == "/" { "" } else { raiz }, &mut ok, &mut mal);
    println!("{ok} ficheros leídos, {} con error", mal.len());
    for m in mal.iter().take(200) {
        println!("  {m}");
    }
}
