//! Conservación de información en los tipos de conversación (ficha T04).
//!
//! Lo que se comprueba aquí no es que los tipos compilen, sino que una
//! conversación **sobreviva** a la ida y vuelta sin perder por el camino lo que
//! la distingue: el turno que no lleva texto, el que lleva texto vacío, los
//! acentos, y qué resultado responde a qué llamada.
//!
//! Esa serialización es para fixtures y persistencia. **No es Chat
//! Completions**: el formato del proveedor lo construye `soso-llm-api` (T10), y
//! confundirlos es la forma más fácil de acabar con dos dominios.

use soso_llm_core::conversation::{
    ChatInput, Message, ModelProfile, Role, ToolCall, ToolChoice, ToolDefinition,
};

fn ida_y_vuelta(m: &Message) -> Message {
    let json = serde_json::to_string(m).expect("serializa");
    serde_json::from_str(&json).unwrap_or_else(|e| panic!("no vuelve: {e} en {json}"))
}

#[test]
fn cada_rol_va_y_vuelve_igual() {
    let casos = vec![
        Message::system("responde en una palabra"),
        Message::user("¿estás listo?"),
        Message::assistant("LISTO"),
        Message::llamadas(vec![ToolCall::nueva(
            "call_1",
            "obtener_hora",
            "{\"zona\":\"local\"}",
        )]),
        Message::resultado("call_1", "13:45"),
    ];
    for m in &casos {
        assert_eq!(*m, ida_y_vuelta(m));
    }
    // Y el rol se escribe como lo espera el resto del mundo.
    assert_eq!(Role::Assistant.como_texto(), "assistant");
    assert_eq!(
        serde_json::to_string(&Role::Tool).unwrap(),
        "\"tool\"",
    );
}

/// Un asistente que solo llama a una herramienta no tiene texto. Eso **no** es
/// lo mismo que tener texto vacío, y el historial tiene que poder distinguirlo:
/// es el caso `content: null` que mandan los clientes de verdad.
#[test]
fn texto_ausente_sobrevive_como_ausente() {
    let sin_texto = Message::llamadas(vec![ToolCall::nueva("c1", "f", "{}")]);
    let vacio = Message::assistant("");

    assert_eq!(ida_y_vuelta(&sin_texto).content, None);
    assert_eq!(ida_y_vuelta(&vacio).content, Some(String::new()));
    assert_ne!(ida_y_vuelta(&sin_texto), ida_y_vuelta(&vacio));

    // Y un `content: null` explícito se lee como ausencia, no como error.
    let crudo = r#"{"role":"assistant","content":null,"tool_calls":[]}"#;
    let leido: Message = serde_json::from_str(crudo).expect("null es válido");
    assert_eq!(leido.content, None);
    assert!(leido.vacio());
}

#[test]
fn el_texto_no_ascii_llega_intacto() {
    let m = Message::user("café ☕ para el señor Ñoño — 3 × 4");
    assert_eq!(
        ida_y_vuelta(&m).content.as_deref(),
        Some("café ☕ para el señor Ñoño — 3 × 4")
    );

    // También dentro de los argumentos, que viajan como texto.
    let llamada = ToolCall::nueva("c1", "escribir", "{\"texto\":\"café ☕\"}");
    let turno = Message::llamadas(vec![llamada]);
    let vuelta = ida_y_vuelta(&turno);
    assert_eq!(vuelta.tool_calls[0].argumentos().unwrap()["texto"], "café ☕");
}

/// El primer perfil emite una llamada por turno, pero el **historial** puede
/// traer turnos anteriores con varias, y cada resultado tiene que seguir
/// atado a la suya.
#[test]
fn varias_llamadas_conservan_a_quien_responde_cada_resultado() {
    let entrada = ChatInput::nuevo(vec![
        Message::user("lee a.rs y b.rs"),
        Message::llamadas(vec![
            ToolCall::nueva("c1", "leer", "{\"ruta\":\"a.rs\"}"),
            ToolCall::nueva("c2", "leer", "{\"ruta\":\"b.rs\"}"),
        ]),
        Message::resultado("c2", "contenido de b"),
        Message::resultado("c1", "contenido de a"),
    ]);

    let json = serde_json::to_string(&entrada).unwrap();
    let vuelta: ChatInput = serde_json::from_str(&json).unwrap();
    assert_eq!(entrada, vuelta);

    let llamadas = &vuelta.messages[1].tool_calls;
    assert_eq!(llamadas.len(), 2);
    assert_eq!(llamadas[0].id, "c1");
    // El orden de los resultados no tiene por qué seguir al de las llamadas.
    assert_eq!(vuelta.messages[2].tool_call_id.as_deref(), Some("c2"));
    assert_eq!(vuelta.messages[3].tool_call_id.as_deref(), Some("c1"));
}

#[test]
fn las_herramientas_y_su_seleccion_van_y_vuelven() {
    let esquema = serde_json::json!({
        "type": "object",
        "properties": {
            "ruta": {"type": "string", "description": "Ruta relativa"},
            "lineas": {"type": "integer"}
        },
        "required": ["ruta", "lineas"]
    });
    let entrada = ChatInput::nuevo(vec![Message::user("enséñame kernel/src/main.rs")])
        .con_herramientas(
            vec![ToolDefinition::nueva(
                "leer_archivo",
                Some(String::from("Lee las primeras líneas")),
                esquema.clone(),
            )],
            ToolChoice::Named(String::from("leer_archivo")),
        );

    let vuelta: ChatInput =
        serde_json::from_str(&serde_json::to_string(&entrada).unwrap()).unwrap();
    assert_eq!(vuelta, entrada);
    // El esquema viaja entero: es JSON Schema, no se interpreta aquí.
    assert_eq!(vuelta.tools[0].parameters, esquema);
    assert_eq!(
        vuelta.tool_choice,
        ToolChoice::Named(String::from("leer_archivo"))
    );
    assert!(vuelta.herramienta("leer_archivo").is_some());
    assert!(vuelta.herramienta("no_existe").is_none());
}

/// Una conversación sin herramientas no escribe campos vacíos: el JSON de
/// persistencia no debería engordar por lo que no hay.
#[test]
fn lo_que_no_hay_no_se_escribe() {
    let entrada = ChatInput::nuevo(vec![Message::user("hola")]);
    let json = serde_json::to_string(&entrada).unwrap();
    assert!(!json.contains("tools"), "{json}");
    assert!(!json.contains("tool_calls"), "{json}");
    assert!(!json.contains("tool_call_id"), "{json}");
    // Y vuelve igual, con los valores por defecto.
    let vuelta: ChatInput = serde_json::from_str(&json).unwrap();
    assert_eq!(vuelta.tool_choice, ToolChoice::Auto);
    assert!(vuelta.tools.is_empty());
}

#[test]
fn el_perfil_del_modelo_va_y_vuelve() {
    let p = ModelProfile {
        id: String::from("soso-coder"),
        directory: String::from("target/qwen2.5-coder-3b-model"),
        family: String::from("qwen2"),
        weights_sha256: String::from("a".repeat(64)),
        tokenizer_sha256: String::from("b".repeat(64)),
        template_sha256: String::from("c".repeat(64)),
        context_tokens: 32768,
        max_output_tokens: 512,
        stop_token_ids: vec![151645, 151643],
    };
    let vuelta: ModelProfile = serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
    assert_eq!(p, vuelta);
    // El id es estable y no es el directorio: un directorio se renombra.
    assert_ne!(vuelta.id, vuelta.directory);
    assert!(vuelta.es_parada(151643));
}
