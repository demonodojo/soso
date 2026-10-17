//! `FileMapper` del host basado en `mmap` de sólo lectura.
//!
//! Con [`crate::service::OwnedMapper`] cada shard se **lee entero a memoria anónima**: un modelo de 7 GB cuesta
//! 7 GB de RAM por proceso y nada se comparte. Con `mmap` los pesos viven en la caché de páginas del kernel: se
//! comparten entre procesos (un servidor y una prueba sobre el mismo modelo no duplican RAM) y el kernel puede
//! recuperarlos bajo presión en vez de matar a un proceso con el OOM killer.

use std::collections::HashMap;
use std::ffi::{c_int, c_void};
use std::os::fd::AsRawFd;

use soso_llm_core::source::{FileMapper, MappedShard};

const PROT_READ: c_int = 1;
const MAP_PRIVATE: c_int = 2;
/// Mapea todas las páginas de una vez: el modelo está troceado en un archivo por tensor (un experto son tres de ~1–2 MiB) y
/// tocarlos con fallos de página sueltos limitaba la copia a la GPU a ~1 GB/s.
const MAP_POPULATE: c_int = 0x8000;

unsafe extern "C" {
    fn mmap(
        addr: *mut c_void,
        len: usize,
        prot: c_int,
        flags: c_int,
        fd: c_int,
        off: i64,
    ) -> *mut c_void;
    fn munmap(addr: *mut c_void, len: usize) -> c_int;
}

#[derive(Default)]
pub struct MmapMapper {
    mapas: HashMap<u64, usize>,
}

impl FileMapper for MmapMapper {
    fn map_file(&mut self, path: &str) -> Result<MappedShard, ()> {
        let f = std::fs::File::open(path).map_err(|_| ())?;
        let len = f.metadata().map_err(|_| ())?.len() as usize;
        if len == 0 {
            return Err(());
        }
        let p = unsafe {
            mmap(
                std::ptr::null_mut(),
                len,
                PROT_READ,
                MAP_PRIVATE | if std::env::var_os("SOSO_SIN_POPULATE").is_some() { 0 } else { MAP_POPULATE },
                f.as_raw_fd(),
                0,
            )
        };
        if p as isize == -1 {
            return Err(());
        }
        let addr = p as u64;
        self.mapas.insert(addr, len);
        Ok(MappedShard { addr, len })
    }

    fn unmap_file(&mut self, shard: &MappedShard) {
        if let Some(len) = self.mapas.remove(&shard.addr) {
            unsafe { munmap(shard.addr as *mut c_void, len) };
        }
    }
}

impl Drop for MmapMapper {
    fn drop(&mut self) {
        for (addr, len) in self.mapas.drain() {
            unsafe { munmap(addr as *mut c_void, len) };
        }
    }
}
