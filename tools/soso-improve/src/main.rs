//! Arranque de `soso-improve`. Todo lo demás vive en la librería del mismo
//! crate, para que las pruebas de integración puedan usarlo.

use soso_improve::{despachar, USO};
use soso_improve_core::Error;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() || args[0] == "-h" || args[0] == "--help" {
        print!("{USO}");
        std::process::exit(if args.is_empty() { 1 } else { 0 });
    }
    let orden = args[0].clone();
    let resto: Vec<String> = args[1..].to_vec();

    let codigo = match despachar(&orden, &resto) {
        Ok(c) => c,
        Err(Error::Inestable(detalle)) => {
            eprintln!("captura inestable: el checkout cambió mientras se leía");
            for d in detalle.split("; ").take(20) {
                eprintln!("  {d}");
            }
            3
        }
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    };
    std::process::exit(codigo);
}
