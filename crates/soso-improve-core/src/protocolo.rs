//! Verificador de los casos de protocolo.
//!
//! Se juzga sobre un **documento de respuesta**: lo que el lanzador grabó al
//! mandar la petición visible contra el servidor. Así el verificador no
//! necesita ni servidor ni modelo, y las mismas aserciones valen para el
//! ejemplo host (T13) y para el guest (T16).
//!
//! ```text
//! {"http_status": 200, "cuerpo": { ... }}                  respuesta completa
//! {"http_status": 200, "trozos_b64": ["ZGF0YT...", ...]}   SSE tal y como llegó
//! ```
//!
//! Los trozos son los bytes recibidos, en orden: el corte entre dos puede caer
//! **en medio de un carácter**, que es justo lo que hay que reensamblar antes
//! de decodificar. Se concatena, se decodifica UTF-8 y solo entonces se
//! interpretan los eventos.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{Error, Resultado};

/// Respuesta grabada, ya reensamblada.
pub struct Documento {
    pub estado: Option<i64>,
    pub cuerpo: Option<Value>,
    pub sse: Option<String>,
    pub eventos: Vec<Value>,
    pub errores_formato: Vec<String>,
}

fn decodificar_b64(texto: &str) -> Resultado<Vec<u8>> {
    // base64 estándar, sin dependencias: son cuatro líneas y evita arrastrar un
    // crate más al userspace de soso.
    let tabla = |c: u8| -> Option<u8> {
        match c {
            b'A'..=b'Z' => Some(c - b'A'),
            b'a'..=b'z' => Some(c - b'a' + 26),
            b'0'..=b'9' => Some(c - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    };
    let mut fuera = Vec::new();
    let mut acumulado: u32 = 0;
    let mut bits = 0;
    for c in texto.bytes() {
        if c == b'=' || c.is_ascii_whitespace() {
            continue;
        }
        let v = tabla(c).ok_or_else(|| Error::formato(format!("base64 inválido: {c:?}")))?;
        acumulado = (acumulado << 6) | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            fuera.push((acumulado >> bits) as u8);
        }
    }
    Ok(fuera)
}

impl Documento {
    pub fn nuevo(crudo: &Value) -> Resultado<Documento> {
        let estado = crudo.get("http_status").and_then(|v| v.as_i64());
        let cuerpo = crudo.get("cuerpo").cloned();
        let mut doc = Documento {
            estado,
            cuerpo,
            sse: None,
            eventos: Vec::new(),
            errores_formato: Vec::new(),
        };
        if let Some(trozos) = crudo.get("trozos_b64").and_then(|v| v.as_array()) {
            let mut datos = Vec::new();
            for t in trozos {
                let texto = t
                    .as_str()
                    .ok_or_else(|| Error::formato("trozos_b64 debe ser lista de cadenas"))?;
                datos.extend(decodificar_b64(texto)?);
            }
            match core::str::from_utf8(&datos) {
                Ok(texto) => doc.sse = Some(texto.to_string()),
                Err(e) => {
                    doc.errores_formato
                        .push(format!("el flujo SSE no es UTF-8 completo: {e}"));
                    doc.sse = Some(String::from_utf8_lossy(&datos).into_owned());
                }
            }
            doc.parsear_eventos();
        }
        Ok(doc)
    }

    fn parsear_eventos(&mut self) {
        let texto = match &self.sse {
            Some(t) => t.clone(),
            None => return,
        };
        for linea in texto.lines() {
            let linea = linea.trim_end_matches('\r');
            if !linea.starts_with("data:") {
                continue;
            }
            let carga = linea["data:".len()..].trim();
            if carga == "[DONE]" {
                continue;
            }
            match serde_json::from_str::<Value>(carga) {
                Ok(v) => self.eventos.push(v),
                Err(e) => self
                    .errores_formato
                    .push(format!("evento SSE ilegible: {e}")),
            }
        }
    }

    /// Mensaje del asistente, venga de una respuesta completa o de deltas.
    pub fn mensaje(&self) -> Option<Value> {
        if let Some(cuerpo) = &self.cuerpo {
            return cuerpo
                .get("choices")
                .and_then(|c| c.as_array())
                .and_then(|c| c.first())
                .and_then(|o| o.get("message"))
                .cloned();
        }
        let mut contenido = String::new();
        let mut nombre = String::new();
        let mut argumentos = String::new();
        let mut hay_llamada = false;
        for ev in &self.eventos {
            let opciones = match ev.get("choices").and_then(|c| c.as_array()) {
                Some(o) => o,
                None => continue,
            };
            for op in opciones {
                let delta = match op.get("delta") {
                    Some(d) => d,
                    None => continue,
                };
                if let Some(t) = delta.get("content").and_then(|c| c.as_str()) {
                    contenido.push_str(t);
                }
                if let Some(llamadas) = delta.get("tool_calls").and_then(|t| t.as_array()) {
                    for tc in llamadas {
                        hay_llamada = true;
                        if let Some(f) = tc.get("function") {
                            if let Some(n) = f.get("name").and_then(|n| n.as_str()) {
                                if !n.is_empty() {
                                    nombre = n.to_string();
                                }
                            }
                            if let Some(a) = f.get("arguments").and_then(|a| a.as_str()) {
                                argumentos.push_str(a);
                            }
                        }
                    }
                }
            }
        }
        let mut mensaje = serde_json::Map::new();
        mensaje.insert("role".to_string(), Value::String("assistant".to_string()));
        mensaje.insert(
            "content".to_string(),
            if contenido.is_empty() {
                Value::Null
            } else {
                Value::String(contenido)
            },
        );
        if hay_llamada {
            let mut funcion = serde_json::Map::new();
            funcion.insert("name".to_string(), Value::String(nombre));
            funcion.insert("arguments".to_string(), Value::String(argumentos));
            let mut llamada = serde_json::Map::new();
            llamada.insert("id".to_string(), Value::String("delta-0".to_string()));
            llamada.insert("type".to_string(), Value::String("function".to_string()));
            llamada.insert("function".to_string(), Value::Object(funcion));
            mensaje.insert("tool_calls".to_string(), Value::Array(alloc::vec![Value::Object(llamada)]));
        }
        Some(Value::Object(mensaje))
    }

    pub fn texto(&self) -> String {
        self.mensaje()
            .and_then(|m| m.get("content").and_then(|c| c.as_str()).map(String::from))
            .unwrap_or_default()
    }

    pub fn llamadas(&self) -> Vec<Value> {
        self.mensaje()
            .and_then(|m| m.get("tool_calls").and_then(|t| t.as_array()).cloned())
            .unwrap_or_default()
    }

    pub fn finish_reason(&self) -> Option<String> {
        if let Some(cuerpo) = &self.cuerpo {
            return cuerpo
                .get("choices")
                .and_then(|c| c.as_array())
                .and_then(|c| c.first())
                .and_then(|o| o.get("finish_reason"))
                .and_then(|f| f.as_str())
                .map(String::from);
        }
        for ev in self.eventos.iter().rev() {
            if let Some(opciones) = ev.get("choices").and_then(|c| c.as_array()) {
                for op in opciones {
                    if let Some(f) = op.get("finish_reason").and_then(|f| f.as_str()) {
                        return Some(f.to_string());
                    }
                }
            }
        }
        None
    }

    pub fn usage(&self) -> Option<&Value> {
        self.cuerpo.as_ref().and_then(|c| c.get("usage"))
    }

    pub fn error(&self) -> Option<&Value> {
        self.cuerpo.as_ref().and_then(|c| c.get("error"))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Esperado {
    pub aserciones: Vec<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResultadoAsercion {
    pub tipo: String,
    pub paso: bool,
    pub motivo: Option<String>,
}

fn cadena(a: &Value, campo: &str) -> Resultado<String> {
    a.get(campo)
        .and_then(|v| v.as_str())
        .map(String::from)
        .ok_or_else(|| Error::formato(format!("la aserción necesita el campo {campo}")))
}

/// Evalúa una aserción. `None` = pasa; `Some(motivo)` = falla.
fn evaluar(doc: &Documento, a: &Value) -> Resultado<Option<String>> {
    let tipo = cadena(a, "tipo")?;
    let fallo = match tipo.as_str() {
        "estado_http" => {
            let esperado = a.get("valor").and_then(|v| v.as_i64());
            if doc.estado != esperado {
                Some(format!(
                    "estado HTTP {:?}, se esperaba {:?}",
                    doc.estado, esperado
                ))
            } else {
                None
            }
        }
        "contenido_no_vacio" => {
            if doc.texto().trim().is_empty() {
                Some("el asistente no devolvió texto".to_string())
            } else {
                None
            }
        }
        "contenido_igual" => {
            let valor = cadena(a, "valor")?;
            let obtenido = doc.texto();
            if obtenido != valor {
                Some(format!("contenido {obtenido:?}, se esperaba {valor:?}"))
            } else {
                None
            }
        }
        "contenido_contiene" => {
            let valor = cadena(a, "valor")?;
            let obtenido = doc.texto();
            let ignorar = a
                .get("ignorar_mayusculas")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            let (aguja, pajar) = if ignorar {
                (valor.to_lowercase(), obtenido.to_lowercase())
            } else {
                (valor.clone(), obtenido.clone())
            };
            if !pajar.contains(&aguja) {
                Some(format!("el contenido no incluye {valor:?}: {obtenido:?}"))
            } else {
                None
            }
        }
        "sin_llamadas" => {
            let n = doc.llamadas().len();
            if n > 0 {
                Some(format!("se esperaba texto y hay {n} llamada(s)"))
            } else {
                None
            }
        }
        "llamada_unica" => {
            let llamadas = doc.llamadas();
            if llamadas.len() != 1 {
                Some(format!(
                    "se esperaba exactamente una llamada y hay {}",
                    llamadas.len()
                ))
            } else {
                let nombre = cadena(a, "nombre")?;
                let funcion = llamadas[0].get("function");
                let real = funcion
                    .and_then(|f| f.get("name"))
                    .and_then(|n| n.as_str())
                    .unwrap_or("");
                if real != nombre {
                    Some(format!("llamada a {real:?}, se esperaba {nombre:?}"))
                } else {
                    let crudo = funcion
                        .and_then(|f| f.get("arguments"))
                        .and_then(|x| x.as_str());
                    match crudo {
                        None => Some(
                            "los argumentos deben venir serializados como cadena JSON".to_string(),
                        ),
                        Some(texto) => match serde_json::from_str::<Value>(texto) {
                            Err(e) => Some(format!(
                                "los argumentos no son JSON completo ({e}): {texto:?}"
                            )),
                            Ok(v) if !v.is_object() => {
                                Some("los argumentos deben ser un objeto JSON".to_string())
                            }
                            Ok(v) => {
                                let faltan: Vec<String> = a
                                    .get("claves_argumentos")
                                    .and_then(|c| c.as_array())
                                    .map(|c| {
                                        c.iter()
                                            .filter_map(|k| k.as_str())
                                            .filter(|k| v.get(*k).is_none())
                                            .map(String::from)
                                            .collect()
                                    })
                                    .unwrap_or_default();
                                if faltan.is_empty() {
                                    None
                                } else {
                                    Some(format!("a los argumentos les faltan claves {faltan:?}"))
                                }
                            }
                        },
                    }
                }
            }
        }
        "finish_reason" => {
            let valor = cadena(a, "valor")?;
            let obtenido = doc.finish_reason();
            if obtenido.as_deref() != Some(valor.as_str()) {
                Some(format!(
                    "finish_reason {obtenido:?}, se esperaba {valor:?}"
                ))
            } else {
                None
            }
        }
        "usage_coherente" => match doc.usage() {
            None => Some("la respuesta no trae usage".to_string()),
            Some(u) => {
                let leer = |k: &str| u.get(k).and_then(|v| v.as_i64());
                match (
                    leer("prompt_tokens"),
                    leer("completion_tokens"),
                    leer("total_tokens"),
                ) {
                    (Some(p), Some(c), Some(t)) => {
                        if p < 0 || c < 0 || t < 0 {
                            Some(format!("usage con valores negativos: {u}"))
                        } else if t != p + c {
                            Some(format!("usage incoherente: {t} != {p} + {c}"))
                        } else {
                            None
                        }
                    }
                    _ => Some(format!("usage incompleto: {u}")),
                }
            }
        },
        "error_presente" => match doc.error() {
            None => Some("se esperaba un objeto error en el cuerpo".to_string()),
            Some(e) => {
                let mensaje = e.get("message").and_then(|m| m.as_str()).unwrap_or("");
                if mensaje.is_empty() {
                    Some("el error no trae message".to_string())
                } else {
                    let mut fallo = None;
                    if let Some(esperado) = a.get("tipo_error").and_then(|v| v.as_str()) {
                        let real = e.get("type").and_then(|t| t.as_str()).unwrap_or("");
                        if real != esperado {
                            fallo = Some(format!(
                                "error.type {real:?}, se esperaba {esperado:?}"
                            ));
                        }
                    }
                    if fallo.is_none() {
                        if let Some(esperado) = a.get("codigo").and_then(|v| v.as_str()) {
                            let real = e.get("code").and_then(|t| t.as_str()).unwrap_or("");
                            if real != esperado {
                                fallo = Some(format!(
                                    "error.code {real:?}, se esperaba {esperado:?}"
                                ));
                            }
                        }
                    }
                    fallo
                }
            }
        },
        "sin_texto_de_asistente" => {
            let con_choices = doc
                .cuerpo
                .as_ref()
                .and_then(|c| c.get("choices"))
                .and_then(|c| c.as_array())
                .map(|c| !c.is_empty())
                .unwrap_or(false);
            if con_choices {
                Some("un error no debe traer texto del asistente".to_string())
            } else if !doc.eventos.is_empty() {
                Some("un error antes de las cabeceras no debe abrir un flujo SSE".to_string())
            } else {
                None
            }
        }
        "flujo_utf8_completo" => {
            if doc.sse.is_none() {
                Some("el caso exige un documento con trozos_b64".to_string())
            } else if !doc.errores_formato.is_empty() {
                Some(doc.errores_formato.join("; "))
            } else if doc.texto().contains('\u{fffd}') {
                Some("el texto reensamblado tiene caracteres de reemplazo (U+FFFD)".to_string())
            } else {
                None
            }
        }
        otro => return Err(Error::formato(format!("aserción desconocida: {otro}"))),
    };
    Ok(fallo)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Informe {
    pub caso: String,
    pub estado: String,
    pub pasadas: usize,
    pub total: usize,
    pub aserciones: Vec<ResultadoAsercion>,
}

/// Juzga un documento contra las aserciones reservadas del caso.
pub fn verificar(caso: &str, esperado: &Esperado, respuesta: &Value) -> Resultado<Informe> {
    let doc = Documento::nuevo(respuesta)?;
    let mut aserciones = Vec::new();
    for a in &esperado.aserciones {
        let motivo = evaluar(&doc, a)?;
        aserciones.push(ResultadoAsercion {
            tipo: a
                .get("tipo")
                .and_then(|t| t.as_str())
                .unwrap_or("?")
                .to_string(),
            paso: motivo.is_none(),
            motivo,
        });
    }
    let pasadas = aserciones.iter().filter(|a| a.paso).count();
    Ok(Informe {
        caso: caso.to_string(),
        estado: if pasadas == aserciones.len() {
            "ok".to_string()
        } else {
            "fallo".to_string()
        },
        pasadas,
        total: aserciones.len(),
        aserciones,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_con_corte_en_medio_de_un_caracter() {
        // "cafÉ" partido: el segundo trozo empieza a mitad del carácter.
        let texto = "café ☕";
        let bytes = texto.as_bytes();
        let corte = bytes.len() - 2;
        let uno = base64(&bytes[..corte]);
        let dos = base64(&bytes[corte..]);
        let juntos = [decodificar_b64(&uno).unwrap(), decodificar_b64(&dos).unwrap()].concat();
        assert_eq!(core::str::from_utf8(&juntos).unwrap(), texto);
    }

    fn base64(datos: &[u8]) -> String {
        const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut fuera = String::new();
        for trozo in datos.chunks(3) {
            let b = [
                trozo[0],
                *trozo.get(1).unwrap_or(&0),
                *trozo.get(2).unwrap_or(&0),
            ];
            let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
            fuera.push(T[(n >> 18) as usize & 63] as char);
            fuera.push(T[(n >> 12) as usize & 63] as char);
            fuera.push(if trozo.len() > 1 {
                T[(n >> 6) as usize & 63] as char
            } else {
                '='
            });
            fuera.push(if trozo.len() > 2 {
                T[n as usize & 63] as char
            } else {
                '='
            });
        }
        fuera
    }
}
