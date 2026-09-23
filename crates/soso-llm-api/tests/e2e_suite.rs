//! T19 — invariantes sobre backend sintético (no cuenta como capacidad de agente).

use std::net::TcpListener;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use soso_llm_api::{
    check_busy_while_generating, check_health_during_generation, run_core_invariants,
    ApiClient, ChatBackend, HostService, PreparedChatCompletion,
};
use soso_llm_core::conversation::ModelProfile;
use soso_llm_core::generation::{
    GenCheckpoint, GenerationObserver, GenerationReport, StopReason,
};
use soso_llm_core::tokenizer::Tokenizer;

const TOKEN: &str = "e2e-token";

fn tok() -> Tokenizer {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/qwen2.5-coder-3b-model/tokenizer.som");
    if let Ok(data) = std::fs::read(&path) {
        Tokenizer::parse(&data).expect("tokenizer T03")
    } else {
        Tokenizer::byte_level()
    }
}

fn profile() -> ModelProfile {
    ModelProfile {
        id: String::from("soso-coder"),
        directory: String::from("fake"),
        family: String::from("qwen2"),
        weights_sha256: String::from("a".repeat(64)),
        tokenizer_sha256: String::from("b".repeat(64)),
        template_sha256: String::from("c".repeat(64)),
        context_tokens: 4096,
        max_output_tokens: 64,
        stop_token_ids: vec![151645],
    }
}

struct SlowBackend {
    inner: ModelProfile,
    tokenizer: Tokenizer,
    delay_ms: u64,
}

impl ChatBackend for SlowBackend {
    fn backend_name(&self) -> &'static str {
        "synthetic-slow"
    }
    fn profile(&self) -> &ModelProfile {
        &self.inner
    }
    fn tokenizer(&self) -> &Tokenizer {
        &self.tokenizer
    }
    fn begin_request(&mut self) {}
    fn generate_observed(
        &mut self,
        _prompt: &[u32],
        prepared: &PreparedChatCompletion,
        observer: &mut dyn GenerationObserver,
    ) -> Result<(Vec<u32>, GenerationReport), ()> {
        for _ in 0..20 {
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
            thread::sleep(Duration::from_millis(self.delay_ms));
        }
        let text = "ok";
        let ids = self.tokenizer.encode(text);
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

fn spawn_slow() -> (u16, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let svc = Arc::new(HostService::new(
        String::from(TOKEN),
        SlowBackend {
            inner: profile(),
            tokenizer: tok(),
            delay_ms: 50,
        },
    ));
    let h = thread::spawn(move || {
        for c in listener.incoming().flatten() {
            let s = Arc::clone(&svc);
            thread::spawn(move || s.handle_connection(c));
        }
    });
    (port, h)
}

fn spawn_fast() -> (u16, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let svc = Arc::new(HostService::new(
        String::from(TOKEN),
        SlowBackend {
            inner: profile(),
            tokenizer: tok(),
            delay_ms: 0,
        },
    ));
    let h = thread::spawn(move || {
        for c in listener.incoming().flatten() {
            let s = Arc::clone(&svc);
            thread::spawn(move || s.handle_connection(c));
        }
    });
    (port, h)
}

#[test]
fn core_invariants_synthetic() {
    let (port, _h) = spawn_fast();
    let client = ApiClient::new(port, TOKEN, "soso-coder");
    let steps = run_core_invariants(&client);
    for s in &steps {
        assert!(s.ok, "{}: {}", s.name, s.detail);
    }
}

#[test]
fn busy_y_health_synthetic() {
    let (port, _h) = spawn_slow();
    let client = ApiClient::new(port, TOKEN, "soso-coder");
    let b = check_busy_while_generating(&client, 64);
    assert!(b.ok, "{}", b.detail);
    let (port2, _h2) = spawn_slow();
    let client2 = ApiClient::new(port2, TOKEN, "soso-coder");
    let h = check_health_during_generation(&client2, 64);
    assert!(h.ok, "{}", h.detail);
}
