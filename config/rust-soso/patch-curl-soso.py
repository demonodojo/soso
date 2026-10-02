#!/usr/bin/env python3
"""C-075: curl treats paths/sockets as unix-only; soso is not cfg(unix)."""
from __future__ import annotations

import sys
from pathlib import Path

MARKER = "C-075 soso"
UNIX_CFG = "#[cfg(unix)]"
UNIX_ANY = '#[cfg(any(unix, target_os = "soso"))]'

OPEN_SOCKET_CVT_OLD = """        #[cfg(unix)]
        fn cvt(socket: Socket) -> curl_sys::curl_socket_t {
            use std::os::unix::prelude::*;
            socket.into_raw_fd()
        }"""

OPEN_SOCKET_CVT_NEW = f"""        // {MARKER}: soso has no std::os::unix; socket2 exposes inherent into_raw_fd
        #[cfg(any(unix, target_os = "soso"))]
        fn cvt(socket: Socket) -> curl_sys::curl_socket_t {{
            #[cfg(all(unix, not(target_os = "soso")))]
            {{
                use std::os::unix::prelude::*;
                socket.into_raw_fd()
            }}
            #[cfg(target_os = "soso")]
            {{
                socket.into_raw_fd()
            }}
        }}"""

PATH2CSTR_OLD = """    #[cfg(unix)]
    fn path2cstr(&mut self, p: &Path) -> Option<CString> {
        use std::os::unix::prelude::*;
        self.bytes2cstr(p.as_os_str().as_bytes())
    }"""

PATH2CSTR_NEW = f"""    #[cfg(any(unix, target_os = "soso"))]
    fn path2cstr(&mut self, p: &Path) -> Option<CString> {{
        #[cfg(all(unix, not(target_os = "soso")))]
        use std::os::unix::prelude::OsStrExt;
        #[cfg(target_os = "soso")]
        use std::os::soso::ffi::OsStrExt;
        self.bytes2cstr(p.as_os_str().as_bytes())
    }}"""

SETOPT_PATH_OLD = """    #[cfg(unix)]
    fn setopt_path(&mut self, opt: curl_sys::CURLoption, val: &Path) -> Result<(), Error> {
        use std::os::unix::prelude::*;
        let s = CString::new(val.as_os_str().as_bytes())?;
        self.setopt_str(opt, &s)
    }"""

SETOPT_PATH_NEW = f"""    #[cfg(any(unix, target_os = "soso"))]
    fn setopt_path(&mut self, opt: curl_sys::CURLoption, val: &Path) -> Result<(), Error> {{
        #[cfg(all(unix, not(target_os = "soso")))]
        use std::os::unix::prelude::OsStrExt;
        #[cfg(target_os = "soso")]
        use std::os::soso::ffi::OsStrExt;
        let s = CString::new(val.as_os_str().as_bytes())?;
        self.setopt_str(opt, &s)
    }}"""


def patch_tree(root: Path) -> bool:
    changed = False
    for path in sorted(root.joinpath("src").rglob("*.rs")):
        text = path.read_text()
        orig = text
        if path.name == "handler.rs" and OPEN_SOCKET_CVT_OLD in text:
            text = text.replace(OPEN_SOCKET_CVT_OLD, OPEN_SOCKET_CVT_NEW, 1)
        if path.name == "handler.rs" and SETOPT_PATH_OLD in text:
            text = text.replace(SETOPT_PATH_OLD, SETOPT_PATH_NEW, 1)
        if path.name == "form.rs" and PATH2CSTR_OLD in text:
            text = text.replace(PATH2CSTR_OLD, PATH2CSTR_NEW, 1)
        if UNIX_CFG in text:
            text = text.replace(UNIX_CFG, UNIX_ANY)
        if text != orig:
            path.write_text(text)
            changed = True
    return changed


def main() -> None:
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {sys.argv[0]} <curl-dir>")
    root = Path(sys.argv[1])
    if not (root / "src").is_dir():
        raise SystemExit(f"{root}: not a curl crate")
    if patch_tree(root):
        print(f"patched {root}")


if __name__ == "__main__":
    main()
