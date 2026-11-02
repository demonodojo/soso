use std::fs::File;
use std::io;

pub struct MmapInner {
    ptr: *mut u8,
    len: usize,
}

impl MmapInner {
    fn new() -> io::Result<MmapInner> {
        Err(io::Error::new(
            io::ErrorKind::Other,
            "platform not supported",
        ))
    }

    // C-001: `lib.rs` pasa `populate` también en plataformas sin unix/windows.
    // C-050: en soso un mapa de sólo lectura es `SYS_MMAP` del fd. El kernel
    // exige `offset + len <= tamaño` (sin redondear) y rellena las páginas
    // al fallar. `map_mut` y el anónimo siguen sin estar.
    pub fn map(len: usize, file: &File, offset: u64, _populate: bool) -> io::Result<MmapInner> {
        #[cfg(target_os = "soso")]
        {
            return map_soso(len, file.as_raw_fd_soso(), offset);
        }
        #[cfg(not(target_os = "soso"))]
        {
            let _ = (len, file, offset);
            MmapInner::new()
        }
    }

    pub fn map_exec(_: usize, _: &File, _: u64, _populate: bool) -> io::Result<MmapInner> {
        MmapInner::new()
    }

    pub fn map_mut(_: usize, _: &File, _: u64, _populate: bool) -> io::Result<MmapInner> {
        MmapInner::new()
    }

    pub fn map_copy(_: usize, _: &File, _: u64, _populate: bool) -> io::Result<MmapInner> {
        MmapInner::new()
    }

    pub fn map_copy_read_only(
        len: usize,
        file: &File,
        offset: u64,
        populate: bool,
    ) -> io::Result<MmapInner> {
        // rustc lee metadatos con `map_copy_read_only`. En soso el mapa de
        // fichero ya es de sólo lectura, así que vale el mismo `SYS_MMAP`.
        Self::map(len, file, offset, populate)
    }

    pub fn map_anon(_: usize, _: bool) -> io::Result<MmapInner> {
        MmapInner::new()
    }

    pub fn flush(&self, _: usize, _: usize) -> io::Result<()> {
        Ok(())
    }

    pub fn flush_async(&self, _: usize, _: usize) -> io::Result<()> {
        Ok(())
    }

    pub fn make_read_only(&mut self) -> io::Result<()> {
        Ok(())
    }

    pub fn make_exec(&mut self) -> io::Result<()> {
        MmapInner::new().map(|_| ())
    }

    pub fn make_mut(&mut self) -> io::Result<()> {
        MmapInner::new().map(|_| ())
    }

    #[inline]
    pub fn ptr(&self) -> *const u8 {
        self.ptr
    }

    #[inline]
    pub fn mut_ptr(&mut self) -> *mut u8 {
        self.ptr
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }
}

unsafe impl Sync for MmapInner {}
unsafe impl Send for MmapInner {}

impl Drop for MmapInner {
    fn drop(&mut self) {
        #[cfg(target_os = "soso")]
        if self.len != 0 {
            unsafe {
                let _ = syscall4(16, self.ptr.expose_provenance() as u64, self.len as u64, 0, 0);
            }
        }
    }
}

#[cfg(target_os = "soso")]
fn map_soso(len: usize, fd: u64, offset: u64) -> io::Result<MmapInner> {
    if len == 0 {
        return Ok(MmapInner { ptr: core::ptr::NonNull::<u8>::dangling().as_ptr(), len: 0 });
    }
    // SAFETY: `SYS_MMAP` = 15. El kernel devuelve la dirección o `-errno`.
    let rc = unsafe { syscall4(15, 0, len as u64, fd, offset) };
    if rc < 0 {
        return Err(io::Error::from_raw_os_error((-rc) as i32));
    }
    Ok(MmapInner {
        ptr: core::ptr::with_exposed_provenance_mut(rc as usize),
        len,
    })
}

#[cfg(target_os = "soso")]
unsafe fn syscall4(n: u64, a1: u64, a2: u64, a3: u64, a4: u64) -> i64 {
    let ret: i64;
    unsafe {
        core::arch::asm!(
            "syscall",
            inlateout("rax") n => ret,
            inlateout("rdi") a1 => _,
            inlateout("rsi") a2 => _,
            inlateout("rdx") a3 => _,
            inlateout("r10") a4 => _,
            lateout("rcx") _,
            lateout("r11") _,
            clobber_abi("C"),
        );
    }
    ret
}
