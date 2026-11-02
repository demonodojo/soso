#!/usr/bin/env python3
"""C-089: cargo bin main.rs is_executable only under cfg(unix)."""
from __future__ import annotations

import sys
from pathlib import Path

MARKER = "C-089 soso"

UNIX_FN = """#[cfg(unix)]
fn is_executable<P: AsRef<Path>>(path: P) -> bool {
    use std::os::unix::prelude::*;
    fs::metadata(path)
        .map(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}"""

UNIX_FN_NEW = """#[cfg(all(unix, not(target_os = "soso")))]
fn is_executable<P: AsRef<Path>>(path: P) -> bool {
    use std::os::unix::prelude::*;
    fs::metadata(path)
        .map(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}"""

SOSO_FN = f"""
// {MARKER}: sin PermissionsExt; mismo criterio que windows
#[cfg(target_os = "soso")]
fn is_executable<P: AsRef<Path>>(path: P) -> bool {{
    path.as_ref().is_file()
}}
"""

WINDOWS_FN = """#[cfg(windows)]
fn is_executable<P: AsRef<Path>>(path: P) -> bool {
    path.as_ref().is_file()
}"""


def patch(main_rs: Path) -> bool:
    text = main_rs.read_text()
    if MARKER in text:
        return False
    if UNIX_FN not in text:
        raise SystemExit(f"{main_rs}: unix is_executable anchor missing")
    if WINDOWS_FN not in text:
        raise SystemExit(f"{main_rs}: windows is_executable anchor missing")
    text = text.replace(UNIX_FN, UNIX_FN_NEW, 1)
    text = text.replace(WINDOWS_FN, SOSO_FN + "\n" + WINDOWS_FN, 1)
    main_rs.write_text(text)
    return True


def main() -> None:
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {sys.argv[0]} <cargo-workspace-root>")
    root = Path(sys.argv[1])
    main_rs = root / "src" / "bin" / "cargo" / "main.rs"
    if patch(main_rs):
        print(f"patched {main_rs}")


if __name__ == "__main__":
    main()
