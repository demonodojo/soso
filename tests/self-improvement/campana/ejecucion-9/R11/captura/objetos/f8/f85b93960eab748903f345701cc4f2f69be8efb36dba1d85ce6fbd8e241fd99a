//! Entidades HTML, charset y descompresión gzip.

use alloc::string::String;
use alloc::vec::Vec;
use miniz_oxide::inflate::decompress_to_vec_zlib;

/// Descomprime el cuerpo si `Content-Encoding` indica gzip/deflate.
pub fn maybe_decompress(body: &[u8], content_encoding: Option<&str>) -> Result<Vec<u8>, ()> {
    let enc = content_encoding.unwrap_or("").trim().to_ascii_lowercase();
    if enc.is_empty() || enc == "identity" {
        return Ok(body.to_vec());
    }
    if enc.contains("gzip") || enc.contains("deflate") {
        decompress_to_vec_zlib(body).map_err(|_| ())
    } else {
        Ok(body.to_vec())
    }
}

/// Convierte bytes HTML a `String` (UTF-8 con fallback Latin-1).
pub fn bytes_to_text(body: &[u8], charset: Option<&str>) -> String {
    let cs = charset.unwrap_or("utf-8").trim().to_ascii_lowercase();
    if cs.contains("utf-8") || cs.contains("utf8") {
        if let Ok(s) = core::str::from_utf8(body) {
            return String::from(s);
        }
    }
    body.iter().map(|&b| b as char).collect()
}

/// Resuelve entidades HTML básicas en texto.
pub fn decode_entities(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'&' {
            if let Some((ch, consumed)) = parse_entity_char(&bytes[i..]) {
                if let Some(c) = ch {
                    out.push(c);
                }
                i += consumed;
                continue;
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

fn parse_entity_char(rest: &[u8]) -> Option<(Option<char>, usize)> {
    let end = rest.iter().position(|&b| b == b';')?;
    let raw = core::str::from_utf8(&rest[1..end]).ok()?;
    let total = end + 1;
    match raw {
        "amp" => Some((Some('&'), total)),
        "lt" => Some((Some('<'), total)),
        "gt" => Some((Some('>'), total)),
        "quot" => Some((Some('"'), total)),
        "apos" => Some((Some('\''), total)),
        "nbsp" => Some((Some('\u{00a0}'), total)),
        _ if raw.starts_with("#x") || raw.starts_with("#X") => {
            let n = u32::from_str_radix(&raw[2..], 16).ok()?;
            Some((char::from_u32(n), total))
        }
        _ if raw.starts_with('#') => {
            let n = raw[1..].parse::<u32>().ok()?;
            Some((char::from_u32(n), total))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entidades_basicas() {
        assert_eq!(decode_entities("a&amp;b"), "a&b");
        assert_eq!(decode_entities("&lt;p&gt;"), "<p>");
        assert_eq!(decode_entities("&#65;"), "A");
        assert_eq!(decode_entities("&#x41;"), "A");
    }

    #[test]
    fn utf8_y_latin1() {
        let utf8 = "hola é".as_bytes();
        assert_eq!(bytes_to_text(utf8, Some("utf-8")), "hola é");
        assert_eq!(bytes_to_text(&[0xE9], Some("iso-8859-1")), "é");
    }
}
