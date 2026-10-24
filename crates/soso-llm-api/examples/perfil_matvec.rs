//! Tiempo de cada matvec de una capa con los pesos reales: `perfil_matvec <dir-modelo> <prefijo p.ej. L00> [reps]`. `HILOS=N`.
use std::time::Instant;

use soso_llm_api::{HostPool, OwnedMapper};
use soso_llm_core::layer::{matvec_view_par, TensorSource};
use soso_llm_core::parallel::{RowParallel, Sequential};
use soso_llm_core::source::MmapTensorSource;
use sosomodel::TensorIndex;

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let (dir, pre) = (&a[0], &a[1]);
    let reps: usize = a.get(2).map(|s| s.parse().unwrap()).unwrap_or(30);
    let index = TensorIndex::parse(&std::fs::read(format!("{dir}/index.som")).unwrap()).unwrap();
    let entries: Vec<_> = index.entries.iter().filter(|e| e.name.starts_with(pre.as_str()) && e.shape.len() == 2 && !e.name.contains(".E") || e.name.starts_with(&format!("{pre}.E00.")) ).cloned().collect();
    let mut source = MmapTensorSource::new(format!("{dir}/shards"), index, OwnedMapper::default());
    let pool = std::env::var("HILOS").ok().and_then(|v| v.parse::<usize>().ok()).filter(|&n| n > 1).map(HostPool::new);
    let par: &dyn RowParallel = match pool.as_ref() { Some(p) => p, None => &Sequential };
    let mut tot = 0.0;
    for e in entries {
        let (rows, cols) = (e.shape[0] as usize, e.shape[1] as usize);
        let x: Vec<f32> = (0..cols).map(|i| ((i * 7 % 13) as f32 - 6.0) * 0.1).collect();
        let mut o = vec![0f32; rows];
        let v = source.tensor_view(&e.name).unwrap();
        matvec_view_par(&v, rows, cols, &x, &mut o, par).unwrap();
        let t = Instant::now();
        for _ in 0..reps { matvec_view_par(&v, rows, cols, &x, &mut o, par).unwrap(); }
        let ms = t.elapsed().as_secs_f64() * 1e3 / reps as f64;
        tot += ms;
        println!("{:28} dt={:2} {rows:6}x{cols:5} {:6.1} MB {ms:7.3} ms {:6.1} GB/s", e.name, e.dtype, v.bytes.len() as f64 / 1e6, v.bytes.len() as f64 / ms / 1e6);
    }
    println!("suma {tot:.2} ms");
}
