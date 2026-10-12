//! Atención de Gemma 4: coincide con una referencia ingenua en f64 para cualquier `head_dim`, escala
//! y ventana, y respeta la cabeza KV elegida.

use soso_llm_core::attn_gemma::{attention_gqa_f16, f16_a_f32};
use soso_llm_core::f16::{f16_to_f32, f32_to_f16};

fn rnd(s: &mut u64) -> f32 {
    *s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    (((*s >> 33) as u32 % 2001) as f32 - 1000.0) * 1e-3
}

/// Referencia en f64 sobre los mismos valores ya redondeados a f16.
fn referencia(q: &[f32], g: usize, k: &[u16], v: &[u16], hd: usize, kv_dim: usize, kvh: usize, t0: usize, t1: usize, scale: f32) -> Vec<f64> {
    let mut out = vec![0f64; g * hd];
    if t0 >= t1 {
        return out;
    }
    for j in 0..g {
        let sc: Vec<f64> = (t0..t1)
            .map(|t| (0..hd).map(|d| q[j * hd + d] as f64 * f16_to_f32(k[t * kv_dim + kvh * hd + d]) as f64).sum::<f64>() * scale as f64)
            .collect();
        let m = sc.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        let e: Vec<f64> = sc.iter().map(|s| (s - m).exp()).collect();
        let z: f64 = e.iter().sum();
        for (i, t) in (t0..t1).enumerate() {
            for d in 0..hd {
                out[j * hd + d] += e[i] / z * f16_to_f32(v[t * kv_dim + kvh * hd + d]) as f64;
            }
        }
    }
    out
}

fn caso(g: usize, hd: usize, kv_heads: usize, kvh: usize, n: usize, t0: usize, t1: usize, scale: f32, sem: u64) {
    let mut s = sem;
    let kv_dim = kv_heads * hd;
    let q: Vec<f32> = (0..g * hd).map(|_| rnd(&mut s) * 3.0).collect();
    let k: Vec<u16> = (0..n * kv_dim).map(|_| f32_to_f16(rnd(&mut s) * 2.0)).collect();
    let v: Vec<u16> = (0..n * kv_dim).map(|_| f32_to_f16(rnd(&mut s))).collect();
    let mut out = vec![7.0f32; g * hd];
    attention_gqa_f16(&q, g, &k, &v, hd, kv_dim, kvh, t0, t1, scale, &mut out);
    let esp = referencia(&q, g, &k, &v, hd, kv_dim, kvh, t0, t1, scale);
    for (i, (a, b)) in out.iter().zip(&esp).enumerate() {
        assert!((*a as f64 - b).abs() < 2e-4 + b.abs() * 2e-4, "g={g} hd={hd} kvh={kvh} [{t0},{t1}) escala {scale}: elemento {i}: {a} vs {b}");
    }
}

#[test]
fn coincide_con_la_referencia_para_las_formas_de_gemma() {
    // Local: 2 cabezas Q por cabeza KV, head_dim 256, 8 cabezas KV. Global: 16 por 1, head_dim 512.
    caso(2, 256, 8, 0, 40, 0, 40, 1.0, 1);
    caso(2, 256, 8, 5, 40, 0, 40, 1.0, 2);
    caso(16, 512, 1, 0, 33, 0, 33, 1.0, 3);
    // Otras dimensiones y la escala clásica.
    caso(4, 64, 2, 1, 50, 0, 50, 1.0 / 8.0, 4);
    caso(1, 8, 1, 0, 9, 0, 9, 0.5, 5);
    caso(3, 20, 2, 1, 17, 0, 17, 1.0, 6); // head_dim que no es múltiplo de 8
}

#[test]
fn la_ventana_solo_mira_las_posiciones_pedidas() {
    caso(2, 256, 8, 3, 100, 60, 100, 1.0, 7);
    caso(16, 512, 1, 0, 100, 76, 100, 1.0, 8);
    caso(2, 32, 2, 0, 10, 9, 10, 1.0, 9); // una sola posición: la salida es su V
    // Lo de fuera de la ventana no influye: cambiar K/V antes de t0 no cambia nada.
    let (g, hd, kvh, n) = (2usize, 16usize, 2usize, 30usize);
    let mut s = 11;
    let kv_dim = kvh * hd;
    let q: Vec<f32> = (0..g * hd).map(|_| rnd(&mut s)).collect();
    let k: Vec<u16> = (0..n * kv_dim).map(|_| f32_to_f16(rnd(&mut s))).collect();
    let v: Vec<u16> = (0..n * kv_dim).map(|_| f32_to_f16(rnd(&mut s))).collect();
    let mut a = vec![0f32; g * hd];
    attention_gqa_f16(&q, g, &k, &v, hd, kv_dim, 1, 20, 30, 1.0, &mut a);
    let (mut k2, mut v2) = (k.clone(), v.clone());
    for x in k2[..20 * kv_dim].iter_mut().chain(v2[..20 * kv_dim].iter_mut()) {
        *x = f32_to_f16(123.0);
    }
    let mut b = vec![0f32; g * hd];
    attention_gqa_f16(&q, g, &k2, &v2, hd, kv_dim, 1, 20, 30, 1.0, &mut b);
    assert_eq!(a, b);
}

#[test]
fn sin_nada_que_atender_la_salida_es_cero() {
    let mut out = vec![5.0f32; 8];
    attention_gqa_f16(&[1.0; 8], 1, &[0; 16], &[0; 16], 8, 8, 0, 3, 3, 1.0, &mut out);
    assert!(out.iter().all(|&x| x == 0.0));
}

#[test]
fn la_conversion_f16_vectorial_coincide_con_la_escalar() {
    let src: Vec<u16> = (0..200u32).map(|i| f32_to_f16(((i as f32) - 100.0) * 0.37)).chain([0x0001, 0x7c00, 0xfc00, 0x0000, 0x8000, 0x03ff, 0x0400, 0x7bff].into_iter()).collect();
    let mut a = vec![0f32; src.len()];
    f16_a_f32(&src, &mut a);
    for (i, x) in src.iter().enumerate() {
        let r = f16_to_f32(*x);
        assert!(a[i].to_bits() == r.to_bits() || (a[i].is_nan() && r.is_nan()), "{i}: {} vs {}", a[i], r);
    }
}
