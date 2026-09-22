//! Servidor de desarrollo host (T13): API Chat Completions sobre pesos reales.
//!
//! ```sh
//! export SOSO_LLM_API_KEY=dev-secret
//! cargo run -p soso-llm-api --features std --example serve_host -- \
//!   --model-dir target/qwen2.5-coder-3b-model \
//!   --profile target/self-improvement/tasks/T13/profile.json \
//!   --port 7422
//! ```

use std::net::{SocketAddr, TcpListener};
use std::sync::Arc;

use soso_llm_api::{load_cpu_backend, run_accept_loop, ChatBackend, HostService};
use soso_llm_core::conversation::ModelProfile;

fn usage() -> ! {
    eprintln!(
        "uso: serve_host --model-dir <dir> --profile <profile.json> [--port <n>]\n\
         token: variable SOSO_LLM_API_KEY (obligatoria)"
    );
    std::process::exit(2);
}

fn main() {
    let mut model_dir: Option<String> = None;
    let mut profile_path: Option<String> = None;
    let mut port: u16 = 7422;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--model-dir" => model_dir = args.next(),
            "--profile" => profile_path = args.next(),
            "--port" => {
                port = args
                    .next()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(7422);
            }
            "--help" | "-h" => usage(),
            other => {
                eprintln!("argumento desconocido: {other}");
                usage();
            }
        }
    }
    let model_dir = model_dir.unwrap_or_else(|| usage());
    let profile_path = profile_path.unwrap_or_else(|| usage());
    let token = std::env::var("SOSO_LLM_API_KEY").unwrap_or_else(|_| {
        eprintln!("SOSO_LLM_API_KEY no definida");
        std::process::exit(2);
    });
    if token.is_empty() {
        eprintln!("SOSO_LLM_API_KEY vacía");
        std::process::exit(2);
    }

    let profile_text = std::fs::read_to_string(&profile_path).unwrap_or_else(|e| {
        eprintln!("perfil: {e}");
        std::process::exit(1);
    });
    let mut profile: ModelProfile =
        serde_json::from_str(&profile_text).unwrap_or_else(|e| {
            eprintln!("perfil JSON: {e}");
            std::process::exit(1);
        });
    profile.directory = model_dir.clone();

    let backend = load_cpu_backend(&model_dir, profile).unwrap_or_else(|e| {
        eprintln!("carga: {e}");
        std::process::exit(1);
    });
    let model_id = backend.profile().id.clone();
    let service = Arc::new(HostService::new(token, backend));
    let addr: SocketAddr = ([127, 0, 0, 1], port).into();
    let listener = TcpListener::bind(addr).unwrap_or_else(|e| {
        eprintln!("bind {addr}: {e}");
        std::process::exit(1);
    });
    eprintln!(
        "serve_host: escuchando en http://127.0.0.1:{port} model={model_id} backend=host-cpu"
    );
    if let Err(e) = run_accept_loop(service, listener) {
        eprintln!("accept: {e}");
        std::process::exit(1);
    }
}
