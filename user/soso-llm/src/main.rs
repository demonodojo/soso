//! Inferencia LLM en userspace de soso.

#![no_std]
#![no_main]

extern crate alloc;

mod pool;

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use libsoso::{println, sys};
use pool::ThreadPool;
use soso_abi::{self as abi, O_RDONLY};
use sosomodel::index::TensorIndex;
use sosomodel::manifest::Manifest;
use soso_llm_core::parallel::RowParallel;
use soso_llm_core::runtime::Runtime;
use soso_llm_core::sample::Sampler;
use soso_llm_core::source::{FileMapper, MappedShard, MmapTensorSource};
use soso_llm_core::tokenizer::{StreamDecoder, Tokenizer};

libsoso::entry!(main);

struct SyscallMapper;

impl FileMapper for SyscallMapper {
    fn map_file(&mut self, path: &str) -> Result<MappedShard, ()> {
        let fd = sys::open(path, O_RDONLY);
        if fd < 0 {
            return Err(());
        }
        let mut st = abi::Stat::default();
        if sys::stat(path, &mut st) < 0 {
            sys::close(fd as u64);
            return Err(());
        }
        let size = st.size as usize;
        let map = sys::mmap(0, size as u64, fd as u64, 0);
        sys::close(fd as u64);
        if map < 0 {
            return Err(());
        }
        let ptr = map as *const u8;
        let _ = unsafe { core::ptr::read_volatile(ptr) };
        Ok(MappedShard {
            addr: map as u64,
            len: size,
        })
    }

    fn unmap_file(&mut self, shard: &MappedShard) {
        let aligned = shard.len.next_multiple_of(4096);
        let _ = sys::munmap(shard.addr, aligned as u64);
    }
}

fn main(args: &str) -> u8 {
    let parts: Vec<&str> = args.split_whitespace().collect();
    if parts.first() == Some(&"run") {
        let name = parts.get(1).copied().unwrap_or("tiny");
        let prompt = parse_prompt(&parts).unwrap_or_default();
        let max_new = parse_flag(&parts, "--max")
            .and_then(|v| v.parse().ok())
            .unwrap_or(16);
        let temp: f32 = parse_flag(&parts, "--temp")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0.0);
        let top_p: f32 = parse_flag(&parts, "--top-p")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0.9);
        let seed: u64 = parse_flag(&parts, "--seed")
            .and_then(|v| v.parse().ok())
            .unwrap_or(42);
        return run_model(name, &prompt, max_new, Sampler::new(temp, top_p, seed));
    }
    println!("uso: soso-llm run <modelo> --prompt <texto> [--max <n>] [--temp <t>] [--top-p <p>] [--seed <s>]");
    1
}

fn parse_flag(parts: &[&str], flag: &str) -> Option<String> {
    parts
        .iter()
        .position(|&p| p == flag)
        .and_then(|i| parts.get(i + 1))
        .map(|&v| v.into())
}

/// El prompt toma todas las palabras hasta el siguiente flag (sosh no
/// interpreta comillas).
fn parse_prompt(parts: &[&str]) -> Option<String> {
    let i = parts.iter().position(|&p| p == "--prompt")?;
    let words: Vec<&str> = parts[i + 1..]
        .iter()
        .take_while(|p| !p.starts_with("--"))
        .copied()
        .collect();
    if words.is_empty() {
        None
    } else {
        Some(words.join(" "))
    }
}

fn run_model(name: &str, prompt: &str, max_new: usize, mut sampler: Sampler) -> u8 {
    let base = format!("/models/{name}");
    let manifest_path = format!("{base}/manifest.som");
    let index_path = format!("{base}/index.som");

    let manifest_data = match read_file(&manifest_path) {
        Ok(d) => d,
        Err(e) => {
            println!("soso-llm: no puedo leer {manifest_path} (errno {e})");
            return 1;
        }
    };
    let manifest = match Manifest::parse(&manifest_data) {
        Ok(m) => m,
        Err(()) => {
            println!("soso-llm: manifest inválido");
            return 1;
        }
    };

    let index_data = match read_file(&index_path) {
        Ok(d) => d,
        Err(e) => {
            println!("soso-llm: no puedo leer {index_path} (errno {e})");
            return 1;
        }
    };
    let index = match TensorIndex::parse(&index_data) {
        Ok(i) => i,
        Err(()) => {
            println!("soso-llm: index inválido");
            return 1;
        }
    };

    println!(
        "soso-llm: modelo {} ({} capas, hidden={})",
        manifest.name, manifest.num_layers, manifest.hidden_dim
    );

    let mut gpu = abi::GpuInfo::default();
    let _ = sys::gpu_info(&mut gpu);
    if gpu.present != 0 {
        println!("soso-llm: GPU detectada, VRAM libre {} bytes", gpu.vram_free);
    } else {
        println!("soso-llm: backend CPU");
    }

    let ram_budget = 32 * 1024 * 1024;
    let vram_budget = if gpu.present != 0 {
        gpu.vram_free.min(64 * 1024 * 1024) as usize
    } else {
        0
    };
    let mut rt = Runtime::new(manifest, index.clone(), ram_budget, vram_budget);
    if rt.validate_shapes().is_err() {
        println!("soso-llm: shapes del index no casan con el manifest");
        return 1;
    }

    let tokenizer = match read_file(&format!("{base}/tokenizer.som")) {
        Ok(data) => match Tokenizer::parse(&data) {
            Ok(t) => t,
            Err(()) => {
                println!("soso-llm: tokenizer.som inválido");
                return 1;
            }
        },
        Err(_) => Tokenizer::byte_level(),
    };

    let shards_base = format!("{base}/shards");
    let mut source = MmapTensorSource::new(shards_base, index, SyscallMapper);

    // Pool de hilos (NCPU). Con 1 CPU o si el spawn falla → secuencial.
    let pool = ThreadPool::new();
    println!("soso-llm: workers={}", pool.workers());
    // Matvec paralelo por filas cuando hay workers.
    let par: Option<&dyn RowParallel> = if pool.workers() > 1 {
        Some(&pool)
    } else {
        None
    };

    let text = if prompt.is_empty() { "hola" } else { prompt };
    let prompt_tokens = tokenizer.encode(text);
    // streaming: cada token se imprime según se genera
    let mut decoder = StreamDecoder::new();
    let t0 = sys::uptime_ms();
    let result = rt.generate_stream_par(
        &mut source,
        &prompt_tokens,
        max_new,
        tokenizer.eos(),
        &mut sampler,
        |t| {
            let s = decoder.push(&tokenizer, t);
            if !s.is_empty() {
                libsoso::print!("{s}");
            }
        },
        par,
    );
    match result {
        Ok(tokens) => {
            let elapsed_ms = (sys::uptime_ms() - t0).max(1) as u64;
            let resto = decoder.finish();
            if !resto.is_empty() {
                libsoso::print!("{resto}");
            }
            println!();
            let n = tokens.len();
            let tok_s = n as f64 * 1000.0 / elapsed_ms as f64;
            println!(
                "soso-llm: generado ({} tokens, {} ms, {:.2} tok/s)",
                n, elapsed_ms, tok_s
            );
            0
        }
        Err(()) => {
            println!("soso-llm: inferencia falló");
            1
        }
    }
}

fn read_file(path: &str) -> Result<Vec<u8>, i64> {
    let fd = sys::open(path, O_RDONLY);
    if fd < 0 {
        return Err(fd);
    }
    let mut st = abi::Stat::default();
    if sys::stat(path, &mut st) < 0 {
        sys::close(fd as u64);
        return Err(-abi::EIO);
    }
    if st.size > 16 * 1024 * 1024 {
        let map = sys::mmap(0, st.size, fd as u64, 0);
        sys::close(fd as u64);
        if map < 0 {
            return Err(map);
        }
        let mut out = Vec::with_capacity(st.size as usize);
        let ptr = map as *const u8;
        for i in 0..st.size as usize {
            out.push(unsafe { core::ptr::read_volatile(ptr.add(i)) });
        }
        sys::munmap(map as u64, st.size.next_multiple_of(4096));
        return Ok(out);
    }
    let mut buf = vec![0u8; st.size as usize];
    let n = sys::read(fd as u64, &mut buf);
    sys::close(fd as u64);
    if n < 0 {
        return Err(n);
    }
    buf.truncate(n as usize);
    Ok(buf)
}
