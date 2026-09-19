//! Plantilla de chat por familia (ficha T06, contrato C2).
//!
//! Traduce un [`ChatInput`] validado al texto que el tokenizer debe segmentar.
//! La referencia son los fixtures de T03; aquí no se inventa formato.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use serde_json::Value;

use crate::conversation::{
    validate_input, ChatError, ChatInput, Message, ModelProfile, Role, ToolDefinition,
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
    validate_input(input)?;
    if !tokenizer.tiene_vocabulario() {
        return Err(ChatError::TokenizacionInvalida {
            motivo: String::from("el tokenizer no trae vocabulario"),
        });
    }
    let texto = match profile.family.as_str() {
        "qwen2" => render_qwen2(input, true)?,
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
