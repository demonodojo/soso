//! Pendrives USB de datos montados en `/media/usbN` (FAT12/16/32).
//!
//! `usb_storage` avisa de cada pendrive que no es el disco del arranque
//! (`montar`) y de cada uno que se va (`desmontar`). Aquí se busca su volumen
//! FAT y se cuelga en `/media`; el VFS desvía a este módulo todo lo que
//! empiece por `/media` y todo inodo con `USBFS_BIT`.
//!
//! Inodo: `USBFS_BIT | montaje << 48 | nodo FAT (48 bits)`. El número de
//! montaje no se reutiliza enseguida (cuenta hasta 254 y vuelve a 0), para
//! que un descriptor abierto en un pendrive que ya se fue no caiga en el
//! siguiente: su E/S da error, que es lo que pasa en Linux.
//!
//! Candados: `MONTAJES` y luego `HOSTS` (cada E/S). `usb_storage` nunca llama
//! aquí con `HOSTS` cogido.

use alloc::string::String;
use alloc::vec::Vec;
use sosofs::layout::{FT_DIR, FT_FILE, InodeItem};
use sosofs::FsError;
use spin::Mutex;
use usbfat::{Disco, Fat, Hallazgo, Nodo, RAIZ};

pub const USBFS_BIT: u64 = 1 << 62;
const MONTAJE_SHIFT: u32 = 48;
const NODO_MASK: u64 = (1 << MONTAJE_SHIFT) - 1;
/// Montaje reservado para el directorio `/media`.
const MEDIA: u8 = 0xFF;
pub const RAIZ_MEDIA: u64 = USBFS_BIT | (MEDIA as u64) << MONTAJE_SHIFT;

struct UsbDisco {
    id: u32,
}

impl Disco for UsbDisco {
    fn leer(&mut self, lba: u64, buf: &mut [u8]) -> bool {
        crate::drivers::usb_storage::datos_leer(self.id, lba, buf)
    }
    fn escribir(&mut self, lba: u64, buf: &[u8]) -> bool {
        crate::drivers::usb_storage::datos_escribir(self.id, lba, buf)
    }
}

struct Montaje {
    mid: u8,
    disco: u32,
    nombre: String,
    fat: Fat<UsbDisco>,
}

static MONTAJES: Mutex<Vec<Montaje>> = Mutex::new(Vec::new());
static SIGUIENTE_MID: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);

pub fn es(ino: u64) -> bool {
    ino & crate::vfs::SOSOMFS_BIT == 0 && ino & USBFS_BIT != 0
}

pub fn es_ruta(path: &str) -> bool {
    path == "/media" || path.starts_with("/media/")
}

pub fn hay_montajes() -> bool {
    !MONTAJES.lock().is_empty()
}

fn ino(mid: u8, n: Nodo) -> u64 {
    USBFS_BIT | (mid as u64) << MONTAJE_SHIFT | (n.codificar() & NODO_MASK)
}

fn partir(ino: u64) -> (u8, Nodo) {
    (
        ((ino >> MONTAJE_SHIFT) & 0xFF) as u8,
        Nodo::decodificar(ino & NODO_MASK),
    )
}

fn errno(e: usbfat::Error) -> FsError {
    use usbfat::Error as E;
    match e {
        E::Io => FsError::Io,
        E::NoEncontrado => FsError::NotFound,
        E::Existe => FsError::Exists,
        E::NoEsDir => FsError::NotADir,
        E::EsDir => FsError::NotAFile,
        E::NoVacio => FsError::DirNotEmpty,
        E::SinEspacio | E::DirLleno => FsError::NoSpace,
        E::NombreInvalido => FsError::NameTooLong,
        E::Corrupto => FsError::Corrupt,
        E::NoSoportado => FsError::Io,
    }
}

/// Un pendrive de datos nuevo: buscar su FAT y colgarlo en `/media`.
pub fn montar(disco: u32, sectores: u64) {
    let mut d = UsbDisco { id: disco };
    let inicio = match usbfat::buscar_volumen(&mut d, sectores) {
        Hallazgo::Fat { inicio, .. } => inicio,
        Hallazgo::Otro(fs) => {
            crate::klogln!("usb: el pendrive (disco {disco}) es {fs}; soso sólo monta FAT12/16/32");
            return;
        }
        Hallazgo::Nada => {
            crate::klogln!("usb: el pendrive (disco {disco}) no tiene un volumen FAT");
            return;
        }
    };
    let fat = match Fat::montar(d, inicio) {
        Ok(f) => f,
        Err(e) => {
            crate::klogln!("usb: no pude montar el FAT del disco {disco}: {e:?}");
            return;
        }
    };
    let mut m = MONTAJES.lock();
    let mut n = 0;
    while m.iter().any(|x| x.nombre == alloc::format!("usb{n}")) {
        n += 1;
    }
    let nombre = alloc::format!("usb{n}");
    let mid = loop {
        let v = SIGUIENTE_MID.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        if v != MEDIA && !m.iter().any(|x| x.mid == v) {
            break v;
        }
    };
    crate::klogln!(
        "usb: pendrive montado en /media/{nombre} ({:?}, etiqueta «{}»)",
        fat.tipo(),
        fat.etiqueta()
    );
    m.push(Montaje {
        mid,
        disco,
        nombre,
        fat,
    });
}

/// El pendrive se ha ido. Lo escrito ya está en él: cada operación vuelca.
pub fn desmontar(disco: u32) {
    let mut m = MONTAJES.lock();
    if let Some(i) = m.iter().position(|x| x.disco == disco) {
        let x = m.remove(i);
        crate::klogln!("usb: desmontado /media/{}", x.nombre);
    }
}

fn con<T>(mid: u8, f: impl FnOnce(&mut Montaje) -> Result<T, usbfat::Error>) -> Result<T, FsError> {
    let mut m = MONTAJES.lock();
    let x = m.iter_mut().find(|x| x.mid == mid).ok_or(FsError::Io)?;
    let r = f(x).map_err(errno);
    // Lo que el pendrive tenga en caché, a la memoria flash: puede
    // desenchufarse en cuanto volvamos.
    let _ = crate::drivers::usb_storage::datos_flush(x.disco);
    r
}

pub fn resolve(path: &str) -> Result<u64, FsError> {
    let resto = path.strip_prefix("/media").ok_or(FsError::NotFound)?;
    let mut partes = resto.split('/').filter(|p| !p.is_empty());
    let Some(nombre) = partes.next() else {
        return Ok(RAIZ_MEDIA);
    };
    let mut m = MONTAJES.lock();
    let x = m.iter_mut().find(|x| x.nombre == nombre).ok_or(FsError::NotFound)?;
    let mut n = RAIZ;
    for p in partes {
        n = x.fat.buscar_en(n, p).map_err(errno)?;
    }
    Ok(ino(x.mid, n))
}

fn item(es_dir: bool, size: u64, mtime: u64) -> InodeItem {
    InodeItem {
        file_type: if es_dir { FT_DIR } else { FT_FILE },
        _pad: [0; 7],
        size: size.into(),
        mtime: mtime.into(),
    }
}

pub fn stat(i: u64) -> Result<InodeItem, FsError> {
    if i == RAIZ_MEDIA {
        return Ok(item(true, 0, 0));
    }
    let (mid, n) = partir(i);
    let mut m = MONTAJES.lock();
    let x = m.iter_mut().find(|x| x.mid == mid).ok_or(FsError::Io)?;
    let info = x.fat.info(n).map_err(errno)?;
    Ok(item(info.es_dir, info.tam, info.mtime))
}

pub fn read_dir(i: u64) -> Result<Vec<(String, u64)>, FsError> {
    if i == RAIZ_MEDIA {
        return Ok(MONTAJES
            .lock()
            .iter()
            .map(|x| (x.nombre.clone(), ino(x.mid, RAIZ)))
            .collect());
    }
    let (mid, n) = partir(i);
    let mut m = MONTAJES.lock();
    let x = m.iter_mut().find(|x| x.mid == mid).ok_or(FsError::Io)?;
    let l = x.fat.listar(n).map_err(errno)?;
    Ok(l.into_iter().map(|(nombre, nodo, _)| (nombre, ino(mid, nodo))).collect())
}

pub fn read_range(i: u64, offset: usize, len: usize, out: &mut [u8]) -> Result<(), FsError> {
    let (mid, n) = partir(i);
    let mut m = MONTAJES.lock();
    let x = m.iter_mut().find(|x| x.mid == mid).ok_or(FsError::Io)?;
    let out = out.get_mut(..len).ok_or(FsError::Corrupt)?;
    let leidos = x.fat.leer_fichero(n, offset as u64, out).map_err(errno)?;
    if leidos != len {
        // Como sosofs: un rango fuera del fichero es un error, no un EOF.
        return Err(FsError::Corrupt);
    }
    Ok(())
}

fn dir_de(i: u64) -> Result<(u8, Nodo), FsError> {
    if i == RAIZ_MEDIA {
        // `/media` es de soso, no de ningún pendrive.
        return Err(FsError::Io);
    }
    Ok(partir(i))
}

pub fn create_file(dir: u64, name: &str, data: &[u8], mtime: u64) -> Result<u64, FsError> {
    let (mid, d) = dir_de(dir)?;
    con(mid, |x| x.fat.escribir_fichero(d, name, data, mtime)).map(|n| ino(mid, n))
}

pub fn append_file(i: u64, data: &[u8], mtime: u64) -> Result<(), FsError> {
    let (mid, n) = dir_de(i)?;
    con(mid, |x| x.fat.anexar(n, data, mtime))
}

pub fn mkdir(dir: u64, name: &str, mtime: u64) -> Result<u64, FsError> {
    let (mid, d) = dir_de(dir)?;
    con(mid, |x| x.fat.crear_dir(d, name, mtime)).map(|n| ino(mid, n))
}

pub fn unlink(dir: u64, name: &str) -> Result<(), FsError> {
    let (mid, d) = dir_de(dir)?;
    con(mid, |x| x.fat.borrar(d, name))
}

pub fn rename(old_dir: u64, old: &str, new_dir: u64, new: &str) -> Result<(), FsError> {
    let (mo, od) = dir_de(old_dir)?;
    let (mn, nd) = dir_de(new_dir)?;
    if mo != mn {
        // Entre dos pendrives (o con sosofs) habría que copiar.
        return Err(FsError::Io);
    }
    con(mo, |x| x.fat.renombrar(od, old, nd, new)).map(|_| ())
}

pub fn truncate(i: u64, size: u64, mtime: u64) -> Result<(), FsError> {
    let (mid, n) = dir_de(i)?;
    con(mid, |x| x.fat.truncar(n, size, mtime))
}

pub fn set_mtime(i: u64, mtime: u64) -> Result<(), FsError> {
    let (mid, n) = dir_de(i)?;
    con(mid, |x| x.fat.poner_mtime_nodo(n, mtime))
}
