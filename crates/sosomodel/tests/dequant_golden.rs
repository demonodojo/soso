//! Valores dorados de la decodificación Q4_K / Q8_0.
//!
//! **Para qué sirve esta tabla.** La misma aritmética existe dos veces: aquí en Rust
//! (`sosomodel::dequant`, lo que ejecuta la CPU y el dispositivo software del kernel)
//! y en C (`lxdde/ports/nouveau/q4k_decode.h`, lo que compila nvcc dentro de los
//! kernels SASS). Si las dos divergen, el síntoma es «la GPU saca otros tokens» y se
//! busca en el silicio, que es donde no está el error.
//!
//! Los números de abajo están **duplicados a propósito** en
//! `tools/gsp-hostcheck/main.c` (`check_q4k_decode`). Cambiar la decodificación de un
//! lado pone en rojo a uno de los dos, que es exactamente lo que se quiere. El bloque
//! y el vector se generan con la misma receta en los dos sitios.

use sosomodel::dequant::{dequant_q4_k, dequant_q8_0, q4k_scale_min, q4k_dot_ref};

/// Superbloque Q4_K de 144 B, determinista y con las escalas de 6 bits repartidas
/// por los dos caminos de `get_scale_min_k4` (sub-bloques <4 y ≥4).
fn bloque_q4k() -> [u8; 144] {
    let mut b = [0u8; 144];
    // d = 0.5 en f16 (0x3800), dmin = 0.25 (0x3400).
    b[0] = 0x00;
    b[1] = 0x38;
    b[2] = 0x00;
    b[3] = 0x34;
    // scales[12]: valores fijos que ejercitan los bits altos (los que componen los
    // sub-bloques 4..7) y no sólo los bajos.
    let scales: [u8; 12] = [0x41, 0x82, 0xC3, 0x04, 0x45, 0x86, 0xC7, 0x08, 0x9A, 0xBC, 0xDE, 0xF0];
    b[4..16].copy_from_slice(&scales);
    // qs[128]: patrón que da nibbles bajos y altos distintos en cada byte.
    for i in 0..128 {
        b[16 + i] = ((i * 7 + 3) % 256) as u8;
    }
    b
}

fn x_256() -> [f32; 256] {
    let mut x = [0.0f32; 256];
    for (i, v) in x.iter_mut().enumerate() {
        *v = ((i % 13) as f32 - 6.0) * 0.125;
    }
    x
}

#[test]
fn q4k_escalas_de_6_bits() {
    let b = bloque_q4k();
    let esperado: [(u8, u8); 8] = [
        (1, 5),
        (2, 6),
        (3, 7),
        (4, 8),
        (26, 25),
        (44, 43),
        (62, 61),
        (0, 15),
    ];
    for j in 0..8 {
        assert_eq!(
            q4k_scale_min(&b[4..16], j),
            esperado[j],
            "sub-bloque {j}: si esto cambia, `q4k_scale_min` de q4k_decode.h también"
        );
    }
}

#[test]
fn q4k_producto_punto_dorado() {
    let b = bloque_q4k();
    let x = x_256();

    // Referencia por descuantización completa + producto punto.
    let mut plano = [0.0f32; 256];
    dequant_q4_k(&b, &mut plano).unwrap();
    let mut ref_dot = 0.0f32;
    for i in 0..256 {
        ref_dot += plano[i] * x[i];
    }

    // Y la misma cuenta por sub-bloques, que es la forma que usa el kernel SASS.
    let porsub = q4k_dot_ref(&b, &x);
    assert!(
        (porsub - ref_dot).abs() <= 1e-3 * ref_dot.abs().max(1.0),
        "por sub-bloques {porsub} vs descuantizando {ref_dot}"
    );

    // El valor dorado: duplicado en check_q4k_decode del hostcheck.
    const DORADO: f32 = -15.875;
    assert!(
        (ref_dot - DORADO).abs() <= 1e-2,
        "producto punto dorado: {ref_dot} (esperaba {DORADO}) — si esto cambia a \
         propósito, actualiza también check_q4k_decode en tools/gsp-hostcheck/main.c"
    );
}

#[test]
fn q80_producto_punto_dorado() {
    // Bloque Q8_0: escala 0.25 + 32 int8 deterministas.
    let mut b = [0u8; 36];
    b[..4].copy_from_slice(&0.25f32.to_le_bytes());
    for i in 0..32 {
        b[4 + i] = ((i as i32 * 9 - 100) as i8) as u8;
    }
    let mut plano = [0.0f32; 32];
    dequant_q8_0(&b, &mut plano).unwrap();
    let mut dot = 0.0f32;
    for i in 0..32 {
        dot += plano[i] * ((i % 13) as f32 - 6.0) * 0.125;
    }
    const DORADO: f32 = 172.593_75;
    assert!(
        (dot - DORADO).abs() <= 1e-3,
        "producto punto Q8_0 dorado: {dot} (esperaba {DORADO})"
    );
}
