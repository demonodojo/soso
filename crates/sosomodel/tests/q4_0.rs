//! Q4_0 (layout GGML): descuantización contra la definición de `dequantize_row_q4_0`.

use sosomodel::dequant::{dequant_q4_0, dequant_q4_0_range, f16_to_f32, row_bytes};
use sosomodel::layout::{DTYPE_Q4_0, Q4_0_BLOCK_BYTES, Q4_0_BLOCK_ELEMS};

/// f32 → f16 (redondeo al más cercano) sólo para construir bloques de prueba con escalas exactas.
fn f16_bits(v: f32) -> u16 {
    // Las escalas de prueba son potencias de dos pequeñas: representables sin redondeo.
    let b = v.to_bits();
    let sign = (b >> 31) as u16;
    let exp = ((b >> 23) & 0xff) as i32 - 127 + 15;
    let frac = ((b >> 13) & 0x3ff) as u16;
    (sign << 15) | ((exp as u16) << 10) | frac
}

fn bloque(d: f32, qs: [u8; 16]) -> Vec<u8> {
    let mut v = f16_bits(d).to_le_bytes().to_vec();
    v.extend_from_slice(&qs);
    v
}

#[test]
fn el_nibble_bajo_es_el_elemento_j_y_el_alto_el_j_mas_16() {
    let mut qs = [0u8; 16];
    for j in 0..16 {
        qs[j] = (j as u8) | (((15 - j) as u8) << 4);
    }
    let b = bloque(0.5, qs);
    let mut out = [0f32; 32];
    dequant_q4_0(&b, &mut out).unwrap();
    for j in 0..16 {
        assert_eq!(out[j], (j as i32 - 8) as f32 * 0.5, "bajo {j}");
        assert_eq!(out[j + 16], ((15 - j as i32) - 8) as f32 * 0.5, "alto {j}");
    }
}

#[test]
fn el_cero_es_el_ocho_y_la_escala_se_aplica() {
    let b = bloque(0.25, [0x88; 16]);
    let mut out = [1f32; 32];
    dequant_q4_0(&b, &mut out).unwrap();
    assert!(out.iter().all(|&v| v == 0.0));
    let b = bloque(2.0, [0xF0; 16]); // bajo 0 → -8, alto 15 → +7
    dequant_q4_0(&b, &mut out).unwrap();
    assert_eq!((out[0], out[16]), (-16.0, 14.0));
}

#[test]
fn el_rango_coincide_con_la_descuantizacion_completa() {
    let mut bytes = Vec::new();
    for i in 0..5u8 {
        bytes.extend(bloque(1.0 + i as f32, core::array::from_fn(|j| (j as u8).wrapping_mul(17).wrapping_add(i))));
    }
    let mut todo = vec![0f32; 5 * Q4_0_BLOCK_ELEMS];
    dequant_q4_0(&bytes, &mut todo).unwrap();
    for (off, n) in [(0, 1), (3, 40), (31, 2), (33, 100), (127, 33)] {
        let mut parte = vec![0f32; n];
        dequant_q4_0_range(&bytes, off, &mut parte).unwrap();
        assert_eq!(parte, todo[off..off + n], "rango {off}+{n}");
    }
    assert!(dequant_q4_0_range(&bytes, 150, &mut [0f32; 20]).is_err());
}

#[test]
fn row_bytes_cuenta_bloques_de_18() {
    assert_eq!(row_bytes(DTYPE_Q4_0, 32), Some(18));
    assert_eq!(row_bytes(DTYPE_Q4_0, 3840), Some(120 * 18));
    assert_eq!(row_bytes(DTYPE_Q4_0, 33), None);
    assert!(f16_to_f32(f16_bits(0.5)) == 0.5);
}
