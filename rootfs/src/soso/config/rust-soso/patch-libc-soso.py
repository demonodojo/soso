#!/usr/bin/env python3
"""Inserta la rama target_os = soso en un árbol libc (registry o soso-libc)."""
from __future__ import annotations

import pathlib
import shutil
import sys

MARKER = "C-074 soso bootstrap"
OLD = """    } else {
        // non-supported targets: empty...
    }"""
SOSO_BRANCH = """    } else if #[cfg(target_os = "soso")] {
        mod primitives;
        pub use crate::primitives::*;

        mod unix;
        pub use crate::unix::*;

        prelude!();
    }"""
SOSO_MINIMAL = """    } else if #[cfg(target_os = "soso")] {
        mod primitives;
        pub use crate::primitives::*;

        mod soso;
        pub use crate::soso::*;
    }"""
OLD_SOSO_ONLY = """    } else if #[cfg(target_os = "soso")] {
        mod primitives;
        pub use crate::primitives::*;

        mod soso;
        pub use crate::soso::*;"""
ALLOW = (
    "#[allow(unused_imports)] // needed while the module is empty on some platforms\n"
    "pub use new::*;"
)
ALLOW2 = (
    "#[allow(unused_imports, unreachable_pub)] // needed while the module is empty on some platforms\n"
    "pub use new::*;"
)


def patch_ioctl_casts(libc_root: pathlib.Path) -> None:
    needle = '#[cfg(any(target_os = "linux", target_os = "android", target_os = "l4re"))]'
    repl = (
        '#[cfg(any(target_os = "linux", target_os = "android", target_os = "l4re", target_os = "soso"))]'
    )
    for rel in ("src/macros.rs", "src/types.rs"):
        path = libc_root / rel
        if not path.is_file():
            continue
        text = path.read_text()
        if needle not in text or "target_os = \"soso\"" in text.split("u32_cast_ioctl", 1)[0][-160:]:
            continue
        path.write_text(text.replace(needle, repl, 1))


def patch_linux_api(libc_root: pathlib.Path) -> None:
    new_mod = libc_root / "src/new/mod.rs"
    if new_mod.is_file():
        nm = new_mod.read_text()
        nm_needle = '    } else if #[cfg(target_os = "linux")] {\n        mod linux_uapi;'
        nm_repl = (
            '    } else if #[cfg(any(target_os = "linux", target_os = "soso"))] {\n'
            "        mod linux_uapi;"
        )
        if nm_needle in nm and "target_os = \"soso\"" not in nm.split("linux_uapi", 1)[0][-80:]:
            new_mod.write_text(nm.replace(nm_needle, nm_repl, 1))
        nm = new_mod.read_text()
        export_needle = '    } else if #[cfg(target_os = "linux")] {\n        pub use linux::can::bcm::*;'
        export_repl = (
            '    } else if #[cfg(any(target_os = "linux", target_os = "soso"))] {\n'
            "        pub use linux::can::bcm::*;"
        )
        if export_needle in nm and "target_os = \"soso\"" not in nm.split("pub use linux::can::bcm", 1)[0][-120:]:
            new_mod.write_text(nm.replace(export_needle, export_repl, 1))

    unix_mod = libc_root / "src/unix/mod.rs"
    text = unix_mod.read_text()
    needle = """    } else if #[cfg(any(
        target_os = "linux",
        target_os = "l4re",
        target_os = "android",
        target_os = "emscripten"
    ))] {"""
    repl = """    } else if #[cfg(any(
        target_os = "linux",
        target_os = "soso",
        target_os = "l4re",
        target_os = "android",
        target_os = "emscripten"
    ))] {"""
    if needle in text and "target_os = \"soso\"," not in text:
        unix_mod.write_text(text.replace(needle, repl, 1))

    linux_like = libc_root / "src/unix/linux_like/mod.rs"
    if not linux_like.is_file():
        return
    ll = linux_like.read_text()
    ll_needle = "    } else if #[cfg(target_os = \"linux\")] {\n        mod linux;"
    ll_repl = (
        "    } else if #[cfg(any(target_os = \"linux\", target_os = \"soso\"))] {\n"
        "        mod linux;"
    )
    ll_soso_only = """    } else if #[cfg(target_os = "soso")] {
        mod linux;
        pub use self::linux::*;
    } else if #[cfg(target_os = "linux")] {
        mod linux;"""
    if ll_soso_only in ll:
        linux_like.write_text(ll.replace(ll_soso_only, ll_repl, 1))
    elif ll_needle in ll and "target_os = \"soso\"" not in ll:
        linux_like.write_text(ll.replace(ll_needle, ll_repl, 1))

    l4re = libc_root / "src/unix/linux_like/linux_l4re_shared.rs"
    if l4re.is_file():
        l4 = l4re.read_text()
        l4_old = '    if #[cfg(not(target_env = "gnu"))] {\n        extern_ty! {\n            pub enum fpos64_t {} // FIXME(linux): fill this out with a struct'
        l4_new = (
            '    if #[cfg(not(any(target_env = "gnu", target_os = "soso")))] {\n'
            "        extern_ty! {\n"
            "            pub enum fpos64_t {} // FIXME(linux): fill this out with a struct"
        )
        if l4_old in l4 and "target_os = \"soso\"" not in l4.split("fpos64_t", 1)[0][-120:]:
            l4re.write_text(l4.replace(l4_old, l4_new, 1))

    linux_mod = libc_root / "src/unix/linux_like/linux/mod.rs"
    if not linux_mod.is_file():
        return
    lm = linux_mod.read_text()
    gnu_needle = "    } else if #[cfg(target_env = \"gnu\")] {\n        mod gnu;"
    gnu_repl = (
        "    } else if #[cfg(any(target_env = \"gnu\", target_os = \"soso\"))] {\n"
        "        mod gnu;"
    )
    if gnu_needle in lm and "target_os = \"soso\"" not in lm:
        linux_mod.write_text(lm.replace(gnu_needle, gnu_repl, 1))

def main() -> None:
    libc_root = pathlib.Path(sys.argv[1])
    vendor_libc = pathlib.Path(sys.argv[2])
    soso_rs = vendor_libc / "soso.rs"
    lib_rs = libc_root / "src/lib.rs"
    if not lib_rs.is_file():
        sys.exit(f"patch-libc-soso: no {lib_rs}")
    shutil.copy(soso_rs, libc_root / "src/soso.rs")
    use_unix = (libc_root / "src/unix/linux_like/mod.rs").is_file()
    branch = SOSO_BRANCH if use_unix else SOSO_MINIMAL
    new = branch + """ else {
        // non-supported targets: empty...
    }"""
    text = lib_rs.read_text()
    unix_branch = """    } else if #[cfg(target_os = "soso")] {
        mod primitives;
        pub use crate::primitives::*;

        mod unix;
        pub use crate::unix::*;

        prelude!();"""
    if OLD_SOSO_ONLY in text and use_unix:
        text = text.replace(OLD_SOSO_ONLY, unix_branch, 1)
    elif 'target_os = "soso"' not in text:
        if OLD not in text:
            sys.exit("patch-libc-soso: libc sin la rama vacía")
        text = text.replace(OLD, new, 1)
    text = text.replace(ALLOW, ALLOW2, 1)
    soso_unix_no_prelude = """    } else if #[cfg(target_os = "soso")] {
        mod primitives;
        pub use crate::primitives::*;

        mod unix;
        pub use crate::unix::*;
    } else {"""
    soso_unix_with_prelude = """    } else if #[cfg(target_os = "soso")] {
        mod primitives;
        pub use crate::primitives::*;

        mod unix;
        pub use crate::unix::*;

        prelude!();
    } else {"""
    if soso_unix_no_prelude in text:
        text = text.replace(soso_unix_no_prelude, soso_unix_with_prelude, 1)
    xous_broken = """        prelude!();
} else if #[cfg(target_os = "soso")] {
        mod primitives;
        pub use crate::primitives::*;

        mod unix;
        pub use crate::unix::*;
    }
    } else {"""
    xous_fixed = """        prelude!();
    } else if #[cfg(target_os = "soso")] {
        mod primitives;
        pub use crate::primitives::*;

        mod unix;
        pub use crate::unix::*;

        prelude!();
    } else {"""
    if xous_broken in text:
        text = text.replace(xous_broken, xous_fixed, 1)
    lib_rs.write_text(text)
    if use_unix:
        patch_ioctl_casts(libc_root)
        patch_linux_api(libc_root)


if __name__ == "__main__":
    main()
