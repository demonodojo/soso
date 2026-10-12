//! Logits de la última posición para una lista de **ids** de token, sin tokenizer ni plantilla. Es la
//! sonda de paridad numérica contra otro runtime (llama.cpp): mismos ids → mismos logits.
//!
//! ```sh
//! cargo run --release -p soso-llm-api --features std --example logits_ids -- \
//!   target/gemma4-12b-model 2,9259 [--top 10]
//! ```
//! `HILOS=N` reparte el cálculo entre N hilos. Imprime una línea por candidato: `id logit prob`
//! (la probabilidad es el softmax de **todos** los logits, ya con el límite blando aplicado).

use std::time::Instant;

use soso_llm_api::{HostPool, OwnedMapper};
use soso_llm_core::parallel::RowParallel;
use soso_llm_core::runtime::Runtime;
use soso_llm_core::source::MmapTensorSource;
use sosomodel::{Manifest, TensorIndex};

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 2 {
        eprintln!("uso: logits_ids <dir-modelo> <id,id,…> [--top N]");
        std::process::exit(2);
    }
    let dir = args.remove(0);
    let ids: Vec<u32> = args.remove(0).split(',').map(|s| s.trim().parse().expect("id")).collect();
    let mut top = 10usize;
    if args.first().map(String::as_str) == Some("--top") {
        args.remove(0);
        top = args.remove(0).parse().expect("top");
    }
    let manifest = Manifest::parse(&std::fs::read(format!("{dir}/manifest.som")).expect("manifest")).expect("manifest válido");
    let index_bytes = std::fs::read(format!("{dir}/index.som")).expect("index");
    let index = TensorIndex::parse(&index_bytes).expect("index válido");
    let mut rt = Runtime::new(manifest, index.clone(), 0, 0);
    rt.validate_shapes().expect("formas coherentes");
    let mut source = MmapTensorSource::new(format!("{dir}/shards"), index, OwnedMapper::default());
    let pool = std::env::var("HILOS").ok().and_then(|v| v.parse::<usize>().ok()).filter(|&n| n > 1).map(HostPool::new);
    let par: Option<&dyn RowParallel> = pool.as_ref().map(|p| p as &dyn RowParallel);

    let t0 = Instant::now();
    rt.reset_sequence();
    rt.prefill_prompt(&mut source, &ids, par, &mut None, None).expect("prefill");
    let t_prefill = t0.elapsed();
    let t1 = Instant::now();
    let logits = rt.logits_par(&mut source, par).expect("logits").to_vec();
    eprintln!("prefill {} token(s): {:.2?}; logits: {:.2?}", ids.len(), t_prefill, t1.elapsed());

    let max = logits.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let suma: f64 = logits.iter().map(|&l| ((l - max) as f64).exp()).sum();
    let mut orden: Vec<usize> = (0..logits.len()).collect();
    orden.sort_by(|&a, &b| logits[b].partial_cmp(&logits[a]).unwrap_or(std::cmp::Ordering::Equal));
    for &i in orden.iter().take(top) {
        println!("{} {:.5} {:.6}", i, logits[i], ((logits[i] - max) as f64).exp() / suma);
    }
}
