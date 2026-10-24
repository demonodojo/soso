//! Las dequantizaciones de `iq.rs` contra libggml (fixtures de `tools/iq-oraculo`): igualdad bit a bit.
use soso_llm_core::iq::{bytes_de_bloque, dequant_bloque, QK_K};

fn comprobar(nombre: &str, ggml: u32) {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/iq/");
    let bin = std::fs::read(format!("{dir}{nombre}.bin")).unwrap();
    let esp = std::fs::read(format!("{dir}{nombre}.f32")).unwrap();
    let bb = bytes_de_bloque(ggml).unwrap();
    let n = bin.len() / bb;
    assert!(n > 0 && esp.len() == n * QK_K * 4, "{nombre}: tamaños");
    let mut y = vec![0f32; QK_K];
    for b in 0..n {
        assert!(dequant_bloque(ggml, &bin[b * bb..(b + 1) * bb], &mut y));
        for (i, v) in y.iter().enumerate() {
            let o = (b * QK_K + i) * 4;
            let e = f32::from_le_bytes([esp[o], esp[o + 1], esp[o + 2], esp[o + 3]]);
            assert_eq!(v.to_bits(), e.to_bits(), "{nombre}: bloque {b} elemento {i}: {v} != {e}");
        }
    }
}

#[test] fn q3_k() { comprobar("Q3_K", 11) }
#[test] fn q5_k() { comprobar("Q5_K", 13) }
#[test] fn iq2_xs() { comprobar("IQ2_XS", 17) }
#[test] fn iq3_xxs() { comprobar("IQ3_XXS", 18) }
#[test] fn iq3_s() { comprobar("IQ3_S", 21) }
#[test] fn iq2_s() { comprobar("IQ2_S", 22) }
#[test] fn iq4_xs() { comprobar("IQ4_XS", 23) }
