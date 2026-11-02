#!/usr/bin/env python3
"""C-086: url::Url::from_file_path gated on unix/windows/redox/wasi/hermit only."""
from __future__ import annotations

import sys
from pathlib import Path

MARKER = "C-086 soso"

MULTI_OLD = """            unix,
            windows,
            target_os = "redox",
            target_os = "wasi",
            target_os = "hermit"
        )"""

MULTI_NEW = """            unix,
            windows,
            target_os = "redox",
            target_os = "wasi",
            target_os = "hermit",
            target_os = "soso"
        )"""

ANY_OLD = 'any(unix, target_os = "redox", target_os = "wasi", target_os = "hermit")'
ANY_NEW = 'any(unix, target_os = "redox", target_os = "wasi", target_os = "hermit", target_os = "soso")'

OSSTR_OLD = """    #[cfg(any(unix, target_os = "redox"))]
    use std::os::unix::prelude::OsStrExt;"""

OSSTR_NEW = f"""    // {MARKER}
    #[cfg(any(unix, target_os = "redox"))]
    use std::os::unix::prelude::OsStrExt;
    #[cfg(target_os = "soso")]
    use std::os::soso::ffi::OsStrExt;"""


def patch(root: Path) -> None:
    lib = root / "src" / "lib.rs"
    if not lib.is_file():
        raise SystemExit(f"{root}: not a url crate")
    text = lib.read_text()
    if MARKER in text:
        return
    if MULTI_OLD not in text:
        raise SystemExit(f"{root}: no está el bloque cfg file URL multilínea")
    text = text.replace(MULTI_OLD, MULTI_NEW)
    if ANY_OLD not in text:
        raise SystemExit(f"{root}: no está el bloque cfg file URL any()")
    text = text.replace(ANY_OLD, ANY_NEW)
    count = text.count(OSSTR_OLD)
    if count == 0:
        raise SystemExit(f"{root}: no está OsStrExt en path helpers")
    text = text.replace(OSSTR_OLD, OSSTR_NEW)
    lib.write_text(text)


def main() -> None:
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {sys.argv[0]} <url-crate-dir>")
    patch(Path(sys.argv[1]))


if __name__ == "__main__":
    main()
