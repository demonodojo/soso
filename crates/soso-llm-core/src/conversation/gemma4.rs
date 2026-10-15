//! Plantilla de chat y llamadas a herramientas de Gemma 4 (G6 de `docs/PLAN-GEMMA4.md`).
//!
//! El render sigue la plantilla Jinja oficial de `google/gemma-4-12b-it-qat-q4_0-gguf`
//! (`tokenizer.chat_template`), turno a turno, y está contrastado con el `/apply-template` de llama.cpp en
//! `tests/fixtures/gemma4_plantilla.json`. Rasgos que no son obvios:
//!
//! - **No hay BOS en el texto**: lo pone el tokenizer, igual que con Qwen.
//! - Un turno de asistente con llamadas **no se cierra**: tras la llamada y su resultado el modelo sigue en el
//!   mismo turno (`<|tool_call>…<tool_call|><|tool_response>…<tool_response|>`), y dos mensajes de asistente
//!   seguidos se funden en un solo turno `model`.
//! - Los argumentos van en el formato propio de Gemma (`clave:valor`, cadenas entre `<|"|>`), no en JSON.
//! - Sin pensamiento, el aviso de generación termina en `<|channel>thought\n<channel|>` (canal vacío).

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use serde_json::{Map, Value};

use crate::conversation::{
    validate_assistant_turn, AssistantTurn, ChatError, ChatInput, Message, Role, ToolCall,
};

const COMILLA: &str = "<|\"|>";
const ABRE_LLAMADA: &str = "<|tool_call>";
const CIERRA_LLAMADA: &str = "<tool_call|>";
const ABRE_CANAL: &str = "<|channel>";
const CIERRA_CANAL: &str = "<channel|>";

/// ¿Tiene este texto generado la forma de una respuesta de Gemma 4? Las marcas no aparecen en otras familias.
pub fn parece_gemma4(texto: &str) -> bool {
    texto.contains(ABRE_LLAMADA) || texto.contains(ABRE_CANAL) || texto.contains(CIERRA_CANAL) || texto.contains("<turn|>")
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Previo {
    Nada,
    Pensar,
    Herramientas,
    Llamada,
    Resultado,
}

/// Texto del prompt. `enable_thinking` pide al modelo que razone antes de contestar.
pub fn render_gemma4(input: &ChatInput, add_generation_prompt: bool, enable_thinking: bool) -> Result<String, ChatError> {
    let mut out = String::new();
    let mut previo = Previo::Nada;
    let mut mensajes: &[Message] = &input.messages;

    let primero_sistema = mensajes.first().is_some_and(|m| m.role == Role::System);
    if enable_thinking || !input.tools.is_empty() || primero_sistema {
        out.push_str("<|turn>system\n");
        if enable_thinking {
            out.push_str("<|think|>\n");
            previo = Previo::Pensar;
        }
        if primero_sistema {
            if let Some(c) = &mensajes[0].content {
                out.push_str(c.trim());
            }
            mensajes = &mensajes[1..];
        }
        if !input.tools.is_empty() {
            for t in &input.tools {
                out.push_str("<|tool>");
                out.push_str(declaracion(&t.name, t.description.as_deref().unwrap_or(""), &t.parameters).trim());
                out.push_str("<tool|>");
            }
            previo = Previo::Herramientas;
        }
        out.push_str("<turn|>\n");
    }

    let mut rol_previo: Option<Role> = None;
    for (i, m) in mensajes.iter().enumerate() {
        if m.role == Role::Tool {
            continue;
        }
        previo = Previo::Nada;
        let es_modelo = m.role == Role::Assistant;
        let sigue_turno = es_modelo && rol_previo == Some(Role::Assistant);
        if !sigue_turno {
            out.push_str("<|turn>");
            out.push_str(if es_modelo { "model" } else { m.role.como_texto() });
            out.push('\n');
        }

        for c in &m.tool_calls {
            out.push_str(ABRE_LLAMADA);
            out.push_str("call:");
            out.push_str(&c.name);
            out.push('{');
            let args = serde_json::from_str::<Value>(&c.arguments).ok();
            if let Some(Value::Object(mapa)) = &args {
                let mut claves: Vec<&String> = mapa.keys().collect();
                claves.sort();
                for (n, k) in claves.iter().enumerate() {
                    if n > 0 {
                        out.push(',');
                    }
                    out.push_str(k);
                    out.push(':');
                    argumento(&mapa[*k], false, &mut out);
                }
            }
            out.push('}');
            out.push_str(CIERRA_LLAMADA);
        }
        if !m.tool_calls.is_empty() {
            previo = Previo::Llamada;
        }

        let mut hubo_resultado = false;
        if !m.tool_calls.is_empty() {
            for seguiente in &mensajes[i + 1..] {
                if seguiente.role != Role::Tool {
                    break;
                }
                let nombre = m
                    .tool_calls
                    .iter()
                    .find(|c| Some(&c.id) == seguiente.tool_call_id.as_ref())
                    .map(|c| c.name.as_str())
                    .unwrap_or("unknown");
                out.push_str("<|tool_response>response:");
                out.push_str(nombre);
                out.push_str("{value:");
                match &seguiente.content {
                    Some(t) => {
                        out.push_str(COMILLA);
                        out.push_str(t);
                        out.push_str(COMILLA);
                    }
                    None => out.push_str("null"),
                }
                out.push_str("}<tool_response|>");
                hubo_resultado = true;
                previo = Previo::Resultado;
            }
        }

        let contenido = match &m.content {
            Some(c) if es_modelo => sin_pensamiento(c),
            Some(c) => c.trim().to_string(),
            None => String::new(),
        };
        out.push_str(&contenido);
        let hay_contenido = !contenido.trim().is_empty();

        let siguiente_no_herramienta = mensajes[i + 1..].iter().find(|x| x.role != Role::Tool).map(|x| x.role);
        let continua = es_modelo
            && siguiente_no_herramienta == Some(Role::Assistant)
            && (m.tool_calls.is_empty() || hubo_resultado);

        if previo == Previo::Llamada && !hubo_resultado {
            out.push_str("<|tool_response>");
        } else if continua {
        } else if !(hubo_resultado && !hay_contenido && siguiente_no_herramienta.is_none()) {
            out.push_str("<turn|>\n");
        }
        rol_previo = Some(m.role);
    }

    if add_generation_prompt {
        if previo != Previo::Resultado && previo != Previo::Llamada {
            out.push_str("<|turn>model\n");
            if !enable_thinking {
                out.push_str("<|channel>thought\n<channel|>");
            }
        } else if previo == Previo::Resultado && enable_thinking {
            out.push_str("<|channel>thought\n");
        }
    }
    Ok(out)
}

/// Quita los canales de pensamiento (`<|channel>…<channel|>`) de un texto del asistente y lo recorta.
fn sin_pensamiento(texto: &str) -> String {
    let mut r = String::new();
    for parte in texto.split(CIERRA_CANAL) {
        match parte.find(ABRE_CANAL) {
            Some(p) => r.push_str(&parte[..p]),
            None => r.push_str(parte),
        }
    }
    r.trim().to_string()
}

fn mayusculas(v: &Value) -> String {
    v.as_str().unwrap_or("").to_uppercase()
}

fn es_vacio(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) | Some(Value::Bool(false)) => true,
        Some(Value::String(s)) => s.is_empty(),
        Some(Value::Array(a)) => a.is_empty(),
        Some(Value::Object(o)) => o.is_empty(),
        Some(Value::Number(n)) => n.as_f64() == Some(0.0),
        Some(Value::Bool(true)) => false,
    }
}

fn claves_ordenadas(m: &Map<String, Value>) -> Vec<&String> {
    let mut k: Vec<&String> = m.keys().collect();
    k.sort();
    k
}

/// `format_argument` de la plantilla.
fn argumento(v: &Value, escapar_claves: bool, out: &mut String) {
    match v {
        Value::Null => out.push_str("null"),
        Value::String(s) => {
            out.push_str(COMILLA);
            out.push_str(s);
            out.push_str(COMILLA);
        }
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Object(m) => {
            out.push('{');
            for (n, k) in claves_ordenadas(m).iter().enumerate() {
                if n > 0 {
                    out.push(',');
                }
                if escapar_claves {
                    out.push_str(COMILLA);
                    out.push_str(k);
                    out.push_str(COMILLA);
                } else {
                    out.push_str(k);
                }
                out.push(':');
                argumento(&m[*k], escapar_claves, out);
            }
            out.push('}');
        }
        Value::Array(a) => {
            out.push('[');
            for (n, x) in a.iter().enumerate() {
                if n > 0 {
                    out.push(',');
                }
                argumento(x, escapar_claves, out);
            }
            out.push(']');
        }
        Value::Number(n) => out.push_str(&n.to_string()),
    }
}

fn lista_requeridos(v: &Value, out: &mut String) {
    out.push('[');
    if let Some(a) = v.as_array() {
        for (n, x) in a.iter().enumerate() {
            if n > 0 {
                out.push(',');
            }
            out.push_str(COMILLA);
            out.push_str(x.as_str().unwrap_or(""));
            out.push_str(COMILLA);
        }
    }
    out.push(']');
}

/// Coma si ya había algo en el objeto; si no, marca que ya lo hay.
fn coma(hay: &mut bool, out: &mut String) {
    if *hay {
        out.push(',');
    } else {
        *hay = true;
    }
}

/// `format_parameters` de la plantilla.
fn parametros(props: &Map<String, Value>, filtrar: bool, out: &mut String) {
    const ESTANDAR: [&str; 5] = ["description", "type", "properties", "required", "nullable"];
    let mut primero = false;
    for clave in claves_ordenadas(props) {
        if filtrar && ESTANDAR.contains(&clave.as_str()) {
            continue;
        }
        let valor = &props[clave];
        if primero {
            out.push(',');
        }
        primero = true;
        out.push_str(clave);
        out.push_str(":{");
        let mut hay = false;
        if !es_vacio(valor.get("description")) {
            out.push_str("description:");
            out.push_str(COMILLA);
            out.push_str(valor["description"].as_str().unwrap_or(""));
            out.push_str(COMILLA);
            hay = true;
        }
        let tipo = valor.get("type").map(mayusculas).unwrap_or_default();
        if tipo == "STRING" {
            if !es_vacio(valor.get("enum")) {
                coma(&mut hay, out);
                out.push_str("enum:");
                argumento(&valor["enum"], true, out);
            }
        } else if tipo == "ARRAY" {
            if let Some(items) = valor.get("items").and_then(Value::as_object).filter(|m| !m.is_empty()) {
                coma(&mut hay, out);
                out.push_str("items:{");
                let mut hay_item = false;
                for k in claves_ordenadas(items) {
                    let v = &items[k];
                    if v.is_null() {
                        continue;
                    }
                    if hay_item {
                        out.push(',');
                    }
                    hay_item = true;
                    match k.as_str() {
                        "properties" => {
                            out.push_str("properties:{");
                            if let Some(p) = v.as_object() {
                                parametros(p, false, out);
                            }
                            out.push('}');
                        }
                        "required" => {
                            out.push_str("required:");
                            lista_requeridos(v, out);
                        }
                        "type" => {
                            out.push_str("type:");
                            match v {
                                Value::String(s) => argumento(&Value::String(s.to_uppercase()), true, out),
                                Value::Array(a) => {
                                    let m: Vec<Value> = a.iter().map(|x| Value::String(mayusculas(x))).collect();
                                    argumento(&Value::Array(m), true, out);
                                }
                                otro => argumento(otro, true, out),
                            }
                        }
                        _ => {
                            out.push_str(k);
                            out.push(':');
                            argumento(v, true, out);
                        }
                    }
                }
                out.push('}');
            }
        }
        if !es_vacio(valor.get("nullable")) {
            coma(&mut hay, out);
            out.push_str("nullable:true");
        }
        if tipo == "OBJECT" {
            if let Some(p) = valor.get("properties").and_then(Value::as_object) {
                coma(&mut hay, out);
                out.push_str("properties:{");
                parametros(p, false, out);
                out.push('}');
            } else if let Some(m) = valor.as_object() {
                coma(&mut hay, out);
                out.push_str("properties:{");
                parametros(m, true, out);
                out.push('}');
            }
            if !es_vacio(valor.get("required")) {
                coma(&mut hay, out);
                out.push_str("required:");
                lista_requeridos(&valor["required"], out);
            }
        }
        coma(&mut hay, out);
        out.push_str("type:");
        out.push_str(COMILLA);
        out.push_str(&tipo);
        out.push_str(COMILLA);
        out.push('}');
    }
}

/// `format_function_declaration` de la plantilla.
fn declaracion(nombre: &str, descripcion: &str, params: &Value) -> String {
    let mut out = format!("declaration:{nombre}{{description:{COMILLA}{descripcion}{COMILLA}");
    if !es_vacio(Some(params)) {
        out.push_str(",parameters:{");
        if let Some(p) = params.get("properties").and_then(Value::as_object).filter(|m| !m.is_empty()) {
            out.push_str("properties:{");
            parametros(p, false, &mut out);
            out.push_str("},");
        }
        if !es_vacio(params.get("required")) {
            out.push_str("required:");
            lista_requeridos(&params["required"], &mut out);
            out.push(',');
        }
        if !es_vacio(params.get("type")) {
            out.push_str("type:");
            out.push_str(COMILLA);
            out.push_str(&mayusculas(&params["type"]));
            out.push_str(COMILLA);
            out.push('}');
        }
    }
    out.push('}');
    out
}

// ---------------------------------------------------------------------------------------------------------
// Salida del modelo
// ---------------------------------------------------------------------------------------------------------

/// Interpreta lo que generó Gemma 4: texto, y como mucho una llamada `<|tool_call>call:nombre{…}<tool_call|>`.
/// Los canales de pensamiento se descartan. Si hay varias llamadas se atiende **la primera**, como con Qwen.
pub fn interpretar(entrada: &ChatInput, generado: &str, id_next: u32, pasante: bool) -> Result<AssistantTurn, ChatError> {
    let mut texto = String::new();
    // Con el pensamiento activo, tras un resultado de herramienta el aviso de generación ya abre el canal
    // (`<|channel>thought\n`), así que lo generado empieza **dentro** del pensamiento y lo cierra con `<channel|>`
    // sin haber escrito la apertura: todo lo anterior a ese cierre es razonamiento.
    let mut resto = generado;
    if let Some(c) = resto.find(CIERRA_CANAL) {
        if resto[..c].find(ABRE_CANAL).is_none() {
            resto = &resto[c + CIERRA_CANAL.len()..];
        }
    }
    let generado = resto;
    // Quitar canales de pensamiento; uno sin cerrar se lleva el resto (el modelo se quedó pensando).
    let mut resto = generado;
    loop {
        match resto.find(ABRE_CANAL) {
            Some(p) => {
                texto.push_str(&resto[..p]);
                match resto[p..].find(CIERRA_CANAL) {
                    Some(q) => resto = &resto[p + q + CIERRA_CANAL.len()..],
                    None => {
                        resto = "";
                        break;
                    }
                }
            }
            None => break,
        }
    }
    texto.push_str(resto);

    let mut llamada: Option<ToolCall> = None;
    let contenido_bruto = match texto.find(ABRE_LLAMADA) {
        Some(p) => {
            let cuerpo = &texto[p + ABRE_LLAMADA.len()..];
            let fin = cuerpo.find(CIERRA_LLAMADA).unwrap_or(cuerpo.len());
            let (nombre, args) = llamada_gemma4(&cuerpo[..fin]).ok_or_else(|| ChatError::ArgumentosInvalidos {
                llamada: format!("call_{id_next}"),
            })?;
            let json = serde_json::to_string(&Value::Object(args)).expect("json serializable").replace(": ", ":");
            llamada = Some(ToolCall::nueva(format!("call_{id_next}"), nombre, json));
            texto[..p].to_string()
        }
        None => texto,
    };
    let limpio = quitar_marcas(contenido_bruto.trim());
    let contenido = if limpio.is_empty() { None } else { Some(limpio.to_string()) };
    if let Some(c) = llamada.as_mut() {
        super::tools::ajustar_tipos(entrada, c);
    }
    match validate_assistant_turn(entrada, contenido.as_deref(), llamada.as_ref()) {
        Ok(()) => {}
        Err(ChatError::ArgumentoInvalido { .. } | ChatError::HerramientaDesconocida { .. }) if pasante => {}
        Err(e) => return Err(e),
    }
    Ok(AssistantTurn { content: contenido, tool_call: llamada })
}

/// Marcas de cierre de turno que el modelo a veces escribe como texto.
fn quitar_marcas(t: &str) -> &str {
    let mut t = t;
    loop {
        let antes = t;
        for m in ["<turn|>", "<eos>", "<|tool_response>"] {
            if let Some(r) = t.strip_suffix(m) {
                t = r.trim_end();
            }
        }
        if t == antes {
            return t;
        }
    }
}

/// `call:nombre{clave:valor,…}` → nombre y argumentos.
fn llamada_gemma4(cuerpo: &str) -> Option<(String, Map<String, Value>)> {
    let c = cuerpo.trim().strip_prefix("call:")?;
    let llave = c.find('{')?;
    let nombre = c[..llave].trim();
    if nombre.is_empty() {
        return None;
    }
    let mut lector = Lector { s: &c[llave..], pos: 0 };
    match lector.valor()? {
        Value::Object(m) => Some((nombre.to_string(), m)),
        _ => None,
    }
}

struct Lector<'a> {
    s: &'a str,
    pos: usize,
}

impl<'a> Lector<'a> {
    fn resto(&self) -> &'a str {
        &self.s[self.pos..]
    }

    fn espacios(&mut self) {
        let r = self.resto();
        self.pos += r.len() - r.trim_start().len();
    }

    fn consumir(&mut self, t: &str) -> bool {
        if self.resto().starts_with(t) {
            self.pos += t.len();
            true
        } else {
            false
        }
    }

    fn valor(&mut self) -> Option<Value> {
        self.espacios();
        if self.consumir(COMILLA) {
            let fin = self.resto().find(COMILLA)?;
            let s = self.resto()[..fin].to_string();
            self.pos += fin + COMILLA.len();
            return Some(Value::String(s));
        }
        if self.consumir("{") {
            let mut m = Map::new();
            loop {
                self.espacios();
                if self.consumir("}") {
                    return Some(Value::Object(m));
                }
                if self.consumir(",") {
                    continue;
                }
                let dos = self.resto().find(':')?;
                let clave = self.resto()[..dos].trim().trim_matches(|c| c == '"').to_string();
                // Las claves escapadas (`<|"|>k<|"|>`) también valen.
                let clave = clave.replace(COMILLA, "");
                self.pos += dos + 1;
                let v = self.valor()?;
                m.insert(clave, v);
            }
        }
        if self.consumir("[") {
            let mut a = Vec::new();
            loop {
                self.espacios();
                if self.consumir("]") {
                    return Some(Value::Array(a));
                }
                if self.consumir(",") {
                    continue;
                }
                a.push(self.valor()?);
            }
        }
        // Valor desnudo: hasta `,`, `}` o `]`.
        let r = self.resto();
        let fin = r.find(|c| c == ',' || c == '}' || c == ']').unwrap_or(r.len());
        let t = r[..fin].trim();
        if t.is_empty() {
            return None;
        }
        self.pos += fin;
        Some(match t {
            "true" => Value::Bool(true),
            "false" => Value::Bool(false),
            "null" => Value::Null,
            _ => serde_json::from_str::<Value>(t)
                .ok()
                .filter(Value::is_number)
                .unwrap_or_else(|| Value::String(t.to_string())),
        })
    }
}
