//! Prueba de aceptación R27: `psk_from_passphrase` valida y deriva la PMK.

use soso_wpa2::crypto::{pbkdf2_psk, psk_from_passphrase, PskError};

fn hex(s: &str) -> Vec<u8> {
    (0..s.len() / 2)
        .map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap())
        .collect()
}

#[test]
fn deriva_como_pbkdf2_con_entradas_validas() {
    let pmk = psk_from_passphrase("password", "IEEE").unwrap();
    assert_eq!(
        pmk.to_vec(),
        hex("f42c6fc52df0ebef9ebb4b90b38a5f902e83fe1b135a70e23aed762e9710a12e")
    );
    assert_eq!(pmk, pbkdf2_psk("password", "IEEE"));
    // Espacios y símbolos imprimibles valen.
    assert_eq!(
        psk_from_passphrase("con espacios ~!", "x").unwrap(),
        pbkdf2_psk("con espacios ~!", "x")
    );
}

#[test]
fn limites_de_longitud_de_passphrase() {
    assert_eq!(psk_from_passphrase("1234567", "red"), Err(PskError::PassphraseLength));
    assert_eq!(psk_from_passphrase("", "red"), Err(PskError::PassphraseLength));
    assert!(psk_from_passphrase("12345678", "red").is_ok());
    let p63 = "a".repeat(63);
    assert_eq!(psk_from_passphrase(&p63, "red").unwrap(), pbkdf2_psk(&p63, "red"));
    // 64 caracteres que no son todo hexadecimal: demasiado larga.
    let p64 = format!("{}z", "a".repeat(63));
    assert_eq!(psk_from_passphrase(&p64, "red"), Err(PskError::PassphraseLength));
    let p65 = "a".repeat(65);
    assert_eq!(psk_from_passphrase(&p65, "red"), Err(PskError::PassphraseLength));
}

#[test]
fn limites_de_longitud_de_ssid_en_bytes() {
    assert_eq!(psk_from_passphrase("password", ""), Err(PskError::SsidLength));
    let s32 = "s".repeat(32);
    assert_eq!(psk_from_passphrase("password", &s32).unwrap(), pbkdf2_psk("password", &s32));
    let s33 = "s".repeat(33);
    assert_eq!(psk_from_passphrase("password", &s33), Err(PskError::SsidLength));
    // 17 caracteres de 2 bytes = 34 bytes: se cuentan bytes.
    let acentos = "ñ".repeat(17);
    assert_eq!(psk_from_passphrase("password", &acentos), Err(PskError::SsidLength));
    let ok = "ñ".repeat(16);
    assert!(psk_from_passphrase("password", &ok).is_ok());
}

#[test]
fn caracteres_no_imprimibles() {
    assert_eq!(psk_from_passphrase("pass\tword", "red"), Err(PskError::PassphraseChars));
    assert_eq!(psk_from_passphrase("pass\x7fword", "red"), Err(PskError::PassphraseChars));
    assert_eq!(psk_from_passphrase("contraseña", "red"), Err(PskError::PassphraseChars));
    // 31 y 32: el espacio (32) vale, el 31 no.
    assert!(psk_from_passphrase("        ", "red").is_ok());
    assert_eq!(psk_from_passphrase("\u{1f}1234567", "red"), Err(PskError::PassphraseChars));
}

#[test]
fn pmk_en_hexadecimal_se_devuelve_tal_cual() {
    let h = "f42c6fc52df0ebef9ebb4b90b38a5f902e83fe1b135a70e23aed762e9710a12e";
    assert_eq!(psk_from_passphrase(h, "IEEE").unwrap().to_vec(), hex(h));
    let mayus = h.to_uppercase();
    assert_eq!(psk_from_passphrase(&mayus, "IEEE").unwrap().to_vec(), hex(h));
    // No pasa por PBKDF2.
    assert_ne!(psk_from_passphrase(h, "IEEE").unwrap(), pbkdf2_psk(h, "IEEE"));
    let ceros = "0".repeat(64);
    assert_eq!(psk_from_passphrase(&ceros, "x").unwrap(), [0u8; 32]);
}

#[test]
fn el_ssid_se_comprueba_antes_que_la_passphrase() {
    assert_eq!(psk_from_passphrase("corta", ""), Err(PskError::SsidLength));
    assert_eq!(psk_from_passphrase("\u{1}", &"s".repeat(40)), Err(PskError::SsidLength));
    // Con SSID inválido tampoco se acepta la PMK en hex.
    assert_eq!(psk_from_passphrase(&"a".repeat(64), ""), Err(PskError::SsidLength));
    // Longitud antes que caracteres.
    assert_eq!(psk_from_passphrase("\u{1}\u{2}", "red"), Err(PskError::PassphraseLength));
}
