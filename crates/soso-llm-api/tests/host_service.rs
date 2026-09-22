//! T13 — servidor host con backend falso (loopback).

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::Duration;

use soso_llm_api::{run_accept_loop, ChatBackend, HostService, PreparedChatCompletion};
use soso_llm_core::conversation::ModelProfile;
use soso_llm_core::generation::{
    GenCheckpoint, GenerationObserver, GenerationReport, StopReason,
};
use soso_llm_core::tokenizer::Tokenizer;

const TOKEN: &str = "test-token";

fn static_tokenizer() -> &'static Tokenizer {
    static TOK: OnceLock<Tokenizer> = OnceLock::new();
    TOK.get_or_init(|| {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/qwen2.5-coder-3b-model/tokenizer.som");
        if let Ok(data) = std::fs::read(&path) {
            Tokenizer::parse(&data).expect("tokenizer.som del perfil T03")
        } else {
            panic!(
                "falta {path:?}: los tests host_service necesitan el tokenizer convertido (T03)"
            );
        }
    })
}

fn test_profile() -> ModelProfile {
    ModelProfile {
        id: String::from("soso-coder"),
        directory: String::from("fake"),
        family: String::from("qwen2"),
        weights_sha256: String::from("a".repeat(64)),
        tokenizer_sha256: String::from("b".repeat(64)),
        template_sha256: String::from("c".repeat(64)),
        context_tokens: 4096,
        max_output_tokens: 128,
        stop_token_ids: vec![151645],
    }
}

struct FakeState {
    resets: AtomicUsize,
    generations: AtomicUsize,
    delay: Duration,
    fail_infer: AtomicBool,
    reply: Mutex<String>,
}

impl FakeState {
    fn new(reply: &str) -> Self {
        Self {
            resets: AtomicUsize::new(0),
            generations: AtomicUsize::new(0),
            delay: Duration::from_millis(0),
            fail_infer: AtomicBool::new(false),
            reply: Mutex::new(String::from(reply)),
        }
    }

    fn begin_request(&self) {
        self.resets.fetch_add(1, Ordering::Relaxed);
    }

    fn generate_observed(
        &self,
        prepared: &PreparedChatCompletion,
        observer: &mut dyn GenerationObserver,
    ) -> Result<(Vec<u32>, GenerationReport), ()> {
        self.generations.fetch_add(1, Ordering::Relaxed);
        if self.fail_infer.load(Ordering::Relaxed) {
            return Err(());
        }
        if self.delay > Duration::ZERO {
            thread::sleep(self.delay);
        }
        if observer.cancel_at(GenCheckpoint::BeforePrefill) {
            return Ok((
                Vec::new(),
                GenerationReport {
                    generated: 0,
                    prompt_tokens: prepared.prompt_tokens,
                    sampled_tokens: 0,
                    stop: StopReason::Cancelled,
                },
            ));
        }
        let text = self.reply.lock().unwrap().clone();
        let ids = static_tokenizer().encode(&text);
        for &t in &ids {
            observer.on_token(t);
        }
        Ok((
            ids.clone(),
            GenerationReport {
                generated: ids.len() as u32,
                prompt_tokens: prepared.prompt_tokens,
                sampled_tokens: ids.len() as u32,
                stop: StopReason::Limit,
            },
        ))
    }
}

struct FakeBackend {
    profile: ModelProfile,
    state: Arc<FakeState>,
}

impl FakeBackend {
    fn new(reply: &str) -> Self {
        Self {
            profile: test_profile(),
            state: Arc::new(FakeState::new(reply)),
        }
    }

    fn with_delay(mut self, delay: Duration) -> Self {
        Arc::get_mut(&mut self.state).unwrap().delay = delay;
        self
    }

    fn state(&self) -> &Arc<FakeState> {
        &self.state
    }
}

impl ChatBackend for FakeBackend {
    fn backend_name(&self) -> &'static str {
        "fake-test"
    }

    fn profile(&self) -> &ModelProfile {
        &self.profile
    }

    fn tokenizer(&self) -> &Tokenizer {
        static_tokenizer()
    }

    fn begin_request(&mut self) {
        self.state.begin_request();
    }

    fn generate_observed(
        &mut self,
        _prompt_ids: &[u32],
        prepared: &PreparedChatCompletion,
        observer: &mut dyn GenerationObserver,
    ) -> Result<(Vec<u32>, GenerationReport), ()> {
        self.state.generate_observed(prepared, observer)
    }
}

struct TestServer {
    addr: std::net::SocketAddr,
    backend: FakeBackend,
    _thread: thread::JoinHandle<()>,
}

impl TestServer {
    fn spawn(backend: FakeBackend) -> Self {
        let addr_backend = backend.state().clone();
        let profile = backend.profile.clone();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let svc = Arc::new(HostService::new(
            String::from(TOKEN),
            FakeBackend {
                profile,
                state: addr_backend,
            },
        ));
        let handle = thread::spawn(move || {
            let _ = run_accept_loop(svc, listener);
        });
        TestServer {
            addr,
            backend,
            _thread: handle,
        }
    }
}

fn http_roundtrip(addr: std::net::SocketAddr, raw: &[u8]) -> Vec<u8> {
    let mut stream = TcpStream::connect(addr).unwrap();
    stream.write_all(raw).unwrap();
    stream.shutdown(std::net::Shutdown::Write).unwrap();
    let mut out = Vec::new();
    stream.read_to_end(&mut out).unwrap();
    out
}

fn auth_header() -> String {
    format!("Authorization: Bearer {TOKEN}\r\n")
}

fn chat_body(user: &str) -> String {
    format!(
        r#"{{"model":"soso-coder","messages":[{{"role":"user","content":"{user}"}}],"max_tokens":16}}"#
    )
}

fn post_chat(addr: std::net::SocketAddr, user: &str) -> Vec<u8> {
    let body = chat_body(user);
    let req = format!(
        "POST /v1/chat/completions HTTP/1.1\r\n{}Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
        auth_header(),
        body.len(),
        body
    );
    http_roundtrip(addr, req.as_bytes())
}

#[test]
fn health_models_y_auth() {
    let server = TestServer::spawn(FakeBackend::new("hola"));
    let health = http_roundtrip(
        server.addr,
        format!(
            "GET /health HTTP/1.1\r\n{}\r\n",
            auth_header()
        )
        .as_bytes(),
    );
    assert!(health.starts_with(b"HTTP/1.1 200"));
    let body = std::str::from_utf8(&health).unwrap();
    assert!(body.contains("host-cpu") || body.contains("fake-test"));
    assert!(body.contains("soso-coder"));

    let models = http_roundtrip(
        server.addr,
        format!("GET /v1/models HTTP/1.1\r\n{}\r\n", auth_header()).as_bytes(),
    );
    assert!(models.starts_with(b"HTTP/1.1 200"));
    assert!(std::str::from_utf8(&models).unwrap().contains("soso-coder"));

    let no_auth = http_roundtrip(
        server.addr,
        b"GET /health HTTP/1.1\r\n\r\n",
    );
    assert!(no_auth.starts_with(b"HTTP/1.1 401"));
}

#[test]
fn chat_json_y_modelo_desconocido() {
    let server = TestServer::spawn(FakeBackend::new("respuesta"));
    let resp = post_chat(server.addr, "uno");
    assert!(resp.starts_with(b"HTTP/1.1 200"));
    assert!(std::str::from_utf8(&resp).unwrap().contains("respuesta"));

    let bad = format!(
        r#"{{"model":"otro","messages":[{{"role":"user","content":"x"}}]}}"#
    );
    let req = format!(
        "POST /v1/chat/completions HTTP/1.1\r\n{}Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
        auth_header(),
        bad.len(),
        bad
    );
    let resp = http_roundtrip(server.addr, req.as_bytes());
    assert!(resp.starts_with(b"HTTP/1.1 404"));
}

#[test]
fn dos_chats_reinician_contexto() {
    let server = TestServer::spawn(FakeBackend::new("alpha"));
    let _ = post_chat(server.addr, "primero");
    let _ = post_chat(server.addr, "segundo");
    assert!(server.backend.state().resets.load(Ordering::Relaxed) >= 2);
    assert_eq!(server.backend.state().generations.load(Ordering::Relaxed), 2);
}

#[test]
fn ocupado_devuelve_429_pero_health_vive() {
    let server = TestServer::spawn(FakeBackend::new("lento").with_delay(Duration::from_millis(400)));
    let addr = server.addr;
    let t1 = thread::spawn(move || post_chat(addr, "bloqueo"));
    thread::sleep(Duration::from_millis(50));
    let busy = post_chat(server.addr, "segundo");
    assert!(busy.starts_with(b"HTTP/1.1 429"));
    let health = http_roundtrip(
        server.addr,
        format!("GET /health HTTP/1.1\r\n{}\r\n", auth_header()).as_bytes(),
    );
    assert!(health.starts_with(b"HTTP/1.1 200"));
    let _ = t1.join();
}

#[test]
fn inferencia_fallida_libera_busy() {
    let mut backend = FakeBackend::new("x");
    Arc::get_mut(&mut backend.state).unwrap()
        .fail_infer
        .store(true, Ordering::Relaxed);
    let server = TestServer::spawn(backend);
    let resp = post_chat(server.addr, "falla");
    assert!(resp.starts_with(b"HTTP/1.1 500"));
    let resp2 = post_chat(server.addr, "otra");
    assert!(
        resp2.starts_with(b"HTTP/1.1 500") || resp2.starts_with(b"HTTP/1.1 200"),
        "busy no debe quedar pegado: {:?}",
        &resp2[..20.min(resp2.len())]
    );
}
