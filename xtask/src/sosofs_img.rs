//! Acceso al sosofs **dentro de una imagen de disco**, para que las pruebas
//! puedan inyectar averías donde de verdad duelen.
//!
//! No es una utilidad de usuario: existe para fabricar estados que en la vida
//! real produce un corte de corriente o un sector que se va —un respaldo que ya
//! no cuadra con su hash, por ejemplo— y comprobar que el arranque los trata
//! como toca en vez de seguir adelante.

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use block_dev::{Block, BlockDevice, BlockError};

/// Una partición de una imagen, vista como bloques de 4 KiB.
pub struct Particion {
    f: File,
    base: u64,
    bloques: u64,
}

impl BlockDevice for Particion {
    fn block_count(&self) -> u64 {
        self.bloques
    }
    fn read_block(&mut self, block: u64, buf: &mut Block) -> Result<(), BlockError> {
        self.f
            .seek(SeekFrom::Start(self.base + block * 4096))
            .map_err(|_| BlockError::Io)?;
        self.f.read_exact(buf).map_err(|_| BlockError::Io)
    }
    fn write_block(&mut self, block: u64, buf: &Block) -> Result<(), BlockError> {
        self.f
            .seek(SeekFrom::Start(self.base + block * 4096))
            .map_err(|_| BlockError::Io)?;
        self.f.write_all(buf).map_err(|_| BlockError::Io)
    }
    fn flush(&mut self) -> Result<(), BlockError> {
        self.f.sync_all().map_err(|_| BlockError::Io)
    }
}

/// Monta el sosofs que empiece en `lba` dentro de `img`.
pub fn montar(img: &Path, lba: u64, sectores: u64) -> Result<sosofs::Sosofs<Particion>, String> {
    let f = OpenOptions::new()
        .read(true)
        .write(true)
        .open(img)
        .map_err(|e| format!("abrir {}: {e}", img.display()))?;
    let dev = Particion {
        f,
        base: lba * 512,
        bloques: sectores / 8,
    };
    sosofs::Sosofs::mount(dev).map_err(|e| format!("montar sosofs: {e:?}"))
}

/// Primer directorio de `/var/lib/soso-update/` que tenga respaldos.
pub fn dir_operacion(fs: &mut sosofs::Sosofs<Particion>) -> Option<String> {
    let base = fs.resolve("/var/lib/soso-update").ok()?;
    for (nombre, ino) in fs.read_dir(base).ok()? {
        if nombre.starts_with('.') {
            continue;
        }
        if fs.lookup(ino, "respaldo").is_ok() {
            return Some(nombre);
        }
    }
    None
}

/// Estropea un fichero **dentro** del sosofs: el contenido deja de cuadrar con
/// el hash que anota el diario, que es exactamente lo que ve el arranque
/// cuando un respaldo se corrompe.
pub fn estropear(fs: &mut sosofs::Sosofs<Particion>, ruta: &str) -> Result<String, String> {
    let (dir, nombre) = ruta.rsplit_once('/').ok_or("ruta sin directorio")?;
    let padre = fs
        .resolve(dir)
        .map_err(|e| format!("resolver {dir}: {e:?}"))?;
    let ino = fs
        .lookup(padre, nombre)
        .map_err(|e| format!("buscar {nombre}: {e:?}"))?;
    let mut datos = fs.read_file(ino).map_err(|e| format!("leer: {e:?}"))?;
    if datos.is_empty() {
        datos.push(0);
    }
    datos[0] ^= 0xFF;
    fs.unlink(padre, nombre).map_err(|e| format!("unlink: {e:?}"))?;
    fs.create_file(padre, nombre, &datos, 0)
        .map_err(|e| format!("escribir: {e:?}"))?;
    Ok(ruta.to_string())
}

/// Primer fichero que cuelgue de `dir`, recorriendo hacia dentro.
pub fn primer_fichero(fs: &mut sosofs::Sosofs<Particion>, dir: &str) -> Option<String> {
    let ino = fs.resolve(dir).ok()?;
    for (nombre, hijo) in fs.read_dir(ino).ok()? {
        if nombre == "." || nombre == ".." {
            continue;
        }
        let st = fs.stat_inode(hijo).ok()?;
        let ruta = format!("{dir}/{nombre}");
        if st.file_type == sosofs::layout::FT_DIR {
            if let Some(r) = primer_fichero(fs, &ruta) {
                return Some(r);
            }
        } else {
            return Some(ruta);
        }
    }
    None
}
