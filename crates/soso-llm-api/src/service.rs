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
use soso_llm_core::conversation::{
    parse_assistant_output, render_messages, ChatError, ModelProfile, ToolChoice,
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

/// `FileMapper` host con liberación real (no leak como `hostrun`).
pub struct OwnedMapper {
    slabs: HashMap<u64, Box<[u8]>>,
}

impl Default for OwnedMapper {
    fn default() -> Self {
        Self {
            slabs: HashMap::new(),
        }
    }
}

impl FileMapper for OwnedMapper {
    fn map_file(&mut self, path: &str) -> Result<MappedShard, ()> {
        let data = std::fs::read(path).map_err(|_| ())?;
        let len = data.len();
        let boxed = data.into_boxed_slice();
        let addr = boxed.as_ptr() as u64;
        self.slabs.insert(addr, boxed);
        Ok(MappedShard { addr, len })
    }

    fn unmap_file(&mut self, shard: &MappedShard) {
        self.slabs.remove(&shard.addr);
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
    source: RunSource,
    profile: ModelProfile,
    tokenizer: Tokenizer,
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

    let rt = Runtime::new(manifest, index.clone(), 0, 0);
    rt.validate_shapes()
        .map_err(|_| LoadError::Modelo(String::from("shapes inválidas")))?;

    let base = MmapTensorSource::new(format!("{model_dir}/shards"), index, OwnedMapper::default());
    let source = if std::env::var("SOSO_SYNC_STAGING").is_ok() {
        RunSource::Plain(base.with_sync_prefetch())
    } else {
        RunSource::Staged(ThreadStagedSource::new(base))
    };

    Ok(CpuBackend {
        rt,
        source,
        profile,
        tokenizer,
    })
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
        self.rt.reset_sequence();
    }

    fn generate_observed(
        &mut self,
        prompt_ids: &[u32],
        prepared: &PreparedChatCompletion,
        observer: &mut dyn GenerationObserver,
    ) -> Result<(Vec<u32>, GenerationReport), ()> {
        let stops = self.profile.stop_token_ids.clone();
        let options = GenerationOptions::new(prepared.max_new_tokens as usize, stops);
        let seed = prepared.seed.unwrap_or(42) as u64;
        let mut sampler = Sampler::new(
            prepared.temperature as f32,
            prepared.top_p as f32,
            seed,
        );
        self.rt.generate_stream_par_observed(
            &mut self.source,
            prompt_ids,
            options,
            &mut sampler,
            observer,
            None,
            &mut None,
        )
    }
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
            _ => Err(ServiceError::NotFound),
        }
    }

    fn post_chat(
        &self,
        req: HttpRequest,
        stream: &mut TcpStream,
    ) -> Result<Vec<u8>, ServiceError> {
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

        let body = std::str::from_utf8(&req.body)
            .map_err(|_| ServiceError::Api(ApiError::PeticionInvalida {
                motivo: String::from("cuerpo no UTF-8"),
            }))?;

        let (prepared, prompt_ids, profile) = {
            let b = self.backend.lock().unwrap();
            let profile = b.profile().clone();
            let prepared = prepare_chat_completion(body, &profile, b.tokenizer())
                .map_err(ServiceError::Api)?;
            let prompt_ids = render_messages(&prepared.input, &profile, b.tokenizer())
                .map_err(|e| ServiceError::Api(e.into()))?;
            (prepared, prompt_ids, profile)
        };

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

        let generated_text = {
            let b = self.backend.lock().unwrap();
            b.tokenizer().decode(&token_ids)
        };
        let turn = parse_assistant_output(&prepared.input, &generated_text, 1)
            .map_err(|e| ServiceError::Api(e.into()))?;

        validate_tool_choice(&prepared, &turn)?;

        let payload = completion_payload(&prepared, &profile, turn, report);
        let json = encode_chat_completion_json(&payload).map_err(ServiceError::Encode)?;
        Ok(format_response(
            200,
            &[("Content-Type", "application/json")],
            &json,
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

        let generated_text = {
            let b = self.backend.lock().unwrap();
            b.tokenizer().decode(&token_ids)
        };
        let turn = parse_assistant_output(&prepared.input, &generated_text, 1)
            .map_err(|e| ServiceError::Api(e.into()))?;
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
