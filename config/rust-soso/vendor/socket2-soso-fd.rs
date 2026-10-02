//! Shim mínimo de `std::os::fd` para compilar socket2 en soso (C-074).
use std::fmt;
use std::marker::PhantomData;
use std::mem::ManuallyDrop;

pub type RawFd = i32;

pub struct BorrowedFd<'fd> {
    fd: RawFd,
    _phantom: PhantomData<&'fd ()>,
}

impl BorrowedFd<'_> {
    pub fn as_raw_fd(&self) -> RawFd {
        self.fd
    }
}

pub struct OwnedFd {
    fd: RawFd,
}

impl OwnedFd {
    pub fn into_raw_fd(self) -> RawFd {
        let fd = self.fd;
        ManuallyDrop::new(self);
        fd
    }
}

impl fmt::Debug for OwnedFd {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OwnedFd").field("fd", &self.fd).finish()
    }
}

impl AsRawFd for OwnedFd {
    fn as_raw_fd(&self) -> RawFd {
        self.fd
    }
}

impl IntoRawFd for OwnedFd {
    fn into_raw_fd(self) -> RawFd {
        OwnedFd::into_raw_fd(self)
    }
}

impl FromRawFd for OwnedFd {
    unsafe fn from_raw_fd(fd: RawFd) -> Self {
        Self { fd }
    }
}

impl AsFd for OwnedFd {
    fn as_fd(&self) -> BorrowedFd<'_> {
        unsafe { BorrowedFd::borrow_raw(self.fd) }
    }
}

pub trait AsRawFd {
    fn as_raw_fd(&self) -> RawFd;
}

pub trait IntoRawFd {
    fn into_raw_fd(self) -> RawFd;
}

pub trait FromRawFd: Sized {
    unsafe fn from_raw_fd(fd: RawFd) -> Self;
}

pub trait AsFd {
    fn as_fd(&self) -> BorrowedFd<'_>;
}

impl BorrowedFd<'_> {
    pub unsafe fn borrow_raw(fd: RawFd) -> BorrowedFd<'static> {
        BorrowedFd {
            fd,
            _phantom: PhantomData,
        }
    }
}
