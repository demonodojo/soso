//! Tiempo por capa en decode (ms, media de N pasos): `perfil_capas <dir-modelo> <id,id,…> [pasos]`. `HILOS=N`, `SOSO_CUDA=1`.
use std::time::Instant;

use soso_llm_api::{HostPool, OwnedMapper};
use soso_llm_core::parallel::RowParallel;
use soso_llm_core::runtime::Runtime;
use soso_llm_core::source::MmapTensorSource;
use sosomodel::{Manifest, TensorIndex};

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let dir = &a[0];
    let ids: Vec<u32> = a[1].split(',').map(|s| s.parse().unwrap()).collect();
    let pasos: usize = a.get(2).map(|s| s.parse().unwrap()).unwrap_or(16);
    let manifest = Manifest::parse(&std::fs::read(format!("{dir}/manifest.som")).unwrap()).unwrap();
    let nl = manifest.num_layers;
    let index = TensorIndex::parse(&std::fs::read(format!("{dir}/index.som")).unwrap()).unwrap();
    let mut rt = Runtime::new(manifest, index.clone(), 0, 0);
    let mut source = MmapTensorSource::new(format!("{dir}/shards"), index, OwnedMapper::default());
    let pool = std::env::var("HILOS").ok().and_then(|v| v.parse::<usize>().ok()).filter(|&n| n > 1).map(HostPool::new);
    let par: Option<&dyn RowParallel> = pool.as_ref().map(|p| p as &dyn RowParallel);
    let mut cuda = if std::env::var("SOSO_CUDA").is_ok() { Some(soso_llm_api::cuda::CudaGpu::nueva().expect("CUDA")) } else { None };
    let mut gpu: Option<&mut dyn soso_llm_core::gpu::GpuDispatch> = cuda.as_mut().map(|g| g as &mut dyn soso_llm_core::gpu::GpuDispatch);
    fn reloj_ns() -> u64 {
        use std::sync::OnceLock;
        static T0: OnceLock<Instant> = OnceLock::new();
        T0.get_or_init(Instant::now).elapsed().as_nanos() as u64 + 1
    }
    if std::env::var("SOSO_SIN_LOTE").is_ok() {
        soso_llm_core::layer::LOTE_ACTIVO.store(false, std::sync::atomic::Ordering::Relaxed);
    }
    rt.reset_sequence();
    rt.prefill_prompt(&mut source, &ids, par, &mut gpu, None).unwrap();
    let mut tok = Runtime::argmax(rt.logits_gpu(&mut source, par, &mut gpu).unwrap());
    soso_llm_core::layer::perfil_activar(Some(reloj_ns));
    let mut t_capa = vec![0f64; nl as usize];
    let (mut t_emb, mut t_log) = (0f64, 0f64);
    let ventana = 25usize;
    let mut t_vent = Instant::now();
    for paso in 0..pasos {
        if paso > 0 && paso % ventana == 0 {
            let f = soso_llm_core::layer::perfil_fases_ns();
            let ms = t_vent.elapsed().as_secs_f64() * 1e3 / ventana as f64;
            let fases: Vec<String> = soso_llm_core::layer::NOMBRES_FASES.iter().zip(f).map(|(n, v)| format!("{n} {:.1}", v as f64 / 1e6 / ventana as f64)).collect();
            eprintln!("  pasos {}..{}: {ms:.1} ms/token | {}", paso - ventana, paso, fases.join(", "));
            soso_llm_core::layer::perfil_activar(Some(reloj_ns));
            t_vent = Instant::now();
        }
        let t = Instant::now();
        rt.embed_token(tok, &mut source).unwrap();
        t_emb += t.elapsed().as_secs_f64();
        for l in 0..nl {
            let t = Instant::now();
            rt.forward_layers_range(l, l + 1, &mut source, par, &mut gpu).unwrap();
            t_capa[l as usize] += t.elapsed().as_secs_f64();
        }
        rt.advance_pos();
        let t = Instant::now();
        tok = Runtime::argmax(rt.logits_gpu(&mut source, par, &mut gpu).unwrap());
        t_log += t.elapsed().as_secs_f64();
    }
    let k = pasos as f64 / 1e3;
    let tot: f64 = t_capa.iter().sum::<f64>() / k;
    println!("embed {:.2} ms, capas {:.1} ms, logits {:.2} ms", t_emb / k, tot, t_log / k);
    let f = soso_llm_core::layer::perfil_fases_ns();
    println!("atención/GDN {:.1} ms, FFN/MoE {:.1} ms por token", f[1] as f64 / 1e6 / pasos as f64, f[3] as f64 / 1e6 / pasos as f64);
    for (n, v) in soso_llm_core::layer::NOMBRES_FASES.iter().zip(f) { println!("  fase {n:<12} {:7.1} ms/token", v as f64 / 1e6 / pasos as f64); }
    for (l, t) in t_capa.iter().enumerate().take(3) {
        println!("capa {l:2}: {:6.2} ms", t / k);
    }
}

// (las fases se imprimen al final)
