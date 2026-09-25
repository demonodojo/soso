//! Wrapper de **wild** para el target soso. Si `wild` está en PATH lo invoca;
//! si no, falla con instrucciones (el port completo vendrá en iteraciones).

use std::env;
use std::process::{Command, ExitCode};

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
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

#[cfg(test)]
mod tests {
    /// El nombre del binario tiene que ser el que el target pide como enlazador.
    ///
    /// Se separaron —el binario se llamaba `wild` y el target pedía
    /// `wild-soso`— y nadie se enteró porque **nada consume ese target
    /// todavía**. Este test los ata: si alguien cambia uno, falla.
    #[test]
    fn el_binario_se_llama_como_el_target_lo_busca() {
        let spec = include_str!("../../../targets/x86_64-unknown-soso.json");
        let linker = spec
            .lines()
            .find_map(|l| l.trim().strip_prefix("\"linker\":"))
            .expect("el target declara un linker")
            .trim()
            .trim_end_matches(',')
            .trim_matches('"');
        assert_eq!(linker, env!("CARGO_BIN_NAME"));
    }
}
