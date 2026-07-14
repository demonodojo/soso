//! Inferencia LLM en userspace de soso.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use libsoso::{println, sys};
use soso_abi::{self as abi, O_RDONLY};
use sosomodel::index::TensorIndex;
use sosomodel::manifest::Manifest;
use soso_llm_core::runtime::{MemoryTensorSource, Runtime};

libsoso::entry!(main);

fn main(args: &str) -> u8 {
    let parts: Vec<&str> = args.split_whitespace().collect();
    if parts.first() == Some(&"run") {
        let name = parts.get(1).copied().unwrap_or("tiny");
        return run_model(name, parts.get(3).copied().unwrap_or(""));
    }
    println!("uso: soso-llm run <modelo> --prompt <texto>");
    1
}

fn run_model(name: &str, _prompt: &str) -> u8 {
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
    let mut rt = Runtime::new(manifest, index, ram_budget, vram_budget);

    let fd = sys::open(&format!("{base}/shards/embed.tensor"), O_RDONLY);
    if fd < 0 {
        println!("soso-llm: no embed.tensor (errno {fd})");
        return 1;
    }
    let mut st = abi::Stat::default();
    if sys::stat(&format!("{base}/shards/embed.tensor"), &mut st) < 0 {
        return 1;
    }
    let size = st.size as usize;
    let map = sys::mmap(0, size as u64, fd as u64, 0);
    sys::close(fd as u64);
    if map < 0 {
        println!("soso-llm: mmap falló (errno {map})");
        return 1;
    }
    let ptr = map as *const u8;
    let _ = unsafe { core::ptr::read_volatile(ptr) };

    let mut source = MemoryTensorSource {
        tensors: alloc::collections::BTreeMap::new(),
    };
    let h = rt.manifest.hidden_dim as usize;
    let vocab = rt.manifest.vocab_size as usize;
    source.tensors.insert(String::from("embed"), vec![0.0f32; vocab * h]);

    rt.embed_token(1, source.tensors.get("embed").unwrap());
    match rt.forward(&source) {
        Ok(()) => println!("soso-llm: forward OK (1 token)"),
        Err(()) => {
            println!("soso-llm: forward falló");
            return 1;
        }
    }

    sys::munmap(map as u64, size.next_multiple_of(4096) as u64);
    0
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
