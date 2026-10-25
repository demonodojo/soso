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

    let mut backend = load_cpu_backend(&model_dir, profile).unwrap_or_else(|e| {
        eprintln!("carga: {e}");
        std::process::exit(1);
    });
    // `HILOS=N`: reparte matvec, atención y FFN entre N hilos (por defecto uno).
    if let Some(n) = std::env::var("HILOS").ok().and_then(|v| v.parse::<usize>().ok()).filter(|&n| n > 1) {
        backend.parallel = Some(std::sync::Arc::new(soso_llm_api::HostPool::new(n)));
        eprintln!("serve_host: {n} hilos");
    }
    // `SOSO_CUDA=1`: pesos residentes en la GPU NVIDIA del host (kernels Q4_0/Q8_0); lo que no cabe va a CPU.
    let mut con_gpu = false;
    if matches!(std::env::var("SOSO_CUDA").as_deref(), Ok("1") | Ok("true")) {
        match soso_llm_api::cuda::CudaGpu::nueva() {
            Ok(g) => {
                backend.gpu = Some(Box::new(g));
                con_gpu = true;
                eprintln!("serve_host: GPU CUDA activa (pesos residentes bajo demanda)");
            }
            Err(e) => eprintln!("serve_host: sin GPU ({e}); sigo en CPU"),
        }
    }
    // Muestreo por defecto de las peticiones que no lo traen (los Qwen3.x recomiendan top_p 0,8 y top_k 20; llama.cpp los aplica solo).
    if let Some(p) = std::env::var("SOSO_TOP_P").ok().and_then(|v| v.parse::<f32>().ok()) {
        soso_llm_api::request::fijar_top_p_defecto(p);
        eprintln!("serve_host: top_p por defecto {p}");
    }
    if std::env::var_os("SOSO_RECORTAR_SALIDA").is_some() {
        soso_llm_api::request::fijar_recortar_salida(true);
        eprintln!("serve_host: la salida se recorta al contexto que quede");
    }
    if let Some(k) = std::env::var("SOSO_TOP_K").ok().and_then(|v| v.parse::<usize>().ok()) {
        soso_llm_core::sample::fijar_top_k_defecto(k);
        eprintln!("serve_host: top_k por defecto {k}");
    }
    let model_id = backend.profile().id.clone();
    let service = Arc::new(HostService::new(token, backend));
    let addr: SocketAddr = ([127, 0, 0, 1], port).into();
    let listener = TcpListener::bind(addr).unwrap_or_else(|e| {
        eprintln!("bind {addr}: {e}");
        std::process::exit(1);
    });
    eprintln!(
        "serve_host: escuchando en http://127.0.0.1:{port} model={model_id} backend={}",
        if con_gpu { "host-cpu+cuda" } else { "host-cpu" }
    );
    if let Err(e) = run_accept_loop(service, listener) {
        eprintln!("accept: {e}");
        std::process::exit(1);
    }
}
