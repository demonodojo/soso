//! Núcleos Q4_0 (layout GGML): el matvec fusionado coincide con descuantizar y multiplicar en f32,
//! tanto el escalar como el AVX2 y los de lote (con zancada).

use soso_llm_core::f16::f32_to_f16;
use soso_llm_core::gemm::{matvec_f32, matvec_q4_0, matvec_q4_0_batch, matvec_q4_0_batch_strided, matvec_q4_0_scalar};
use sosomodel::dequant::dequant_q4_0;
use sosomodel::layout::{Q4_0_BLOCK_BYTES, Q4_0_BLOCK_ELEMS};

/// LCG determinista.
fn rnd(s: &mut u64) -> u32 {
    *s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    (*s >> 33) as u32
}

fn pesos(rows: usize, cols: usize, semilla: u64) -> Vec<u8> {
    let mut s = semilla;
    let mut v = Vec::new();
    for _ in 0..rows * cols / Q4_0_BLOCK_ELEMS {
        let d = ((rnd(&mut s) % 2000) as f32 - 1000.0) * 1e-3;
        v.extend_from_slice(&f32_to_f16(d).to_le_bytes());
        for _ in 0..16 {
            v.push(rnd(&mut s) as u8);
        }
    }
    v
}

fn vector(n: usize, semilla: u64) -> Vec<f32> {
    let mut s = semilla;
    (0..n).map(|_| ((rnd(&mut s) % 2001) as f32 - 1000.0) * 1e-3).collect()
}

fn referencia(bytes: &[u8], rows: usize, cols: usize, x: &[f32]) -> Vec<f32> {
    let mut plano = vec![0f32; rows * cols];
    dequant_q4_0(bytes, &mut plano).unwrap();
    let mut out = vec![0f32; rows];
    matvec_f32(&plano, rows, cols, x, &mut out);
    out
}

fn cerca(a: &[f32], b: &[f32], ctx: &str) {
    for (i, (x, y)) in a.iter().zip(b).enumerate() {
        let tol = 2e-3 + x.abs().max(y.abs()) * 2e-4;
        assert!((x - y).abs() <= tol, "{ctx}: fila {i}: {x} vs {y}");
    }
}

#[test]
fn el_matvec_q4_0_coincide_con_descuantizar_y_multiplicar() {
    for (rows, cols) in [(1, 32), (5, 96), (7, 3840), (16, 15360 / 4), (3, 64)] {
        let w = pesos(rows, cols, 1 + cols as u64);
        let x = vector(cols, 99);
        let esperado = referencia(&w, rows, cols, &x);
        let mut out = vec![0f32; rows];
        matvec_q4_0(&w, rows, cols, &x, &mut out).unwrap();
        cerca(&out, &esperado, &format!("{rows}x{cols}"));
        let mut esc = vec![0f32; rows];
        matvec_q4_0_scalar(&w, rows, cols, &x, &mut esc);
        cerca(&esc, &esperado, &format!("escalar {rows}x{cols}"));
    }
}

#[test]
fn el_lote_da_lo_mismo_que_uno_a_uno_y_respeta_la_zancada() {
    let (rows, cols) = (11, 256);
    let w = pesos(rows, cols, 7);
    for n in [1usize, 2, 3, 5, 8, 9, 17] {
        let xs = vector(n * cols, 31 + n as u64);
        let mut lote = vec![0f32; n * rows];
        matvec_q4_0_batch(&w, rows, cols, &xs, n, &mut lote).unwrap();
        for j in 0..n {
            let uno = referencia(&w, rows, cols, &xs[j * cols..(j + 1) * cols]);
            cerca(&lote[j * rows..(j + 1) * rows], &uno, &format!("lote n={n} j={j}"));
        }
        // Con zancada mayor que `rows`: lo que no es resultado no se toca.
        let stride = rows + 5;
        let mut z = vec![-77.0f32; n * stride];
        matvec_q4_0_batch_strided(&w, rows, cols, &xs, n, &mut z, stride).unwrap();
        for j in 0..n {
            cerca(&z[j * stride..j * stride + rows], &lote[j * rows..(j + 1) * rows], &format!("zancada n={n}"));
            if j + 1 < n {
                assert!(z[j * stride + rows..(j + 1) * stride].iter().all(|&v| v == -77.0), "se pisó el hueco");
            }
        }
    }
}

#[test]
fn las_formas_invalidas_se_rechazan() {
    let w = pesos(2, 64, 3);
    let x = vector(64, 4);
    let mut o = vec![0f32; 2];
    assert!(matvec_q4_0(&w, 2, 60, &x[..60], &mut o).is_err(), "cols no múltiplo de 32");
    assert!(matvec_q4_0(&w[..w.len() - 1], 2, 64, &x, &mut o).is_err(), "bytes de menos");
    assert!(matvec_q4_0(&w, 2, 64, &x[..63], &mut o).is_err(), "x corto");
    assert!(matvec_q4_0(&w, 3, 64, &x, &mut o).is_err());
    assert_eq!(w.len(), 2 * 64 / 32 * Q4_0_BLOCK_BYTES);
}

/// Los wrappers sólo despachan a AVX2 si el crate se compila con `+avx2`; este test llama al núcleo
/// directamente (con detección en tiempo de ejecución) para que el camino rápido se pruebe siempre.
#[cfg(target_arch = "x86_64")]
#[test]
fn el_nucleo_avx2_se_prueba_aunque_el_build_no_lo_active() {
    if !(is_x86_feature_detected!("avx2") && is_x86_feature_detected!("fma")) {
        eprintln!("sin AVX2: se omite");
        return;
    }
    use soso_llm_core::gemm::avx2;
    for (rows, cols) in [(1, 32), (5, 96), (9, 3840)] {
        let w = pesos(rows, cols, 5 + rows as u64);
        let x = vector(cols, 8);
        let esperado = referencia(&w, rows, cols, &x);
        let mut out = vec![0f32; rows];
        unsafe { avx2::matvec_q4_0(&w, rows, cols, &x, &mut out) };
        cerca(&out, &esperado, &format!("avx2 {rows}x{cols}"));
    }
    let (rows, cols) = (6, 128);
    let w = pesos(rows, cols, 21);
    for n in [1usize, 4, 8, 11] {
        let xs = vector(n * cols, 40 + n as u64);
        let mut z = vec![0f32; n * rows];
        unsafe { avx2::matvec_q4_0_batch(&w, rows, cols, &xs, n, &mut z, rows) };
        for j in 0..n {
            let uno = referencia(&w, rows, cols, &xs[j * cols..(j + 1) * cols]);
            cerca(&z[j * rows..(j + 1) * rows], &uno, &format!("avx2 lote n={n}"));
        }
    }
}
