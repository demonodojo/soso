//! Oráculo estadístico independiente para dos tokens con PLD determinista.
#![cfg(feature = "std")]

use soso_llm_core::{generation::GenerationOptions, runtime::{MemoryTensorSource, Runtime}, sample::Sampler};
use sosomodel::{index::{make_f32_entry, TensorIndex}, manifest::Manifest};
use std::collections::BTreeMap;

fn modelo() -> (Runtime, MemoryTensorSource) {
    let mut m = Manifest::tiny("estadistico");
    m.hidden_dim = 4;
    m.ffn_dim = 4;
    m.num_layers = 1;
    m.num_heads = 1;
    m.num_kv_heads = 1;
    m.vocab_size = 3;
    m.fill_layers_from_globals();
    let mut tensors = BTreeMap::new();
    let mut index = TensorIndex::default();
    let mut insertar = |nombre: &str, shape: &[u32], valores: Vec<f32>| {
        index.entries.push(make_f32_entry(index.entries.len() as u32, nombre, nombre, 0, shape));
        tensors.insert(nombre.to_owned(), valores);
    };
    for nombre in ["attn_norm", "ffn_norm"] {
        insertar(&format!("L00.{nombre}"), &[4], vec![1.0; 4]);
    }
    for nombre in ["attn_q", "attn_k", "attn_v", "attn_output", "ffn_up", "ffn_down"] {
        insertar(&format!("L00.{nombre}"), &[4, 4], vec![0.0; 16]);
    }
    insertar("embed", &[3, 4], vec![1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0]);
    insertar("lm_head", &[3, 4], vec![0.5f32.ln(), 0.0, 0.0, 0.0,
                                     0.3f32.ln(), 0.0, 0.0, 0.0,
                                     0.2f32.ln(), 0.0, 0.0, 0.0]);
    let rt = Runtime::new(m, index, 0, 0);
    (rt, MemoryTensorSource { tensors })
}

fn mezclar(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

#[test]
fn distribucion_de_dos_tokens_con_y_sin_borrador() {
    const N: usize = 20_000;
    let prompt = [0u32, 0, 0, 0];
    for (top_p, probs, umbral) in [(1.0, [0.5, 0.3, 0.2], 32.0), (0.7, [0.625, 0.375, 0.0], 22.0)] {
        let mut conteos = [[0usize; 3]; 3];
        let mut ofrecidos = 0u64;
        let mut aceptados = 0u64;
        let (mut rt, mut src) = modelo();
        rt.validate_shapes().expect("modelo estadístico");
        for i in 0..N {
            let ejecutar = |rt: &mut Runtime, src: &mut MemoryTensorSource, activo| {
                rt.reset_sequence();
                rt.borrador_activo = activo;
                let mut sampler = Sampler::new(1.0, top_p, mezclar(i as u64));
                sampler.top_k = 0;
                rt.generate_stream_par_with_report(src, &prompt, GenerationOptions::new(2, vec![]),
                    &mut sampler, |_| {}, None, &mut None, &mut |_, _| {}).unwrap().0
            };
            let antes_o = rt.borrador_ofrecidos;
            let antes_a = rt.borrador_aceptados;
            let espec = ejecutar(&mut rt, &mut src, true);
            ofrecidos += rt.borrador_ofrecidos - antes_o;
            aceptados += rt.borrador_aceptados - antes_a;
            let exacto = ejecutar(&mut rt, &mut src, false);
            assert_eq!(espec, exacto, "semilla {i}, top_p={top_p}");
            conteos[espec[4] as usize][espec[5] as usize] += 1;
        }
        assert!(aceptados > 1000 && ofrecidos > aceptados + 1000, "ofrecidos {ofrecidos}, aceptados {aceptados}");
        let mut chi2 = 0.0;
        for a in 0..3 {
            for b in 0..3 {
                let esperado = N as f64 * probs[a] * probs[b];
                if esperado == 0.0 { assert_eq!(conteos[a][b], 0); continue; }
                chi2 += (conteos[a][b] as f64 - esperado).powi(2) / esperado;
            }
        }
        eprintln!("top_p={top_p}: χ²={chi2:.3}, ofrecidos={ofrecidos}, aceptados={aceptados}, conteos={conteos:?}");
        // Umbrales superiores al percentil 99,99 % de χ² para 8 y 3 grados.
        assert!(chi2 < umbral, "top_p={top_p}: χ²={chi2}, conteos={conteos:?}");
    }
}
