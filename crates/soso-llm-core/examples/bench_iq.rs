//! Compute puro de los i-quants (matriz sintética en RAM, sin E/S): descuantizar a f32 frente a producto entero × Q8_K.
//! `RUSTFLAGS="-C target-feature=+avx2,+fma" cargo run --release -p soso-llm-core --features std --example bench_iq`
use soso_llm_core::gemm::{iq_q8k_activar, matvec_iq};
use sosomodel::layout::{DTYPE_IQ2_S, DTYPE_IQ3_XXS, DTYPE_IQ4_XS};
use std::time::Instant;

fn main() {
    let (rows, cols) = (4096usize, 5120usize);
    println!("AVX2 en el binario: {}", cfg!(target_feature = "avx2"));
    {
        use soso_llm_core::gemm::{matvec_q4_k, matvec_q8_0};
        let x: Vec<f32> = (0..cols).map(|i| ((i * 7 % 13) as f32 - 6.0) * 0.1).collect();
        let mut o = vec![0f32; rows];
        let rb8 = cols / 32 * 36;
        let mut w8 = vec![0u8; rows * rb8];
        for (i, b) in w8.chunks_mut(36).enumerate() { b[..4].copy_from_slice(&0.01f32.to_le_bytes()); for (k, v) in b[4..].iter_mut().enumerate() { *v = ((i + k) % 200) as u8; } }
        let t = Instant::now();
        for _ in 0..5 { matvec_q8_0(&w8, rows, cols, &x, &mut o).unwrap(); }
        let ms = t.elapsed().as_secs_f64() * 200.0;
        println!("Q8_0 f32      {ms:7.1} ms  {:5.2} GB/s", (rows * rb8) as f64 / (ms / 1e3) / 1e9);
        let rb4 = cols / 256 * 144;
        let mut w4 = vec![0u8; rows * rb4];
        for (i, v) in w4.iter_mut().enumerate() { *v = (i * 31 % 251) as u8; }
        for b in w4.chunks_mut(144) { b[0] = 0; b[1] = 0x2c; b[2] = 0; b[3] = 0x2c; }
        let t = Instant::now();
        for _ in 0..5 { matvec_q4_k(&w4, rows, cols, &x, &mut o).unwrap(); }
        let ms = t.elapsed().as_secs_f64() * 200.0;
        println!("Q4_K f32      {ms:7.1} ms  {:5.2} GB/s", (rows * rb4) as f64 / (ms / 1e3) / 1e9);
    }
    for (nombre, dt) in [("IQ3_XXS", DTYPE_IQ3_XXS), ("IQ2_S", DTYPE_IQ2_S), ("IQ4_XS", DTYPE_IQ4_XS)] {
        let rb = sosomodel::dequant::row_bytes(dt, cols).unwrap();
        let mut s = 1u64;
        let mut w: Vec<u8> = (0..rows * rb).map(|_| { s = s.wrapping_mul(6364136223846793005).wrapping_add(1); (s >> 56) as u8 }).collect();
        for b in w.chunks_mut(rb / (cols / 256)) { b[0] = 0x00; b[1] = 0x2c; } // d = 0.0625 (f16)
        let x: Vec<f32> = (0..cols).map(|i| ((i * 7 % 13) as f32 - 6.0) * 0.1).collect();
        let mut o = vec![0f32; rows];
        for (etq, on) in [("f32 ", false), ("q8k ", true)] {
            iq_q8k_activar(on);
            matvec_iq(dt, &w, rows, cols, &x, &mut o).unwrap();
            let t = Instant::now();
            for _ in 0..5 { matvec_iq(dt, &w, rows, cols, &x, &mut o).unwrap(); }
            let ms = t.elapsed().as_secs_f64() * 200.0;
            println!("{nombre} {etq} {ms:7.1} ms  {:5.2} GB/s", (rows * rb) as f64 / (ms / 1e3) / 1e9);
        }
    }
}
