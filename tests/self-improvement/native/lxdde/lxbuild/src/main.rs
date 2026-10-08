//! `cargo xtask lx-build` como programa suelto (T42, perfil `qemu-lxdde`): compila el C de
//! los ports de lxdde con el clang que haya (`LX_CC`) y los agrupa en `liblxdde.a`.
//!
//! El módulo `lx_build` es **una copia** de `xtask/src/lx_build.rs` (`preparar.sh` la
//! trae); aquí sólo van las dos cosas de `xtask` que usa: `project_root` y `as_user`.
//! Uso: `lxbuild <puerto>...` con `SOSO_LX_ROOT=<raíz>` (por defecto el directorio actual);
//! deja `<raíz>/target/lxdde/liblxdde.a`.

use std::path::PathBuf;

mod lx_build;

pub fn project_root() -> PathBuf {
    std::env::var_os("SOSO_LX_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().expect("directorio actual"))
}

/// Lo que `xtask` hace para no dejar ficheros de root tras un `sudo`: aquí no aplica.
mod as_user {
    use std::path::Path;
    use std::process::Command;

    pub struct Guard;
    pub fn as_invoking_user_for_build(_root: &Path) -> Guard {
        Guard
    }
    pub fn apply_invoking_user(_cmd: &mut Command) {}
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    lx_build::run(&args);
}
