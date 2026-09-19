//! Validación de una conversación **antes** de generar (ficha T05, contrato C3).
//!
//! Esto no ejecuta herramientas, no toca la red y no modifica el historial:
//! recibe un `&ChatInput` y devuelve `Ok(())` o el `ChatError` concreto que
//! explica qué entrada es inválida. Que reciba una referencia compartida es
//! parte del contrato, no una casualidad de firma.
//!
//! Dos ideas sostienen el módulo:
//!
//! - **Una llamada pendiente bloquea la generación.** Si el asistente pidió
//!   `call_1` y nadie mandó su resultado, el historial está a medias: pedirle
//!   al modelo que siga es pedirle que se invente lo que devolvió la
//!   herramienta. Se rechaza en vez de continuar.
//! - **Un esquema que no se entiende se rechaza, no se ignora.** Aceptar un
//!   `pattern` sin comprobarlo es peor que devolver 422: el modelo emitiría
//!   argumentos que nadie validó y que el cliente cree validados. Solo
//!   `description` y `title` se saltan, porque son anotaciones y no reglas.
//!
//! El subconjunto de JSON Schema admitido es deliberadamente pequeño:
//! `type` (escalares, `object`, `array`), `properties`, `required`,
//! `additionalProperties` booleano, `items`, `enum` y `anyOf`. Cualquier otra
//! palabra —incluido `$ref`— es [`ChatError::EsquemaNoSoportado`]. Ampliarlo
//! exige un fixture del esquema real, no bajar la guardia.
//!
//! Las rutas de los errores son **JSON Pointer** (RFC 6901) sobre el documento
//! del que se habla: el esquema en los errores de esquema, el objeto de
//! argumentos en los de argumentos. La raíz es la cadena vacía.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use serde_json::Value;

use super::{ChatError, ChatInput, Role, ToolCall, ToolChoice, ToolDefinition};

/// Máximo de turnos en el historial (C3).
pub const MAX_MENSAJES: usize = 64;
/// Máximo de herramientas declaradas (C3).
pub const MAX_HERRAMIENTAS: usize = 32;
/// Máximo de bytes por argumentos de una llamada (C3).
pub const MAX_ARGUMENTOS_BYTES: usize = 64 * 1024;
/// Niveles de anidamiento admitidos en un esquema; la raíz es el nivel 1.
pub const MAX_PROFUNDIDAD_ESQUEMA: usize = 8;

/// Palabras que cambian la validación y que sí se implementan.
const REGLAS: &[&str] = &[
    "type",
    "properties",
    "required",
    "additionalProperties",
    "items",
    "enum",
    "anyOf",
];

/// Anotaciones: se transportan, no validan nada, y saltárselas es correcto.
const ANOTACIONES: &[&str] = &["description", "title"];

const TIPOS: &[&str] = &[
    "object", "array", "string", "number", "integer", "boolean", "null",
];

/// Valida una llamada ya parseada de la salida del modelo (T07).
///
/// Comprueba id, límites, nombre declarado y argumentos contra el esquema.
pub fn validate_parsed_tool_call(entrada: &ChatInput, llamada: &ToolCall) -> Result<(), ChatError> {
    validar_llamada(entrada, llamada, &[])
}

/// Comprueba `tool_choice` frente al turno parseado (texto y/o llamada).
pub fn validate_assistant_turn(
    entrada: &ChatInput,
    content: Option<&str>,
    llamada: Option<&ToolCall>,
) -> Result<(), ChatError> {
    validar_seleccion_salida(entrada, llamada)?;
    if let Some(c) = llamada {
        validate_parsed_tool_call(entrada, c)?;
    }
    let _ = content;
    Ok(())
}

fn validar_seleccion_salida(
    entrada: &ChatInput,
    llamada: Option<&ToolCall>,
) -> Result<(), ChatError> {
    match (&entrada.tool_choice, llamada) {
        (ToolChoice::None, Some(_)) => Err(ChatError::SeleccionInvalida {
            motivo: String::from("tool_choice none pero el modelo emitió una llamada"),
        }),
        (ToolChoice::Required, None) => Err(ChatError::SeleccionInvalida {
            motivo: String::from("tool_choice required pero no hay llamada"),
        }),
        (ToolChoice::Named(nombre), None) => Err(ChatError::SeleccionInvalida {
            motivo: format!("tool_choice named `{nombre}` pero no hay llamada"),
        }),
        (ToolChoice::Named(nombre), Some(c)) if c.name != *nombre => {
            Err(ChatError::SeleccionInvalida {
                motivo: format!(
                    "tool_choice named `{nombre}` pero la llamada es `{}`",
                    c.name
                ),
            })
        }
        _ => Ok(()),
    }
}

/// Comprueba una conversación entera contra C3.
///
/// No muta la entrada ni ejecuta nada. El orden de comprobación va de lo barato
/// a lo caro: límites, herramientas, selección e historial.
pub fn validate_input(entrada: &ChatInput) -> Result<(), ChatError> {
    limite("messages", entrada.messages.len(), MAX_MENSAJES)?;
    limite("tools", entrada.tools.len(), MAX_HERRAMIENTAS)?;
    validar_herramientas(&entrada.tools)?;
    validar_seleccion(entrada)?;
    validar_historial(entrada)
}

fn limite(que: &str, recibido: usize, maximo: usize) -> Result<(), ChatError> {
    if recibido > maximo {
        return Err(ChatError::LimiteExcedido {
            que: String::from(que),
            maximo: maximo as u32,
            recibido: recibido as u32,
        });
    }
    Ok(())
}

fn historial(motivo: impl Into<String>) -> ChatError {
    ChatError::HistorialInvalido {
        motivo: motivo.into(),
    }
}

// ---------------------------------------------------------------- herramientas

fn validar_herramientas(tools: &[ToolDefinition]) -> Result<(), ChatError> {
    for (i, t) in tools.iter().enumerate() {
        if t.name.is_empty() {
            return Err(ChatError::HistorialInvalido {
                motivo: format!("herramienta {i} sin nombre"),
            });
        }
        // Nombres únicos: con dos «leer_archivo» distintos, validar unos
        // argumentos es echar a suertes contra qué esquema se comparan.
        if tools[..i].iter().any(|p| p.name == t.name) {
            return Err(ChatError::HerramientaDuplicada {
                nombre: t.name.clone(),
            });
        }
        validar_esquema_raiz(&t.name, &t.parameters)?;
    }
    Ok(())
}

fn validar_seleccion(entrada: &ChatInput) -> Result<(), ChatError> {
    match &entrada.tool_choice {
        ToolChoice::Auto | ToolChoice::None => Ok(()),
        ToolChoice::Required => {
            if entrada.tools.is_empty() {
                Err(ChatError::SeleccionInvalida {
                    motivo: String::from("required sin herramientas declaradas"),
                })
            } else {
                Ok(())
            }
        }
        ToolChoice::Named(nombre) => {
            if entrada.herramienta(nombre).is_none() {
                Err(ChatError::HerramientaDesconocida {
                    nombre: nombre.clone(),
                })
            } else {
                Ok(())
            }
        }
    }
}

// ------------------------------------------------------------------ historial

fn validar_historial(entrada: &ChatInput) -> Result<(), ChatError> {
    let mensajes = &entrada.messages;
    match mensajes.first().map(|m| m.role) {
        None => return Err(historial("sin mensajes")),
        Some(Role::System) | Some(Role::User) => {}
        Some(_) => return Err(historial("la conversación no empieza por system ni user")),
    }

    // Ids de llamada vistos alguna vez (para duplicados) y los que siguen sin
    // resultado (para impedir generar a medias).
    let mut vistos: Vec<&str> = Vec::new();
    let mut resueltos: Vec<&str> = Vec::new();
    let mut pendientes: Vec<&str> = Vec::new();
    let mut fuera_del_encabezado = false;

    for (i, m) in mensajes.iter().enumerate() {
        if m.vacio() {
            return Err(historial(format!(
                "mensaje {i}: sin texto y sin llamadas, no aporta nada"
            )));
        }

        // Una llamada pendiente solo la cierra un resultado. Cualquier otro
        // turno por medio deja el historial incoherente.
        if !pendientes.is_empty() && m.role != Role::Tool {
            return Err(ChatError::LlamadaPendiente {
                llamada: pendientes[0].to_string(),
            });
        }

        match m.role {
            Role::System => {
                if fuera_del_encabezado {
                    return Err(historial(format!(
                        "mensaje {i}: system fuera del encabezado de la conversación"
                    )));
                }
                comprobar_campos_de_texto(i, m)?;
            }
            Role::User => {
                fuera_del_encabezado = true;
                comprobar_campos_de_texto(i, m)?;
            }
            Role::Assistant => {
                fuera_del_encabezado = true;
                if m.tool_call_id.is_some() {
                    return Err(historial(format!(
                        "mensaje {i}: assistant con tool_call_id"
                    )));
                }
                for llamada in &m.tool_calls {
                    validar_llamada(entrada, llamada, &vistos)?;
                    vistos.push(&llamada.id);
                    pendientes.push(&llamada.id);
                }
            }
            Role::Tool => {
                fuera_del_encabezado = true;
                if !m.tool_calls.is_empty() {
                    return Err(historial(format!("mensaje {i}: tool con tool_calls")));
                }
                if m.content.is_none() {
                    return Err(historial(format!("mensaje {i}: tool sin contenido")));
                }
                let id = match m.tool_call_id.as_deref() {
                    Some(id) if !id.is_empty() => id,
                    // Emparejarlo «por el único que quedaba» sería inventar el
                    // vínculo; el resultado se queda sin llamada que lo pidiera.
                    _ => {
                        return Err(ChatError::ResultadoHuerfano {
                            tool_call_id: String::new(),
                        })
                    }
                };
                if resueltos.contains(&id) {
                    return Err(ChatError::ResultadoDuplicado {
                        tool_call_id: id.to_string(),
                    });
                }
                match pendientes.iter().position(|p| *p == id) {
                    Some(pos) => {
                        pendientes.remove(pos);
                        resueltos.push(id);
                    }
                    None => {
                        return Err(ChatError::ResultadoHuerfano {
                            tool_call_id: id.to_string(),
                        })
                    }
                }
            }
        }
    }

    // Generar con una llamada sin resultado es pedirle al modelo que se invente
    // lo que devolvió la herramienta.
    if let Some(p) = pendientes.first() {
        return Err(ChatError::LlamadaPendiente {
            llamada: p.to_string(),
        });
    }
    Ok(())
}

fn comprobar_campos_de_texto(i: usize, m: &super::Message) -> Result<(), ChatError> {
    if !m.tool_calls.is_empty() {
        return Err(historial(format!(
            "mensaje {i}: solo assistant puede llevar tool_calls"
        )));
    }
    if m.tool_call_id.is_some() {
        return Err(historial(format!(
            "mensaje {i}: solo tool puede llevar tool_call_id"
        )));
    }
    if m.content.is_none() {
        return Err(historial(format!("mensaje {i}: turno de texto sin texto")));
    }
    Ok(())
}

fn validar_llamada(
    entrada: &ChatInput,
    llamada: &ToolCall,
    vistos: &[&str],
) -> Result<(), ChatError> {
    if llamada.id.is_empty() {
        return Err(historial("llamada sin id"));
    }
    if vistos.contains(&llamada.id.as_str()) {
        return Err(ChatError::LlamadaDuplicada {
            llamada: llamada.id.clone(),
        });
    }
    limite(
        "arguments",
        llamada.arguments.len(),
        MAX_ARGUMENTOS_BYTES,
    )?;
    let Some(herramienta) = entrada.herramienta(&llamada.name) else {
        // Sin la declaración no hay esquema contra el que comparar: no se puede
        // decir que estos argumentos sean válidos.
        return Err(ChatError::HerramientaDesconocida {
            nombre: llamada.name.clone(),
        });
    };
    let argumentos = llamada.argumentos()?;
    validar_valor(&llamada.id, &argumentos, &herramienta.parameters, "")
}

// --------------------------------------------------------------------- esquema

fn esquema_no_soportado(
    herramienta: &str,
    ruta: &str,
    motivo: impl Into<String>,
) -> ChatError {
    ChatError::EsquemaNoSoportado {
        herramienta: String::from(herramienta),
        ruta: String::from(ruta),
        motivo: motivo.into(),
    }
}

/// El esquema de `parameters` describe siempre un objeto de argumentos.
fn validar_esquema_raiz(herramienta: &str, esquema: &Value) -> Result<(), ChatError> {
    let Some(obj) = esquema.as_object() else {
        return Err(esquema_no_soportado(
            herramienta,
            "",
            "parameters no es un objeto",
        ));
    };
    if obj.get("type").and_then(Value::as_str) != Some("object") {
        return Err(esquema_no_soportado(
            herramienta,
            "",
            "parameters debe declarar type: object",
        ));
    }
    validar_esquema(herramienta, esquema, "", 1)
}

fn validar_esquema(
    herramienta: &str,
    esquema: &Value,
    ruta: &str,
    profundidad: usize,
) -> Result<(), ChatError> {
    if profundidad > MAX_PROFUNDIDAD_ESQUEMA {
        return Err(esquema_no_soportado(
            herramienta,
            ruta,
            format!("más de {MAX_PROFUNDIDAD_ESQUEMA} niveles de anidamiento"),
        ));
    }
    let Some(obj) = esquema.as_object() else {
        return Err(esquema_no_soportado(
            herramienta,
            ruta,
            "un subesquema tiene que ser un objeto",
        ));
    };

    for clave in obj.keys() {
        if ANOTACIONES.contains(&clave.as_str()) {
            continue;
        }
        if !REGLAS.contains(&clave.as_str()) {
            // `$ref`, `pattern`, `minimum`, `oneOf`… Ignorarlas aceptaría
            // argumentos que nadie comprobó.
            return Err(esquema_no_soportado(
                herramienta,
                &format!("{ruta}/{}", puntero(clave)),
                "palabra de esquema no implementada",
            ));
        }
    }

    if let Some(t) = obj.get("type") {
        match t.as_str() {
            Some(nombre) if TIPOS.contains(&nombre) => {}
            Some(_) => {
                return Err(esquema_no_soportado(
                    herramienta,
                    &format!("{ruta}/type"),
                    "tipo desconocido",
                ))
            }
            // `type: ["string","null"]` se escribe con anyOf en este subconjunto.
            None => {
                return Err(esquema_no_soportado(
                    herramienta,
                    &format!("{ruta}/type"),
                    "type tiene que ser una cadena",
                ))
            }
        }
    }

    if let Some(props) = obj.get("properties") {
        let Some(mapa) = props.as_object() else {
            return Err(esquema_no_soportado(
                herramienta,
                &format!("{ruta}/properties"),
                "properties tiene que ser un objeto",
            ));
        };
        for (nombre, sub) in mapa {
            validar_esquema(
                herramienta,
                sub,
                &format!("{ruta}/properties/{}", puntero(nombre)),
                profundidad + 1,
            )?;
        }
    }

    if let Some(req) = obj.get("required") {
        let Some(lista) = req.as_array() else {
            return Err(esquema_no_soportado(
                herramienta,
                &format!("{ruta}/required"),
                "required tiene que ser una lista",
            ));
        };
        for (i, entrada) in lista.iter().enumerate() {
            let Some(nombre) = entrada.as_str() else {
                return Err(esquema_no_soportado(
                    herramienta,
                    &format!("{ruta}/required/{i}"),
                    "los requeridos son nombres de propiedad",
                ));
            };
            // Un requerido que no está en properties solo se puede comprobar
            // por presencia; casi siempre es una errata.
            let declarado = obj
                .get("properties")
                .and_then(Value::as_object)
                .is_some_and(|m| m.contains_key(nombre));
            if !declarado {
                return Err(esquema_no_soportado(
                    herramienta,
                    &format!("{ruta}/required/{i}"),
                    "requerido sin propiedad declarada",
                ));
            }
        }
    }

    if let Some(extra) = obj.get("additionalProperties") {
        if !extra.is_boolean() {
            return Err(esquema_no_soportado(
                herramienta,
                &format!("{ruta}/additionalProperties"),
                "solo se admite true o false",
            ));
        }
    }

    if let Some(items) = obj.get("items") {
        if items.is_array() {
            return Err(esquema_no_soportado(
                herramienta,
                &format!("{ruta}/items"),
                "items por posición no está implementado",
            ));
        }
        validar_esquema(
            herramienta,
            items,
            &format!("{ruta}/items"),
            profundidad + 1,
        )?;
    }

    if let Some(e) = obj.get("enum") {
        match e.as_array() {
            Some(v) if !v.is_empty() => {}
            _ => {
                return Err(esquema_no_soportado(
                    herramienta,
                    &format!("{ruta}/enum"),
                    "enum tiene que ser una lista no vacía",
                ))
            }
        }
    }

    if let Some(a) = obj.get("anyOf") {
        let alternativas = match a.as_array() {
            Some(v) if !v.is_empty() => v,
            _ => {
                return Err(esquema_no_soportado(
                    herramienta,
                    &format!("{ruta}/anyOf"),
                    "anyOf tiene que ser una lista no vacía",
                ))
            }
        };
        for (i, sub) in alternativas.iter().enumerate() {
            validar_esquema(
                herramienta,
                sub,
                &format!("{ruta}/anyOf/{i}"),
                profundidad + 1,
            )?;
        }
    }

    Ok(())
}

// ------------------------------------------------------------------ argumentos

fn argumento_invalido(
    llamada: &str,
    ruta: &str,
    motivo: impl Into<String>,
) -> ChatError {
    ChatError::ArgumentoInvalido {
        llamada: String::from(llamada),
        ruta: String::from(ruta),
        motivo: motivo.into(),
    }
}

/// Comprueba un valor contra un esquema ya admitido por [`validar_esquema`].
///
/// `ruta` es el JSON Pointer del valor dentro del objeto de argumentos, para
/// que el error diga exactamente qué argumento falla y no solo que «falla».
fn validar_valor(
    llamada: &str,
    valor: &Value,
    esquema: &Value,
    ruta: &str,
) -> Result<(), ChatError> {
    let obj = match esquema.as_object() {
        Some(o) => o,
        None => return Ok(()),
    };

    if let Some(alternativas) = obj.get("enum").and_then(Value::as_array) {
        if !alternativas.iter().any(|a| a == valor) {
            return Err(argumento_invalido(llamada, ruta, "valor fuera de enum"));
        }
    }

    if let Some(alternativas) = obj.get("anyOf").and_then(Value::as_array) {
        if !alternativas
            .iter()
            .any(|sub| validar_valor(llamada, valor, sub, ruta).is_ok())
        {
            return Err(argumento_invalido(
                llamada,
                ruta,
                "no encaja en ninguna alternativa de anyOf",
            ));
        }
    }

    let Some(tipo) = obj.get("type").and_then(Value::as_str) else {
        // Sin `type` (y ya comprobados enum/anyOf) el esquema no restringe más.
        return Ok(());
    };

    match tipo {
        "object" => {
            let Some(mapa) = valor.as_object() else {
                return Err(argumento_invalido(llamada, ruta, "se esperaba un objeto"));
            };
            let props = obj.get("properties").and_then(Value::as_object);
            if let Some(req) = obj.get("required").and_then(Value::as_array) {
                for nombre in req.iter().filter_map(Value::as_str) {
                    if !mapa.contains_key(nombre) {
                        return Err(argumento_invalido(
                            llamada,
                            &format!("{ruta}/{}", puntero(nombre)),
                            "argumento requerido ausente",
                        ));
                    }
                }
            }
            let admite_extras = obj
                .get("additionalProperties")
                .and_then(Value::as_bool)
                .unwrap_or(true);
            for (nombre, sub) in mapa {
                let hijo = format!("{ruta}/{}", puntero(nombre));
                match props.and_then(|p| p.get(nombre)) {
                    Some(esq) => validar_valor(llamada, sub, esq, &hijo)?,
                    None if admite_extras => {}
                    None => {
                        return Err(argumento_invalido(
                            llamada,
                            &hijo,
                            "propiedad no declarada en el esquema",
                        ))
                    }
                }
            }
        }
        "array" => {
            let Some(lista) = valor.as_array() else {
                return Err(argumento_invalido(llamada, ruta, "se esperaba una lista"));
            };
            if let Some(items) = obj.get("items") {
                for (i, v) in lista.iter().enumerate() {
                    validar_valor(llamada, v, items, &format!("{ruta}/{i}"))?;
                }
            }
        }
        "string" => {
            if !valor.is_string() {
                return Err(argumento_invalido(llamada, ruta, "se esperaba una cadena"));
            }
        }
        "integer" => {
            if !es_entero(valor) {
                return Err(argumento_invalido(llamada, ruta, "se esperaba un entero"));
            }
        }
        "number" => {
            if !valor.is_number() {
                return Err(argumento_invalido(llamada, ruta, "se esperaba un número"));
            }
        }
        "boolean" => {
            if !valor.is_boolean() {
                return Err(argumento_invalido(llamada, ruta, "se esperaba un booleano"));
            }
        }
        "null" => {
            if !valor.is_null() {
                return Err(argumento_invalido(llamada, ruta, "se esperaba null"));
            }
        }
        _ => {}
    }
    Ok(())
}

/// `20.0` es un entero en JSON Schema aunque se escriba con coma flotante.
fn es_entero(valor: &Value) -> bool {
    if valor.is_i64() || valor.is_u64() {
        return true;
    }
    match valor.as_f64() {
        Some(f) => f == libm::trunc(f) && f.is_finite(),
        None => false,
    }
}

/// Escapa un nombre de propiedad como componente de JSON Pointer (RFC 6901).
fn puntero(nombre: &str) -> String {
    let mut s = String::with_capacity(nombre.len());
    for c in nombre.chars() {
        match c {
            '~' => s.push_str("~0"),
            '/' => s.push_str("~1"),
            _ => s.push(c),
        }
    }
    s
}
