#!/usr/bin/env python3
"""home 0.5.x: home_dir_inner only under cfg(unix)."""
from __future__ import annotations

import sys
from pathlib import Path

MARKER = "C-087 home soso"

UNIX_FN = """#[cfg(unix)]
fn home_dir_inner() -> Option<PathBuf> {
    #[allow(deprecated)]
    std::env::home_dir()
}"""

SOSO_FN = f"""#[cfg(unix)]
fn home_dir_inner() -> Option<PathBuf> {{
    #[allow(deprecated)]
    std::env::home_dir()
}}

// {MARKER}
#[cfg(target_os = "soso")]
fn home_dir_inner() -> Option<PathBuf> {{
    std::env::var_os("HOME").map(PathBuf::from)
}}"""


def patch(root: Path) -> bool:
    lib = root / "src" / "lib.rs"
    text = lib.read_text()
    if MARKER in text:
        return False
    if UNIX_FN not in text:
        raise SystemExit(f"{lib}: unix home_dir_inner anchor missing")
    lib.write_text(text.replace(UNIX_FN, SOSO_FN, 1))
    return True


def main() -> None:
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {sys.argv[0]} <home-crate-dir>")
    root = Path(sys.argv[1])
    if patch(root):
        print(f"patched {root}")


if __name__ == "__main__":
    main()
