//! Aceptación reservada de R38.

use soso_web_core::decode::maybe_decompress;

/// "hola soso, hola soso, hola soso\n" repetido 3 veces (96 bytes).
fn esperado() -> Vec<u8> {
    b"hola soso, hola soso, hola soso\n".repeat(3)
}

/// Deflate crudo (sin cabecera zlib) de `esperado()`.
const RAW: [u8; 20] = [
    0xcb, 0xc8, 0xcf, 0x49, 0x54, 0x28, 0xce, 0x2f, 0xce, 0xd7, 0x51, 0xc8, 0xc0, 0xc2, 0xe4, 0xca, 0xa0, 0x50,
    0x1e, 0x00,
];

/// Mismo contenido envuelto en zlib.
const ZLIB: [u8; 27] = [
    0x78, 0x9c, 0xcb, 0xc8, 0xcf, 0x49, 0x54, 0x28, 0xce, 0x2f, 0xce, 0xd7, 0x51, 0xc8, 0xc0, 0xc2, 0xe4, 0xc2,
    0x26, 0x48, 0x8a, 0x3c, 0x00, 0x6f, 0x0c, 0x21, 0xaf,
];

/// Miembro gzip con la cabecera dada (bytes 4..10 ya incluidos) y `extra` entre cabecera y datos.
fn gzip(flags: u8, extra: &[u8], isize: u32) -> Vec<u8> {
    let mut g = vec![0x1f, 0x8b, 8, flags, 0, 0, 0, 0, 0, 3];
    g.extend_from_slice(extra);
    g.extend_from_slice(&RAW);
    g.extend_from_slice(&[0, 0, 0, 0]); // CRC32: no se comprueba
    g.extend_from_slice(&isize.to_le_bytes());
    g
}

#[test]
fn sin_codificacion_pasa_igual() {
    assert_eq!(maybe_decompress(b"abc", None).unwrap(), b"abc");
    assert_eq!(maybe_decompress(b"abc", Some(" Identity ")).unwrap(), b"abc");
    assert_eq!(maybe_decompress(b"abc", Some("br")).unwrap(), b"abc");
}

#[test]
fn gzip_simple() {
    let g = gzip(0, &[], 96);
    assert_eq!(maybe_decompress(&g, Some("gzip")).unwrap(), esperado());
    assert_eq!(maybe_decompress(&g, Some("X-GZIP")).unwrap(), esperado());
}

#[test]
fn gzip_con_campos_opcionales() {
    // FEXTRA (3 bytes de extra), FNAME y FCOMMENT.
    let mut extra = vec![3, 0, b'a', b'b', b'c'];
    extra.extend_from_slice(b"nombre.txt\0");
    extra.extend_from_slice(b"un comentario\0");
    let g = gzip(0x04 | 0x08 | 0x10, &extra, 96);
    assert_eq!(maybe_decompress(&g, Some("gzip")).unwrap(), esperado());
    // FHCRC: dos bytes de CRC de cabecera.
    let g = gzip(0x02, &[0xaa, 0xbb], 96);
    assert_eq!(maybe_decompress(&g, Some("gzip")).unwrap(), esperado());
    // Sólo FNAME.
    let g = gzip(0x08, b"f\0", 96);
    assert_eq!(maybe_decompress(&g, Some("gzip")).unwrap(), esperado());
}

#[test]
fn gzip_corrupto_o_truncado_es_error() {
    let ok = gzip(0, &[], 96);
    // ISIZE que no cuadra.
    assert!(maybe_decompress(&gzip(0, &[], 95), Some("gzip")).is_err());
    // Magia o método erróneos.
    let mut mal = ok.clone();
    mal[1] = 0x8c;
    assert!(maybe_decompress(&mal, Some("gzip")).is_err());
    let mut mal = ok.clone();
    mal[2] = 7;
    assert!(maybe_decompress(&mal, Some("gzip")).is_err());
    // Truncado: sin cola, a medio dato, sólo cabecera, vacío.
    assert!(maybe_decompress(&ok[..ok.len() - 8], Some("gzip")).is_err());
    assert!(maybe_decompress(&ok[..20], Some("gzip")).is_err());
    assert!(maybe_decompress(&ok[..10], Some("gzip")).is_err());
    assert!(maybe_decompress(&[], Some("gzip")).is_err());
    // Nombre sin terminador y extra más largo que el cuerpo.
    let mut sin_nul = vec![0x1f, 0x8b, 8, 0x08, 0, 0, 0, 0, 0, 3];
    sin_nul.extend_from_slice(&[b'x'; 20]);
    assert!(maybe_decompress(&sin_nul, Some("gzip")).is_err());
    let g = gzip(0x04, &[0xff, 0xff], 96);
    assert!(maybe_decompress(&g, Some("gzip")).is_err());
}

#[test]
fn deflate_zlib_y_crudo() {
    assert_eq!(maybe_decompress(&ZLIB, Some("deflate")).unwrap(), esperado());
    assert_eq!(maybe_decompress(&RAW, Some("deflate")).unwrap(), esperado());
    assert!(maybe_decompress(&[0xff, 0xff, 0xff, 0xff], Some("deflate")).is_err());
}
