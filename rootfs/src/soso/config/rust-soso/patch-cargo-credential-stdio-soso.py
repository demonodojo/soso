#!/usr/bin/env python3
"""C-087a: cargo-credential stdio.rs mod imp only under cfg(unix)."""
from __future__ import annotations

import sys
from pathlib import Path

MARKER = "C-087a soso"

IMP_UNIX_OLD = """#[cfg(unix)]
mod imp {
    use super::Stdio;
    use libc::{STDIN_FILENO, STDOUT_FILENO, close, dup, dup2};
    use std::{fs::File, io::Error, os::fd::AsRawFd};
    pub const IN_DEVICE: &str = "/dev/tty";"""

IMP_UNIX_NEW = """#[cfg(all(unix, not(target_os = "soso")))]
mod imp {
    use super::Stdio;
    use libc::{STDIN_FILENO, STDOUT_FILENO, close, dup, dup2};
    use std::{fs::File, io::Error, os::fd::AsRawFd};
    pub const IN_DEVICE: &str = "/dev/tty";"""

IMP_SOSO = f"""
// {MARKER}: dup/dup2 como unix; sin /dev/tty fiable en soso
#[cfg(target_os = "soso")]
mod imp {{
    use super::Stdio;
    use libc::{{c_int, close, dup, dup2}};
    use std::{{fs::File, io::Error}};
    const STDIN_FILENO: c_int = 0;
    const STDOUT_FILENO: c_int = 1;
    pub const IN_DEVICE: &str = "/dev/null";
    pub const OUT_DEVICE: &str = "/dev/null";
    pub const NULL_DEVICE: &str = "/dev/null";

    pub struct ReplacementGuard {{
        std_fileno: i32,
        previous: i32,
    }}

    impl ReplacementGuard {{
        pub(super) fn new(stdio: Stdio, replacement: &mut File) -> Result<ReplacementGuard, Error> {{
            let std_fileno = match stdio {{
                Stdio::Stdin => STDIN_FILENO,
                Stdio::Stdout => STDOUT_FILENO,
            }};

            let previous;
            unsafe {{
                previous = dup(std_fileno);
                if previous == -1 {{
                    return Err(std::io::Error::last_os_error());
                }}
                if dup2(replacement.as_raw_fd_soso() as c_int, std_fileno) == -1 {{
                    return Err(std::io::Error::last_os_error());
                }}
            }}

            Ok(ReplacementGuard {{
                previous,
                std_fileno,
            }})
        }}
    }}

    impl Drop for ReplacementGuard {{
        fn drop(&mut self) {{
            unsafe {{
                dup2(self.previous, self.std_fileno);
                close(self.previous);
            }}
        }}
    }}
}}
"""

WINDOWS_ANCHOR = """#[cfg(windows)]
mod imp {
    use super::Stdio;
    use std::{fs::File, io::Error, os::windows::prelude::AsRawHandle};"""


LIBC_DEP_OLD = "[target.'cfg(unix)'.dependencies]\nlibc.workspace = true"
LIBC_DEP_NEW = "[target.'cfg(any(unix, target_os = \"soso\"))'.dependencies]\nlibc.workspace = true"


def patch(root: Path) -> bool:
    changed = False
    toml = root / "Cargo.toml"
    if toml.is_file():
        text = toml.read_text()
        if LIBC_DEP_OLD in text and LIBC_DEP_NEW not in text:
            toml.write_text(text.replace(LIBC_DEP_OLD, LIBC_DEP_NEW, 1))
            changed = True

    stdio = root / "src" / "stdio.rs"
    text = stdio.read_text()
    if MARKER in text:
        fixed = False
        bad_use = "use std::{fs::File, io::Error, os::fd::AsRawFd};"
        good_use = "use std::{fs::File, io::Error};"
        if bad_use in text:
            text = text.replace(bad_use, good_use, 1)
            fixed = True
        legacy_libc = "use libc::{STDIN_FILENO, STDOUT_FILENO, c_int, close, dup, dup2};"
        fixed_libc = "use libc::{c_int, close, dup, dup2};\n    const STDIN_FILENO: c_int = 0;\n    const STDOUT_FILENO: c_int = 1;"
        if legacy_libc in text:
            text = text.replace(legacy_libc, fixed_libc, 1)
            fixed = True
        if "replacement.as_raw_fd()" in text:
            text = text.replace(
                "replacement.as_raw_fd()",
                "replacement.as_raw_fd_soso() as c_int",
                1,
            )
            fixed = True
        if fixed:
            stdio.write_text(text)
        return changed or fixed
    if IMP_UNIX_OLD not in text:
        raise SystemExit(f"{stdio}: unix imp anchor missing")
    if WINDOWS_ANCHOR not in text:
        raise SystemExit(f"{stdio}: windows imp anchor missing")
    text = text.replace(IMP_UNIX_OLD, IMP_UNIX_NEW, 1)
    text = text.replace(WINDOWS_ANCHOR, IMP_SOSO + "\n" + WINDOWS_ANCHOR, 1)
    stdio.write_text(text)
    return True


def main() -> None:
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {sys.argv[0]} <cargo-credential-dir>")
    root = Path(sys.argv[1])
    if patch(root):
        print(f"patched {root}")


if __name__ == "__main__":
    main()
