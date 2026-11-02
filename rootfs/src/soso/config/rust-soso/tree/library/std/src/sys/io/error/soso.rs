//! Errores de E/S en soso.
//!
//! soso **no tiene un `errno` global**: cada syscall devuelve `-errno` en `rax`
//! y quien la llama lo traduce ahí mismo. Por eso `errno()` devuelve 0 y no
//! finge un estado que no existe; los códigos llegan por
//! `io::Error::from_raw_os_error`, que es la vía que sí usa la PAL.
//!
//! El resto sí es real: los números salen de `soso_abi`, no de una tabla
//! copiada de Linux. Coinciden con los de Linux en casi todo porque el ABI de
//! soso los tomó de ahí, pero la fuente es `crates/soso-abi/src/lib.rs`.

use crate::fmt;
use crate::io::ErrorKind;

/// soso no tiene `errno` global; ver el comentario del módulo.
pub fn errno() -> i32 {
    0
}

pub fn is_interrupted(code: i32) -> bool {
    code as i64 == soso_rt::EINTR
}

pub fn decode_error_kind(code: i32) -> ErrorKind {
    use soso_rt as abi;
    match code as i64 {
        abi::ENOENT => ErrorKind::NotFound,
        abi::EINTR => ErrorKind::Interrupted,
        abi::EIO => ErrorKind::Other,
        abi::EBADF => ErrorKind::InvalidInput,
        abi::ECHILD => ErrorKind::NotFound,
        abi::ENOMEM => ErrorKind::OutOfMemory,
        abi::EFAULT => ErrorKind::InvalidInput,
        abi::EEXIST => ErrorKind::AlreadyExists,
        abi::ENOTDIR => ErrorKind::NotADirectory,
        abi::EISDIR => ErrorKind::IsADirectory,
        abi::EINVAL => ErrorKind::InvalidInput,
        abi::EMFILE => ErrorKind::Other,
        abi::ENOSPC => ErrorKind::StorageFull,
        abi::ESPIPE => ErrorKind::NotSeekable,
        abi::ENAMETOOLONG => ErrorKind::InvalidFilename,
        abi::ENOSYS => ErrorKind::Unsupported,
        abi::ENOTEMPTY => ErrorKind::DirectoryNotEmpty,
        abi::EPIPE => ErrorKind::BrokenPipe,
        abi::EAGAIN => ErrorKind::WouldBlock,
        abi::EBUSY => ErrorKind::ResourceBusy,
        abi::EROFS => ErrorKind::ReadOnlyFilesystem,
        abi::ENOTSUP => ErrorKind::Unsupported,
        abi::ECONNREFUSED => ErrorKind::ConnectionRefused,
        abi::EADDRINUSE => ErrorKind::AddrInUse,
        abi::ENOTCONN => ErrorKind::NotConnected,
        abi::ETIMEDOUT => ErrorKind::TimedOut,
        abi::ESRCH => ErrorKind::NotFound,
        _ => ErrorKind::Uncategorized,
    }
}

pub fn format_error(errno: i32, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    use soso_rt as abi;
    let nombre = match errno as i64 {
        abi::ENOENT => "no such file or directory",
        abi::ESRCH => "no such process",
        abi::EINTR => "interrupted system call",
        abi::EIO => "input/output error",
        abi::EBADF => "bad file descriptor",
        abi::ECHILD => "no child processes",
        abi::EAGAIN => "resource temporarily unavailable",
        abi::ENOMEM => "cannot allocate memory",
        abi::EFAULT => "bad address",
        abi::EBUSY => "device or resource busy",
        abi::EEXIST => "file exists",
        abi::ENOTDIR => "not a directory",
        abi::EISDIR => "is a directory",
        abi::EINVAL => "invalid argument",
        abi::EMFILE => "too many open files",
        abi::ENOSPC => "no space left on device",
        abi::ESPIPE => "illegal seek",
        abi::EROFS => "read-only file system",
        abi::EPIPE => "broken pipe",
        abi::ENAMETOOLONG => "file name too long",
        abi::ENOSYS => "function not implemented",
        abi::ENOTEMPTY => "directory not empty",
        abi::ENOTSUP => "operation not supported",
        abi::ECONNREFUSED => "connection refused",
        abi::EADDRINUSE => "address already in use",
        abi::ENOTCONN => "transport endpoint is not connected",
        abi::ETIMEDOUT => "connection timed out",
        _ => return write!(f, "unknown soso error (errno {errno})"),
    };
    f.write_str(nombre)
}
