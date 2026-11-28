//! Enrutador VFS: `/models/*` → sosomfs; resto → sosofs.

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use sosofs::layout::{FT_DIR, FT_FILE, InodeItem};
use sosofs::FsError as SosoFsError;
use sosomfs::FsError as SomFsError;

pub const SOSOMFS_BIT: u64 = 1 << 63;
const ENTRY_MODELS_ROOT: u32 = 0xFFFF_FFFF;
const ENTRY_MODEL_DIR: u32 = 0xFFFF_FFFE;
const ENTRY_SHARDS_DIR: u32 = 0xFFFF_FFFD;

pub(crate) fn is_sosomfs(ino: u64) -> bool {
    ino & SOSOMFS_BIT != 0
}

fn decode(ino: u64) -> (u32, u32) {
    let raw = ino & !SOSOMFS_BIT;
    ((raw >> 32) as u32, raw as u32)
}

fn encode(model_idx: u32, entry: u32) -> u64 {
    SOSOMFS_BIT | ((model_idx as u64) << 32) | (entry as u64)
}

// ---- ejecutables en marcha cuyo fichero se reemplaza ----
//
// Un ELF grande se carga **bajo demanda**: cada falta de página lee su trozo
// del inodo. Si mientras tanto alguien reemplaza o borra el fichero (instalar
// un paquete de sistema, `soso-pkg` reescribiéndose a sí mismo), sosofs suelta
// ese inodo y la siguiente falta muere con «lectura FS falló». Como en Unix, el
// contenido tiene que seguir vivo mientras lo use un proceso: antes de soltar un
// inodo en uso se copia entero a memoria, y las lecturas de ese inodo salen de
// la copia hasta que el último espacio de direcciones que lo usa desaparece.
// Los números de inodo de sosofs no se reutilizan, así que la copia no puede
// tapar a un fichero nuevo.

/// Inodos de sosofs con un ELF cargado bajo demanda → espacios que lo usan.
static EN_USO: spin::Mutex<alloc::collections::BTreeMap<u64, u32>> =
    spin::Mutex::new(alloc::collections::BTreeMap::new());
/// Contenido de los inodos en uso que ya no existen en sosofs.
static HUERFANOS: spin::Mutex<alloc::collections::BTreeMap<u64, Vec<u8>>> =
    spin::Mutex::new(alloc::collections::BTreeMap::new());
/// Atajo para no tomar el candado en cada lectura cuando no hay ninguno.
static HAY_HUERFANOS: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

/// Un espacio de direcciones empieza a leer páginas de `ino` bajo demanda.
pub fn retener_inodo(ino: u64) {
    if ino == 0 || is_sosomfs(ino) {
        return;
    }
    *EN_USO.lock().entry(ino).or_insert(0) += 1;
}

/// El espacio que lo retenía ha desaparecido. Con el último se libera la copia.
pub fn soltar_inodo(ino: u64) {
    if ino == 0 || is_sosomfs(ino) {
        return;
    }
    let ultimo = {
        let mut en_uso = EN_USO.lock();
        match en_uso.get_mut(&ino) {
            Some(n) if *n > 1 => {
                *n -= 1;
                false
            }
            Some(_) => {
                en_uso.remove(&ino);
                true
            }
            None => false,
        }
    };
    if ultimo && HAY_HUERFANOS.load(core::sync::atomic::Ordering::Acquire) {
        // Fuera del candado: soltar varios MiB no debe alargar la sección crítica.
        let copia = {
            let mut h = HUERFANOS.lock();
            let copia = h.remove(&ino);
            HAY_HUERFANOS.store(!h.is_empty(), core::sync::atomic::Ordering::Release);
            copia
        };
        drop(copia);
    }
}

/// Si `name` en `dir` es un ejecutable en uso, guarda su contenido antes de
/// que sosofs lo suelte. Se llama con el candado de sosofs cogido, así que no
/// hay hueco entre la copia y el reemplazo.
fn conservar_si_en_uso(fs: &mut crate::fs::Fs, dir: u64, name: &str) {
    let Ok(ino) = fs.lookup(dir, name) else {
        return;
    };
    if !EN_USO.lock().contains_key(&ino) || HUERFANOS.lock().contains_key(&ino) {
        return;
    }
    let Ok(st) = fs.stat_inode(ino) else {
        return;
    };
    if st.file_type != FT_FILE {
        return;
    }
    let size = st.size.get() as usize;
    let mut copia = Vec::new();
    if copia.try_reserve_exact(size).is_err() {
        crate::println!("vfs: sin memoria para conservar el ejecutable en uso (ino={ino})");
        return;
    }
    copia.resize(size, 0);
    if fs.read_file_range(ino, 0, size, &mut copia).is_err() {
        crate::println!("vfs: no pude conservar el ejecutable en uso (ino={ino})");
        return;
    }
    HUERFANOS.lock().insert(ino, copia);
    HAY_HUERFANOS.store(true, core::sync::atomic::Ordering::Release);
}

/// Lectura desde la copia de un inodo que ya no está en sosofs.
fn leer_huerfano(ino: u64, offset: usize, len: usize, out: &mut [u8]) -> Option<Result<(), SosoFsError>> {
    if !HAY_HUERFANOS.load(core::sync::atomic::Ordering::Acquire) {
        return None;
    }
    let h = HUERFANOS.lock();
    let copia = h.get(&ino)?;
    let fin = offset.checked_add(len)?;
    Some(match (copia.get(offset..fin), out.get_mut(..len)) {
        (Some(src), Some(dst)) => {
            dst.copy_from_slice(src);
            Ok(())
        }
        _ => Err(SosoFsError::Corrupt),
    })
}

pub fn is_models_path(path: &str) -> bool {
    path == "/models" || path.starts_with("/models/")
}

fn models_ready() -> bool {
    crate::fs::MODELS.get().is_some()
}

fn som_errno(e: SomFsError) -> SosoFsError {
    match e {
        SomFsError::NotFound => SosoFsError::NotFound,
        SomFsError::NotADir => SosoFsError::NotADir,
        SomFsError::NotAFile => SosoFsError::NotAFile,
        _ => SosoFsError::Io,
    }
}

pub fn resolve(path: &str) -> Result<u64, SosoFsError> {
    #[cfg(feature = "drv-usb")]
    if crate::drivers::usbfs::es_ruta(path) {
        return crate::drivers::usbfs::resolve(path);
    }
    if is_models_path(path) {
        if !models_ready() {
            return Err(SosoFsError::NotFound);
        }
        let mfs = crate::fs::MODELS.get().ok_or(SosoFsError::Io)?;
        let mfs = mfs.lock();
        if path == "/models" || path == "/models/" {
            return Ok(encode(0, ENTRY_MODELS_ROOT));
        }
        let (model, sub) = parse_models(path)?;
        let model_idx = mfs
            .catalog
            .models
            .iter()
            .position(|m| m.name == model)
            .ok_or(SosoFsError::NotFound)? as u32;
        if sub.is_empty() {
            return Ok(encode(model_idx, ENTRY_MODEL_DIR));
        }
        if sub == "shards" || sub == "shards/" {
            return Ok(encode(model_idx, ENTRY_SHARDS_DIR));
        }
        let rel = sub.to_string();
        let Some(shard_idx) = mfs
            .catalog
            .models
            .get(model_idx as usize)
            .and_then(|m| m.shards.iter().position(|s| s.rel_path == rel))
        else {
            return Err(SosoFsError::NotFound);
        };
        let shard_idx = shard_idx as u32;
        Ok(encode(model_idx, shard_idx))
    } else {
        let fs = crate::fs::FS.get().ok_or(SosoFsError::Io)?;
        fs.lock().resolve(path)
    }
}

fn parse_models(path: &str) -> Result<(&str, &str), SosoFsError> {
    let rest = path
        .trim_start_matches('/')
        .strip_prefix("models")
        .ok_or(SosoFsError::NotFound)?
        .strip_prefix('/')
        .unwrap_or("");
    if rest.is_empty() {
        return Err(SosoFsError::NotADir);
    }
    match rest.split_once('/') {
        Some((m, s)) => Ok((m, s)),
        None => Ok((rest, "")),
    }
}

/// ¿Está libre el candado del FS?
///
/// Para caminos de emergencia (panic, muerte de un proceso) que sólo pueden
/// intentar escribir si nadie lo tiene tomado: forzar su desbloqueo dejaría el
/// árbol CoW a medio commit, que es peor que perder la cola del log. Es una
/// sonda best-effort: otro core puede tomarlo justo después.
pub fn fs_disponible() -> bool {
    match crate::fs::FS.get() {
        Some(fs) => fs.try_lock().is_some(),
        None => false,
    }
}

pub fn stat_inode(ino: u64) -> Result<InodeItem, SosoFsError> {
    #[cfg(feature = "drv-usb")]
    if crate::drivers::usbfs::es(ino) {
        return crate::drivers::usbfs::stat(ino);
    }
    if !is_sosomfs(ino) {
        let fs = crate::fs::FS.get().ok_or(SosoFsError::Io)?;
        return fs.lock().stat_inode(ino);
    }
    let mfs = crate::fs::MODELS.get().ok_or(SosoFsError::Io)?;
    let mfs = mfs.lock();
    let (model_idx, entry) = decode(ino);
    let ft = match entry {
        ENTRY_MODELS_ROOT | ENTRY_MODEL_DIR | ENTRY_SHARDS_DIR => FT_DIR,
        _ => FT_FILE,
    };
    let size = if ft == FT_FILE {
        mfs.catalog
            .models
            .get(model_idx as usize)
            .and_then(|m| m.shards.get(entry as usize))
            .map(|s| s.byte_len)
            .unwrap_or(0)
    } else {
        0
    };
    Ok(InodeItem {
        file_type: ft,
        _pad: [0; 7],
        size: size.into(),
        mtime: 0.into(),
    })
}

pub fn read_dir(ino: u64) -> Result<Vec<(String, u64)>, SosoFsError> {
    #[cfg(feature = "drv-usb")]
    if crate::drivers::usbfs::es(ino) {
        return crate::drivers::usbfs::read_dir(ino);
    }
    if !is_sosomfs(ino) {
        let fs = crate::fs::FS.get().ok_or(SosoFsError::Io)?;
        let mut entries = fs.lock().read_dir(ino)?;
        if ino == sosofs::layout::ROOT_INODE && models_ready() {
            if !entries.iter().any(|(n, _)| n == "models") {
                entries.push((String::from("models"), encode(0, ENTRY_MODELS_ROOT)));
            }
        }
        // `/media` con los pendrives montados; tapa un /media de sosofs.
        #[cfg(feature = "drv-usb")]
        if ino == sosofs::layout::ROOT_INODE && crate::drivers::usbfs::hay_montajes() {
            entries.retain(|(n, _)| n != "media");
            entries.push((String::from("media"), crate::drivers::usbfs::RAIZ_MEDIA));
        }
        return Ok(entries);
    }
    let mfs = crate::fs::MODELS.get().ok_or(SosoFsError::Io)?;
    let mfs = mfs.lock();
    let (model_idx, entry) = decode(ino);
    if entry == ENTRY_MODELS_ROOT {
        return Ok(mfs
            .catalog
            .models
            .iter()
            .enumerate()
            .map(|(i, m)| (m.name.clone(), encode(i as u32, ENTRY_MODEL_DIR)))
            .collect());
    }
    let model = mfs
        .catalog
        .models
        .get(model_idx as usize)
        .ok_or(SosoFsError::NotFound)?;
    if entry == ENTRY_MODEL_DIR {
        let mut out = Vec::new();
        if let Some(i) = model.shards.iter().position(|s| s.rel_path == "manifest.som") {
            out.push((String::from("manifest.som"), encode(model_idx, i as u32)));
        }
        if let Some(i) = model.shards.iter().position(|s| s.rel_path == "index.som") {
            out.push((String::from("index.som"), encode(model_idx, i as u32)));
        }
        out.push((String::from("shards"), encode(model_idx, ENTRY_SHARDS_DIR)));
        return Ok(out);
    }
    if entry == ENTRY_SHARDS_DIR {
        return Ok(model
            .shards
            .iter()
            .enumerate()
            .filter(|(_, s)| s.rel_path.starts_with("shards/"))
            .map(|(i, s)| {
                let name = s
                    .rel_path
                    .strip_prefix("shards/")
                    .unwrap_or(&s.rel_path)
                    .to_string();
                (name, encode(model_idx, i as u32))
            })
            .collect());
    }
    Err(SosoFsError::NotADir)
}

pub fn read_file(ino: u64) -> Result<Vec<u8>, SosoFsError> {
    let mut buf = Vec::new();
    let st = stat_inode(ino)?;
    let size = st.size.get() as usize;
    buf.resize(size, 0);
    read_file_range(ino, 0, size, &mut buf)?;
    Ok(buf)
}

pub fn read_file_range(ino: u64, offset: usize, len: usize, out: &mut [u8]) -> Result<(), SosoFsError> {
    #[cfg(feature = "drv-usb")]
    if crate::drivers::usbfs::es(ino) {
        return crate::drivers::usbfs::read_range(ino, offset, len, out);
    }
    if !is_sosomfs(ino) {
        if let Some(r) = leer_huerfano(ino, offset, len, out) {
            return r;
        }
        let fs = crate::fs::FS.get().ok_or(SosoFsError::Io)?;
        return fs.lock().read_file_range(ino, offset, len, out);
    }
    let mfs = crate::fs::MODELS.get().ok_or(SosoFsError::Io)?;
    let mut mfs = mfs.lock();
    let (model_idx, entry) = decode(ino);
    let shard = mfs
        .catalog
        .models
        .get(model_idx as usize)
        .and_then(|m| m.shards.get(entry as usize))
        .cloned()
        .ok_or(SosoFsError::NotFound)?;
    mfs.read_range(&shard, offset, len, out)
        .map_err(som_errno)
}

/// Lectura para faltas de página de mmap: por la caché, pero sin reverificar
/// el CRC del extent. El consumidor del shard ya comprueba el payload, y
/// verificar aquí obliga a releer el extent entero (relleno incluido) por cada
/// página que caiga en su primer bloque. Mismo criterio que `read_file_range_direct`.
pub fn read_file_range_mmap(ino: u64, offset: usize, len: usize, out: &mut [u8]) -> Result<(), SosoFsError> {
    if !is_sosomfs(ino) {
        return read_file_range(ino, offset, len, out);
    }
    let mfs = crate::fs::MODELS.get().ok_or(SosoFsError::Io)?;
    let mut mfs = mfs.lock();
    let (model_idx, entry) = decode(ino);
    let shard = mfs
        .catalog
        .models
        .get(model_idx as usize)
        .and_then(|m| m.shards.get(entry as usize))
        .cloned()
        .ok_or(SosoFsError::NotFound)?;
    mfs.read_range_sin_crc(&shard, offset, len, out)
        .map_err(som_errno)
}

/// Lectura directa (sin caché de bloques) para rangos grandes de modelos;
/// el resto de inodos van por la ruta normal con caché.
/// Lectura agrupada que además deja los bloques en la caché: el racimo de las
/// faltas de página, que se relee al remapear un shard.
pub fn read_file_range_racimo(ino: u64, offset: usize, out: &mut [u8]) -> Result<(), SosoFsError> {
    if !is_sosomfs(ino) {
        let len = out.len();
        return read_file_range(ino, offset, len, out);
    }
    let mfs = crate::fs::MODELS.get().ok_or(SosoFsError::Io)?;
    let mut mfs = mfs.lock();
    let (model_idx, entry) = decode(ino);
    let shard = mfs
        .catalog
        .models
        .get(model_idx as usize)
        .and_then(|m| m.shards.get(entry as usize))
        .cloned()
        .ok_or(SosoFsError::NotFound)?;
    let len = out.len();
    mfs.read_range_cacheado(&shard, offset, len, out)
        .map_err(som_errno)
}

pub fn read_file_range_direct(ino: u64, offset: usize, out: &mut [u8]) -> Result<(), SosoFsError> {
    if !is_sosomfs(ino) {
        let len = out.len();
        return read_file_range(ino, offset, len, out);
    }
    let mfs = crate::fs::MODELS.get().ok_or(SosoFsError::Io)?;
    let mut mfs = mfs.lock();
    let (model_idx, entry) = decode(ino);
    let shard = mfs
        .catalog
        .models
        .get(model_idx as usize)
        .and_then(|m| m.shards.get(entry as usize))
        .cloned()
        .ok_or(SosoFsError::NotFound)?;
    let len = out.len();
    mfs.read_range_direct(&shard, offset, len, out)
        .map_err(som_errno)
}

pub fn create_file(dir: u64, name: &str, data: &[u8], mtime: u64) -> Result<u64, SosoFsError> {
    #[cfg(feature = "drv-usb")]
    if crate::drivers::usbfs::es(dir) {
        return crate::drivers::usbfs::create_file(dir, name, data, mtime);
    }
    if is_sosomfs(dir) {
        return Err(SosoFsError::Io);
    }
    let fs = crate::fs::FS.get().ok_or(SosoFsError::Io)?;
    let mut fs = fs.lock();
    conservar_si_en_uso(&mut fs, dir, name);
    fs.create_file(dir, name, data, mtime)
}

pub fn append_file(ino: u64, data: &[u8], mtime: u64) -> Result<(), SosoFsError> {
    #[cfg(feature = "drv-usb")]
    if crate::drivers::usbfs::es(ino) {
        return crate::drivers::usbfs::append_file(ino, data, mtime);
    }
    if is_sosomfs(ino) {
        return Err(SosoFsError::Io);
    }
    let fs = crate::fs::FS.get().ok_or(SosoFsError::Io)?;
    fs.lock().append_file(ino, data, mtime)
}

pub fn mkdir(dir: u64, name: &str, mtime: u64) -> Result<u64, SosoFsError> {
    #[cfg(feature = "drv-usb")]
    if crate::drivers::usbfs::es(dir) {
        return crate::drivers::usbfs::mkdir(dir, name, mtime);
    }
    if is_sosomfs(dir) {
        return Err(SosoFsError::Io);
    }
    let fs = crate::fs::FS.get().ok_or(SosoFsError::Io)?;
    fs.lock().mkdir(dir, name, mtime)
}

pub fn unlink(dir: u64, name: &str) -> Result<(), SosoFsError> {
    #[cfg(feature = "drv-usb")]
    if crate::drivers::usbfs::es(dir) {
        return crate::drivers::usbfs::unlink(dir, name);
    }
    if is_sosomfs(dir) {
        return Err(SosoFsError::Io);
    }
    let fs = crate::fs::FS.get().ok_or(SosoFsError::Io)?;
    let mut fs = fs.lock();
    conservar_si_en_uso(&mut fs, dir, name);
    fs.unlink(dir, name)
}

pub fn rename(
    old_dir: u64,
    old_name: &str,
    new_dir: u64,
    new_name: &str,
    mtime: u64,
) -> Result<(), SosoFsError> {
    #[cfg(feature = "drv-usb")]
    {
        let (a, b) = (crate::drivers::usbfs::es(old_dir), crate::drivers::usbfs::es(new_dir));
        if a && b {
            return crate::drivers::usbfs::rename(old_dir, old_name, new_dir, new_name);
        }
        if a || b {
            // Entre un pendrive y sosofs habría que copiar.
            return Err(SosoFsError::Io);
        }
    }
    if is_sosomfs(old_dir) || is_sosomfs(new_dir) {
        return Err(SosoFsError::Io);
    }
    let fs = crate::fs::FS.get().ok_or(SosoFsError::Io)?;
    let mut fs = fs.lock();
    conservar_si_en_uso(&mut fs, new_dir, new_name);
    fs.rename(old_dir, old_name, new_dir, new_name, mtime)
}

pub fn truncate_path(path: &str, size: u64, mtime: u64) -> Result<(), SosoFsError> {
    let ino = resolve(path)?;
    #[cfg(feature = "drv-usb")]
    if crate::drivers::usbfs::es(ino) {
        return crate::drivers::usbfs::truncate(ino, size, mtime);
    }
    if is_sosomfs(ino) {
        return Err(SosoFsError::Io);
    }
    let fs = crate::fs::FS.get().ok_or(SosoFsError::Io)?;
    fs.lock().truncate_file(ino, size, mtime)
}

pub fn utime_path(path: &str, mtime: u64) -> Result<(), SosoFsError> {
    let ino = resolve(path)?;
    #[cfg(feature = "drv-usb")]
    if crate::drivers::usbfs::es(ino) {
        return crate::drivers::usbfs::set_mtime(ino, mtime);
    }
    if is_sosomfs(ino) {
        return Err(SosoFsError::Io);
    }
    let fs = crate::fs::FS.get().ok_or(SosoFsError::Io)?;
    fs.lock().set_mtime(ino, mtime)
}

pub fn lookup(dir: u64, name: &str) -> Result<u64, SosoFsError> {
    let entries = read_dir(dir)?;
    entries
        .into_iter()
        .find(|(n, _)| n == name)
        .map(|(_, ino)| ino)
        .ok_or(SosoFsError::NotFound)
}

#[allow(dead_code)]
pub fn cache_policy_for_inode(ino: u64) -> u8 {
    if !is_sosomfs(ino) {
        return sosomfs::layout::CACHE_NORMAL;
    }
    let (model_idx, entry) = decode(ino);
    if let Some(mfs) = crate::fs::MODELS.get() {
        let mfs = mfs.lock();
        if let Some(shard) = mfs
            .catalog
            .models
            .get(model_idx as usize)
            .and_then(|m| m.shards.get(entry as usize))
        {
            return shard.cache_policy;
        }
    }
    sosomfs::layout::CACHE_PIN
}