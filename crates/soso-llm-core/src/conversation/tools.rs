//! Parser incremental de `<tool_call>` en la salida del asistente (T07, C2–C3).

use alloc::format;
use alloc::string::{String, ToString};

use serde::Deserialize;
use serde_json::Value;

use super::{ChatError, ChatInput, ToolCall};
use super::validate::{validate_assistant_turn, MAX_ARGUMENTOS_BYTES};

const OPEN: &str = "<tool_call>";
const CLOSE: &str = "</tool_call>";

/// Turno de asistente reconstruido desde texto generado.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssistantTurn {
    pub content: Option<String>,
    pub tool_call: Option<ToolCall>,
}

enum Phase {
    Text,
    InCall,
    AfterCall,
}

/// Acumula fragmentos de salida hasta [`finish`](Self::finish).
pub struct ToolCallParser {
    phase: Phase,
    hold: String,
    text: String,
    body: String,
    id_next: u32,
    tool_call: Option<ToolCall>,
}

impl ToolCallParser {
    pub fn new(id_next: u32) -> Self {
        ToolCallParser {
            phase: Phase::Text,
            hold: String::new(),
            text: String::new(),
            body: String::new(),
            id_next,
            tool_call: None,
        }
    }

    /// Añade un fragmento de texto generado.
    pub fn push(&mut self, chunk: &str) -> Result<(), ChatError> {
        if chunk.is_empty() {
            return Ok(());
        }
        match self.phase {
            Phase::AfterCall => {
                if chunk.chars().any(|c| !c.is_whitespace()) {
                    return Err(historial("varias llamadas o texto tras una llamada en un turno"));
                }
                return Ok(());
            }
            Phase::Text | Phase::InCall => {}
        }

        let mut data = String::new();
        data.push_str(&self.hold);
        data.push_str(chunk);
        self.hold.clear();
        self.process(&data)?;
        Ok(())
    }

    /// Cierra la generación y valida contra la petición.
    pub fn finish(mut self, entrada: &ChatInput) -> Result<AssistantTurn, ChatError> {
        if !self.hold.is_empty() {
            match self.phase {
                Phase::Text => {
                    self.text.push_str(&self.hold);
                    self.hold.clear();
                }
                Phase::InCall => {
                    self.body.push_str(&self.hold);
                    self.hold.clear();
                }
                Phase::AfterCall => {}
            }
        }
        if matches!(self.phase, Phase::InCall) {
            return Err(historial("llamada a herramienta truncada"));
        }
        let content = if self.text.is_empty() {
            None
        } else {
            Some(self.text)
        };
        validate_assistant_turn(entrada, content.as_deref(), self.tool_call.as_ref())?;
        Ok(AssistantTurn {
            content,
            tool_call: self.tool_call,
        })
    }

    fn process(&mut self, mut data: &str) -> Result<(), ChatError> {
        loop {
            match self.phase {
                Phase::Text => {
                    if let Some(pos) = data.find(OPEN) {
                        self.text.push_str(&data[..pos]);
                        data = &data[pos + OPEN.len()..];
                        self.phase = Phase::InCall;
                        self.body.clear();
                        continue;
                    }
                    let (committed, partial) = split_partial_suffix(data, OPEN);
                    self.text.push_str(&committed);
                    self.hold = partial;
                    break;
                }
                Phase::InCall => {
                    if let Some(pos) = data.find(CLOSE) {
                        self.body.push_str(&data[..pos]);
                        data = &data[pos + CLOSE.len()..];
                        let llamada = parse_body(&self.body, self.id_next)?;
                        self.id_next += 1;
                        self.tool_call = Some(llamada);
                        self.phase = Phase::AfterCall;
                        continue;
                    }
                    let (committed, partial) = split_partial_suffix(data, CLOSE);
                    self.body.push_str(&committed);
                    self.hold = partial;
                    break;
                }
                Phase::AfterCall => {
                    if data.chars().any(|c| !c.is_whitespace()) {
                        return Err(historial(
                            "varias llamadas o texto tras una llamada en un turno",
                        ));
                    }
                    break;
                }
            }
        }
        Ok(())
    }
}

/// Atajo para pruebas y entradas completas.
pub fn parse_assistant_output(
    entrada: &ChatInput,
    generated: &str,
    id_next: u32,
) -> Result<AssistantTurn, ChatError> {
    let mut parser = ToolCallParser::new(id_next);
    parser.push(generated)?;
    parser.finish(entrada)
}

fn historial(motivo: impl Into<String>) -> ChatError {
    ChatError::HistorialInvalido {
        motivo: motivo.into(),
    }
}

fn split_partial_suffix(data: &str, tag: &str) -> (String, String) {
    if data.is_empty() {
        return (String::new(), String::new());
    }
    let max = data.len().min(tag.len().saturating_sub(1));
    for n in (1..=max).rev() {
        if tag.starts_with(&data[data.len() - n..]) {
            let split = data.len() - n;
            return (data[..split].to_string(), data[split..].to_string());
        }
    }
    (data.to_string(), String::new())
}

#[derive(Deserialize)]
struct WireCall {
    name: String,
    arguments: Value,
}

fn parse_body(body: &str, id: u32) -> Result<ToolCall, ChatError> {
    let json = body.trim();
    if json.is_empty() {
        return Err(historial("tool_call sin cuerpo JSON"));
    }
    let wire: WireCall = serde_json::from_str(json).map_err(|_| {
        ChatError::ArgumentosInvalidos {
            llamada: format!("call_{id}"),
        }
    })?;
    if !wire.arguments.is_object() {
        return Err(ChatError::ArgumentosInvalidos {
            llamada: format!("call_{id}"),
        });
    }
    let arguments = json_compact(&wire.arguments);
    if arguments.len() > MAX_ARGUMENTOS_BYTES {
        return Err(ChatError::LimiteExcedido {
            que: String::from("arguments"),
            maximo: MAX_ARGUMENTOS_BYTES as u32,
            recibido: arguments.len() as u32,
        });
    }
    Ok(ToolCall::nueva(
        format!("call_{id}"),
        wire.name,
        arguments,
    ))
}

fn json_compact(value: &Value) -> String {
    serde_json::to_string(value)
        .expect("json serializable")
        .replace(": ", ":")
        .replace(", ", ",")
}
