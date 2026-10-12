//! Runner host: ejecuta un modelo .som convertido a velocidad nativa.
//! Para depurar calidad/numérica sin QEMU:
//!   cargo run --release -p soso-llm-core --features std --example hostrun -- \
//!     target/tinyllama-model "The capital of France is" 8 [temp] [top_p] [seed]

use soso_llm_core::plan::{MemoryPlanConfig, MemoryPreset, MemSnapshot, ResourcePlanner};
use soso_llm_core::runtime::Runtime;
use soso_llm_core::sample::Sampler;
use soso_llm_core::source::{FileMapper, MappedShard, MmapTensorSource};
use soso_llm_core::ThreadStagedSource;
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

enum RunSource {
    Plain(MmapTensorSource<StdMapper>),
    Staged(ThreadStagedSource<StdMapper>),
}

impl soso_llm_core::layer::TensorSource for RunSource {
    fn load_f32(&mut self, name: &str, out: &mut [f32]) -> Result<(), ()> {
        match self {
            Self::Plain(s) => s.load_f32(name, out),
            Self::Staged(s) => s.load_f32(name, out),
        }
    }

    fn load_f32_range(&mut self, name: &str, elem_off: usize, out: &mut [f32]) -> Result<(), ()> {
        match self {
            Self::Plain(s) => s.load_f32_range(name, elem_off, out),
            Self::Staged(s) => s.load_f32_range(name, elem_off, out),
        }
    }

    fn tensor_view(&mut self, name: &str) -> Result<soso_llm_core::layer::TensorView<'_>, ()> {
        match self {
            Self::Plain(s) => s.tensor_view(name),
            Self::Staged(s) => s.tensor_view(name),
        }
    }

    fn prefetch_shards(&mut self, shards: &[String]) {
        match self {
            Self::Plain(s) => s.prefetch_shards(shards),
            Self::Staged(s) => s.prefetch_shards(shards),
        }
    }

    fn kick_prefetch_shards(&mut self, shards: &[String]) {
        match self {
            Self::Plain(s) => s.kick_prefetch_shards(shards),
            Self::Staged(s) => s.kick_prefetch_shards(shards),
        }
    }

    fn wait_prefetch(&mut self) {
        match self {
            Self::Plain(s) => s.wait_prefetch(),
            Self::Staged(s) => s.wait_prefetch(),
        }
    }

    fn kick_moe_prefetch(&mut self, shards: &[String]) {
        match self {
            Self::Plain(s) => s.kick_moe_prefetch(shards),
            Self::Staged(s) => s.kick_moe_prefetch(shards),
        }
    }

    fn wait_moe_prefetch(&mut self) {
        match self {
            Self::Plain(s) => s.wait_moe_prefetch(),
            Self::Staged(s) => s.wait_moe_prefetch(),
        }
    }

    fn release_shards_except(&mut self, keep: &[String]) {
        match self {
            Self::Plain(s) => s.release_shards_except(keep),
            Self::Staged(s) => s.release_shards_except(keep),
        }
    }

    fn prefetch_embed_row(&mut self, token: u32, hidden: usize) {
        match self {
            Self::Plain(s) => s.prefetch_embed_row(token, hidden),
            Self::Staged(s) => s.prefetch_embed_row(token, hidden),
        }
    }
}

/// Reloj real: el planificador replanifica según el tiempo, y con un reloj
/// clavado a 0 no se ejercita ese camino.
fn reloj_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn mem_snapshot() -> MemSnapshot {
    let free_mb: u64 = std::env::var("SOSO_FREE_MB")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1600);
    let free_frames = free_mb * 1024 * 1024 / 4096;
    MemSnapshot {
        total_frames: free_frames.saturating_mul(2),
        free_frames,
        reclaimable_frames: 0,
    }
}

fn memory_plan_config() -> MemoryPlanConfig {
    let preset = match std::env::var("SOSO_MEM_PRESET").as_deref() {
        Ok("tight") => MemoryPreset::Tight,
        Ok("balanced") => MemoryPreset::Balanced,
        Ok("max-pin") | Ok("max_pin") => MemoryPreset::MaxPin,
        _ => MemoryPreset::Auto,
    };
    let trunk_frac_pct = std::env::var("SOSO_TRUNK_FRAC")
        .ok()
        .and_then(|v| v.parse().ok());
    MemoryPlanConfig {
        preset,
        trunk_frac_pct,
        ring_slots: 2,
    }
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
    let base_source = MmapTensorSource::new(format!("{dir}/shards"), index.clone(), StdMapper);
    let mut source = if std::env::var("SOSO_SYNC_STAGING").is_ok() {
        RunSource::Plain(base_source.with_sync_prefetch())
    } else {
        RunSource::Staged(ThreadStagedSource::new(base_source))
    };

    // `SOSO_PLANNER=1` enciende el `ResourcePlanner` y el decode planificado,
    // que es lo que usa `soso-llm` en la máquina. Sin esto el arnés de host no
    // ejercita `touch_moe_experts` ni el prefetch MoE, y un fallo que sólo
    // aparece con planificador obliga a depurarlo dentro de QEMU.
    if std::env::var("SOSO_PLANNER").is_ok() {
        let mem = mem_snapshot();
        let cfg = memory_plan_config();
        let planner = ResourcePlanner::with_config(
            &rt.manifest, &index, mem, 0, false, cfg,
        );
        eprintln!(
            "planificador: activado — {} (libre ~{} MiB)",
            planner.memory_plan_summary(),
            mem.free_bytes() / 1024 / 1024,
        );
        rt.set_planner(planner);
    }

    // "@bos" = prompt de un solo token BOS (RoPE identidad en pos 0)
    let prompt_tokens = if prompt == "@bos" {
        vec![1u32]
    } else if std::env::var("SOSO_CHAT").is_ok() {
        // Con la plantilla del modelo, que es como lo va a usar `ask`. Sin esto
        // un modelo de chat contesta ensalada de palabras y no hay forma de
        // juzgar la calidad sin arrancar QEMU o la placa.
        let plantilla = std::env::var("SOSO_PLANTILLA")
            .unwrap_or_else(|_| rt.manifest.chat_template.clone());
        if plantilla.is_empty() {
            eprintln!("SOSO_CHAT: el modelo no trae plantilla; prompt crudo");
        } else {
            eprintln!("plantilla: {plantilla:?}");
        }
        soso_llm_core::chat::render(&plantilla, &prompt, &tokenizer)
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
    if std::env::var("SOSO_PLANNER").is_ok() {
        // El mismo camino que usa `soso-llm` en la máquina.
        let mut gpu: Option<&mut dyn soso_llm_core::gpu::GpuDispatch> = None;
        let r = rt.generate_stream_planned(
            &mut source,
            &prompt_tokens,
            max_new,
            tokenizer.eos(),
            &mut sampler,
            |t| {
                let s = dec.push(&tokenizer, t);
                if !s.is_empty() {
                    print!("{s}");
                    let _ = std::io::stdout().flush();
                }
            },
            None,
            &mut gpu,
            reloj_ms,
            mem_snapshot,
        );
        let dt = t0.elapsed().as_secs_f64();
        match r {
            Ok(t) => {
                let generated = t.len().saturating_sub(prompt_tokens.len());
                eprintln!("\nplanificado: {} tokens ({} nuevos)", t.len(), generated);
                if generated > 0 && dt > 0.0 {
                    eprintln!(
                        "rendimiento: {:.2} tok/s ({:.0} ms/token)",
                        generated as f64 / dt,
                        dt * 1000.0 / generated as f64,
                    );
                }
                if let Some(pl) = rt.planner.as_ref() {
                    let st = pl.stats();
                    eprintln!(
                        "tronco: {} hits, {} misses, {} KiB | MoE: {} resident, {} JIT, {} fríos | staging {} ms",
                        st.trunk_hits,
                        st.trunk_misses,
                        st.trunk_bytes_read / 1024,
                        st.moe_resident_hits,
                        st.moe_jit_hits,
                        st.moe_misses,
                        st.stage_wait_ms,
                    );
                }
                if std::env::var("SOSO_MOE_TRACE").is_ok() {
                    if let Some(pl) = rt.planner.as_ref() {
                        let path = std::env::var("SOSO_MOE_TRACE_OUT")
                            .unwrap_or_else(|_| "target/moe_trace.bin".into());
                        let mut buf = Vec::new();
                        for &(layer, expert) in pl.moe_trace() {
                            buf.extend_from_slice(&layer.to_le_bytes());
                            buf.extend_from_slice(&expert.to_le_bytes());
                        }
                        if let Err(e) = std::fs::write(&path, &buf) {
                            eprintln!("moe trace: no se pudo escribir {path}: {e}");
                        } else {
                            eprintln!(
                                "moe trace: {} entradas → {} (sim: python3 tools/sim-moe-cache.py {path})",
                                pl.moe_trace().len(),
                                path
                            );
                        }
                    }
                }
            }
            Err(()) => {
                eprintln!("\nplanificado: FALLÓ (mismo Err(()) que «inferencia falló» en soso)");
                std::process::exit(1);
            }
        }
        return;
    }

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
