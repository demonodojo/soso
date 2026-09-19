//! Parseo Chat Completions → dominio + preparación para inferencia (T10, C3).

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use serde::Deserialize;
use serde_json::Value;
use soso_llm_core::conversation::{
    validate_input, ChatError, ChatInput, Message, ModelProfile, Role, ToolCall, ToolChoice,
    ToolDefinition,
};
use soso_llm_core::tokenizer::Tokenizer;
use soso_llm_core::conversation::render_messages;

/// Petición POST `/v1/chat/completions` tras deserializar JSON.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WireChatCompletion {
    pub model: Option<String>,
    pub messages: Vec<WireMessage>,
    #[serde(default)]
    pub tools: Vec<WireTool>,
    #[serde(default)]
    pub tool_choice: Option<WireToolChoice>,
    #[serde(default)]
    pub stream: bool,
    #[serde(default)]
    pub stream_options: Option<WireStreamOptions>,
    #[serde(default)]
    pub max_tokens: Option<u32>,
    #[serde(default)]
    pub max_completion_tokens: Option<u32>,
    #[serde(default)]
    pub temperature: Option<f64>,
    #[serde(default)]
    pub top_p: Option<f64>,
    #[serde(default)]
    pub seed: Option<i64>,
    #[serde(default)]
    pub parallel_tool_calls: Option<bool>,
    #[serde(default)]
    pub n: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WireMessage {
    pub role: String,
    #[serde(default)]
    pub content: Option<Value>,
    #[serde(default)]
    pub tool_calls: Vec<WireToolCall>,
    #[serde(default)]
    pub tool_call_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WireToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub function: WireFunctionCall,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WireFunctionCall {
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WireTool {
    #[serde(rename = "type")]
    pub kind: String,
    pub function: WireFunctionDef,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WireFunctionDef {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    pub parameters: Value,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(untagged)]
pub enum WireToolChoice {
    Modo(String),
    Funcion {
        #[serde(rename = "type")]
        kind: String,
        function: WireToolChoiceFn,
    },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WireToolChoiceFn {
    pub name: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WireStreamOptions {
    #[serde(default)]
    pub include_usage: bool,
}

/// Salida lista para T16/T12.
#[derive(Debug, Clone, PartialEq)]
pub struct PreparedChatCompletion {
    pub input: ChatInput,
    pub model_id: String,
    pub prompt_tokens: u32,
    pub max_new_tokens: u32,
    pub stream: bool,
    pub include_usage: bool,
    pub temperature: f64,
    pub top_p: f64,
    pub seed: Option<i64>,
}

/// Error del adaptador API (mapeable a HTTP).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApiError {
    JsonInvalido { motivo: String },
    PeticionInvalida { motivo: String },
    ModeloDesconocido { model: String },
    ModalidadNoSoportada { tipo: String },
    AliasContradictorio { campo: String },
    ParametroNoFinito { campo: String },
    Dominio(ChatError),
}

impl ApiError {
    pub fn status_code(&self) -> u16 {
        match self {
            ApiError::ModeloDesconocido { .. } => 404,
            ApiError::ModalidadNoSoportada { .. }
            | ApiError::Dominio(
                ChatError::ContextoExcedido { .. }
                | ChatError::EsquemaNoSoportado { .. }
                | ChatError::FamiliaNoImplementada { .. },
            ) => 422,
            ApiError::JsonInvalido { .. }
            | ApiError::PeticionInvalida { .. }
            | ApiError::AliasContradictorio { .. }
            | ApiError::ParametroNoFinito { .. }
            | ApiError::Dominio(
                ChatError::HistorialInvalido { .. }
                | ChatError::ResultadoHuerfano { .. }
                | ChatError::ResultadoDuplicado { .. }
                | ChatError::LlamadaDuplicada { .. }
                | ChatError::LlamadaPendiente { .. }
                | ChatError::HerramientaDuplicada { .. }
                | ChatError::SeleccionInvalida { .. }
                | ChatError::ArgumentoInvalido { .. }
                | ChatError::LimiteExcedido { .. }
                | ChatError::HerramientaDesconocida { .. }
                | ChatError::ArgumentosInvalidos { .. }
                | ChatError::TokenizacionInvalida { .. },
            ) => 400,
        }
    }

    pub fn error_type(&self) -> &'static str {
        match self.status_code() {
            404 => "not_found_error",
            422 => "invalid_request_error",
            _ => "invalid_request_error",
        }
    }

    pub fn code(&self) -> &'static str {
        match self {
            ApiError::ModeloDesconocido { .. } => "model_not_found",
            ApiError::ModalidadNoSoportada { .. } => "unsupported_modality",
            ApiError::AliasContradictorio { .. } => "conflicting_parameters",
            ApiError::ParametroNoFinito { .. } => "invalid_parameter",
            ApiError::JsonInvalido { .. } => "invalid_json",
            ApiError::PeticionInvalida { .. } => "invalid_request",
            ApiError::Dominio(ChatError::ContextoExcedido { .. }) => "context_length_exceeded",
            ApiError::Dominio(ChatError::EsquemaNoSoportado { .. }) => "unsupported_schema",
            ApiError::Dominio(ChatError::LimiteExcedido { .. }) => "limit_exceeded",
            _ => "invalid_request",
        }
    }

    pub fn message(&self) -> String {
        match self {
            ApiError::JsonInvalido { motivo } => format!("JSON inválido: {motivo}"),
            ApiError::PeticionInvalida { motivo } => motivo.clone(),
            ApiError::ModeloDesconocido { model } => format!("modelo desconocido: {model}"),
            ApiError::ModalidadNoSoportada { tipo } => {
                format!("modalidad de contenido no soportada: {tipo}")
            }
            ApiError::AliasContradictorio { campo } => {
                format!("valores contradictorios en {campo}")
            }
            ApiError::ParametroNoFinito { campo } => format!("{campo} debe ser un número finito"),
            ApiError::Dominio(e) => {
                let s = e.to_string();
                if s.len() > 512 {
                    format!("{}…", &s[..512])
                } else {
                    s
                }
            }
        }
    }

    pub fn to_error_body(&self) -> Value {
        serde_json::json!({
            "error": {
                "message": self.message(),
                "type": self.error_type(),
                "code": self.code(),
            }
        })
    }
}

impl From<ChatError> for ApiError {
    fn from(e: ChatError) -> Self {
        ApiError::Dominio(e)
    }
}

pub fn parse_chat_completions(body: &str) -> Result<WireChatCompletion, ApiError> {
    serde_json::from_str(body).map_err(|e| ApiError::JsonInvalido {
        motivo: e.to_string(),
    })
}

pub fn validate_wire_policy(wire: &WireChatCompletion) -> Result<(), ApiError> {
    let model = wire.model.as_deref().unwrap_or("").trim();
    if model.is_empty() {
        return Err(ApiError::PeticionInvalida {
            motivo: String::from("falta el campo model"),
        });
    }
    match wire.n {
        None | Some(1) => {}
        Some(n) => {
            return Err(ApiError::PeticionInvalida {
                motivo: format!("n={n} no soportado; solo n=1"),
            });
        }
    }
    if wire.parallel_tool_calls == Some(true) {
        return Err(ApiError::PeticionInvalida {
            motivo: String::from("parallel_tool_calls debe ser false o omitirse"),
        });
    }
    match (wire.max_tokens, wire.max_completion_tokens) {
        (Some(a), Some(b)) if a != b => {
            return Err(ApiError::AliasContradictorio {
                campo: String::from("max_tokens y max_completion_tokens"),
            });
        }
        _ => {}
    }
    if let Some(t) = wire.temperature {
        comprobar_finito("temperature", t)?;
    }
    if let Some(p) = wire.top_p {
        comprobar_finito("top_p", p)?;
    }
    Ok(())
}

fn comprobar_finito(campo: &str, v: f64) -> Result<(), ApiError> {
    if v.is_finite() {
        Ok(())
    } else {
        Err(ApiError::ParametroNoFinito {
            campo: String::from(campo),
        })
    }
}

pub fn wire_to_chat_input(wire: &WireChatCompletion) -> Result<ChatInput, ApiError> {
    let mut messages = Vec::with_capacity(wire.messages.len());
    for (i, wm) in wire.messages.iter().enumerate() {
        messages.push(wire_message(wm, i)?);
    }
    let tools = wire
        .tools
        .iter()
        .enumerate()
        .map(|(i, t)| wire_tool(t, i))
        .collect::<Result<Vec<_>, _>>()?;
    let tool_choice = wire_tool_choice(wire.tool_choice.as_ref())?;
    Ok(ChatInput {
        messages,
        tools,
        tool_choice,
    })
}

fn wire_message(wm: &WireMessage, indice: usize) -> Result<Message, ApiError> {
    let role = parse_role(&wm.role, indice)?;
    let content = parse_content(wm.content.as_ref(), indice)?;
    let mut tool_calls = Vec::with_capacity(wm.tool_calls.len());
    for (j, tc) in wm.tool_calls.iter().enumerate() {
        tool_calls.push(wire_tool_call(tc, indice, j)?);
    }
    Ok(Message {
        role,
        content,
        tool_calls,
        tool_call_id: wm.tool_call_id.clone(),
    })
}

fn parse_role(s: &str, indice: usize) -> Result<Role, ApiError> {
    match s {
        "system" => Ok(Role::System),
        "user" => Ok(Role::User),
        "assistant" => Ok(Role::Assistant),
        "tool" => Ok(Role::Tool),
        otro => Err(ApiError::PeticionInvalida {
            motivo: format!("messages[{indice}].role desconocido: {otro}"),
        }),
    }
}

fn parse_content(val: Option<&Value>, indice: usize) -> Result<Option<String>, ApiError> {
    let Some(v) = val else {
        return Ok(None);
    };
    match v {
        Value::Null => Ok(None),
        Value::String(s) => Ok(Some(s.clone())),
        Value::Array(parts) => {
            let mut out = String::new();
            for (j, part) in parts.iter().enumerate() {
                let obj = part.as_object().ok_or_else(|| ApiError::PeticionInvalida {
                    motivo: format!("messages[{indice}].content[{j}] debe ser un objeto"),
                })?;
                let tipo = obj
                    .get("type")
                    .and_then(|t| t.as_str())
                    .ok_or_else(|| ApiError::PeticionInvalida {
                        motivo: format!("messages[{indice}].content[{j}] sin type"),
                    })?;
                match tipo {
                    "text" => {
                        let text = obj
                            .get("text")
                            .and_then(|t| t.as_str())
                            .ok_or_else(|| ApiError::PeticionInvalida {
                                motivo: format!("messages[{indice}].content[{j}] text inválido"),
                            })?;
                        if !out.is_empty() {
                            out.push('\n');
                        }
                        out.push_str(text);
                    }
                    "image_url" | "input_audio" | "video" | "file" => {
                        return Err(ApiError::ModalidadNoSoportada {
                            tipo: String::from(tipo),
                        });
                    }
                    otro => {
                        return Err(ApiError::ModalidadNoSoportada {
                            tipo: String::from(otro),
                        });
                    }
                }
            }
            if out.is_empty() {
                Ok(None)
            } else {
                Ok(Some(out))
            }
        }
        _ => Err(ApiError::PeticionInvalida {
            motivo: format!("messages[{indice}].content debe ser string, null o array"),
        }),
    }
}

fn wire_tool_call(tc: &WireToolCall, msg: usize, j: usize) -> Result<ToolCall, ApiError> {
    if tc.kind != "function" {
        return Err(ApiError::PeticionInvalida {
            motivo: format!(
                "messages[{msg}].tool_calls[{j}].type debe ser function"
            ),
        });
    }
    Ok(ToolCall::nueva(
        tc.id.clone(),
        tc.function.name.clone(),
        tc.function.arguments.clone(),
    ))
}

fn wire_tool(t: &WireTool, i: usize) -> Result<ToolDefinition, ApiError> {
    if t.kind != "function" {
        return Err(ApiError::PeticionInvalida {
            motivo: format!("tools[{i}].type debe ser function"),
        });
    }
    Ok(ToolDefinition::nueva(
        t.function.name.clone(),
        t.function.description.clone(),
        t.function.parameters.clone(),
    ))
}

fn wire_tool_choice(w: Option<&WireToolChoice>) -> Result<ToolChoice, ApiError> {
    let Some(w) = w else {
        return Ok(ToolChoice::Auto);
    };
    match w {
        WireToolChoice::Modo(s) => match s.as_str() {
            "auto" => Ok(ToolChoice::Auto),
            "none" => Ok(ToolChoice::None),
            "required" => Ok(ToolChoice::Required),
            otro => Err(ApiError::PeticionInvalida {
                motivo: format!("tool_choice desconocido: {otro}"),
            }),
        },
        WireToolChoice::Funcion { kind, function } => {
            if kind != "function" {
                return Err(ApiError::PeticionInvalida {
                    motivo: String::from("tool_choice.type debe ser function"),
                });
            }
            Ok(ToolChoice::Named(function.name.clone()))
        }
    }
}

fn resolver_max_new(wire: &WireChatCompletion, profile: &ModelProfile) -> u32 {
    let pedido = wire
        .max_tokens
        .or(wire.max_completion_tokens)
        .unwrap_or(profile.max_output_tokens);
    pedido.min(profile.max_output_tokens)
}

pub fn prepare_chat_completion(
    body: &str,
    profile: &ModelProfile,
    tokenizer: &Tokenizer,
) -> Result<PreparedChatCompletion, ApiError> {
    let wire = parse_chat_completions(body)?;
    validate_wire_policy(&wire)?;
    let input = wire_to_chat_input(&wire)?;
    validate_input(&input)?;
    let model = wire.model.as_ref().unwrap().trim();
    if model != profile.id {
        return Err(ApiError::ModeloDesconocido {
            model: model.to_string(),
        });
    }
    let max_new_tokens = resolver_max_new(&wire, profile);
    let ids = render_messages(&input, profile, tokenizer)?;
    let prompt_tokens = ids.len() as u32;
    if !profile.cabe(prompt_tokens, max_new_tokens) {
        return Err(ChatError::ContextoExcedido {
            necesarios: prompt_tokens.saturating_add(max_new_tokens),
            disponibles: profile.context_tokens,
        }
        .into());
    }
    let temperature = wire.temperature.unwrap_or(1.0);
    let top_p = wire.top_p.unwrap_or(1.0);
    comprobar_finito("temperature", temperature)?;
    comprobar_finito("top_p", top_p)?;
    let include_usage = wire
        .stream_options
        .as_ref()
        .is_some_and(|o| o.include_usage);
    Ok(PreparedChatCompletion {
        input,
        model_id: model.to_string(),
        prompt_tokens,
        max_new_tokens,
        stream: wire.stream,
        include_usage,
        temperature,
        top_p,
        seed: wire.seed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finito_rechaza_nan() {
        assert!(comprobar_finito("temperature", f64::NAN).is_err());
    }
}
