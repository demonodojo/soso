//! Inferencia LLM en userspace de soso.

#![no_std]
#![no_main]

extern crate alloc;

mod ask;
mod cuda_host;
mod distributed;
mod net;
mod pool;
mod serve;
mod serve_poll;
mod session;
mod staging;

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use ask::{resto_tras, run_ask};
use distributed::{crc_bytes, default_keepalive, default_timeouts, DistributedConfig};
use libsoso::{println, sys};
use pool::ThreadPool;
use soso_abi::{self as abi, O_RDONLY};
use soso_llm_core::layer::TensorSource;
use soso_llm_core::parallel::RowParallel;
use soso_llm_core::plan::{MemSnapshot, MemoryPlanConfig, MemoryPreset, ResourcePlanner};
use soso_llm_core::pipeline::{PipelinePlan, PipelineRole};
use soso_llm_core::runtime::{Backend, Runtime};
use soso_llm_core::conversation::ModelProfile;
use soso_llm_core::generation::{GenerationOptions, GenerationReport};
use soso_llm_core::sample::Sampler;
use soso_llm_core::tokenizer::StreamDecoder;
use soso_llm_api::PreparedChatCompletion;
use session::{liberar_sesion, load_model, preparar_sesion, Sesion};
use sosomodel::manifest::Manifest;

libsoso::entry!(main);

fn read_iostat() -> abi::IoStat {
    let mut io = abi::IoStat::default();
    let _ = sys::iostat(&mut io);
    io
}

/// Imprime lo que ha costado el disco entre dos instantáneas.
///
/// `peticiones` frente a `bloques` es la cifra que dice si una optimización
/// agrupó lecturas o simplemente leyó menos: son cosas distintas y hasta ahora
/// no se distinguían porque no se medía ninguna de las dos.
fn print_iostat(antes: &abi::IoStat) {
    let ahora = read_iostat();
    let peticiones = ahora.peticiones.saturating_sub(antes.peticiones);
    let bloques = ahora.bloques.saturating_sub(antes.bloques);
    let ms = ahora.nanos.saturating_sub(antes.nanos) / 1_000_000;
    let aciertos = ahora.cache_aciertos.saturating_sub(antes.cache_aciertos);
    let fallos = ahora.cache_fallos.saturating_sub(antes.cache_fallos);
    let total_cache = aciertos + fallos;
    let pct = if total_cache > 0 { aciertos * 100 / total_cache } else { 0 };
    let us = if peticiones > 0 {
        (ahora.nanos.saturating_sub(antes.nanos) / peticiones) / 1000
    } else {
        0
    };
    println!(
        "soso-llm: disco — {} peticiones, {} bloques ({} KiB), {} ms ({} us/petición), caché {} %",
        peticiones,
        bloques,
        bloques * 4,
        ms,
        us,
        pct,
    );
}

pub(crate) fn read_mem_snapshot() -> MemSnapshot {
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

pub(crate) fn read_file(path: &str) -> Result<Vec<u8>, i64> {
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


fn main(args: &[String]) -> u8 {
    // `ask` se resuelve sobre el string CRUDO, antes de trocear: todo lo que
    // va detrás es la pregunta, con sus comillas, sus tildes y sus `|` o `>`.
    // Es la única forma de que el texto llegue tal como se escribió, y por eso
    // sosh lo desvía aquí sin pasarlo por su tokenizador.
    //
    // Desde T62 `main` recibe el argv de verdad, así que la línea literal sólo
    // existe cuando el llamante no pasó argv (`spawn_io`). Si pasó argv, la
    // pregunta es el argumento siguiente —entera, con sus espacios— y no hay
    // nada que reconstruir.
    let linea = libsoso::linea_cruda();
    if let Some(texto) = linea.as_deref().and_then(|l| resto_tras("ask", l)) {
        return run_ask(texto);
    }
    if args.first().map(|s| s.as_str()) == Some("ask") {
        // Con argv la pregunta ya viene troceada por quien llamó, así que esto
        // es lo más literal que queda. La ruta que conserva el texto exacto es
        // el builtin `ask` de sosh, que no pasa por aquí: va por el socket de
        // askd sin tokenizar.
        return run_ask(args[1..].join(" ").trim());
    }
    if args.first().map(|s| s.as_str()) == Some("askd") {
        return ask::run_askd();
    }
    let parts: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    if parts.first() == Some(&"serve") {
        return serve::run(&parts[1..]);
    }
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
            parse_memory_plan(&parts),
            parts.contains(&"--chat"),
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
    println!("    [--mem-tight|--mem-balanced|--mem-max-pin]  (preset trunk-first)");
    println!("    [--trunk-frac <0-100>] [--ring-slots <1|2>]");
    println!("  soso-llm node <modelo> --listen <puerto> --layers <start>:<end>");
    println!("  soso-llm worker ...  (alias de node)");
    println!("  soso-llm ask <pregunta>   (texto literal; lo normal es usar `ask`)");
    println!("  soso-llm serve --model <nombre> --port <puerto> --token-file <ruta>");
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

fn parse_memory_plan(parts: &[&str]) -> MemoryPlanConfig {
    let preset = if parts.iter().any(|&p| p == "--mem-tight") {
        MemoryPreset::Tight
    } else if parts.iter().any(|&p| p == "--mem-balanced") {
        MemoryPreset::Balanced
    } else if parts.iter().any(|&p| p == "--mem-max-pin") {
        MemoryPreset::MaxPin
    } else {
        MemoryPreset::Auto
    };
    let trunk_frac_pct = parse_flag(parts, "--trunk-frac").and_then(|v| v.parse().ok());
    let ring_slots = parse_flag(parts, "--ring-slots")
        .and_then(|v| v.parse().ok())
        .unwrap_or(2);
    MemoryPlanConfig {
        preset,
        trunk_frac_pct,
        ring_slots,
    }
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


/// Diagnóstico: fd 3 en askd, stdout en `soso-llm run`.
fn traza(askd: bool, msg: &str) {
    if askd {
        libsoso::logln!("{msg}");
    } else {
        println!("{msg}");
    }
}

fn keepalive_dot(fd: u64) {
    let _ = sys::write_all(fd, b".");
}

fn traza_gpu_askd(g: &soso_gpu::SysGpu) {
    let (calls, uploads, resident, sin_sitio) = g.stats();
    libsoso::logln!(
        "soso-llm: dispositivo «{}» — {} matvec, {} subidas de pesos, {} matrices residentes, {} sin sitio (a CPU), último on_gpu={}",
        g.device_name(),
        calls,
        uploads,
        resident,
        sin_sitio,
        g.last_on_gpu() as u8
    );
}

fn run_model(
    name: &str,
    prompt: &str,
    max_new: usize,
    mut sampler: Sampler,
    force_cpu: bool,
    mem_plan: MemoryPlanConfig,
    chat: bool,
) -> u8 {
    let io0 = read_iostat();
    let mut sesion = match preparar_sesion(name, force_cpu, mem_plan, true, true) {
        Ok(s) => s,
        Err(c) => return c,
    };
    let rc = if !chat {
        generar(&mut sesion, prompt, max_new, &mut sampler, true, Some(&io0))
    } else {
        let conf = ask::leer_conf();
        let plantilla = ask::plantilla_efectiva(&conf, &sesion);
        if plantilla.is_empty() {
            println!("soso-llm: --chat sin plantilla (ni el modelo ni /etc/llm.conf traen una)");
        } else {
            println!("soso-llm: plantilla de chat aplicada");
        }
        let tokens = soso_llm_core::chat::render(plantilla, prompt, &sesion.bundle.tokenizer);
        generar_tokens(
            &mut sesion,
            &tokens,
            max_new,
            &mut sampler,
            true,
            Some(&io0),
            None,
            false,
        )
    };
    liberar_sesion(&mut sesion);
    rc
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
    let mut bundle = match load_model(name, PipelineRole::Head, head.layer_start, head.layer_end, true) {
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
    let run = if cfg.standby {
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
    let rc = match run {
        Ok(()) => 0,
        Err(()) => {
            println!("soso-llm: inferencia distribuida falló");
            1
        }
    };
    bundle.source.shutdown_worker();
    rc
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
    let mut bundle = match load_model(name, role, layer_start, layer_end, true) {
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
    let rc = match distributed::run_node(&mut bundle.rt, &mut bundle.source, &cfg, par) {
        Ok(()) => 0,
        Err(()) => {
            println!("soso-llm: nodo terminó con error");
            1
        }
    };
    bundle.source.shutdown_worker();
    rc
}

/// Genera la respuesta a `prompt` sobre una sesión ya cargada, en streaming.
///
/// `io0` es la marca de E/S desde la que contar (la de antes de cargar, para
/// que `soso-llm run` siga informando de la carga y el decode juntos); `None`
/// omite esa línea.
fn generar(
    sesion: &mut Sesion,
    prompt: &str,
    max_new: usize,
    sampler: &mut Sampler,
    verboso: bool,
    io0: Option<&abi::IoStat>,
) -> u8 {
    let text = if prompt.is_empty() { "hola" } else { prompt };
    // `@bos` = prompt de un solo token BOS, igual que en el arnés de host
    // (`examples/hostrun.rs`). Aquí faltaba, así que `--prompt @bos` se
    // tokenizaba byte a byte: '@'=64, 'b'=98, 'o'=111, 's'=115. Con un modelo
    // de vocabulario pequeño —`tiny-moe` tiene 64— ninguno de esos índices
    // existe, `embed_token` devolvía `Err` y la única señal era un escueto
    // «inferencia falló». El `tiny` denso no lo notaba porque su vocabulario es
    // 256 y cualquier byte es un token válido.
    let prompt_tokens = if text == "@bos" {
        alloc::vec![1u32]
    } else {
        sesion.bundle.tokenizer.encode(text)
    };
    generar_tokens(sesion, &prompt_tokens, max_new, sampler, verboso, io0, None, false)
}

/// Quita C0/NUL/� que tumbarían la sesión SSH si se cuelan al canal.
fn texto_ask_seguro(s: &str) -> alloc::string::String {
    s.chars()
        .filter(|c| *c == '\n' || *c == '\t' || (!c.is_control() && *c != '\u{FFFD}'))
        .collect()
}

/// Escribe un trozo al cliente de ask y cede el CPU para que sosh lo imprima.
fn emitir_ask(fd: u64, s: &str) {
    let limpio = texto_ask_seguro(s);
    if limpio.is_empty() {
        return;
    }
    let _ = sys::write_all(fd, limpio.as_bytes());
}

/// Socket del cliente de askd mientras se genera. El hook de capa no puede
/// capturar el `fd` (es un `fn` en el runtime).
static ASK_TICK_FD: AtomicU64 = AtomicU64::new(u64::MAX);
static ASK_LAYER_T0: AtomicU64 = AtomicU64::new(0);
static ASK_INFER_T0: AtomicU64 = AtomicU64::new(0);
static ASK_GPU_FOR_TICK: AtomicU64 = AtomicU64::new(0);
static ASK_TICK_N: AtomicU32 = AtomicU32::new(0);
/// Puntos de espera al socket hasta el primer token; el texto no se mezcla.
static ASK_KEEPALIVE: AtomicBool = AtomicBool::new(false);

fn askd_elapsed_ms(t0: u64) -> u64 {
    let now = sys::uptime_ms().max(0) as u64;
    now.saturating_sub(t0)
}

fn ask_keepalive_dot() {
    if !ASK_KEEPALIVE.load(Ordering::Relaxed) {
        return;
    }
    let fd = ASK_TICK_FD.load(Ordering::Relaxed);
    if fd != u64::MAX {
        keepalive_dot(fd);
    }
}

fn ask_layer_tick(layer: u32, n: u32) {
    let now = sys::uptime_ms().max(0) as u64;
    let t0 = ASK_LAYER_T0.swap(now, Ordering::Relaxed);
    let ms = if t0 == 0 {
        0
    } else {
        now.saturating_sub(t0)
    };
    let seq = ASK_TICK_N.fetch_add(1, Ordering::Relaxed);
    let ptr = ASK_GPU_FOR_TICK.load(Ordering::Relaxed);
    // Prefill (primer recorrido de capas) o capa lenta: el resto a 32×N líneas
    // de log se come el tok/s.
    let prefill = n > 0 && seq < n;
    if seq == 0 {
        let infer0 = ASK_INFER_T0.load(Ordering::Relaxed);
        let total = if infer0 != 0 {
            askd_elapsed_ms(infer0)
        } else {
            0
        };
        libsoso::logln!(
            "askd: primera capa completada ({}/{} capas, {} ms capa, {} ms inferencia)",
            layer + 1,
            n,
            ms,
            total
        );
    }
    if ptr != 0 && (prefill || ms >= 80) {
        let on_gpu = unsafe { (*(ptr as *const soso_gpu::SysGpu)).last_on_gpu() as u8 };
        libsoso::logln!(
            "soso-llm: capa {}/{} {} ms on_gpu={}",
            layer.saturating_add(1),
            n,
            ms,
            on_gpu
        );
    }
    ask_keepalive_dot();
}

/// Punto de espera al cliente y traza en fd 3 al entrar en una capa.
fn ask_layer_enter(layer: u32, n: u32) {
    if layer == 0 {
        let infer0 = ASK_INFER_T0.load(Ordering::Relaxed);
        let total = if infer0 != 0 {
            askd_elapsed_ms(infer0)
        } else {
            0
        };
        libsoso::logln!("askd: forward capa 1/{n} (+{total} ms inferencia)");
    }
    ask_keepalive_dot();
}

/// Generación para la API HTTP guest: sin stdout ni protocolo ask (T16).
pub(crate) fn generar_para_api(
    sesion: &mut Sesion,
    prompt_ids: &[u32],
    prepared: &PreparedChatCompletion,
    profile: &ModelProfile,
    observer: &mut dyn soso_llm_core::generation::GenerationObserver,
) -> Result<(Vec<u32>, GenerationReport), ()> {
    if sesion.pool.is_none() {
        sesion.pool = Some(ThreadPool::new());
    }
    sesion.bundle.source.disable_worker();
    let options = GenerationOptions::new(
        prepared.max_new_tokens as usize,
        profile.stop_token_ids.clone(),
    );
    let seed = prepared.seed.unwrap_or(42) as u64;
    let mut sampler = Sampler::new(
        prepared.temperature as f32,
        prepared.top_p as f32,
        seed,
    );
    let par: Option<&dyn RowParallel> = sesion
        .pool
        .as_ref()
        .filter(|p| p.workers() > 1)
        .map(|p| p as &dyn RowParallel);
    let mut gpu_ref: Option<&mut dyn soso_llm_core::gpu::GpuDispatch> = sesion
        .sys_gpu
        .as_mut()
        .map(|g| g as &mut dyn soso_llm_core::gpu::GpuDispatch);
    sesion.bundle.rt.generate_stream_planned_observed(
        &mut sesion.bundle.source,
        prompt_ids,
        options,
        &mut sampler,
        observer,
        par,
        &mut gpu_ref,
        || sys::uptime_ms().max(0) as u64,
        &mut read_mem_snapshot,
    )
}

/// Igual que `generar` pero con el prompt YA tokenizado.
///
/// Existe porque una plantilla de chat no se puede expresar como texto: el fin de
/// turno es el token EOS y no sus cuatro letras (ver `soso_llm_core::chat`), así
/// que quien la aplica trae tokens, no una cadena.
pub(crate) fn generar_tokens(
    sesion: &mut Sesion,
    prompt_tokens: &[u32],
    max_new: usize,
    sampler: &mut Sampler,
    verboso: bool,
    io0: Option<&abi::IoStat>,
    fd_out: Option<u64>,
    drop_pool: bool,
) -> u8 {
    // askd también usa el pool: Mixtral en un solo core tarda minutos/token
    // y el terminal parece colgado. Los workers duermen en futex entre
    // matvecs, así que Drop vuelve al accept (2026-08-31 era spin al 100 %).
    // Staging async sigue apagado: con SMP el worker no llega a `done`.
    if sesion.pool.is_none() {
        sesion.pool = Some(ThreadPool::new());
    }
    let mut decoder = StreamDecoder::new();
    let mut streamed = alloc::string::String::new();
    let t0 = sys::uptime_ms();
    let result = {
        let par: Option<&dyn RowParallel> = sesion
            .pool
            .as_ref()
            .filter(|p| p.workers() > 1)
            .map(|p| p as &dyn RowParallel);
        let gpu_tick_ptr = sesion
            .sys_gpu
            .as_ref()
            .map(|g| g as *const soso_gpu::SysGpu as u64)
            .unwrap_or(0);
        let mut gpu_ref: Option<&mut dyn soso_llm_core::gpu::GpuDispatch> = sesion
            .sys_gpu
            .as_mut()
            .map(|g| g as &mut dyn soso_llm_core::gpu::GpuDispatch);
        let bundle = &mut sesion.bundle;
        let eos = bundle.tokenizer.eos();
        if let Some(fd) = fd_out {
            let infer0 = sys::uptime_ms().max(0) as u64;
            ASK_TICK_FD.store(fd, Ordering::Relaxed);
            ASK_INFER_T0.store(infer0, Ordering::Relaxed);
            ASK_LAYER_T0.store(infer0, Ordering::Relaxed);
            ASK_GPU_FOR_TICK.store(gpu_tick_ptr, Ordering::Relaxed);
            ASK_TICK_N.store(0, Ordering::Relaxed);
            ASK_KEEPALIVE.store(true, Ordering::Relaxed);
            bundle.rt.layer_hook = Some(ask_layer_tick);
            bundle.rt.layer_enter_hook = Some(ask_layer_enter);
        }
        // askd: sin clock_ms/refresh_mem (syscall en el matvec AVX2) y sin
        // planificador por token. `run` sigue con el camino cronometrado.
        // Un punto por capa mientras espera: Mixtral tarda minutos en el
        // prefill y, si no hay tráfico, el cliente corta a los 4 min.
        if let Some(fd) = fd_out {
            let t_prefill = sys::uptime_ms();
            keepalive_dot(fd);
            bundle.rt.generate_stream_par(
                &mut bundle.source,
                prompt_tokens,
                max_new,
                eos,
                sampler,
                |t| {
                    let s = decoder.push(&bundle.tokenizer, t);
                    if !s.is_empty() {
                        let limpio = texto_ask_seguro(&s);
                        if !limpio.is_empty() {
                            if ASK_KEEPALIVE.swap(false, Ordering::Relaxed) {
                                emitir_ask(fd, "\n");
                            }
                            streamed.push_str(&limpio);
                            emitir_ask(fd, &limpio);
                        }
                    }
                },
                par,
                &mut gpu_ref,
                &mut |i, total| {
                    let ms = (sys::uptime_ms() - t_prefill).max(0) as u64;
                    if i == 1 || i == total || (total > 4 && i % 4 == 0) {
                        libsoso::logln!("askd: prefill token {i}/{total} (+{ms} ms)");
                    }
                    ask_keepalive_dot();
                    if i == total {
                        emitir_ask(fd, "\n");
                    }
                },
            )
        } else {
            bundle.rt.generate_stream_planned(
                &mut bundle.source,
                prompt_tokens,
                max_new,
                eos,
                sampler,
                |t| {
                    let s = decoder.push(&bundle.tokenizer, t);
                    if !s.is_empty() {
                        // NUL/C0/� por el canal SSH tumban la sesión (A7:
                        // veinte `soso-llm run` en el mismo SSH).
                        let limpio = texto_ask_seguro(&s);
                        if !limpio.is_empty() {
                            libsoso::print!("{limpio}");
                        }
                    }
                },
                par,
                &mut gpu_ref,
                clock_ms,
                read_mem_snapshot,
            )
        }
    };
    ASK_TICK_FD.store(u64::MAX, Ordering::Relaxed);
    ASK_INFER_T0.store(0, Ordering::Relaxed);
    ASK_GPU_FOR_TICK.store(0, Ordering::Relaxed);
    ASK_KEEPALIVE.store(false, Ordering::Relaxed);
    sesion.bundle.rt.layer_hook = None;
    sesion.bundle.rt.layer_enter_hook = None;
    if drop_pool {
        sesion.pool = None;
    }
    match result {
        Ok(tokens) => {
            let elapsed_ms = (sys::uptime_ms() - t0).max(1) as u64;
            let resto = decoder.finish();
            if !resto.is_empty() {
                let limpio = texto_ask_seguro(&resto);
                if fd_out.is_some() {
                    streamed.push_str(&limpio);
                    if let Some(fd) = fd_out {
                        emitir_ask(fd, &limpio);
                    }
                } else if !limpio.is_empty() {
                    libsoso::print!("{limpio}");
                }
            }
            if let Some(fd) = fd_out {
                if streamed.is_empty() || !streamed.ends_with('\n') {
                    emitir_ask(fd, "\n");
                }
                let _ = sys::write_all(fd, &[crate::ask::PROTO_FIN]);
            }
            if fd_out.is_none() {
                println!();
            }
            let n = tokens.len();
            let tok_s = n as f64 * 1000.0 / elapsed_ms as f64;
            let resumen = format!(
                "soso-llm: generado ({} tokens, {} ms, {:.2} tok/s)",
                n, elapsed_ms, tok_s
            );
            if fd_out.is_some() {
                if let Some(ref g) = sesion.sys_gpu {
                    traza_gpu_askd(g);
                }
                libsoso::logln!("{resumen}");
                return 0;
            }
            if let Some(ref g) = sesion.sys_gpu {
                g.print_diagnostics_run(n, elapsed_ms);
            }
            println!("{resumen}");
            if !verboso {
                return 0;
            }
            if let Some(io0) = io0 {
                print_iostat(io0);
            }
            if let Some(pl) = sesion.bundle.rt.planner.as_ref() {
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
                    "soso-llm: streaming — prefetch {} ({} ms), staging wait {} ms, liberaciones shard {} ({} ms), ventanas KV {}",
                    st.prefeches,
                    st.stream_ms,
                    st.stage_wait_ms,
                    st.shard_releases,
                    st.release_ms,
                    st.kv_slides,
                );
                if st.moe_spec_hits > 0 || st.moe_spec_misses > 0 {
                    println!(
                        "soso-llm: MoE especulativo — {} aciertos, {} fallos",
                        st.moe_spec_hits, st.moe_spec_misses
                    );
                }
                if st.trunk_hits > 0 || st.trunk_misses > 0 {
                    println!(
                        "soso-llm: tronco — {} hits, {} misses, {} KiB leídos",
                        st.trunk_hits,
                        st.trunk_misses,
                        st.trunk_bytes_read / 1024,
                    );
                }
                if st.moe_resident_hits > 0 || st.moe_jit_hits > 0 || st.moe_misses > 0 {
                    println!(
                        "soso-llm: MoE cache — {} resident, {} JIT, {} fríos",
                        st.moe_resident_hits,
                        st.moe_jit_hits,
                        st.moe_misses,
                    );
                }
                if st.pld_attempts > 0 || st.pld_accepted > 0 {
                    println!(
                        "soso-llm: prompt-lookup — {} aceptados en {} intentos (n≈{}, draft≤{})",
                        st.pld_accepted, st.pld_attempts, st.pld_prefer_n, st.pld_max_draft
                    );
                }
            }
            0
        }
        Err(()) => {
            let (layer, key) = soso_llm_core::last_infer_op();
            let gpu_fail = sesion.sys_gpu.as_ref().and_then(|g| g.last_fail());
            let mut msg = String::from("soso-llm: inferencia falló");
            if let Some(l) = layer {
                msg.push_str(&format!(" capa {l}"));
            }
            if !key.is_empty() {
                msg.push_str(&format!(" tensor {key}"));
            }
            if let Some(r) = gpu_fail {
                msg.push_str(&format!(" gpu={r}"));
            }
            traza(fd_out.is_some(), &msg);
            if let Some(fd) = fd_out {
                emitir_ask(fd, &format!("{msg}\n"));
            }
            if let Some(ref g) = sesion.sys_gpu {
                if fd_out.is_some() {
                    traza_gpu_askd(g);
                } else {
                    g.print_diagnostics();
                }
            }
            1
        }
    }
}
