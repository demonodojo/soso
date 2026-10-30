//! Atención de prefill con tensor cores (`attn_prefill_mma*`) contra una atención causal de referencia en CPU, con datos aleatorios, tamaños que no
//! son múltiplo de los tiles y las dos formas de cabeza que admite (hd 256 con group 8, hd 128 con group 4). Sin GPU la prueba se salta.
#![cfg(feature = "std")]

use soso_llm_api::cuda::CudaGpu;
use soso_llm_core::f16::{f16_to_f32, f32_to_f16};
use soso_llm_core::gpu::GpuDispatch;

fn xorshift(s: &mut u64) -> f32 {
    *s ^= *s << 13;
    *s ^= *s >> 7;
    *s ^= *s << 17;
    ((*s >> 40) as f32 / (1u64 << 24) as f32) * 2.0 - 1.0
}

fn probar(heads: usize, kv_heads: usize, hd: usize, n: usize, seq_antes: usize) {
    let mut gpu = match CudaGpu::nueva() {
        Ok(g) => g,
        Err(e) => {
            eprintln!("sin CUDA ({e}): prueba omitida");
            return;
        }
    };
    let (group, kv_dim, q_dim) = (heads / kv_heads, kv_heads * hd, heads * hd);
    let total = seq_antes + n;
    let mut s = 0x9E3779B97F4A7C15u64 ^ (hd as u64) << 20 ^ n as u64;
    let q: Vec<f32> = (0..n * q_dim).map(|_| xorshift(&mut s) * 3.0).collect();
    let k: Vec<u16> = (0..total * kv_dim).map(|_| f32_to_f16(xorshift(&mut s) * 3.0)).collect();
    let v: Vec<u16> = (0..total * kv_dim).map(|_| f32_to_f16(xorshift(&mut s))).collect();
    let mut out = vec![0f32; n * q_dim];
    let mut filas = 0usize;
    let ok = gpu.atencion_prefill(7, &q, &mut out, n, seq_antes, heads, kv_heads, hd, &k, &v, &mut filas).expect("atención");
    assert!(ok, "el despacho rechazó la forma");
    // referencia
    let escala = 1.0 / (hd as f32).sqrt();
    let (mut err2, mut ref2) = (0f64, 0f64);
    for t in 0..n {
        let lim = seq_antes + t + 1;
        for hq in 0..heads {
            let kh = hq / group;
            let qv = &q[t * q_dim + hq * hd..t * q_dim + (hq + 1) * hd];
            let mut sc: Vec<f32> = (0..lim)
                .map(|p| (0..hd).map(|d| qv[d] * f16_to_f32(k[p * kv_dim + kh * hd + d])).sum::<f32>() * escala)
                .collect();
            let m = sc.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
            let mut l = 0f32;
            for x in sc.iter_mut() {
                *x = (*x - m).exp();
                l += *x;
            }
            for d in 0..hd {
                let r: f32 = (0..lim).map(|p| sc[p] * f16_to_f32(v[p * kv_dim + kh * hd + d])).sum::<f32>() / l;
                let g = out[t * q_dim + hq * hd + d];
                err2 += ((g - r) as f64).powi(2);
                ref2 += (r as f64).powi(2);
            }
        }
    }
    let rel = (err2 / ref2.max(1e-30)).sqrt();
    eprintln!("hd {hd} group {group} n {n} seq_antes {seq_antes}: error relativo rms {rel:.5}");
    // Q y P van en f16: ~1e-3; un fallo de indexado o de máscara daría ~1
    assert!(rel < 0.01, "error relativo {rel}");
}

#[test]
fn atencion_mma_hd256_group8() {
    probar(16, 2, 256, 70, 45);
    probar(16, 2, 256, 1, 0);
    probar(16, 2, 256, 200, 0);
}

#[test]
fn atencion_mma_hd128_group4() {
    probar(8, 2, 128, 70, 45);
    probar(8, 2, 128, 33, 100);
}
