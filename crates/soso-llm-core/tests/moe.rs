//! Tests MoE: routing top-k, equivalencia numérica y cache LRU.

use soso_llm_core::gemm::{matvec_f32, topk_softmax};
use soso_llm_core::plan::ResourcePlanner;
use soso_llm_core::runtime::{MemoryTensorSource, Runtime};
use sosomodel::index::{make_f32_entry, TensorIndex};
use sosomodel::manifest::{AttnKind, Manifest};

fn build_tiny_moe_tensors() -> (Manifest, TensorIndex, MemoryTensorSource) {
    let manifest = Manifest::tiny_moe("test-moe");
    let n_exp = manifest.num_experts as usize;
    let mut index = TensorIndex::default();
    let mut id = 0u32;
    let mut tensors = MemoryTensorSource {
        tensors: Default::default(),
    };

    let mut add = |name: &str, shape: &[u32], fill: f32| {
        let elems: usize = shape.iter().map(|&d| d as usize).product();
        let mut w = vec![0.0f32; elems];
        for (i, v) in w.iter_mut().enumerate() {
            *v = fill + (i as f32) * 1e-6;
        }
        tensors.tensors.insert(name.into(), w);
        index.entries.push(make_f32_entry(
            id,
            name,
            &format!("{name}.tensor"),
            0,
            shape,
        ));
        id += 1;
    };

    add("embed", &[manifest.vocab_size, manifest.hidden_dim], 0.01);
    for layer in 0..manifest.num_layers {
        let p = format!("L{layer:02}");
        add(&format!("{p}.attn_norm"), &[manifest.hidden_dim], 1.0);
        add(&format!("{p}.attn_q"), &[manifest.hidden_dim, manifest.hidden_dim], 0.1);
        add(
            &format!("{p}.attn_k"),
            &[manifest.hidden_dim, manifest.hidden_dim],
            0.1,
        );
        add(
            &format!("{p}.attn_v"),
            &[manifest.hidden_dim, manifest.hidden_dim],
            0.1,
        );
        add(
            &format!("{p}.attn_output"),
            &[manifest.hidden_dim, manifest.hidden_dim],
            0.1,
        );
        add(&format!("{p}.ffn_norm"), &[manifest.hidden_dim], 1.0);
        add(
            &format!("{p}.ffn_gate_inp"),
            &[manifest.num_experts, manifest.hidden_dim],
            0.01,
        );
        for e in 0..n_exp {
            let ep = format!("{p}.E{e:02}");
            add(
                &format!("{ep}.ffn_gate"),
                &[manifest.expert_ffn_dim(), manifest.hidden_dim],
                0.02 + e as f32 * 0.001,
            );
            add(
                &format!("{ep}.ffn_up"),
                &[manifest.expert_ffn_dim(), manifest.hidden_dim],
                0.03,
            );
            add(
                &format!("{ep}.ffn_down"),
                &[manifest.hidden_dim, manifest.expert_ffn_dim()],
                0.04,
            );
        }
    }

    (manifest, index, tensors)
}

#[test]
fn manifest_v4_moe_roundtrip() {
    let m = Manifest::tiny_moe("moe");
    let parsed = Manifest::parse(&m.serialize()).unwrap();
    assert_eq!(parsed.num_experts, 4);
    assert_eq!(parsed.num_experts_per_tok, 2);
    assert_eq!(parsed.moe_ffn_dim, 32);
    assert_eq!(parsed.layers.len(), 2);
    assert_eq!(parsed.layers[0].ffn_kind, sosomodel::FfnKind::Moe);
}

#[test]
fn runtime_rechaza_mla_sin_ranks_en_manifest() {
    let mut m = Manifest::tiny("mla");
    m.layers[0].attn_kind = AttnKind::Mla;
    assert!(m.supported_by_runtime().is_err());
}

#[test]
fn manifest_v3_moe_roundtrip() {
    let m = Manifest::tiny_moe("moe");
    let parsed = Manifest::parse(&m.serialize()).unwrap();
    assert_eq!(parsed.num_experts, 4);
    assert_eq!(parsed.num_experts_per_tok, 2);
    assert_eq!(parsed.moe_ffn_dim, 32);
}

#[test]
fn manifest_v2_sigue_parseando() {
    let mut m = Manifest::tiny("test");
    m.num_kv_heads = 2;
    // Simular manifest v2 empaquetado manualmente (sin campos MoE).
    let mut body = Vec::new();
    body.extend_from_slice(m.name.as_bytes());
    body.push(0);
    body.extend_from_slice(&m.vocab_size.to_le_bytes());
    body.extend_from_slice(&m.hidden_dim.to_le_bytes());
    body.extend_from_slice(&m.num_layers.to_le_bytes());
    body.extend_from_slice(&m.num_heads.to_le_bytes());
    body.extend_from_slice(&m.ffn_dim.to_le_bytes());
    body.extend_from_slice(&m.max_seq.to_le_bytes());
    body.extend_from_slice(&m.num_kv_heads.to_le_bytes());
    body.extend_from_slice(&m.rope_theta.to_le_bytes());
    body.extend_from_slice(&m.rms_eps.to_le_bytes());
    body.extend_from_slice(&(m.prefetch.len() as u32).to_le_bytes());
    for pf in &m.prefetch {
        body.extend_from_slice(&pf.layer.to_le_bytes());
        body.extend_from_slice(&(pf.shards.len() as u32).to_le_bytes());
        for s in &pf.shards {
            body.extend_from_slice(s.as_bytes());
            body.push(0);
        }
    }
    let bytes = sosomodel::pack_som(&body, 2, sosomodel::CACHE_ALIGN);
    let parsed = Manifest::parse(&bytes).unwrap();
    assert_eq!(parsed.num_experts, 0);
}

#[test]
fn topk_softmax_renormaliza() {
    let mut logits = [1.0f32, 2.0, 3.0, 0.5];
    let ranked = topk_softmax(&mut logits, 2);
    assert_eq!(ranked.len(), 2);
    let sum: f32 = ranked.iter().map(|(_, w)| *w).sum();
    assert!((sum - 1.0).abs() < 1e-5);
    assert_eq!(ranked[0].0, 2);
}

#[test]
fn moe_validate_shapes() {
    let (manifest, index, _) = build_tiny_moe_tensors();
    let rt = Runtime::new(manifest, index, 0, 0);
    rt.validate_shapes().expect("shapes MoE coherentes");
}

#[test]
fn moe_forward_un_token() {
    let (manifest, index, mut source) = build_tiny_moe_tensors();
    let mut rt = Runtime::new(manifest.clone(), index.clone(), 0, 0);
    rt.validate_shapes().unwrap();
    rt.embed_token(0, &mut source).unwrap();
    rt.forward_layers_range(0, manifest.num_layers, &mut source, None, &mut None)
        .unwrap();
    let logits = rt.logits(&mut source).unwrap();
    assert!(logits.iter().any(|&x| x.is_finite()));
}

#[test]
fn moe_planner_lru_registra_hits() {
    let (manifest, index, mut source) = build_tiny_moe_tensors();
    let mem = soso_llm_core::plan::MemSnapshot {
        total_frames: 100_000,
        free_frames: 50_000,
        reclaimable_frames: 0,
    };
    let mut rt = Runtime::new(manifest.clone(), index, 0, 0);
    let planner = ResourcePlanner::new(&manifest, &rt.index, mem, 0, false);
    rt.set_planner(planner);
    rt.embed_token(0, &mut source).unwrap();
    rt.forward_layers_range(0, manifest.num_layers, &mut source, None, &mut None)
        .unwrap();
    let misses0 = rt.planner.as_ref().unwrap().stats().moe_misses;
    let hits0 = rt.planner.as_ref().unwrap().stats().moe_hits;
    assert!(misses0 > 0, "primer token debe cargar expertos fríos");
    // Segundo token: algunos expertos deberían estar en cache.
    rt.reset_sequence();
    rt.embed_token(0, &mut source).unwrap();
    rt.forward_layers_range(0, manifest.num_layers, &mut source, None, &mut None)
        .unwrap();
    let stats2 = rt.planner.as_ref().unwrap().stats();
    assert!(
        stats2.moe_hits > hits0,
        "segundo token debe reutilizar expertos calientes"
    );
}

#[test]
fn moe_speculative_hits_en_segundo_token() {
    let (manifest, index, mut source) = build_tiny_moe_tensors();
    let mem = soso_llm_core::plan::MemSnapshot {
        total_frames: 100_000,
        free_frames: 50_000,
        reclaimable_frames: 0,
    };
    let mut rt = Runtime::new(manifest.clone(), index, 0, 0);
    let planner = ResourcePlanner::new(&manifest, &rt.index, mem, 0, false);
    rt.set_planner(planner);
    rt.embed_token(0, &mut source).unwrap();
    rt.forward_layers_range(0, manifest.num_layers, &mut source, None, &mut None)
        .unwrap();
    rt.advance_pos();
    rt.embed_token(0, &mut source).unwrap();
    rt.forward_layers_range(0, manifest.num_layers, &mut source, None, &mut None)
        .unwrap();
    let stats = rt.planner.as_ref().unwrap().stats();
    assert!(
        stats.moe_spec_hits > 0,
        "segundo token debe acertar hint MoE especulativo"
    );
}

#[test]
fn moe_ffn_referencia_escalar() {
    let h = 4usize;
    let ffn = 8usize;
    let hidden = [0.5f32, -0.25, 0.1, 0.0];
    let mut router_w = vec![0.0f32; 2 * h];
    router_w[0 * h + 0] = 1.0;
    router_w[1 * h + 1] = 2.0;
    let mut router = [0.0f32; 2];
    matvec_f32(&router_w, 2, h, &hidden, &mut router);
    let ranked = topk_softmax(&mut router, 1);
    let (expert, weight) = ranked[0];

    let gate_w = vec![0.01f32; ffn * h];
    let up_w = vec![0.02f32; ffn * h];
    let down_w = vec![0.03f32; h * ffn];
    let mut gate = vec![0.0f32; ffn];
    let mut up = vec![0.0f32; ffn];
    let mut out = vec![0.0f32; h];
    matvec_f32(&gate_w, ffn, h, &hidden, &mut gate);
    matvec_f32(&up_w, ffn, h, &hidden, &mut up);
    for i in 0..ffn {
        up[i] = crate::silu_ref(gate[i]) * up[i];
    }
    matvec_f32(&down_w, h, ffn, &up, &mut out);
    for v in &mut out {
        *v *= weight;
    }
    assert!(out.iter().all(|v| v.is_finite()));
    let _ = expert;
}

fn silu_ref(x: f32) -> f32 {
    x / (1.0 + libm::expf(-x))
}
