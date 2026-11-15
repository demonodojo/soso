//! Servidor HTTP de desarrollo en host (T13, C3–C4). Solo con `feature = "std"`.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};
use sosomodel::index::TensorIndex;
use sosomodel::manifest::Manifest;
use soso_llm_core::conversation::gramatica::restriccion_de_herramientas;
use soso_llm_core::conversation::salida::interpretar_ids;
use soso_llm_core::restriccion::TablaBytes;
use soso_llm_core::conversation::{
    parse_assistant_output_familia, render_messages, ChatError, ModelProfile, ToolChoice,
};
use soso_llm_core::generation::{
    GenCheckpoint, GenerationObserver, GenerationOptions, GenerationReport,
};
use soso_llm_core::layer::TensorSource;
use soso_llm_core::runtime::Runtime;
use soso_llm_core::sample::Sampler;
use soso_llm_core::source::{FileMapper, MappedShard, MmapTensorSource};
use soso_llm_core::tokenizer::Tokenizer;
use soso_llm_core::ThreadStagedSource;

use crate::{
    encode_api_error_json, encode_chat_completion_json, encode_stream_failure,
    encode_success_stream, format_response, format_sse_response_headers, prepare_chat_completion,
    text_delta_splits, ApiError, CompletionPayload, EncodeError, FeedOutcome, HttpError,
    HttpReader, HttpRequest, PreparedChatCompletion,
};

/// Error al cargar pesos o perfil en el arranque real.
#[derive(Debug)]
pub enum LoadError {
    Io(String),
    Perfil(String),
    Modelo(String),
}

impl core::fmt::Display for LoadError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            LoadError::Io(s) | LoadError::Perfil(s) | LoadError::Modelo(s) => f.write_str(s),
        }
    }
}

/// Errores del servicio HTTP (incluye ocupado y auth).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServiceError {
    Api(ApiError),
    Unauthorized,
    NotFound,
    MethodNotAllowed,
    Http(HttpError),
    Busy,
    Inferencia,
    Encode(EncodeError),
    Io(String),
}

pub fn sha256_hex(data: &[u8]) -> String {
    let digest = Sha256::digest(data);
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// `FileMapper` del host. Por defecto **mapea** los shards con `mmap` de sólo lectura (los pesos viven en la
/// caché de páginas: se comparten entre procesos y el kernel puede recuperarlos bajo presión, en vez de que cada
/// proceso copie el modelo entero a memoria anónima y el OOM killer mate a uno). `SOSO_LEER_SHARDS=1` vuelve a
/// leer cada shard a un búfer propio (con liberación real, no leak como `hostrun`).
pub struct OwnedMapper {
    slabs: HashMap<u64, Box<[u8]>>,
    mmap: crate::host_mmap::MmapMapper,
    leer: bool,
}

impl Default for OwnedMapper {
    fn default() -> Self {
        Self {
            slabs: HashMap::new(),
            mmap: crate::host_mmap::MmapMapper::default(),
            leer: std::env::var_os("SOSO_LEER_SHARDS").is_some(),
        }
    }
}

impl FileMapper for OwnedMapper {
    fn map_file(&mut self, path: &str) -> Result<MappedShard, ()> {
        if !self.leer {
            return self.mmap.map_file(path);
        }
        let data = std::fs::read(path).map_err(|_| ())?;
        let len = data.len();
        let boxed = data.into_boxed_slice();
        let addr = boxed.as_ptr() as u64;
        self.slabs.insert(addr, boxed);
        Ok(MappedShard { addr, len })
    }

    fn unmap_file(&mut self, shard: &MappedShard) {
        if self.slabs.remove(&shard.addr).is_none() {
            self.mmap.unmap_file(shard);
        }
    }
}

/// Inferencia: una sesión residente; cada petición reinicia KV.
pub trait ChatBackend: Send {
    fn backend_name(&self) -> &'static str;
    fn profile(&self) -> &ModelProfile;
    fn tokenizer(&self) -> &Tokenizer;
    /// Reinicia contexto KV antes de cada generación.
    fn begin_request(&mut self);
    fn generate_observed(
        &mut self,
        prompt_ids: &[u32],
        prepared: &PreparedChatCompletion,
        observer: &mut dyn GenerationObserver,
    ) -> Result<(Vec<u32>, GenerationReport), ()>;

    /// Decisión Kev (`POST /v1/systemone`); por defecto no implementado.
    fn systemone_json(&mut self, _body: &str) -> Result<String, String> {
        Err(String::from("backend no soporta /v1/systemone"))
    }
}

enum RunSource {
    Plain(MmapTensorSource<OwnedMapper>),
    Staged(ThreadStagedSource<OwnedMapper>),
}

impl TensorSource for RunSource {
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

    fn tensor_view(
        &mut self,
        name: &str,
    ) -> Result<soso_llm_core::layer::TensorView<'_>, ()> {
        match self {
            Self::Plain(s) => s.tensor_view(name),
            Self::Staged(s) => s.tensor_view(name),
        }
    }

    fn prefetch_shards(&mut self, shards: &[alloc::string::String]) {
        match self {
            Self::Plain(s) => s.prefetch_shards(shards),
            Self::Staged(s) => s.prefetch_shards(shards),
        }
    }

    fn kick_prefetch_shards(&mut self, shards: &[alloc::string::String]) {
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

    fn kick_moe_prefetch(&mut self, shards: &[alloc::string::String]) {
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

    fn release_shards_except(&mut self, keep: &[alloc::string::String]) {
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

/// Runtime real sobre CPU host.
pub struct CpuBackend {
    pub rt: Runtime,
    /// Hilos para repartir las filas del matvec, la atención y la FFN (`None` = un hilo).
    pub parallel: Option<std::sync::Arc<crate::host_pool::HostPool>>,
    /// Aceleración por GPU (p. ej. [`crate::cuda::CudaGpu`]); `None` = todo en CPU.
    pub gpu: Option<Box<dyn soso_llm_core::gpu::GpuDispatch + Send>>,
    source: RunSource,
    profile: ModelProfile,
    tokenizer: Tokenizer,
    /// Bytes de cada token, para restringir las llamadas a herramienta por gramática (se calcula una vez).
    tabla_bytes: Arc<TablaBytes>,
    /// Prompt de la petición anterior (sólo para el diagnóstico de cuánto prefijo se comparte).
    ultimo_prompt: Vec<u32>,
    cache_kv: Option<CacheKvHost>,
    model_dir: std::path::PathBuf,
}

struct CacheKvHost {
    directorio: std::path::PathBuf,
    perfil: [u8; 32],
    max_bytes: usize,
}

impl CacheKvHost {
    fn recuperar(&self, rt: &mut Runtime, prompt: &[u32]) -> Result<bool, String> {
        let candidatos = crate::kvstore_host::listar(&self.directorio).map_err(|e| e.to_string())?;
        let mut mejor = rt.prefijo_kv_en_memoria(prompt);
        let mut elegida = None;
        for ruta in candidatos {
            let imagen = match crate::kvstore_host::leer(&ruta, self.max_bytes) {
                Ok(i) => i,
                Err(_) => continue,
            };
            let prefijo = match rt.prefijo_imagen_kv(&imagen, &self.perfil, prompt, self.max_bytes) {
                Ok(p) => p,
                Err(_) => continue,
            };
            if prefijo > mejor {
                mejor = prefijo;
                elegida = Some(imagen);
            }
        }
        let Some(imagen) = elegida else { return Ok(false); };
        rt.importar_kv(&imagen, &self.perfil, self.max_bytes)
            .map_err(|e| format!("restaurar KV: {e:?}"))?;
        Ok(true)
    }

    fn guardar(&self, rt: &Runtime) -> Result<(), String> {
        let imagen = rt.exportar_kv(&self.perfil, self.max_bytes).map_err(|e| format!("exportar KV: {e:?}"))?;
        crate::kvstore_host::guardar_multiple(&self.directorio, &imagen, self.max_bytes).map_err(|e| e.to_string())
    }
}

impl CpuBackend {
    /// Habilita una caché persistente para una única identidad en un directorio
    /// privado ya existente. `ejecucion` debe identificar binario y opciones que
    /// afecten el KV; los archivos del modelo deben permanecer inmutables.
    pub fn habilitar_cache_kv(&mut self, directorio: &std::path::Path,
                              ejecucion: &[u8], max_bytes: usize) -> Result<(), String> {
        if self.gpu.is_some() || max_bytes == 0 { return Err("la caché requiere CPU y límite positivo".into()); }
        crate::kvstore_host::verificar_directorio_privado(directorio).map_err(|e| e.to_string())?;
        let directorio = directorio.canonicalize().map_err(|e| e.to_string())?;
        let perfil = crate::identidad_kv::calcular(&self.model_dir, ejecucion).map_err(|e| e.to_string())?;
        self.cache_kv = Some(CacheKvHost { directorio, perfil, max_bytes });
        Ok(())
    }
}

pub fn load_cpu_backend(model_dir: &str, profile: ModelProfile) -> Result<CpuBackend, LoadError> {
    if model_dir.is_empty() {
        return Err(LoadError::Perfil(String::from("model-dir vacío")));
    }
    if profile.directory != model_dir && !profile.directory.is_empty() {
        // El CLI puede fijar el directorio; el id estable sigue en profile.id.
    }
    let manifest_path = format!("{model_dir}/manifest.som");
    let index_path = format!("{model_dir}/index.som");
    let tokenizer_path = format!("{model_dir}/tokenizer.som");

    let index_bytes = std::fs::read(&index_path)
        .map_err(|e| LoadError::Io(format!("index.som: {e}")))?;
    let tok_bytes = std::fs::read(&tokenizer_path)
        .map_err(|e| LoadError::Io(format!("tokenizer.som: {e}")))?;

    let index_hash = sha256_hex(&index_bytes);
    let tok_hash = sha256_hex(&tok_bytes);
    if index_hash != profile.weights_sha256 {
        return Err(LoadError::Perfil(format!(
            "hash index.som {index_hash} != perfil.weights_sha256 {}",
            profile.weights_sha256
        )));
    }
    if tok_hash != profile.tokenizer_sha256 {
        return Err(LoadError::Perfil(format!(
            "hash tokenizer.som {tok_hash} != perfil.tokenizer_sha256 {}",
            profile.tokenizer_sha256
        )));
    }

    let manifest = Manifest::parse(
        &std::fs::read(&manifest_path).map_err(|e| LoadError::Io(e.to_string()))?,
    )
    .map_err(|_| LoadError::Modelo(String::from("manifest.som inválido")))?;
    let index = TensorIndex::parse(&index_bytes)
        .map_err(|_| LoadError::Modelo(String::from("index.som inválido")))?;
    let tokenizer = Tokenizer::parse(&tok_bytes)
        .map_err(|_| LoadError::Modelo(String::from("tokenizer.som inválido")))?;

    let mut rt = Runtime::new(manifest, index.clone(), 0, 0);
    // El reloj deja que la política del borrador mida lo que cuesta cada ronda (`especulativo`).
    rt.reloj = Some(reloj_ms);
    rt.validate_shapes()
        .map_err(|_| LoadError::Modelo(String::from("shapes inválidas")))?;

    let base = MmapTensorSource::new(format!("{model_dir}/shards"), index, OwnedMapper::default());
    let source = if std::env::var("SOSO_SYNC_STAGING").is_ok() {
        RunSource::Plain(base.with_sync_prefetch())
    } else {
        RunSource::Staged(ThreadStagedSource::new(base))
    };

    let tabla_bytes = Arc::new(tokenizer.tabla_bytes());
    Ok(CpuBackend {
        ultimo_prompt: Vec::new(),
        cache_kv: None,
        model_dir: std::path::PathBuf::from(model_dir),
        rt,
        parallel: None,
        gpu: None,
        source,
        profile,
        tokenizer,
        tabla_bytes,
    })
}

/// ¿Termina el prompt dentro de un razonamiento abierto (`<think>`, canal de Gemma)? El aviso de generación de
/// algunas plantillas lo deja así, y lo que el modelo escriba hasta cerrarlo no es respuesta ni llamada.
fn razonamiento_abierto(tok: &Tokenizer, prompt_ids: &[u32]) -> bool {
    let cola = &prompt_ids[prompt_ids.len().saturating_sub(24)..];
    let texto = tok.decode(cola);
    let abierto = |abre: &str, cierra: &str| match (texto.rfind(abre), texto.rfind(cierra)) {
        (Some(a), Some(c)) => a > c,
        (Some(_), None) => true,
        _ => false,
    };
    abierto("<think>", "</think>") || abierto("<|channel>", "<channel|>")
}

/// Milisegundos monotónicos desde el primer uso (el origen no importa: sólo se restan).
fn reloj_ms() -> u64 {
    use std::sync::OnceLock;
    use std::time::Instant;
    static ORIGEN: OnceLock<Instant> = OnceLock::new();
    ORIGEN.get_or_init(Instant::now).elapsed().as_millis() as u64
}

/// Observa sin alterar la entrega ni los puntos de cancelación del cliente.
struct MedidorGeneracion<'a> {
    destino: &'a mut dyn GenerationObserver,
    inicio: std::time::Instant,
    inicio_generacion: std::time::Instant,
    activo: bool,
    prefill: Option<std::time::Duration>,
    tokens: Vec<std::time::Duration>,
}

#[derive(Default)]
struct MedicionCache {
    lectura_s: Option<f64>,
    escritura_s: Option<f64>,
    restaurada: bool,
}

impl GenerationObserver for MedidorGeneracion<'_> {
    fn on_token(&mut self, token: u32) {
        if self.activo { self.tokens.push(self.inicio.elapsed()); }
        self.destino.on_token(token);
    }

    fn cancel_at(&mut self, at: GenCheckpoint) -> bool {
        if self.activo && matches!(at, GenCheckpoint::BeforeDecode) {
            self.prefill = Some(self.inicio_generacion.elapsed());
        }
        self.destino.cancel_at(at)
    }
}

impl MedidorGeneracion<'_> {
    fn json(&self, rt: &Runtime, prompt: usize, inicio: (u64, u64, u64), gpu: Option<(soso_llm_core::gpu::GpuStats, soso_llm_core::gpu::GpuStats)>, cache: Option<&MedicionCache>) -> serde_json::Value {
        let mut intervalos: Vec<f64> = self.tokens.windows(2)
            .map(|w| (w[1] - w[0]).as_secs_f64()).collect();
        intervalos.sort_by(f64::total_cmp);
        let percentil = |p: usize| intervalos.get((intervalos.len() * p).div_ceil(100).saturating_sub(1)).copied();
        let decode_s = self.tokens.first().zip(self.tokens.last()).map(|(a, b)| (*b - *a).as_secs_f64());
        serde_json::json!({
            "prompt_tokens": prompt,
            "reused_prompt_tokens": rt.ultimo_prefijo_reutilizado,
            "prefill_tokens": prompt.saturating_sub(rt.ultimo_prefijo_reutilizado),
            "generated_tokens": self.tokens.len(),
            "prefill_s": self.prefill.map(|d| d.as_secs_f64()),
            "ttft_s": self.tokens.first().map(|d| d.as_secs_f64()),
            "decode_tokens_s": decode_s.filter(|&s| s > 0.0 && self.tokens.len() > 1)
                .map(|s| (self.tokens.len() - 1) as f64 / s),
            "inter_token_p50_s": percentil(50), "inter_token_p95_s": percentil(95),
            "total_s": self.inicio.elapsed().as_secs_f64(),
            "decode_forwards": rt.forwards_decode.saturating_sub(inicio.0),
            "draft_offered": rt.borrador_ofrecidos.saturating_sub(inicio.1),
            "draft_accepted": rt.borrador_aceptados.saturating_sub(inicio.2),
            "cache_kv": cache.map(|c| serde_json::json!({"lectura_s": c.lectura_s, "escritura_s": c.escritura_s, "restaurada": c.restaurada})),
            "cuda_decode_transferencias": gpu.map(|(a, b)| serde_json::json!({
                "pasos": b.decode_pasos.saturating_sub(a.decode_pasos),
                "embed_htod_bytes": b.decode_embed_up.saturating_sub(a.decode_embed_up),
                "control_htod_bytes": b.decode_control_up.saturating_sub(a.decode_control_up),
                "logits_dtoh_bytes": b.decode_logits_down.saturating_sub(a.decode_logits_down),
                "kv_dtoh_bytes": b.decode_kv_down.saturating_sub(a.decode_kv_down),
            })),
        })
    }
}

impl ChatBackend for CpuBackend {
    fn backend_name(&self) -> &'static str {
        "host-cpu"
    }

    fn profile(&self) -> &ModelProfile {
        &self.profile
    }

    fn tokenizer(&self) -> &Tokenizer {
        &self.tokenizer
    }

    fn begin_request(&mut self) {
        // No reinicia la caché K/V: la generación reutiliza el prefijo común con
        // la anterior y se reinicia sola si no comparte nada (T22).
    }

    fn generate_observed(
        &mut self,
        prompt_ids: &[u32],
        prepared: &PreparedChatCompletion,
        observer: &mut dyn GenerationObserver,
    ) -> Result<(Vec<u32>, GenerationReport), ()> {
        let t_inicio = std::time::Instant::now();
        let mut medicion_cache = self.cache_kv.as_ref().filter(|_| self.gpu.is_none()).map(|_| MedicionCache::default());
        if self.gpu.is_none() {
            if let Some(cache) = &self.cache_kv {
                let inicio = std::time::Instant::now();
                let resultado = cache.recuperar(&mut self.rt, prompt_ids);
                if let Some(m) = medicion_cache.as_mut() {
                    m.lectura_s = Some(inicio.elapsed().as_secs_f64());
                    m.restaurada = matches!(resultado, Ok(true));
                }
                if let Err(e) = resultado { eprintln!("[soso] caché KV no reutilizada: {e}"); }
            }
        }
        let stops = self.profile.stop_token_ids.clone();
        // Con herramientas declaradas, la llamada que escriba el modelo sólo puede ser una bien formada de la
        // familia (nombre declarado, claves del esquema, JSON/etiquetas cerradas). Fuera de una llamada el
        // muestreo es el de siempre.
        let restriccion = restriccion_de_herramientas(
            &self.profile.family,
            &prepared.input,
            self.tabla_bytes.clone(),
            razonamiento_abierto(&self.tokenizer, prompt_ids),
        );
        let options = GenerationOptions::new(prepared.max_new_tokens as usize, stops).con_restriccion(restriccion);
        let seed = prepared.seed.unwrap_or(42) as u64;
        let mut sampler = Sampler::new(
            prepared.temperature as f32,
            prepared.top_p as f32,
            seed,
        );
        let gpu_previo = self.gpu.as_ref().map(|g| g.stats());
        let mut gpu: Option<&mut dyn soso_llm_core::gpu::GpuDispatch> =
            self.gpu.as_mut().map(|b| &mut **b as &mut dyn soso_llm_core::gpu::GpuDispatch);
        let perfilar = std::env::var_os("SOSO_PERFIL").is_some();
        if perfilar {
            fn reloj_ns() -> u64 {
                use std::sync::OnceLock;
                static T0: OnceLock<std::time::Instant> = OnceLock::new();
                T0.get_or_init(std::time::Instant::now).elapsed().as_nanos() as u64 + 1
            }
            soso_llm_core::layer::perfil_activar(Some(reloj_ns));
        }
        let comun = self.ultimo_prompt.iter().zip(prompt_ids).take_while(|(a, b)| a == b).count();
        eprintln!(
            "[soso] petición: prompt de {} tokens ({} en común con la anterior), hasta {} nuevos",
            prompt_ids.len(),
            comun,
            prepared.max_new_tokens
        );
        self.ultimo_prompt = prompt_ids.to_vec();
        let previo = (self.rt.forwards_decode, self.rt.borrador_ofrecidos, self.rt.borrador_aceptados);
        let mut medidor = MedidorGeneracion {
            destino: observer, inicio: t_inicio, inicio_generacion: std::time::Instant::now(),
            activo: std::env::var_os("SOSO_BENCH_JSON").is_some(),
            prefill: None, tokens: Vec::new(),
        };
        let (secuencia, report) = self.rt.generate_stream_par_observed(
            &mut self.source,
            prompt_ids,
            options,
            &mut sampler,
            &mut medidor,
            self.parallel.as_deref().map(|p| p as &dyn soso_llm_core::parallel::RowParallel),
            &mut gpu,
            &mut None,
        )?;
        if self.gpu.is_none() && !matches!(report.stop, soso_llm_core::generation::StopReason::Cancelled) {
            if let Some(cache) = &self.cache_kv {
                let inicio = std::time::Instant::now();
                let resultado = cache.guardar(&self.rt);
                if let Some(m) = medicion_cache.as_mut() { m.escritura_s = Some(inicio.elapsed().as_secs_f64()); }
                if let Err(e) = resultado {
                    eprintln!("[soso] caché KV no guardada: {e}");
                }
            }
        }
        if medidor.activo {
            let gpu_posterior = self.gpu.as_ref().map(|g| g.stats());
            eprintln!("[soso] medicion: {}", medidor.json(&self.rt, prompt_ids.len(), previo, gpu_previo.zip(gpu_posterior), medicion_cache.as_ref()));
        }
        // `ChatBackend` devuelve sólo lo generado. El runtime devuelve la secuencia
        // entera (prompt + generación): sin recortarla el parser de llamadas leía el
        // prompt, con sus `<tool_call>` de ejemplo, como si fuera la salida del modelo.
        eprintln!(
            "[soso] respuesta: {} tokens generados en {:.1} s (en total, con el prefill)",
            secuencia.len().saturating_sub(prompt_ids.len()),
            t_inicio.elapsed().as_secs_f64()
        );
        if let Some(g) = self.gpu.as_ref() {
            let st = soso_llm_core::gpu::GpuDispatch::stats(&**g);
            eprintln!("[soso] expertos en GPU: {} aciertos, {} fallos (acumulado)", st.flujo_aciertos, st.flujo_fallos);
        }
        if perfilar {
            let f = soso_llm_core::layer::perfil_fases_ns();
            let partes: Vec<String> = soso_llm_core::layer::NOMBRES_FASES.iter().zip(f).map(|(n, v)| format!("{n} {:.1}", v as f64 / 1e9)).collect();
            eprintln!("[soso] fases (s): {}", partes.join(", "));
        }
        Ok((solo_generados(&secuencia, prompt_ids.len()), report))
    }

    fn systemone_json(&mut self, body: &str) -> Result<String, String> {
        crate::systemone::run_systemone(body, &mut self.rt, &self.tokenizer, &mut self.source)
        .map_err(|e| match e {
            crate::systemone::SystemOneError::Json(m)
            | crate::systemone::SystemOneError::Modelo(m)
            | crate::systemone::SystemOneError::Inferencia(m) => m,
        })
    }
}

/// Quita el prompt del principio de una secuencia devuelta por el runtime.
///
/// Por longitud y no buscando un separador: los ids del prompt son exactamente los
/// que se pasaron, y un separador podría aparecer también en lo generado.
pub fn solo_generados(secuencia: &[u32], prompt_len: usize) -> Vec<u32> {
    secuencia.get(prompt_len..).unwrap_or(&[]).to_vec()
}

/// Observador que propaga cancelación externa (desconexión / escritura fallida).
pub struct CancelBridge {
    pub flag: Arc<AtomicBool>,
}

impl GenerationObserver for CancelBridge {
    fn cancel_at(&mut self, _: GenCheckpoint) -> bool {
        self.flag.load(Ordering::Acquire)
    }
}

struct WriteFailGuard<'a, W: Write> {
    inner: &'a mut W,
    cancel: Arc<AtomicBool>,
}

impl<W: Write> Write for WriteFailGuard<'_, W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self.inner.write(buf) {
            Ok(n) => Ok(n),
            Err(e) => {
                self.cancel.store(true, Ordering::Release);
                Err(e)
            }
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

pub struct HostService<B: ChatBackend + Send> {
    token: String,
    busy: AtomicBool,
    backend: Mutex<B>,
}

impl<B: ChatBackend + Send> HostService<B> {
    pub fn new(token: String, backend: B) -> Self {
        Self {
            token,
            busy: AtomicBool::new(false),
            backend: Mutex::new(backend),
        }
    }

    pub fn try_acquire_busy(&self) -> bool {
        self.busy
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }

    pub fn release_busy(&self) {
        self.busy.store(false, Ordering::Release);
    }

    pub fn handle_connection(&self, stream: TcpStream) {
        let mut stream = stream;
        self.dispatch(&mut stream);
    }

    fn dispatch(&self, stream: &mut TcpStream) {
        let req = match read_http_request(stream) {
            Ok(r) => r,
            Err(e) => {
                let _ = stream.write_all(&error_response(&e));
                return;
            }
        };
        match self.route(req, stream) {
            Ok(resp) if !resp.is_empty() => {
                let _ = stream.write_all(&resp);
            }
            Ok(_) => {}
            Err(e) => {
                let _ = stream.write_all(&error_response(&e));
            }
        }
    }

    fn route(&self, req: HttpRequest, stream: &mut TcpStream) -> Result<Vec<u8>, ServiceError> {
        if !check_bearer(&req, &self.token) {
            return Ok(unauthorized_body());
        }
        match (req.method.as_str(), req.path.as_str()) {
            ("GET", "/health") => {
                let b = self.backend.lock().unwrap();
                Ok(health_body(&b.profile().id, b.backend_name()))
            }
            ("GET", "/v1/models") => {
                let id = self.backend.lock().unwrap().profile().id.clone();
                Ok(models_body(&id))
            }
            ("POST", "/v1/chat/completions") => self.post_chat(req, stream),
            (_, "/v1/chat/completions") => Err(ServiceError::MethodNotAllowed),
            ("POST", "/v1/systemone") => self.post_systemone(req),
            (_, "/v1/systemone") => Err(ServiceError::MethodNotAllowed),
            _ => Err(ServiceError::NotFound),
        }
    }

    fn post_chat(
        &self,
        req: HttpRequest,
        stream: &mut TcpStream,
    ) -> Result<Vec<u8>, ServiceError> {
        // Igual que en el guest: un contexto que no cabe es 422, aunque haya
        // otra generación en curso. Ocupar el hueco antes de mirar el cuerpo
        // convertía ese rechazo en 429.
        let body = std::str::from_utf8(&req.body)
            .map_err(|_| ServiceError::Api(ApiError::PeticionInvalida {
                motivo: String::from("cuerpo no UTF-8"),
            }))?;

        let (prepared, prompt_ids, profile) = {
            // Una generación en curso tiene tomado el backend: esperar aquí
            // atendía la petición cuando acababa la otra (200) en vez de
            // contestar 429, que es lo que dice C3 (T21: OpenCode manda a la vez
            // la petición principal y la de título). Con el backend libre, o
            // cogido sólo un instante por el cierre de otra petición, se espera.
            let b = match self.backend.try_lock() {
                Ok(b) => b,
                Err(_) if self.busy.load(Ordering::Acquire) => return Ok(busy_body()),
                Err(_) => self.backend.lock().unwrap(),
            };
            let profile = b.profile().clone();
            let prepared = prepare_chat_completion(body, &profile, b.tokenizer())
                .map_err(ServiceError::Api)?;
            let prompt_ids = render_messages(&prepared.input, &profile, b.tokenizer())
                .map_err(|e| ServiceError::Api(e.into()))?;
            (prepared, prompt_ids, profile)
        };

        if !self.try_acquire_busy() {
            return Ok(busy_body());
        }
        struct Guard<'a, B: ChatBackend + Send> {
            svc: &'a HostService<B>,
        }
        impl<B: ChatBackend + Send> Drop for Guard<'_, B> {
            fn drop(&mut self) {
                if let Ok(mut b) = self.svc.backend.lock() {
                    b.begin_request();
                }
                self.svc.release_busy();
            }
        }
        let _guard = Guard { svc: self };

        if prepared.stream {
            return self.post_chat_stream(stream, prepared, prompt_ids, profile);
        }

        let cancel = Arc::new(AtomicBool::new(false));
        let mut bridge = CancelBridge {
            flag: Arc::clone(&cancel),
        };

        let (token_ids, report) = {
            let mut backend = self.backend.lock().unwrap();
            backend.begin_request();
            backend
                .generate_observed(&prompt_ids, &prepared, &mut bridge)
                .map_err(|_| ServiceError::Inferencia)?
        };

        if report.stop == soso_llm_core::generation::StopReason::Cancelled {
            return Err(ServiceError::Inferencia);
        }

        let (generated_text, turn) = {
            let b = self.backend.lock().unwrap();
            let texto = b.tokenizer().decode(&token_ids);
            // Un marcador emitido como token no se confunde con el mismo texto escrito dentro de un argumento.
            let turno = interpretar_ids(b.tokenizer(), &token_ids, |t| {
                parse_assistant_output_familia(
                    &prepared.input,
                    t,
                    prepared.input.siguiente_id_llamada(),
                    &profile.family,
                )
            });
            (texto, turno)
        };
        let turn = turn.map_err(|e| {
            eprintln!(
                "serve: salida rechazada ({e:?}): {:?}",
                generated_text.chars().take(1500).collect::<String>()
            );
            ServiceError::Api(e.into())
        })?;

        validate_tool_choice(&prepared, &turn)?;

        let payload = completion_payload(&prepared, &profile, turn, report);
        let json = encode_chat_completion_json(&payload).map_err(ServiceError::Encode)?;
        Ok(format_response(
            200,
            &[("Content-Type", "application/json")],
            &json,
        ))
    }

    fn post_systemone(&self, req: HttpRequest) -> Result<Vec<u8>, ServiceError> {
        let body = std::str::from_utf8(&req.body)
            .map_err(|_| ServiceError::Api(ApiError::PeticionInvalida {
                motivo: String::from("cuerpo no UTF-8"),
            }))?;
        let mut backend = self.backend.lock().unwrap();
        backend.begin_request();
        let json = backend
            .systemone_json(body)
            .map_err(|m| ServiceError::Api(ApiError::PeticionInvalida { motivo: m }))?;
        Ok(format_response(
            200,
            &[("Content-Type", "application/json")],
            json.as_bytes(),
        ))
    }

    fn post_chat_stream(
        &self,
        stream: &mut TcpStream,
        prepared: PreparedChatCompletion,
        prompt_ids: Vec<u32>,
        profile: ModelProfile,
    ) -> Result<Vec<u8>, ServiceError> {
        let cancel = Arc::new(AtomicBool::new(false));
        let mut bridge = CancelBridge {
            flag: Arc::clone(&cancel),
        };

        let mut writer = WriteFailGuard {
            inner: stream,
            cancel: Arc::clone(&cancel),
        };
        writer
            .write_all(&format_sse_response_headers(200))
            .map_err(|e| ServiceError::Io(e.to_string()))?;

        let (token_ids, report) = {
            let mut backend = self.backend.lock().unwrap();
            backend.begin_request();
            backend
                .generate_observed(&prompt_ids, &prepared, &mut bridge)
                .map_err(|_| ServiceError::Inferencia)?
        };

        if cancel.load(Ordering::Acquire)
            || report.stop == soso_llm_core::generation::StopReason::Cancelled
        {
            let events = encode_stream_failure("generacion cancelada", "cancelled");
            for e in events {
                let _ = writer.write_all(&e);
            }
            return Ok(Vec::new());
        }

        let (generated_text, turn) = {
            let b = self.backend.lock().unwrap();
            let texto = b.tokenizer().decode(&token_ids);
            // Un marcador emitido como token no se confunde con el mismo texto escrito dentro de un argumento.
            let turno = interpretar_ids(b.tokenizer(), &token_ids, |t| {
                parse_assistant_output_familia(
                    &prepared.input,
                    t,
                    prepared.input.siguiente_id_llamada(),
                    &profile.family,
                )
            });
            (texto, turno)
        };
        let turn = turn.map_err(|e| {
            eprintln!(
                "serve: salida rechazada ({e:?}): {:?}",
                generated_text.chars().take(1500).collect::<String>()
            );
            ServiceError::Api(e.into())
        })?;
        validate_tool_choice(&prepared, &turn)?;

        let payload = completion_payload(&prepared, &profile, turn, report);
        let text = payload.content.as_deref().unwrap_or("");
        let splits = if text.is_empty() {
            Vec::new()
        } else {
            text_delta_splits(text, &[text.len()])
        };
        let events = encode_success_stream(&payload, &splits, prepared.include_usage)
            .map_err(ServiceError::Encode)?;
        for e in events {
            writer
                .write_all(&e)
                .map_err(|e| ServiceError::Io(e.to_string()))?;
        }

        Ok(Vec::new())
    }
}

pub fn run_accept_loop<B: ChatBackend + Send + 'static>(
    service: Arc<HostService<B>>,
    listener: TcpListener,
) -> Result<(), std::io::Error> {
    for conn in listener.incoming() {
        let stream = conn?;
        let svc = Arc::clone(&service);
        std::thread::spawn(move || svc.handle_connection(stream));
    }
    Ok(())
}

fn read_http_request(stream: &mut TcpStream) -> Result<HttpRequest, ServiceError> {
    let mut reader = HttpReader::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = stream
            .read(&mut buf)
            .map_err(|e| ServiceError::Io(e.to_string()))?;
        if n == 0 {
            return match reader.signal_eof() {
                Ok(FeedOutcome::Complete(req)) => Ok(req),
                _ => Err(ServiceError::Http(HttpError::IncompleteBody)),
            };
        }
        match reader
            .feed(&buf[..n])
            .map_err(ServiceError::Http)?
        {
            FeedOutcome::Complete(req) => return Ok(req),
            FeedOutcome::NeedMore => continue,
        }
    }
}

fn check_bearer(req: &HttpRequest, token: &str) -> bool {
    if token.is_empty() {
        return false;
    }
    for (k, v) in &req.headers {
        if k.eq_ignore_ascii_case("authorization") {
            let v = v.trim();
            if let Some(rest) = v.strip_prefix("Bearer ") {
                return rest.trim() == token;
            }
            if let Some(rest) = v.strip_prefix("bearer ") {
                return rest.trim() == token;
            }
        }
    }
    false
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn completion_id() -> String {
    format!("chatcmpl-{}", unix_now())
}

fn completion_payload(
    prepared: &PreparedChatCompletion,
    _profile: &ModelProfile,
    turn: soso_llm_core::conversation::AssistantTurn,
    report: GenerationReport,
) -> CompletionPayload {
    CompletionPayload {
        id: completion_id(),
        model: prepared.model_id.clone(),
        created: unix_now(),
        content: turn.content,
        tool_calls: turn
            .tool_call
            .map(|c| vec![c])
            .unwrap_or_default(),
        report,
    }
}

fn validate_tool_choice(
    prepared: &PreparedChatCompletion,
    turn: &soso_llm_core::conversation::AssistantTurn,
) -> Result<(), ServiceError> {
    match prepared.input.tool_choice {
        ToolChoice::Required | ToolChoice::Named(_) if turn.tool_call.is_none() => {
            Err(ServiceError::Api(ApiError::Dominio(
                ChatError::SeleccionInvalida {
                    motivo: String::from("tool_choice exige una llamada válida"),
                },
            )))
        }
        _ => Ok(()),
    }
}

fn health_body(model: &str, backend: &str) -> Vec<u8> {
    let body = format!(
        r#"{{"status":"ready","model":"{model}","backend":"{backend}"}}"#
    );
    format_response(200, &[("Content-Type", "application/json")], body.as_bytes())
}

fn models_body(model_id: &str) -> Vec<u8> {
    let body = format!(
        r#"{{"object":"list","data":[{{"id":"{model_id}","object":"model"}}]}}"#
    );
    format_response(200, &[("Content-Type", "application/json")], body.as_bytes())
}

fn unauthorized_body() -> Vec<u8> {
    let body = "{\"error\":{\"message\":\"autenticacion invalida\",\"type\":\"invalid_request_error\",\"code\":\"invalid_api_key\"}}";
    format_response(401, &[("Content-Type", "application/json")], body.as_bytes())
}

fn busy_body() -> Vec<u8> {
    let body = "{\"error\":{\"message\":\"generacion en curso\",\"type\":\"invalid_request_error\",\"code\":\"busy\"}}";
    format_response(429, &[("Content-Type", "application/json")], body.as_bytes())
}

fn error_response(err: &ServiceError) -> Vec<u8> {
    match err {
        ServiceError::Api(e) => format_response(
            e.status_code(),
            &[("Content-Type", "application/json")],
            &encode_api_error_json(e),
        ),
        ServiceError::Unauthorized => unauthorized_body(),
        ServiceError::NotFound => format_response(
            404,
            &[("Content-Type", "application/json")],
            br#"{"error":{"message":"ruta desconocida","type":"not_found_error","code":"not_found"}}"#,
        ),
        ServiceError::MethodNotAllowed => format_response(
            405,
            &[("Content-Type", "application/json")],
            br#"{"error":{"message":"metodo no permitido","type":"invalid_request_error","code":"method_not_allowed"}}"#,
        ),
        ServiceError::Busy => busy_body(),
        ServiceError::Inferencia => format_response(
            500,
            &[("Content-Type", "application/json")],
            br#"{"error":{"message":"inferencia fallo","type":"server_error","code":"inference_error"}}"#,
        ),
        ServiceError::Encode(_) | ServiceError::Http(_) | ServiceError::Io(_) => format_response(
            500,
            &[("Content-Type", "application/json")],
            br#"{"error":{"message":"error interno","type":"internal_error","code":"internal"}}"#,
        ),
    }
}

/// Serializa un error de servicio como respuesta HTTP completa (tests).
pub fn http_error_bytes(err: &ServiceError) -> Vec<u8> {
    error_response(err)
}

#[cfg(test)]
mod tests_gramatica {
    use super::*;

    fn tokenizer() -> Tokenizer {
        let ruta = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/self-improvement/reference/tokenizer.som");
        Tokenizer::parse(&std::fs::read(ruta).expect("tokenizer.som")).expect("tokenizer válido")
    }

    #[test]
    fn el_razonamiento_abierto_se_lee_de_la_cola_del_prompt() {
        let tok = tokenizer();
        let ids = |t: &str| tok.encode_trozo(t, false, false);
        assert!(razonamiento_abierto(&tok, &ids("<|im_start|>assistant\n<think>\n")));
        assert!(!razonamiento_abierto(&tok, &ids("<|im_start|>assistant\n<think>\nlisto\n</think>\n")));
        assert!(!razonamiento_abierto(&tok, &ids("<|im_start|>assistant\n")));
        assert!(!razonamiento_abierto(&tok, &[]));
    }
}

#[cfg(test)]
mod tests_medidor {
    use super::*;

    #[derive(Default)]
    struct Obs { tokens: Vec<u32>, checkpoints: usize }
    impl GenerationObserver for Obs {
        fn on_token(&mut self, t: u32) { self.tokens.push(t); }
        fn cancel_at(&mut self, _: GenCheckpoint) -> bool { self.checkpoints += 1; self.checkpoints == 2 }
    }

    #[test]
    fn medidor_reenvia_tokens_y_cancelacion_y_emite_deltas() {
        let mut destino = Obs::default();
        let mut rt = Runtime::new(Manifest::tiny("prueba"), TensorIndex::default(), 0, 0);
        rt.ultimo_prefijo_reutilizado = 3;
        rt.forwards_decode = 9;
        rt.borrador_ofrecidos = 7;
        rt.borrador_aceptados = 4;
        let mut m = MedidorGeneracion { destino: &mut destino, inicio: std::time::Instant::now(), inicio_generacion: std::time::Instant::now(), activo: true, prefill: None, tokens: Vec::new() };
        assert!(!m.cancel_at(GenCheckpoint::BeforeDecode));
        m.on_token(42);
        assert!(m.cancel_at(GenCheckpoint::BeforeSample));
        let mut antes = soso_llm_core::gpu::GpuStats::default();
        antes.decode_logits_down = 100;
        let mut despues = antes;
        despues.decode_logits_down += 512;
        despues.decode_embed_up += 16;
        despues.decode_pasos += 1;
        let cm = MedicionCache { lectura_s: Some(0.02), escritura_s: Some(0.03), restaurada: true };
        let j = m.json(&rt, 5, (5, 2, 1), Some((antes, despues)), Some(&cm));
        assert_eq!(j["reused_prompt_tokens"], 3);
        assert_eq!(j["prefill_tokens"], 2);
        assert_eq!(j["generated_tokens"], 1);
        assert_eq!(j["decode_forwards"], 4);
        assert_eq!(j["draft_offered"], 5);
        assert_eq!(j["draft_accepted"], 3);
        assert!(j["prefill_s"].is_number());
        assert!(j["decode_tokens_s"].is_null());
        assert_eq!(j["cuda_decode_transferencias"]["logits_dtoh_bytes"], 512);
        assert_eq!(j["cuda_decode_transferencias"]["embed_htod_bytes"], 16);
        assert_eq!(j["cuda_decode_transferencias"]["pasos"], 1);
        assert_eq!(j["cache_kv"]["restaurada"], true);
        assert_eq!(j["cache_kv"]["lectura_s"], 0.02);
        drop(m);
        assert_eq!(destino.tokens, [42]);
        assert_eq!(destino.checkpoints, 2);
    }

    #[test]
    fn cache_recupera_solo_prefijo_mejor_y_tolera_archivo_corrupto() {
        use soso_llm_core::kvsnap::{self, Capa, Snapshot};
        let dir = std::env::temp_dir().join(format!("soso-cache-servicio-{}-{:?}", std::process::id(), std::thread::current().id()));
        std::fs::create_dir(&dir).unwrap();
        let cache = CacheKvHost { directorio: dir.clone(), perfil: [7; 32], max_bytes: 4 * 1024 * 1024 };
        let manifest = Manifest::tiny("fixture");
        let dims: Vec<usize> = (0..manifest.num_layers).map(|l| manifest.effective_num_kv_heads(l) as usize * manifest.effective_head_dim(l) as usize).collect();
        let tokens = vec![3, 4, 5];
        let capas = dims.iter().map(|d| Capa { k: vec![0; tokens.len() * d], v: vec![0; tokens.len() * d] }).collect();
        let s = Snapshot { tokens: tokens.clone(), hidden: vec![0.0; manifest.hidden_dim as usize], capas };
        let bytes = kvsnap::codificar(&s, &cache.perfil, &dims, cache.max_bytes).unwrap();
        let mut primero = Runtime::new(manifest.clone(), TensorIndex::default(), 0, 0);
        primero.importar_kv(&bytes, &cache.perfil, cache.max_bytes).unwrap();
        cache.guardar(&primero).unwrap();
        let mut segundo = Runtime::new(manifest, TensorIndex::default(), 0, 0);
        assert!(!cache.recuperar(&mut segundo, &[99, 98]).unwrap());
        assert_eq!(segundo.pos, 0);
        assert!(cache.recuperar(&mut segundo, &[3, 4, 5, 6]).unwrap());
        assert_eq!(segundo.prefijo_kv_en_memoria(&[3, 4, 5, 6]), 3);
        assert!(!cache.recuperar(&mut segundo, &[3, 4, 5, 6]).unwrap());
        let mut roto = bytes;
        let n = roto.len();
        roto[n - 33] ^= 1;
        let ruta = crate::kvstore_host::listar(&dir).unwrap().pop().unwrap();
        std::fs::write(ruta, roto).unwrap();
        assert!(!cache.recuperar(&mut segundo, &[3, 4, 5, 6]).unwrap());
        assert_eq!(segundo.prefijo_kv_en_memoria(&[3, 4, 5, 6]), 3);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn cache_elige_prefijo_mas_largo_entre_conversaciones() {
        use soso_llm_core::kvsnap::{self, Capa, Snapshot};
        let dir = std::env::temp_dir().join(format!("soso-cache-multiple-{}-{:?}", std::process::id(), std::thread::current().id()));
        std::fs::create_dir(&dir).unwrap();
        let cache = CacheKvHost { directorio: dir.clone(), perfil: [7; 32], max_bytes: 4 * 1024 * 1024 };
        let manifest = Manifest::tiny("fixture");
        let dims: Vec<usize> = (0..manifest.num_layers).map(|l| manifest.effective_num_kv_heads(l) as usize * manifest.effective_head_dim(l) as usize).collect();
        for tokens in [vec![3, 4], vec![9, 8, 7], vec![3, 4, 5, 6]] {
            let capas = dims.iter().map(|d| Capa { k: vec![0; tokens.len() * d], v: vec![0; tokens.len() * d] }).collect();
            let s = Snapshot { tokens, hidden: vec![0.0; manifest.hidden_dim as usize], capas };
            let bytes = kvsnap::codificar(&s, &cache.perfil, &dims, cache.max_bytes).unwrap();
            let mut rt = Runtime::new(manifest.clone(), TensorIndex::default(), 0, 0);
            rt.importar_kv(&bytes, &cache.perfil, cache.max_bytes).unwrap();
            cache.guardar(&rt).unwrap();
        }
        assert_eq!(crate::kvstore_host::listar(&dir).unwrap().len(), 3);
        let mut rt = Runtime::new(manifest, TensorIndex::default(), 0, 0);
        assert!(cache.recuperar(&mut rt, &[3, 4, 5, 6, 7]).unwrap());
        assert_eq!(rt.prefijo_kv_en_memoria(&[3, 4, 5, 6, 7]), 4);
        assert!(cache.recuperar(&mut rt, &[9, 8, 7, 1]).unwrap());
        assert_eq!(rt.prefijo_kv_en_memoria(&[9, 8, 7, 1]), 3);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
