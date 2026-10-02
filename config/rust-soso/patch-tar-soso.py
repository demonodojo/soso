#!/usr/bin/env python3
"""C-087b: tar 0.4.x gates path/metadata helpers on cfg(unix)."""
from __future__ import annotations

import sys
from pathlib import Path

MARKER = "C-087b soso"
UNIX_NW = '#[cfg(all(unix, not(target_arch = "wasm32")))]'
UNIX_NW_NO_SOSO = '#[cfg(all(unix, not(target_arch = "wasm32"), not(target_os = "soso")))]'
UNIX_WIN_NW = '#[cfg(all(any(unix, windows), not(target_arch = "wasm32")))]'
UNIX_WIN_NW_SOSO = '#[cfg(all(any(unix, windows, target_os = "soso"), not(target_arch = "wasm32")))]'

HEADER_PRELUDE_OLD = """#[cfg(all(unix, not(target_arch = "wasm32")))]
use std::os::unix::prelude::*;
#[cfg(windows)]
use std::os::windows::prelude::*;"""

HEADER_PRELUDE_NEW = f"""{UNIX_NW_NO_SOSO}
use std::os::unix::prelude::*;
#[cfg(target_os = "soso")]
use std::os::soso::ffi::OsStrExt;
#[cfg(windows)]
use std::os::windows::prelude::*;"""

FILL_SOSO = f"""
    // {MARKER}: sin MetadataExt unix; aproximar como Windows
    #[cfg(target_os = "soso")]
    fn fill_platform_from(&mut self, meta: &fs::Metadata, mode: HeaderMode) {{
        match mode {{
            HeaderMode::Complete => {{
                self.set_uid(0);
                self.set_gid(0);
                let mtime = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs())
                    .unwrap_or(1);
                self.set_mtime(mtime);
                let fs_mode = if meta.is_dir() {{ 0o755 }} else {{ 0o644 }};
                self.set_mode(fs_mode);
            }}
            HeaderMode::Deterministic => {{
                self.set_uid(0);
                self.set_gid(0);
                self.set_mtime(DETERMINISTIC_TIMESTAMP);
                let fs_mode = if meta.is_dir() {{ 0o755 }} else {{ 0o644 }};
                self.set_mode(fs_mode);
            }}
        }}
        let ft = meta.file_type();
        self.set_entry_type(if ft.is_dir() {{
            EntryType::dir()
        }} else if ft.is_file() {{
            EntryType::file()
        }} else if ft.is_symlink() {{
            EntryType::symlink()
        }} else {{
            EntryType::new(b' ')
        }});
    }}

"""

FILL_UNIX_CFG_OLD = """    #[cfg(all(unix, not(target_arch = "wasm32")))]
    fn fill_platform_from(&mut self, meta: &fs::Metadata, mode: HeaderMode) {"""

FILL_UNIX_CFG_NEW = (
    FILL_SOSO
    + """    #[cfg(all(unix, not(target_arch = "wasm32"), not(target_os = "soso")))]
    fn fill_platform_from(&mut self, meta: &fs::Metadata, mode: HeaderMode) {"""
)

ENDS_UNIX = """#[cfg(all(unix, not(target_arch = "wasm32")))]
fn ends_with_slash(p: &Path) -> bool {
    p.as_os_str().as_bytes().ends_with(b"/")
}"""

ENDS_SOSO = f"""{UNIX_NW_NO_SOSO}
fn ends_with_slash(p: &Path) -> bool {{
    p.as_os_str().as_bytes().ends_with(b"/")
}}

#[cfg(target_os = "soso")]
fn ends_with_slash(p: &Path) -> bool {{
    p.as_os_str().as_bytes().ends_with(b"/")
}}
"""

PATH2BYTES_UNIX = """#[cfg(all(unix, not(target_arch = "wasm32")))]
/// On unix this will never fail
pub fn path2bytes(p: &Path) -> io::Result<Cow<'_, [u8]>> {
    Ok(Cow::Borrowed(p.as_os_str().as_bytes()))
}"""

PATH2BYTES_SOSO = f"""{UNIX_NW_NO_SOSO}
/// On unix this will never fail
pub fn path2bytes(p: &Path) -> io::Result<Cow<'_, [u8]>> {{
    Ok(Cow::Borrowed(p.as_os_str().as_bytes()))
}}

#[cfg(target_os = "soso")]
pub fn path2bytes(p: &Path) -> io::Result<Cow<'_, [u8]>> {{
    Ok(Cow::Borrowed(p.as_os_str().as_bytes()))
}}"""

BYTES2PATH_UNIX = """#[cfg(all(unix, not(target_arch = "wasm32")))]
/// On unix this operation can never fail.
pub fn bytes2path(bytes: Cow<[u8]>) -> io::Result<Cow<Path>> {
    use std::ffi::{OsStr, OsString};

    Ok(match bytes {
        Cow::Borrowed(bytes) => Cow::Borrowed(Path::new(OsStr::from_bytes(bytes))),
        Cow::Owned(bytes) => Cow::Owned(PathBuf::from(OsString::from_vec(bytes))),
    })
}"""

BYTES2PATH_SOSO = f"""{UNIX_NW_NO_SOSO}
/// On unix this operation can never fail.
pub fn bytes2path(bytes: Cow<[u8]>) -> io::Result<Cow<Path>> {{
    use std::ffi::{{OsStr, OsString}};

    Ok(match bytes {{
        Cow::Borrowed(bytes) => Cow::Borrowed(Path::new(OsStr::from_bytes(bytes))),
        Cow::Owned(bytes) => Cow::Owned(PathBuf::from(OsString::from_vec(bytes))),
    }})
}}

#[cfg(target_os = "soso")]
pub fn bytes2path(bytes: Cow<[u8]>) -> io::Result<Cow<Path>> {{
    use std::ffi::{{OsStr, OsString}};
    use std::os::soso::ffi::{{OsStrExt, OsStringExt}};

    Ok(match bytes {{
        Cow::Borrowed(bytes) => Cow::Borrowed(Path::new(OsStr::from_bytes(bytes))),
        Cow::Owned(bytes) => Cow::Owned(PathBuf::from(OsString::from_vec(bytes))),
    }})
}}"""

LIB_EXPORT_OLD = """#[cfg(all(any(unix, windows), not(target_arch = "wasm32")))]
pub use crate::header::DETERMINISTIC_TIMESTAMP;"""

LIB_EXPORT_NEW = """#[cfg(all(any(unix, windows, target_os = "soso"), not(target_arch = "wasm32")))]
pub use crate::header::DETERMINISTIC_TIMESTAMP;"""

ENTRY_SYMLINK_UNIX = """            #[cfg(all(unix, not(target_arch = "wasm32")))]
            fn symlink(src: &Path, dst: &Path) -> io::Result<()> {
                ::std::os::unix::fs::symlink(src, dst)
            }"""

ENTRY_SYMLINK_SOSO = f"""            {UNIX_NW_NO_SOSO}
            fn symlink(src: &Path, dst: &Path) -> io::Result<()> {{
                ::std::os::unix::fs::symlink(src, dst)
            }}
            #[cfg(target_os = "soso")]
            fn symlink(_src: &Path, _dst: &Path) -> io::Result<()> {{
                Err(io::Error::new(io::ErrorKind::Unsupported, "symlinks not supported"))
            }}"""

ENTRY_OWNERSHIP_STUB_OLD = """        #[cfg(any(windows, target_arch = "wasm32"))]
        fn _set_ownerships(
            _: &Path,
            _: &Option<&mut std::fs::File>,
            _: u64,
            _: u64,
        ) -> io::Result<()> {
            Ok(())
        }"""

ENTRY_OWNERSHIP_STUB_NEW = """        #[cfg(any(windows, target_arch = "wasm32", target_os = "soso"))]
        fn _set_ownerships(
            _: &Path,
            _: &Option<&mut std::fs::File>,
            _: u64,
            _: u64,
        ) -> io::Result<()> {
            Ok(())
        }"""

ENTRY_PERMS_WASM = """        #[cfg(target_arch = "wasm32")]
        #[allow(unused_variables)]
        fn _set_perms(
            dst: &Path,
            f: Option<&mut std::fs::File>,
            mode: u32,
            mask: u32,
            _preserve: bool,
        ) -> io::Result<()> {
            Err(io::Error::new(io::ErrorKind::Other, "Not implemented"))
        }"""

ENTRY_PERMS_SOSO = f"""
        // {MARKER}
        #[cfg(target_os = "soso")]
        #[allow(unused_variables)]
        fn _set_perms(
            _dst: &Path,
            _f: Option<&mut std::fs::File>,
            _mode: u32,
            _mask: u32,
            _preserve: bool,
        ) -> io::Result<()> {{
            Ok(())
        }}

"""


BYTES2PATH_SOSO_IMPORT_OLD = """#[cfg(target_os = "soso")]
pub fn bytes2path(bytes: Cow<[u8]>) -> io::Result<Cow<Path>> {
    use std::ffi::{OsStr, OsString};

    Ok(match bytes {"""

BYTES2PATH_SOSO_IMPORT_NEW = """#[cfg(target_os = "soso")]
pub fn bytes2path(bytes: Cow<[u8]>) -> io::Result<Cow<Path>> {
    use std::ffi::{OsStr, OsString};
    use std::os::soso::ffi::{OsStrExt, OsStringExt};

    Ok(match bytes {"""


def patch_header(header: Path) -> bool:
    text = header.read_text()
    if MARKER in text:
        if BYTES2PATH_SOSO_IMPORT_OLD in text:
            header.write_text(text.replace(BYTES2PATH_SOSO_IMPORT_OLD, BYTES2PATH_SOSO_IMPORT_NEW, 1))
            return True
        return False
    orig = text
    for old, new in (
        (HEADER_PRELUDE_OLD, HEADER_PRELUDE_NEW),
        (UNIX_WIN_NW, UNIX_WIN_NW_SOSO),
        (FILL_UNIX_CFG_OLD, FILL_UNIX_CFG_NEW),
        (ENDS_UNIX, ENDS_SOSO),
        (PATH2BYTES_UNIX, PATH2BYTES_SOSO),
        (BYTES2PATH_UNIX, BYTES2PATH_SOSO),
    ):
        if old not in text:
            raise SystemExit(f"{header}: anchor missing for patch")
        text = text.replace(old, new, 1)
    if text != orig:
        header.write_text(text)
        return True
    return False


def patch_lib(lib: Path) -> bool:
    text = lib.read_text()
    if LIB_EXPORT_OLD not in text:
        return False
    if MARKER in text:
        return False
    lib.write_text(text.replace(LIB_EXPORT_OLD, LIB_EXPORT_NEW, 1))
    return True


def patch_entry(entry: Path) -> bool:
    text = entry.read_text()
    if MARKER in text:
        return False
    orig = text
    if ENTRY_SYMLINK_UNIX in text:
        text = text.replace(ENTRY_SYMLINK_UNIX, ENTRY_SYMLINK_SOSO, 1)
    if ENTRY_OWNERSHIP_STUB_OLD in text:
        text = text.replace(ENTRY_OWNERSHIP_STUB_OLD, ENTRY_OWNERSHIP_STUB_NEW, 1)
    if ENTRY_PERMS_WASM in text:
        text = text.replace(ENTRY_PERMS_WASM, ENTRY_PERMS_SOSO + ENTRY_PERMS_WASM, 1)
    if text != orig:
        entry.write_text(text)
        return True
    return False


def patch(root: Path) -> bool:
    changed = False
    changed |= patch_header(root / "src" / "header.rs")
    changed |= patch_lib(root / "src" / "lib.rs")
    changed |= patch_entry(root / "src" / "entry.rs")
    return changed


def main() -> None:
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {sys.argv[0]} <tar-crate-dir>")
    root = Path(sys.argv[1])
    if patch(root):
        print(f"patched {root}")


if __name__ == "__main__":
    main()
