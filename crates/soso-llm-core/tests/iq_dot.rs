//! Producto IQ × Q8_K: el escalar coincide con «descuantizar y multiplicar» y el despachado (AVX2 si el binario lo trae)
//! coincide bit a bit con el escalar. Para ejercitar AVX2: `RUSTFLAGS="-C target-feature=+avx2,+fma"`.
use soso_llm_core::iq::{bytes_de_bloque, dequant_bloque, GGML_IQ2_S, GGML_IQ2_XS, GGML_IQ3_S, GGML_IQ3_XXS, GGML_IQ4_XS, GGML_Q5_K, QK_K};
use soso_llm_core::iq_dot::{dot_bloque, dot_bloque_escalar, factor_fila};
use soso_llm_core::q8k::{quantize_row_q8_k, Q8_K_BLOCK_BYTES};

fn lcg(s: &mut u64) -> f32 {
    *s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    ((*s >> 33) as f32 / (1u64 << 31) as f32 - 0.5) * 4.0
}

fn comprobar(nombre: &str, ggml: u32) {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/iq/");
    let bin = std::fs::read(format!("{dir}{nombre}.bin")).unwrap();
    let bb = bytes_de_bloque(ggml).unwrap();
    let mut seed = 7u64;
    let (mut maxrel, mut iguales) = (0f64, true);
    for b in 0..bin.len() / bb {
        let w = &bin[b * bb..(b + 1) * bb];
        let x: Vec<f32> = (0..QK_K).map(|_| lcg(&mut seed)).collect();
        let mut q8 = vec![0u8; Q8_K_BLOCK_BYTES];
        quantize_row_q8_k(&x, &mut q8);
        let d8 = f32::from_le_bytes([q8[0], q8[1], q8[2], q8[3]]);
        let mut wf = vec![0f32; QK_K];
        assert!(dequant_bloque(ggml, w, &mut wf));
        let r: f64 = (0..QK_K).map(|i| wf[i] as f64 * (d8 * (q8[4 + i] as i8) as f32) as f64).sum();
        let e = dot_bloque_escalar(ggml, w, &q8) as f64 * factor_fila(ggml) as f64;
        let scale = wf.iter().map(|v| v.abs() as f64).sum::<f64>() * 4.0 + 1e-9;
        maxrel = maxrel.max((e - r).abs() / scale);
        iguales &= dot_bloque(ggml, w, &q8).to_bits() == dot_bloque_escalar(ggml, w, &q8).to_bits();
    }
    assert!(maxrel < 1e-5, "{nombre}: error relativo {maxrel}");
    assert!(iguales, "{nombre}: despachado != escalar");
}

#[test] fn iq3_xxs() { comprobar("IQ3_XXS", GGML_IQ3_XXS) }
#[test] fn iq2_s() { comprobar("IQ2_S", GGML_IQ2_S) }
#[test] fn iq4_xs() { comprobar("IQ4_XS", GGML_IQ4_XS) }
#[test] fn iq3_s() { comprobar("IQ3_S", GGML_IQ3_S) }
#[test] fn q5_k() { comprobar("Q5_K", GGML_Q5_K) }

/// El camino por lotes de tokens da, para cada token, el mismo f32 que el producto de un solo vector.
fn lote(nombre: &str, ggml: u32) {
    use soso_llm_core::iq_dot::matvec_q8k;
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/iq/");
    let bin = std::fs::read(format!("{dir}{nombre}.bin")).unwrap();
    let bb = bytes_de_bloque(ggml).unwrap();
    let (rows, blocks, n) = (6usize, 2usize, 5usize);
    let cols = blocks * QK_K;
    let w = &bin[..rows * blocks * bb];
    let mut seed = 99u64;
    let mut q8 = vec![0u8; n * blocks * Q8_K_BLOCK_BYTES];
    for j in 0..n {
        let x: Vec<f32> = (0..cols).map(|_| lcg(&mut seed)).collect();
        quantize_row_q8_k(&x, &mut q8[j * blocks * Q8_K_BLOCK_BYTES..(j + 1) * blocks * Q8_K_BLOCK_BYTES]);
    }
    let mut juntos = vec![0f32; n * rows];
    matvec_q8k(ggml, w, rows, cols, &q8, n, &mut juntos, rows).unwrap();
    for j in 0..n {
        let mut uno = vec![0f32; rows];
        matvec_q8k(ggml, w, rows, cols, &q8[j * blocks * Q8_K_BLOCK_BYTES..(j + 1) * blocks * Q8_K_BLOCK_BYTES], 1, &mut uno, rows).unwrap();
        for r in 0..rows {
            assert_eq!(juntos[j * rows + r].to_bits(), uno[r].to_bits(), "{nombre}: token {j} fila {r}");
        }
    }
}

#[test] fn lote_iq3_xxs() { lote("IQ3_XXS", GGML_IQ3_XXS) }
#[test] fn lote_iq2_s() { lote("IQ2_S", GGML_IQ2_S) }
#[test] fn lote_iq4_xs() { lote("IQ4_XS", GGML_IQ4_XS) }
#[test] fn lote_iq3_s() { lote("IQ3_S", GGML_IQ3_S) }
#[test] fn lote_q5_k() { lote("Q5_K", GGML_Q5_K) }
#[test] fn iq2_xs() { comprobar("IQ2_XS", GGML_IQ2_XS) }
#[test] fn lote_iq2_xs() { lote("IQ2_XS", GGML_IQ2_XS) }
