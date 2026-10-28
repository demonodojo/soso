//! E/S mínima sobre sosofs.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use libsoso::{abi, sys};

pub const O_RDONLY: u64 = abi::O_RDONLY;
pub const O_WRONLY: u64 = abi::O_WRONLY;

pub fn exists(path: &str) -> bool {
    let mut st = abi::Stat::default();
    sys::stat(path, &mut st) >= 0
}

pub fn read_bytes(path: &str, max: usize) -> Option<Vec<u8>> {
    let fd = sys::open(path, O_RDONLY);
    if fd < 0 {
        return None;
    }
    let mut out = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        let n = sys::read(fd as u64, &mut buf);
        if n <= 0 {
            break;
        }
        out.extend_from_slice(&buf[..n as usize]);
        if out.len() >= max {
            break;
        }
    }
    let _ = sys::close(fd as u64);
    Some(out)
}

pub fn write_bytes(path: &str, data: &[u8]) -> Result<(), i64> {
    let fd = sys::open(path, O_WRONLY);
    if fd < 0 {
        return Err(fd);
    }
    if let Err(e) = sys::write_all(fd as u64, data) {
        let _ = sys::close(fd as u64);
        return Err(e);
    }
    let r = sys::close(fd as u64);
    if r < 0 {
        Err(r)
    } else {
        Ok(())
    }
}

pub fn mkdir_all(path: &str) {
    if path.is_empty() || exists(path) {
        return;
    }
    if let Some(p) = path.rfind('/') {
        if p > 0 {
            mkdir_all(&path[..p]);
        }
    }
    let _ = sys::mkdir(path);
}

pub fn list_dir(path: &str) -> Vec<String> {
    let fd = sys::open(path, O_RDONLY);
    if fd < 0 {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut ents = [abi::Dirent::default(); 32];
    loop {
        let n = sys::getdents(fd as u64, &mut ents);
        if n <= 0 {
            break;
        }
        let cuantas = (n as usize / abi::DIRENT_SIZE).min(ents.len());
        for e in &ents[..cuantas] {
            if let Ok(nombre) = core::str::from_utf8(e.name_bytes()) {
                if !nombre.is_empty() && nombre != "." && nombre != ".." {
                    out.push(nombre.into());
                }
            }
        }
    }
    let _ = sys::close(fd as u64);
    out
}

pub fn borrar_arbol(ruta: &str) {
    let fd = sys::open(ruta, O_RDONLY);
    if fd >= 0 {
        let hijos = list_dir(ruta);
        let _ = sys::close(fd as u64);
        for h in hijos {
            borrar_arbol(&format!("{ruta}/{h}"));
        }
    }
    let _ = sys::unlink(ruta);
}

pub fn nombres_bin_sistema() -> Vec<String> {
    list_dir("/bin")
}

pub fn espacio_libre() -> u64 {
    let mut fs = abi::FsInfo::default();
    if sys::fsinfo(&mut fs) >= 0 && fs.block_size > 0 {
        fs.free_blocks.saturating_mul(fs.block_size)
    } else {
        u64::MAX
    }
}
