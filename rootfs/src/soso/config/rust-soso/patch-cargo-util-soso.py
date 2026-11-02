#!/usr/bin/env python3
"""C-078: cargo-util uses std::os::unix; soso only has os::soso::ffi."""
from __future__ import annotations

import sys
from pathlib import Path

MARKER = "C-078 soso"
UNIX_NOT_SOSO = "#[cfg(all(unix, not(target_os = \"soso\")))]"
UNIX_OR_SOSO = '#[cfg(any(unix, target_os = "soso"))]'

READ2_MOD_OLD = f"""{UNIX_OR_SOSO}
mod imp {{
    use libc::{{F_GETFL, F_SETFL, O_NONBLOCK, c_int, fcntl}};"""

READ2_MOD_NEW = f"""{UNIX_NOT_SOSO}
mod imp {{
    use libc::{{F_GETFL, F_SETFL, O_NONBLOCK, c_int, fcntl}};"""

READ2_SOSO_V1 = f"""
// {MARKER}: pipes sin poll/fcntl en soso
#[cfg(target_os = "soso")]
mod imp {{
    use std::io;
    use std::io::prelude::*;
    use std::process::{{ChildStderr, ChildStdout}};

    pub fn read2(
        mut out_pipe: ChildStdout,
        mut err_pipe: ChildStderr,
        data: &mut dyn FnMut(bool, &mut Vec<u8>, bool),
    ) -> io::Result<()> {{
        let mut out = Vec::new();
        let mut err = Vec::new();
        out_pipe.read_to_end(&mut out)?;
        data(true, &mut out, true);
        err_pipe.read_to_end(&mut err)?;
        data(false, &mut err, true);
        Ok(())
    }}
}}
"""

# C-110: sin poll/fcntl no se puede esperar a las dos tuberías a la vez, y leer
# stdout hasta EOF mientras rustc llena stderr interbloquea a los dos (rustc
# duerme en la tubería llena, cargo en la de stdout). stderr se drena en un
# hilo; `data` se llama desde el hilo que llama, con todo lo recibido.
READ2_SOSO = f"""
// {MARKER}: pipes sin poll/fcntl en soso (C-110: stderr en un hilo)
#[cfg(target_os = "soso")]
mod imp {{
    use std::io;
    use std::io::prelude::*;
    use std::process::{{ChildStderr, ChildStdout}};

    pub fn read2(
        mut out_pipe: ChildStdout,
        mut err_pipe: ChildStderr,
        data: &mut dyn FnMut(bool, &mut Vec<u8>, bool),
    ) -> io::Result<()> {{
        let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
        let lector = std::thread::spawn(move || {{
            let mut buf = [0u8; 8192];
            loop {{
                match err_pipe.read(&mut buf) {{
                    Ok(0) | Err(_) => break,
                    Ok(n) => {{
                        if tx.send(buf[..n].to_vec()).is_err() {{
                            break;
                        }}
                    }}
                }}
            }}
        }});
        let mut out = Vec::new();
        let leido = out_pipe.read_to_end(&mut out);
        let _ = lector.join();
        let mut err = Vec::new();
        for trozo in rx.try_iter() {{
            err.extend_from_slice(&trozo);
        }}
        leido?;
        data(true, &mut out, true);
        data(false, &mut err, true);
        Ok(())
    }}
}}
"""

PATH2BYTES_OLD = """pub fn path2bytes(path: &Path) -> Result<&[u8]> {
    #[cfg(any(unix, target_os = "soso"))]
    {
        use std::os::unix::prelude::*;
        Ok(path.as_os_str().as_bytes())
    }"""

PATH2BYTES_NEW = f"""pub fn path2bytes(path: &Path) -> Result<&[u8]> {{
    {UNIX_NOT_SOSO}
    {{
        use std::os::unix::prelude::*;
        Ok(path.as_os_str().as_bytes())
    }}
    #[cfg(target_os = "soso")]
    {{
        use std::os::soso::ffi::OsStrExt;
        Ok(path.as_os_str().as_bytes())
    }}"""

BYTES2PATH_OLD = """pub fn bytes2path(bytes: &[u8]) -> Result<PathBuf> {
    #[cfg(any(unix, target_os = "soso"))]
    {
        use std::os::unix::prelude::*;
        Ok(PathBuf::from(OsStr::from_bytes(bytes)))
    }"""

BYTES2PATH_NEW = f"""pub fn bytes2path(bytes: &[u8]) -> Result<PathBuf> {{
    {UNIX_NOT_SOSO}
    {{
        use std::os::unix::prelude::*;
        Ok(PathBuf::from(OsStr::from_bytes(bytes)))
    }}
    #[cfg(target_os = "soso")]
    {{
        use std::ffi::OsStr;
        use std::os::soso::ffi::OsStrExt;
        Ok(PathBuf::from(OsStr::from_bytes(bytes)))
    }}"""

ARG0_OLD = """        #[cfg(any(unix, target_os = "soso"))]
        if let Some(arg0) = self.get_arg0() {
            use std::os::unix::process::CommandExt as _;
            command.arg0(arg0);
        }"""

ARG0_NEW = f"""        {UNIX_NOT_SOSO}
        if let Some(arg0) = self.get_arg0() {{
            use std::os::unix::process::CommandExt as _;
            command.arg0(arg0);
        }}"""

IMP_MOD_OLD = f"""{UNIX_OR_SOSO}
mod imp {{
    use super::{{ProcessBuilder, ProcessError, close_tempfile_and_log_error, debug_force_argfile}};
    use anyhow::Result;
    use std::io;
    use std::os::unix::process::CommandExt;"""

IMP_MOD_NEW = f"""{UNIX_NOT_SOSO}
mod imp {{
    use super::{{ProcessBuilder, ProcessError, close_tempfile_and_log_error, debug_force_argfile}};
    use anyhow::Result;
    use std::io;
    use std::os::unix::process::CommandExt;"""

IMP_SOSO = f"""
// {MARKER}: sin CommandExt::exec en soso
#[cfg(target_os = "soso")]
mod imp {{
    use super::{{ProcessBuilder, ProcessError}};
    use anyhow::Result;
    use std::io;

    pub fn exec_replace(process_builder: &ProcessBuilder) -> Result<()> {{
        process_builder.exec()
    }}

    pub fn command_line_too_big(_err: &io::Error) -> bool {{
        false
    }}
}}
"""

STATUS_UNIX_OLD = """    #[cfg(any(unix, target_os = "soso"))]
    fn status_to_string(status: ExitStatus) -> String {
        use std::os::unix::process::*;"""

STATUS_UNIX_NEW = f"""    {UNIX_NOT_SOSO}
    fn status_to_string(status: ExitStatus) -> String {{
        use std::os::unix::process::*;"""

STATUS_SOSO = """
    #[cfg(target_os = "soso")]
    fn status_to_string(status: ExitStatus) -> String {
        status.to_string()
    }
"""

LINK_DIR_OLD = """    let link_result = if src.is_dir() {
        #[cfg(any(unix, target_os = "soso"))]
        use std::os::unix::fs::symlink;
        #[cfg(windows)]
        // FIXME: This should probably panic or have a copy fallback. Symlinks
        // are not supported in all windows environments. Currently symlinking
        // is only used for .dSYM directories on macos, but this shouldn't be
        // accidentally relied upon.
        use std::os::windows::fs::symlink_dir as symlink;

        let dst_dir = dst.parent().unwrap();
        let src = if src.starts_with(dst_dir) {
            src.strip_prefix(dst_dir).unwrap()
        } else {
            src
        };
        symlink(src, dst)
    } else {"""

LINK_DIR_NEW = f"""    let link_result = if src.is_dir() {{
        {UNIX_NOT_SOSO}
        {{
            use std::os::unix::fs::symlink;
            let dst_dir = dst.parent().unwrap();
            let src = if src.starts_with(dst_dir) {{
                src.strip_prefix(dst_dir).unwrap()
            }} else {{
                src
            }};
            symlink(src, dst)
        }}
        #[cfg(target_os = "soso")]
        {{
            use std::io::{{Error, ErrorKind}};
            Err(Error::new(
                ErrorKind::Unsupported,
                "directory link_or_copy on soso",
            ))
        }}
        #[cfg(windows)]
        {{
        // FIXME: This should probably panic or have a copy fallback. Symlinks
        // are not supported in all windows environments. Currently symlinking
        // is only used for .dSYM directories on macos, but this shouldn't be
        // accidentally relied upon.
        use std::os::windows::fs::symlink_dir as symlink;

        let dst_dir = dst.parent().unwrap();
        let src = if src.starts_with(dst_dir) {{
            src.strip_prefix(dst_dir).unwrap()
        }} else {{
            src
        }};
        symlink(src, dst)
        }}
    }} else {{"""

CARGO_LIBC_OLD = '[target.\'cfg(unix)\'.dependencies]\nlibc.workspace = true'
CARGO_LIBC_NEW = (
    '[target.\'cfg(any(unix, target_os = "soso"))\'.dependencies]\nlibc.workspace = true'
)


def patch_file(path: Path) -> bool:
    text = path.read_text()
    orig = text
    name = path.name

    if name == "read2.rs":
        if READ2_SOSO_V1.strip() in text:
            text = text.replace(READ2_SOSO_V1.strip(), READ2_SOSO.strip(), 1)
        elif READ2_SOSO.strip() not in text:
            if READ2_MOD_OLD in text:
                text = text.replace(READ2_MOD_OLD, READ2_MOD_NEW, 1)
            if "#[cfg(windows)]" in text and MARKER not in text:
                text = text.replace("#[cfg(windows)]", READ2_SOSO + "\n#[cfg(windows)]", 1)

    if name == "paths.rs":
        for old, new in (
            (PATH2BYTES_OLD, PATH2BYTES_NEW),
            (BYTES2PATH_OLD, BYTES2PATH_NEW),
            (LINK_DIR_OLD, LINK_DIR_NEW),
        ):
            if old in text:
                text = text.replace(old, new, 1)
        text = text.replace(
            "#[cfg(any(unix, target_os = \"soso\"))]\n    let perms = path.metadata",
            f"{UNIX_NOT_SOSO}\n    let perms = path.metadata",
            1,
        )
        text = text.replace(
            "#[cfg(any(unix, target_os = \"soso\"))]\n    if let Some(perms) = perms",
            f"{UNIX_NOT_SOSO}\n    if let Some(perms) = perms",
            1,
        )
        text = text.replace(
            "#[cfg(any(unix, target_os = \"soso\"))]\n    fn write_atomic_permissions",
            f"{UNIX_NOT_SOSO}\n    fn write_atomic_permissions",
            1,
        )
        text = text.replace(
            "#[cfg(any(unix, target_os = \"soso\"))]\n        std::os::unix::fs::symlink",
            f"{UNIX_NOT_SOSO}\n        std::os::unix::fs::symlink",
            1,
        )

    if name == "process_builder.rs":
        if IMP_MOD_OLD in text:
            text = text.replace(IMP_MOD_OLD, IMP_MOD_NEW, 1)
        if ARG0_OLD in text:
            text = text.replace(ARG0_OLD, ARG0_NEW, 1)
        if IMP_SOSO.strip() not in text and "#[cfg(windows)]" in text:
            text = text.replace(
                "#[cfg(windows)]\nmod imp {",
                IMP_SOSO + "\n#[cfg(windows)]\nmod imp {",
                1,
            )

    if name == "process_error.rs":
        if STATUS_UNIX_OLD in text:
            text = text.replace(STATUS_UNIX_OLD, STATUS_UNIX_NEW, 1)
        if STATUS_SOSO.strip() not in text and "#[cfg(windows)]" in text:
            text = text.replace(
                "#[cfg(windows)]\n    fn status_to_string",
                STATUS_SOSO + "\n    #[cfg(windows)]\n    fn status_to_string",
                1,
            )

    if text != orig:
        path.write_text(text)
        return True
    return False


def patch_cargo_toml(path: Path) -> bool:
    text = path.read_text()
    if CARGO_LIBC_NEW in text:
        return False
    if CARGO_LIBC_OLD not in text:
        return False
    path.write_text(text.replace(CARGO_LIBC_OLD, CARGO_LIBC_NEW, 1))
    return True


def main() -> None:
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {sys.argv[0]} <cargo-util-dir>")
    root = Path(sys.argv[1])
    changed = patch_cargo_toml(root / "Cargo.toml")
    for path in sorted((root / "src").rglob("*.rs")):
        changed |= patch_file(path)
    if changed:
        print(f"patched {root}")


if __name__ == "__main__":
    main()
