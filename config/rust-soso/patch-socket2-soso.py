#!/usr/bin/env python3
"""C-074: socket2 only enables sys/unix for cfg(unix); soso is not unix."""
from __future__ import annotations

import re
import shutil
import sys
from pathlib import Path

MARKER = "C-074 soso"
WASI_P1 = 'all(target_os = "wasi", not(target_env = "p1"))'
OLD = f"any(unix, {WASI_P1})"
NEW = f'any(unix, target_os = "soso", {WASI_P1})'
COMPILE_OLD = f'#[cfg(not(any(windows, unix, {WASI_P1}))))]'
COMPILE_NEW = f'#[cfg(not(any(windows, unix, target_os = "soso", {WASI_P1}))))]'
# Bootstrap stage2-tools: libc del sysroot exige esta feature (ver build-cargo-20).
RUSTC_PRIVATE = '#![feature(rustc_private)]'
FROM_LINES = (
    "from!(net::TcpStream, Socket);",
    "from!(net::TcpListener, Socket);",
    "from!(net::UdpSocket, Socket);",
    "from!(Socket, net::TcpStream);",
    "from!(Socket, net::TcpListener);",
    "from!(Socket, net::UdpSocket);",
)
FROM_BLOCK = "\n".join(FROM_LINES)


def from_guard_block() -> str:
    lines = "\n".join(f'#[cfg(not(target_os = "soso"))]\n{line}' for line in FROM_LINES)
    return f"// {MARKER}: std::net sin FromRawFd en soso todavía\n{lines}"


SOCKET_LIBC_ANCHOR = "use std::time::Duration;\n\nuse crate::sys"
SOCKET_LIBC_EXTERN = """use std::time::Duration;

#[cfg(any(unix, target_os = "soso", all(target_os = "wasi", not(target_env = "p1"))))]
extern crate libc;

use crate::sys"""

OSSTRAX_NEEDLE = """#[cfg(not(target_os = "wasi"))]
use std::os::unix::ffi::OsStrExt;"""
OSSTRAX_REPL = """#[cfg(all(not(target_os = "wasi"), not(target_os = "soso")))]
use std::os::unix::ffi::OsStrExt;
#[cfg(target_os = "soso")]
use std::os::soso::ffi::OsStrExt;"""

FD_USE = "use std::os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, IntoRawFd, OwnedFd, RawFd};"
FD_USE_REPL = """#[cfg(not(target_os = "soso"))]
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, IntoRawFd, OwnedFd, RawFd};
#[cfg(target_os = "soso")]
use self::soso_fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, IntoRawFd, OwnedFd, RawFd};"""

SOSO_FD_MOD = """#[cfg(target_os = "soso")]
pub(crate) mod soso_fd;

"""

SOCKET_FD_BLOCK = """#[cfg(any(unix, all(target_os = "wasi", not(target_env = "p1"))))]
use std::os::fd::{FromRawFd, IntoRawFd};
#[cfg(target_os = "soso")]
use crate::sys::soso_fd::{FromRawFd, IntoRawFd};"""

SOCKREF_FD_BLOCK = """#[cfg(any(unix, all(target_os = "wasi", not(target_env = "p1"))))]
use std::os::fd::{AsFd, AsRawFd, FromRawFd};
#[cfg(target_os = "soso")]
use crate::sys::soso_fd::{AsFd, AsRawFd, FromRawFd};"""

REEXPORT_BLOCK = """// C-074 soso: reexport fd shim for socket.rs / sockref.rs
#[cfg(target_os = "soso")]
pub(crate) use soso_fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, IntoRawFd, OwnedFd, RawFd};
"""

LIBC_DEP_OLD = '[target.\'cfg(any(unix, target_os = "wasi"))\'.dependencies.libc]'
LIBC_DEP_NEW = '[target.\'cfg(any(unix, target_os = "wasi", target_os = "soso"))\'.dependencies.libc]'

UNIX_ALL_FEATURE = '#[cfg(all(feature = "all", unix))]'
UNIX_ALL_NO_SOSO = '#[cfg(all(feature = "all", unix, not(target_os = "soso")))]'

SOCKET_TYPE_OLD = "pub(crate) type Socket = std::os::fd::OwnedFd;"
SOCKET_TYPE_NEW = """#[cfg(not(target_os = "soso"))]
pub(crate) type Socket = std::os::fd::OwnedFd;
#[cfg(target_os = "soso")]
pub(crate) type Socket = self::soso_fd::OwnedFd;"""

IOVLEN_USIZE = """    target_os = "android",
))]
type IovLen = usize;"""
IOVLEN_USIZE_SOSO = """    target_os = "android",
    target_os = "soso",
))]
type IovLen = usize;"""

AS_UNIX_FN = """    pub fn as_unix(&self) -> Option<std::os::unix::net::SocketAddr> {
        let path = self.as_pathname()?;
        // SAFETY: we can represent this as a valid pathname, then so can the
        // standard library.
        Some(std::os::unix::net::SocketAddr::from_pathname(path).unwrap())
    }"""
AS_UNIX_FN_SOSO = """    #[cfg(not(target_os = "soso"))]
    pub fn as_unix(&self) -> Option<std::os::unix::net::SocketAddr> {
        let path = self.as_pathname()?;
        // SAFETY: we can represent this as a valid pathname, then so can the
        // standard library.
        Some(std::os::unix::net::SocketAddr::from_pathname(path).unwrap())
    }"""

INTO_RAW_FD_SOSO = """    // C-075: curl `open_socket` without std::os::unix on soso
    #[cfg(target_os = "soso")]
    pub fn into_raw_fd(self) -> c_int {
        self.into_raw()
    }
"""
INTO_RAW_FD_ANCHOR = """    pub(crate) fn into_raw(self) -> sys::RawSocket {
        sys::socket_into_raw(self.inner)
    }

    /// Creates a new socket and sets common flags."""


def strip_fd_trait_imports(text: str) -> str:
    return re.sub(
        r"(?:#\[cfg\([^\]]*\)\]\n)*"
        r"use (?:std::os::fd|self::soso_fd)::\{[^}]+\};\n",
        "",
        text,
    )


def normalize_unix_fd_imports(text: str) -> str:
    text = strip_fd_trait_imports(text)
    if FD_USE in text:
        text = text.replace(FD_USE + "\n", "")
    anchor = "#[cfg(all(not(target_os = \"wasi\"), not(target_os = \"soso\")))]\nuse std::os::unix::ffi::OsStrExt;"
    if FD_USE_REPL not in text:
        if anchor in text:
            text = text.replace(anchor, FD_USE_REPL + "\n" + anchor, 1)
        else:
            text = text.replace(
                "use std::num::NonZeroUsize;\n",
                "use std::num::NonZeroUsize;\n" + FD_USE_REPL + "\n",
                1,
            )
    return text


def fix_socket_fd_imports(text: str) -> str:
    text = re.sub(
        r"(?:#\[cfg\(target_os = \"soso\"\)\]\n)?"
        r"use crate::sys::soso_fd::\{[^}]+\};\n",
        "",
        text,
    )
    text = re.sub(
        r"#\[cfg\(any\(unix,[^\n]+\)\]\nuse std::os::fd::\{[^}]+\};\n",
        "",
        text,
    )
    needle = "#[cfg(windows)]"
    if needle in text and SOCKET_FD_BLOCK not in text:
        text = text.replace(needle, SOCKET_FD_BLOCK + "\n" + needle, 1)
    return text


def fix_sockref_fd_imports(text: str) -> str:
    text = re.sub(
        r"(?:#\[cfg\(target_os = \"soso\"\)\]\n)?"
        r"use crate::sys::soso_fd::\{[^}]+\};\n",
        "",
        text,
    )
    text = re.sub(
        r"#\[cfg\(any\(unix,[^\n]+\)\]\nuse std::os::fd::\{[^}]+\};\n",
        "",
        text,
    )
    needle = "#[cfg(windows)]"
    if needle in text and SOCKREF_FD_BLOCK not in text:
        text = text.replace(needle, SOCKREF_FD_BLOCK + "\n" + needle, 1)
    return text


def patch_cargo(root: Path) -> bool:
    for name in ("Cargo.toml", "Cargo.toml.orig"):
        cargo = root / name
        if not cargo.is_file():
            continue
        text = cargo.read_text()
        if LIBC_DEP_NEW in text:
            continue
        if LIBC_DEP_OLD not in text:
            continue
        cargo.write_text(text.replace(LIBC_DEP_OLD, LIBC_DEP_NEW, 1))
        return True
    return False


def patch_file(path: Path) -> bool:
    text = path.read_text()
    orig = text
    if OLD in text:
        text = text.replace(OLD, NEW)
    if path.name == "lib.rs":
        if COMPILE_OLD in text:
            text = text.replace(COMPILE_OLD, COMPILE_NEW, 1)
        text = re.sub(r"#!\[cfg_attr\(target_os = \"soso\", feature\(rustc_private\)\)\]\n", "", text)
        if RUSTC_PRIVATE not in text and "#![allow(clippy::needless_lifetimes)]" in text:
            text = text.replace(
                "#![allow(clippy::needless_lifetimes)]\n",
                "#![allow(clippy::needless_lifetimes)]\n" + RUSTC_PRIVATE + "\n",
                1,
            )
    if path.name == "socket.rs":
        guard = from_guard_block()
        if guard not in text:
            if FROM_BLOCK in text:
                text = text.replace(FROM_BLOCK, guard, 1)
            elif MARKER in text:
                text = re.sub(
                    rf"// {re.escape(MARKER)}:[^\n]*\n"
                    rf"(?:#\[cfg\(not\(target_os = \"soso\"\)\)\]\n)?"
                    rf"(?:from!\([^)]+\);\n)+",
                    guard + "\n",
                    text,
                    count=1,
                )
        if "extern crate libc;" not in text and SOCKET_LIBC_ANCHOR in text:
            text = text.replace(SOCKET_LIBC_ANCHOR, SOCKET_LIBC_EXTERN, 1)
        if MARKER in text:
            text = re.sub(
                rf"(?:// {re.escape(MARKER)}:[^\n]*\n)+",
                f"// {MARKER}: std::net sin FromRawFd en soso todavía\n",
                text,
                count=1,
            )
        text = fix_socket_fd_imports(text)
        if INTO_RAW_FD_ANCHOR in text and "C-075: curl" not in text:
            text = text.replace(
                INTO_RAW_FD_ANCHOR,
                INTO_RAW_FD_ANCHOR.replace(
                    "\n\n    /// Creates a new socket",
                    "\n" + INTO_RAW_FD_SOSO + "\n    /// Creates a new socket",
                ),
                1,
            )
    if path.name == "sockref.rs":
        text = fix_sockref_fd_imports(text)
    if path.name == "unix.rs":
        if "extern crate libc;" not in text:
            text = text.replace("use std::cmp::min;\n", "extern crate libc;\n\nuse std::cmp::min;\n", 1)
        text = normalize_unix_fd_imports(text)
        if OSSTRAX_NEEDLE in text:
            text = text.replace(OSSTRAX_NEEDLE, OSSTRAX_REPL, 1)
        if REEXPORT_BLOCK.strip() in text:
            text = text.replace(REEXPORT_BLOCK, "", 1)
        if SOCKET_TYPE_OLD in text and SOCKET_TYPE_NEW not in text:
            text = text.replace(SOCKET_TYPE_OLD, SOCKET_TYPE_NEW, 1)
        if IOVLEN_USIZE in text and "target_os = \"soso\"," not in text.split("type IovLen = usize", 1)[0][-200:]:
            text = text.replace(IOVLEN_USIZE, IOVLEN_USIZE_SOSO, 1)
        if AS_UNIX_FN in text and AS_UNIX_FN_SOSO not in text:
            text = text.replace(AS_UNIX_FN, AS_UNIX_FN_SOSO, 1)
        if "pub(crate) mod soso_fd" not in text:
            if "mod soso_fd" in text:
                text = text.replace("mod soso_fd;", "pub(crate) mod soso_fd;", 1)
            else:
                text = text.replace("use std::cmp::min;\n", "use std::cmp::min;\n" + SOSO_FD_MOD, 1)
        text = text.replace("super::soso_fd", "self::soso_fd")
        if UNIX_ALL_FEATURE in text and UNIX_ALL_NO_SOSO not in text:
            text = text.replace(UNIX_ALL_FEATURE, UNIX_ALL_NO_SOSO)
    if text != orig:
        path.write_text(text)
        return True
    return False


def main() -> None:
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {sys.argv[0]} <socket2-dir>")
    root = Path(sys.argv[1])
    vendor_fd = Path(__file__).resolve().parent / "vendor" / "socket2-soso-fd.rs"
    dst_fd = root / "src" / "sys" / "soso_fd.rs"
    dst_fd.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy(vendor_fd, dst_fd)
    any_changed = patch_cargo(root)
    for path in sorted(root.joinpath("src").rglob("*.rs")):
        any_changed |= patch_file(path)
    if any_changed:
        print(f"patched {root}")


if __name__ == "__main__":
    main()
