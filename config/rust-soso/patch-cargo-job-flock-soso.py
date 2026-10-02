#!/usr/bin/env python3
"""C-088: cargo util/job.rs and flock.rs gated on cfg(unix)."""
from __future__ import annotations

import sys
from pathlib import Path

MARKER = "C-088 soso"

JOB_UNIX_OLD = """#[cfg(unix)]
mod imp {
    use std::env;

    pub type Setup = ();

    pub unsafe fn setup() -> Option<()> {"""

JOB_UNIX_NEW = """#[cfg(all(unix, not(target_os = "soso")))]
mod imp {
    use std::env;

    pub type Setup = ();

    pub unsafe fn setup() -> Option<()> {"""

JOB_SOSO = f"""
// {MARKER}: sin setsid en soso; Ctrl-C vía grupo de procesos del kernel
#[cfg(target_os = "soso")]
mod imp {{
    pub type Setup = ();

    pub unsafe fn setup() -> Option<()> {{
        Some(())
    }}
}}
"""

JOB_WINDOWS = """#[cfg(windows)]
mod imp {
    use std::io;"""

FLOCK_ERR_UNIX = """#[cfg(unix)]
fn error_unsupported(err: &std::io::Error) -> bool {
    match err.raw_os_error() {
        // Unfortunately, depending on the target, these may or may not be the same.
        // For targets in which they are the same, the duplicate pattern causes a warning.
        #[allow(unreachable_patterns)]
        Some(libc::ENOTSUP | libc::EOPNOTSUPP) => true,
        Some(libc::ENOSYS) => true,
        _ => err.kind() == std::io::ErrorKind::Unsupported,
    }
}"""

FLOCK_ERR_SOSO = f"""#[cfg(unix)]
fn error_unsupported(err: &std::io::Error) -> bool {{
    match err.raw_os_error() {{
        // Unfortunately, depending on the target, these may or may not be the same.
        // For targets in which they are the same, the duplicate pattern causes a warning.
        #[allow(unreachable_patterns)]
        Some(libc::ENOTSUP | libc::EOPNOTSUPP) => true,
        Some(libc::ENOSYS) => true,
        _ => err.kind() == std::io::ErrorKind::Unsupported,
    }}
}}

// {MARKER}
#[cfg(target_os = "soso")]
fn error_unsupported(err: &std::io::Error) -> bool {{
    err.kind() == std::io::ErrorKind::Unsupported
}}"""

CARGO_LIBC_OLD = "[target.'cfg(unix)'.dependencies]\nlibc.workspace = true"
CARGO_LIBC_NEW = "[target.'cfg(any(unix, target_os = \"soso\"))'.dependencies]\nlibc.workspace = true"


def patch_job(job: Path) -> bool:
    text = job.read_text()
    if MARKER in text:
        return False
    if JOB_UNIX_OLD not in text:
        raise SystemExit(f"{job}: unix imp anchor missing")
    if JOB_WINDOWS not in text:
        raise SystemExit(f"{job}: windows imp anchor missing")
    text = text.replace(JOB_UNIX_OLD, JOB_UNIX_NEW, 1)
    text = text.replace(JOB_WINDOWS, JOB_SOSO + "\n" + JOB_WINDOWS, 1)
    job.write_text(text)
    return True


def patch_flock(flock: Path) -> bool:
    text = flock.read_text()
    if MARKER in text:
        legacy = """    err.kind() == std::io::ErrorKind::Unsupported
        || matches!(err.raw_os_error(), Some(libc::ENOSYS))"""
        if legacy in text:
            flock.write_text(
                text.replace(
                    legacy,
                    "    err.kind() == std::io::ErrorKind::Unsupported",
                    1,
                )
            )
            return True
        return False
    if FLOCK_ERR_UNIX not in text:
        raise SystemExit(f"{flock}: error_unsupported anchor missing")
    flock.write_text(text.replace(FLOCK_ERR_UNIX, FLOCK_ERR_SOSO, 1))
    return True


def patch_cargo_toml(cargo_root: Path) -> bool:
    toml = cargo_root / "Cargo.toml"
    if not toml.is_file():
        return False
    text = toml.read_text()
    if CARGO_LIBC_NEW in text:
        return False
    if CARGO_LIBC_OLD not in text:
        return False
    toml.write_text(text.replace(CARGO_LIBC_OLD, CARGO_LIBC_NEW, 1))
    return True


def patch(cargo_root: Path) -> bool:
    util = cargo_root / "src" / "util"
    changed = patch_job(util / "job.rs")
    changed |= patch_flock(util / "flock.rs")
    changed |= patch_cargo_toml(cargo_root)
    return changed


def main() -> None:
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {sys.argv[0]} <cargo-workspace-root>")
    root = Path(sys.argv[1])
    if patch(root):
        print(f"patched {root}")


if __name__ == "__main__":
    main()
