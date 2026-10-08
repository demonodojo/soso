//! Parser incremental de llamadas en la salida del asistente (T07, C2–C3).
//!
//! Acepta `<tool_call>` y el bloque Markdown que emite el modelo cuando no
//! usa esa etiqueta: una cerca ` ```json ` con el mismo objeto `name` /
//! `arguments`.

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
    InFence,
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
            Phase::Text | Phase::InCall | Phase::InFence => {}
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
                Phase::InCall | Phase::InFence => {
                    self.body.push_str(&self.hold);
                    self.hold.clear();
                }
                Phase::AfterCall => {}
            }
        }
        if matches!(self.phase, Phase::InCall | Phase::InFence) {
            // El modelo suele cerrar el turno (token de parada) sin escribir `</tool_call>`
            // ni la cerca de cierre. Si lo que escribió es una llamada **completa** vale;
            // si el JSON está cortado, sigue siendo una llamada truncada.
            let cuerpo = self
                .body
                .trim_end_matches(|c: char| c.is_whitespace() || c == '`')
                .to_string();
            match parse_body(&cuerpo, self.id_next) {
                Ok(llamada) => {
                    self.tool_call = Some(llamada);
                    self.phase = Phase::AfterCall;
                }
                Err(_) => return Err(historial("llamada a herramienta truncada")),
            }
        } else if self.tool_call.is_none() {
            // JSON desnudo: sin etiqueta ni cerca, el modelo escribe sólo el objeto
            // `{"name":…, "arguments":{…}}`. Se toma por llamada únicamente si es **todo**
            // el turno y nombra una herramienta declarada; si no, es texto.
            let t = self.text.trim();
            if t.starts_with('{') && t.ends_with('}') {
                if let Some(w) = primer_objeto(t) {
                    if w.arguments.is_object() && entrada.herramienta(&w.name).is_some() {
                        if let Ok(llamada) = parse_body(t, self.id_next) {
                            self.tool_call = Some(llamada);
                            self.text.clear();
                        }
                    }
                }
            }
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
                Phase::Text => match next_open(data) {
                    Some(NextOpen::Tool(pos)) => {
                        self.text.push_str(&data[..pos]);
                        data = &data[pos + OPEN.len()..];
                        self.phase = Phase::InCall;
                        self.body.clear();
                        continue;
                    }
                    Some(NextOpen::Fence { at, len }) => {
                        self.text.push_str(&data[..at]);
                        data = &data[at + len..];
                        self.phase = Phase::InFence;
                        self.body.clear();
                        continue;
                    }
                    Some(NextOpen::Hold(at)) => {
                        self.text.push_str(&data[..at]);
                        self.hold = data[at..].to_string();
                        break;
                    }
                    None => {
                        let (committed, partial) = hold_open_prefix(data);
                        self.text.push_str(&committed);
                        self.hold = partial;
                        break;
                    }
                },
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
                Phase::InFence => {
                    if let Some(at) = fence_close(data) {
                        self.body.push_str(&data[..at]);
                        data = after_fence_close(data, at);
                        let llamada = parse_body(&self.body, self.id_next)?;
                        self.id_next += 1;
                        self.tool_call = Some(llamada);
                        self.phase = Phase::AfterCall;
                        continue;
                    }
                    let (committed, partial) = split_partial_suffix(data, "\n```");
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
    parser.push(sin_marca_de_turno(generated))?;
    parser.finish(entrada)
}

/// El 7B a veces abre su respuesta con un `<|im_start|>` suelto (y a veces `assistant`),
/// como si empezara otro turno. No es contenido: se quita del principio, nada más.
fn sin_marca_de_turno(texto: &str) -> &str {
    let t = texto.trim_start();
    let Some(resto) = t.strip_prefix("<|im_start|>") else {
        return texto;
    };
    let resto = resto.trim_start();
    resto.strip_prefix("assistant").map(str::trim_start).unwrap_or(resto)
}

fn historial(motivo: impl Into<String>) -> ChatError {
    ChatError::HistorialInvalido {
        motivo: motivo.into(),
    }
}

enum NextOpen {
    Tool(usize),
    Fence { at: usize, len: usize },
    Hold(usize),
}

enum FenceScan {
    Open { at: usize, len: usize },
    NeedMore { at: usize },
    None,
}

/// Primera apertura: `<tool_call>` o una cerca Markdown al inicio de línea.
fn next_open(data: &str) -> Option<NextOpen> {
    let tool = data.find(OPEN);
    match scan_fence(data) {
        FenceScan::Open { at, len } => {
            if tool.is_some_and(|t| t < at) {
                Some(NextOpen::Tool(tool.unwrap()))
            } else {
                Some(NextOpen::Fence { at, len })
            }
        }
        FenceScan::NeedMore { at } => {
            if tool.is_some_and(|t| t < at) {
                Some(NextOpen::Tool(tool.unwrap()))
            } else {
                Some(NextOpen::Hold(at))
            }
        }
        FenceScan::None => tool.map(NextOpen::Tool),
    }
}

/// Cerca ` ``` ` o ` ```json ` al principio de una línea, cerrada con salto.
fn scan_fence(data: &str) -> FenceScan {
    let bytes = data.as_bytes();
    let mut i = 0;
    while i + 3 <= bytes.len() {
        let at_line = i == 0 || bytes[i - 1] == b'\n';
        if at_line && bytes[i..].starts_with(b"```") {
            let after = &data[i + 3..];
            let mut lang_len = 0;
            for c in after.chars() {
                if c.is_ascii_alphabetic() {
                    lang_len += c.len_utf8();
                } else {
                    break;
                }
            }
            let lang = &after[..lang_len];
            if !lang.is_empty() && !lang.eq_ignore_ascii_case("json") {
                i += 3;
                continue;
            }
            let rest = &after[lang_len..];
            let ws_bytes = rest
                .chars()
                .take_while(|c| *c == ' ' || *c == '\t')
                .map(|c| c.len_utf8())
                .sum::<usize>();
            let after_ws = &rest[ws_bytes..];
            if after_ws.starts_with('\n') {
                return FenceScan::Open {
                    at: i,
                    len: 3 + lang_len + ws_bytes + 1,
                };
            }
            if after_ws.is_empty() {
                return FenceScan::NeedMore { at: i };
            }
            i += 3;
            continue;
        }
        i += 1;
    }
    if let Some(at) = partial_fence_at(data) {
        return FenceScan::NeedMore { at };
    }
    FenceScan::None
}

/// Sufijo de línea que todavía puede ser el arranque de una cerca.
fn partial_fence_at(data: &str) -> Option<usize> {
    let line_start = data.rfind('\n').map(|p| p + 1).unwrap_or(0);
    let line = &data[line_start..];
    if line.is_empty() {
        return None;
    }
    if line.strip_prefix("```json").is_some()
        || line.strip_prefix("```JSON").is_some()
        || line.strip_prefix("```").is_some()
    {
        let rest = line
            .strip_prefix("```json")
            .or_else(|| line.strip_prefix("```JSON"))
            .or_else(|| line.strip_prefix("```"))
            .unwrap_or("");
        if rest.chars().all(|c| c == ' ' || c == '\t') {
            return Some(line_start);
        }
        return None;
    }
    if "```json".starts_with(line) || "```JSON".starts_with(line) {
        return Some(line_start);
    }
    None
}

/// Índice del salto que precede a la línea de cierre ` ``` `.
fn fence_close(data: &str) -> Option<usize> {
    let mut from = 0;
    while let Some(rel) = data[from..].find("\n```") {
        let at = from + rel;
        let after = &data[at + 4..];
        let line_len = after.find('\n').unwrap_or(after.len());
        let line = &after[..line_len];
        if line.chars().all(|c| c.is_whitespace()) {
            return Some(at);
        }
        from = at + 4;
    }
    None
}

fn after_fence_close<'a>(data: &'a str, at: usize) -> &'a str {
    let rest = &data[at + 4..];
    let line_len = rest.find('\n').unwrap_or(rest.len());
    let mut end = at + 4 + line_len;
    if rest.as_bytes().get(line_len) == Some(&b'\n') {
        end += 1;
    }
    &data[end..]
}

/// Conserva un sufijo que podría ser el principio de `<tool_call>` o de una cerca.
fn hold_open_prefix(data: &str) -> (String, String) {
    if data.is_empty() {
        return (String::new(), String::new());
    }
    let max = data.len().min("<tool_call>".len().max("```json".len()));
    for n in (1..=max).rev() {
        let split = data.len() - n;
        if !data.is_char_boundary(split) {
            continue;
        }
        let suf = &data[split..];
        let line_ok = split == 0 || data.as_bytes().get(split - 1) == Some(&b'\n');
        let tool_ok = OPEN.starts_with(suf);
        let fence_ok = line_ok
            && ("```json".starts_with(suf) || "```JSON".starts_with(suf) || "```".starts_with(suf));
        if tool_ok || fence_ok {
            return (data[..split].to_string(), data[split..].to_string());
        }
    }
    (data.to_string(), String::new())
}

fn split_partial_suffix(data: &str, tag: &str) -> (String, String) {
    if data.is_empty() {
        return (String::new(), String::new());
    }
    let max = data.len().min(tag.len().saturating_sub(1));
    for n in (1..=max).rev() {
        let split = data.len() - n;
        // El corte va por **bytes**, y el modelo genera UTF-8: sin esta guarda,
        // una salida acabada en un carácter multibyte —«¡», «☕», cualquier
        // acento— revienta con «is not a char boundary» y se lleva por delante
        // el proceso entero del servidor. Un offset que parte un carácter no
        // puede ser el principio de una etiqueta ASCII, así que saltárselo no
        // pierde ninguna coincidencia.
        if !data.is_char_boundary(split) {
            continue;
        }
        if tag.starts_with(&data[split..]) {
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
    // Si el modelo escribe varias llamadas seguidas en el mismo bloque (quiere hacer todos
    // los pasos de golpe), se atiende **la primera**: el contrato es una llamada por turno
    // (`parallel_tool_calls: false`) y el agente pedirá el resto en el turno siguiente.
    // Lo que sigue a ese primer objeto no se interpreta. Un primer objeto incompleto o
    // inválido sigue siendo un error.
    let wire = primer_objeto(json).ok_or_else(|| ChatError::ArgumentosInvalidos {
        llamada: format!("call_{id}"),
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

/// Primer objeto `{"name":…, "arguments":…}` de `json`; lo que venga detrás no se mira.
///
/// Si el JSON se corta **sólo por faltar llaves o corchetes de cierre** (el 7B suele
/// cerrar `arguments` y olvidarse del objeto exterior) se completan, siempre que no
/// haya una cadena sin cerrar ni falte nada más que los cierres. Un valor cortado
/// (`"ruta": "a.r`, `"ruta":`) sigue sin ser una llamada.
fn primer_objeto(json: &str) -> Option<WireCall> {
    let leer = |t: &str| {
        serde_json::Deserializer::from_str(t)
            .into_iter::<WireCall>()
            .next()
            .and_then(|r| r.ok())
    };
    if let Some(w) = leer(json) {
        return Some(w);
    }
    let cierres = cierres_que_faltan(json)?;
    let mut completo = String::from(json.trim_end());
    completo.push_str(&cierres);
    leer(&completo)
}

/// Los `}` y `]` que cierran lo que `json` dejó abierto; `None` si hay una cadena sin
/// cerrar, si lo abierto es demasiado (más de 3 niveles) o si no falta nada.
fn cierres_que_faltan(json: &str) -> Option<String> {
    let mut pila: alloc::vec::Vec<char> = alloc::vec::Vec::new();
    let (mut en_cadena, mut escape) = (false, false);
    for c in json.chars() {
        if en_cadena {
            if escape {
                escape = false;
            } else if c == '\\' {
                escape = true;
            } else if c == '"' {
                en_cadena = false;
            }
            continue;
        }
        match c {
            '"' => en_cadena = true,
            '{' => pila.push('}'),
            '[' => pila.push(']'),
            '}' | ']' => {
                if pila.pop() != Some(c) {
                    return None;
                }
            }
            _ => {}
        }
    }
    if en_cadena || pila.is_empty() || pila.len() > 3 {
        return None;
    }
    Some(pila.into_iter().rev().collect())
}

fn json_compact(value: &Value) -> String {
    serde_json::to_string(value)
        .expect("json serializable")
        .replace(": ", ":")
        .replace(", ", ",")
}
