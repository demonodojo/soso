//! Arranque de `soso-improve`. Todo lo demás vive en la librería del mismo
//! crate, para que las pruebas de integración puedan usarlo.

use soso_improve::{capacidades, despachar, USO};
use soso_improve_core::cli::Codigo;
use soso_improve_core::Error;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() || args[0] == "-h" || args[0] == "--help" {
        print!("{USO}");
        std::process::exit(if args.is_empty() {
            Codigo::Error.como_i32()
        } else {
            Codigo::Exito.como_i32()
        });
    }
    if args[0] == "capacidades" {
        for c in capacidades().lista() {
            println!("{}", c.nombre());
        }
        std::process::exit(Codigo::Exito.como_i32());
    }
    let orden = args[0].clone();
    let resto: Vec<String> = args[1..].to_vec();

    let codigo = match despachar(&orden, &resto) {
        Ok(c) => c,
        Err(e) => {
            // La clasificación es la del core, compartida con el guest (T45):
            // `Inestable` tiene código propio porque no es que la medida
            // fallara, es que no se pudo medir.
            if let Error::Inestable(detalle) = &e {
                eprintln!("captura inestable: el checkout cambió mientras se leía");
                for d in detalle.split("; ").take(20) {
                    eprintln!("  {d}");
                }
            } else {
                eprintln!("error: {e}");
            }
            Codigo::de_error(&e).como_i32()
        }
    };
    std::process::exit(codigo);
}
