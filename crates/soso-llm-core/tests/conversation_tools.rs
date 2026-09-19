//! T07 — parse incremental de `<tool_call>` en salida del asistente.

use serde_json::json;
use soso_llm_core::conversation::{
    parse_assistant_output, AssistantTurn, ChatError, ChatInput, Message, ToolCallParser,
    ToolChoice, ToolDefinition, MAX_ARGUMENTOS_BYTES,
};

fn leer_archivo() -> ToolDefinition {
    ToolDefinition::nueva(
        "leer_archivo",
        Some(String::from("Lee las primeras líneas de un archivo")),
        json!({
            "type": "object",
            "properties": {
                "ruta": {"type": "string"},
                "lineas": {"type": "integer"}
            },
            "required": ["ruta"],
            "additionalProperties": false
        }),
    )
}

fn entrada_auto() -> ChatInput {
    ChatInput::nuevo(vec![Message::user("hola")])
        .con_herramientas(vec![leer_archivo()], ToolChoice::Auto)
}

fn llamada_valida() -> &'static str {
    r#"<tool_call>
{"name": "leer_archivo", "arguments": {"lineas":20,"ruta":"kernel/src/main.rs"}}
</tool_call>"#
}

fn parse_incremental(entrada: &ChatInput, texto: &str, id: u32) -> Result<AssistantTurn, ChatError> {
    let mut p = ToolCallParser::new(id);
    for chunk in fragmentos(texto) {
        p.push(chunk)?;
    }
    p.finish(entrada)
}

fn fragmentos(texto: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let bytes = texto.as_bytes();
    let mut i = 0;
    while i <= bytes.len() {
        if i < bytes.len() {
            let mut end = i + 1;
            while end <= bytes.len() && !texto.is_char_boundary(end) {
                end += 1;
            }
            out.push(&texto[i..end]);
        } else {
            out.push("");
        }
        if i >= bytes.len() {
            break;
        }
        i += 1;
    }
    out
}

fn todas_las_particiones(texto: &str) -> Vec<Vec<&str>> {
    let mut particiones = vec![vec![texto]];
    for i in 1..texto.len() {
        if texto.is_char_boundary(i) {
            particiones.push(vec![&texto[..i], &texto[i..]]);
        }
    }
    particiones
}

#[test]
fn llamada_valida_sin_texto() {
    let entrada = entrada_auto();
    let turno = parse_assistant_output(&entrada, llamada_valida(), 0).expect("parse");
    assert_eq!(turno.content, None);
    let call = turno.tool_call.expect("llamada");
    assert_eq!(call.id, "call_0");
    assert_eq!(call.name, "leer_archivo");
    assert_eq!(call.arguments, r#"{"lineas":20,"ruta":"kernel/src/main.rs"}"#);
}

#[test]
fn texto_antes_de_la_llamada() {
    let entrada = entrada_auto();
    let texto = format!(
        "Uso el lector.\n{}",
        llamada_valida()
    );
    let turno = parse_assistant_output(&entrada, &texto, 3).expect("parse");
    assert_eq!(turno.content.as_deref(), Some("Uso el lector.\n"));
    assert!(turno.tool_call.is_some());
}

#[test]
fn solo_texto_sin_tags() {
    let entrada = entrada_auto();
    let texto = "Explico el comando sin invocar herramientas en este turno.";
    let turno = parse_assistant_output(&entrada, texto, 0).expect("parse");
    assert_eq!(turno.content.as_deref(), Some(texto));
    assert!(turno.tool_call.is_none());
}

#[test]
fn unicode_en_texto_y_argumentos() {
    let entrada = entrada_auto();
    let texto = format!(
        "Archivo café ☕\n<tool_call>\n{{\"name\": \"leer_archivo\", \"arguments\": {{\"ruta\":\"notas.txt\",\"lineas\":1}}}}\n</tool_call>"
    );
    let turno = parse_assistant_output(&entrada, &texto, 0).expect("parse");
    assert!(turno.content.unwrap().contains('☕'));
    let args = turno.tool_call.unwrap().argumentos().unwrap();
    assert_eq!(args["ruta"], "notas.txt");
    assert_eq!(args["lineas"], 1);
}

#[test]
fn particiones_coinciden_con_entrada_completa() {
    let entrada = entrada_auto();
    let casos = [
        llamada_valida(),
        "solo texto plano",
        &format!("prefijo\n{}", llamada_valida()),
    ];
    for caso in casos {
        let esperado = parse_assistant_output(&entrada, caso, 0).expect("referencia");
        for partes in todas_las_particiones(caso) {
            let mut p = ToolCallParser::new(0);
            for (i, chunk) in partes.iter().enumerate() {
                p.push(chunk).unwrap_or_else(|e| {
                    panic!("push falló en partición {:?} chunk {i}: {e}", partes)
                });
            }
            let obtenido = p.finish(&entrada).unwrap_or_else(|e| {
                panic!("finish falló en partición {:?}: {e}", partes)
            });
            assert_eq!(obtenido, esperado, "partición {:?}", partes);
        }
    }
}

#[test]
fn fragmentacion_byte_a_byte() {
    let entrada = entrada_auto();
    let texto = llamada_valida();
    let ref_turno = parse_assistant_output(&entrada, texto, 0).unwrap();
    let inc = parse_incremental(&entrada, texto, 0).unwrap();
    assert_eq!(inc, ref_turno);
}

#[test]
fn json_invalido_no_emite_llamada() {
    let entrada = entrada_auto();
    let texto = r#"<tool_call>
{"name": "leer_archivo", "arguments": {broken}
</tool_call>"#;
    let err = parse_assistant_output(&entrada, texto, 0).unwrap_err();
    assert!(matches!(err, ChatError::ArgumentosInvalidos { .. }));
}

#[test]
fn nombre_desconocido() {
    let entrada = entrada_auto();
    let texto = r#"<tool_call>
{"name": "no_existe", "arguments": {"ruta":"a"}}
</tool_call>"#;
    let err = parse_assistant_output(&entrada, texto, 0).unwrap_err();
    assert!(matches!(err, ChatError::HerramientaDesconocida { .. }));
}

#[test]
fn truncado_sin_cierre() {
    let entrada = entrada_auto();
    let texto = r#"<tool_call>
{"name": "leer_archivo", "arguments": {"ruta":"a"#;
    let err = parse_assistant_output(&entrada, texto, 0).unwrap_err();
    assert!(matches!(err, ChatError::HistorialInvalido { .. }));
}

#[test]
fn dos_llamadas_en_un_turno() {
    let entrada = entrada_auto();
    let texto = format!("{}\n{}", llamada_valida(), llamada_valida());
    let err = parse_assistant_output(&entrada, &texto, 0).unwrap_err();
    assert!(matches!(err, ChatError::HistorialInvalido { .. }));
}

#[test]
fn texto_despues_de_llamada() {
    let entrada = entrada_auto();
    let texto = format!("{}extra", llamada_valida());
    let err = parse_assistant_output(&entrada, &texto, 0).unwrap_err();
    assert!(matches!(err, ChatError::HistorialInvalido { .. }));
}

#[test]
fn limite_argumentos_excedido() {
    let entrada = entrada_auto();
    let gordo = "x".repeat(MAX_ARGUMENTOS_BYTES + 1);
    let texto = format!(
        r#"<tool_call>
{{"name": "leer_archivo", "arguments": {{"ruta":"{gordo}"}}}}
</tool_call>"#
    );
    let err = parse_assistant_output(&entrada, &texto, 0).unwrap_err();
    assert!(matches!(err, ChatError::LimiteExcedido { .. }));
}

#[test]
fn tool_choice_none_rechaza_llamada() {
    let entrada = ChatInput::nuevo(vec![Message::user("hola")])
        .con_herramientas(vec![leer_archivo()], ToolChoice::None);
    let err = parse_assistant_output(&entrada, llamada_valida(), 0).unwrap_err();
    assert!(matches!(err, ChatError::SeleccionInvalida { .. }));
}

#[test]
fn tool_choice_required_exige_llamada() {
    let entrada = ChatInput::nuevo(vec![Message::user("hola")])
        .con_herramientas(vec![leer_archivo()], ToolChoice::Required);
    let err = parse_assistant_output(&entrada, "solo texto", 0).unwrap_err();
    assert!(matches!(err, ChatError::SeleccionInvalida { .. }));
}

#[test]
fn tool_choice_named_exige_nombre() {
    let entrada = ChatInput::nuevo(vec![Message::user("hola")])
        .con_herramientas(vec![leer_archivo()], ToolChoice::Named(String::from("leer_archivo")));
    let turno = parse_assistant_output(&entrada, llamada_valida(), 0).expect("ok");
    assert_eq!(turno.tool_call.unwrap().name, "leer_archivo");

    let otro = ChatInput::nuevo(vec![Message::user("hola")])
        .con_herramientas(vec![leer_archivo()], ToolChoice::Named(String::from("otra")));
    let err = parse_assistant_output(&otro, llamada_valida(), 0).unwrap_err();
    assert!(
        matches!(err, ChatError::SeleccionInvalida { .. })
            || matches!(err, ChatError::HerramientaDesconocida { .. })
    );
}

#[test]
fn delimitador_partido_entre_chunks() {
    let entrada = entrada_auto();
    let texto = llamada_valida();
    let split = texto.find("{").unwrap();
    let mut p = ToolCallParser::new(0);
    p.push(&texto[..split]).unwrap();
    p.push(&texto[split..]).unwrap();
    let turno = p.finish(&entrada).unwrap();
    assert!(turno.tool_call.is_some());
}
