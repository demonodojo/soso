//! Las dos rutas de atención de decodificación (rápida por tiles en f16 y la genérica de
//! `attention_decode_kv`, que usa el planificador) deben dar lo mismo cuando no hay dispersa.

use soso_llm_core::attn::{attention_decode_f16_tiled, attention_decode_kv};
use soso_llm_core::kv::{KvDtype, LayerKv};

fn lcg(estado: &mut u64) -> f32 {
    *estado = estado.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    ((*estado >> 33) as f32 / (1u64 << 31) as f32) * 2.0 - 1.0
}

fn comparar(tokens: usize, kv_heads: usize, head_dim: usize, heads: usize) -> f32 {
    let kv_dim = kv_heads * head_dim;
    let mut s = 12345u64;
    let mut kv = LayerKv::with_capacity_dtype(tokens + 8, kv_dim, KvDtype::F16);
    for _ in 0..tokens {
        let k: Vec<f32> = (0..kv_dim).map(|_| lcg(&mut s) * 2.0).collect();
        let v: Vec<f32> = (0..kv_dim).map(|_| lcg(&mut s)).collect();
        kv.append_f16(&k, &v);
    }
    let group = heads / kv_heads;
    let mut peor = 0.0f32;
    for head in 0..heads {
        let q: Vec<f32> = (0..head_dim).map(|_| lcg(&mut s) * 2.0).collect();
        let kv_head = head / group;
        let mut a = vec![0.0f32; head_dim];
        let mut b = vec![0.0f32; head_dim];
        attention_decode_f16_tiled(
            &q, kv.k_f16_slice(), kv.v_f16_slice(), head_dim, kv_dim, kv_head, tokens, &mut a,
        );
        attention_decode_kv(&q, &kv, head_dim, kv_dim, kv_head, tokens, &mut b, None, false);
        for d in 0..head_dim {
            peor = peor.max((a[d] - b[d]).abs());
        }
    }
    peor
}

#[test]
fn rutas_de_atencion_equivalentes_con_gqa() {
    // Forma del Qwen2.5-Coder-7B: 28 cabezas de 128, 4 cabezas KV.
    for tokens in [1usize, 33, 300, 4151] {
        let e = comparar(tokens, 4, 128, 28);
        assert!(e < 1e-3, "{tokens} tokens: error máximo {e}");
    }
}
