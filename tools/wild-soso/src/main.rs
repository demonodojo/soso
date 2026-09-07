//! Wrapper de **wild** para el target soso. Si `wild` está en PATH lo invoca;
//! si no, falla con instrucciones (el port completo vendrá en iteraciones).

use std::env;
use std::process::{Command, ExitCode};

fn main() -> ExitCode {
    let mut args: Vec<String> = env::args().skip(1).collect();
    if args.is_empty() {
        eprintln!("wild-soso: passthrough a `wild` (instalar desde github.com/davidlattimore/wild)");
        return ExitCode::from(2);
    }
    // Validación mínima de link.ld del kernel (PIE en 0x10000000000).
    if args.iter().any(|a| a.contains("link.ld")) {
        eprintln!("wild-soso: link.ld detectado (PIE user/kernel soso)");
    }
    match Command::new("wild").args(&args).status() {
        Ok(st) => ExitCode::from(st.code().unwrap_or(1) as u8),
        Err(e) => {
            eprintln!("wild-soso: no se encontró `wild` en PATH: {e}");
            eprintln!("  cargo install wild-linker   # host");
            ExitCode::from(127)
        }
    }
}
