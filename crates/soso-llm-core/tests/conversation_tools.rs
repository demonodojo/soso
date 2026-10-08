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

/// Lo que escribió el Coder-7B en Q07: el JSON correcto, en una cerca Markdown.
fn llamada_en_markdown() -> &'static str {
    "```json\n{\"name\": \"leer_archivo\", \"arguments\": {\"ruta\": \"kernel/src/main.rs\", \"lineas\": 20}}\n```"
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
        llamada_en_markdown(),
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
fn cerca_markdown_es_la_llamada() {
    let entrada = ChatInput::nuevo(vec![Message::user("hola")])
        .con_herramientas(vec![leer_archivo()], ToolChoice::Required);
    let turno = parse_assistant_output(&entrada, llamada_en_markdown(), 1).expect("parse");
    assert_eq!(turno.content, None);
    let call = turno.tool_call.expect("llamada");
    assert_eq!(call.id, "call_1");
    assert_eq!(call.name, "leer_archivo");
    let args = call.argumentos().unwrap();
    assert_eq!(args["ruta"], "kernel/src/main.rs");
    assert_eq!(args["lineas"], 20);
}

// Decisión de T22 (2026-10-08), que invierte la de T07: el 7B escribe a menudo la llamada
// como un JSON suelto, sin etiqueta ni cerca. Si es **todo** el turno y nombra una
// herramienta declarada con argumentos objeto, es una llamada. Lo que no cumple eso sigue
// siendo texto (ver `json_desnudo_que_no_es_una_llamada_se_queda_como_texto`).
#[test]
fn json_suelto_con_herramienta_declarada_es_llamada() {
    let entrada = entrada_auto();
    let texto = r#"{"name": "leer_archivo", "arguments": {"ruta": "a", "lineas": 1}}"#;
    let turno = parse_assistant_output(&entrada, texto, 0).expect("parse");
    let call = turno.tool_call.expect("llamada");
    assert_eq!(call.name, "leer_archivo");
    assert_eq!(turno.content, None);
}

#[test]
fn cerca_markdown_con_json_cortado_es_truncada() {
    let entrada = entrada_auto();
    // El JSON no está completo: aquí sí es una llamada truncada. (Con el JSON completo y
    // sin cerca de cierre, la llamada vale: `llamada_sin_cerrar_la_cerca_…`.)
    let texto = "```json\n{\"name\": \"leer_archivo\", \"arguments\": {\"ruta\": \"a.r";
    let err = parse_assistant_output(&entrada, texto, 0).unwrap_err();
    assert!(matches!(err, ChatError::HistorialInvalido { .. }));
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

/// Reproducción del panic que mataba `soso-llm serve` en el guest (T56): el
/// modelo acaba una respuesta en un carácter multibyte y el buscador de
/// etiquetas partidas cortaba por índice de byte dentro de ese carácter.
#[test]
fn texto_acabado_en_caracter_multibyte_no_revienta() {
    let entrada = ChatInput::nuevo(vec![Message::user("hola")]);
    for cola in ["¡", "☕", "ñ", "日本語", "¡Hola!¡"] {
        let texto = alloc_texto(cola);
        let turno = parse_assistant_output(&entrada, &texto, 0)
            .unwrap_or_else(|e| panic!("parse falló con cola {cola:?}: {e:?}"));
        assert!(
            turno.content.as_deref().unwrap_or("").ends_with(cola),
            "se perdió la cola {cola:?}"
        );
    }
}

/// Rellena hasta pasar de los 128 bytes para que el corte caiga lejos del
/// principio, como en la salida real que lo destapó.
fn alloc_texto(cola: &str) -> String {
    let mut s = String::from("respuesta del modelo ");
    while s.len() < 130 {
        s.push_str("bla ");
    }
    s.push_str(cola);
    s
}

// T21: los ids de llamada no pueden repetirse entre turnos. OpenCode reenvía el
// historial con los ids que le dimos y la validación rechaza los duplicados.
#[test]
fn los_ids_de_llamada_continuan_la_numeracion_del_historial() {
    use soso_llm_core::conversation::ToolCall;
    let vacio = ChatInput::nuevo(vec![Message::user("hola")]);
    assert_eq!(vacio.siguiente_id_llamada(), 1);

    let con_dos = ChatInput::nuevo(vec![
        Message::user("hola"),
        Message::llamadas(vec![ToolCall::nueva("call_1", "a", "{}")]),
        Message::resultado("call_1", "ok"),
        Message::llamadas(vec![ToolCall::nueva("call_2", "a", "{}")]),
        Message::resultado("call_2", "ok"),
    ]);
    assert_eq!(con_dos.siguiente_id_llamada(), 3);

    // Ids que no son nuestros no cuentan ni rompen la numeración.
    let ajeno = ChatInput::nuevo(vec![
        Message::user("hola"),
        Message::llamadas(vec![ToolCall::nueva("toolu_xyz", "a", "{}")]),
    ]);
    assert_eq!(ajeno.siguiente_id_llamada(), 1);
}

// T22: formas con las que el 7B escribe una llamada (vistas con el prompt real de OpenCode).
#[test]
fn llamada_sin_cerrar_la_cerca_o_la_etiqueta_vale_si_el_json_esta_completo() {
    let entrada = entrada_auto();
    // Cerca abierta y el turno termina sin cerrarla.
    let t = parse_assistant_output(
        &entrada,
        "```json\n{\"name\": \"leer_archivo\", \"arguments\": {\"ruta\": \"a.rs\"}}\n",
        1,
    )
    .expect("cerca sin cerrar");
    assert_eq!(t.tool_call.expect("llamada").name, "leer_archivo");
    // Etiqueta abierta sin `</tool_call>`.
    let t = parse_assistant_output(
        &entrada,
        "<tool_call>\n{\"name\": \"leer_archivo\", \"arguments\": {\"ruta\": \"a.rs\"}}",
        1,
    )
    .expect("etiqueta sin cerrar");
    assert!(t.tool_call.is_some());
}

#[test]
fn json_cortado_sigue_siendo_llamada_truncada() {
    let entrada = entrada_auto();
    for texto in [
        "```json\n{\"name\": \"leer_archivo\", \"arguments\": {\"ruta\": \"a.r",
        "<tool_call>\n{\"name\": \"leer_archivo\", \"arguments\": {\"ruta\":",
    ] {
        let err = parse_assistant_output(&entrada, texto, 1).unwrap_err();
        assert!(
            matches!(err, ChatError::HistorialInvalido { ref motivo } if motivo.contains("truncada")),
            "{texto}: {err:?}"
        );
    }
}

#[test]
fn json_desnudo_con_herramienta_declarada_es_una_llamada() {
    let entrada = entrada_auto();
    let t = parse_assistant_output(
        &entrada,
        "{\"name\": \"leer_archivo\", \"arguments\": {\"ruta\": \"a.rs\", \"lineas\": 3}}",
        4,
    )
    .expect("json desnudo");
    let c = t.tool_call.expect("llamada");
    assert_eq!((c.id.as_str(), c.name.as_str()), ("call_4", "leer_archivo"));
    assert_eq!(t.content, None);
}

#[test]
fn json_desnudo_que_no_es_una_llamada_se_queda_como_texto() {
    let entrada = entrada_auto();
    for texto in [
        // herramienta que no existe
        "{\"name\": \"borrar_todo\", \"arguments\": {}}",
        // argumentos que no son un objeto
        "{\"name\": \"leer_archivo\", \"arguments\": \"a.rs\"}",
        // JSON cualquiera
        "{\"a\": 1}",
        // hay texto alrededor: no es todo el turno
        "Aquí va: {\"name\": \"leer_archivo\", \"arguments\": {\"ruta\": \"a.rs\"}}",
    ] {
        let t = parse_assistant_output(&entrada, texto, 1).expect(texto);
        assert!(t.tool_call.is_none(), "{texto}");
        assert_eq!(t.content.as_deref(), Some(texto));
    }
}

#[test]
fn varias_llamadas_en_un_bloque_se_atiende_la_primera() {
    let entrada = entrada_auto();
    let dos = "{\"name\": \"leer_archivo\", \"arguments\": {\"ruta\": \"a.rs\"}}\n{\"name\": \"leer_archivo\", \"arguments\": {\"ruta\": \"b.rs\"}}";
    for texto in [
        format!("```json\n{dos}\n```"),
        format!("<tool_call>\n{dos}\n</tool_call>"),
        dos.to_string(),
    ] {
        let t = parse_assistant_output(&entrada, &texto, 1).expect(&texto);
        let c = t.tool_call.expect("llamada");
        assert_eq!(c.arguments, "{\"ruta\":\"a.rs\"}", "{texto}");
    }
    // La primera incompleta no se salva por haber una segunda detrás.
    let rota = "```json\n{\"name\": \"leer_archivo\", \"arguments\": {\"ruta\": \n{\"name\": \"leer_archivo\", \"arguments\": {\"ruta\": \"b.rs\"}}\n```";
    assert!(parse_assistant_output(&entrada, rota, 1).is_err());
}

// T22: el 7B cierra `arguments` y se olvida del `}` del objeto exterior.
#[test]
fn falta_la_llave_final_se_completa_pero_un_valor_cortado_no() {
    let entrada = entrada_auto();
    let sin_llave = "{\"name\": \"leer_archivo\", \"arguments\": {\"ruta\": \"a.rs\", \"lineas\": 3}";
    for texto in [
        sin_llave.to_string(),
        format!("```json\n{sin_llave}\n```"),
        format!("<tool_call>\n{sin_llave}\n</tool_call>"),
    ] {
        let t = parse_assistant_output(&entrada, &texto, 1).expect(&texto);
        assert_eq!(t.tool_call.expect("llamada").arguments, "{\"lineas\":3,\"ruta\":\"a.rs\"}", "{texto}");
    }
    // Cortado en mitad de un valor o de una cadena: sigue sin ser una llamada.
    for texto in [
        "```json\n{\"name\": \"leer_archivo\", \"arguments\": {\"ruta\": \"a.r",
        "```json\n{\"name\": \"leer_archivo\", \"arguments\": {\"ruta\":",
        "```json\n{\"name\": \"leer_archivo\", \"arguments\": {\"ruta\": \"a.rs\",",
    ] {
        let err = parse_assistant_output(&entrada, texto, 1).unwrap_err();
        assert!(matches!(err, ChatError::HistorialInvalido { .. } | ChatError::ArgumentosInvalidos { .. }), "{texto}: {err:?}");
    }
    // Una llave de menos con cadena abierta dentro de un string escapado no confunde.
    let con_escape = "{\"name\": \"leer_archivo\", \"arguments\": {\"ruta\": \"a\\\"}\\\".rs\"}";
    let t = parse_assistant_output(&entrada, con_escape, 1).expect("escapes");
    assert_eq!(t.tool_call.expect("llamada").arguments, "{\"ruta\":\"a\\\"}\\\".rs\"}");
}

#[test]
fn una_marca_de_turno_suelta_al_principio_no_es_contenido() {
    let entrada = entrada_auto();
    let json = "{\"name\": \"leer_archivo\", \"arguments\": {\"ruta\": \"a.rs\"}}";
    for prefijo in ["<|im_start|>\n", "<|im_start|>assistant\n", "  <|im_start|> "] {
        let t = parse_assistant_output(&entrada, &format!("{prefijo}{json}"), 1).expect(prefijo);
        assert!(t.tool_call.is_some(), "{prefijo:?}");
        assert_eq!(t.content, None, "{prefijo:?}");
    }
    // Sólo se quita del principio: en medio del texto es contenido y se conserva.
    let t = parse_assistant_output(&entrada, "hola <|im_start|> adiós", 1).expect("texto");
    assert_eq!(t.content.as_deref(), Some("hola <|im_start|> adiós"));
}
