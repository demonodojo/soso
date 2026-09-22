//! Propietario de sesión residente (T15): carga única, reset por petición.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use libsoso::{println, sys};
use soso_abi as abi;
use soso_llm_core::conversation::ModelProfile;
use soso_llm_core::pipeline::PipelineRole;
use soso_llm_core::plan::{MemoryPlanConfig, ResourcePlanner};
use soso_llm_core::layer::TensorSource;
use soso_llm_core::runtime::{Backend, Runtime};
use soso_llm_core::source::MmapTensorSource;
use soso_llm_core::tokenizer::Tokenizer;
use sosomodel::index::TensorIndex;
use sosomodel::manifest::Manifest;

use crate::distributed::crc_bytes;
use crate::pool::ThreadPool;
use crate::staging::StagedSource;
use crate::{read_file, read_iostat, read_mem_snapshot};

/// Cómo elegir el modelo: ask conserva el fallback histórico; la API exige id exacto.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PoliticaModelo {
    AskHistorica,
    ApiEstricta,
}

pub fn resolver_modelo(
    politica: PoliticaModelo,
    preferido_conf: &str,
    catalogo: &[String],
) -> Option<String> {
    match politica {
        PoliticaModelo::AskHistorica => {
            if !preferido_conf.is_empty() && catalogo.iter().any(|m| m == preferido_conf) {
                return Some(String::from(preferido_conf));
            }
            catalogo.first().cloned()
        }
        PoliticaModelo::ApiEstricta => {
            if preferido_conf.is_empty() {
                return None;
            }
            if catalogo.iter().any(|m| m == preferido_conf) {
                Some(String::from(preferido_conf))
            } else {
                None
            }
        }
    }
}

pub(crate) struct ModelBundle {
    pub(crate) rt: Runtime,
    pub(crate) source: StagedSource,
    pub(crate) tokenizer: Tokenizer,
    pub(crate) manifest_crc: u32,
    pub(crate) index_crc: u32,
}

fn model_base(name: &str) -> String {
    for prefix in ["/models/", "/var/models/"] {
        let p = format!("{prefix}{name}/manifest.som");
        let mut st = soso_abi::Stat::default();
        if sys::stat(&p, &mut st) == 0 {
            return format!("{prefix}{name}");
        }
    }
    format!("/models/{name}")
}

fn read_model_catalog(name: &str) -> Result<(Manifest, TensorIndex, u32, u32), u8> {
    let base = model_base(name);
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
    Ok((manifest, index, manifest_crc, index_crc))
}

fn load_model_from_catalog(
    name: &str,
    manifest: Manifest,
    index: TensorIndex,
    manifest_crc: u32,
    index_crc: u32,
    role: PipelineRole,
    layer_start: u32,
    layer_end: u32,
    staging: bool,
) -> Result<ModelBundle, u8> {
    let base = model_base(name);
    let rt = Runtime::new(manifest, index.clone(), 32 * 1024 * 1024, 0);
    if let Err(why) = rt.validate_shapes_for_role(role, layer_start, layer_end) {
        println!("soso-llm: shapes del index no casan con el rol ({why})");
        return Err(1);
    }

    let tokenizer = match read_file(&format!("{base}/tokenizer.som")) {
        Ok(data) => Tokenizer::parse(&data).map_err(|_| {
            println!("soso-llm: tokenizer.som inválido");
            1u8
        })?,
        Err(_) => Tokenizer::byte_level(),
    };

    let inner = MmapTensorSource::new(format!("{base}/shards"), index, crate::staging::SyscallMapper);
    let inner = if staging {
        inner
    } else {
        inner.with_sync_prefetch()
    };
    let mut source = StagedSource::new(inner);
    if staging {
        source.enable_worker();
    } else {
        source.disable_worker();
    }
    Ok(ModelBundle {
        rt,
        source,
        tokenizer,
        manifest_crc,
        index_crc,
    })
}

pub(crate) fn load_model(
    name: &str,
    role: PipelineRole,
    layer_start: u32,
    layer_end: u32,
    staging: bool,
) -> Result<ModelBundle, u8> {
    let (manifest, index, manifest_crc, index_crc) = read_model_catalog(name)?;
    load_model_from_catalog(
        name,
        manifest,
        index,
        manifest_crc,
        index_crc,
        role,
        layer_start,
        layer_end,
        staging,
    )
}
fn emit_plan_lines(lines: &[String], echo_fd: Option<u64>) {
    let askd = echo_fd.is_some();
    for line in lines {
        traza(askd, line);
    }
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

/// Modelo cargado y listo para generar.
pub struct Sesion {
    pub(crate) bundle: ModelBundle,
    pub(crate) pool: Option<ThreadPool>,
    pub(crate) sys_gpu: Option<soso_gpu::SysGpu>,
    pub(crate) modelo: String,
}

/// Apaga workers de inferencia antes de salir del proceso.
pub(crate) fn liberar_sesion(sesion: &mut Sesion) {
    sesion.liberar();
}

/// Prefetch con progreso en serie (evita minutos sin línea en SOSOLOG).
fn prefetch_shards_logged(
    source: &mut StagedSource,
    shards: &[String],
    echo_fd: Option<u64>,
) {
    let n = shards.len();
    let askd = echo_fd.is_some();
    for (i, shard) in shards.iter().enumerate() {
        let step = i + 1;
        if step == 1 || step == n || step % 8 == 0 {
            traza(askd, &format!("askd: prefetch {step}/{n} {shard}"));
            if let Some(fd) = echo_fd {
                keepalive_dot(fd);
            }
        }
        source.prefetch_shards(&[shard.clone()]);
    }
}

/// Carga el modelo y decide planificador, backend y workers.
///
/// `verboso` apaga el diagnóstico entero: `ask` quiere la respuesta y nada
/// más, y `soso-llm run` sigue contándolo todo.
pub(crate) fn preparar_sesion(
    name: &str,
    force_cpu: bool,
    mem_plan: MemoryPlanConfig,
    verboso: bool,
    with_pool: bool,
) -> Result<Sesion, u8> {
    preparar_sesion_echo(name, force_cpu, mem_plan, verboso, with_pool, None)
}

pub(crate) fn preparar_sesion_echo(
    name: &str,
    force_cpu: bool,
    mem_plan: MemoryPlanConfig,
    verboso: bool,
    with_pool: bool,
    echo_fd: Option<u64>,
) -> Result<Sesion, u8> {
    let t_sess = sys::uptime_ms();
    let askd = echo_fd.is_some();
    let io0 = read_iostat();
    let (manifest, index, manifest_crc, index_crc) = read_model_catalog(name)?;
    let num_layers = manifest.num_layers;

    let mut gpu = abi::GpuInfo::default();
    let _ = sys::gpu_info(&mut gpu);
    let mem = read_mem_snapshot();
    let planner = ResourcePlanner::with_config(
        &manifest,
        &index,
        mem,
        gpu.vram_free,
        false,
        mem_plan,
    );
    emit_plan_lines(&planner.explain_load_plan(&manifest, &index), echo_fd);

    let t_carga = sys::uptime_ms();
    let mut bundle = load_model_from_catalog(
        name,
        manifest,
        index,
        manifest_crc,
        index_crc,
        PipelineRole::Full,
        0,
        num_layers,
        with_pool,
    )?;
    // La carga en frío va aparte de tok/s: `generado` sólo cronometra el
    // decode, y el disco se gasta casi entero antes de que ese reloj arranque.
    // Medirlas juntas es lo que hacía invisible el coste de E/S.
    let carga_ms = (sys::uptime_ms() - t_carga).max(0) as u64;
    if askd {
        traza(true, &format!("askd: catálogo+disco {name} — {carga_ms} ms"));
    }
    let io_carga = read_iostat();
    if verboso {
        println!(
            "soso-llm: carga en frío — {} ms, {} peticiones de disco, {} bloques ({} ms de disco)",
            carga_ms,
            io_carga.peticiones.saturating_sub(io0.peticiones),
            io_carga.bloques.saturating_sub(io0.bloques),
            io_carga.nanos.saturating_sub(io0.nanos) / 1_000_000,
        );
        println!(
            "soso-llm: modelo {} ({} capas, hidden={})",
            bundle.rt.manifest.name, bundle.rt.manifest.num_layers, bundle.rt.manifest.hidden_dim
        );
    }

    bundle.rt.set_planner(planner);
    if verboso {
        if let Some(pl) = bundle.rt.planner.as_ref() {
            println!(
                "soso-llm: planificador (detalle) — preset {:?}, replan cada {} tokens",
                pl.plan_config().preset,
                soso_llm_core::plan::REPLAN_EVERY_TOKENS,
            );
        }
    }
    let t_backend = sys::uptime_ms();
    let mut sys_gpu = if force_cpu { None } else { soso_gpu::SysGpu::new() };
    let backend_ms = (sys::uptime_ms() - t_backend).max(0) as u64;
    traza(
        askd,
        &format!(
            "soso-llm: backend {} (+{} ms)",
            if sys_gpu.is_some() { "GPU" } else { "CPU" },
            backend_ms
        ),
    );
    if askd {
        traza(
            true,
            &format!(
                "askd: backend {} (+{backend_ms} ms)",
                if sys_gpu.is_some() { "GPU" } else { "CPU" }
            ),
        );
    }
    // El `present` del kernel no basta para decidir: un dispositivo puede aceptar
    // búferes y no ejecutar nada (iGPU Intel), y entonces `SysGpu::new` dice no.
    // Anunciar "GPU detectada" mirando sólo `present` era prometer un offload que
    // no iba a ocurrir — y con el dispositivo software, además, mentir.
    if force_cpu {
        if verboso {
            println!("soso-llm: backend CPU (--cpu)");
        }
        bundle.rt.set_backend(Backend::Cpu);
    } else if let Some(ref mut g) = sys_gpu {
        // `gpu_info` inicial puede ver vram_bufs=0 antes de que GSP exponga el
        // pool; `SysGpu::new` ya releyó. Sin esto, eager_vram queda en false y
        // el prefetch por USB (~4,5 GiB) parece un cuelgue tras «backend GPU».
        let _ = sys::gpu_info(&mut gpu);
        let vram_free = gpu.vram_free;
        if verboso {
            println!(
                "soso-llm: dispositivo de cómputo «{}» (fase {}), VRAM libre {} bytes",
                g.device_name(),
                g.phase(),
                vram_free
            );
        }
        bundle.rt.set_backend(Backend::Auto);
        bundle.rt.tiers.vram_budget = vram_free as usize;
        if let Some(pl) = bundle.rt.planner.as_mut() {
            pl.refresh_vram(&bundle.rt.manifest, &bundle.rt.index, vram_free);
        }
        let model_payload = soso_llm_core::plan::total_model_vram_bytes(&bundle.rt.index);
        let model_g6 = soso_llm_core::plan::total_model_vram_g6_budget_bytes(&bundle.rt.index);
        let keep_mapped = bundle
            .rt
            .planner
            .as_ref()
            .is_some_and(|p| p.keep_weights_mapped());
        let eager_vram = gpu.vram_bufs != 0 && vram_free > 0;
        let full_vram = model_g6 > 0 && model_g6 <= vram_free;
        if askd {
            traza(
                true,
                &format!(
                    "askd: VRAM — modelo {} MiB (G6 {} MiB), pool {} MiB, libre {} MiB, tablas {}, eager={}, residente={}",
                    model_payload >> 20,
                    model_g6 >> 20,
                    gpu.vram_pool_free >> 20,
                    vram_free >> 20,
                    gpu.g6_pt_free,
                    eager_vram,
                    full_vram
                ),
            );
        }
        let index = &bundle.rt.index;
        let mut shards = Vec::new();
        for e in &index.entries {
            if !shards.iter().any(|s| s == &e.shard) {
                shards.push(e.shard.clone());
            }
        }
        // Sin eager: prefetch para streaming CPU. Con eager: una pasada calienta
        // el page cache antes de subir (sin ella cada tensor_view faultea el USB).
        if keep_mapped && (!eager_vram || full_vram) && !shards.is_empty() {
            prefetch_shards_logged(&mut bundle.source, &shards, echo_fd);
        }
        if full_vram {
            g.fijar_pesos_residentes();
        }
        if eager_vram {
            let mut gpu_info = abi::GpuInfo::default();
            let (dma0, bounce0) = if sys::gpu_info(&mut gpu_info) == 0 {
                (gpu_info.uploads_dma, gpu_info.uploads_bounce)
            } else {
                (0, 0)
            };
            let t_up = sys::uptime_ms().max(0);
            let uploads0 = g.stats().1;
            let mut bytes_subidos = 0u64;
            let to_upload: Vec<(String, Vec<u32>)> = bundle
                .rt
                .index
                .entries
                .iter()
                .map(|e| (e.name.clone(), e.shape.clone()))
                .collect();
            let n_up = to_upload.len();
            traza(askd, &format!("askd: subida GPU — {n_up} tensores"));
            for (i, (name, shape)) in to_upload.iter().enumerate() {
                let step = i + 1;
                if step == 1 || step == n_up || step % 8 == 0 {
                    let ms = (sys::uptime_ms() - t_up).max(0) as u64;
                    traza(
                        askd,
                        &format!("askd: subida GPU {step}/{n_up} (+{ms} ms) ({name})"),
                    );
                    if let Some(fd) = echo_fd {
                        keepalive_dot(fd);
                    }
                }
                if let Ok(view) = bundle.source.tensor_view(name) {
                    if g.subir_tensor(name, &view, shape) {
                        bytes_subidos =
                            bytes_subidos.saturating_add(view.bytes.len() as u64);
                    }
                }
            }
            let uploaded = g.stats().1.saturating_sub(uploads0);
            let ms_up = (sys::uptime_ms() - t_up).max(0) as u64;
            traza(
                askd,
                &format!("askd: subida GPU fin — {uploaded} ok, {bytes_subidos} B en {ms_up} ms"),
            );
            let (dma, bounce) = if sys::gpu_info(&mut gpu_info) == 0 {
                (
                    gpu_info.uploads_dma.saturating_sub(dma0),
                    gpu_info.uploads_bounce.saturating_sub(bounce0),
                )
            } else {
                (0, 0)
            };
            if askd {
                libsoso::logln!(
                    "soso-llm: subida eager VRAM — {} MiB, {} tensores, {} ms (dma={} bounce={})",
                    bytes_subidos >> 20,
                    uploaded,
                    ms_up,
                    dma,
                    bounce
                );
            } else {
                g.log_subida_eager(uploaded, bytes_subidos, ms_up, dma, bounce);
            }
        }
    } else if gpu.present != 0 {
        if verboso {
            println!(
                "soso-llm: hay GPU («{}») pero no ejecuta kernels; backend CPU",
                libsoso::str_hasta_nul(&gpu.name)
            );
        }
        bundle.rt.set_backend(Backend::Cpu);
    } else {
        if verboso {
            println!("soso-llm: backend CPU");
        }
        bundle.rt.set_backend(Backend::Cpu);
    }

    let pool = if with_pool {
        let pool = ThreadPool::new();
        if verboso {
            println!("soso-llm: workers={}", pool.workers());
        }
        Some(pool)
    } else {
        None
    };
    if askd {
        let total_ms = (sys::uptime_ms() - t_sess).max(0) as u64;
        traza(true, &format!("askd: sesión {name} preparada — {total_ms} ms total"));
    }
    Ok(Sesion {
        bundle,
        pool,
        sys_gpu,
        modelo: String::from(name),
    })
}

impl Sesion {
    /// Reinicia KV/RoPE para una petición nueva sin recargar pesos (C4).
    pub fn reset_peticion(&mut self) {
        self.bundle.rt.reset_sequence();
    }

    pub fn liberar(&mut self) {
        self.bundle.source.shutdown_worker();
        self.pool = None;
    }
}

/// Una carga por proceso; T16 HTTP reutilizará este propietario.
pub struct PropietarioSesion {
    pub sesion: Option<Sesion>,
    pub modelo: String,
}

impl PropietarioSesion {
    pub fn vacio() -> Self {
        Self {
            sesion: None,
            modelo: String::new(),
        }
    }

    pub fn necesita_carga(&self, want: &str) -> bool {
        self.sesion.as_ref().map(|s| s.modelo != want).unwrap_or(true)
    }
}

/// Preparación sin fd del protocolo ask (T16).
pub fn preparar_sesion_api(
    name: &str,
    force_cpu: bool,
    mem_plan: MemoryPlanConfig,
) -> Result<Sesion, u8> {
    preparar_sesion_echo(name, force_cpu, mem_plan, false, true, None)
}

/// Perfil expuesto por `/v1/models` y validación de peticiones API (alias = nombre cargado).
pub fn perfil_api(sesion: &Sesion) -> ModelProfile {
    let m = &sesion.bundle.rt.manifest;
    let base = model_base(&sesion.modelo);
    let family = if m.chat_template.contains("<|im_start|>") {
        String::from("qwen2")
    } else {
        String::new()
    };
    let mut stops = Vec::new();
    if let Some(eos) = sesion.bundle.tokenizer.eos() {
        stops.push(eos);
    }
    let max_out = m.max_seq.min(4096);
    ModelProfile {
        id: sesion.modelo.clone(),
        directory: base,
        family,
        weights_sha256: String::new(),
        tokenizer_sha256: String::new(),
        template_sha256: String::new(),
        context_tokens: m.max_seq,
        max_output_tokens: max_out,
        stop_token_ids: stops,
    }
}
