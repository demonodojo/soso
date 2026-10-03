#!/usr/bin/env python3
"""C-104 jobserver 0.1.x: soso usa la implementación wasm (hilos, sin proceso
cruzado), pero cargo llama a `configure`/`string_arg` al lanzar rustc y esas
dos hacen panic. Sin jobserver heredado, el hijo trabaja con lo que tenga."""
from __future__ import annotations

import sys
from pathlib import Path

MARKER = "C-104 jobserver soso"

STRING_ARG_OLD = """    pub fn string_arg(&self) -> String {
        panic!(
            "On this platform there is no cross process jobserver support,
             so Client::configure is not supported."
        );
    }"""
STRING_ARG_NEW = f"""    // {MARKER}
    pub fn string_arg(&self) -> String {{
        String::new()
    }}"""

CONFIGURE_OLD = """    pub fn configure(&self, _cmd: &mut Command) {
        unreachable!();
    }"""
CONFIGURE_NEW = """    pub fn configure(&self, _cmd: &mut Command) {}"""


def patch(root: Path) -> bool:
    wasm = root / "src" / "wasm.rs"
    text = wasm.read_text()
    if MARKER in text:
        return False
    for old in (STRING_ARG_OLD, CONFIGURE_OLD):
        if old not in text:
            raise SystemExit(f"{wasm}: ancla ausente: {old[:40]!r}")
    text = text.replace(STRING_ARG_OLD, STRING_ARG_NEW, 1).replace(CONFIGURE_OLD, CONFIGURE_NEW, 1)
    wasm.write_text(text)
    return True


def main() -> None:
    if len(sys.argv) != 2:
        raise SystemExit(f"uso: {sys.argv[0]} <jobserver-dir>")
    root = Path(sys.argv[1])
    if patch(root):
        print(f"patched {root}")


if __name__ == "__main__":
    main()
