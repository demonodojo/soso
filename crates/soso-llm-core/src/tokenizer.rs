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

/// Prefijo de espacio SentencePiece (`▁` U+2581).
const SPACE_SP: char = '\u{2581}';
/// Prefijo de espacio GPT-2 / Qwen2 (`Ġ` U+0120).
const SPACE_GPT2: char = '\u{0120}';
/// GPT-2 bytes_to_unicode de controles habituales (el resto del texto va UTF-8).
const GPT2_NL: char = '\u{010A}'; // Ċ ← \\n
const GPT2_TAB: char = '\u{0109}'; // ĉ ← \\t
const GPT2_CR: char = '\u{010D}'; // ċ ← \\r

/// Cómo segmenta este vocabulario.
///
/// Hasta ahora se deducía contando marcas de espacio, que acierta casi siempre
/// y no sirve para decidir el **algoritmo**: un vocabulario GPT-2/Qwen2 es BPE
/// por rangos de fusión, y sin esa tabla no se puede reproducir su
/// segmentación (ficha T52).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Segmentacion {
    /// Pieza más larga que encaje. Es lo que hace falta para SentencePiece.
    PiezaMasLarga,
    /// BPE byte-level: símbolos iniciales y fusiones en orden de rango.
    BpeByteLevel,
}

pub struct VocabTokenizer {
    pieces: Vec<String>,
    lookup: BTreeMap<String, u32>,
    max_piece_len: usize,
    /// Marca de espacio del vocabulario (`▁` o `Ġ`).
    space_mark: char,
    /// SentencePiece pone la marca al empezar el texto; GPT-2/Qwen2 no.
    leading_space: bool,
    /// Qué algoritmo pide este vocabulario.
    segmentacion: Segmentacion,
    /// Fusiones BPE en orden de rango: `(izquierda, derecha)` como índices de
    /// `pieces`. Vacío = el modelo no las trajo.
    ///
    /// Se guardan como índices y no como cadenas a propósito: el vocabulario de
    /// Qwen2.5 tiene ~151 k piezas y ~151 k fusiones; repetir el texto de cada
    /// lado multiplicaría por seis el tamaño del `tokenizer.som`.
    merges: Vec<(u32, u32)>,
    /// Por cada par fusionable: su rango (menor = antes) y la pieza que sale.
    /// Se resuelve al cargar para no concatenar cadenas en cada fusión.
    rango: BTreeMap<(u32, u32), (u32, u32)>,
    /// Tokens añadidos, los que hay que apartar antes de segmentar.
    /// Ordenados de más largo a más corto para que gane el más específico.
    especiales: Vec<u32>,
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
    /// Cuántas piezas tiene el vocabulario.
    ///
    /// Hace falta para comprobar que un id de referencia cabe en este
    /// tokenizer: un id por encima del vocabulario no es una divergencia de
    /// segmentación, es otro vocabulario.
    pub fn vocab_len(&self) -> usize {
        match self {
            Tokenizer::ByteLevel => 256,
            Tokenizer::Vocab(v) => v.pieces.len(),
        }
    }

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
    /// Vocabulario sin fusiones: segmenta por pieza más larga, como siempre.
    pub fn new(pieces: Vec<String>, bos: u32, eos: u32) -> Self {
        Self::con_merges(pieces, bos, eos, Vec::new())
    }

    /// Vocabulario con su tabla de fusiones BPE.
    ///
    /// Que haya fusiones es lo que decide el algoritmo: un vocabulario que las
    /// trae es BPE por rangos, y uno que no, se segmenta como antes.
    pub fn con_merges(
        pieces: Vec<String>,
        bos: u32,
        eos: u32,
        merges: Vec<(u32, u32)>,
    ) -> Self {
        let mut lookup = BTreeMap::new();
        let mut max_piece_len = 1;
        let mut n_sp = 0u32;
        let mut n_gpt2 = 0u32;
        for (i, p) in pieces.iter().enumerate() {
            max_piece_len = max_piece_len.max(p.len());
            lookup.entry(p.clone()).or_insert(i as u32);
            if p.contains(SPACE_SP) {
                n_sp += 1;
            }
            if p.contains(SPACE_GPT2) {
                n_gpt2 += 1;
            }
        }
        let gpt2 = n_gpt2 > n_sp;
        let mut rango = BTreeMap::new();
        for (i, par) in merges.iter().enumerate() {
            let (a, b) = *par;
            let (Some(izq), Some(der)) = (pieces.get(a as usize), pieces.get(b as usize)) else {
                continue;
            };
            let mut unida = String::with_capacity(izq.len() + der.len());
            unida.push_str(izq);
            unida.push_str(der);
            // Una fusión cuyo resultado no está en el vocabulario no se puede
            // aplicar; se ignora en vez de inventar un id.
            if let Some(&resultado) = lookup.get(&unida) {
                rango.entry((a, b)).or_insert((i as u32, resultado));
            }
        }
        let segmentacion = if merges.is_empty() {
            Segmentacion::PiezaMasLarga
        } else {
            Segmentacion::BpeByteLevel
        };
        // En un BPE, **toda** pieza de más de un carácter sale de una fusión.
        // La que no sale de ninguna es un token añadido: `<|im_start|>`,
        // `<tool_call>`… Deducirlo así evita una heurística sobre su forma y
        // sale exacto: 256 piezas de un byte + una por fusión + las añadidas.
        let mut resultados = alloc::collections::BTreeSet::new();
        for (_, (_, resultado)) in rango.iter() {
            resultados.insert(*resultado);
        }
        let mut especiales: Vec<u32> = if merges.is_empty() {
            Vec::new()
        } else {
            pieces
                .iter()
                .enumerate()
                .filter(|(i, p)| {
                    p.chars().count() > 1 && !resultados.contains(&(*i as u32))
                })
                .map(|(i, _)| i as u32)
                .collect()
        };
        especiales.sort_by_key(|i| core::cmp::Reverse(pieces[*i as usize].len()));
        Self {
            pieces,
            lookup,
            max_piece_len,
            space_mark: if gpt2 { SPACE_GPT2 } else { SPACE_SP },
            leading_space: !gpt2,
            segmentacion,
            merges,
            rango,
            especiales,
            bos,
            eos,
        }
    }

    /// Qué algoritmo pide este vocabulario.
    pub fn segmentacion(&self) -> Segmentacion {
        self.segmentacion
    }

    /// Fusiones en orden de rango, como índices de piezas.
    pub fn merges(&self) -> &[(u32, u32)] {
        &self.merges
    }

    /// Rango de un par, si es una fusión conocida. Menor = se aplica antes.
    pub fn rango_de(&self, izquierda: u32, derecha: u32) -> Option<u32> {
        self.rango.get(&(izquierda, derecha)).map(|(r, _)| *r)
    }

    /// Rango y pieza resultante de fusionar dos piezas.
    pub fn fusion_de(&self, izquierda: u32, derecha: u32) -> Option<(u32, u32)> {
        self.rango.get(&(izquierda, derecha)).copied()
    }

    /// Texto de una pieza, tal y como está en el vocabulario.
    ///
    /// No pasa por `decode`: eso traduce bytes y **suprime los marcadores de
    /// parada**, que es justo lo que hay que poder mirar al diagnosticar una
    /// divergencia.
    pub fn pieza_de(&self, id: u32) -> Option<&str> {
        self.pieces.get(id as usize).map(|p| p.as_str())
    }

    /// Índice de una pieza exacta.
    pub fn id_de_pieza(&self, pieza: &str) -> Option<u32> {
        self.lookup.get(pieza).copied()
    }

    /// v1 — bos u32 | eos u32 | count u32 | (len u32 | bytes)*
    ///
    /// Se sigue emitiendo v1 cuando no hay fusiones: un `tokenizer.som` de un
    /// modelo SentencePiece no tiene por qué cambiar de versión, y así los
    /// modelos ya convertidos siguen siendo byte a byte los mismos.
    pub fn serialize(pieces: &[String], bos: u32, eos: u32) -> Vec<u8> {
        pack_som(&Self::cuerpo(pieces, bos, eos), 1, CACHE_ALIGN)
    }

    /// v2 — lo de v1, y después: n_merges u32 | (izquierda u32, derecha u32)*
    ///
    /// Los lados son índices de `pieces`, en orden de rango.
    pub fn serialize_con_merges(
        pieces: &[String],
        bos: u32,
        eos: u32,
        merges: &[(u32, u32)],
    ) -> Vec<u8> {
        if merges.is_empty() {
            return Self::serialize(pieces, bos, eos);
        }
        let mut body = Self::cuerpo(pieces, bos, eos);
        body.extend_from_slice(&(merges.len() as u32).to_le_bytes());
        for (a, b) in merges {
            body.extend_from_slice(&a.to_le_bytes());
            body.extend_from_slice(&b.to_le_bytes());
        }
        pack_som(&body, 2, CACHE_ALIGN)
    }

    fn cuerpo(pieces: &[String], bos: u32, eos: u32) -> Vec<u8> {
        let mut body = Vec::new();
        body.extend_from_slice(&bos.to_le_bytes());
        body.extend_from_slice(&eos.to_le_bytes());
        body.extend_from_slice(&(pieces.len() as u32).to_le_bytes());
        for p in pieces {
            body.extend_from_slice(&(p.len() as u32).to_le_bytes());
            body.extend_from_slice(p.as_bytes());
        }
        body
    }

    /// Lee v1 y v2. Un `tokenizer.som` antiguo sigue cargando y se comporta
    /// igual que antes: sin fusiones, segmentación por pieza más larga.
    pub fn parse(data: &[u8]) -> Result<Self, ()> {
        let (version, body) = parse_som(data)?;
        if version > 2 {
            return Err(());
        }
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
        let mut merges = Vec::new();
        if version >= 2 {
            let n = r.u32()? as usize;
            merges.reserve(n);
            for _ in 0..n {
                let a = r.u32()?;
                let b = r.u32()?;
                if a as usize >= pieces.len() || b as usize >= pieces.len() {
                    // Una fusión que apunta fuera del vocabulario no se puede
                    // aplicar: mejor rechazar el archivo que segmentar a medias.
                    return Err(());
                }
                merges.push((a, b));
            }
        }
        Ok(Self::con_merges(pieces, bos, eos, merges))
    }

    /// Greedy longest-match. SentencePiece: espacios → `▁` y `▁` inicial.
    /// GPT-2 / Qwen2: espacios → `Ġ`, sin marca al empezar.
    /// Caracteres sin pieza caen al byte-fallback `<0xXX>`.
    /// Tokens añadidos del vocabulario, de más largo a más corto.
    pub fn especiales(&self) -> &[u32] {
        &self.especiales
    }

    /// Fusiona los símbolos de un pre-token siguiendo el orden de rango.
    fn fusionar(&self, simbolos: &mut Vec<u32>) {
        loop {
            let mut mejor: Option<(usize, u32, u32)> = None;
            for i in 0..simbolos.len().saturating_sub(1) {
                if let Some((rango, resultado)) = self.fusion_de(simbolos[i], simbolos[i + 1]) {
                    if mejor.map(|(_, r, _)| rango < r).unwrap_or(true) {
                        mejor = Some((i, rango, resultado));
                    }
                }
            }
            let Some((i, _, resultado)) = mejor else {
                return;
            };
            simbolos[i] = resultado;
            simbolos.remove(i + 1);
        }
    }

    /// Segmenta un trozo sin tokens especiales: pre-token, byte-level y fusiones.
    fn encode_trozo_bpe(&self, texto: &str, out: &mut Vec<u32>) {
        let mut resto = texto;
        while !resto.is_empty() {
            let largo = largo_pretoken(resto).max(1);
            let (pre, siguiente) = resto.split_at(largo);
            resto = siguiente;

            let mapeado = a_bytelevel(pre);
            let mut simbolos = Vec::with_capacity(mapeado.len());
            for ch in mapeado.chars() {
                let mut buf = [0u8; 4];
                let pieza = ch.encode_utf8(&mut buf);
                match self.lookup.get(pieza) {
                    Some(&id) => simbolos.push(id),
                    // Un vocabulario byte-level completo tiene los 256; si
                    // falta alguno, ese byte no se puede representar y se salta
                    // en vez de inventar un id.
                    None => continue,
                }
            }
            self.fusionar(&mut simbolos);
            out.extend_from_slice(&simbolos);
        }
    }

    /// Segmentación BPE: primero los tokens especiales, luego el resto.
    ///
    /// Las apariciones se buscan **de una vez** y no una por trozo: repetir el
    /// barrido sobre el resto del texto por cada marcador encontrado hace el
    /// coste cuadrático, y aquí entran prompts de miles de tokens.
    fn encode_bpe(&self, text: &str, out: &mut Vec<u32>) {
        let mut marcas: Vec<(usize, usize, u32)> = Vec::new();
        for id in &self.especiales {
            let pieza = &self.pieces[*id as usize];
            for (pos, _) in text.match_indices(pieza.as_str()) {
                marcas.push((pos, pieza.len(), *id));
            }
        }
        // Por posición; a igualdad, gana el más largo.
        marcas.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)));

        let mut i = 0;
        for (pos, largo, id) in marcas {
            if pos < i {
                continue; // solapa con un marcador ya consumido
            }
            if pos > i {
                self.encode_trozo_bpe(&text[i..pos], out);
            }
            out.push(id);
            i = pos + largo;
        }
        if i < text.len() {
            self.encode_trozo_bpe(&text[i..], out);
        }
    }

    fn encode(&self, text: &str, bos: bool, prefijo: bool) -> Vec<u32> {
        let mut out = Vec::new();
        if bos && self.bos != NO_TOKEN {
            out.push(self.bos);
        }
        if self.segmentacion == Segmentacion::BpeByteLevel {
            self.encode_bpe(text, &mut out);
            return out;
        }
        let replaced = if self.space_mark == SPACE_GPT2 {
            gpt2_prepare(text)
        } else {
            text.replace(' ', &String::from(self.space_mark))
        };
        let normalized = if prefijo && self.leading_space {
            format!("{}{replaced}", self.space_mark)
        } else {
            replaced
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
        } else if self.segmentacion == Segmentacion::BpeByteLevel {
            // En un vocabulario byte-level cada carácter de la pieza **es** un
            // byte: `Ã¡` son los dos bytes de «á». Devolverlos como UTF-8 del
            // propio carácter era lo que convertía las tildes en mojibake.
            for ch in piece.chars() {
                match char_a_byte(ch) {
                    Some(b) => bytes.push(b),
                    None => {
                        let mut buf = [0u8; 4];
                        bytes.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                    }
                }
            }
        } else {
            for ch in piece.chars() {
                if ch == SPACE_SP || ch == SPACE_GPT2 {
                    bytes.push(b' ');
                } else if ch == GPT2_NL {
                    bytes.push(b'\n');
                } else if ch == GPT2_TAB {
                    bytes.push(b'\t');
                } else if ch == GPT2_CR {
                    bytes.push(b'\r');
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

/// Normaliza texto al alfabeto de tokens Qwen2/GPT-2 (espacio/`\\n`/`\\t`/`\\r`).
/// Tabla bytes→unicode de GPT-2, la que usan Qwen2 y compañía.
///
/// El vocabulario de un BPE byte-level no contiene letras acentuadas: contiene
/// **un carácter imprimible por byte**. «más» son los bytes `6D C3 A1 73`, que
/// en esa tabla son `m Ã ¡ s`. Sin esta traducción, buscar la letra «á» en el
/// vocabulario encuentra la pieza que representa al byte `0xE1`, que es otra
/// cosa: el modelo recibe un token que no significa lo que pone.
///
/// Los bytes imprimibles se representan a sí mismos; los 68 restantes van a
/// `U+0100 + n`. De ahí salen las constantes que ya había (`Ġ` para el espacio,
/// `Ċ` para el salto de línea).
fn byte_a_char(b: u8) -> char {
    match b {
        0x21..=0x7E | 0xA1..=0xAC | 0xAE..=0xFF => b as char,
        0x00..=0x20 => char::from_u32(0x100 + b as u32).unwrap_or('\u{fffd}'),
        0x7F..=0xA0 => char::from_u32(0x100 + 33 + (b as u32 - 0x7F)).unwrap_or('\u{fffd}'),
        0xAD => char::from_u32(0x100 + 67).unwrap_or('\u{fffd}'),
    }
}

/// La inversa: de carácter del vocabulario al byte que representa.
fn char_a_byte(c: char) -> Option<u8> {
    let v = c as u32;
    match v {
        0x21..=0x7E | 0xA1..=0xAC | 0xAE..=0xFF => Some(v as u8),
        0x100..=0x120 => Some((v - 0x100) as u8),
        0x121..=0x142 => Some((v - 0x100 - 33 + 0x7F) as u8),
        0x143 => Some(0xAD),
        _ => None,
    }
}

/// Texto a la representación byte-level del vocabulario.
fn a_bytelevel(texto: &str) -> String {
    let mut out = String::with_capacity(texto.len());
    for b in texto.bytes() {
        out.push(byte_a_char(b));
    }
    out
}


/// Longitud del siguiente pre-token, siguiendo el patrón de la familia.
///
/// El `pre_tokenizer` de Qwen2.5 es esta alternancia, y el orden importa
/// porque una alternancia de expresión regular se resuelve por la izquierda:
///
/// ```text
/// (?i:'s|'t|'re|'ve|'m|'ll|'d)   contracciones
/// | [^\r\n\p{L}\p{N}]?\p{L}+      un símbolo suelto y letras
/// | \p{N}                         un dígito, de uno en uno
/// |  ?[^\s\p{L}\p{N}]+[\r\n]*     espacio opcional, signos y saltos
/// | \s*[\r\n]+                   saltos con lo que lleven delante
/// | \s+(?!\S)                     espacios finales
/// | \s+                           espacios
/// ```
///
/// Se implementa a mano porque meter un motor de expresiones regulares en un
/// crate `no_std` que va dentro del sistema operativo es un precio alto por
/// siete alternativas fijas. Las fusiones **nunca cruzan** un pre-token: de ahí
/// salía que un `.` se pegara al `<` siguiente.
fn largo_pretoken(s: &str) -> usize {
    let bytes = s.as_bytes();
    if bytes.is_empty() {
        return 0;
    }
    let letra = |c: char| c.is_alphabetic();
    let numero = |c: char| c.is_numeric();
    let blanco = |c: char| c.is_whitespace();

    // 1. Contracciones inglesas, sin distinguir mayúsculas.
    if bytes[0] == b'\'' {
        const SUFIJOS: &[&str] = &["s", "t", "re", "ve", "m", "ll", "d"];
        for suf in SUFIJOS {
            let fin = 1 + suf.len();
            if s.len() >= fin && s[1..fin].eq_ignore_ascii_case(suf) {
                return fin;
            }
        }
    }

    let mut it = s.char_indices().peekable();
    let (_, primero) = *it.peek().expect("no vacío");

    // 2. Un símbolo opcional (que no sea salto ni letra ni dígito) y letras.
    {
        let mut i = 0;
        let mut chars = s.chars();
        let mut c = primero;
        if !letra(c) && !numero(c) && c != '\r' && c != '\n' {
            i += c.len_utf8();
            chars.next();
            match chars.next() {
                Some(sig) => c = sig,
                None => c = '\0',
            }
        }
        if letra(c) {
            let mut fin = i;
            for (j, ch) in s[i..].char_indices() {
                if letra(ch) {
                    fin = i + j + ch.len_utf8();
                } else {
                    break;
                }
            }
            return fin;
        }
    }

    // 3. Un dígito suelto.
    if numero(primero) {
        return primero.len_utf8();
    }

    // 4. Espacio opcional, signos, y los saltos que vengan detrás.
    {
        let mut i = 0;
        if primero == ' ' {
            i = 1;
        }
        let mut fin = i;
        for (j, ch) in s[i..].char_indices() {
            if !blanco(ch) && !letra(ch) && !numero(ch) {
                fin = i + j + ch.len_utf8();
            } else {
                break;
            }
        }
        if fin > i {
            for (j, ch) in s[fin..].char_indices() {
                if ch == '\r' || ch == '\n' {
                    fin += j + ch.len_utf8() - j;
                } else {
                    break;
                }
            }
            // Recuento simple de los saltos finales.
            let mut k = fin;
            while k < s.len() {
                let ch = s[k..].chars().next().unwrap();
                if ch == '\r' || ch == '\n' {
                    k += ch.len_utf8();
                } else {
                    break;
                }
            }
            return k;
        }
    }

    // 5. Blancos seguidos de al menos un salto.
    {
        let mut k = 0;
        while k < s.len() {
            let ch = s[k..].chars().next().unwrap();
            if blanco(ch) && ch != '\r' && ch != '\n' {
                k += ch.len_utf8();
            } else {
                break;
            }
        }
        let inicio_saltos = k;
        while k < s.len() {
            let ch = s[k..].chars().next().unwrap();
            if ch == '\r' || ch == '\n' {
                k += ch.len_utf8();
            } else {
                break;
            }
        }
        if k > inicio_saltos {
            return k;
        }
    }

    // 6 y 7. Blancos: si detrás viene algo que no es blanco, se deja el último
    // para quien venga (eso es el `(?!\S)` del patrón original).
    if blanco(primero) {
        let mut k = 0;
        let mut ultimo = 0;
        while k < s.len() {
            let ch = s[k..].chars().next().unwrap();
            if blanco(ch) {
                ultimo = k;
                k += ch.len_utf8();
            } else {
                break;
            }
        }
        if k < s.len() && ultimo > 0 {
            return ultimo;
        }
        return k;
    }

    // Nada encajó: un carácter, para no quedarse parado.
    primero.len_utf8()
}

fn gpt2_prepare(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            ' ' => out.push(SPACE_GPT2),
            '\n' => out.push(GPT2_NL),
            '\t' => out.push(GPT2_TAB),
            '\r' => out.push(GPT2_CR),
            other => out.push(other),
        }
    }
    out
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

    /// Piezas al estilo GPT-2/Qwen2 con sus fusiones, para el formato v2.
    /// Vocabulario BPE realista en pequeño: piezas de un carácter, las demás
    /// salidas de una fusión, y un token añadido que no sale de ninguna.
    ///
    /// Índices: 0 `a` · 1 `b` · 2 `c` · 3 `ab` · 4 `abc` · 5 `<|fin|>`
    fn vocab_bpe() -> (Vec<String>, Vec<(u32, u32)>) {
        let pieces: Vec<String> = ["a", "b", "c", "ab", "abc", "<|fin|>"]
            .iter()
            .map(|s| String::from(*s))
            .collect();
        // (a,b) -> ab ; (ab,c) -> abc
        let merges = alloc::vec![(0u32, 1u32), (3, 2)];
        (pieces, merges)
    }

    #[test]
    fn el_formato_v2_conserva_las_fusiones() {
        let (pieces, merges) = vocab_bpe();
        let datos = VocabTokenizer::serialize_con_merges(&pieces, 0, 0, &merges);
        let leido = VocabTokenizer::parse(&datos).expect("v2 debe parsearse");
        assert_eq!(leido.merges(), merges.as_slice());
        assert_eq!(leido.segmentacion(), Segmentacion::BpeByteLevel);
        // El rango es la posición: menor se aplica antes.
        assert_eq!(leido.rango_de(0, 1), Some(0));
        assert_eq!(leido.rango_de(3, 2), Some(1));
        assert_eq!(leido.rango_de(2, 0), None);
        assert_eq!(leido.fusion_de(0, 1), Some((0, 3)), "(a,b) produce ab");
        assert_eq!(leido.id_de_pieza("abc"), Some(4));
    }

    /// Un `tokenizer.som` de los de antes tiene que seguir cargando igual.
    /// Una fusión cuyo resultado no está en el vocabulario no se puede aplicar:
    /// se ignora al cargar en vez de inventar un id.
    #[test]
    fn una_fusion_sin_resultado_en_el_vocabulario_se_ignora() {
        let pieces: Vec<String> = ["a", "b"].iter().map(|s| String::from(*s)).collect();
        // (a,b) -> "ab", que no está.
        let datos = VocabTokenizer::serialize_con_merges(&pieces, 0, 0, &[(0, 1)]);
        let leido = VocabTokenizer::parse(&datos).expect("el archivo es válido");
        assert_eq!(leido.merges(), &[(0, 1)], "la tabla se conserva tal cual");
        assert_eq!(leido.fusion_de(0, 1), None, "pero no se puede aplicar");
    }

    #[test]
    fn el_formato_v1_sigue_cargando_sin_fusiones() {
        let pieces: Vec<String> = ["<s>", "</s>", "\u{2581}hola"]
            .iter()
            .map(|s| String::from(*s))
            .collect();
        let datos = VocabTokenizer::serialize(&pieces, 0, 1);
        let leido = VocabTokenizer::parse(&datos).expect("v1 debe seguir parseándose");
        assert!(leido.merges().is_empty());
        assert_eq!(leido.segmentacion(), Segmentacion::PiezaMasLarga);
        assert_eq!(leido.bos, 0);
        assert_eq!(leido.eos, 1);
    }

    /// Sin fusiones se sigue emitiendo v1: los modelos ya convertidos no
    /// cambian ni un byte por este cambio de formato.
    #[test]
    fn sin_fusiones_el_archivo_no_cambia() {
        let pieces: Vec<String> = ["a", "b"].iter().map(|s| String::from(*s)).collect();
        let v1 = VocabTokenizer::serialize(&pieces, 0, 1);
        let igual = VocabTokenizer::serialize_con_merges(&pieces, 0, 1, &[]);
        assert_eq!(v1, igual);
    }

    /// Una fusión que apunta fuera del vocabulario no se puede aplicar: el
    /// archivo se rechaza en vez de segmentar a medias.
    #[test]
    fn una_fusion_fuera_del_vocabulario_invalida_el_archivo() {
        let (pieces, _) = vocab_bpe();
        let malas = alloc::vec![(0u32, 99u32)];
        let datos = VocabTokenizer::serialize_con_merges(&pieces, 0, 0, &malas);
        assert!(VocabTokenizer::parse(&datos).is_err());
    }

    #[test]
    fn una_version_futura_se_rechaza() {
        let (pieces, merges) = vocab_bpe();
        let mut datos = VocabTokenizer::serialize_con_merges(&pieces, 0, 0, &merges);
        // El contenedor .som guarda la versión en los bytes 12..16.
        datos[12..16].copy_from_slice(&9u32.to_le_bytes());
        assert!(VocabTokenizer::parse(&datos).is_err());
    }

    #[test]
    fn el_texto_con_tildes_va_y_vuelve() {
        // Vocabulario byte-level mínimo: los 256 bytes y una fusión.
        let mut pieces: Vec<String> = (0..=255u8).map(|b| String::from(byte_a_char(b))).collect();
        pieces.push(String::from("Ã¡"));
        let merges = alloc::vec![(0xC3u32, 0xA1u32)];
        let datos = VocabTokenizer::serialize_con_merges(&pieces, NO_TOKEN, NO_TOKEN, &merges);
        let tok = Tokenizer::parse(&datos).unwrap();
        let ids = tok.encode("más");
        assert_eq!(tok.decode(&ids), "más", "ids = {ids:?}");
        assert_eq!(tok.decode(&tok.encode("café ☕")), "café ☕");
    }

    #[test]
    fn la_tabla_byte_level_es_la_de_gpt2() {
        // Los valores que ya usaba el código, ahora salen de la tabla general.
        assert_eq!(byte_a_char(b' '), SPACE_GPT2);
        assert_eq!(byte_a_char(b'\n'), GPT2_NL);
        assert_eq!(byte_a_char(b'\t'), GPT2_TAB);
        assert_eq!(byte_a_char(b'\r'), GPT2_CR);
        assert_eq!(byte_a_char(b'A'), 'A');
        // «á» son dos bytes, y cada uno tiene su carácter.
        assert_eq!(a_bytelevel("á"), "Ã¡");
        // Ida y vuelta para los 256.
        for b in 0..=255u8 {
            assert_eq!(char_a_byte(byte_a_char(b)), Some(b), "byte {b}");
        }
    }

    #[test]
    fn el_pretokenizador_separa_como_la_familia() {
        // Un signo no se pega a la letra siguiente, y el espacio se va con la
        // palabra: de ahí salían `.<` y compañía.
        let trozos = |mut s: &str| {
            let mut out = alloc::vec::Vec::new();
            while !s.is_empty() {
                let n = largo_pretoken(s).max(1);
                out.push(String::from(&s[..n]));
                s = &s[n..];
            }
            out
        };
        assert_eq!(trozos("hola mundo"), alloc::vec!["hola", " mundo"]);
        assert_eq!(trozos("."), alloc::vec!["."]);
        assert_eq!(trozos(".<"), alloc::vec![".<"]);
        assert_eq!(trozos("a.\nb"), alloc::vec!["a", ".\n", "b"]);
        assert_eq!(trozos("12"), alloc::vec!["1", "2"], "los dígitos van sueltos");
        assert_eq!(trozos("don't"), alloc::vec!["don", "'t"]);
        assert_eq!(trozos("  x"), alloc::vec![" ", " x"], "el último espacio es del siguiente");
    }

    /// Con fusiones, el vocabulario se segmenta por rangos y no por pieza más
    /// larga: es la diferencia que hacía que «Responde» saliera partido mal.
    #[test]
    fn con_fusiones_se_segmenta_por_rango() {
        let (pieces, merges) = vocab_bpe();
        let datos = VocabTokenizer::serialize_con_merges(&pieces, NO_TOKEN, NO_TOKEN, &merges);
        let tok = Tokenizer::parse(&datos).unwrap();
        // a+b -> ab (rango 0) y ab+c -> abc (rango 1).
        assert_eq!(tok.encode("abc"), alloc::vec![4]);
        // Sin fusión aplicable, cada carácter va suelto.
        assert_eq!(tok.encode("acb"), alloc::vec![0, 2, 1]);
    }

    #[test]
    fn los_tokens_anadidos_se_apartan_antes_de_segmentar() {
        let (pieces, merges) = vocab_bpe();
        let datos = VocabTokenizer::serialize_con_merges(&pieces, NO_TOKEN, NO_TOKEN, &merges);
        let tok = Tokenizer::parse(&datos).unwrap();
        // `<|fin|>` no sale de ninguna fusión: se deduce que es añadido y no se
        // parte, ni se lleva por delante lo que tiene al lado.
        assert_eq!(tok.encode("abc<|fin|>abc"), alloc::vec![4, 5, 4]);
        if let Tokenizer::Vocab(v) = &tok {
            assert_eq!(v.especiales(), &[5]);
        }
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
    fn gpt2_espacio_sin_prefijo_inicial() {
        let pieces: Vec<String> = [
            "<|im_start|>",
            "hola",
            "\u{0120}mundo",
            "\u{0120}",
        ]
        .iter()
        .map(|s| String::from(*s))
        .collect();
        let tok = Tokenizer::Vocab(VocabTokenizer::new(pieces, NO_TOKEN, NO_TOKEN));
        let ids = tok.encode_trozo("hola mundo", false, true);
        assert_eq!(ids, vec![1, 2]);
        assert_eq!(tok.decode(&ids), "hola mundo");
    }

    #[test]
    fn gpt2_salto_de_linea() {
        let pieces: Vec<String> = ["hola", "\u{010A}", "mundo", "\u{0120}x"]
            .iter()
            .map(|s| String::from(*s))
            .collect();
        let tok = Tokenizer::Vocab(VocabTokenizer::new(pieces, NO_TOKEN, NO_TOKEN));
        let ids = tok.encode_trozo("hola\nmundo", false, true);
        assert_eq!(ids, vec![0, 1, 2]);
        assert_eq!(tok.decode(&ids), "hola\nmundo");
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
