#!/usr/bin/env python3
"""C-079: git2 gates path/openssl helpers on cfg(unix); soso is not unix."""
from __future__ import annotations

import sys
from pathlib import Path

MARKER = "C-079 soso"

OPENSSL_STUB_OLD = """#[cfg(any(
    windows,
    target_os = "macos",
    target_os = "ios",
    not(feature = "https")
))]
fn openssl_env_init() {}"""

OPENSSL_STUB_NEW = """#[cfg(any(
    windows,
    target_os = "macos",
    target_os = "ios",
    target_os = "soso",
    not(feature = "https")
))]
fn openssl_env_init() {}"""

BYTES2PATH_UNIX = """#[cfg(unix)]
pub fn bytes2path(b: &[u8]) -> &Path {
    use std::os::unix::prelude::*;
    Path::new(OsStr::from_bytes(b))
}"""

BYTES2PATH_SOSO = f"""#[cfg(unix)]
pub fn bytes2path(b: &[u8]) -> &Path {{
    use std::os::unix::prelude::*;
    Path::new(OsStr::from_bytes(b))
}}
// {MARKER}
#[cfg(target_os = "soso")]
pub fn bytes2path(b: &[u8]) -> &Path {{
    use std::os::soso::ffi::OsStrExt;
    Path::new(OsStr::from_bytes(b))
}}"""

OSSTRING_UNIX = """impl IntoCString for OsString {
    #[cfg(unix)]
    fn into_c_string(self) -> Result<CString, Error> {
        use std::os::unix::prelude::*;
        let s: &OsStr = self.as_ref();
        Ok(CString::new(s.as_bytes())?)
    }"""

OSSTRING_SOSO = f"""impl IntoCString for OsString {{
    #[cfg(all(unix, not(target_os = "soso")))]
    fn into_c_string(self) -> Result<CString, Error> {{
        use std::os::unix::prelude::*;
        let s: &OsStr = self.as_ref();
        Ok(CString::new(s.as_bytes())?)
    }}
    #[cfg(target_os = "soso")]
    fn into_c_string(self) -> Result<CString, Error> {{
        use std::os::soso::ffi::OsStrExt;
        let s: &OsStr = self.as_ref();
        Ok(CString::new(s.as_bytes())?)
    }}"""


def patch(root: Path) -> bool:
    changed = False
    lib = root / "src" / "lib.rs"
    text = lib.read_text()
    if OPENSSL_STUB_OLD in text:
        lib.write_text(text.replace(OPENSSL_STUB_OLD, OPENSSL_STUB_NEW, 1))
        changed = True

    util = root / "src" / "util.rs"
    text = util.read_text()
    orig = text
    if BYTES2PATH_UNIX in text and MARKER not in text:
        text = text.replace(BYTES2PATH_UNIX, BYTES2PATH_SOSO, 1)
    if OSSTRING_UNIX in text and "target_os = \"soso\"" not in text.split("impl IntoCString for OsString", 1)[-1][:400]:
        text = text.replace(OSSTRING_UNIX, OSSTRING_SOSO, 1)
    if text != orig:
        util.write_text(text)
        changed = True
    return changed


def main() -> None:
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {sys.argv[0]} <git2-dir>")
    root = Path(sys.argv[1])
    if patch(root):
        print(f"patched {root}")


if __name__ == "__main__":
    main()
