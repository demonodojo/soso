//! Ficheros de soso. Leer un fuente es `SYS_OPEN` + `SYS_READ`.
//! Listar un directorio es `SYS_GETDENTS`. Saber si un camino existe es
//! `SYS_STAT` (`Path::exists` lo usa para no descartar los `.rlib`).
//! El tamaño de un descriptor abierto es `SYS_FSTAT`.
//! Crear un directorio es `SYS_MKDIR`.
//! Crear un fichero es `SYS_OPEN` con `O_CREAT`.
//! Escribir el contenido es `SYS_WRITE`. `copy` lee el origen y lo
//! escribe en el destino.

use crate::ffi::OsString;
use crate::fmt;
use crate::fs::TryLockError;
use crate::io::{self, BorrowedCursor, IoSlice, IoSliceMut, SeekFrom};
use crate::os::soso::ffi::{OsStrExt, OsStringExt};
use crate::path::{Path, PathBuf};
pub use crate::sys::fs::common::Dir;
use crate::sys::time::SystemTime;
use crate::sys::unsupported;

pub struct File {
    fd: u64,
}

#[derive(Clone, Copy)]
pub struct FileAttr {
    size: u64,
    file_type: u8,
}

pub struct ReadDir {
    fd: u64,
    path: PathBuf,
    buf: Vec<u8>,
    pos: usize,
    filled: usize,
}

pub struct DirEntry {
    dir: PathBuf,
    name: OsString,
}

#[derive(Clone, Debug)]
pub struct OpenOptions {
    read: bool,
    write: bool,
    append: bool,
    truncate: bool,
    create: bool,
    create_new: bool,
}

#[derive(Copy, Clone, Debug, Default)]
pub struct FileTimes {}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FilePermissions {
    readonly: bool,
}

#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub struct FileType(u8);

#[derive(Debug)]
pub struct DirBuilder {}

impl FileAttr {
    pub fn size(&self) -> u64 {
        self.size
    }

    pub fn perm(&self) -> FilePermissions {
        // soso no guarda bits de permiso. Un valor escribible evita el
        // pánico al enlazar, que pregunta `metadata().permissions()`.
        FilePermissions { readonly: false }
    }

    pub fn file_type(&self) -> FileType {
        FileType(self.file_type)
    }

    pub fn modified(&self) -> io::Result<SystemTime> {
        unsupported()
    }

    pub fn accessed(&self) -> io::Result<SystemTime> {
        unsupported()
    }

    pub fn created(&self) -> io::Result<SystemTime> {
        unsupported()
    }
}

impl FilePermissions {
    pub fn readonly(&self) -> bool {
        self.readonly
    }

    pub fn set_readonly(&mut self, readonly: bool) {
        self.readonly = readonly;
    }
}

impl FileTimes {
    pub fn set_accessed(&mut self, _t: SystemTime) {}
    pub fn set_modified(&mut self, _t: SystemTime) {}
}

impl FileType {
    pub fn is_dir(&self) -> bool {
        self.0 == soso_rt::FT_DIR
    }

    pub fn is_file(&self) -> bool {
        self.0 == soso_rt::FT_FILE
    }

    pub fn is_symlink(&self) -> bool {
        false
    }
}

impl fmt::Debug for ReadDir {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ReadDir").field("fd", &self.fd).finish()
    }
}

impl ReadDir {
    fn siguiente(&mut self) -> io::Result<Option<DirEntry>> {
        loop {
            if self.pos >= self.filled {
                let n = map_syscall(unsafe {
                    soso_rt::syscall3(
                        soso_rt::SYS_GETDENTS,
                        self.fd,
                        self.buf.as_mut_ptr().expose_provenance() as u64,
                        self.buf.len() as u64,
                    )
                })?;
                if n == 0 {
                    return Ok(None);
                }
                self.filled = n;
                self.pos = 0;
            }
            let nrec = soso_rt::DIRENT_SIZE;
            if self.filled - self.pos < nrec {
                return Err(io::Error::from_raw_os_error(soso_rt::EIO as i32));
            }
            let rec = &self.buf[self.pos..self.pos + nrec];
            self.pos += nrec;
            let name_len = rec[9] as usize;
            let name = &rec[10..10 + name_len];
            if name.is_empty() || name == b"." || name == b".." {
                continue;
            }
            return Ok(Some(DirEntry {
                dir: self.path.clone(),
                name: OsString::from_vec(name.to_vec()),
            }));
        }
    }
}

impl Drop for ReadDir {
    fn drop(&mut self) {
        unsafe {
            let _ = soso_rt::syscall1(soso_rt::SYS_CLOSE, self.fd);
        }
    }
}

impl Iterator for ReadDir {
    type Item = io::Result<DirEntry>;

    fn next(&mut self) -> Option<io::Result<DirEntry>> {
        match self.siguiente() {
            Ok(Some(e)) => Some(Ok(e)),
            Ok(None) => None,
            Err(e) => Some(Err(e)),
        }
    }
}

impl DirEntry {
    pub fn path(&self) -> PathBuf {
        self.dir.join(&self.name)
    }

    pub fn file_name(&self) -> OsString {
        self.name.clone()
    }

    pub fn metadata(&self) -> io::Result<FileAttr> {
        unsupported()
    }

    pub fn file_type(&self) -> io::Result<FileType> {
        unsupported()
    }
}

impl OpenOptions {
    pub fn new() -> OpenOptions {
        OpenOptions {
            read: false,
            write: false,
            append: false,
            truncate: false,
            create: false,
            create_new: false,
        }
    }

    pub fn read(&mut self, read: bool) {
        self.read = read;
    }
    pub fn write(&mut self, write: bool) {
        self.write = write;
    }
    pub fn append(&mut self, append: bool) {
        self.append = append;
    }
    pub fn truncate(&mut self, truncate: bool) {
        self.truncate = truncate;
    }
    pub fn create(&mut self, create: bool) {
        self.create = create;
    }
    pub fn create_new(&mut self, create_new: bool) {
        self.create_new = create_new;
    }
}

fn map_syscall(n: i64) -> io::Result<usize> {
    if n < 0 { Err(io::Error::from_raw_os_error((-n) as i32)) } else { Ok(n as usize) }
}

impl File {
    pub fn open(path: &Path, opts: &OpenOptions) -> io::Result<File> {
        let escribir = opts.write || opts.append || opts.truncate || opts.create || opts.create_new;
        if !opts.read && !escribir {
            return unsupported();
        }
        let mut flags = if escribir { soso_rt::O_WRONLY } else { soso_rt::O_RDONLY };
        if opts.append {
            flags |= soso_rt::O_APPEND;
        }
        if opts.truncate {
            flags |= soso_rt::O_TRUNC;
        }
        if opts.create || opts.create_new {
            flags |= soso_rt::O_CREAT;
        }
        if opts.create_new {
            flags |= soso_rt::O_EXCL;
        }
        let bytes = path.as_os_str().as_bytes();
        if bytes.is_empty() {
            return Err(io::Error::from_raw_os_error(soso_rt::EINVAL as i32));
        }
        // SAFETY: `bytes` vive en esta llamada; el kernel copia el camino.
        let fd = unsafe {
            soso_rt::syscall3(
                soso_rt::SYS_OPEN,
                bytes.as_ptr().expose_provenance() as u64,
                bytes.len() as u64,
                flags,
            )
        };
        if fd < 0 {
            return Err(io::Error::from_raw_os_error((-fd) as i32));
        }
        Ok(File { fd: fd as u64 })
    }

    pub(crate) fn as_raw_fd(&self) -> u64 {
        self.fd
    }

    pub fn file_attr(&self) -> io::Result<FileAttr> {
        let mut st = soso_rt::Stat::default();
        // SAFETY: `st` vive en esta llamada; el kernel escribe un `Stat`.
        let rc = unsafe {
            soso_rt::syscall3(
                soso_rt::SYS_FSTAT,
                self.fd,
                (&mut st as *mut soso_rt::Stat).expose_provenance() as u64,
                0,
            )
        };
        if rc < 0 {
            return Err(io::Error::from_raw_os_error((-rc) as i32));
        }
        Ok(FileAttr { size: st.size, file_type: st.file_type })
    }

    pub fn fsync(&self) -> io::Result<()> {
        unsupported()
    }

    pub fn datasync(&self) -> io::Result<()> {
        unsupported()
    }

    pub fn lock(&self) -> io::Result<()> {
        unsupported()
    }

    pub fn lock_shared(&self) -> io::Result<()> {
        unsupported()
    }

    pub fn try_lock(&self) -> Result<(), TryLockError> {
        Err(TryLockError::Error(crate::sys::unsupported_err()))
    }

    pub fn try_lock_shared(&self) -> Result<(), TryLockError> {
        Err(TryLockError::Error(crate::sys::unsupported_err()))
    }

    pub fn unlock(&self) -> io::Result<()> {
        unsupported()
    }

    pub fn truncate(&self, _size: u64) -> io::Result<()> {
        unsupported()
    }

    pub fn read(&self, buf: &mut [u8]) -> io::Result<usize> {
        // SAFETY: `buf` es exclusivo; el kernel sólo escribe dentro.
        map_syscall(unsafe {
            soso_rt::syscall3(
                soso_rt::SYS_READ,
                self.fd,
                buf.as_mut_ptr().expose_provenance() as u64,
                buf.len() as u64,
            )
        })
    }

    pub fn read_vectored(&self, bufs: &mut [IoSliceMut<'_>]) -> io::Result<usize> {
        io::default_read_vectored(|b| self.read(b), bufs)
    }

    pub fn is_read_vectored(&self) -> bool {
        false
    }

    pub fn read_buf(&self, cursor: BorrowedCursor<'_, u8>) -> io::Result<()> {
        io::default_read_buf(|b| self.read(b), cursor)
    }

    pub fn write(&self, buf: &[u8]) -> io::Result<usize> {
        // SAFETY: `buf` vive en esta llamada; el kernel sólo lee dentro.
        map_syscall(unsafe {
            soso_rt::syscall3(
                soso_rt::SYS_WRITE,
                self.fd,
                buf.as_ptr().expose_provenance() as u64,
                buf.len() as u64,
            )
        })
    }

    pub fn write_vectored(&self, bufs: &[IoSlice<'_>]) -> io::Result<usize> {
        io::default_write_vectored(|b| self.write(b), bufs)
    }

    pub fn is_write_vectored(&self) -> bool {
        false
    }

    pub fn flush(&self) -> io::Result<()> {
        Ok(())
    }

    pub fn seek(&self, pos: SeekFrom) -> io::Result<u64> {
        let (off, whence) = match pos {
            SeekFrom::Start(n) => (n as i64, soso_rt::SEEK_SET),
            SeekFrom::Current(n) => (n, soso_rt::SEEK_CUR),
            SeekFrom::End(n) => (n, soso_rt::SEEK_END),
        };
        // SAFETY: el fd es de este fichero. `off as u64` conserva el bit de signo
        // y el kernel vuelve a leer el argumento como `i64`.
        let rc = unsafe { soso_rt::syscall3(soso_rt::SYS_SEEK, self.fd, off as u64, whence) };
        if rc < 0 {
            Err(io::Error::from_raw_os_error((-rc) as i32))
        } else {
            Ok(rc as u64)
        }
    }

    pub fn size(&self) -> Option<io::Result<u64>> {
        None
    }

    pub fn tell(&self) -> io::Result<u64> {
        self.seek(SeekFrom::Current(0))
    }

    pub fn duplicate(&self) -> io::Result<File> {
        unsupported()
    }

    pub fn set_permissions(&self, _perm: FilePermissions) -> io::Result<()> {
        unsupported()
    }

    pub fn set_times(&self, _times: FileTimes) -> io::Result<()> {
        unsupported()
    }
}

impl Drop for File {
    fn drop(&mut self) {
        unsafe {
            let _ = soso_rt::syscall1(soso_rt::SYS_CLOSE, self.fd);
        }
    }
}

impl DirBuilder {
    pub fn new() -> DirBuilder {
        DirBuilder {}
    }

    pub fn mkdir(&self, p: &Path) -> io::Result<()> {
        let bytes = p.as_os_str().as_bytes();
        if bytes.is_empty() {
            return Err(io::Error::from_raw_os_error(soso_rt::EINVAL as i32));
        }
        // SAFETY: `bytes` vive en esta llamada; el kernel copia el camino.
        let rc = unsafe {
            soso_rt::syscall3(
                soso_rt::SYS_MKDIR,
                bytes.as_ptr().expose_provenance() as u64,
                bytes.len() as u64,
                0,
            )
        };
        if rc < 0 {
            Err(io::Error::from_raw_os_error((-rc) as i32))
        } else {
            Ok(())
        }
    }
}

impl fmt::Debug for File {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("File").field("fd", &self.fd).finish()
    }
}

fn abrir(path: &Path) -> io::Result<u64> {
    let bytes = path.as_os_str().as_bytes();
    if bytes.is_empty() {
        return Err(io::Error::from_raw_os_error(soso_rt::EINVAL as i32));
    }
    // SAFETY: `bytes` vive en esta llamada; el kernel copia el camino.
    let fd = unsafe {
        soso_rt::syscall3(
            soso_rt::SYS_OPEN,
            bytes.as_ptr().expose_provenance() as u64,
            bytes.len() as u64,
            soso_rt::O_RDONLY,
        )
    };
    if fd < 0 {
        Err(io::Error::from_raw_os_error((-fd) as i32))
    } else {
        Ok(fd as u64)
    }
}

pub fn readdir(p: &Path) -> io::Result<ReadDir> {
    let fd = abrir(p)?;
    Ok(ReadDir {
        fd,
        path: p.to_path_buf(),
        buf: vec![0; soso_rt::DIRENT_SIZE * 16],
        pos: 0,
        filled: 0,
    })
}

pub fn unlink(p: &Path) -> io::Result<()> {
    let bytes = p.as_os_str().as_bytes();
    if bytes.is_empty() {
        return Err(io::Error::from_raw_os_error(soso_rt::EINVAL as i32));
    }
    // SAFETY: `bytes` vive en esta llamada; el kernel copia el camino.
    let rc = unsafe {
        soso_rt::syscall3(
            soso_rt::SYS_UNLINK,
            bytes.as_ptr().expose_provenance() as u64,
            bytes.len() as u64,
            0,
        )
    };
    if rc < 0 {
        Err(io::Error::from_raw_os_error((-rc) as i32))
    } else {
        Ok(())
    }
}

pub fn rename(_old: &Path, _new: &Path) -> io::Result<()> {
    unsupported()
}

pub fn set_perm(_p: &Path, _perm: FilePermissions) -> io::Result<()> {
    unsupported()
}

pub fn set_perm_nofollow(_p: &Path, _perm: FilePermissions) -> io::Result<()> {
    unsupported()
}

pub fn set_times(_p: &Path, _times: FileTimes) -> io::Result<()> {
    unsupported()
}

pub fn set_times_nofollow(_p: &Path, _times: FileTimes) -> io::Result<()> {
    unsupported()
}

pub fn rmdir(_p: &Path) -> io::Result<()> {
    unsupported()
}

pub fn remove_dir_all(_path: &Path) -> io::Result<()> {
    unsupported()
}

pub fn exists(path: &Path) -> io::Result<bool> {
    match stat(path) {
        Ok(_) => Ok(true),
        Err(e) if e.raw_os_error() == Some(soso_rt::ENOENT as i32) => Ok(false),
        Err(e) => Err(e),
    }
}

pub fn readlink(_p: &Path) -> io::Result<PathBuf> {
    unsupported()
}

pub fn symlink(_original: &Path, _link: &Path) -> io::Result<()> {
    unsupported()
}

pub fn link(_src: &Path, _dst: &Path) -> io::Result<()> {
    unsupported()
}

pub fn stat(p: &Path) -> io::Result<FileAttr> {
    let bytes = p.as_os_str().as_bytes();
    if bytes.is_empty() {
        return Err(io::Error::from_raw_os_error(soso_rt::EINVAL as i32));
    }
    let mut st = soso_rt::Stat::default();
    // SAFETY: `bytes` y `st` viven en esta llamada; el kernel copia el camino
    // y escribe un `Stat` en `st`.
    let rc = unsafe {
        soso_rt::syscall3(
            soso_rt::SYS_STAT,
            bytes.as_ptr().expose_provenance() as u64,
            bytes.len() as u64,
            (&mut st as *mut soso_rt::Stat).expose_provenance() as u64,
        )
    };
    if rc < 0 {
        return Err(io::Error::from_raw_os_error((-rc) as i32));
    }
    Ok(FileAttr { size: st.size, file_type: st.file_type })
}

pub fn lstat(p: &Path) -> io::Result<FileAttr> {
    stat(p)
}

pub fn canonicalize(_p: &Path) -> io::Result<PathBuf> {
    unsupported()
}

pub fn copy(from: &Path, to: &Path) -> io::Result<u64> {
    let mut lectura = OpenOptions::new();
    lectura.read(true);
    let src = File::open(from, &lectura)?;
    let mut escritura = OpenOptions::new();
    escritura.write(true);
    escritura.create(true);
    escritura.truncate(true);
    let dst = File::open(to, &escritura)?;
    let mut buf = vec![0u8; 64 * 1024];
    let mut total = 0u64;
    loop {
        let n = src.read(&mut buf)?;
        if n == 0 {
            break;
        }
        let mut off = 0;
        while off < n {
            let w = dst.write(&buf[off..n])?;
            if w == 0 {
                return Err(io::Error::new(io::ErrorKind::WriteZero, "escritura vacía"));
            }
            off += w;
        }
        total += n as u64;
    }
    Ok(total)
}
