//! Convierte un fichero GGUF (llama) al layout sosomodel (.som).

use std::env;
use std::path::PathBuf;
use std::process;

fn main() {
    let mut args = env::args().skip(1);
    let gguf = args.next().unwrap_or_else(|| {
        eprintln!("uso: convert-gguf <modelo.gguf> [dir_salida] [--name nombre]");
        process::exit(2);
    });
    let mut out = PathBuf::from("target/converted-model");
    let mut name = None;
    let mut rest: Vec<String> = args.collect();
    let mut i = 0;
    while i < rest.len() {
        if rest[i] == "--name" {
            name = rest.get(i + 1).cloned();
            rest.remove(i);
            rest.remove(i);
            continue;
        }
        if !rest[i].starts_with('-') && out == PathBuf::from("target/converted-model") {
            out = PathBuf::from(&rest[i]);
            rest.remove(i);
            continue;
        }
        i += 1;
    }
    if let Err(e) = gguf2som::convert_path(&gguf, &out, name.as_deref()) {
        eprintln!("convert-gguf: {e}");
        process::exit(1);
    }
    println!("convert-gguf: modelo escrito en {}", out.display());
}
