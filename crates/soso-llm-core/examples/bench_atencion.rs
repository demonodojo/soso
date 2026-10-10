//! T22: coste de la atención de decodificación (una consulta contra `tokens` posiciones de KV),
//! ruta rápida por tiles frente a la genérica que usa el planificador con H2O.
//!
//! ```sh
//! RUSTFLAGS="-C target-feature=+avx2,+fma" cargo run --release -p soso-llm-core --features std \
//!   --example bench_atencion -- 2000 40 8 128
//! ```
use std::time::Instant;

use soso_llm_core::attn::{attention_decode_f16_tiled, attention_decode_kv};
use soso_llm_core::attn_grupo::{attention_f16_group, attention_f16_tokens};
use soso_llm_core::kv::{KvDtype, LayerKv};

fn main() {
    let a: Vec<usize> = std::env::args().skip(1).filter_map(|v| v.parse().ok()).collect();
    let (tokens, heads, kv_heads, head_dim) = (
        a.first().copied().unwrap_or(2000),
        a.get(1).copied().unwrap_or(40),
        a.get(2).copied().unwrap_or(8),
        a.get(3).copied().unwrap_or(128),
    );
    let kv_dim = kv_heads * head_dim;
    let mut s = 1u64;
    let mut r = move || {
        s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((s >> 33) as f32 / (1u64 << 31) as f32) * 2.0 - 1.0
    };
    let mut kv = LayerKv::with_capacity_dtype(tokens + 8, kv_dim, KvDtype::F16);
    for _ in 0..tokens {
        let k: Vec<f32> = (0..kv_dim).map(|_| r()).collect();
        let v: Vec<f32> = (0..kv_dim).map(|_| r()).collect();
        kv.append_f16(&k, &v);
    }
    let q: Vec<f32> = (0..head_dim).map(|_| r()).collect();
    let group = heads / kv_heads;
    let mut out = vec![0.0f32; head_dim];
    let reps = 20;

    let t = Instant::now();
    for _ in 0..reps {
        for h in 0..heads {
            attention_decode_f16_tiled(&q, kv.k_f16_slice(), kv.v_f16_slice(), head_dim, kv_dim, h / group, tokens, &mut out);
        }
    }
    let rapida = t.elapsed().as_secs_f64() / reps as f64;

    let mut masa = vec![0.0f32; tokens];
    let t = Instant::now();
    for _ in 0..reps {
        for h in 0..heads {
            attention_decode_kv(&q, &kv, head_dim, kv_dim, h / group, tokens, &mut out, Some(&mut masa), false);
        }
    }
    let generica = t.elapsed().as_secs_f64() / reps as f64;

    // Kernel por grupos GQA (una consulta por vez) y por bloque de 4 tokens (decodificación / prefill).
    let mut qs = vec![0.0f32; 4 * heads * head_dim];
    for x in qs.iter_mut() {
        *x = r();
    }
    let mut outs = vec![0.0f32; 4 * heads * head_dim];
    let t = Instant::now();
    for _ in 0..reps {
        for kvh in 0..kv_heads {
            attention_f16_group(&qs[kvh * group * head_dim..], group, kv.k_f16_slice(), kv.v_f16_slice(), head_dim, kv_dim, kvh, tokens, &mut outs[kvh * group * head_dim..]);
        }
    }
    let grupo = t.elapsed().as_secs_f64() / reps as f64;
    let q_dim = heads * head_dim;
    let t = Instant::now();
    for _ in 0..reps {
        for kvh in 0..kv_heads {
            attention_f16_tokens(&qs[kvh * group * head_dim..], &mut outs[kvh * group * head_dim..], q_dim, 4, group, tokens - 3, kv.k_f16_slice(), kv.v_f16_slice(), head_dim, kv_dim, kvh);
        }
    }
    let bloque4 = t.elapsed().as_secs_f64() / reps as f64;
    let flops = 4.0 * tokens as f64 * head_dim as f64 * heads as f64;
    println!(
        "  grupos GQA (1 token): {:.3} ms ({:.1} GFLOP/s) · bloque de 4 tokens: {:.3} ms ({:.1} GFLOP/s por token-cabeza)",
        grupo * 1e3, flops / grupo / 1e9, bloque4 * 1e3, 4.0 * flops / bloque4 / 1e9
    );
    println!(
        "{tokens} tokens, {heads} cabezas ({kv_heads} KV) x {head_dim}: rápida {:.3} ms ({:.1} GFLOP/s) · genérica+H2O {:.3} ms ({:.1} GFLOP/s) · x{:.1}",
        rapida * 1e3, flops / rapida / 1e9, generica * 1e3, flops / generica / 1e9, generica / rapida
    );
}
