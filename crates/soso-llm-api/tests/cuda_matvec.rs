//! Paridad de los kernels CUDA del host (G9) contra una decodificación de referencia en CPU.
//! Sin GPU/driver NVIDIA la prueba se salta (lo dice por stderr); con ellos corre sola.
#![cfg(feature = "std")]

use soso_llm_api::cuda::CudaGpu;
use soso_llm_core::gpu::GpuDispatch;
use soso_llm_core::layer::TensorView;
use sosomodel::layout::{DTYPE_Q4_0, DTYPE_Q4_K, DTYPE_Q8_0};

fn azar(estado: &mut u64) -> u32 {
    *estado = estado
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    (*estado >> 33) as u32
}

/// f16 de una escala pequeña (0.0156 … 0.125), en bits.
const ESCALAS_F16: [u16; 4] = [0x2400, 0x2800, 0x2C00, 0x3000];

fn f16_a_f32(b: u16) -> f32 {
    soso_llm_core::f16::f16_to_f32(b)
}

fn matriz_q4_0(rows: usize, cols: usize, e: &mut u64) -> Vec<u8> {
    let mut v = Vec::new();
    for _ in 0..rows * cols / 32 {
        v.extend_from_slice(&ESCALAS_F16[azar(e) as usize % 4].to_le_bytes());
        for _ in 0..16 {
            v.push(azar(e) as u8);
        }
    }
    v
}

fn ref_q4_0(w: &[u8], rows: usize, cols: usize, x: &[f32], n: usize) -> Vec<f32> {
    let nb = cols / 32;
    let mut y = vec![0.0f32; n * rows];
    for t in 0..n {
        for r in 0..rows {
            let mut s = 0.0f64;
            for b in 0..nb {
                let bl = &w[(r * nb + b) * 18..(r * nb + b) * 18 + 18];
                let d = f16_a_f32(u16::from_le_bytes([bl[0], bl[1]])) as f64;
                for j in 0..16 {
                    let lo = (bl[2 + j] & 0xf) as i32 - 8;
                    let hi = (bl[2 + j] >> 4) as i32 - 8;
                    s += d * lo as f64 * x[t * cols + b * 32 + j] as f64;
                    s += d * hi as f64 * x[t * cols + b * 32 + j + 16] as f64;
                }
            }
            y[t * rows + r] = s as f32;
        }
    }
    y
}

fn matriz_q8_0(rows: usize, cols: usize, e: &mut u64) -> Vec<u8> {
    let mut v = Vec::new();
    for _ in 0..rows * cols / 32 {
        v.extend_from_slice(&0.02f32.to_le_bytes());
        for _ in 0..32 {
            v.push(azar(e) as u8);
        }
    }
    v
}

fn ref_q8_0(w: &[u8], rows: usize, cols: usize, x: &[f32]) -> Vec<f32> {
    let nb = cols / 32;
    (0..rows)
        .map(|r| {
            let mut s = 0.0f64;
            for b in 0..nb {
                let bl = &w[(r * nb + b) * 36..(r * nb + b) * 36 + 36];
                let d = f32::from_le_bytes([bl[0], bl[1], bl[2], bl[3]]) as f64;
                for j in 0..32 {
                    s += d * (bl[4 + j] as i8) as f64 * x[b * 32 + j] as f64;
                }
            }
            s as f32
        })
        .collect()
}

fn cerca(a: &[f32], b: &[f32], que: &str) {
    assert_eq!(a.len(), b.len());
    let mut peor = 0.0f32;
    for (x, y) in a.iter().zip(b) {
        peor = peor.max((x - y).abs() / (1.0 + y.abs()));
    }
    assert!(peor < 2e-4, "{que}: error relativo máximo {peor}");
}

#[test]
fn kernels_cuda_igualan_a_la_referencia() {
    let mut gpu = match CudaGpu::nueva() {
        Ok(g) => g,
        Err(e) => {
            eprintln!("cuda_matvec: SALTADA, no hay CUDA ({e})");
            return;
        }
    };
    let mut e = 12345u64;
    let (rows, cols) = (300usize, 3840usize);
    let w = matriz_q4_0(rows, cols, &mut e);
    let vista = TensorView {
        bytes: &w,
        dtype: DTYPE_Q4_0,
        elems: rows * cols,
    };
    for n in [1usize, 8, 13] {
        let x: Vec<f32> = (0..n * cols)
            .map(|_| (azar(&mut e) % 2001) as f32 / 1000.0 - 1.0)
            .collect();
        let mut y = vec![0.0f32; n * rows];
        let ok = if n == 1 {
            gpu.matvec("q4", &vista, rows, cols, &x, &mut y).unwrap()
        } else {
            gpu.matmul("q4", &vista, rows, cols, n, &x, &mut y).unwrap()
        };
        assert!(ok, "la GPU debía atender Q4_0 n={n}");
        cerca(&y, &ref_q4_0(&w, rows, cols, &x, n), &format!("Q4_0 n={n}"));
    }
    // Q8_0 (la cabeza de logits de Gemma).
    let w8 = matriz_q8_0(rows, cols, &mut e);
    let v8 = TensorView {
        bytes: &w8,
        dtype: DTYPE_Q8_0,
        elems: rows * cols,
    };
    let x: Vec<f32> = (0..cols)
        .map(|_| (azar(&mut e) % 2001) as f32 / 1000.0 - 1.0)
        .collect();
    let mut y = vec![0.0f32; rows];
    assert!(gpu.matvec("q8", &v8, rows, cols, &x, &mut y).unwrap());
    cerca(&y, &ref_q8_0(&w8, rows, cols, &x), "Q8_0");
    assert_eq!(gpu.residentes(), 2);
}

/// El servidor crea el backend en el hilo principal y lo usa desde el de cada petición.
#[test]
fn el_backend_se_puede_usar_desde_otro_hilo() {
    let gpu = match CudaGpu::nueva() {
        Ok(g) => g,
        Err(e) => {
            eprintln!("cuda_matvec: SALTADA, no hay CUDA ({e})");
            return;
        }
    };
    std::thread::spawn(move || {
        let mut gpu = gpu;
        let mut e = 7u64;
        let (rows, cols) = (64usize, 256usize);
        let w = matriz_q4_0(rows, cols, &mut e);
        let vista = TensorView {
            bytes: &w,
            dtype: DTYPE_Q4_0,
            elems: rows * cols,
        };
        let x: Vec<f32> = (0..cols).map(|i| i as f32 / 100.0).collect();
        let mut y = vec![0.0f32; rows];
        assert!(
            gpu.matvec("t", &vista, rows, cols, &x, &mut y)
                .expect("sin error CUDA")
        );
        cerca(&y, &ref_q4_0(&w, rows, cols, &x, 1), "otro hilo");
    })
    .join()
    .unwrap();
}

/// Formas pequeñas (menos de 32 bloques por fila) y todos los tamaños de lote: las del MLP de una capa chica.
#[test]
fn matmul_con_formas_pequenas_y_lotes_de_1_a_8() {
    let mut gpu = match CudaGpu::nueva() {
        Ok(g) => g,
        Err(e) => {
            eprintln!("cuda_matvec: SALTADA, no hay CUDA ({e})");
            return;
        }
    };
    let mut e = 4242u64;
    for (rows, cols) in [
        (256usize, 256usize),
        (512, 256),
        (256, 512),
        (128, 512),
        (64, 256),
    ] {
        let w = matriz_q4_0(rows, cols, &mut e);
        let vista = TensorView {
            bytes: &w,
            dtype: DTYPE_Q4_0,
            elems: rows * cols,
        };
        for n in 1..=8usize {
            let x: Vec<f32> = (0..n * cols)
                .map(|_| (azar(&mut e) % 2001) as f32 / 1000.0 - 1.0)
                .collect();
            let mut y = vec![0.0f32; n * rows];
            let clave = format!("{rows}x{cols}n{n}");
            let ok = if n == 1 {
                gpu.matvec(&clave, &vista, rows, cols, &x, &mut y).unwrap()
            } else {
                gpu.matmul(&clave, &vista, rows, cols, n, &x, &mut y)
                    .unwrap()
            };
            assert!(ok);
            cerca(
                &y,
                &ref_q4_0(&w, rows, cols, &x, n),
                &format!("{rows}x{cols} n={n}"),
            );
        }
    }
}

/// Q4_K (los pesos de Mellum): se compara con la descuantización de `sosomodel`, que es la que usa la CPU.
#[test]
fn kernel_q4k_iguala_a_la_descuantizacion() {
    let mut gpu = match CudaGpu::nueva() {
        Ok(g) => g,
        Err(e) => {
            eprintln!("cuda_matvec: SALTADA, no hay CUDA ({e})");
            return;
        }
    };
    let mut e = 777u64;
    let (rows, cols) = (200usize, 2304usize);
    let mut w = Vec::new();
    for _ in 0..rows * cols / 256 {
        w.extend_from_slice(&ESCALAS_F16[azar(&mut e) as usize % 4].to_le_bytes());
        w.extend_from_slice(&ESCALAS_F16[azar(&mut e) as usize % 4].to_le_bytes());
        for _ in 0..140 {
            w.push(azar(&mut e) as u8);
        }
    }
    let mut ancho = vec![0.0f32; rows * cols];
    sosomodel::dequant::dequant_q4_k(&w, &mut ancho).unwrap();
    let vista = TensorView {
        bytes: &w,
        dtype: DTYPE_Q4_K,
        elems: rows * cols,
    };
    for n in [1usize, 5] {
        let x: Vec<f32> = (0..n * cols)
            .map(|_| (azar(&mut e) % 2001) as f32 / 1000.0 - 1.0)
            .collect();
        let mut y = vec![0.0f32; n * rows];
        let ok = if n == 1 {
            gpu.matvec("k", &vista, rows, cols, &x, &mut y).unwrap()
        } else {
            gpu.matmul("k", &vista, rows, cols, n, &x, &mut y).unwrap()
        };
        assert!(ok, "la GPU debía atender Q4_K n={n}");
        let r: Vec<f32> = (0..n * rows)
            .map(|i| {
                let (t, f) = (i / rows, i % rows);
                (0..cols)
                    .map(|c| ancho[f * cols + c] as f64 * x[t * cols + c] as f64)
                    .sum::<f64>() as f32
            })
            .collect();
        cerca(&y, &r, &format!("Q4_K n={n}"));
    }
    // Una forma que el kernel no admite (cols no múltiplo de 256) se rechaza sin error.
    let corto = TensorView {
        bytes: &w[..144],
        dtype: DTYPE_Q4_K,
        elems: 256,
    };
    let x = vec![0.0f32; 128];
    let mut y = vec![0.0f32; 2];
    assert!(!gpu.matvec("raro", &corto, 2, 128, &x, &mut y).unwrap());
}
