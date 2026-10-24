//! Paridad capa a capa con llama.cpp: embebe **un** token y vuelca el estado oculto tras cada capa
//! (`DIR/l_out-N.f32`, `DIR/inp_embd.f32`), el mismo formato que `tools/iq-oraculo/volcar_capas.c`.
//!
//! ```sh
//! cargo run --release -p soso-llm-api --features std --example volcar_capas -- <dir-modelo> <id> <dir-salida> [n_capas]
//! ```
use soso_llm_api::{HostPool, OwnedMapper};
use soso_llm_core::parallel::RowParallel;
use soso_llm_core::runtime::Runtime;
use soso_llm_core::source::MmapTensorSource;
use sosomodel::{Manifest, TensorIndex};

fn volcar(ruta: String, v: &[f32]) {
    let b: Vec<u8> = v.iter().flat_map(|x| x.to_le_bytes()).collect();
    std::fs::write(ruta, b).expect("escribir");
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 3 {
        eprintln!("uso: volcar_capas <dir-modelo> <id> <dir-salida> [n_capas]");
        std::process::exit(2);
    }
    let (dir, id, out) = (&args[0], args[1].parse::<u32>().expect("id"), &args[2]);
    std::fs::create_dir_all(out).expect("dir");
    let manifest = Manifest::parse(&std::fs::read(format!("{dir}/manifest.som")).expect("manifest")).expect("manifest válido");
    let n = args.get(3).map(|s| s.parse::<u32>().expect("n")).unwrap_or(manifest.num_layers);
    let index = TensorIndex::parse(&std::fs::read(format!("{dir}/index.som")).expect("index")).expect("index válido");
    let mut rt = Runtime::new(manifest, index.clone(), 0, 0);
    rt.validate_shapes().expect("formas coherentes");
    let mut source = MmapTensorSource::new(format!("{dir}/shards"), index, OwnedMapper::default());
    let pool = std::env::var("HILOS").ok().and_then(|v| v.parse::<usize>().ok()).filter(|&n| n > 1).map(HostPool::new);
    let par: Option<&dyn RowParallel> = pool.as_ref().map(|p| p as &dyn RowParallel);
    let mut gpu: Option<&mut dyn soso_llm_core::gpu::GpuDispatch> = None;
    rt.reset_sequence();
    rt.embed_token(id, &mut source).expect("embed");
    volcar(format!("{out}/inp_embd.f32"), rt.hidden_slice());
    for l in 0..n {
        rt.forward_layers_range(l, l + 1, &mut source, par, &mut gpu).expect("capa");
        volcar(format!("{out}/l_out-{l}.f32"), rt.hidden_slice());
        eprintln!("capa {l} ok");
    }
}
