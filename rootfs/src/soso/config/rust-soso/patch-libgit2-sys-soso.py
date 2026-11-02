#!/usr/bin/env python3
"""C-077: libgit2-sys openssl_init is unix+https only; soso has https but not unix."""
from __future__ import annotations

import sys
from pathlib import Path

MARKER = "C-077 soso"

OPENSSL_INIT_OLD = """#[cfg(all(unix, feature = "https"))]
#[doc(hidden)]
pub fn openssl_init() {
    openssl_sys::init();
}

#[cfg(any(windows, not(feature = "https")))]
#[doc(hidden)]
pub fn openssl_init() {}"""

OPENSSL_INIT_NEW = f"""#[cfg(all(unix, not(target_os = "soso"), feature = "https"))]
#[doc(hidden)]
pub fn openssl_init() {{
    openssl_sys::init();
}}

// {MARKER}: sin openssl-sys en soso (T42)
#[cfg(any(windows, target_os = "soso", not(feature = "https")))]
#[doc(hidden)]
pub fn openssl_init() {{}}"""


def patch_lib_rs(path: Path) -> bool:
    text = path.read_text()
    if OPENSSL_INIT_OLD not in text:
        if MARKER in text:
            return False
        raise SystemExit(f"{path}: openssl_init needle missing")
    path.write_text(text.replace(OPENSSL_INIT_OLD, OPENSSL_INIT_NEW, 1))
    return True


def main() -> None:
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {sys.argv[0]} <libgit2-sys-dir>")
    root = Path(sys.argv[1])
    if patch_lib_rs(root / "lib.rs"):
        print(f"patched {root}")


if __name__ == "__main__":
    main()
