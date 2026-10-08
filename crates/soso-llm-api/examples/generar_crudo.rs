//! T22: genera sobre pesos reales en el host y enseña el texto **crudo** del modelo.
//!
//! El servidor sólo cuenta si la llamada a herramienta fue válida; para saber *qué*
//! escribió el modelo cuando no lo fue hace falta verlo antes del parseo. El host
//! usa el mismo runtime que el guest, pero va mucho más deprisa, así que sirve para
//! iterar sobre el prompt sin pagar horas de prefill en QEMU.
//!
//! ```sh
//! cargo run --release -p soso-llm-api --features std --example generar_crudo -- \
//!   target/qwen2.5-coder-7b-model [--max 200] peticion1.json [peticion2.json …]
//! ```
//! Con varias peticiones seguidas se aprovecha el prefijo común (T81).

use std::time::Instant;

use soso_llm_api::{load_cpu_backend, prepare_chat_completion, ChatBackend};
use soso_llm_core::conversation::ModelProfile;
use soso_llm_core::generation::{GenCheckpoint, GenerationObserver};

struct Mostrar {
    ids: Vec<u32>,
}

impl GenerationObserver for Mostrar {
    fn on_token(&mut self, id: u32) {
        self.ids.push(id);
    }
    fn cancel_at(&mut self, _at: GenCheckpoint) -> bool {
        false
    }
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 2 {
        eprintln!("uso: generar_crudo <dir-modelo> [--max N] <peticion.json>…");
        std::process::exit(2);
    }
    let dir = args.remove(0);
    let mut max: Option<u32> = None;
    if args.first().map(String::as_str) == Some("--max") {
        args.remove(0);
        max = args.remove(0).parse().ok();
    }
    let profile = ModelProfile {
        id: String::from("qwen2.5-coder-7b"),
        directory: dir.clone(),
        family: String::from("qwen2"),
        // Los del `model-lock-7b.json` de T14: el backend comprueba el hash del índice.
        weights_sha256: String::from(
            "acc11e3ea6bde123e22f19160574f7d058a39a04ae3dd9aa0db78fb0994aa3e4",
        ),
        tokenizer_sha256: String::from(
            "12c46448a072b0e16485ab1c35abf575c84a30e6eb6ab4b89dd82a5f91faba73",
        ),
        template_sha256: String::from(
            "8eb6214486f4b35b6a643afa2065ee6d4d6248dfd277455d0f535ff2429fe985",
        ),
        context_tokens: 32_768,
        max_output_tokens: 2048,
        stop_token_ids: vec![151_645, 151_643],
    };
    let mut backend = load_cpu_backend(&dir, profile.clone()).unwrap_or_else(|e| {
        eprintln!("carga: {e}");
        std::process::exit(1);
    });
    if std::env::var_os("PLANNER").is_some() {
        // Planificador como el del guest con 8 GiB: pesos residentes, ventana de KV,
        // y atención dispersa para secuencias de más de 256 tokens.
        use soso_llm_core::plan::{MemSnapshot, ResourcePlanner};
        let mem = MemSnapshot {
            total_frames: 8 * 1024 * 1024 * 1024 / 4096,
            free_frames: 6 * 1024 * 1024 * 1024 / 4096,
            reclaimable_frames: 0,
        };
        let m = backend.rt.manifest.clone();
        let mut pl = ResourcePlanner::new(&m, &backend.rt.index, mem, 0, false);
        if std::env::var_os("NO_SPARSE").is_some() {
            pl.desactivar_atencion_dispersa();
        }
        eprintln!(
            "PLANNER: h2o={} sparse(seq>256)={} ventana={} pesos residentes={}",
            pl.use_h2o(),
            pl.use_sparse_attn(1000),
            pl.kv_window_tokens(),
            pl.keep_weights_mapped()
        );
        backend.rt.set_planner(pl);
    }
    if std::env::var_os("KV_I8").is_some() {
        // Fuerza KV int8 (KIVI-lite) como hace el planificador del guest con ventana corta.
        use soso_llm_core::kv::{KvDtype, LayerKv};
        let m = backend.rt.manifest.clone();
        for l in 0..m.num_layers {
            let kv_dim = m.effective_num_kv_heads(l) as usize * m.effective_head_dim(l) as usize;
            backend.rt.kv[l as usize] = LayerKv::with_capacity_dtype(256, kv_dim, KvDtype::I8);
        }
        eprintln!("KV int8 FORZADO (KV_I8)");
    }
    if std::env::var_os("SIN_BLOQUES").is_some() {
        backend.rt.prefill_por_bloques = false;
        eprintln!("prefill por bloques DESACTIVADO (SIN_BLOQUES)");
    }
    for peticion in &args {
        let mut cuerpo: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(peticion).expect("leer")).expect("json");
        cuerpo["model"] = serde_json::json!(profile.id);
        cuerpo["stream"] = serde_json::json!(false);
        if let Some(m) = max {
            cuerpo["max_tokens"] = serde_json::json!(m);
        }
        let texto = cuerpo.to_string();
        let prepared = prepare_chat_completion(&texto, &profile, backend.tokenizer())
            .unwrap_or_else(|e| {
                eprintln!("{peticion}: prepare: {e:?}");
                std::process::exit(1);
            });
        let prompt = soso_llm_core::conversation::render_messages(
            &prepared.input,
            &profile,
            backend.tokenizer(),
        )
        .expect("render");
        let t = Instant::now();
        let mut obs = Mostrar { ids: Vec::new() };
        // Sin `begin_request`: la caché se reaprovecha entre peticiones (T81).
        let (_, rep) = backend
            .generate_observed(&prompt, &prepared, &mut obs)
            .unwrap_or_else(|_| {
                eprintln!("{peticion}: la inferencia falló");
                std::process::exit(1);
            });
        println!(
            "=== {peticion} · prompt {} tokens ({} en bloque) · {} generados · {:.1} s · parada {:?}",
            prompt.len(),
            backend.rt.tokens_en_bloque,
            obs.ids.len(),
            t.elapsed().as_secs_f64(),
            rep.stop
        );
        println!("{}", backend.tokenizer().decode(&obs.ids));
        println!("=== ids {:?}", obs.ids);
    }
}
