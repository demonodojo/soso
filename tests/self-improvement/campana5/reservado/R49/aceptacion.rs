//! Aceptación reservada de R49.

use soso_llm_core::tokenizer::{StreamDecoder, Tokenizer, VocabTokenizer, NO_TOKEN};

fn piezas(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

fn stream(tok: &Tokenizer, ids: &[u32]) -> Vec<String> {
    let mut d = StreamDecoder::new();
    ids.iter().map(|&t| d.push(tok, t)).collect()
}

#[test]
fn byte_invalido_se_emite_de_inmediato_como_fffd() {
    let tok = Tokenizer::byte_level();
    assert_eq!(stream(&tok, &[0xFF, b'a' as u32, b'b' as u32]), vec!["\u{FFFD}", "a", "b"]);
    // el valido anterior sigue saliendo igual
    assert_eq!(stream(&tok, &[b'x' as u32, 0xFF, 0xC3, 0xB1]), vec!["x", "\u{FFFD}", "", "ñ"]);
}

#[test]
fn prefijo_incompleto_seguido_de_un_no_continuacion() {
    let tok = Tokenizer::byte_level();
    // E2 espera continuacion; 'a' no lo es: FFFD y despues la 'a' en el mismo push
    assert_eq!(stream(&tok, &[0xE2, b'a' as u32]), vec!["", "\u{FFFD}a"]);
    // E2 82 es un prefijo valido de tres bytes; 'a' lo rompe: un solo FFFD
    assert_eq!(stream(&tok, &[0xE2, 0x82, b'a' as u32]), vec!["", "", "\u{FFFD}a"]);
    // un continuador suelto es invalido
    assert_eq!(stream(&tok, &[0x80, b'z' as u32]), vec!["\u{FFFD}", "z"]);
}

#[test]
fn el_flujo_completo_equivale_a_from_utf8_lossy() {
    let tok = Tokenizer::byte_level();
    let bytes: Vec<u8> = vec![
        b'h', 0xFF, 0xFE, b'o', 0xE2, 0x82, b'!', 0xC3, 0xB1, 0xF0, 0x9F, 0x98, b'k', 0x80, 0x80, 0xC3,
    ];
    let mut d = StreamDecoder::new();
    let mut s = String::new();
    for &b in &bytes {
        s.push_str(&d.push(&tok, b as u32));
    }
    s.push_str(&d.finish());
    assert_eq!(s, String::from_utf8_lossy(&bytes));
    // un prefijo incompleto al final sale en finish()
    let mut d = StreamDecoder::new();
    assert_eq!(d.push(&tok, 0xC3), "");
    assert_eq!(d.finish(), "\u{FFFD}");
    assert_eq!(d.finish(), "");
}

#[test]
fn decode_gpt2_conserva_el_espacio_inicial() {
    let tok = Tokenizer::Vocab(VocabTokenizer::new(
        piezas(&["hola", "\u{0120}mundo", "\u{0120}", "\u{0120}x"]),
        NO_TOKEN,
        NO_TOKEN,
    ));
    assert_eq!(tok.decode(&[1]), " mundo");
    assert_eq!(tok.decode(&[2]), " ");
    assert_eq!(tok.decode(&[0, 1]), "hola mundo");
    let ids = tok.encode(" hola");
    assert_eq!(tok.decode(&ids), " hola");
    let ids = tok.encode_trozo(" mundo", false, true);
    assert_eq!(tok.decode(&ids), " mundo");
}

#[test]
fn decode_sentencepiece_quita_un_solo_espacio_inicial() {
    let tok = Tokenizer::Vocab(VocabTokenizer::new(
        piezas(&["<s>", "</s>", "\u{2581}hola", "\u{2581}\u{2581}a", "\u{2581}"]),
        0,
        1,
    ));
    assert_eq!(tok.decode(&[0, 2]), "hola");
    assert_eq!(tok.decode(&[3]), " a");
    assert_eq!(tok.decode(&[4]), "");
}

#[test]
fn pieza_con_signo_no_es_un_byte() {
    let tok = Tokenizer::Vocab(VocabTokenizer::new(
        piezas(&["<s>", "</s>", "<0x+1>", "<0x41>", "<0xc3>", "<0xB1>"]),
        0,
        1,
    ));
    assert_eq!(tok.decode(&[2]), "<0x+1>");
    assert_eq!(tok.decode(&[3]), "A");
    // dos digitos hexadecimales, mayusculas o minusculas, siguen siendo bytes
    assert_eq!(tok.decode(&[4, 5]), "ñ");
    let mut d = StreamDecoder::new();
    assert_eq!(d.push(&tok, 2), "<0x+1>");
}

#[test]
fn byte_level_ignora_ids_que_no_caben_en_un_byte() {
    let tok = Tokenizer::byte_level();
    assert_eq!(tok.decode(&[b'a' as u32, 0x141, 256, b'b' as u32, u32::MAX]), "ab");
    let mut v = Vec::new();
    tok.token_bytes(0x141, &mut v);
    tok.token_bytes(255, &mut v);
    assert_eq!(v, vec![255]);
    let mut d = StreamDecoder::new();
    assert_eq!(d.push(&tok, 0x141), "");
    assert_eq!(d.push(&tok, b'q' as u32), "q");
    // los ids validos de siempre no cambian
    let ids = tok.encode("año");
    assert_eq!(tok.decode(&ids), "año");
}
