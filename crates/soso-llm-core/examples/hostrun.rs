//! Runner host: ejecuta un modelo .som convertido a velocidad nativa.
//! Para depurar calidad/numérica sin QEMU:
//!   cargo run --release -p soso-llm-core --features std --example hostrun -- \
//!     target/tinyllama-model "The capital of France is" 8 [temp] [top_p] [seed]

use soso_llm_core::runtime::Runtime;
use soso_llm_core::sample::Sampler;
use soso_llm_core::source::{FileMapper, MappedShard, MmapTensorSource};
use soso_llm_core::tokenizer::{StreamDecoder, Tokenizer};
use sosomodel::index::TensorIndex;
use sosomodel::manifest::Manifest;
use std::io::Write;

/// FileMapper host: lee el fichero completo y filtra el puntero (leak
/// consciente: proceso corto).
struct StdMapper;

impl FileMapper for StdMapper {
    fn map_file(&mut self, path: &str) -> Result<MappedShard, ()> {
        let data = std::fs::read(path).map_err(|_| ())?;
        let len = data.len();
        let ptr = Box::leak(data.into_boxed_slice()).as_ptr();
        Ok(MappedShard {
            addr: ptr as u64,
            len,
        })
    }

    fn unmap_file(&mut self, _shard: &MappedShard) {}
}

fn main() {
    let mut args = std::env::args().skip(1);
    let dir = args.next().unwrap_or_else(|| "target/tinyllama-model".into());
    let prompt = args.next().unwrap_or_else(|| "hola".into());
    let max_new: usize = args.next().and_then(|v| v.parse().ok()).unwrap_or(8);
    let temp: f32 = args.next().and_then(|v| v.parse().ok()).unwrap_or(0.0);
    let top_p: f32 = args.next().and_then(|v| v.parse().ok()).unwrap_or(0.9);
    let seed: u64 = args.next().and_then(|v| v.parse().ok()).unwrap_or(42);

    let manifest = Manifest::parse(&std::fs::read(format!("{dir}/manifest.som")).unwrap()).unwrap();
    let index = TensorIndex::parse(&std::fs::read(format!("{dir}/index.som")).unwrap()).unwrap();
    let tokenizer = match std::fs::read(format!("{dir}/tokenizer.som")) {
        Ok(d) => Tokenizer::parse(&d).expect("tokenizer.som inválido"),
        Err(_) => Tokenizer::byte_level(),
    };

    eprintln!(
        "modelo: {} ({} capas, hidden={}, heads={}/{}, vocab={})",
        manifest.name,
        manifest.num_layers,
        manifest.hidden_dim,
        manifest.num_heads,
        manifest.num_kv_heads,
        manifest.vocab_size
    );

    let mut rt = Runtime::new(manifest, index.clone(), 0, 0);
    rt.validate_shapes().expect("shapes inválidas");
    let mut source = MmapTensorSource::new(format!("{dir}/shards"), index, StdMapper);
    source.async_staging = true;

    // "@bos" = prompt de un solo token BOS (RoPE identidad en pos 0)
    let prompt_tokens = if prompt == "@bos" {
        vec![1u32]
    } else {
        tokenizer.encode(&prompt)
    };
    eprintln!("prompt tokens: {prompt_tokens:?}");

    if std::env::var("SOSO_DEBUG").is_ok() {
        let stats = |v: &[f32]| {
            let n = v.len() as f32;
            let mean = v.iter().sum::<f32>() / n;
            let rms = (v.iter().map(|x| x * x).sum::<f32>() / n).sqrt();
            let nan = v.iter().filter(|x| !x.is_finite()).count();
            (mean, rms, nan)
        };
        rt.reset_sequence();
        for (i, &t) in prompt_tokens.iter().enumerate() {
            rt.embed_token(t, &mut source).unwrap();
            let (m, r, nan) = stats(&rt.hidden);
            eprintln!("tok {i} ({t}) embed: mean={m:.5} rms={r:.5} nan={nan}");
            rt.forward_step(&mut source).unwrap();
            let (m, r, nan) = stats(&rt.hidden);
            eprintln!("tok {i} ({t}) tras {} capas: mean={m:.5} rms={r:.5} nan={nan}", rt.manifest.num_layers);
        }
        let logits = rt.logits(&mut source).unwrap().to_vec();
        let (m, r, nan) = stats(&logits);
        let mut idx: Vec<usize> = (0..logits.len()).collect();
        idx.sort_unstable_by(|&a, &b| logits[b].partial_cmp(&logits[a]).unwrap_or(std::cmp::Ordering::Equal));
        eprintln!("logits: mean={m:.4} rms={r:.4} nan={nan}");
        for &i in &idx[..8] {
            eprintln!("  top: token {i} logit={:.4}", logits[i]);
        }
        return;
    }

    let mut sampler = Sampler::new(temp, top_p, seed);
    let mut dec = StreamDecoder::new();
    let t0 = std::time::Instant::now();
    let tokens = rt
        .generate_stream(
            &mut source,
            &prompt_tokens,
            max_new,
            tokenizer.eos(),
            &mut sampler,
            |t| {
                let s = dec.push(&tokenizer, t);
                print!("{s}");
                std::io::stdout().flush().ok();
            },
        )
        .expect("generate falló");
    println!("{}", dec.finish());
    let dt = t0.elapsed().as_secs_f32();
    let generated = tokens.len() - prompt_tokens.len();
    eprintln!(
        "{} tokens ({} nuevos) en {dt:.1}s ({:.2} tok/s)",
        tokens.len(),
        generated,
        tokens.len() as f32 / dt
    );
    eprintln!("tokens: {:?}", &tokens[prompt_tokens.len()..]);
}
