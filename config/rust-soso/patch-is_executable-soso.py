#!/usr/bin/env python3
"""C-080: is_executable only implements IsExecutable under cfg(unix)."""
from __future__ import annotations

import sys
from pathlib import Path

MARKER = "C-080 soso"
SOSO_MOD = f"""
// {MARKER}: sin PermissionsExt en soso; basta con fichero regular
#[cfg(target_os = "soso")]
mod soso {{
    use std::path::Path;

    use super::IsExecutable;

    impl IsExecutable for Path {{
        fn is_executable(&self) -> bool {{
            self.metadata().map(|m| m.is_file()).unwrap_or(false)
        }}
    }}
}}
"""

WASM_MOD = """#[cfg(any(target_os = "wasi", target_family = "wasm"))]
mod wasm {"""


def patch(root: Path) -> bool:
    lib = root / "src" / "lib.rs"
    text = lib.read_text()
    if MARKER in text:
        return False
    if WASM_MOD not in text:
        raise SystemExit(f"{lib}: wasm mod anchor missing")
    lib.write_text(text.replace(WASM_MOD, SOSO_MOD + "\n" + WASM_MOD, 1))
    return True


def main() -> None:
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {sys.argv[0]} <is_executable-dir>")
    root = Path(sys.argv[1])
    if patch(root):
        print(f"patched {root}")


if __name__ == "__main__":
    main()
