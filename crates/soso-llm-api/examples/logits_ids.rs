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

    // `SOSO_CUDA=1`: kernels CUDA del host para los matmul y la cabeza de logits.
    let mut cuda = if matches!(std::env::var("SOSO_CUDA").as_deref(), Ok("1")) {
        Some(soso_llm_api::cuda::CudaGpu::nueva().expect("CUDA"))
    } else {
        None
    };
    let mut gpu: Option<&mut dyn soso_llm_core::gpu::GpuDispatch> =
        cuda.as_mut().map(|g| g as &mut dyn soso_llm_core::gpu::GpuDispatch);

    fn reloj_ns() -> u64 {
        use std::sync::OnceLock;
        static T0: OnceLock<Instant> = OnceLock::new();
        T0.get_or_init(Instant::now).elapsed().as_nanos() as u64 + 1
    }
    let perfil = std::env::var("PERFIL").is_ok();
    if perfil {
        soso_llm_core::layer::perfil_activar(Some(reloj_ns));
    }
    if std::env::var("SOSO_SIN_PARTIR").is_ok() {
        soso_llm_core::layer::SIN_PARTIR.store(true, std::sync::atomic::Ordering::Relaxed);
    }
    if std::env::var("SOSO_IQ_F32").is_ok() {
        soso_llm_core::gemm::iq_q8k_activar(false);
    }
    if std::env::var("PREPARAR_DECODE").is_ok() {
        rt.decode_gpu_preparar(&mut source, &mut gpu);
    }
    // `REPETIR=N`: N prefills seguidos (el primero paga mmap y CRC de cada shard; los siguientes miden el régimen estable).
    for r in 1..std::env::var("REPETIR").ok().and_then(|v| v.parse::<usize>().ok()).unwrap_or(1) {
        let t = Instant::now();
        rt.reset_sequence();
        rt.prefill_prompt(&mut source, &ids, par, &mut gpu, None).expect("prefill");
        eprintln!("prefill previo {r}: {:.2?}", t.elapsed());
    }
    if perfil {
        soso_llm_core::layer::perfil_activar(Some(reloj_ns));
    }
    let t0 = Instant::now();
    rt.reset_sequence();
    rt.prefill_prompt(&mut source, &ids, par, &mut gpu, None).expect("prefill");
    let t_prefill = t0.elapsed();
    let t1 = Instant::now();
    let logits = rt.logits_gpu(&mut source, par, &mut gpu).expect("logits").to_vec();
    eprintln!("prefill {} token(s): {:.2?}; logits: {:.2?}", ids.len(), t_prefill, t1.elapsed());

    // `DECODIFICAR=N`: N pasos de decodificación (un token + logits) tras el prefill; imprime ms por paso.
    if let Some(pasos) = std::env::var("DECODIFICAR")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
    {
        let mut tok = Runtime::argmax(&logits);
        let mut generados = alloc_vec_ids(tok);
        let t = Instant::now();
        let (mut t_fwd, mut t_log) = (0.0f64, 0.0f64);
        let mut por_paso: Vec<f64> = Vec::new();
        for _ in 0..pasos {
            let a = Instant::now();
            if std::env::var("DECODIFICAR_GPU").is_ok() {
                rt.embed_token(tok, &mut source).expect("embed");
                assert!(rt.forward_token_gpu(&mut source, par, &mut gpu).expect("decode gpu"), "decode GPU no disponible");
            } else {
                rt.prefill_prompt(&mut source, &[tok], par, &mut gpu, None).expect("paso");
            }
            t_fwd += a.elapsed().as_secs_f64();
            let b = Instant::now();
            let l = rt.logits_gpu(&mut source, par, &mut gpu).expect("logits");
            tok = Runtime::argmax(l);
            generados.push(tok);
            t_log += b.elapsed().as_secs_f64();
            por_paso.push(a.elapsed().as_secs_f64() * 1e3);
        }
        let tot = t.elapsed().as_secs_f64();
        rt.decode_gpu_cerrar(&mut gpu);
        eprintln!("ids generados: {:?}", generados);
        let mut o = por_paso.clone();
        o.sort_by(|a, b| a.partial_cmp(b).unwrap());
        eprintln!("ms por paso: primero {:.0}, mediana {:.1}, mín {:.1}", por_paso[0], o[o.len() / 2], o[0]);
        eprintln!(
            "decodificar {pasos}: {:.2} ms/token ({:.0} tok/s); capas {:.2} ms, logits {:.2} ms",
            tot * 1e3 / pasos as f64,
            pasos as f64 / tot,
            t_fwd * 1e3 / pasos as f64,
            t_log * 1e3 / pasos as f64
        );
    }
    if perfil {
        let f = soso_llm_core::layer::perfil_fases_ns();
        let total: u64 = f.iter().sum::<u64>().max(1);
        for (n, v) in soso_llm_core::layer::NOMBRES_FASES.iter().zip(f) {
            eprintln!("  fase {n:<12} {:>8.2} s  {:>4.1} %", v as f64 / 1e9, v as f64 * 100.0 / total as f64);
        }
    }
    if let Some(g) = cuda.as_mut() {
        let st = soso_llm_core::gpu::GpuDispatch::stats(g);
        eprintln!(
            "  gpu: {} lanzamientos, {} residentes, {} MiB en VRAM, {:.0} ms de CPU en moe_capa",
            st.launches,
            g.residentes(),
            g.vram_usada() >> 20,
            st.total_ns as f64 / 1e6
        );
        let (t_cp, mib, t_gpu) = g.flujo_diagnostico();
        if mib > 0.0 {
            eprintln!("  flujo: {t_cp:.0} ms copiando {mib:.0} MiB ({:.1} GB/s), {t_gpu:.0} ms de GPU antes del router", mib / 1024.0 / (t_cp / 1000.0).max(1e-9));
        }
        let p = g.perfil_capa();
        let total: f64 = p.iter().map(|(_, ms)| ms).sum::<f64>().max(1e-9);
        for (n, ms) in p {
            eprintln!("  gpu {n:<16} {:>8.1} ms  {:>4.1} %", ms, ms * 100.0 / total);
        }
    }
    let max = logits.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let suma: f64 = logits.iter().map(|&l| ((l - max) as f64).exp()).sum();
    let mut orden: Vec<usize> = (0..logits.len()).collect();
    orden.sort_by(|&a, &b| logits[b].partial_cmp(&logits[a]).unwrap_or(std::cmp::Ordering::Equal));
    for &i in orden.iter().take(top) {
        println!("{} {:.5} {:.6}", i, logits[i], ((logits[i] - max) as f64).exp() / suma);
    }
}

fn alloc_vec_ids(primero: u32) -> Vec<u32> {
    vec![primero]
}
