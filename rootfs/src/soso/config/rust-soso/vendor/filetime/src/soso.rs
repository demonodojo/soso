//! soso: metadatos vía `std::fs::Metadata` (`modified` / `accessed`), sin `libc`.

use crate::FileTime;
use std::fs::{self, Metadata};
use std::io;
use std::path::Path;

fn from_system_time_result(t: io::Result<std::time::SystemTime>) -> FileTime {
    match t {
        Ok(st) => FileTime::from_system_time(st),
        Err(_) => FileTime::zero(),
    }
}

pub fn from_last_modification_time(meta: &Metadata) -> FileTime {
    from_system_time_result(meta.modified())
}

pub fn from_last_access_time(meta: &Metadata) -> FileTime {
    from_system_time_result(meta.accessed())
}

pub fn from_creation_time(_meta: &Metadata) -> Option<FileTime> {
    None
}

pub fn open(path: &Path) -> io::Result<fs::File> {
    fs::File::open(path).or_else(|_| fs::OpenOptions::new().write(true).open(path))
}

pub fn set_symlink_file_times(p: &Path, atime: FileTime, mtime: FileTime) -> io::Result<()> {
    open(p)?.set_times(
        fs::FileTimes::new()
            .set_accessed(atime.into())
            .set_modified(mtime.into()),
    )
}
