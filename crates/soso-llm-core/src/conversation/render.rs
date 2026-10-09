//! Plantilla de chat por familia (ficha T06, contrato C2).
//!
//! Traduce un [`ChatInput`] validado al texto que el tokenizer debe segmentar.
//! La referencia son los fixtures de T03; aquí no se inventa formato.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use serde_json::Value;

use crate::conversation::{
    validate_input_pasante, ChatError, ChatInput, Message, ModelProfile, Role, ToolDefinition,
};
use crate::tokenizer::Tokenizer;

const IM_START: &str = "<|im_start|>";
const IM_END: &str = "<|im_end|>";
const SYSTEM_POR_DEFECTO: &str =
    "You are Qwen, created by Alibaba Cloud. You are a helpful assistant.";

const TOOLS_CABECERA: &str = "\n\n# Tools\n\nYou may call one or more functions to assist with the user query.\n\nYou are provided with function signatures within <tools></tools> XML tags:\n<tools>";

const TOOLS_PIE: &str = "\n</tools>\n\nFor each function call, return a json object with function name and arguments within <tool_call></tool_call> XML tags:\n<tool_call>\n{\"name\": <function-name>, \"arguments\": <args-json-object>}\n</tool_call>";

/// Render completo + tokenización para el perfil indicado.
pub fn render_messages(
    input: &ChatInput,
    profile: &ModelProfile,
    tokenizer: &Tokenizer,
) -> Result<Vec<u32>, ChatError> {
    // Pasante: el historial puede traer llamadas que el propio modelo escribió mal y que el
    // servidor devolvió tal cual (T22); pintar el prompt sólo exige estructura coherente.
    validate_input_pasante(input)?;
    if !tokenizer.tiene_vocabulario() {
        return Err(ChatError::TokenizacionInvalida {
            motivo: String::from("el tokenizer no trae vocabulario"),
        });
    }
    let texto = match profile.family.as_str() {
        "qwen2" => render_qwen2(input, true)?,
        "qwen3coder" => render_qwen3coder(input, true)?,
        otro => {
            return Err(ChatError::FamiliaNoImplementada {
                family: String::from(otro),
            });
        }
    };
    Ok(tokenizer.encode(&texto))
}

/// Texto ChatML Qwen2 alineado con la plantilla oficial de Hugging Face.
pub fn render_qwen2(input: &ChatInput, add_generation_prompt: bool) -> Result<String, ChatError> {
    let mut out = String::new();
    let messages = &input.messages;
    let tools = &input.tools;

    if !tools.is_empty() {
        out.push_str(IM_START);
        out.push_str("system\n");
        if messages.first().is_some_and(|m| m.role == Role::System) {
            out.push_str(contenido(&messages[0]));
        } else {
            out.push_str(SYSTEM_POR_DEFECTO);
        }
        out.push_str(TOOLS_CABECERA);
        for tool in tools {
            out.push('\n');
            out.push_str(&herramienta_en_cable(tool)?);
        }
        out.push_str(TOOLS_PIE);
        out.push_str(IM_END);
        out.push('\n');
    } else if messages.first().is_some_and(|m| m.role == Role::System) {
        turno_simple(&mut out, "system", contenido(&messages[0]));
    } else {
        turno_simple(&mut out, "system", SYSTEM_POR_DEFECTO);
    }

    for (i, message) in messages.iter().enumerate() {
        let primero = i == 0;
        match message.role {
            Role::User => turno_simple(&mut out, "user", contenido(message)),
            // El primer system ya fue cabecera (con o sin herramientas).
            Role::System if primero => {}
            Role::System => turno_simple(&mut out, "system", contenido(message)),
            Role::Assistant if message.tool_calls.is_empty() => {
                turno_simple(&mut out, "assistant", contenido(message));
            }
            Role::Assistant => turno_asistente_con_llamadas(&mut out, message)?,
            Role::Tool => {
                let prev_tool = i > 0 && messages[i - 1].role == Role::Tool;
                let next_tool = messages
                    .get(i + 1)
                    .is_some_and(|m| m.role == Role::Tool);
                if !prev_tool {
                    out.push_str(IM_START);
                    out.push_str("user");
                }
                out.push_str("\n<tool_response>\n");
                out.push_str(contenido(message));
                out.push_str("\n</tool_response>");
                if !next_tool {
                    out.push_str(IM_END);
                    out.push('\n');
                }
            }
        }
    }

    if add_generation_prompt {
        out.push_str(IM_START);
        out.push_str("assistant\n");
    }
    Ok(out)
}

fn contenido(m: &Message) -> &str {
    m.content.as_deref().unwrap_or("")
}

fn turno_simple(out: &mut String, rol: &str, cuerpo: &str) {
    out.push_str(IM_START);
    out.push_str(rol);
    out.push('\n');
    out.push_str(cuerpo);
    out.push_str(IM_END);
    out.push('\n');
}

fn turno_asistente_con_llamadas(out: &mut String, message: &Message) -> Result<(), ChatError> {
    out.push_str(IM_START);
    out.push_str("assistant");
    if let Some(texto) = message.content.as_ref() {
        if !texto.is_empty() {
            out.push('\n');
            out.push_str(texto);
        }
    }
    for call in &message.tool_calls {
        let args: Value = serde_json::from_str(&call.arguments).map_err(|_| {
            ChatError::ArgumentosInvalidos {
                llamada: call.id.clone(),
            }
        })?;
        out.push_str("\n<tool_call>\n");
        out.push_str(&format!(
            r#"{{"name": "{}", "arguments": {}}}"#,
            call.name,
            tojson_compact(&args)
        ));
        out.push_str("\n</tool_call>");
    }
    out.push_str(IM_END);
    out.push('\n');
    Ok(())
}

/// Mismo objeto que espera la plantilla HF (`type` + `function`).
fn herramienta_en_cable(tool: &ToolDefinition) -> Result<String, ChatError> {
    let mut function = serde_json::Map::new();
    if let Some(desc) = &tool.description {
        function.insert(
            String::from("description"),
            Value::String(desc.clone()),
        );
    }
    function.insert(String::from("name"), Value::String(tool.name.clone()));
    function.insert(
        String::from("parameters"),
        tool.parameters.clone(),
    );
    let wire = serde_json::json!({
        "function": Value::Object(function),
        "type": "function",
    });
    Ok(tojson_compact(&wire))
}

/// Equivalente al filtro Jinja `tojson` (compacto, sin espacios extra).
fn tojson_compact(value: &Value) -> String {
    // `serde_json` con solo `alloc` no expone CompactFormatter; el filtro HF
    // tampoco deja espacios tras `:` ni `,`.
    serde_json::to_string(value)
        .expect("json serializable")
        .replace(": ", ":")
        .replace(", ", ",")
}

// ------------------------------------------------------------------ Qwen3-Coder

const Q3C_SISTEMA_POR_DEFECTO: &str =
    "You are Qwen, a helpful AI assistant that can interact with a computer to solve tasks.";

const Q3C_TOOLS_CABECERA: &str = "\n\n# Tools\n\nYou have access to the following functions:\n\n<tools>";

const Q3C_TOOLS_PIE: &str = "\n</tools>\n\nIf you choose to call a function ONLY reply in the following format with NO suffix:\n\n<tool_call>\n<function=example_function_name>\n<parameter=example_parameter_1>\nvalue_1\n</parameter>\n<parameter=example_parameter_2>\nThis is the value for the second parameter\nthat can span\nmultiple lines\n</parameter>\n</function>\n</tool_call>\n\n<IMPORTANT>\nReminder:\n- Function calls MUST follow the specified format: an inner <function=...></function> block must be nested within <tool_call></tool_call> XML tags\n- Required parameters MUST be specified\n- You may provide optional reasoning for your function call in natural language BEFORE the function call, but NOT after\n- If there is no function call available, answer the question like normal with your current knowledge and do not tell the user about function calls\n</IMPORTANT>";

/// Plantilla de Qwen3-Coder (`Qwen/Qwen3-Coder-30B-A3B-Instruct`, `chat_template.jinja`).
///
/// Distinta de la de Qwen2.5: las herramientas van en XML (`<function><name>…`), las llamadas
/// son `<function=NOMBRE><parameter=P>VALOR</parameter></function>` y no hay system por defecto
/// si no hay herramientas. Se comprueba byte a byte contra la plantilla Jinja oficial
/// (`tests/self-improvement/reference/qwen3coder/`).
///
/// Diferencia conocida: las claves de un objeto JSON salen en orden alfabético (serde_json sin
/// `preserve_order`), mientras que Hugging Face conserva el de inserción.
pub fn render_qwen3coder(input: &ChatInput, add_generation_prompt: bool) -> Result<String, ChatError> {
    let mut out = String::new();
    let tools = &input.tools;
    let (sistema, resto): (Option<&str>, &[Message]) = match input.messages.first() {
        Some(m) if m.role == Role::System => (Some(contenido(m)), &input.messages[1..]),
        _ => (None, &input.messages[..]),
    };
    if let Some(s) = sistema {
        out.push_str(IM_START);
        out.push_str("system\n");
        out.push_str(s);
    } else if !tools.is_empty() {
        out.push_str(IM_START);
        out.push_str("system\n");
        out.push_str(Q3C_SISTEMA_POR_DEFECTO);
    }
    if !tools.is_empty() {
        out.push_str(Q3C_TOOLS_CABECERA);
        for tool in tools {
            out.push_str("\n<function>\n<name>");
            out.push_str(&tool.name);
            out.push_str("</name>");
            if let Some(d) = &tool.description {
                out.push_str("\n<description>");
                out.push_str(d.trim());
                out.push_str("</description>");
            }
            out.push_str("\n<parameters>");
            if let Some(props) = tool.parameters.get("properties").and_then(Value::as_object) {
                for (nombre, campos) in props {
                    out.push_str("\n<parameter>\n<name>");
                    out.push_str(nombre);
                    out.push_str("</name>");
                    if let Some(t) = campos.get("type") {
                        out.push_str("\n<type>");
                        out.push_str(&py_string(t));
                        out.push_str("</type>");
                    }
                    if let Some(d) = campos.get("description") {
                        out.push_str("\n<description>");
                        out.push_str(py_string(d).trim());
                        out.push_str("</description>");
                    }
                    extra_keys(&mut out, campos, &["name", "type", "description"]);
                    out.push_str("\n</parameter>");
                }
            }
            extra_keys(&mut out, &tool.parameters, &["type", "properties"]);
            out.push_str("\n</parameters>");
            out.push_str("\n</function>");
        }
        out.push_str(Q3C_TOOLS_PIE);
    }
    if sistema.is_some() || !tools.is_empty() {
        out.push_str(IM_END);
        out.push('\n');
    }

    for (i, message) in resto.iter().enumerate() {
        match message.role {
            Role::Assistant if !message.tool_calls.is_empty() => {
                out.push_str(IM_START);
                out.push_str("assistant");
                let texto = contenido(message).trim();
                if !texto.is_empty() {
                    out.push('\n');
                    out.push_str(texto);
                    out.push('\n');
                }
                for call in &message.tool_calls {
                    let args: Value = serde_json::from_str(&call.arguments).map_err(|_| {
                        ChatError::ArgumentosInvalidos {
                            llamada: call.id.clone(),
                        }
                    })?;
                    out.push_str("\n<tool_call>\n<function=");
                    out.push_str(&call.name);
                    out.push_str(">\n");
                    if let Some(obj) = args.as_object() {
                        for (nombre, valor) in obj {
                            out.push_str("<parameter=");
                            out.push_str(nombre);
                            out.push_str(">\n");
                            match valor {
                                Value::Object(_) | Value::Array(_) => out.push_str(&py_json(valor)),
                                otro => out.push_str(&py_string(otro)),
                            }
                            out.push_str("\n</parameter>\n");
                        }
                    }
                    out.push_str("</function>\n</tool_call>");
                }
                out.push_str(IM_END);
                out.push('\n');
            }
            Role::Tool => {
                let anterior_no_es_tool = i > 0 && resto[i - 1].role != Role::Tool;
                if anterior_no_es_tool {
                    out.push_str(IM_START);
                    out.push_str("user\n");
                }
                out.push_str("<tool_response>\n");
                out.push_str(contenido(message));
                out.push_str("\n</tool_response>\n");
                let es_ultimo = i + 1 == resto.len();
                let siguiente_no_es_tool = resto.get(i + 1).is_some_and(|m| m.role != Role::Tool);
                if siguiente_no_es_tool || es_ultimo {
                    out.push_str(IM_END);
                    out.push('\n');
                }
            }
            _ => turno_simple(&mut out, rol_texto(message.role), contenido(message)),
        }
    }
    if add_generation_prompt {
        out.push_str(IM_START);
        out.push_str("assistant\n");
    }
    Ok(out)
}

fn rol_texto(r: Role) -> &'static str {
    match r {
        Role::System => "system",
        Role::User => "user",
        Role::Assistant => "assistant",
        Role::Tool => "tool",
    }
}

/// `render_extra_keys` de la plantilla: las claves de `obj` que no están en `tratadas`.
fn extra_keys(out: &mut String, obj: &Value, tratadas: &[&str]) {
    let Some(mapa) = obj.as_object() else {
        return;
    };
    for (clave, valor) in mapa {
        if tratadas.contains(&clave.as_str()) {
            continue;
        }
        out.push_str("\n<");
        out.push_str(clave);
        out.push('>');
        match valor {
            Value::Object(_) | Value::Array(_) => out.push_str(&py_json(valor)),
            otro => out.push_str(&py_string(otro)),
        }
        out.push_str("</");
        out.push_str(clave);
        out.push('>');
    }
}

/// El filtro Jinja `string` sobre un valor escalar de Python: `True`/`False`/`None`, números y
/// cadenas tal cual.
fn py_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Bool(true) => String::from("True"),
        Value::Bool(false) => String::from("False"),
        Value::Null => String::from("None"),
        Value::Number(n) => alloc::format!("{n}"),
        otro => py_json(otro),
    }
}

/// `json.dumps(x, ensure_ascii=False)` de Python: separadores `", "` y `": "`.
fn py_json(v: &Value) -> String {
    match v {
        Value::Object(m) => {
            let partes: alloc::vec::Vec<String> = m
                .iter()
                .map(|(k, x)| {
                    alloc::format!("{}: {}", serde_json::to_string(k).unwrap_or_default(), py_json(x))
                })
                .collect();
            alloc::format!("{{{}}}", partes.join(", "))
        }
        Value::Array(a) => {
            let partes: alloc::vec::Vec<String> = a.iter().map(py_json).collect();
            alloc::format!("[{}]", partes.join(", "))
        }
        otro => serde_json::to_string(otro).unwrap_or_default(),
    }
}
