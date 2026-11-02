#!/usr/bin/env python3
"""C-068: apunta las tres entradas getrandom del lock de Cargo a src/soso-getrandom-*."""
from __future__ import annotations

import re
import sys
from pathlib import Path

VERSIONS = ("0.2.17", "0.3.4", "0.4.3")
DIRS = {
    "0.2.17": "soso-getrandom-0.2",
    "0.3.4": "soso-getrandom-0.3",
    "0.4.3": "soso-getrandom-0.4",
}


def main() -> None:
    lock_path = Path(sys.argv[1])
    rust_src = Path(sys.argv[2]).resolve()
    text = lock_path.read_text()
    for ver in VERSIONS:
        crate_dir = (rust_src / DIRS[ver]).resolve()
        if not crate_dir.is_dir():
            sys.exit(f"missing {crate_dir}")
        uri = crate_dir.as_uri()
        block = (
            rf'(\[\[package\]\]\nname = "getrandom"\nversion = "{re.escape(ver)}"\n)'
            r'source = "[^"]+"\n'
            r'checksum = "[^"]+"\n'
        )
        repl = rf'\1source = "path+{uri}#getrandom@{ver}"\n'
        if re.search(rf'name = "getrandom"\nversion = "{re.escape(ver)}"\nsource = "path\+', text):
            continue  # ya apuntado (el script se puede volver a ejecutar)
        text, n = re.subn(block, repl, text, count=1)
        if n != 1:
            sys.exit(f"getrandom {ver}: expected 1 lock entry, patched {n}")
    lock_path.write_text(text)


if __name__ == "__main__":
    main()
