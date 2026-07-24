//! Proxy TCP soso → llama-server (L6-H CUDA en host Linux).

use std::env;

fn main() {
    let mut listen = String::from("0.0.0.0:11400");
    let mut llama = String::from("http://127.0.0.1:8080");
    let args: Vec<String> = env::args().collect();
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--listen" => {
                i += 1;
                if i < args.len() {
                    listen = args[i].clone();
                }
            }
            "--llama" => {
                i += 1;
                if i < args.len() {
                    llama = args[i].clone();
                }
            }
            "--help" | "-h" => {
                eprintln!("uso: cuda-proxy [--listen HOST:PORT] [--llama http://HOST:PORT]");
                return;
            }
            other => {
                eprintln!("argumento desconocido: {other}");
                return;
            }
        }
        i += 1;
    }
    if let Err(e) = cuda_proxy::serve(&listen, &llama) {
        eprintln!("cuda-proxy: {e}");
        std::process::exit(1);
    }
}
