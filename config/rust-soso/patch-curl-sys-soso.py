#!/usr/bin/env python3
"""C-073: curl-sys sets link_openssl for non-Windows but openssl-sys is unix-only in Cargo.toml."""
from __future__ import annotations

import sys
from pathlib import Path

MARKER = "C-073 soso: skip OpenSSL until T42"
BUILD_SNIPPET = f"""        }} else if target.contains("-soso") {{
            // {MARKER}
        }} else {{"""

BUILD_NEEDLE = """        } else {
            cfg.define("USE_OPENSSL", None)
                .file("curl/lib/vtls/openssl.c");

            println!("cargo:rustc-cfg=link_openssl");"""

LIB_NEEDLE = "#[cfg(link_openssl)]\nextern crate openssl_sys;"
LIB_REPL = "#[cfg(all(link_openssl, unix))]\nextern crate openssl_sys;"


def patch_build_rs(path: Path) -> bool:
    text = path.read_text()
    if MARKER in text:
        return False
    if BUILD_NEEDLE not in text:
        raise SystemExit(f"{path}: build.rs needle missing")
    path.write_text(text.replace(BUILD_NEEDLE, BUILD_SNIPPET, 1))
    return True


def patch_lib_rs(path: Path) -> bool:
    text = path.read_text()
    changed = False
    if LIB_NEEDLE in text:
        text = text.replace(LIB_NEEDLE, LIB_REPL, 1)
        changed = True
    elif "#[cfg(all(link_openssl, unix))]" not in text:
        raise SystemExit(f"{path}: lib.rs openssl needle missing")

    unix_cfg = "#[cfg(unix)]"
    unix_any = '#[cfg(any(unix, target_os = "soso"))]'
    if unix_cfg in text:
        text = text.replace(unix_cfg, unix_any)
        changed = True
    path.write_text(text)
    return changed


def main() -> None:
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {sys.argv[0]} <curl-sys-dir>")
    root = Path(sys.argv[1])
    changed = patch_build_rs(root / "build.rs")
    changed |= patch_lib_rs(root / "lib.rs")
    if changed:
        print(f"patched {root}")


if __name__ == "__main__":
    main()
