//! T82: ¿el matvec cuantizado está limitado por memoria o por cómputo?
//!
//! Compilar con AVX2 (`RUSTFLAGS="-C target-feature=+avx2,+fma"`): sin eso los
//! kernels son los escalares de referencia y la medida no vale para el guest.
//! Mide el matvec Q4_K de la FFN del 7B (18944 × 3584) con 1 vector, con N matvec
//! sueltos y con el kernel por lotes (el que usa el prefill por bloques).

use std::time::Instant;

use soso_llm_core::gemm::{matvec_q4_k, matvec_q4_k_batch};
use sosomodel::layout::{Q4_K_BLOCK_BYTES, Q4_K_BLOCK_ELEMS};

fn main() {
    let (rows, cols) = (18_944usize, 3_584usize);
    let rb = (cols / Q4_K_BLOCK_ELEMS) * Q4_K_BLOCK_BYTES;
    let mut bytes = vec![0u8; rows * rb];
    let mut s = 0x1234_5678u32;
    for b in bytes.iter_mut() {
        s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        *b = (s >> 24) as u8;
    }
    // Escalas f16 sanas (d, dmin al principio de cada bloque) para no medir NaN.
    for blk in bytes.chunks_exact_mut(Q4_K_BLOCK_BYTES) {
        blk[0] = 0x00;
        blk[1] = 0x2c; // ≈ 0,0625
        blk[2] = 0x00;
        blk[3] = 0x28;
    }
    let mib = bytes.len() as f64 / 1_048_576.0;
    println!("matriz {rows}×{cols} Q4_K = {mib:.1} MiB; AVX2 en el binario: {}", cfg!(target_feature = "avx2"));

    for n in [1usize, 4, 8, 16, 32] {
        let xs: Vec<Vec<f32>> = (0..n)
            .map(|k| (0..cols).map(|i| ((i * 7 + k * 13) % 17) as f32 * 0.01 - 0.08).collect())
            .collect();
        let mut outs = vec![vec![0f32; rows]; n];
        // Referencia: n matvec independientes (lo que hace el prefill hoy).
        let t = Instant::now();
        let reps = 3;
        for _ in 0..reps {
            for k in 0..n {
                matvec_q4_k(&bytes, rows, cols, &xs[k], &mut outs[k]).unwrap();
            }
        }
        let sep = t.elapsed().as_secs_f64() / reps as f64;
        // Lote real (T82): n vectores con acumuladores independientes.
        let xs_flat: Vec<f32> = xs.iter().flatten().copied().collect();
        let mut lote_out = vec![0f32; n * rows];
        let t = Instant::now();
        for _ in 0..reps {
            matvec_q4_k_batch(&bytes, rows, cols, &xs_flat, n, &mut lote_out).unwrap();
        }
        let lote = t.elapsed().as_secs_f64() / reps as f64;
        let outs2: Vec<Vec<f32>> = lote_out.chunks(rows).map(|c| c.to_vec()).collect();
        assert_eq!(outs, outs2, "el lote tiene que dar bit a bit lo mismo");
        println!(
            "n={n:>2}: separados {:.1} ms ({:.2} ms/vec) · en lote {:.1} ms ({:.2} ms/vec) · aceleración {:.2}×",
            sep * 1e3,
            sep * 1e3 / n as f64,
            lote * 1e3,
            lote * 1e3 / n as f64,
            sep / lote
        );
    }
}
