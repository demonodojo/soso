//! Interpretación de la salida del modelo distinguiendo marcadores **emitidos como token** de los
//! escritos con piezas corrientes.
//!
//! Los parsers de `tools.rs` y `gemma4.rs` trabajan sobre texto: buscan `</think>`, `<tool_call>`,
//! `<turn|>`… con `find`/`rfind`. Un argumento que cite esas cadenas (el modelo edita un fichero que las
//! contiene) parte la llamada o corta el razonamiento donde no toca. El tokenizer sabe qué tramos del texto
//! salieron de un token especial; aquí se usa para que, **si el modelo emite un marcador como token**, las
//! apariciones del mismo marcador escritas como texto no cuenten.
//!
//! Se hace neutralizando esas apariciones antes de analizar (se les intercala [`NEUTRO`] tras el primer
//! carácter) y quitando la marca de lo que sale del parser. Un marcador que el modelo sólo escribe como texto
//! (modelos que no lo tienen como token en su salida) se analiza como siempre.

use alloc::string::String;
use alloc::vec::Vec;
use core::ops::Range;

use super::{AssistantTurn, ChatError};
use crate::tokenizer::Tokenizer;

/// Carácter de uso privado que parte un marcador escrito como texto. Se elimina del resultado.
pub const NEUTRO: char = '\u{E000}';

/// Decodifica `ids` y analiza el texto con `parse`, ignorando los marcadores escritos como texto cuando el
/// mismo marcador aparece también como token especial.
pub fn interpretar_ids<F>(tokenizer: &Tokenizer, ids: &[u32], parse: F) -> Result<AssistantTurn, ChatError>
where
    F: FnOnce(&str) -> Result<AssistantTurn, ChatError>,
{
    let (texto, marcados) = tokenizer.decode_marcado(ids);
    if marcados.is_empty() {
        return parse(&texto);
    }
    let especiales = tokenizer.textos_especiales();
    let (protegido, hubo) = neutralizar(&texto, &marcados, &especiales);
    if !hubo {
        return parse(&texto);
    }
    parse(&protegido).map(restaurar)
}

/// Intercala [`NEUTRO`] en las apariciones **como texto** de los marcadores que el modelo también emitió
/// como token. Devuelve el texto resultante y si cambió algo.
pub fn neutralizar(texto: &str, marcados: &[Range<usize>], especiales: &[String]) -> (String, bool) {
    let mut cortes: Vec<usize> = Vec::new();
    for m in especiales {
        if m.is_empty() {
            continue;
        }
        let emitido = marcados.iter().any(|r| &texto[r.clone()] == m.as_str());
        if !emitido {
            continue;
        }
        let primer = m.chars().next().map(char::len_utf8).unwrap_or(1);
        for (pos, _) in texto.match_indices(m.as_str()) {
            if marcados.iter().any(|r| r.start == pos) {
                continue;
            }
            cortes.push(pos + primer);
        }
    }
    if cortes.is_empty() {
        return (String::from(texto), false);
    }
    cortes.sort_unstable();
    cortes.dedup();
    let mut out = String::with_capacity(texto.len() + cortes.len() * NEUTRO.len_utf8());
    let mut desde = 0;
    for c in cortes {
        out.push_str(&texto[desde..c]);
        out.push(NEUTRO);
        desde = c;
    }
    out.push_str(&texto[desde..]);
    (out, true)
}

/// Quita [`NEUTRO`] de todo lo que devolvió el parser.
pub fn restaurar(mut turno: AssistantTurn) -> AssistantTurn {
    let limpia = |s: &mut String| {
        if s.contains(NEUTRO) {
            s.retain(|c| c != NEUTRO);
        }
    };
    if let Some(c) = turno.content.as_mut() {
        limpia(c);
    }
    if let Some(t) = turno.tool_call.as_mut() {
        limpia(&mut t.name);
        limpia(&mut t.arguments);
    }
    turno
}
