//! Plantilla de chat y parser de Gemma 4 contra llama.cpp (G6 de `docs/PLAN-GEMMA4.md`).
//!
//! `fixtures/gemma4_plantilla.json`: por escenario, el prompt que produjo `/apply-template` de llama-server
//! con la plantilla oficial, con y sin `enable_thinking`. Sin modelo: corre siempre.
#![cfg(feature = "std")]

use soso_llm_core::conversation::gemma4::{interpretar, render_gemma4};
use soso_llm_core::conversation::{ChatInput, Message, ToolDefinition};

#[test]
fn render_igual_que_apply_template_de_llama_cpp() {
    let raiz = env!("CARGO_MANIFEST_DIR");
    let datos = std::fs::read_to_string(format!("{raiz}/tests/fixtures/gemma4_plantilla.json")).unwrap();
    let casos: serde_json::Value = serde_json::from_str(&datos).unwrap();
    let mut fallos = Vec::new();
    for c in casos.as_array().unwrap() {
        let mensajes: Vec<Message> = serde_json::from_value(c["messages"].clone()).unwrap();
        let tools: Vec<ToolDefinition> = serde_json::from_value(c["tools"].clone()).unwrap();
        let entrada = ChatInput::nuevo(mensajes).con_herramientas(tools, Default::default());
        for (clave, pensar) in [("sin_pensar", false), ("pensando", true)] {
            let esperado = c[clave].as_str().unwrap();
            let obtenido = render_gemma4(&entrada, true, pensar).unwrap();
            if obtenido != esperado {
                let i = obtenido.chars().zip(esperado.chars()).take_while(|(a, b)| a == b).count();
                let ctx = |t: &str| t.chars().skip(i.saturating_sub(25)).take(70).collect::<String>();
                fallos.push(format!("{} [{clave}] difiere en {i}:\n  soso:  {:?}\n  llama: {:?}", c["nombre"], ctx(&obtenido), ctx(esperado)));
            }
        }
    }
    assert!(fallos.is_empty(), "{}", fallos.join("\n"));
}

fn entrada_con_read() -> ChatInput {
    let t: ToolDefinition = serde_json::from_value(serde_json::json!({
        "name": "read",
        "parameters": {"type": "object", "properties": {"filePath": {"type": "string"}, "limit": {"type": "integer"}}, "required": ["filePath"]}
    }))
    .unwrap();
    ChatInput::nuevo(vec![Message::user("lee")]).con_herramientas(vec![t], Default::default())
}

#[test]
fn llamada_a_herramienta() {
    let e = entrada_con_read();
    let t = interpretar(&e, "<|tool_call>call:read{filePath:<|\"|>src/lib.rs<|\"|>,limit:5}<tool_call|>", 3, false).unwrap();
    let c = t.tool_call.expect("llamada");
    assert_eq!((c.id.as_str(), c.name.as_str()), ("call_3", "read"));
    let v: serde_json::Value = serde_json::from_str(&c.arguments).unwrap();
    assert_eq!(v, serde_json::json!({"filePath": "src/lib.rs", "limit": 5}));
    assert_eq!(t.content, None);
}

#[test]
fn llamada_con_texto_pensamiento_y_cierre_de_turno() {
    let e = entrada_con_read();
    let t = interpretar(
        &e,
        "<|channel>thought\nvoy a leer<channel|>Leo el archivo.<|tool_call>call:read{filePath:<|\"|>a, b}<|\"|>}<tool_call|><|tool_response>",
        1,
        false,
    )
    .unwrap();
    assert_eq!(t.content.as_deref(), Some("Leo el archivo."));
    // una cadena puede contener `,` y `}`
    let v: serde_json::Value = serde_json::from_str(&t.tool_call.unwrap().arguments).unwrap();
    assert_eq!(v["filePath"], "a, b}");
}

#[test]
fn respuesta_sin_llamada_y_argumentos_anidados() {
    let e = entrada_con_read();
    let t = interpretar(&e, "Hecho.<turn|>", 1, false).unwrap();
    assert_eq!((t.content.as_deref(), t.tool_call.is_none()), (Some("Hecho."), true));
    let t = interpretar(&e, "<|tool_call>call:read{filePath:<|\"|>x<|\"|>,limit:3}", 1, false).unwrap(); // sin cierre
    assert!(t.tool_call.is_some());
    let t = interpretar(&e, "<|tool_call>call:read{filePath:<|\"|>x<|\"|>,extra:{a:[1,true,null,<|\"|>s<|\"|>]}}<tool_call|>", 1, true).unwrap();
    let v: serde_json::Value = serde_json::from_str(&t.tool_call.unwrap().arguments).unwrap();
    assert_eq!(v["extra"], serde_json::json!({"a": [1, true, null, "s"]}));
}

#[test]
fn llamada_mal_formada_es_error() {
    let e = entrada_con_read();
    assert!(interpretar(&e, "<|tool_call>call:read{filePath:<|\"|>sin cerrar", 1, true).is_err());
}
