//! Producto por bloque Q8_0 con tensor cores enteros (`q8_mmq`) contra la referencia en f64. Las activaciones se cuantizan a i8 por grupos de 32,
//! así que el error esperado es ~4e-3 relativo al rms; un fallo de fragmentos o de escalas daría ~1. Sin GPU la prueba se salta.
#![cfg(feature = "std")]

use soso_llm_api::cuda::CudaGpu;

fn azar(e: &mut u64) -> u32 {
    *e ^= *e << 13;
    *e ^= *e >> 7;
    *e ^= *e << 17;
    (*e >> 32) as u32
}

fn probar(rows: usize, cols: usize, n: usize) {
    let mut gpu = match CudaGpu::nueva() {
        Ok(g) => g,
        Err(e) => {
            eprintln!("cuda_q8_mmq: SALTADA, no hay CUDA ({e})");
            return;
        }
    };
    let mut e = 0x1234_5678_9ABC_DEF1u64 ^ (rows * 31 + cols * 7 + n) as u64;
    let mut w = Vec::new();
    for _ in 0..rows * cols / 32 {
        let d = 0.002 + (azar(&mut e) % 100) as f32 * 0.0001;
        w.extend_from_slice(&d.to_le_bytes());
        for _ in 0..32 {
            w.push(azar(&mut e) as u8);
        }
    }
    let x: Vec<f32> = (0..n * cols).map(|_| (azar(&mut e) % 2001) as f32 / 1000.0 - 1.0).collect();
    let nb = cols / 32;
    let (mut err2, mut ref2) = (0f64, 0f64);
    let ent = gpu.probar_matmul_q8(&w, rows, cols, &x, n, true).expect("mmq");
    let f16 = gpu.probar_matmul_q8(&w, rows, cols, &x, n, false).expect("f16");
    for t in 0..n {
        for r in 0..rows {
            let mut s = 0f64;
            for b in 0..nb {
                let bl = &w[(r * nb + b) * 36..(r * nb + b) * 36 + 36];
                let d = f32::from_le_bytes([bl[0], bl[1], bl[2], bl[3]]) as f64;
                for j in 0..32 {
                    s += d * (bl[4 + j] as i8) as f64 * x[t * cols + b * 32 + j] as f64;
                }
            }
            err2 += ((ent[t * rows + r] as f64) - s).powi(2);
            ref2 += s * s;
            assert!(((f16[t * rows + r] as f64) - s).abs() < 0.02 * (1.0 + s.abs()), "el camino f16 se desvió: fila {r} token {t}");
        }
    }
    let rel = (err2 / ref2.max(1e-30)).sqrt();
    eprintln!("Q8_0 {rows}x{cols} n={n}: error relativo rms {rel:.5}");
    assert!(rel < 0.02, "error relativo {rel}");
}

#[test]
fn q8_mmq_iguala_a_la_referencia() {
    probar(128, 256, 64);
    probar(100, 384, 70); // filas y tokens que no son múltiplo de los tiles
    probar(256, 2048, 33);
    probar(64, 128, 1);
}
