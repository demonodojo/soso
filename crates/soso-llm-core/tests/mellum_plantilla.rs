//! El prompt que construye soso para Mellum 2.1 (familia `qwen2`: ChatML con herramientas en JSON) frente al de la
//! plantilla oficial vía `/apply-template` de llama.cpp. Los mismos escenarios que `gemma4_plantilla`. Sin modelo.
#![cfg(feature = "std")]

use soso_llm_core::conversation::render::render_mellum;
use soso_llm_core::conversation::{ChatInput, Message, ToolDefinition};

#[test]
fn prompt_de_mellum_igual_que_la_plantilla_oficial() {
    let raiz = env!("CARGO_MANIFEST_DIR");
    let datos = std::fs::read_to_string(format!("{raiz}/tests/fixtures/mellum_plantilla.json")).unwrap();
    let casos: serde_json::Value = serde_json::from_str(&datos).unwrap();
    // Las líneas que son JSON (herramientas y llamadas) se comparan como JSON: el `Value` de serde_json ordena las
    // claves, la plantilla las conserva en el orden de inserción; el contenido es el mismo.
    fn canonico(t: &str) -> String {
        t.lines()
            .map(|l| match serde_json::from_str::<serde_json::Value>(l) {
                Ok(v) if l.starts_with('{') => v.to_string(),
                _ => l.to_string(),
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
    let mut iguales = 0;
    let mut difieren = Vec::new();
    for c in casos.as_array().unwrap() {
        let mensajes: Vec<Message> = serde_json::from_value(c["messages"].clone()).unwrap();
        let tools: Vec<ToolDefinition> = serde_json::from_value(c["tools"].clone()).unwrap();
        let entrada = ChatInput::nuevo(mensajes).con_herramientas(tools, Default::default());
        // Escenario propio de Gemma (marcas de canal en el contenido): no aplica a Mellum.
        if c["nombre"] == "pensamiento en el historial" {
            continue;
        }
        let esperado = canonico(c["prompt"].as_str().unwrap());
        let esperado = esperado.as_str();
        let obtenido = canonico(&render_mellum(&entrada, true).unwrap());
        if obtenido == esperado {
            iguales += 1;
        } else {
            let i = obtenido.chars().zip(esperado.chars()).take_while(|(a, b)| a == b).count();
            let ctx = |t: &str| t.chars().skip(i.saturating_sub(30)).take(80).collect::<String>();
            difieren.push(format!("{} difiere en {i}:\n  soso:  {:?}\n  llama: {:?}", c["nombre"], ctx(&obtenido), ctx(esperado)));
        }
    }
    eprintln!("{iguales} iguales, {} distintos", difieren.len());
    assert!(difieren.is_empty(), "{}", difieren.join("\n"));
}
