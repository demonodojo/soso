//! T06 — render Qwen2 frente a los cinco fixtures de referencia (T03).

use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::Value;
use soso_llm_core::conversation::{
    render_messages, ChatInput, Message, ModelProfile, Role, ToolCall, ToolChoice, ToolDefinition,
    ChatError,
};
use soso_llm_core::tokenizer::Tokenizer;

fn raiz_repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("raíz del repo")
}

fn dir_fixtures() -> PathBuf {
    raiz_repo().join("tests/self-improvement/reference")
}

fn tokenizer_qwen() -> Option<Tokenizer> {
    let ruta = raiz_repo().join("target/qwen2.5-coder-3b-model/tokenizer.som");
    let datos = std::fs::read(ruta).ok()?;
    let tok = Tokenizer::parse(&datos).ok()?;
    tok.tiene_vocabulario().then_some(tok)
}

fn perfil_qwen() -> ModelProfile {
    ModelProfile {
        id: String::from("soso-coder"),
        directory: String::from("target/qwen2.5-coder-3b-model"),
        family: String::from("qwen2"),
        weights_sha256: String::from("0"),
        tokenizer_sha256: String::from("0"),
        template_sha256: String::from("0"),
        context_tokens: 32_768,
        max_output_tokens: 2048,
        stop_token_ids: vec![151645, 151643],
    }
}

#[derive(Deserialize)]
struct Fixture {
    nombre: String,
    mensajes: Value,
    #[serde(default)]
    herramientas: Option<Value>,
    texto: String,
    ids: Vec<u32>,
}

fn mensajes_desde_hf(valor: &Value) -> Vec<Message> {
    let arr = valor.as_array().expect("mensajes es array");
    arr.iter().map(mensaje_desde_hf).collect()
}

fn mensaje_desde_hf(v: &Value) -> Message {
    let role = match v["role"].as_str().unwrap_or("user") {
        "system" => Role::System,
        "user" => Role::User,
        "assistant" => Role::Assistant,
        "tool" => Role::Tool,
        otro => panic!("rol desconocido: {otro}"),
    };
    let content = v.get("content").and_then(|c| {
        if c.is_null() {
            None
        } else {
            Some(c.as_str().unwrap_or("").to_string())
        }
    });
    let mut tool_calls = Vec::new();
    if let Some(calls) = v.get("tool_calls").and_then(|c| c.as_array()) {
        for (i, call) in calls.iter().enumerate() {
            let func = call.get("function").unwrap_or(call);
            let name = func["name"].as_str().unwrap().to_string();
            let args = &func["arguments"];
            let arguments = if args.is_string() {
                args.as_str().unwrap().to_string()
            } else {
                serde_json::to_string(args).unwrap()
            };
            tool_calls.push(ToolCall::nueva(
                format!("call_{i}"),
                name,
                arguments,
            ));
        }
    }
    let tool_call_id = v
        .get("tool_call_id")
        .and_then(|x| x.as_str())
        .map(String::from);
    Message {
        role,
        content,
        tool_calls,
        tool_call_id,
    }
}

fn herramientas_desde_hf(valor: &Value) -> Vec<ToolDefinition> {
    let arr = valor.as_array().expect("herramientas es array");
    arr.iter()
        .map(|t| {
            let func = t.get("function").expect("function");
            ToolDefinition::nueva(
                func["name"].as_str().unwrap(),
                func.get("description")
                    .and_then(|d| d.as_str())
                    .map(String::from),
                func["parameters"].clone(),
            )
        })
        .collect()
}

/// Los fixtures HF no traen `tool_call_id`; el dominio sí lo exige (T05).
fn enlazar_resultados_de_herramienta(messages: &mut [Message]) {
    for i in 0..messages.len() {
        if messages[i].role != Role::Tool || messages[i].tool_call_id.is_some() {
            continue;
        }
        for j in (0..i).rev() {
            if messages[j].role == Role::Assistant && !messages[j].tool_calls.is_empty() {
                messages[i].tool_call_id = Some(messages[j].tool_calls[0].id.clone());
                break;
            }
        }
    }
}

fn entrada_desde_fixture(f: &Fixture) -> ChatInput {
    let mut messages = mensajes_desde_hf(&f.mensajes);
    enlazar_resultados_de_herramienta(&mut messages);
    let mut input = ChatInput::nuevo(messages);
    if let Some(h) = &f.herramientas {
        let tools = herramientas_desde_hf(h);
        input = input.con_herramientas(tools, ToolChoice::Auto);
    }
    input
}

const CASOS: &[&str] = &[
    "simple",
    "system",
    "historial",
    "herramienta-esquema",
    "herramienta-resultado",
];

#[test]
fn los_cinco_fixtures_coinciden_en_texto_y_tokens() {
    let Some(tokenizer) = tokenizer_qwen() else {
        panic!(
            "falta target/qwen2.5-coder-3b-model/tokenizer.som; convierte el modelo del perfil T03"
        );
    };
    let perfil = perfil_qwen();
    for nombre in CASOS {
        let ruta = dir_fixtures().join(format!("{nombre}.json"));
        let raw = std::fs::read_to_string(&ruta).unwrap_or_else(|e| panic!("{ruta:?}: {e}"));
        let f: Fixture = serde_json::from_str(&raw).unwrap();
        let input = entrada_desde_fixture(&f);
        let ids = render_messages(&input, &perfil, &tokenizer)
            .unwrap_or_else(|e| panic!("{}: {e}", f.nombre));
        assert_eq!(
            ids, f.ids,
            "{}: divergencia de tokens ({} vs {})",
            f.nombre,
            ids.len(),
            f.ids.len()
        );
        let texto = soso_llm_core::conversation::render::render_qwen2(&input, true).unwrap();
        assert_eq!(
            texto, f.texto,
            "{}: el texto renderizado no coincide con la referencia",
            f.nombre
        );
    }
}

#[test]
fn otra_familia_devuelve_error() {
    let Some(tokenizer) = tokenizer_qwen() else {
        return;
    };
    let input = ChatInput::nuevo(vec![Message::user("hola")]);
    let mut perfil = perfil_qwen();
    perfil.family = String::from("llama3");
    match render_messages(&input, &perfil, &tokenizer) {
        Err(ChatError::FamiliaNoImplementada { family }) => assert_eq!(family, "llama3"),
        otro => panic!("esperaba FamiliaNoImplementada, obtuve {otro:?}"),
    }
}
