//! T05 — validación del historial y de los esquemas de herramientas.
//!
//! Los positivos son las formas que mandan los clientes de verdad (los casos
//! Q04/Q05 del banco); los negativos, cada regla de C3 por separado. Lo que se
//! comprueba en todos es que el error **dice qué** está mal, no solo que algo
//! lo está: un 400 sin ruta obliga a adivinar cuál de veinte argumentos falla.

use serde_json::json;
use soso_llm_core::conversation::{
    validate_input, ChatError, ChatInput, Message, Role, ToolCall, ToolChoice, ToolDefinition,
    MAX_ARGUMENTOS_BYTES, MAX_HERRAMIENTAS, MAX_MENSAJES,
};

fn leer_archivo() -> ToolDefinition {
    ToolDefinition::nueva(
        "leer_archivo",
        Some(String::from("Lee las primeras líneas de un archivo")),
        json!({
            "type": "object",
            "properties": {
                "ruta": {"type": "string", "description": "Ruta relativa"},
                "lineas": {"type": "integer", "description": "Cuántas líneas"}
            },
            "required": ["ruta"],
            "additionalProperties": false
        }),
    )
}

/// Un objeto dentro de una lista dentro de un objeto, con enum y anyOf.
fn buscar() -> ToolDefinition {
    ToolDefinition::nueva(
        "buscar",
        None,
        json!({
            "type": "object",
            "properties": {
                "consulta": {"type": "string"},
                "modo": {"type": "string", "enum": ["literal", "regex"]},
                "limite": {"anyOf": [{"type": "integer"}, {"type": "null"}]},
                "filtros": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "campo": {"type": "string"},
                            "valores": {"type": "array", "items": {"type": "string"}}
                        },
                        "required": ["campo"],
                        "additionalProperties": false
                    }
                }
            },
            "required": ["consulta"],
            "additionalProperties": false
        }),
    )
}

fn con_leer(messages: Vec<Message>) -> ChatInput {
    ChatInput::nuevo(messages).con_herramientas(vec![leer_archivo()], ToolChoice::Auto)
}

// ------------------------------------------------------------------ positivos

#[test]
fn la_forma_del_banco_es_valida() {
    // Q04/Q05: user → assistant con la llamada y content null → tool con su id.
    let entrada = con_leer(vec![
        Message::user("Lee el archivo notas.txt"),
        Message::llamadas(vec![ToolCall::nueva(
            "call_9",
            "leer_archivo",
            "{\"ruta\": \"notas.txt\"}",
        )]),
        Message::resultado("call_9", "lista de la compra"),
    ]);
    let antes = entrada.clone();
    validate_input(&entrada).expect("historial válido");
    // Validar no toca nada ni ejecuta la herramienta.
    assert_eq!(entrada, antes);
}

#[test]
fn system_al_principio_y_varios_turnos() {
    let entrada = ChatInput::nuevo(vec![
        Message::system("Eres breve"),
        Message::user("hola"),
        Message::assistant("qué tal"),
        Message::user("¿y ahora?"),
    ]);
    validate_input(&entrada).expect("historial de texto válido");
}

#[test]
fn argumentos_anidados_completos() {
    let args = json!({
        "consulta": "fn main",
        "modo": "literal",
        "limite": 20,
        "filtros": [
            {"campo": "ruta", "valores": ["kernel", "user"]},
            {"campo": "lenguaje"}
        ]
    })
    .to_string();
    let entrada = ChatInput::nuevo(vec![
        Message::user("busca"),
        Message::llamadas(vec![ToolCall::nueva("c1", "buscar", args)]),
        Message::resultado("c1", "3 resultados"),
    ])
    .con_herramientas(vec![buscar()], ToolChoice::Auto);
    validate_input(&entrada).expect("argumentos anidados válidos");
}

#[test]
fn any_of_acepta_las_dos_ramas() {
    for limite in ["20", "null"] {
        let args = format!("{{\"consulta\":\"x\",\"limite\":{limite}}}");
        let entrada = ChatInput::nuevo(vec![
            Message::user("busca"),
            Message::llamadas(vec![ToolCall::nueva("c1", "buscar", args)]),
            Message::resultado("c1", "ok"),
        ])
        .con_herramientas(vec![buscar()], ToolChoice::Auto);
        validate_input(&entrada).unwrap_or_else(|e| panic!("limite={limite}: {e}"));
    }
}

#[test]
fn varias_llamadas_previas_con_su_resultado_cada_una() {
    // El orden de los resultados no tiene por qué ser el de las llamadas.
    let entrada = con_leer(vec![
        Message::user("lee a y b"),
        Message::llamadas(vec![
            ToolCall::nueva("c1", "leer_archivo", "{\"ruta\":\"a\"}"),
            ToolCall::nueva("c2", "leer_archivo", "{\"ruta\":\"b\"}"),
        ]),
        Message::resultado("c2", "contenido de b"),
        Message::resultado("c1", "contenido de a"),
    ]);
    validate_input(&entrada).expect("las dos llamadas quedan resueltas");
}

#[test]
fn las_selecciones_declaradas_valen() {
    for choice in [
        ToolChoice::Auto,
        ToolChoice::None,
        ToolChoice::Required,
        ToolChoice::Named(String::from("leer_archivo")),
    ] {
        let entrada = ChatInput::nuevo(vec![Message::user("haz algo")])
            .con_herramientas(vec![leer_archivo()], choice.clone());
        validate_input(&entrada).unwrap_or_else(|e| panic!("{choice:?}: {e}"));
    }
}

#[test]
fn las_anotaciones_no_estorban() {
    let t = ToolDefinition::nueva(
        "anotada",
        None,
        json!({
            "type": "object",
            "title": "Argumentos",
            "description": "Lo que sea",
            "properties": {"x": {"type": "string", "title": "Equis"}},
            "required": ["x"]
        }),
    );
    let entrada = ChatInput::nuevo(vec![Message::user("hola")])
        .con_herramientas(vec![t], ToolChoice::Auto);
    validate_input(&entrada).expect("description y title no validan nada");
}

// ------------------------------------------------------------------ negativos

#[test]
fn resultado_con_id_que_nadie_pidio() {
    let entrada = con_leer(vec![
        Message::user("lee notas"),
        Message::llamadas(vec![ToolCall::nueva(
            "call_1",
            "leer_archivo",
            "{\"ruta\":\"notas\"}",
        )]),
        Message::resultado("call_7", "contenido"),
    ]);
    assert_eq!(
        validate_input(&entrada),
        Err(ChatError::ResultadoHuerfano {
            tool_call_id: String::from("call_7")
        })
    );
}

#[test]
fn resultado_sin_id_no_se_empareja_a_ojo() {
    // Aunque solo haya una llamada pendiente: inferir el vínculo es inventarlo.
    let entrada = con_leer(vec![
        Message::user("lee notas"),
        Message::llamadas(vec![ToolCall::nueva(
            "call_1",
            "leer_archivo",
            "{\"ruta\":\"notas\"}",
        )]),
        Message {
            role: Role::Tool,
            content: Some(String::from("contenido")),
            tool_calls: Vec::new(),
            tool_call_id: None,
        },
    ]);
    assert_eq!(
        validate_input(&entrada),
        Err(ChatError::ResultadoHuerfano {
            tool_call_id: String::new()
        })
    );
}

#[test]
fn dos_resultados_para_la_misma_llamada() {
    let entrada = con_leer(vec![
        Message::user("lee notas"),
        Message::llamadas(vec![ToolCall::nueva(
            "call_1",
            "leer_archivo",
            "{\"ruta\":\"notas\"}",
        )]),
        Message::resultado("call_1", "contenido"),
        Message::resultado("call_1", "otra vez"),
    ]);
    assert_eq!(
        validate_input(&entrada),
        Err(ChatError::ResultadoDuplicado {
            tool_call_id: String::from("call_1")
        })
    );
}

#[test]
fn dos_llamadas_con_el_mismo_id() {
    let entrada = con_leer(vec![
        Message::user("lee a y b"),
        Message::llamadas(vec![
            ToolCall::nueva("c1", "leer_archivo", "{\"ruta\":\"a\"}"),
            ToolCall::nueva("c1", "leer_archivo", "{\"ruta\":\"b\"}"),
        ]),
    ]);
    assert_eq!(
        validate_input(&entrada),
        Err(ChatError::LlamadaDuplicada {
            llamada: String::from("c1")
        })
    );
}

#[test]
fn no_se_genera_con_una_llamada_pendiente() {
    // Al final del historial…
    let colgada = con_leer(vec![
        Message::user("lee notas"),
        Message::llamadas(vec![ToolCall::nueva(
            "c1",
            "leer_archivo",
            "{\"ruta\":\"notas\"}",
        )]),
    ]);
    assert_eq!(
        validate_input(&colgada),
        Err(ChatError::LlamadaPendiente {
            llamada: String::from("c1")
        })
    );

    // …y también cuando alguien mete otro turno por en medio.
    let interrumpida = con_leer(vec![
        Message::user("lee notas"),
        Message::llamadas(vec![ToolCall::nueva(
            "c1",
            "leer_archivo",
            "{\"ruta\":\"notas\"}",
        )]),
        Message::user("da igual, olvídalo"),
        Message::resultado("c1", "tarde"),
    ]);
    assert_eq!(
        validate_input(&interrumpida),
        Err(ChatError::LlamadaPendiente {
            llamada: String::from("c1")
        })
    );
}

#[test]
fn tipo_erroneo_en_un_argumento_anidado() {
    let args = json!({
        "consulta": "x",
        "filtros": [{"campo": "ruta", "valores": ["a", 7]}]
    })
    .to_string();
    let entrada = ChatInput::nuevo(vec![
        Message::user("busca"),
        Message::llamadas(vec![ToolCall::nueva("c1", "buscar", args)]),
        Message::resultado("c1", "ok"),
    ])
    .con_herramientas(vec![buscar()], ToolChoice::Auto);
    assert_eq!(
        validate_input(&entrada),
        Err(ChatError::ArgumentoInvalido {
            llamada: String::from("c1"),
            ruta: String::from("/filtros/0/valores/1"),
            motivo: String::from("se esperaba una cadena"),
        })
    );
}

#[test]
fn requerido_ausente() {
    let entrada = con_leer(vec![
        Message::user("lee"),
        Message::llamadas(vec![ToolCall::nueva(
            "c1",
            "leer_archivo",
            "{\"lineas\":20}",
        )]),
        Message::resultado("c1", "ok"),
    ]);
    assert_eq!(
        validate_input(&entrada),
        Err(ChatError::ArgumentoInvalido {
            llamada: String::from("c1"),
            ruta: String::from("/ruta"),
            motivo: String::from("argumento requerido ausente"),
        })
    );
}

#[test]
fn propiedad_extra_con_additional_properties_false() {
    let entrada = con_leer(vec![
        Message::user("lee"),
        Message::llamadas(vec![ToolCall::nueva(
            "c1",
            "leer_archivo",
            "{\"ruta\":\"a\",\"recursivo\":true}",
        )]),
        Message::resultado("c1", "ok"),
    ]);
    assert_eq!(
        validate_input(&entrada),
        Err(ChatError::ArgumentoInvalido {
            llamada: String::from("c1"),
            ruta: String::from("/recursivo"),
            motivo: String::from("propiedad no declarada en el esquema"),
        })
    );
}

#[test]
fn valor_fuera_de_enum() {
    let args = json!({"consulta": "x", "modo": "glob"}).to_string();
    let entrada = ChatInput::nuevo(vec![
        Message::user("busca"),
        Message::llamadas(vec![ToolCall::nueva("c1", "buscar", args)]),
        Message::resultado("c1", "ok"),
    ])
    .con_herramientas(vec![buscar()], ToolChoice::Auto);
    assert_eq!(
        validate_input(&entrada),
        Err(ChatError::ArgumentoInvalido {
            llamada: String::from("c1"),
            ruta: String::from("/modo"),
            motivo: String::from("valor fuera de enum"),
        })
    );
}

#[test]
fn esquema_demasiado_profundo() {
    // Nueve niveles de objetos anidados: uno más de los admitidos.
    let mut esquema = json!({"type": "string"});
    for _ in 0..8 {
        esquema = json!({"type": "object", "properties": {"n": esquema}});
    }
    let entrada = ChatInput::nuevo(vec![Message::user("hola")])
        .con_herramientas(
            vec![ToolDefinition::nueva("honda", None, esquema)],
            ToolChoice::Auto,
        );
    match validate_input(&entrada) {
        Err(ChatError::EsquemaNoSoportado {
            herramienta,
            ruta,
            motivo,
        }) => {
            assert_eq!(herramienta, "honda");
            assert!(motivo.contains("niveles"), "{motivo}");
            assert!(ruta.ends_with("/properties/n"), "{ruta}");
        }
        otro => panic!("se esperaba un esquema demasiado profundo: {otro:?}"),
    }
}

#[test]
fn keywords_no_implementadas_se_rechazan() {
    // Ignorar `pattern` aceptaría argumentos que nadie comprobó; `$ref` ni
    // siquiera se puede resolver aquí.
    let casos = [
        (json!({"type": "string", "pattern": "^a"}), "/properties/x/pattern"),
        (json!({"$ref": "#/definitions/x"}), "/properties/x/$ref"),
        (json!({"type": "integer", "minimum": 1}), "/properties/x/minimum"),
        (json!({"oneOf": [{"type": "string"}]}), "/properties/x/oneOf"),
    ];
    for (sub, ruta_esperada) in casos {
        let esquema = json!({
            "type": "object",
            "properties": {"x": sub},
            "required": ["x"]
        });
        let entrada = ChatInput::nuevo(vec![Message::user("hola")]).con_herramientas(
            vec![ToolDefinition::nueva("t", None, esquema.clone())],
            ToolChoice::Auto,
        );
        assert_eq!(
            validate_input(&entrada),
            Err(ChatError::EsquemaNoSoportado {
                herramienta: String::from("t"),
                ruta: String::from(ruta_esperada),
                motivo: String::from("palabra de esquema no implementada"),
            }),
            "{esquema}"
        );
    }
}

#[test]
fn tipos_compuestos_y_items_por_posicion_no_estan_implementados() {
    for (esquema, ruta) in [
        (
            json!({"type": "object", "properties": {"x": {"type": ["string", "null"]}}}),
            "/properties/x/type",
        ),
        (
            json!({"type": "object", "properties": {"x": {"type": "array", "items": [{"type": "string"}]}}}),
            "/properties/x/items",
        ),
        (
            json!({"type": "object", "properties": {"x": {"type": "object", "additionalProperties": {"type": "string"}}}}),
            "/properties/x/additionalProperties",
        ),
    ] {
        let entrada = ChatInput::nuevo(vec![Message::user("hola")]).con_herramientas(
            vec![ToolDefinition::nueva("t", None, esquema.clone())],
            ToolChoice::Auto,
        );
        match validate_input(&entrada) {
            Err(ChatError::EsquemaNoSoportado { ruta: r, .. }) => assert_eq!(r, ruta, "{esquema}"),
            otro => panic!("{esquema}: {otro:?}"),
        }
    }
}

#[test]
fn un_requerido_sin_propiedad_declarada_no_pasa() {
    let esquema = json!({
        "type": "object",
        "properties": {"ruta": {"type": "string"}},
        "required": ["rutas"]
    });
    let entrada = ChatInput::nuevo(vec![Message::user("hola")]).con_herramientas(
        vec![ToolDefinition::nueva("t", None, esquema)],
        ToolChoice::Auto,
    );
    assert_eq!(
        validate_input(&entrada),
        Err(ChatError::EsquemaNoSoportado {
            herramienta: String::from("t"),
            ruta: String::from("/required/0"),
            motivo: String::from("requerido sin propiedad declarada"),
        })
    );
}

#[test]
fn tool_choice_con_un_nombre_que_no_existe() {
    // Q08 del banco.
    let entrada = ChatInput::nuevo(vec![Message::user("Haz lo que sea")]).con_herramientas(
        vec![leer_archivo()],
        ToolChoice::Named(String::from("escribir_archivo")),
    );
    assert_eq!(
        validate_input(&entrada),
        Err(ChatError::HerramientaDesconocida {
            nombre: String::from("escribir_archivo")
        })
    );
}

#[test]
fn required_sin_herramientas_no_se_puede_cumplir() {
    let entrada = ChatInput {
        messages: vec![Message::user("hola")],
        tools: Vec::new(),
        tool_choice: ToolChoice::Required,
    };
    assert_eq!(
        validate_input(&entrada),
        Err(ChatError::SeleccionInvalida {
            motivo: String::from("required sin herramientas declaradas")
        })
    );
}

#[test]
fn una_llamada_a_una_herramienta_no_declarada() {
    let entrada = con_leer(vec![
        Message::user("borra"),
        Message::llamadas(vec![ToolCall::nueva("c1", "borrar_todo", "{}")]),
        Message::resultado("c1", "ok"),
    ]);
    assert_eq!(
        validate_input(&entrada),
        Err(ChatError::HerramientaDesconocida {
            nombre: String::from("borrar_todo")
        })
    );
}

#[test]
fn dos_herramientas_con_el_mismo_nombre() {
    let entrada = ChatInput::nuevo(vec![Message::user("hola")])
        .con_herramientas(vec![leer_archivo(), leer_archivo()], ToolChoice::Auto);
    assert_eq!(
        validate_input(&entrada),
        Err(ChatError::HerramientaDuplicada {
            nombre: String::from("leer_archivo")
        })
    );
}

#[test]
fn argumentos_que_no_son_un_objeto_json() {
    for texto in ["{\"ruta\": \"a\",", "[1,2]", "", "null"] {
        let entrada = con_leer(vec![
            Message::user("lee"),
            Message::llamadas(vec![ToolCall::nueva("c1", "leer_archivo", texto)]),
            Message::resultado("c1", "ok"),
        ]);
        assert_eq!(
            validate_input(&entrada),
            Err(ChatError::ArgumentosInvalidos {
                llamada: String::from("c1")
            }),
            "{texto:?}"
        );
    }
}

#[test]
fn los_limites_de_c3_se_comprueban() {
    let muchos: Vec<Message> = (0..=MAX_MENSAJES).map(|i| Message::user(format!("{i}"))).collect();
    assert_eq!(
        validate_input(&ChatInput::nuevo(muchos)),
        Err(ChatError::LimiteExcedido {
            que: String::from("messages"),
            maximo: MAX_MENSAJES as u32,
            recibido: MAX_MENSAJES as u32 + 1,
        })
    );

    let tools: Vec<ToolDefinition> = (0..=MAX_HERRAMIENTAS)
        .map(|i| ToolDefinition::nueva(format!("t{i}"), None, serde_json::json!({"type": "object"})))
        .collect();
    let entrada = ChatInput::nuevo(vec![Message::user("hola")])
        .con_herramientas(tools, ToolChoice::Auto);
    assert_eq!(
        validate_input(&entrada),
        Err(ChatError::LimiteExcedido {
            que: String::from("tools"),
            maximo: MAX_HERRAMIENTAS as u32,
            recibido: MAX_HERRAMIENTAS as u32 + 1,
        })
    );

    let relleno = "a".repeat(MAX_ARGUMENTOS_BYTES);
    let gordos = format!("{{\"ruta\":\"{relleno}\"}}");
    let largos = con_leer(vec![
        Message::user("lee"),
        Message::llamadas(vec![ToolCall::nueva("c1", "leer_archivo", gordos.clone())]),
        Message::resultado("c1", "ok"),
    ]);
    assert_eq!(
        validate_input(&largos),
        Err(ChatError::LimiteExcedido {
            que: String::from("arguments"),
            maximo: MAX_ARGUMENTOS_BYTES as u32,
            recibido: gordos.len() as u32,
        })
    );
}

#[test]
fn historiales_estructuralmente_rotos() {
    let casos: Vec<(&str, ChatInput)> = vec![
        ("sin mensajes", ChatInput::nuevo(Vec::new())),
        (
            "empieza por assistant",
            ChatInput::nuevo(vec![Message::assistant("hola")]),
        ),
        (
            "system por en medio",
            ChatInput::nuevo(vec![
                Message::user("hola"),
                Message::system("ahora sé breve"),
            ]),
        ),
        (
            "turno vacío",
            ChatInput::nuevo(vec![Message {
                role: Role::User,
                content: None,
                tool_calls: Vec::new(),
                tool_call_id: None,
            }]),
        ),
        (
            "user con tool_calls",
            ChatInput::nuevo(vec![Message {
                role: Role::User,
                content: Some(String::from("hola")),
                tool_calls: vec![ToolCall::nueva("c1", "leer_archivo", "{}")],
                tool_call_id: None,
            }]),
        ),
        (
            "assistant con tool_call_id",
            ChatInput::nuevo(vec![
                Message::user("hola"),
                Message {
                    role: Role::Assistant,
                    content: Some(String::from("ya")),
                    tool_calls: Vec::new(),
                    tool_call_id: Some(String::from("c1")),
                },
            ]),
        ),
    ];
    for (nombre, entrada) in casos {
        match validate_input(&entrada) {
            Err(ChatError::HistorialInvalido { motivo }) => {
                assert!(!motivo.is_empty(), "{nombre}: motivo vacío")
            }
            otro => panic!("{nombre}: se esperaba HistorialInvalido, salió {otro:?}"),
        }
    }
}

#[test]
fn el_error_dice_donde() {
    // Un mensaje sin ruta obliga a adivinar cuál de veinte argumentos falla.
    let e = ChatError::ArgumentoInvalido {
        llamada: String::from("c1"),
        ruta: String::from("/filtros/0/campo"),
        motivo: String::from("se esperaba una cadena"),
    };
    assert_eq!(
        e.to_string(),
        "argumentos de c1 en /filtros/0/campo: se esperaba una cadena"
    );
    let raiz = ChatError::EsquemaNoSoportado {
        herramienta: String::from("t"),
        ruta: String::new(),
        motivo: String::from("parameters no es un objeto"),
    };
    assert_eq!(
        raiz.to_string(),
        "esquema de t en (raíz): parameters no es un objeto"
    );
}

#[test]
fn el_esquema_real_del_fixture_de_t03_pasa() {
    // No vale con esquemas inventados: este es el que T03 sacó de la plantilla
    // oficial de Qwen2.5-Coder, con su `required` y sus descripciones.
    let ruta = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/self-improvement/reference/herramienta-esquema.json"
    );
    let fixture: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(ruta).expect("fixture de T03")).unwrap();
    let f = &fixture["herramientas"][0]["function"];
    let herramienta = ToolDefinition::nueva(
        f["name"].as_str().unwrap(),
        f["description"].as_str().map(String::from),
        f["parameters"].clone(),
    );

    let entrada = ChatInput::nuevo(vec![
        Message::user(fixture["mensajes"][0]["content"].as_str().unwrap()),
        // El fixture de T03 es el render de HF y sus resultados van sin id
        // porque la plantilla no los usa; el dominio sí los exige, así que la
        // llamada se reconstruye con el id que pondría el cliente.
        Message::llamadas(vec![ToolCall::nueva(
            "call_1",
            "leer_archivo",
            "{\"ruta\": \"kernel/src/main.rs\", \"lineas\": 20}",
        )]),
        Message::resultado("call_1", "fn main() { /* … */ }"),
    ])
    .con_herramientas(vec![herramienta], ToolChoice::Auto);
    validate_input(&entrada).expect("el esquema real del fixture entra en el subconjunto");
}

#[test]
fn el_argumento_del_fixture_con_el_tipo_cambiado_falla() {
    let ruta = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/self-improvement/reference/herramienta-esquema.json"
    );
    let fixture: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(ruta).unwrap()).unwrap();
    let f = &fixture["herramientas"][0]["function"];
    let herramienta = ToolDefinition::nueva("leer_archivo", None, f["parameters"].clone());
    let entrada = ChatInput::nuevo(vec![
        Message::user("lee"),
        Message::llamadas(vec![ToolCall::nueva(
            "call_1",
            "leer_archivo",
            "{\"ruta\": \"a.rs\", \"lineas\": \"20\"}",
        )]),
        Message::resultado("call_1", "ok"),
    ])
    .con_herramientas(vec![herramienta], ToolChoice::Auto);
    assert_eq!(
        validate_input(&entrada),
        Err(ChatError::ArgumentoInvalido {
            llamada: String::from("call_1"),
            ruta: String::from("/lineas"),
            motivo: String::from("se esperaba un entero"),
        })
    );
}
