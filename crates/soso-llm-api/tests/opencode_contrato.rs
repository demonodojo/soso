//! T21 — contraste del contrato C3 con peticiones **reales** de OpenCode.
//!
//! Los fixtures de `tests/self-improvement/opencode/` los capturó un servidor
//! guionizado frente a OpenCode 1.18.34 (ver el `LEEME.md` de esa carpeta). Aquí
//! se pasan por el mismo decodificador y validador que usa el servidor.

use std::path::{Path, PathBuf};

use soso_llm_api::{
    parse_chat_completions, prepare_chat_completion, validate_wire_policy, wire_to_chat_input,
};
use soso_llm_core::conversation::{ModelProfile, Role, MAX_HERRAMIENTAS, MAX_MENSAJES};
use soso_llm_core::tokenizer::Tokenizer;

fn raiz() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("raíz del repo")
}

fn fixtures() -> PathBuf {
    raiz().join("tests/self-improvement/opencode")
}

fn peticiones() -> Vec<(String, String)> {
    let mut v: Vec<_> = std::fs::read_dir(fixtures())
        .expect("falta tests/self-improvement/opencode")
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let n = e.file_name().to_string_lossy().into_owned();
            (n.starts_with("peticion-") && n.ends_with(".json")).then(|| {
                let t = std::fs::read_to_string(e.path()).unwrap();
                (n, t)
            })
        })
        .collect();
    v.sort();
    v
}

fn perfil() -> ModelProfile {
    ModelProfile {
        id: String::from("soso-coder"),
        directory: String::from("target/qwen2.5-coder-7b-model"),
        family: String::from("qwen2"),
        weights_sha256: String::from("0"),
        tokenizer_sha256: String::from("0"),
        template_sha256: String::from("0"),
        context_tokens: 32_768,
        max_output_tokens: 2048,
        stop_token_ids: vec![151645, 151643],
    }
}

fn tokenizer() -> Tokenizer {
    let convertido = raiz().join("target/qwen2.5-coder-7b-model/tokenizer.som");
    let ruta = if convertido.is_file() {
        convertido
    } else {
        raiz().join("tests/self-improvement/reference/tokenizer.som")
    };
    let datos = std::fs::read(&ruta).unwrap_or_else(|_| panic!("falta {}", ruta.display()));
    Tokenizer::parse(&datos).expect("tokenizer")
}

#[test]
fn hay_cuatro_peticiones_capturadas() {
    assert_eq!(peticiones().len(), 4, "se esperaban las 4 capturas de OpenCode");
}

#[test]
fn la_peticion_de_titulo_tambien_se_decodifica() {
    // Sólo existe si no se desactiva el agente `title` (ver docs/self-improvement/opencode.md):
    // sin herramientas y con dos mensajes `user` seguidos tras el `system`.
    let t = std::fs::read_to_string(fixtures().join("titulo-sin-desactivar.json")).unwrap();
    let wire = parse_chat_completions(&t).expect("decodifica");
    validate_wire_policy(&wire).expect("política");
    let input = wire_to_chat_input(&wire).expect("conversión");
    assert!(input.tools.is_empty());
    let roles: Vec<Role> = input.messages.iter().map(|m| m.role).collect();
    assert_eq!(roles, [Role::System, Role::User, Role::User]);
}

#[test]
fn opencode_pasa_la_politica_y_el_decodificador() {
    for (nombre, cuerpo) in peticiones() {
        let wire = parse_chat_completions(&cuerpo)
            .unwrap_or_else(|e| panic!("{nombre}: no se decodifica: {}", e.message()));
        validate_wire_policy(&wire)
            .unwrap_or_else(|e| panic!("{nombre}: la política la rechaza: {}", e.message()));
        let input = wire_to_chat_input(&wire)
            .unwrap_or_else(|e| panic!("{nombre}: no se convierte: {}", e.message()));
        assert!(input.messages.len() <= MAX_MENSAJES, "{nombre}");
        assert!(input.tools.len() <= MAX_HERRAMIENTAS, "{nombre}");
    }
}

#[test]
fn opencode_se_prepara_y_cabe_en_el_contexto() {
    let tok = tokenizer();
    let perfil = perfil();
    for (nombre, cuerpo) in peticiones() {
        let p = prepare_chat_completion(&cuerpo, &perfil, &tok)
            .unwrap_or_else(|e| panic!("{nombre}: no se prepara: {}", e.message()));
        assert!(p.stream, "{nombre}: OpenCode siempre pide stream");
        assert!(p.include_usage, "{nombre}: pide usage");
        assert!(p.max_new_tokens <= 2048, "{nombre}");
        assert!(p.prompt_tokens + p.max_new_tokens <= 32_768, "{nombre}");
    }
}

#[test]
fn el_historial_de_herramientas_conserva_el_orden() {
    let (_, cuerpo) = peticiones().pop().unwrap();
    let wire = parse_chat_completions(&cuerpo).unwrap();
    let input = wire_to_chat_input(&wire).unwrap();
    let roles: Vec<Role> = input.messages.iter().map(|m| m.role).collect();
    assert_eq!(roles[0], Role::System);
    assert_eq!(roles[1], Role::User);
    for par in roles[2..].chunks(2) {
        assert_eq!(par, [Role::Assistant, Role::Tool]);
    }
}

// ---------------------------------------------------------------------------
// Regresión de T21: el servidor numera las llamadas continuando el historial.
// Con OpenCode real, la segunda llamada de una sesión salía otra vez como
// `call_1`, OpenCode la reenviaba y el propio servidor la rechazaba por duplicada.

#[cfg(feature = "std")]
mod ids {
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::Arc;
    use std::thread;

    use soso_llm_api::{run_accept_loop, ChatBackend, HostService, PreparedChatCompletion};
    use soso_llm_core::conversation::ModelProfile;
    use soso_llm_core::generation::{GenerationObserver, GenerationReport, StopReason};
    use soso_llm_core::tokenizer::Tokenizer;

    struct SiempreLlama {
        profile: ModelProfile,
        tokenizer: Tokenizer,
    }

    impl ChatBackend for SiempreLlama {
        fn backend_name(&self) -> &'static str {
            "siempre-llama"
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
            let texto = "<tool_call>\n{\"name\": \"read\", \"arguments\": {\"filePath\": \"/a\"}}\n</tool_call>";
            let ids = self.tokenizer.encode(texto);
            for &t in &ids {
                observer.on_token(t);
            }
            Ok((
                ids.clone(),
                GenerationReport {
                    generated: ids.len() as u32,
                    prompt_tokens: prepared.prompt_tokens,
                    sampled_tokens: ids.len() as u32,
                    stop: StopReason::StopToken(151_645),
                },
            ))
        }
    }

    fn peticion(historial: &str) -> String {
        let cuerpo = format!(
            r#"{{"model":"soso-coder","stream":false,"tool_choice":"auto",
"messages":[{{"role":"user","content":"lee"}}{historial}],
"tools":[{{"type":"function","function":{{"name":"read","description":"lee",
"parameters":{{"type":"object","properties":{{"filePath":{{"type":"string"}}}},"required":["filePath"]}}}}}}]}}"#
        );
        format!(
            "POST /v1/chat/completions HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer k\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            cuerpo.len(),
            cuerpo
        )
    }

    fn id_de_la_respuesta(historial: &str) -> String {
        let perfil = super::perfil();
        let service = Arc::new(HostService::new(
            String::from("k"),
            SiempreLlama {
                profile: perfil,
                tokenizer: super::tokenizer(),
            },
        ));
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        thread::spawn(move || {
            let _ = run_accept_loop(service, listener);
        });
        let mut s = TcpStream::connect(addr).unwrap();
        s.write_all(peticion(historial).as_bytes()).unwrap();
        s.shutdown(std::net::Shutdown::Write).unwrap();
        let mut out = String::new();
        s.read_to_string(&mut out).unwrap();
        assert!(out.starts_with("HTTP/1.1 200"), "{out}");
        let cuerpo = out.split("\r\n\r\n").nth(1).unwrap();
        let v: serde_json::Value = serde_json::from_str(cuerpo).unwrap();
        v["choices"][0]["message"]["tool_calls"][0]["id"]
            .as_str()
            .unwrap_or_else(|| panic!("sin tool_calls: {cuerpo}"))
            .to_string()
    }

    #[test]
    fn primera_llamada_es_call_1() {
        assert_eq!(id_de_la_respuesta(""), "call_1");
    }

    #[test]
    fn con_una_llamada_previa_la_nueva_es_call_2() {
        let historial = r#",{"role":"assistant","content":"","tool_calls":[{"id":"call_1","type":"function","function":{"name":"read","arguments":"{\"filePath\":\"/a\"}"}}]},
{"role":"tool","tool_call_id":"call_1","content":"ok"}"#;
        assert_eq!(id_de_la_respuesta(historial), "call_2");
    }
}
