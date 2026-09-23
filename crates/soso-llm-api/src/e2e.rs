//! Invariantes API reutilizables (T19): transporte HTTP sobre TCP std.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use soso_llm_core::conversation::ModelProfile;

/// Cliente loopback hacia el puerto reenviado o un servidor de prueba.
#[derive(Debug, Clone)]
pub struct ApiClient {
    pub host_port: u16,
    pub token: String,
    pub model_id: String,
}

impl ApiClient {
    pub fn new(host_port: u16, token: impl Into<String>, model_id: impl Into<String>) -> Self {
        Self {
            host_port,
            token: token.into(),
            model_id: model_id.into(),
        }
    }

    fn addr(&self) -> String {
        format!("127.0.0.1:{}", self.host_port)
    }

    pub fn roundtrip(&self, raw: &[u8]) -> Result<Vec<u8>, String> {
        self.roundtrip_timeouts(raw, Duration::from_secs(10), Duration::from_secs(600))
    }

    pub fn roundtrip_timeouts(
        &self,
        raw: &[u8],
        connect: Duration,
        read: Duration,
    ) -> Result<Vec<u8>, String> {
        let addr = self
            .addr()
            .parse()
            .map_err(|e| format!("addr: {e}"))?;
        let mut stream = TcpStream::connect_timeout(&addr, connect)
            .map_err(|e| format!("connect {}: {e}", self.addr()))?;
        stream.set_read_timeout(Some(read)).ok();
        stream
            .write_all(raw)
            .map_err(|e| format!("write: {e}"))?;
        stream.shutdown(std::net::Shutdown::Write).ok();
        let mut out = Vec::new();
        stream
            .read_to_end(&mut out)
            .map_err(|e| format!("read: {e}"))?;
        Ok(out)
    }

    pub fn auth_header(&self) -> String {
        format!("Authorization: Bearer {}\r\n", self.token)
    }

    pub fn get(&self, path: &str, with_auth: bool) -> Result<Vec<u8>, String> {
        self.get_timeouts(path, with_auth, Duration::from_secs(10), Duration::from_secs(600))
    }

    pub fn get_timeouts(
        &self,
        path: &str,
        with_auth: bool,
        connect: Duration,
        read: Duration,
    ) -> Result<Vec<u8>, String> {
        let req = if with_auth {
            format!("GET {path} HTTP/1.1\r\nHost: localhost\r\n{}\r\n", self.auth_header())
        } else {
            format!("GET {path} HTTP/1.1\r\nHost: localhost\r\n\r\n")
        };
        self.roundtrip_timeouts(req.as_bytes(), connect, read)
    }

    pub fn post_json(&self, path: &str, body: &str, with_auth: bool) -> Result<Vec<u8>, String> {
        self.post_json_timeouts(path, body, with_auth, Duration::from_secs(10), Duration::from_secs(600))
    }

    pub fn post_json_timeouts(
        &self,
        path: &str,
        body: &str,
        with_auth: bool,
        connect: Duration,
        read: Duration,
    ) -> Result<Vec<u8>, String> {
        let req = if with_auth {
            format!(
                "POST {path} HTTP/1.1\r\nHost: localhost\r\n{}Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
                self.auth_header(),
                body.len()
            )
        } else {
            format!(
                "POST {path} HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            )
        };
        self.roundtrip_timeouts(req.as_bytes(), connect, read)
    }

    pub fn chat_user(&self, content: &str, max_tokens: u32, stream: bool) -> Result<Vec<u8>, String> {
        self.chat_user_timeouts(
            content,
            max_tokens,
            stream,
            Duration::from_secs(10),
            Duration::from_secs(600),
        )
    }

    pub fn chat_user_timeouts(
        &self,
        content: &str,
        max_tokens: u32,
        stream: bool,
        connect: Duration,
        read: Duration,
    ) -> Result<Vec<u8>, String> {
        let esc = content.replace('\\', "\\\\").replace('"', "\\\"");
        let stream_key = if stream { "true" } else { "false" };
        let body = format!(
            r#"{{"model":"{}","messages":[{{"role":"user","content":"{esc}"}}],"max_tokens":{max_tokens},"stream":{stream_key}}}"#,
            self.model_id
        );
        self.post_json_timeouts("/v1/chat/completions", &body, true, connect, read)
    }
}

/// Límites HTTP para el camino guest QEMU (generaciones cortas).
#[derive(Debug, Clone, Copy)]
pub struct GuestHttpLimits {
    pub connect: Duration,
    pub read: Duration,
    pub chat_max_tokens: u32,
    pub overlap_max_tokens: u32,
}

impl Default for GuestHttpLimits {
    fn default() -> Self {
        Self {
            connect: Duration::from_secs(10),
            // Medido el 2026-09-23 en el guest QEMU (1 vCPU, 2 GiB) con
            // Qwen2.5-Coder-3B: una petición de **dos** tokens tarda unos
            // 200 s. No es sorprendente —el presupuesto de pesos es de
            // 1040 MiB para un modelo de 2212 MiB, así que cada token pagina
            // desde disco—, pero con los 60 s de antes el cliente se rendía
            // antes de que el servidor pudiera contestar y el informe decía
            // «sin respuesta» donde lo que había era lentitud. Fijar el
            // presupuesto de verdad es cosa de T14; esto sólo evita declarar
            // muerto un servidor que está trabajando.
            read: Duration::from_secs(600),
            chat_max_tokens: 2,
            overlap_max_tokens: 4,
        }
    }
}

#[derive(Debug, Clone)]
pub struct StepResult {
    pub name: &'static str,
    pub ok: bool,
    pub detail: String,
}

impl StepResult {
    fn ok(name: &'static str) -> Self {
        Self {
            name,
            ok: true,
            detail: String::new(),
        }
    }
    fn fail(name: &'static str, detail: impl Into<String>) -> Self {
        Self {
            name,
            ok: false,
            detail: detail.into(),
        }
    }
}

fn status_line(resp: &[u8]) -> Option<&str> {
    std::str::from_utf8(resp).ok()?.lines().next()
}

pub fn run_core_invariants(client: &ApiClient) -> Vec<StepResult> {
    run_core_invariants_with(client, None)
}

/// Como `run_core_invariants`, con tokens y timeouts reducidos para guest.
pub fn run_core_invariants_guest(client: &ApiClient, lim: GuestHttpLimits) -> Vec<StepResult> {
    run_core_invariants_with(client, Some(lim))
}

fn run_core_invariants_with(client: &ApiClient, lim: Option<GuestHttpLimits>) -> Vec<StepResult> {
    vec![
        check_health(client, lim),
        check_models(client, lim),
        check_auth_invalid(client, lim),
        check_chat_json(client, lim),
        check_unknown_model(client, lim),
        check_unicode(client, lim),
        check_large_body(client, lim),
        check_sse(client, lim),
    ]
}

fn http_timeouts(lim: Option<GuestHttpLimits>) -> (Duration, Duration) {
    match lim {
        Some(g) => (g.connect, g.read),
        None => (Duration::from_secs(10), Duration::from_secs(600)),
    }
}

fn chat_max_tokens(lim: Option<GuestHttpLimits>, default_host: u32) -> u32 {
    lim.map(|g| g.chat_max_tokens).unwrap_or(default_host)
}

fn check_health(client: &ApiClient, lim: Option<GuestHttpLimits>) -> StepResult {
    let (connect, read) = http_timeouts(lim);
    match client.get_timeouts("/health", true, connect, read) {
        Ok(r) if status_line(&r).is_some_and(|l| l.contains("200")) => {
            let body = String::from_utf8_lossy(&r);
            if body.contains("ready") || body.contains("status") {
                StepResult::ok("health")
            } else {
                StepResult::fail("health", "200 sin status")
            }
        }
        Ok(r) => StepResult::fail("health", format!("{:?}", status_line(&r))),
        Err(e) => StepResult::fail("health", e),
    }
}

fn check_models(client: &ApiClient, lim: Option<GuestHttpLimits>) -> StepResult {
    let (connect, read) = http_timeouts(lim);
    match client.get_timeouts("/v1/models", true, connect, read) {
        Ok(r) if status_line(&r).is_some_and(|l| l.contains("200")) => {
            let body = String::from_utf8_lossy(&r);
            if body.contains(&client.model_id) {
                StepResult::ok("models")
            } else {
                StepResult::fail("models", "id ausente")
            }
        }
        Ok(r) => StepResult::fail("models", format!("{:?}", status_line(&r))),
        Err(e) => StepResult::fail("models", e),
    }
}

fn check_auth_invalid(client: &ApiClient, lim: Option<GuestHttpLimits>) -> StepResult {
    let (connect, read) = http_timeouts(lim);
    match client.get_timeouts("/health", false, connect, read) {
        Ok(r) if status_line(&r).is_some_and(|l| l.contains("401")) => StepResult::ok("auth_401"),
        Ok(r) => StepResult::fail("auth_401", format!("{:?}", status_line(&r))),
        Err(e) => StepResult::fail("auth_401", e),
    }
}

fn check_chat_json(client: &ApiClient, lim: Option<GuestHttpLimits>) -> StepResult {
    let (connect, read) = http_timeouts(lim);
    let chat = chat_max_tokens(lim, 8);
    match client.chat_user_timeouts("di hola", chat, false, connect, read) {
        Ok(r) if status_line(&r).is_some_and(|l| l.contains("200")) => StepResult::ok("chat_json"),
        Ok(r) => StepResult::fail("chat_json", format!("{:?}", status_line(&r))),
        Err(e) => StepResult::fail("chat_json", e),
    }
}

fn check_unknown_model(client: &ApiClient, lim: Option<GuestHttpLimits>) -> StepResult {
    let (connect, read) = http_timeouts(lim);
    let body = r#"{"model":"no-existe-xyz","messages":[{"role":"user","content":"x"}]}"#;
    match client.post_json_timeouts("/v1/chat/completions", body, true, connect, read) {
        Ok(r) if status_line(&r).is_some_and(|l| l.contains("404")) => {
            StepResult::ok("modelo_desconocido")
        }
        Ok(r) => StepResult::fail("modelo_desconocido", format!("{:?}", status_line(&r))),
        Err(e) => StepResult::fail("modelo_desconocido", e),
    }
}

fn check_unicode(client: &ApiClient, lim: Option<GuestHttpLimits>) -> StepResult {
    let (connect, read) = http_timeouts(lim);
    let chat = chat_max_tokens(lim, 8);
    match client.chat_user_timeouts("café ☕", chat, false, connect, read) {
        Ok(r) if status_line(&r).is_some_and(|l| l.contains("200")) => StepResult::ok("unicode"),
        Ok(r) => StepResult::fail("unicode", format!("{:?}", status_line(&r))),
        Err(e) => StepResult::fail("unicode", e),
    }
}

fn check_large_body(client: &ApiClient, lim: Option<GuestHttpLimits>) -> StepResult {
    let (connect, read) = http_timeouts(lim);
    let chat = chat_max_tokens(lim, 4);
    let pad = "x".repeat(1200);
    match client.chat_user_timeouts(&pad, chat, false, connect, read) {
        Ok(r) if status_line(&r).is_some_and(|l| l.contains("200")) => StepResult::ok("cuerpo_grande"),
        Ok(r) => StepResult::fail("cuerpo_grande", format!("{:?}", status_line(&r))),
        Err(e) => StepResult::fail("cuerpo_grande", e),
    }
}

fn check_sse(client: &ApiClient, lim: Option<GuestHttpLimits>) -> StepResult {
    let (connect, read) = http_timeouts(lim);
    let chat = chat_max_tokens(lim, 4);
    match client.chat_user_timeouts("uno", chat, true, connect, read) {
        Ok(r) if status_line(&r).is_some_and(|l| l.contains("200")) => {
            let body = String::from_utf8_lossy(&r);
            if body.contains("text/event-stream") || body.contains("data:") {
                StepResult::ok("sse")
            } else {
                StepResult::fail("sse", "sin event-stream")
            }
        }
        Ok(r) => StepResult::fail("sse", format!("{:?}", status_line(&r))),
        Err(e) => StepResult::fail("sse", e),
    }
}

/// Segunda petición chat mientras la primera aún no termina → 429.
pub fn check_busy_while_generating(
    client: &ApiClient,
    long_max_tokens: u32,
) -> StepResult {
    check_busy_while_generating_with(client, long_max_tokens, None)
}

pub fn check_busy_while_generating_guest(
    client: &ApiClient,
    lim: GuestHttpLimits,
) -> StepResult {
    check_busy_while_generating_with(client, lim.overlap_max_tokens, Some(lim))
}

fn check_busy_while_generating_with(
    client: &ApiClient,
    long_max_tokens: u32,
    lim: Option<GuestHttpLimits>,
) -> StepResult {
    let (connect, read) = http_timeouts(lim);
    let addr = client.addr();
    let body = format!(
        r#"{{"model":"{}","messages":[{{"role":"user","content":"cuenta lento"}}],"max_tokens":{long_max_tokens}}}"#,
        client.model_id
    );
    let req = format!(
        "POST /v1/chat/completions HTTP/1.1\r\nHost: localhost\r\n{}Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        client.auth_header(),
        body.len()
    );
    let slow_addr = addr.parse().ok();
    let mut slow = match slow_addr.and_then(|a| TcpStream::connect_timeout(&a, connect).ok()) {
        Some(s) => s,
        None => return StepResult::fail("busy_429", "connect lenta"),
    };
    if slow.write_all(req.as_bytes()).is_err() {
        return StepResult::fail("busy_429", "write lenta");
    }
    std::thread::sleep(Duration::from_millis(80));
    match client.chat_user_timeouts("otra", 2, false, connect, read) {
        Ok(r) if status_line(&r).is_some_and(|l| l.contains("429")) => StepResult::ok("busy_429"),
        Ok(r) => StepResult::fail("busy_429", format!("{:?}", status_line(&r))),
        Err(e) => StepResult::fail("busy_429", e),
    }
}

/// Health accesible mientras hay generación (conexión auxiliar / poll T17).
pub fn check_health_during_generation(client: &ApiClient, long_max_tokens: u32) -> StepResult {
    check_health_during_generation_with(client, long_max_tokens, None)
}

pub fn check_health_during_generation_guest(
    client: &ApiClient,
    lim: GuestHttpLimits,
) -> StepResult {
    check_health_during_generation_with(client, lim.overlap_max_tokens, Some(lim))
}

fn check_health_during_generation_with(
    client: &ApiClient,
    long_max_tokens: u32,
    lim: Option<GuestHttpLimits>,
) -> StepResult {
    let (connect, read) = http_timeouts(lim);
    let addr = client.addr();
    let body = format!(
        r#"{{"model":"{}","messages":[{{"role":"user","content":"largo"}}],"max_tokens":{long_max_tokens}}}"#,
        client.model_id
    );
    let req = format!(
        "POST /v1/chat/completions HTTP/1.1\r\nHost: localhost\r\n{}Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        client.auth_header(),
        body.len()
    );
    let slow_addr = addr.parse().ok();
    let mut slow = match slow_addr.and_then(|a| TcpStream::connect_timeout(&a, connect).ok()) {
        Some(s) => s,
        None => return StepResult::fail("health_en_generacion", "connect lenta"),
    };
    let _ = slow.write_all(req.as_bytes());
    std::thread::sleep(Duration::from_millis(500));
    match client.get_timeouts("/health", true, connect, read) {
        Ok(r) if status_line(&r).is_some_and(|l| l.contains("200")) => {
            StepResult::ok("health_en_generacion")
        }
        Ok(r) => StepResult::fail("health_en_generacion", format!("{:?}", status_line(&r))),
        Err(e) => StepResult::fail("health_en_generacion", e),
    }
}

pub fn check_server_down(host_port: u16) -> StepResult {
    match TcpStream::connect(format!("127.0.0.1:{host_port}")) {
        Ok(_) => StepResult::fail("servidor_apagado", "sigue aceptando TCP"),
        Err(_) => StepResult::ok("servidor_apagado"),
    }
}

/// Perfil mínimo leído de model-lock / T03.
#[derive(Debug, serde::Deserialize)]
pub struct ProfileFile {
    pub schema_version: u32,
    pub som: ProfileSom,
}

#[derive(Debug, serde::Deserialize)]
pub struct ProfileSom {
    pub nombre: String,
    pub directorio: Option<String>,
}

impl ProfileFile {
    pub fn catalog_name(&self) -> &str {
        &self.som.nombre
    }
}

pub fn model_profile_from_lock(p: &ProfileFile) -> ModelProfile {
    ModelProfile {
        id: p.som.nombre.clone(),
        directory: p
            .som
            .directorio
            .clone()
            .unwrap_or_else(|| format!("/models/{}", p.som.nombre)),
        family: String::from("qwen2"),
        weights_sha256: String::new(),
        tokenizer_sha256: String::new(),
        template_sha256: String::new(),
        context_tokens: 4096,
        max_output_tokens: 128,
        stop_token_ids: vec![],
    }
}
