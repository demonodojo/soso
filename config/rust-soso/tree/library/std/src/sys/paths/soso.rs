//! Rutas de soso: directorio de trabajo y `PATH` separado por `:`.

use crate::ffi::{OsStr, OsString};
use crate::fmt;
use crate::io;
use crate::os::soso::ffi::{OsStrExt, OsStringExt};
use crate::path::PathBuf;

const PATH_SEPARATOR: u8 = b':';

pub fn getcwd() -> io::Result<PathBuf> {
    let mut buf = [0u8; 512];
    let rc = unsafe {
        soso_rt::syscall3(
            soso_rt::SYS_GETCWD,
            buf.as_mut_ptr().expose_provenance() as u64,
            buf.len() as u64,
            0,
        )
    };
    if rc < 0 {
        return Err(io::Error::from_raw_os_error((-rc) as i32));
    }
    let end = buf.iter().position(|b| *b == 0).unwrap_or(buf.len());
    Ok(PathBuf::from(OsString::from_vec(buf[..end].to_vec())))
}

pub struct SplitPaths<'a> {
    rest: &'a [u8],
    done: bool,
}

pub fn split_paths(unparsed: &OsStr) -> SplitPaths<'_> {
    SplitPaths { rest: unparsed.as_bytes(), done: false }
}

impl<'a> Iterator for SplitPaths<'a> {
    type Item = PathBuf;

    fn next(&mut self) -> Option<PathBuf> {
        if self.done {
            return None;
        }
        let (part, rest, done) = match self.rest.iter().position(|b| *b == b':') {
            Some(i) => (&self.rest[..i], &self.rest[i + 1..], false),
            None => {
                let part = self.rest;
                (part, &part[part.len()..], true)
            }
        };
        self.rest = rest;
        self.done = done;
        Some(PathBuf::from(OsStr::from_bytes(part)))
    }
}

#[derive(Debug)]
pub struct JoinPathsError;

pub fn join_paths<I, T>(paths: I) -> Result<OsString, JoinPathsError>
where
    I: Iterator<Item = T>,
    T: AsRef<OsStr>,
{
    let mut joined = Vec::new();
    for (i, path) in paths.enumerate() {
        let path = path.as_ref().as_bytes();
        if i > 0 {
            joined.push(PATH_SEPARATOR);
        }
        if path.contains(&PATH_SEPARATOR) {
            return Err(JoinPathsError);
        }
        joined.extend_from_slice(path);
    }
    Ok(OsString::from_vec(joined))
}

impl fmt::Display for JoinPathsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "path segment contains separator `:`")
    }
}

impl crate::error::Error for JoinPathsError {}
