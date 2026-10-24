//! Decisión Kev / System One en el host (equivalente a `llama-decide` o `POST /v1/systemone`).
//!
//! ```sh
//! cargo run -p soso-llm-api --features std --example decide -- \
//!   target/kev-4b-model request.json
//! ```
//! Si no pasas fichero, lee el JSON de stdin.

use soso_llm_api::{load_cpu_backend, sha256_hex, ChatBackend};
use soso_llm_core::conversation::ModelProfile;

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        eprintln!("uso: decide <dir-modelo.som> [peticion.json]");
        std::process::exit(2);
    }
    let dir = args.remove(0);
    let body = if let Some(path) = args.first() {
        std::fs::read_to_string(path).unwrap_or_else(|e| {
            eprintln!("leer {path}: {e}");
            std::process::exit(1);
        })
    } else {
        use std::io::Read;
        let mut s = String::new();
        std::io::stdin().read_to_string(&mut s).expect("stdin");
        s
    };

    let index_path = format!("{dir}/index.som");
    let tok_path = format!("{dir}/tokenizer.som");
    let index_bytes = std::fs::read(&index_path).unwrap_or_else(|e| {
        eprintln!("{index_path}: {e}");
        std::process::exit(1);
    });
    let tok_bytes = std::fs::read(&tok_path).unwrap_or_else(|e| {
        eprintln!("{tok_path}: {e}");
        std::process::exit(1);
    });

    let profile = ModelProfile {
        id: String::from("kev"),
        directory: dir.clone(),
        family: String::from("qwen35"),
        weights_sha256: sha256_hex(&index_bytes),
        tokenizer_sha256: sha256_hex(&tok_bytes),
        template_sha256: String::new(),
        context_tokens: 8192,
        max_output_tokens: 0,
        stop_token_ids: vec![],
    };

    let mut backend = load_cpu_backend(&dir, profile).unwrap_or_else(|e| {
        eprintln!("carga: {e}");
        std::process::exit(1);
    });
    if let Some(n) = std::env::var("HILOS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|&n| n > 1)
    {
        backend.parallel = Some(std::sync::Arc::new(soso_llm_api::HostPool::new(n)));
    }

    let out = backend.systemone_json(&body).unwrap_or_else(|e| {
        eprintln!("decide: {e}");
        std::process::exit(1);
    });
    println!("{out}");
}
