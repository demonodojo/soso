//! Atención por grupos GQA: exactitud frente a una referencia en f64 y, sobre todo, invariancia:
//! el resultado de una consulta no puede depender de cuántas más viajan con ella en el grupo.
//!
//! Los kernels vectoriales sólo existen si el test se compila con `-C target-feature=+avx2,+fma`;
//! sin eso se comprueba el camino escalar de reserva con las mismas aserciones.

use soso_llm_core::attn_grupo::attention_f16_group;
use soso_llm_core::f16::{f16_to_f32, f32_to_f16};

fn lcg(s: &mut u64) -> f32 {
    *s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    ((*s >> 33) as f32 / (1u64 << 31) as f32) * 2.0 - 1.0
}

struct Caso {
    k: Vec<u16>,
    v: Vec<u16>,
    kv_dim: usize,
}

fn caso(tokens: usize, kv_heads: usize, hd: usize, s: &mut u64) -> Caso {
    let kv_dim = kv_heads * hd;
    let k = (0..tokens * kv_dim).map(|_| f32_to_f16(lcg(s) * 3.0)).collect();
    let v = (0..tokens * kv_dim).map(|_| f32_to_f16(lcg(s))).collect();
    Caso { k, v, kv_dim }
}

/// Referencia en f64: softmax(q·k/√d)·v sobre los valores f16 ya redondeados.
fn referencia(q: &[f32], c: &Caso, hd: usize, kv_head: usize, n: usize) -> Vec<f64> {
    let inv = 1.0 / (hd as f64).sqrt();
    let sc: Vec<f64> = (0..n)
        .map(|t| {
            (0..hd)
                .map(|d| q[d] as f64 * f16_to_f32(c.k[t * c.kv_dim + kv_head * hd + d]) as f64)
                .sum::<f64>()
                * inv
        })
        .collect();
    let m = sc.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let e: Vec<f64> = sc.iter().map(|s| (s - m).exp()).collect();
    let z: f64 = e.iter().sum();
    (0..hd)
        .map(|d| {
            (0..n)
                .map(|t| e[t] / z * f16_to_f32(c.v[t * c.kv_dim + kv_head * hd + d]) as f64)
                .sum()
        })
        .collect()
}

#[test]
fn coincide_con_la_referencia_f64() {
    let mut s = 7u64;
    for hd in [64usize, 128] {
        for n in [1usize, 2, 7, 8, 9, 31, 32, 33, 64, 100, 257] {
            let c = caso(n, 2, hd, &mut s);
            let g = 5;
            let qs: Vec<f32> = (0..g * hd).map(|_| lcg(&mut s) * 2.0).collect();
            let mut out = vec![0.0f32; g * hd];
            attention_f16_group(&qs, g, &c.k, &c.v, hd, c.kv_dim, 1, n, &mut out);
            for j in 0..g {
                let r = referencia(&qs[j * hd..(j + 1) * hd], &c, hd, 1, n);
                let err = (0..hd).map(|d| (out[j * hd + d] as f64 - r[d]).abs()).fold(0.0, f64::max);
                assert!(err < 2e-5, "hd {hd}, {n} tokens, consulta {j}: error {err}");
            }
        }
    }
}

#[test]
fn el_resultado_no_depende_del_tamano_del_grupo() {
    let mut s = 99u64;
    for hd in [64usize, 128] {
        for n in [1usize, 5, 8, 33, 130, 300] {
            let c = caso(n, 4, hd, &mut s);
            for g in [1usize, 2, 5, 7, 8, 9, 12] {
                let qs: Vec<f32> = (0..g * hd).map(|_| lcg(&mut s) * 2.0).collect();
                let mut junto = vec![0.0f32; g * hd];
                attention_f16_group(&qs, g, &c.k, &c.v, hd, c.kv_dim, 2, n, &mut junto);
                for j in 0..g {
                    let mut solo = vec![0.0f32; hd];
                    attention_f16_group(&qs[j * hd..(j + 1) * hd], 1, &c.k, &c.v, hd, c.kv_dim, 2, n, &mut solo);
                    assert_eq!(
                        &junto[j * hd..(j + 1) * hd],
                        &solo[..],
                        "hd {hd}, {n} tokens, g {g}: la consulta {j} cambia según el grupo"
                    );
                }
            }
        }
    }
}

#[test]
fn un_prefijo_da_lo_mismo_que_una_secuencia_mas_corta() {
    // El prefill por bloques atiende a `pos0 + i + 1` posiciones del mismo KV: leer sólo el prefijo.
    let mut s = 5u64;
    let hd = 128;
    let largo = caso(200, 2, hd, &mut s);
    let n = 77;
    let corto = Caso {
        k: largo.k[..n * largo.kv_dim].to_vec(),
        v: largo.v[..n * largo.kv_dim].to_vec(),
        kv_dim: largo.kv_dim,
    };
    let qs: Vec<f32> = (0..3 * hd).map(|_| lcg(&mut s)).collect();
    let (mut a, mut b) = (vec![0.0f32; 3 * hd], vec![0.0f32; 3 * hd]);
    attention_f16_group(&qs, 3, &largo.k, &largo.v, hd, largo.kv_dim, 0, n, &mut a);
    attention_f16_group(&qs, 3, &corto.k, &corto.v, hd, corto.kv_dim, 0, n, &mut b);
    assert_eq!(a, b);
}

#[test]
fn puntuaciones_grandes_no_desbordan() {
    let mut s = 3u64;
    let hd = 128;
    let c = caso(64, 1, hd, &mut s);
    let qs: Vec<f32> = (0..hd).map(|_| lcg(&mut s) * 40.0).collect();
    let mut out = vec![0.0f32; hd];
    attention_f16_group(&qs, 1, &c.k, &c.v, hd, c.kv_dim, 0, 64, &mut out);
    assert!(out.iter().all(|x| x.is_finite()));
    let r = referencia(&qs, &c, hd, 0, 64);
    let err = (0..hd).map(|d| (out[d] as f64 - r[d]).abs()).fold(0.0, f64::max);
    assert!(err < 1e-3, "error {err}");
}

use soso_llm_core::attn_grupo::attention_f16_tokens;

#[test]
fn un_bloque_de_tokens_da_a_cada_consulta_lo_que_daria_sola() {
    let mut s = 1234u64;
    for hd in [64usize, 128] {
        let kv_heads = 3;
        let total = 200;
        let c = caso(total, kv_heads, hd, &mut s);
        for g in [1usize, 5, 7] {
            for ntok in [1usize, 3, 8, 9, 20] {
                for seq_primero in [1usize, 5, 31, 32, 33, 100] {
                    if seq_primero + ntok > total {
                        continue;
                    }
                    let q_dim = kv_heads * g * hd;
                    let kvh = 1;
                    let qs: Vec<f32> = (0..ntok * q_dim).map(|_| lcg(&mut s) * 2.0).collect();
                    let mut junto = vec![0.0f32; ntok * q_dim];
                    let base = kvh * g * hd;
                    attention_f16_tokens(
                        &qs[base..],
                        &mut junto[base..],
                        q_dim,
                        ntok,
                        g,
                        seq_primero,
                        &c.k,
                        &c.v,
                        hd,
                        c.kv_dim,
                        kvh,
                    );
                    for i in 0..ntok {
                        for j in 0..g {
                            let off = i * q_dim + base + j * hd;
                            let mut solo = vec![0.0f32; hd];
                            attention_f16_group(&qs[off..off + hd], 1, &c.k, &c.v, hd, c.kv_dim, kvh, seq_primero + i, &mut solo);
                            assert_eq!(
                                &junto[off..off + hd],
                                &solo[..],
                                "hd {hd}, g {g}, {ntok} tokens desde {seq_primero}: token {i} cabeza {j} cambia en el bloque"
                            );
                        }
                    }
                }
            }
        }
    }
}
