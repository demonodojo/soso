#!/usr/bin/env python3
"""C-076: libssh2-sys gates socket type and init on cfg(unix); soso is not unix."""
from __future__ import annotations

import sys
from pathlib import Path

MARKER = "C-076 soso"

OPENSSL_EXTERN_OLD = "#[cfg(unix)]\nextern crate openssl_sys;"
OPENSSL_EXTERN_NEW = '#[cfg(all(unix, not(target_os = "soso")))]\nextern crate openssl_sys;'

SOCKET_TYPE_OLD = "#[cfg(unix)]\npub type libssh2_socket_t = c_int;"
SOCKET_TYPE_NEW = '#[cfg(any(unix, target_os = "soso"))]\npub type libssh2_socket_t = c_int;'

PLATFORM_UNIX_OLD = """    #[cfg(unix)]
    unsafe fn platform_init() {
        // On Unix we want to funnel through openssl_sys to initialize OpenSSL,
        // so be sure to tell libssh2 to not do its own thing as we've already
        // taken care of it.
        openssl_sys::init();
        assert_eq!(libssh2_init(LIBSSH2_INIT_NO_CRYPTO), 0);
    }"""

PLATFORM_UNIX_NEW = f"""    #[cfg(all(unix, not(target_os = "soso")))]
    unsafe fn platform_init() {{
        // On Unix we want to funnel through openssl_sys to initialize OpenSSL,
        // so be sure to tell libssh2 to not do its own thing as we've already
        // taken care of it.
        openssl_sys::init();
        assert_eq!(libssh2_init(LIBSSH2_INIT_NO_CRYPTO), 0);
    }}

    // {MARKER}: sin openssl-sys en el target (T42); igual que Windows.
    #[cfg(target_os = "soso")]
    unsafe fn platform_init() {{
        assert_eq!(libssh2_init(0), 0);
    }}"""


def patch_lib_rs(path: Path) -> bool:
    text = path.read_text()
    orig = text
    if OPENSSL_EXTERN_OLD in text:
        text = text.replace(OPENSSL_EXTERN_OLD, OPENSSL_EXTERN_NEW, 1)
    if SOCKET_TYPE_OLD in text:
        text = text.replace(SOCKET_TYPE_OLD, SOCKET_TYPE_NEW, 1)
    if PLATFORM_UNIX_OLD in text and MARKER not in text:
        text = text.replace(PLATFORM_UNIX_OLD, PLATFORM_UNIX_NEW, 1)
    if text != orig:
        path.write_text(text)
        return True
    return False


def main() -> None:
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {sys.argv[0]} <libssh2-sys-dir>")
    root = Path(sys.argv[1])
    if patch_lib_rs(root / "lib.rs"):
        print(f"patched {root}")


if __name__ == "__main__":
    main()
