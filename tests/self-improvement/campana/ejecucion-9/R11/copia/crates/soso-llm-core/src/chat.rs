//! Plantillas de chat: de una pregunta a la secuencia de tokens que el modelo
//! espera ver.
//!
//! Un modelo *chat* no se ha entrenado con texto suelto, sino con turnos
//! marcados. A TinyLlama-1.1B-Chat hay que darle esto:
//!
//! ```text
//! <|user|>
//! hola</s>
//! <|assistant|>
//! ```
//!
//! Sin los marcadores no ve una conversación: ve un fragmento de corpus y lo
//! continúa, que es de dónde salía la ensalada de palabras de `ask hola` en la
//! placa.
//!
//! Y el `</s>` de ahí arriba **es el token EOS**, no los cuatro caracteres: como
//! texto son cuatro piezas (`<`, `/`, `s`, `>`) que el modelo nunca vio como
//! separador de turno. Por eso esto renderiza a **tokens** y no a una cadena: hay
//! que poder meter un id concreto en medio.
//!
//! La plantilla se escribe con dos marcadores:
//!
//! - `{prompt}` — la pregunta
//! - `{eos}` — el token de fin del modelo
//!
//! Lo demás va literal. Un `{algo}` que no sea de los dos se queda tal cual, sin
//! error: es texto de la plantilla y no nos toca juzgarlo.

use alloc::string::String;
use alloc::vec::Vec;

use crate::tokenizer::Tokenizer;

pub const MARCA_PROMPT: &str = "{prompt}";
pub const MARCA_EOS: &str = "{eos}";

/// Plantilla vacía = sin plantilla, o sea el texto tal cual.
pub fn render(plantilla: &str, pregunta: &str, tok: &Tokenizer) -> Vec<u32> {
    if plantilla.is_empty() {
        return tok.encode(pregunta);
    }
    let mut out = Vec::new();
    // BOS una sola vez y al principio, que es lo que hace SentencePiece con el
    // texto entero. Los trozos van luego como continuación: repetir el BOS en
    // cada segmento le pinta al modelo tres comienzos de documento.
    if let Some(b) = tok.bos() {
        out.push(b);
    }
    // El `▁` de cortesía también es del principio del texto y de nadie más;
    // añadirlo en cada trozo siembra espacios donde la plantilla no los tiene.
    let mut primer_texto = true;
    let mut empujar = |out: &mut Vec<u32>, txt: &str| {
        if txt.is_empty() {
            return;
        }
        out.extend(tok.encode_trozo(txt, false, primer_texto));
        primer_texto = false;
    };

    let mut resto = plantilla;
    while !resto.is_empty() {
        let marca = match (resto.find(MARCA_PROMPT), resto.find(MARCA_EOS)) {
            (Some(p), Some(e)) if p <= e => Some((p, MARCA_PROMPT)),
            (Some(_), Some(e)) => Some((e, MARCA_EOS)),
            (Some(p), None) => Some((p, MARCA_PROMPT)),
            (None, Some(e)) => Some((e, MARCA_EOS)),
            (None, None) => None,
        };
        let Some((pos, marca)) = marca else {
            empujar(&mut out, resto);
            break;
        };
        empujar(&mut out, &resto[..pos]);
        if marca == MARCA_PROMPT {
            empujar(&mut out, pregunta);
        } else if let Some(eos) = tok.eos() {
            out.push(eos);
        }
        resto = &resto[pos + marca.len()..];
    }
    out
}

/// `\n`, `\t` y `\\` de un valor de fichero de configuración.
///
/// Las plantillas llevan saltos de línea y `/etc/llm.conf` es de `clave=valor`
/// por línea, así que el fichero guarda `\n` y aquí se deshace. Una barra
/// seguida de otra cosa se deja como está: es texto de la plantilla, no un
/// escape roto.
pub fn desescapar(valor: &str) -> String {
    let mut out = String::with_capacity(valor.len());
    let mut chars = valor.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('\\') => out.push('\\'),
            Some(otro) => {
                out.push('\\');
                out.push(otro);
            }
            None => out.push('\\'),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tokenizer::VocabTokenizer;
    use alloc::string::String;
    use alloc::vec;

    /// Vocabulario de juguete: bos=0, eos=1, y piezas para armar la plantilla de
    /// TinyLlama sin arrastrar las 32000 de verdad.
    fn toy() -> Tokenizer {
        let pieces: Vec<String> = [
            "<s>", "</s>", "\u{2581}hola", "hola", "<|user|>", "<|assistant|>", "\n",
            "\u{2581}<|user|>",
        ]
        .iter()
        .map(|s| String::from(*s))
        .collect();
        Tokenizer::Vocab(VocabTokenizer::new(pieces, 0, 1))
    }

    const PLANTILLA: &str = "<|user|>\n{prompt}{eos}\n<|assistant|>\n";

    #[test]
    fn plantilla_vacia_es_el_texto_tal_cual() {
        let tok = toy();
        assert_eq!(render("", "hola", &tok), tok.encode("hola"));
    }

    #[test]
    fn bos_una_vez_y_al_principio() {
        let tok = toy();
        let ids = render(PLANTILLA, "hola", &tok);
        assert_eq!(ids[0], 0, "el primer token tiene que ser el bos");
        assert_eq!(ids.iter().filter(|&&t| t == 0).count(), 1);
    }

    #[test]
    fn eos_es_el_token_y_no_sus_cuatro_letras() {
        let tok = toy();
        let ids = render(PLANTILLA, "hola", &tok);
        // El id 1 (eos) aparece EXACTAMENTE una vez, donde iba `{eos}`, y detrás
        // sigue habiendo plantilla: si se hubiera codificado como texto no
        // estaría el id y sí cuatro piezas más.
        let pos = ids.iter().position(|&t| t == 1).expect("falta el eos");
        assert!(pos > 1 && pos < ids.len() - 1);
        assert_eq!(ids.iter().filter(|&&t| t == 1).count(), 1);
    }

    #[test]
    fn el_prompt_va_sin_prefijo_de_espacio() {
        let tok = toy();
        let ids = render(PLANTILLA, "hola", &tok);
        // 3 = "hola" pelado; 2 = "▁hola". Dentro de la plantilla la pregunta
        // sigue a un `\n`, así que no lleva espacio delante.
        assert!(ids.contains(&3), "esperaba «hola» sin `▁`: {ids:?}");
        assert!(!ids.contains(&2), "se coló un `▁hola`: {ids:?}");
    }

    #[test]
    fn marcador_desconocido_va_literal() {
        let tok = toy();
        // `{otro}` no es marcador: no debe desaparecer ni dar error. Se compara
        // contra la misma plantilla con el texto ya puesto a mano.
        let con_marca = render("<|user|>\n{otro}{prompt}", "hola", &tok);
        let a_mano = render("<|user|>\n{otro}hola", "", &tok);
        assert_eq!(con_marca, a_mano);
    }

    #[test]
    fn byte_level_no_mete_bos_de_la_nada() {
        let tok = Tokenizer::byte_level();
        // No tiene bos; la plantilla se codifica byte a byte (y por eso quien
        // aplica plantillas pregunta antes por `tiene_vocabulario`).
        let ids = render("[{prompt}]", "A", &tok);
        assert_eq!(ids, vec![b'[' as u32, b'A' as u32, b']' as u32]);
    }

    #[test]
    fn desescapar_saltos_y_barras() {
        assert_eq!(desescapar("a\\nb"), "a\nb");
        assert_eq!(desescapar("a\\tb"), "a\tb");
        assert_eq!(desescapar("a\\\\b"), "a\\b");
        assert_eq!(desescapar("a\\qb"), "a\\qb");
        assert_eq!(desescapar("cola\\"), "cola\\");
    }
}
