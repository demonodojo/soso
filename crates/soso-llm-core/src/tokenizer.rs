//! Tokenizers: byte-level (fallback, vocab 256) y vocabulario tipo
//! SentencePiece cargado de `tokenizer.som` (piezas con `▁` como espacio y
//! byte-fallback `<0xXX>`).

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use sosomodel::{pack_som, parse_som, Reader, CACHE_ALIGN};

/// Id reservado para "no hay token" (bos/eos ausentes).
pub const NO_TOKEN: u32 = u32::MAX;

pub enum Tokenizer {
    ByteLevel,
    Vocab(VocabTokenizer),
}

pub struct VocabTokenizer {
    pieces: Vec<String>,
    lookup: BTreeMap<String, u32>,
    max_piece_len: usize,
    pub bos: u32,
    pub eos: u32,
}

impl Tokenizer {
    pub fn byte_level() -> Self {
        Tokenizer::ByteLevel
    }

    /// Token de parada para el bucle de generación.
    pub fn eos(&self) -> Option<u32> {
        match self {
            Tokenizer::ByteLevel => Some(0),
            Tokenizer::Vocab(v) => (v.eos != NO_TOKEN).then_some(v.eos),
        }
    }

    /// Token de comienzo, si el vocabulario tiene uno. El byte-level no tiene:
    /// sus tokens son bytes y no hay ninguno reservado.
    pub fn bos(&self) -> Option<u32> {
        match self {
            Tokenizer::ByteLevel => None,
            Tokenizer::Vocab(v) => (v.bos != NO_TOKEN).then_some(v.bos),
        }
    }

    pub fn encode(&self, text: &str) -> Vec<u32> {
        self.encode_trozo(text, true, true)
    }

    /// Codifica un TROZO de un texto mayor.
    ///
    /// `bos` y `prefijo` son lo que distingue el principio del prompt de lo que
    /// va detrás: SentencePiece pone el BOS y el `▁` de cortesía **una vez**, al
    /// empezar el texto, no en cada pedazo. Sin poder apagarlos, una plantilla de
    /// chat armada por segmentos repetía el BOS en cada uno y sembraba espacios
    /// que el modelo no vio al entrenar.
    pub fn encode_trozo(&self, text: &str, bos: bool, prefijo: bool) -> Vec<u32> {
        match self {
            Tokenizer::ByteLevel => text.bytes().map(|b| b as u32).collect(),
            Tokenizer::Vocab(v) => v.encode(text, bos, prefijo),
        }
    }

    /// `true` si el modelo trae su propio vocabulario (`tokenizer.som`).
    ///
    /// Con el byte-level de reserva no hay tokens de verdad —`<|user|>` son nueve
    /// bytes de ruido— así que quien vaya a aplicar una plantilla de chat debe
    /// preguntar esto antes.
    pub fn tiene_vocabulario(&self) -> bool {
        matches!(self, Tokenizer::Vocab(_))
    }

    pub fn decode(&self, tokens: &[u32]) -> String {
        match self {
            Tokenizer::ByteLevel => {
                let bytes: Vec<u8> = tokens.iter().map(|&t| t as u8).collect();
                String::from_utf8_lossy(&bytes).into_owned()
            }
            Tokenizer::Vocab(v) => v.decode(tokens),
        }
    }

    /// Bytes crudos de un token (para el decode incremental). bos/eos no
    /// emiten nada.
    pub fn token_bytes(&self, t: u32, out: &mut Vec<u8>) {
        match self {
            Tokenizer::ByteLevel => out.push(t as u8),
            Tokenizer::Vocab(v) => v.token_bytes(t, out),
        }
    }

    /// Parsea un `tokenizer.som`.
    pub fn parse(data: &[u8]) -> Result<Self, ()> {
        Ok(Tokenizer::Vocab(VocabTokenizer::parse(data)?))
    }
}

impl VocabTokenizer {
    pub fn new(pieces: Vec<String>, bos: u32, eos: u32) -> Self {
        let mut lookup = BTreeMap::new();
        let mut max_piece_len = 1;
        for (i, p) in pieces.iter().enumerate() {
            max_piece_len = max_piece_len.max(p.len());
            lookup.entry(p.clone()).or_insert(i as u32);
        }
        Self {
            pieces,
            lookup,
            max_piece_len,
            bos,
            eos,
        }
    }

    /// Cuerpo: bos u32 | eos u32 | count u32 | (len u32 | bytes)*
    pub fn serialize(pieces: &[String], bos: u32, eos: u32) -> Vec<u8> {
        let mut body = Vec::new();
        body.extend_from_slice(&bos.to_le_bytes());
        body.extend_from_slice(&eos.to_le_bytes());
        body.extend_from_slice(&(pieces.len() as u32).to_le_bytes());
        for p in pieces {
            body.extend_from_slice(&(p.len() as u32).to_le_bytes());
            body.extend_from_slice(p.as_bytes());
        }
        pack_som(&body, 1, CACHE_ALIGN)
    }

    pub fn parse(data: &[u8]) -> Result<Self, ()> {
        let (_version, body) = parse_som(data)?;
        let mut r = Reader::new(body);
        let bos = r.u32()?;
        let eos = r.u32()?;
        let count = r.u32()? as usize;
        let mut pieces = Vec::new();
        for _ in 0..count {
            let len = r.u32()? as usize;
            let bytes = r.take(len)?;
            pieces.push(String::from(
                core::str::from_utf8(bytes).map_err(|_| ())?,
            ));
        }
        Ok(Self::new(pieces, bos, eos))
    }

    /// Greedy longest-match sobre el texto normalizado (espacios → `▁`,
    /// `▁` inicial). Caracteres sin pieza caen al byte-fallback `<0xXX>`.
    fn encode(&self, text: &str, bos: bool, prefijo: bool) -> Vec<u32> {
        let mut out = Vec::new();
        if bos && self.bos != NO_TOKEN {
            out.push(self.bos);
        }
        let normalized = if prefijo {
            format!("\u{2581}{}", text.replace(' ', "\u{2581}"))
        } else {
            text.replace(' ', "\u{2581}")
        };
        let s = normalized.as_str();
        let mut i = 0;
        while i < s.len() {
            let mut matched = false;
            let max_len = self.max_piece_len.min(s.len() - i);
            for l in (1..=max_len).rev() {
                if !s.is_char_boundary(i + l) {
                    continue;
                }
                if let Some(&id) = self.lookup.get(&s[i..i + l]) {
                    out.push(id);
                    i += l;
                    matched = true;
                    break;
                }
            }
            if !matched {
                // byte-fallback del carácter completo
                let ch_len = s[i..].chars().next().map(|c| c.len_utf8()).unwrap_or(1);
                for b in s[i..i + ch_len].bytes() {
                    if let Some(&id) = self.lookup.get(&format!("<0x{b:02X}>")) {
                        out.push(id);
                    }
                }
                i += ch_len;
            }
        }
        out
    }

    fn token_bytes(&self, t: u32, bytes: &mut Vec<u8>) {
        if t == self.bos || t == self.eos {
            return;
        }
        let Some(piece) = self.pieces.get(t as usize) else {
            return;
        };
        if let Some(b) = parse_byte_piece(piece) {
            bytes.push(b);
        } else {
            for ch in piece.chars() {
                if ch == '\u{2581}' {
                    bytes.push(b' ');
                } else {
                    let mut buf = [0u8; 4];
                    bytes.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                }
            }
        }
    }

    fn decode(&self, tokens: &[u32]) -> String {
        let mut bytes: Vec<u8> = Vec::new();
        for &t in tokens {
            self.token_bytes(t, &mut bytes);
        }
        let text = String::from_utf8_lossy(&bytes).into_owned();
        String::from(text.strip_prefix(' ').unwrap_or(&text))
    }
}

/// Decoder incremental para streaming: acumula bytes de tokens y emite solo
/// prefijos UTF-8 completos (un carácter multibyte puede cruzar tokens).
pub struct StreamDecoder {
    pending: Vec<u8>,
}

impl StreamDecoder {
    pub fn new() -> Self {
        Self {
            pending: Vec::new(),
        }
    }

    /// Añade un token y devuelve el texto listo para imprimir.
    pub fn push(&mut self, tok: &Tokenizer, t: u32) -> String {
        tok.token_bytes(t, &mut self.pending);
        match core::str::from_utf8(&self.pending) {
            Ok(s) => {
                let out = String::from(s);
                self.pending.clear();
                out
            }
            Err(e) => {
                let n = e.valid_up_to();
                let out = String::from_utf8_lossy(&self.pending[..n]).into_owned();
                self.pending.drain(..n);
                // secuencia inválida (no solo incompleta): descartarla para
                // no acumular basura indefinidamente
                if self.pending.len() > 4 {
                    self.pending.clear();
                }
                out
            }
        }
    }

    /// Restos al terminar (secuencia incompleta → lossy).
    pub fn finish(&mut self) -> String {
        let out = String::from_utf8_lossy(&self.pending).into_owned();
        self.pending.clear();
        out
    }
}

impl Default for StreamDecoder {
    fn default() -> Self {
        Self::new()
    }
}

/// Reconoce piezas byte-fallback `<0xXX>`.
fn parse_byte_piece(piece: &str) -> Option<u8> {
    let hex = piece.strip_prefix("<0x")?.strip_suffix('>')?;
    if hex.len() != 2 {
        return None;
    }
    u8::from_str_radix(hex, 16).ok()
}

#[cfg(test)]
#[cfg(feature = "std")]
mod tests {
    use super::*;

    fn vocab_toy() -> Tokenizer {
        let pieces: Vec<String> = [
            "<s>", "</s>", "\u{2581}hola", "\u{2581}mundo", "\u{2581}", "ho", "la", "<0xC3>",
            "<0xB1>",
        ]
        .iter()
        .map(|s| String::from(*s))
        .collect();
        Tokenizer::Vocab(VocabTokenizer::new(pieces, 0, 1))
    }

    #[test]
    fn encode_greedy_y_decode() {
        let tok = vocab_toy();
        let ids = tok.encode("hola mundo");
        // bos + ▁hola + ▁mundo
        assert_eq!(ids, vec![0, 2, 3]);
        assert_eq!(tok.decode(&ids), "hola mundo");
    }

    #[test]
    fn byte_fallback_utf8() {
        let tok = vocab_toy();
        // 'ñ' (C3 B1) no tiene pieza directa → byte-fallback
        let ids = tok.encode("ñ");
        assert_eq!(tok.decode(&ids), "ñ");
    }

    #[test]
    fn roundtrip_som() {
        let pieces: Vec<String> = ["<s>", "</s>", "\u{2581}hola"]
            .iter()
            .map(|s| String::from(*s))
            .collect();
        let data = VocabTokenizer::serialize(&pieces, 0, 1);
        let parsed = VocabTokenizer::parse(&data).unwrap();
        assert_eq!(parsed.pieces.len(), 3);
        assert_eq!(parsed.bos, 0);
        assert_eq!(parsed.eos, 1);
    }

    #[test]
    fn byte_level_decode_utf8() {
        let tok = Tokenizer::byte_level();
        let ids = tok.encode("año");
        assert_eq!(tok.decode(&ids), "año");
    }

    #[test]
    fn stream_decoder_multibyte_entre_tokens() {
        // byte-level: 'ñ' son dos tokens (0xC3, 0xB1); el primero no debe
        // emitir nada y el segundo el carácter completo
        let tok = Tokenizer::byte_level();
        let mut dec = StreamDecoder::new();
        assert_eq!(dec.push(&tok, 0xC3), "");
        assert_eq!(dec.push(&tok, 0xB1), "ñ");
        assert_eq!(dec.push(&tok, b'a' as u32), "a");
        assert_eq!(dec.finish(), "");
    }
}
