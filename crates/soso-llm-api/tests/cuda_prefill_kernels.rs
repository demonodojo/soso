//! Kernels del prefill de Qwen3.6 contra una referencia en f64: el producto f32 del router (`sgemm_nt64`), la norma con residual del hidden residente
//! (`res_suma_rms`) y la recurrencia GDN (`gdn_rec`). Sin GPU las pruebas se saltan.
#![cfg(feature = "std")]

use soso_llm_api::cuda::CudaGpu;

fn azar(e: &mut u64) -> u32 {
    *e ^= *e << 13;
    *e ^= *e >> 7;
    *e ^= *e << 17;
    (*e >> 32) as u32
}

/// Uniforme en [-a, a].
fn uni(e: &mut u64, a: f32) -> f32 {
    ((azar(e) % 20001) as f32 / 10000.0 - 1.0) * a
}

fn gpu() -> Option<CudaGpu> {
    match CudaGpu::nueva() {
        Ok(g) => Some(g),
        Err(e) => {
            eprintln!("cuda_prefill_kernels: SALTADA, no hay CUDA ({e})");
            None
        }
    }
}

#[test]
fn sgemm_del_router_iguala_a_la_referencia() {
    let Some(mut g) = gpu() else { return };
    // filas y tokens que no son múltiplo de los tiles (64 × 32); K múltiplo de 16
    for (rows, k, n) in [(256usize, 2048usize, 70usize), (100, 96, 33), (64, 16, 32), (257, 2048, 1)] {
        let mut e = 0x9E37_79B9_7F4A_7C15u64 ^ (rows * 31 + k * 7 + n) as u64;
        let w: Vec<f32> = (0..rows * k).map(|_| uni(&mut e, 0.5)).collect();
        let x: Vec<f32> = (0..n * k).map(|_| uni(&mut e, 1.0)).collect();
        let y = g.probar_sgemm(&w, rows, k, &x, n).expect("sgemm");
        let mut peor = 0f64;
        for t in 0..n {
            for r in 0..rows {
                let s: f64 = (0..k).map(|i| w[r * k + i] as f64 * x[t * k + i] as f64).sum();
                peor = peor.max((y[t * rows + r] as f64 - s).abs() / (1.0 + s.abs()));
            }
        }
        eprintln!("sgemm {rows}x{k} n={n}: peor error relativo {peor:.2e}");
        assert!(peor < 1e-4, "sgemm {rows}x{k} n={n}: {peor}");
    }
}

#[test]
fn norma_con_residual_iguala_a_la_referencia() {
    let Some(mut g) = gpu() else { return };
    for (n, h, sumar) in [(37usize, 2048usize, true), (5, 2048, false), (3, 300, true)] {
        let mut e = 0xD1B5_4A32_D192_ED03u64 ^ (n * 131 + h) as u64;
        let eps = 1e-6f32;
        let r: Vec<f32> = (0..n * h).map(|_| uni(&mut e, 3.0)).collect();
        let y: Vec<f32> = (0..n * h).map(|_| uni(&mut e, 1.0)).collect();
        let w: Vec<f32> = (0..h).map(|_| 0.5 + uni(&mut e, 0.5).abs()).collect();
        let (ro, xo) = g.probar_res_rms(&r, &y, &w, n, h, eps, sumar).expect("res_rms");
        for t in 0..n {
            let fila: Vec<f64> = (0..h).map(|i| r[t * h + i] as f64 + if sumar { y[t * h + i] as f64 } else { 0.0 }).collect();
            let ss: f64 = fila.iter().map(|v| v * v).sum();
            let escala = 1.0 / (ss / h as f64 + eps as f64).sqrt();
            for i in 0..h {
                assert!((ro[t * h + i] as f64 - fila[i]).abs() < 1e-5 * (1.0 + fila[i].abs()), "residual t={t} i={i}");
                let esp = fila[i] * escala * w[i] as f64;
                assert!((xo[t * h + i] as f64 - esp).abs() < 1e-4 * (1.0 + esp.abs()), "norma t={t} i={i}: {} vs {esp}", xo[t * h + i]);
            }
        }
    }
}

/// Referencia de la recurrencia GDN por cabeza V (misma matemática que el kernel).
#[allow(clippy::too_many_arguments)]
fn gdn_ref(up: &[f32], z: &[f32], decay: &[f32], beta: &[f32], nw: &[f32], estado: &mut [f64], n: usize, n_k: usize, n_v: usize, eps: f64) -> Vec<f64> {
    let (qk_dim, v_dim) = (n_k * 128, n_v * 128);
    let qkv_dim = 2 * qk_dim + v_dim;
    let mut out = vec![0f64; n * v_dim];
    for vh in 0..n_v {
        let kh = vh % n_k;
        let s = &mut estado[vh * 128 * 128..(vh + 1) * 128 * 128];
        for t in 0..n {
            let base = t * qkv_dim;
            let q: Vec<f64> = (0..128).map(|i| up[base + kh * 128 + i] as f64).collect();
            let k: Vec<f64> = (0..128).map(|i| up[base + qk_dim + kh * 128 + i] as f64).collect();
            let v: Vec<f64> = (0..128).map(|i| up[base + 2 * qk_dim + vh * 128 + i] as f64).collect();
            let (sq, sk): (f64, f64) = (q.iter().map(|x| x * x).sum(), k.iter().map(|x| x * x).sum());
            let (iq, ik) = (1.0 / (sq + eps).sqrt() / (128f64).sqrt(), 1.0 / (sk + eps).sqrt());
            let (qn, kn): (Vec<f64>, Vec<f64>) = (q.iter().map(|x| x * iq).collect(), k.iter().map(|x| x * ik).collect());
            let (g, b) = (decay[t * n_v + vh] as f64, beta[t * n_v + vh] as f64);
            let mut o = vec![0f64; 128];
            for col in 0..128 {
                let kv: f64 = (0..128).map(|r| s[r * 128 + col] * kn[r]).sum();
                let delta = (v[col] - g * kv) * b;
                let mut acc = 0.0;
                for r in 0..128 {
                    s[r * 128 + col] = g * s[r * 128 + col] + kn[r] * delta;
                    acc += s[r * 128 + col] * qn[r];
                }
                o[col] = acc;
            }
            let ss: f64 = o.iter().map(|x| x * x).sum();
            let inv = 1.0 / (ss / 128.0 + eps).sqrt();
            for col in 0..128 {
                let zz = z[t * v_dim + vh * 128 + col] as f64;
                out[t * v_dim + vh * 128 + col] = o[col] * inv * nw[col] as f64 * (zz / (1.0 + (-zz).exp()));
            }
        }
    }
    out
}

#[test]
fn recurrencia_gdn_iguala_a_la_referencia() {
    let Some(mut g) = gpu() else { return };
    // n impar y de un solo token: ejercitan la precarga del token siguiente y su borde
    for (n, n_k, n_v) in [(37usize, 2usize, 4usize), (1, 2, 4), (64, 1, 2)] {
        let mut e = 0xA5A5_5A5A_1234_4321u64 ^ (n * 17 + n_v) as u64;
        let (qk_dim, v_dim) = (n_k * 128, n_v * 128);
        let qkv_dim = 2 * qk_dim + v_dim;
        let up: Vec<f32> = (0..n * qkv_dim).map(|_| uni(&mut e, 1.5)).collect();
        let z: Vec<f32> = (0..n * v_dim).map(|_| uni(&mut e, 2.0)).collect();
        let decay: Vec<f32> = (0..n * n_v).map(|_| 0.8 + (azar(&mut e) % 1000) as f32 / 5000.0).collect();
        let beta: Vec<f32> = (0..n * n_v).map(|_| (azar(&mut e) % 1000) as f32 / 1000.0).collect();
        let nw: Vec<f32> = (0..128).map(|_| 0.5 + (azar(&mut e) % 1000) as f32 / 1000.0).collect();
        let s0: Vec<f32> = (0..n_v * 128 * 128).map(|_| uni(&mut e, 0.1)).collect();
        let mut s_gpu = s0.clone();
        let eps = 1e-6f32;
        let out = g.probar_gdn_rec(&up, &z, &decay, &beta, &nw, &mut s_gpu, n, n_k, n_v, eps).expect("gdn_rec");
        let mut s_ref: Vec<f64> = s0.iter().map(|&v| v as f64).collect();
        let esp = gdn_ref(&up, &z, &decay, &beta, &nw, &mut s_ref, n, n_k, n_v, eps as f64);
        let (mut err2, mut ref2) = (0f64, 0f64);
        for (a, b) in out.iter().zip(&esp) {
            err2 += (*a as f64 - b).powi(2);
            ref2 += b * b;
        }
        let (mut errs, mut refs) = (0f64, 0f64);
        for (a, b) in s_gpu.iter().zip(&s_ref) {
            errs += (*a as f64 - b).powi(2);
            refs += b * b;
        }
        let (rel, rels) = ((err2 / ref2.max(1e-30)).sqrt(), (errs / refs.max(1e-30)).sqrt());
        eprintln!("gdn_rec n={n} n_k={n_k} n_v={n_v}: salida {rel:.2e}, estado {rels:.2e}");
        assert!(rel < 1e-3, "salida: {rel}");
        assert!(rels < 1e-3, "estado: {rels}");
    }
}
