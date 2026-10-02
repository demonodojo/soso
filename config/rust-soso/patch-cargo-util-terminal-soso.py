#!/usr/bin/env python3
"""C-081: cargo-util-terminal shell.rs mod imp only under cfg(unix)."""
from __future__ import annotations

import sys
from pathlib import Path

MARKER = "C-081 soso"

IMP_UNIX_OLD = """#[cfg(unix)]
mod imp {
    use super::{Shell, TtyWidth};
    use std::mem;

    pub fn stderr_width() -> TtyWidth {"""

IMP_UNIX_NEW = """#[cfg(all(unix, not(target_os = "soso")))]
mod imp {
    use super::{Shell, TtyWidth};
    use std::mem;

    pub fn stderr_width() -> TtyWidth {"""

IMP_SOSO = f"""
// {MARKER}: sin ioctl TIOCGWINSZ en soso
#[cfg(target_os = "soso")]
mod imp {{
    use super::{{Shell, TtyWidth}};
    use std::io::prelude::*;

    pub fn stderr_width() -> TtyWidth {{
        TtyWidth::NoTty
    }}

    pub fn err_erase_line(shell: &mut Shell) {{
        let _ = shell.output.stderr().write_all(b"\\x1B[K");
    }}
}}
"""

WINDOWS_ANCHOR = """#[cfg(windows)]
mod imp {
    use std::{cmp, mem, ptr};"""


def patch(root: Path) -> bool:
    shell = root / "src" / "shell.rs"
    text = shell.read_text()
    if MARKER in text:
        return False
    if IMP_UNIX_OLD not in text:
        raise SystemExit(f"{shell}: unix imp anchor missing")
    if WINDOWS_ANCHOR not in text:
        raise SystemExit(f"{shell}: windows imp anchor missing")
    text = text.replace(IMP_UNIX_OLD, IMP_UNIX_NEW, 1)
    text = text.replace(WINDOWS_ANCHOR, IMP_SOSO + "\n" + WINDOWS_ANCHOR, 1)
    shell.write_text(text)
    return True


def main() -> None:
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {sys.argv[0]} <cargo-util-terminal-dir>")
    root = Path(sys.argv[1])
    if patch(root):
        print(f"patched {root}")


if __name__ == "__main__":
    main()
