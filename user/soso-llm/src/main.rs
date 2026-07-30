//! Inferencia LLM en userspace de soso.

#![no_std]
#![no_main]

extern crate alloc;

mod cuda_host;
mod distributed;
mod gpu;
mod net;
mod pool;

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use distributed::{crc_bytes, default_keepalive, default_timeouts, DistributedConfig};
use libsoso::{println, sys};
use pool::ThreadPool;
use soso_abi::{self as abi, O_RDONLY};
use soso_llm_core::parallel::RowParallel;
use soso_llm_core::plan::{MemSnapshot, ResourcePlanner};
use soso_llm_core::pipeline::{PipelinePlan, PipelineRole};
use soso_llm_core::runtime::{Backend, Runtime};
use soso_llm_core::sample::Sampler;
use soso_llm_core::source::{FileMapper, MappedShard, MmapTensorSource};
use soso_llm_core::tokenizer::{StreamDecoder, Tokenizer};
use sosomodel::index::TensorIndex;
use sosomodel::manifest::Manifest;

libsoso::entry!(main);

fn read_mem_snapshot() -> MemSnapshot {
    let mut mi = abi::MemInfo::default();
    if sys::meminfo(&mut mi) == 0 {
        MemSnapshot {
            total_frames: mi.total_frames,
            free_frames: mi.free_frames,
            reclaimable_frames: mi.reclaimable_frames,
        }
    } else {
        MemSnapshot::default()
    }
}

fn clock_ms() -> u64 {
    sys::uptime_ms().max(0) as u64
}

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

struct ModelBundle {
    rt: Runtime,
    source: MmapTensorSource<SyscallMapper>,
    tokenizer: Tokenizer,
    manifest_crc: u32,
    index_crc: u32,
}

fn main(args: &str) -> u8 {
    let parts: Vec<&str> = args.split_whitespace().collect();
    if parts.first() == Some(&"node") || parts.first() == Some(&"worker") {
        return run_node_cmd(&parts);
    }
    if parts.first() == Some(&"run") {
        let name = parts.get(1).copied().unwrap_or("tiny");
        let prompt = parse_prompt(&parts).unwrap_or_default();
        let max_new = parse_flag(&parts, "--max")
            .and_then(|v| v.parse().ok())
            .unwrap_or(16);
        let temp: f32 = parse_flag(&parts, "--temp")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0.7);
        let top_p: f32 = parse_flag(&parts, "--top-p")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0.9);
        let seed: u64 = parse_flag(&parts, "--seed")
            .and_then(|v| v.parse().ok())
            .unwrap_or(42);
        if let Some(cuda_host) = parse_flag(&parts, "--cuda-host") {
            let text = if prompt.is_empty() { "hola" } else { &prompt };
            let cfg = cuda_host::config_from_run(
                &cuda_host, name, text, max_new, temp, top_p, seed,
            );
            return cuda_host::run(&cfg);
        }
        if let Some(pipeline) = parse_flag(&parts, "--pipeline") {
            let splits = parse_splits(&parts).unwrap_or_default();
            let (step_to, hs_to, _) = parse_timeouts(&parts);
            let (ping_int, ping_idle, standby_retry) = parse_keepalive(&parts);
            let standby = parts.iter().any(|&p| p == "--standby");
            return run_distributed_head(
                name,
                &prompt,
                max_new,
                Sampler::new(temp, top_p, seed),
                seed,
                &pipeline,
                &splits,
                step_to,
                hs_to,
                ping_int,
                ping_idle,
                standby,
                standby_retry,
            );
        }
        if let Some(remote) = parse_flag(&parts, "--remote") {
            let split: u32 = parse_flag(&parts, "--split")
                .and_then(|v| v.parse().ok())
                .unwrap_or(2);
            let splits = format!("{split}");
            let (step_to, hs_to, _) = parse_timeouts(&parts);
            let (ping_int, ping_idle, standby_retry) = parse_keepalive(&parts);
            let standby = parts.iter().any(|&p| p == "--standby");
            return run_distributed_head(
                name,
                &prompt,
                max_new,
                Sampler::new(temp, top_p, seed),
                seed,
                &remote,
                &splits,
                step_to,
                hs_to,
                ping_int,
                ping_idle,
                standby,
                standby_retry,
            );
        }
        // Dispositivo software del kernel: enciende el camino de syscalls GPU en
        // una máquina sin GPU. Es para pruebas —lo calcula la CPU del kernel— y se
        // dice tal cual en el log; sirve para ejercitar alloc/map/submit/read y el
        // cacheo de pesos, que de otro modo sólo se estrenarían en la tarjeta real.
        let soft = parts.contains(&"--gpu-soft");
        let force_cpu = parts.contains(&"--cpu");
        if soft && force_cpu {
            println!("soso-llm: --cpu y --gpu-soft no se combinan; elige uno");
            return 1;
        }
        if soft {
            let rc = sys::gpu_submit(b"SOFTG");
            if rc < 0 {
                println!("soso-llm: no se pudo activar el dispositivo software (rc={rc})");
            }
        }
        let rc = run_model(
            name,
            &prompt,
            max_new,
            Sampler::new(temp, top_p, seed),
            force_cpu,
        );
        if soft {
            // Se apaga al salir: el dispositivo es estado GLOBAL del kernel, y
            // dejarlo puesto hace que el siguiente proceso de este arranque vea una
            // GPU que no existe (y se crea el offload).
            let _ = sys::gpu_submit(b"SOFTX");
        }
        return rc;
    }
    println!("uso:");
    println!("  soso-llm run <modelo> --prompt <texto> [--max <n>]");
    println!("    [--cuda-host <ip:puerto>]  (inferencia CUDA en host Linux, L6-H)");
    println!("    [--cpu]                    (fuerza matvec en CPU; para comparar tok/s)");
    println!("    [--gpu-soft]               (dispositivo software del kernel: ejercita");
    println!("                                el camino de syscalls GPU sin GPU real)");
    println!("    [--pipeline <ip:puerto>,...] [--splits <n1,n2,...>]");
    println!("    [--step-timeout-ms <ms>] [--handshake-timeout-ms <ms>]");
    println!("    [--ping-interval-ms <ms>] [--ping-idle-ms <ms>]");
    println!("    [--standby] [--standby-retry-ms <ms>]");
    println!("    [--remote <ip:puerto> --split <n>]  (compat v1)");
    println!("  soso-llm node <modelo> --listen <puerto> --layers <start>:<end>");
    println!("  soso-llm worker ...  (alias de node)");
    1
}

fn run_node_cmd(parts: &[&str]) -> u8 {
    let name = parts.get(1).copied().unwrap_or("tiny");
    let listen: u16 = parse_flag(parts, "--listen")
        .and_then(|v| v.parse().ok())
        .unwrap_or(9900);
    let (layer_start, layer_end) = match parse_layers(parts) {
        Some(v) => v,
        None => {
            let split: u32 = parse_flag(parts, "--split")
                .and_then(|v| v.parse().ok())
                .unwrap_or(2);
            let num_layers = read_num_layers(name).unwrap_or(4);
            (split, num_layers)
        }
    };
    run_node(name, layer_start, layer_end, listen, &parts)
}

fn parse_keepalive(parts: &[&str]) -> (u64, u64, u64) {
    let (def_ping, def_idle, def_retry) = default_keepalive();
    let ping = parse_flag(parts, "--ping-interval-ms")
        .and_then(|v| v.parse().ok())
        .unwrap_or(def_ping);
    let idle = parse_flag(parts, "--ping-idle-ms")
        .and_then(|v| v.parse().ok())
        .unwrap_or(def_idle);
    let retry = parse_flag(parts, "--standby-retry-ms")
        .and_then(|v| v.parse().ok())
        .unwrap_or(def_retry);
    (ping, idle, retry)
}

fn parse_timeouts(parts: &[&str]) -> (u64, u64, u64) {
    let (def_step, def_hs, def_accept) = default_timeouts();
    let step = parse_flag(parts, "--step-timeout-ms")
        .and_then(|v| v.parse().ok())
        .unwrap_or(def_step);
    let hs = parse_flag(parts, "--handshake-timeout-ms")
        .and_then(|v| v.parse().ok())
        .unwrap_or(def_hs);
    let accept = parse_flag(parts, "--accept-timeout-ms")
        .and_then(|v| v.parse().ok())
        .unwrap_or(def_accept);
    (step, hs, accept)
}

fn read_num_layers(name: &str) -> Option<u32> {
    let path = format!("/models/{name}/manifest.som");
    let data = read_file(&path).ok()?;
    Manifest::parse(&data).ok().map(|m| m.num_layers)
}

fn parse_flag(parts: &[&str], flag: &str) -> Option<String> {
    parts
        .iter()
        .position(|&p| p == flag)
        .and_then(|i| parts.get(i + 1))
        .map(|&v| v.into())
}

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

fn parse_layers(parts: &[&str]) -> Option<(u32, u32)> {
    let s = parse_flag(parts, "--layers")?;
    let (a, b) = s.split_once(':')?;
    Some((a.parse().ok()?, b.parse().ok()?))
}

fn parse_splits(parts: &[&str]) -> Option<String> {
    parse_flag(parts, "--splits")
}

fn parse_split_list(splits: &str) -> Vec<u32> {
    splits
        .split(',')
        .filter_map(|p| p.trim().parse().ok())
        .collect()
}

fn parse_pipeline_list(pipeline: &str) -> Vec<String> {
    pipeline
        .split(',')
        .map(|s| String::from(s.trim()))
        .filter(|s| !s.is_empty())
        .collect()
}

fn load_model(
    name: &str,
    role: PipelineRole,
    layer_start: u32,
    layer_end: u32,
) -> Result<ModelBundle, u8> {
    let base = format!("/models/{name}");
    let manifest_path = format!("{base}/manifest.som");
    let index_path = format!("{base}/index.som");

    let manifest_data = read_file(&manifest_path).map_err(|e| {
        println!("soso-llm: no puedo leer {manifest_path} (errno {e})");
        1u8
    })?;
    let manifest = Manifest::parse(&manifest_data).map_err(|_| {
        println!("soso-llm: manifest inválido");
        1u8
    })?;
    let manifest_crc = crc_bytes(&manifest_data);

    let index_data = read_file(&index_path).map_err(|e| {
        println!("soso-llm: no puedo leer {index_path} (errno {e})");
        1u8
    })?;
    let index = TensorIndex::parse(&index_data).map_err(|_| {
        println!("soso-llm: index inválido");
        1u8
    })?;
    let index_crc = crc_bytes(&index_data);

    let mut rt = Runtime::new(manifest, index.clone(), 32 * 1024 * 1024, 0);
    if rt
        .validate_shapes_for_role(role, layer_start, layer_end)
        .is_err()
    {
        println!("soso-llm: shapes del index no casan con el rol");
        return Err(1);
    }

    let tokenizer = match read_file(&format!("{base}/tokenizer.som")) {
        Ok(data) => Tokenizer::parse(&data).map_err(|_| {
            println!("soso-llm: tokenizer.som inválido");
            1u8
        })?,
        Err(_) => Tokenizer::byte_level(),
    };

    let source = MmapTensorSource::new(format!("{base}/shards"), index, SyscallMapper);
    Ok(ModelBundle {
        rt,
        source,
        tokenizer,
        manifest_crc,
        index_crc,
    })
}

fn run_distributed_head(
    name: &str,
    prompt: &str,
    max_new: usize,
    mut sampler: Sampler,
    seed: u64,
    pipeline: &str,
    splits: &str,
    step_timeout_ms: u64,
    handshake_timeout_ms: u64,
    ping_interval_ms: u64,
    ping_idle_ms: u64,
    standby: bool,
    standby_retry_ms: u64,
) -> u8 {
    let remotes = parse_pipeline_list(pipeline);
    let split_vals = parse_split_list(splits);
    let num_layers = match read_num_layers(name) {
        Some(n) => n,
        None => {
            println!("soso-llm: no puedo leer manifest");
            return 1;
        }
    };
    let plan = match PipelinePlan::from_splits(&split_vals, num_layers) {
        Ok(p) => p,
        Err(()) => {
            println!("soso-llm: splits inválidos");
            return 1;
        }
    };
    if remotes.len() != plan.remote_count() {
        println!(
            "soso-llm: pipeline tiene {} nodos pero splits implican {}",
            remotes.len(),
            plan.remote_count()
        );
        return 1;
    }
    let head = plan.head_segment();
    let mut bundle = match load_model(name, PipelineRole::Head, head.layer_start, head.layer_end) {
        Ok(b) => b,
        Err(c) => return c,
    };
    let pool = ThreadPool::new();
    println!("soso-llm: workers={}", pool.workers());
    let par: Option<&dyn RowParallel> = if pool.workers() > 1 {
        Some(&pool)
    } else {
        None
    };
    let (_, _, accept_to) = default_timeouts();
    let cfg = DistributedConfig {
        plan,
        remotes,
        listen_port: 0,
        layer_start: head.layer_start,
        layer_end: head.layer_end,
        model_name: String::from(name),
        manifest_crc: bundle.manifest_crc,
        index_crc: bundle.index_crc,
        step_timeout_ms,
        handshake_timeout_ms,
        accept_timeout_ms: accept_to,
        ping_interval_ms,
        ping_idle_ms,
        standby,
        standby_retry_ms,
    };
    let text = if prompt.is_empty() { "test" } else { prompt };
    let run = if standby {
        distributed::run_head_standby(
            &mut bundle.rt,
            &mut bundle.source,
            &bundle.tokenizer,
            &cfg,
            text,
            max_new,
            sampler,
            seed,
            par,
        )
    } else {
        distributed::run_head(
            &mut bundle.rt,
            &mut bundle.source,
            &bundle.tokenizer,
            &cfg,
            text,
            max_new,
            &mut sampler,
            seed,
            par,
        )
    };
    match run {
        Ok(()) => 0,
        Err(()) => {
            println!("soso-llm: inferencia distribuida falló");
            1
        }
    }
}

fn run_node(name: &str, layer_start: u32, layer_end: u32, listen: u16, parts: &[&str]) -> u8 {
    let num_layers = match read_num_layers(name) {
        Some(n) => n,
        None => {
            println!("soso-llm: no puedo leer manifest");
            return 1;
        }
    };
    let role = PipelineRole::from_layer_range(layer_start, layer_end, num_layers);
    let mut bundle = match load_model(name, role, layer_start, layer_end) {
        Ok(b) => b,
        Err(c) => return c,
    };
    let pool = ThreadPool::new();
    println!("soso-llm: workers={}", pool.workers());
    let par: Option<&dyn RowParallel> = if pool.workers() > 1 {
        Some(&pool)
    } else {
        None
    };
    let (step_to, hs_to, accept_to) = parse_timeouts(parts);
    let (ping_int, ping_idle, standby_retry) = parse_keepalive(parts);
    let cfg = DistributedConfig {
        plan: PipelinePlan {
            segments: Vec::new(),
        },
        remotes: Vec::new(),
        listen_port: listen,
        layer_start,
        layer_end,
        model_name: String::from(name),
        manifest_crc: bundle.manifest_crc,
        index_crc: bundle.index_crc,
        step_timeout_ms: step_to,
        handshake_timeout_ms: hs_to,
        accept_timeout_ms: accept_to,
        ping_interval_ms: ping_int,
        ping_idle_ms: ping_idle,
        standby: false,
        standby_retry_ms: standby_retry,
    };
    match distributed::run_node(&mut bundle.rt, &mut bundle.source, &cfg, par) {
        Ok(()) => 0,
        Err(()) => {
            println!("soso-llm: nodo terminó con error");
            1
        }
    }
}

fn run_model(
    name: &str,
    prompt: &str,
    max_new: usize,
    mut sampler: Sampler,
    force_cpu: bool,
) -> u8 {
    let num_layers = read_num_layers(name).unwrap_or(4);
    let mut bundle = match load_model(name, PipelineRole::Full, 0, num_layers) {
        Ok(b) => b,
        Err(c) => return c,
    };
    println!(
        "soso-llm: modelo {} ({} capas, hidden={})",
        bundle.rt.manifest.name, bundle.rt.manifest.num_layers, bundle.rt.manifest.hidden_dim
    );

    let mut gpu = abi::GpuInfo::default();
    let _ = sys::gpu_info(&mut gpu);
    let mem = read_mem_snapshot();
    let planner = ResourcePlanner::new(
        &bundle.rt.manifest,
        &bundle.rt.index,
        mem,
        gpu.vram_free,
        false,
    );
    bundle.rt.set_planner(planner);
    if let Some(pl) = bundle.rt.planner.as_ref() {
        let st = pl.stats();
        println!(
            "soso-llm: planificador — presupuesto pesos {} KiB, modelo {} KiB, capas CPU/GPU/remoto {}/{}/{}",
            st.weight_budget_bytes / 1024,
            st.model_weight_bytes / 1024,
            st.cpu_layers,
            st.gpu_layers,
            st.remote_layers,
        );
        println!(
            "soso-llm: streaming — working-set {} capas, ventana KV {} tokens (LayerKV+StreamingLLM), KV {} H2O={} sparse={}",
            st.resident_layers,
            st.kv_window_tokens,
            if st.kv_dtype_i8 != 0 { "int8" } else { "f16" },
            st.h2o_enabled,
            st.sparse_attn,
        );
        println!(
            "soso-llm: memoria — libre {} KiB, reclaimable {} KiB",
            mem.free_bytes() / 1024,
            mem.reclaimable_bytes() / 1024,
        );
    }
    let mut sys_gpu = if force_cpu {
        None
    } else {
        gpu::SysGpu::new()
    };
    // El `present` del kernel no basta para decidir: un dispositivo puede aceptar
    // búferes y no ejecutar nada (iGPU Intel), y entonces `SysGpu::new` dice no.
    // Anunciar "GPU detectada" mirando sólo `present` era prometer un offload que
    // no iba a ocurrir — y con el dispositivo software, además, mentir.
    if force_cpu {
        println!("soso-llm: backend CPU (--cpu)");
        bundle.rt.set_backend(Backend::Cpu);
    } else if let Some(ref g) = sys_gpu {
        println!(
            "soso-llm: dispositivo de cómputo «{}» (fase {}), VRAM libre {} bytes",
            g.device_name(),
            g.phase(),
            gpu.vram_free
        );
        bundle.rt.set_backend(Backend::Auto);
        bundle.rt.tiers.vram_budget = gpu.vram_free as usize;
        if let Some(pl) = bundle.rt.planner.as_mut() {
            pl.set_vram_free(gpu.vram_free);
        }
    } else if gpu.present != 0 {
        println!(
            "soso-llm: hay GPU («{}») pero no ejecuta kernels; backend CPU",
            libsoso::str_hasta_nul(&gpu.name)
        );
        bundle.rt.set_backend(Backend::Cpu);
    } else {
        println!("soso-llm: backend CPU");
        bundle.rt.set_backend(Backend::Cpu);
    }

    let mut gpu_ref: Option<&mut dyn soso_llm_core::gpu::GpuDispatch> = sys_gpu
        .as_mut()
        .map(|g| g as &mut dyn soso_llm_core::gpu::GpuDispatch);

    let pool = ThreadPool::new();
    println!("soso-llm: workers={}", pool.workers());
    let par: Option<&dyn RowParallel> = if pool.workers() > 1 {
        Some(&pool)
    } else {
        None
    };
    let text = if prompt.is_empty() { "hola" } else { prompt };
    let prompt_tokens = bundle.tokenizer.encode(text);
    let mut decoder = StreamDecoder::new();
    let t0 = sys::uptime_ms();
    let result = bundle.rt.generate_stream_planned(
        &mut bundle.source,
        &prompt_tokens,
        max_new,
        bundle.tokenizer.eos(),
        &mut sampler,
        |t| {
            let s = decoder.push(&bundle.tokenizer, t);
            if !s.is_empty() {
                libsoso::print!("{s}");
            }
        },
        par,
        &mut gpu_ref,
        clock_ms,
        read_mem_snapshot,
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
            if let Some(pl) = bundle.rt.planner.as_ref() {
                let st = pl.stats();
                println!(
                    "soso-llm: planificador — replanes {}, latencia media CPU/GPU/remoto {:.1}/{:.1}/{:.1} ms",
                    st.replans,
                    st.avg_cpu_ms,
                    st.avg_gpu_ms,
                    st.avg_remote_ms,
                );
                if st.avg_matvec_ms > 0.0 || st.avg_attn_ms > 0.0 {
                    println!(
                        "soso-llm: hot path — matvec {:.2} ms/capa, attn {:.2} ms/capa",
                        st.avg_matvec_ms, st.avg_attn_ms
                    );
                }
                println!(
                    "soso-llm: streaming — prefetch {}, liberaciones shard {}, ventanas KV {}",
                    st.prefeches,
                    st.shard_releases,
                    st.kv_slides,
                );
                if st.pld_attempts > 0 || st.pld_accepted > 0 {
                    println!(
                        "soso-llm: prompt-lookup — {} aceptados en {} intentos (n≈{}, draft≤{})",
                        st.pld_accepted, st.pld_attempts, st.pld_prefer_n, st.pld_max_draft
                    );
                }
            }
            // Los pesos SUBIDOS frente a las llamadas es la cifra que dice si el
            // cacheo funciona: sin él eran una subida de la matriz entera por
            // llamada, y en el log no se veía nada raro.
            if let Some(ref g) = sys_gpu {
                g.print_diagnostics();
            }
            0
        }
        Err(()) => {
            println!("soso-llm: inferencia falló");
            if let Some(ref g) = sys_gpu {
                g.print_diagnostics();
            }
            1
        }
    }
}

/// El nombre que da el kernel viene en un `[u8; 32]` con relleno a cero.
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
