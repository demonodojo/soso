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

fn is_sosomfs(ino: u64) -> bool {
    ino & SOSOMFS_BIT != 0
}

fn decode(ino: u64) -> (u32, u32) {
    let raw = ino & !SOSOMFS_BIT;
    ((raw >> 32) as u32, raw as u32)
}

fn encode(model_idx: u32, entry: u32) -> u64 {
    SOSOMFS_BIT | ((model_idx as u64) << 32) | (entry as u64)
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

pub fn stat_inode(ino: u64) -> Result<InodeItem, SosoFsError> {
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
    if !is_sosomfs(ino) {
        let fs = crate::fs::FS.get().ok_or(SosoFsError::Io)?;
        let mut entries = fs.lock().read_dir(ino)?;
        if ino == sosofs::layout::ROOT_INODE && models_ready() {
            if !entries.iter().any(|(n, _)| n == "models") {
                entries.push((String::from("models"), encode(0, ENTRY_MODELS_ROOT)));
            }
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
    if !is_sosomfs(ino) {
        let fs = crate::fs::FS.get().ok_or(SosoFsError::Io)?;
        return fs.lock().read_file_range(ino, offset, len, out);
    }
    let mut mfs = crate::fs::MODELS.get().ok_or(SosoFsError::Io)?;
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

/// Lectura directa (sin caché de bloques) para rangos grandes de modelos;
/// el resto de inodos van por la ruta normal con caché.
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
    if is_sosomfs(dir) {
        return Err(SosoFsError::Io);
    }
    let fs = crate::fs::FS.get().ok_or(SosoFsError::Io)?;
    fs.lock().create_file(dir, name, data, mtime)
}

pub fn mkdir(dir: u64, name: &str, mtime: u64) -> Result<u64, SosoFsError> {
    if is_sosomfs(dir) {
        return Err(SosoFsError::Io);
    }
    let fs = crate::fs::FS.get().ok_or(SosoFsError::Io)?;
    fs.lock().mkdir(dir, name, mtime)
}

pub fn unlink(dir: u64, name: &str) -> Result<(), SosoFsError> {
    if is_sosomfs(dir) {
        return Err(SosoFsError::Io);
    }
    let fs = crate::fs::FS.get().ok_or(SosoFsError::Io)?;
    fs.lock().unlink(dir, name)
}

pub fn lookup(dir: u64, name: &str) -> Result<u64, SosoFsError> {
    let entries = read_dir(dir)?;
    entries
        .into_iter()
        .find(|(n, _)| n == name)
        .map(|(_, ino)| ino)
        .ok_or(SosoFsError::NotFound)
}

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