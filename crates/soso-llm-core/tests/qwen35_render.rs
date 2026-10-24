//! Render de Qwen3.5/3.6 contra `/apply-template` de llama.cpp (plantilla Jinja del GGUF de Unsloth).
//! Los casos los genera `tests/self-improvement/reference/qwen35/generar.py`.

use std::path::Path;

use serde_json::Value;
use soso_llm_core::conversation::render::render_qwen35;
use soso_llm_core::conversation::{ChatInput, Message, Role, ToolCall, ToolChoice, ToolDefinition};

fn casos() -> Vec<Value> {
    let ruta = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/self-improvement/reference/qwen35/casos.json");
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
                            let a = &c["function"]["arguments"];
                            let args = a.as_str().map(String::from).unwrap_or_else(|| a.to_string());
                            ToolCall::nueva(c["id"].as_str().unwrap(), c["function"]["name"].as_str().unwrap(), args)
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
fn coincide_byte_a_byte_con_llama_cpp() {
    let todos = casos();
    assert!(todos.len() >= 8);
    for caso in &todos {
        let nombre = caso["nombre"].as_str().unwrap();
        let e = entrada(&caso["entrada"]);
        assert_eq!(render_qwen35(&e, true, false).expect(nombre), caso["esperado"].as_str().unwrap(), "caso {nombre} (sin pensar)");
        assert_eq!(render_qwen35(&e, true, true).expect(nombre), caso["esperado_pensar"].as_str().unwrap(), "caso {nombre} (pensando)");
    }
}
