//! Tiempo de cada matriz de una capa multiplicada por un bloque de `n` vectores, en GPU y en CPU:
//! `perfil_matmul <dir-modelo> <prefijo p.ej. L00> [n] [reps]`. `HILOS=N`, `SOSO_CUDA=1`.
use std::time::Instant;

use soso_llm_api::{HostPool, OwnedMapper};
use soso_llm_core::layer::{matvec_view_batch_par, TensorSource};
use soso_llm_core::parallel::{RowParallel, Sequential};
use soso_llm_core::source::MmapTensorSource;
use sosomodel::TensorIndex;

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let (dir, pre) = (&a[0], &a[1]);
    let n: usize = a.get(2).map(|s| s.parse().unwrap()).unwrap_or(256);
    let reps: usize = a.get(3).map(|s| s.parse().unwrap()).unwrap_or(5);
    let index = TensorIndex::parse(&std::fs::read(format!("{dir}/index.som")).unwrap()).unwrap();
    let entries: Vec<_> = index.entries.iter().filter(|e| e.name.starts_with(pre.as_str()) && e.shape.len() == 2 && !e.name.contains(".E") && !e.name.contains(".S")).cloned().collect();
    let mut source = MmapTensorSource::new(format!("{dir}/shards"), index, OwnedMapper::default());
    let pool = std::env::var("HILOS").ok().and_then(|v| v.parse::<usize>().ok()).filter(|&n| n > 1).map(HostPool::new);
    let par: &dyn RowParallel = match pool.as_ref() { Some(p) => p, None => &Sequential };
    let mut cuda = if std::env::var("SOSO_CUDA").is_ok() { Some(soso_llm_api::cuda::CudaGpu::nueva().expect("CUDA")) } else { None };
    for e in entries {
        let (rows, cols) = (e.shape[0] as usize, e.shape[1] as usize);
        let x: Vec<f32> = (0..n * cols).map(|i| ((i * 7 % 13) as f32 - 6.0) * 0.1).collect();
        let mut o = vec![0f32; n * rows];
        let v = source.tensor_view(&e.name).unwrap();
        let mut q8 = vec![0u8; soso_llm_core::q8k::q8k_buffer_len(n, cols.max(256).next_multiple_of(256))];
        matvec_view_batch_par(&v, rows, cols, &x, n, &mut o, par, Some(&mut q8)).unwrap();
        let t = Instant::now();
        for _ in 0..reps { matvec_view_batch_par(&v, rows, cols, &x, n, &mut o, par, Some(&mut q8)).unwrap(); }
        let cpu = t.elapsed().as_secs_f64() * 1e3 / reps as f64;
        let mut gpu_ms = f64::NAN;
        if let Some(g) = cuda.as_mut() {
            let gd: &mut dyn soso_llm_core::gpu::GpuDispatch = g;
            if soso_llm_core::gpu::try_gpu_matmul(gd, &e.name, &v, rows, cols, n, &x, &mut o) == Ok(true) {
                let t = Instant::now();
                for _ in 0..reps { soso_llm_core::gpu::try_gpu_matmul(gd, &e.name, &v, rows, cols, n, &x, &mut o).unwrap(); }
                gpu_ms = t.elapsed().as_secs_f64() * 1e3 / reps as f64;
            }
        }
        println!("{:24} dt={:2} {rows:6}x{cols:5} n={n}  CPU {cpu:8.2} ms   GPU {gpu_ms:8.2} ms", e.name, e.dtype);
    }
}
