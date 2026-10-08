//! T82 — los matvec por lotes dan **exactamente** lo mismo que n matvec sueltos.
//!
//! Con AVX2 compilado (`RUSTFLAGS="-C target-feature=+avx2,+fma"`) se prueban los
//! kernels por lotes de verdad; sin él el lote es un bucle de los sueltos y la
//! prueba pasa trivialmente. CI debe correr ambas.

use soso_llm_core::gemm::{matvec_q4_k, matvec_q4_k_batch, matvec_q8_0, matvec_q8_0_batch};
use sosomodel::layout::{Q4_K_BLOCK_BYTES, Q4_K_BLOCK_ELEMS, Q8_0_BLOCK_BYTES, Q8_0_BLOCK_ELEMS};

struct Lcg(u32);
impl Lcg {
    fn next(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        self.0 >> 8
    }
    fn f(&mut self) -> f32 {
        (self.next() % 2001) as f32 / 1000.0 - 1.0
    }
}

fn q4_k(rows: usize, cols: usize, g: &mut Lcg) -> Vec<u8> {
    let rb = (cols / Q4_K_BLOCK_ELEMS) * Q4_K_BLOCK_BYTES;
    let mut v = vec![0u8; rows * rb];
    for b in v.iter_mut() {
        *b = (g.next() & 0xFF) as u8;
    }
    for blk in v.chunks_exact_mut(Q4_K_BLOCK_BYTES) {
        // d y dmin como f16 finitos y pequeños (0x2c00 ≈ 0,0625; 0x2800 ≈ 0,03).
        blk[0] = (g.next() & 0xFF) as u8;
        blk[1] = 0x28 | (g.next() & 0x03) as u8;
        blk[2] = (g.next() & 0xFF) as u8;
        blk[3] = 0x24 | (g.next() & 0x03) as u8;
    }
    v
}

fn q8_0(rows: usize, cols: usize, g: &mut Lcg) -> Vec<u8> {
    let rb = (cols / Q8_0_BLOCK_ELEMS) * Q8_0_BLOCK_BYTES;
    let mut v = vec![0u8; rows * rb];
    for blk in v.chunks_exact_mut(Q8_0_BLOCK_BYTES) {
        let scale = (g.next() % 100) as f32 * 0.001 + 0.001;
        blk[..4].copy_from_slice(&scale.to_le_bytes());
        for b in blk[4..].iter_mut() {
            *b = (g.next() & 0xFF) as u8;
        }
    }
    v
}

fn vectores(n: usize, cols: usize, g: &mut Lcg) -> Vec<f32> {
    (0..n * cols).map(|_| g.f()).collect()
}

fn bits(v: &[f32]) -> Vec<u32> {
    v.iter().map(|x| x.to_bits()).collect()
}

#[test]
fn q4_k_por_lotes_es_identico_bit_a_bit() {
    let mut g = Lcg(7);
    for &(rows, cols) in &[(1usize, 256usize), (5, 512), (37, 768)] {
        let w = q4_k(rows, cols, &mut g);
        for n in 1..=19usize {
            let xs = vectores(n, cols, &mut g);
            let mut lote = vec![0f32; n * rows];
            matvec_q4_k_batch(&w, rows, cols, &xs, n, &mut lote).unwrap();
            let mut sueltos = vec![0f32; n * rows];
            for j in 0..n {
                matvec_q4_k(
                    &w,
                    rows,
                    cols,
                    &xs[j * cols..(j + 1) * cols],
                    &mut sueltos[j * rows..(j + 1) * rows],
                )
                .unwrap();
            }
            assert_eq!(bits(&lote), bits(&sueltos), "q4_k rows={rows} cols={cols} n={n}");
        }
    }
}

#[test]
fn q8_0_por_lotes_es_identico_bit_a_bit() {
    let mut g = Lcg(11);
    for &(rows, cols) in &[(1usize, 32usize), (6, 96), (41, 256)] {
        let w = q8_0(rows, cols, &mut g);
        for n in 1..=19usize {
            let xs = vectores(n, cols, &mut g);
            let mut lote = vec![0f32; n * rows];
            matvec_q8_0_batch(&w, rows, cols, &xs, n, &mut lote).unwrap();
            let mut sueltos = vec![0f32; n * rows];
            for j in 0..n {
                matvec_q8_0(
                    &w,
                    rows,
                    cols,
                    &xs[j * cols..(j + 1) * cols],
                    &mut sueltos[j * rows..(j + 1) * rows],
                )
                .unwrap();
            }
            assert_eq!(bits(&lote), bits(&sueltos), "q8_0 rows={rows} cols={cols} n={n}");
        }
    }
}

#[test]
fn formas_incoherentes_son_error() {
    let mut g = Lcg(3);
    let w = q4_k(2, 256, &mut g);
    let xs = vectores(2, 256, &mut g);
    let mut o = vec![0f32; 3]; // debería ser 2*2
    assert!(matvec_q4_k_batch(&w, 2, 256, &xs, 2, &mut o).is_err());
    let mut o = vec![0f32; 4];
    assert!(matvec_q4_k_batch(&w, 2, 250, &xs, 2, &mut o).is_err());
}
