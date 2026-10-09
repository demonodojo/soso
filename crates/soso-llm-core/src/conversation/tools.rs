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
    /// Texto con el que se abrió la cerca en curso (para devolverlo como contenido si
    /// resulta que no era una llamada).
    fence_open: String,
    id_next: u32,
    tool_call: Option<ToolCall>,
    /// Devolver la llamada aunque no cuadre con el esquema o nombre una herramienta no
    /// declarada: lo decide el cliente (ver [`ToolCallParser::pasante`]).
    pasante: bool,
}

impl ToolCallParser {
    pub fn new(id_next: u32) -> Self {
        ToolCallParser {
            phase: Phase::Text,
            hold: String::new(),
            text: String::new(),
            body: String::new(),
            fence_open: String::new(),
            id_next,
            tool_call: None,
            pasante: false,
        }
    }

    /// Modo «pasante», como la API de OpenAI: una llamada bien formada se devuelve tal cual
    /// la escribió el modelo aunque sus argumentos no cumplan el esquema o nombre una
    /// herramienta no declarada. Quien la ejecuta (OpenCode) la comprueba y, si falla, le
    /// devuelve el error al modelo para que se corrija; con un 400 la sesión acababa sin
    /// esa oportunidad. Una llamada que ni siquiera es JSON válido sigue siendo error.
    pub fn pasante(mut self) -> Self {
        self.pasante = true;
        self
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
        if matches!(self.phase, Phase::InFence) && !self.body.trim_start().starts_with('{') {
            // Cerca sin cerrar que no abre una llamada: es texto.
            self.text.push_str(&self.fence_open);
            self.text.push_str(&self.body);
            self.body.clear();
            self.phase = Phase::Text;
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
        if let Some(call) = self.tool_call.as_mut() {
            ajustar_tipos(entrada, call);
        }
        match validate_assistant_turn(entrada, content.as_deref(), self.tool_call.as_ref()) {
            Ok(()) => {}
            Err(ChatError::ArgumentoInvalido { .. } | ChatError::HerramientaDesconocida { .. })
                if self.pasante => {}
            Err(e) => return Err(e),
        }
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
                        self.fence_open = data[at..at + len].to_string();
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
                        let resto = after_fence_close(data, at);
                        if !self.body.trim_start().starts_with('{') {
                            // No es una llamada: es texto con un bloque de código (un
                            // diff, un fragmento de Rust…). Se conserva como contenido.
                            self.text.push_str(&self.fence_open);
                            self.text.push_str(&self.body);
                            self.text.push_str(&data[at..data.len() - resto.len()]);
                            self.body.clear();
                            self.phase = Phase::Text;
                            data = resto;
                            continue;
                        }
                        data = resto;
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

/// Como [`parse_assistant_output`] en modo [`ToolCallParser::pasante`]: es lo que usan los
/// servidores.
pub fn parse_assistant_output_pasante(
    entrada: &ChatInput,
    generated: &str,
    id_next: u32,
) -> Result<AssistantTurn, ChatError> {
    let mut parser = ToolCallParser::new(id_next).pasante();
    parser.push(sin_marca_de_turno(generated))?;
    parser.finish(entrada)
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

/// El 7B a veces abre o cierra su respuesta con marcas de turno sueltas (`<|im_start|>`,
/// `<|im_end|>`, `<|endoftext|>`, y tras la primera un `assistant`), como si empezara o
/// acabara otro turno. No son contenido: se quitan del principio y del final, nada más.
fn sin_marca_de_turno(texto: &str) -> &str {
    const MARCAS: [&str; 3] = ["<|im_start|>", "<|im_end|>", "<|endoftext|>"];
    let mut t = texto.trim();
    loop {
        let antes = t;
        for m in MARCAS {
            if let Some(r) = t.strip_prefix(m) {
                t = r.trim_start();
                if m == "<|im_start|>" {
                    if let Some(r) = t.strip_prefix("assistant") {
                        t = r.trim_start();
                    }
                }
            }
            if let Some(r) = t.strip_suffix(m) {
                t = r.trim_end();
            }
        }
        if t == antes {
            break;
        }
    }
    if t.len() == texto.trim().len() {
        texto
    } else {
        t
    }
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

/// Etiquetas de cerca en las que el 7B suele envolver una llamada JSON.
fn lenguaje_de_llamada(lang: &str) -> bool {
    ["json", "bash", "sh", "shell", "console", "text", "tool", "tool_call"]
        .iter()
        .any(|l| lang.eq_ignore_ascii_case(l))
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
            if !lang.is_empty() && !lenguaje_de_llamada(lang) {
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
    // `parameters` es la clave de otros formatos de llamada a funciones y el 7B la usa a
    // veces en lugar de la `arguments` de Qwen.
    #[serde(alias = "parameters")]
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
    if json.trim_start().starts_with("<function=") {
        return llamada_xml(json);
    }
    let leer = |t: &str| {
        serde_json::Deserializer::from_str(t)
            .into_iter::<WireCall>()
            .next()
            .and_then(|r| r.ok())
    };
    if let Some(w) = leer(json) {
        return Some(w);
    }
    // El 7B escribe a veces los saltos de línea dentro de una cadena tal cual en vez de `\n`
    // (JSON no admite caracteres de control en cadenas): se escapan y se vuelve a intentar.
    let escapado = escapar_controles_en_cadenas(json);
    if let Some(w) = leer(&escapado) {
        return Some(w);
    }
    let cierres = cierres_que_faltan(&escapado)?;
    let mut completo = String::from(escapado.trim_end());
    completo.push_str(&cierres);
    leer(&completo)
}

/// Sustituye los saltos de línea, retornos y tabulaciones que hay **dentro de cadenas** por sus
/// escapes JSON; fuera de las cadenas el texto no se toca.
fn escapar_controles_en_cadenas(json: &str) -> String {
    let mut out = String::with_capacity(json.len() + 16);
    let (mut en_cadena, mut escape) = (false, false);
    for c in json.chars() {
        if en_cadena {
            if escape {
                escape = false;
                out.push(c);
                continue;
            }
            match c {
                '\\' => {
                    escape = true;
                    out.push(c);
                }
                '"' => {
                    en_cadena = false;
                    out.push(c);
                }
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                '\t' => out.push_str("\\t"),
                otro => out.push(otro),
            }
        } else {
            if c == '"' {
                en_cadena = true;
            }
            out.push(c);
        }
    }
    out
}

/// Llamada en el formato XML de Qwen3-Coder:
///
/// ```text
/// <function=NOMBRE>
/// <parameter=P>
/// valor (puede ocupar varias líneas)
/// </parameter>
/// </function>
/// ```
///
/// Todos los valores salen como cadenas (el texto entre las etiquetas, sin el salto de línea
/// que las separa); `ajustar_tipos` los convierte después al tipo del esquema.
fn llamada_xml(texto: &str) -> Option<WireCall> {
    let t = texto.trim_start().strip_prefix("<function=")?;
    let fin_nombre = t.find('>')?;
    let nombre = t[..fin_nombre].trim();
    if nombre.is_empty() {
        return None;
    }
    let mut resto = &t[fin_nombre + 1..];
    let mut args = serde_json::Map::new();
    loop {
        let r = resto.trim_start();
        if r.is_empty() || r.starts_with("</function>") {
            break;
        }
        let r = r.strip_prefix("<parameter=")?;
        let fin = r.find('>')?;
        let clave = r[..fin].trim().to_string();
        let cuerpo = &r[fin + 1..];
        let cierre = cuerpo.find("</parameter>")?;
        let mut valor = &cuerpo[..cierre];
        valor = valor.strip_prefix('\n').unwrap_or(valor);
        valor = valor.strip_suffix('\n').unwrap_or(valor);
        args.insert(clave, Value::String(String::from(valor)));
        resto = &cuerpo[cierre + "</parameter>".len()..];
    }
    Some(WireCall {
        name: String::from(nombre),
        arguments: Value::Object(args),
    })
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

/// El 7B escribe a veces `"true"` donde el esquema pide un booleano, o `"5"` donde pide un
/// número. Se convierte al tipo del esquema **sólo** cuando la cadena es exactamente eso;
/// cualquier otra discrepancia de tipo la sigue rechazando la validación.
fn ajustar_tipos(entrada: &ChatInput, call: &mut ToolCall) {
    let Some(def) = entrada.herramienta(&call.name) else {
        return;
    };
    let Ok(mut args) = serde_json::from_str::<Value>(&call.arguments) else {
        return;
    };
    let (Some(obj), Some(props)) = (
        args.as_object_mut(),
        def.parameters.get("properties").and_then(Value::as_object),
    ) else {
        return;
    };
    let mut cambiado = false;
    for (clave, valor) in obj.iter_mut() {
        let Some(tipo) = props.get(clave).and_then(|p| p.get("type")).and_then(Value::as_str)
        else {
            continue;
        };
        let Some(texto) = valor.as_str() else {
            continue;
        };
        let nuevo = match tipo {
            "boolean" => match texto {
                "true" => Some(Value::Bool(true)),
                "false" => Some(Value::Bool(false)),
                _ => None,
            },
            "array" | "object" => serde_json::from_str::<Value>(texto.trim()).ok().filter(|v| {
                (tipo == "array" && v.is_array()) || (tipo == "object" && v.is_object())
            }),
            "integer" => texto.trim().parse::<i64>().ok().map(Value::from),
            "number" => texto
                .trim()
                .parse::<f64>()
                .ok()
                .filter(|f| f.is_finite())
                .and_then(serde_json::Number::from_f64)
                .map(Value::Number),
            _ => None,
        };
        if let Some(n) = nuevo {
            *valor = n;
            cambiado = true;
        }
    }
    if cambiado {
        call.arguments = json_compact(&args);
    }
}

fn json_compact(value: &Value) -> String {
    serde_json::to_string(value)
        .expect("json serializable")
        .replace(": ", ":")
        .replace(", ", ",")
}
