//! T21: servidor real (`HostService`, mismo SSE y mismo parseo de herramientas
//! que en producción) con un modelo **guionizado** en vez de pesos.
//!
//! El «modelo» emite, según cuántos resultados de herramienta lleva el
//! historial, una llamada `read`, otra `edit`, otra `bash` y un texto final, en
//! el formato `<tool_call>` de Qwen. Sirve para ver que OpenCode acepta lo que
//! de verdad contesta soso, sin esperar a un 7B.
//!
//! ```sh
//! export SOSO_LLM_API_KEY=clave-de-prueba
//! cargo run -p soso-llm-api --features std --example serve_guion -- \
//!   --repo /ruta/al/repo --port 7433 [--retraso-ms 2000]
//! ```

use std::net::{SocketAddr, TcpListener};
use std::sync::Arc;

use soso_llm_api::{run_accept_loop, ChatBackend, HostService, PreparedChatCompletion};
use soso_llm_core::conversation::{ModelProfile, Role};
use soso_llm_core::generation::{GenerationObserver, GenerationReport, StopReason};
use soso_llm_core::tokenizer::Tokenizer;

const IM_END: u32 = 151_645;

struct Guion {
    profile: ModelProfile,
    tokenizer: Tokenizer,
    repo: String,
    /// Retraso por generación (`--retraso-ms`): simula un modelo lento para
    /// probar la admisión (429 mientras hay una generación en curso).
    retraso_ms: u64,
}

impl Guion {
    fn texto(&self, resultados: usize, hay_herramientas: bool) -> String {
        if !hay_herramientas {
            return String::from("Prueba de contrato");
        }
        let f = format!("{}/hola.txt", self.repo);
        match resultados {
            0 => format!(
                "<tool_call>\n{{\"name\": \"read\", \"arguments\": {{\"filePath\": \"{f}\"}}}}\n</tool_call>"
            ),
            // `GUION_LINEAS=1`: la herramienta propia `reemplazar_lineas` (T22) en vez de `edit`.
            1 if std::env::var_os("GUION_LINEAS").is_some() => format!(
                "<tool_call>\n{{\"name\": \"reemplazar_lineas\", \"arguments\": {{\"filePath\": \"{f}\", \"desde\": 1, \"hasta\": 1, \"texto\": \"hola soso\"}}}}\n</tool_call>"
            ),
            1 => format!(
                "<tool_call>\n{{\"name\": \"edit\", \"arguments\": {{\"filePath\": \"{f}\", \"oldString\": \"mundo\", \"newString\": \"soso\"}}}}\n</tool_call>"
            ),
            2 => String::from(
                "<tool_call>\n{\"name\": \"bash\", \"arguments\": {\"command\": \"git status --short\", \"description\": \"estado\"}}\n</tool_call>",
            ),
            _ => String::from("Hecho: leí, edité y comprobé."),
        }
    }
}

impl ChatBackend for Guion {
    fn backend_name(&self) -> &'static str {
        "guion"
    }
    fn profile(&self) -> &ModelProfile {
        &self.profile
    }
    fn tokenizer(&self) -> &Tokenizer {
        &self.tokenizer
    }
    fn begin_request(&mut self) {}
    fn generate_observed(
        &mut self,
        _prompt_ids: &[u32],
        prepared: &PreparedChatCompletion,
        observer: &mut dyn GenerationObserver,
    ) -> Result<(Vec<u32>, GenerationReport), ()> {
        let resultados = prepared
            .input
            .messages
            .iter()
            .filter(|m| m.role == Role::Tool)
            .count();
        if self.retraso_ms > 0 {
            std::thread::sleep(std::time::Duration::from_millis(self.retraso_ms));
        }
        let texto = self.texto(resultados, !prepared.input.tools.is_empty());
        let mut ids = self.tokenizer.encode(&texto);
        for &t in &ids {
            observer.on_token(t);
        }
        ids.push(IM_END);
        Ok((
            ids.clone(),
            GenerationReport {
                generated: ids.len() as u32 - 1,
                prompt_tokens: prepared.prompt_tokens,
                sampled_tokens: ids.len() as u32,
                stop: StopReason::StopToken(IM_END),
            },
        ))
    }
}

fn main() {
    let mut repo = String::from(".");
    let mut port: u16 = 7433;
    let mut retraso_ms: u64 = 0;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--repo" => repo = args.next().unwrap_or(repo),
            "--retraso-ms" => {
                retraso_ms = args.next().and_then(|v| v.parse().ok()).unwrap_or(0)
            }
            "--port" => port = args.next().and_then(|v| v.parse().ok()).unwrap_or(port),
            otro => {
                eprintln!("argumento desconocido: {otro}");
                std::process::exit(2);
            }
        }
    }
    let token = std::env::var("SOSO_LLM_API_KEY").unwrap_or_else(|_| {
        eprintln!("SOSO_LLM_API_KEY no definida");
        std::process::exit(2);
    });
    let raiz = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let tok = raiz.join("tests/self-improvement/reference/tokenizer.som");
    let datos = std::fs::read(&tok).unwrap_or_else(|e| {
        eprintln!("{}: {e}", tok.display());
        std::process::exit(1);
    });
    let tokenizer = Tokenizer::parse(&datos).expect("tokenizer.som");
    let profile = ModelProfile {
        id: String::from("soso-coder"),
        directory: String::from("guion"),
        family: String::from("qwen2"),
        weights_sha256: String::from("0"),
        tokenizer_sha256: String::from("0"),
        template_sha256: String::from("0"),
        context_tokens: 32_768,
        max_output_tokens: 2048,
        stop_token_ids: vec![IM_END, 151_643],
    };
    let service = Arc::new(HostService::new(
        token,
        Guion {
            profile,
            tokenizer,
            repo,
            retraso_ms,
        },
    ));
    let addr: SocketAddr = ([127, 0, 0, 1], port).into();
    let listener = TcpListener::bind(addr).unwrap_or_else(|e| {
        eprintln!("bind {addr}: {e}");
        std::process::exit(1);
    });
    eprintln!("serve_guion escuchando en {addr}");
    let _ = run_accept_loop(service, listener);
}
