//! Render de Qwen3-Coder contra la plantilla Jinja oficial.
//!
//! Los casos (`tests/self-improvement/reference/qwen3coder/casos.json`) los genera `generar.py`
//! con `chat_template.jinja` de `Qwen/Qwen3-Coder-30B-A3B-Instruct` (sha256 en `LEEME.md`).

use std::path::Path;

use serde_json::Value;
use soso_llm_core::conversation::render::render_qwen3coder;
use soso_llm_core::conversation::{ChatInput, Message, Role, ToolCall, ToolChoice, ToolDefinition};

fn casos() -> Vec<Value> {
    let ruta = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/self-improvement/reference/qwen3coder/casos.json");
    serde_json::from_str(&std::fs::read_to_string(ruta).expect("casos.json")).unwrap()
}

fn mensaje(m: &Value) -> Message {
    let contenido = m["content"].as_str().unwrap_or("").to_string();
    match m["role"].as_str().unwrap() {
        "system" => Message::texto(Role::System, contenido),
        "user" => Message::user(contenido),
        "assistant" => {
            if let Some(llamadas) = m.get("tool_calls").and_then(Value::as_array) {
                let mut msg = Message::llamadas(
                    llamadas
                        .iter()
                        .map(|c| {
                            ToolCall::nueva(
                                c["id"].as_str().unwrap(),
                                c["function"]["name"].as_str().unwrap(),
                                c["function"]["arguments"].to_string(),
                            )
                        })
                        .collect(),
                );
                if !contenido.is_empty() {
                    msg.content = Some(contenido);
                }
                msg
            } else {
                Message::texto(Role::Assistant, contenido)
            }
        }
        "tool" => Message::resultado(m["tool_call_id"].as_str().unwrap(), contenido),
        otro => panic!("rol {otro}"),
    }
}

fn entrada(c: &Value) -> ChatInput {
    let mensajes: Vec<Message> = c["messages"].as_array().unwrap().iter().map(mensaje).collect();
    let herramientas: Vec<ToolDefinition> = c["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| {
            let f = &t["function"];
            ToolDefinition::nueva(
                f["name"].as_str().unwrap(),
                f.get("description").and_then(Value::as_str).map(String::from),
                f["parameters"].clone(),
            )
        })
        .collect();
    ChatInput::nuevo(mensajes).con_herramientas(herramientas, ToolChoice::Auto)
}

#[test]
fn coincide_byte_a_byte_con_la_plantilla_oficial() {
    let todos = casos();
    assert!(todos.len() >= 8);
    for caso in &todos {
        let nombre = caso["nombre"].as_str().unwrap();
        let obtenido = render_qwen3coder(&entrada(&caso["entrada"]), true).expect(nombre);
        let esperado = caso["esperado"].as_str().unwrap();
        assert_eq!(obtenido, esperado, "caso {nombre}");
    }
}
