//! Tests arquitecturas extendidas: shared experts, MLA, LatentMoE, MXFP4, flags.
//!
//! Requiere `--features std`.

use soso_llm_core::layer::TensorSource;
use soso_llm_core::plan::{classify_weight_bytes, is_shared_expert_tensor, kv_bytes_per_token};
use soso_llm_core::quant::{dequant_mxfp4, quantize_mxfp4};
use soso_llm_core::runtime::{MemoryTensorSource, Runtime};
use soso_llm_core::source::host::{MemFileMapper, ThreadStagedSource};
use soso_llm_core::source::MmapTensorSource;
use sosomodel::index::{make_f32_entry, make_mxfp4_entry, pack_shard, TensorIndex};
use sosomodel::manifest::{AttnKind, FfnKind, LayerSpec, Manifest};
use std::thread;

fn add_tensor(
    tensors: &mut MemoryTensorSource,
    index: &mut TensorIndex,
    id: &mut u32,
    name: &str,
    shape: &[u32],
    fill: f32,
) {
    let elems: usize = shape.iter().map(|&d| d as usize).product();
    let mut w = vec![fill; elems];
    for (i, v) in w.iter_mut().enumerate() {
        *v = fill + (i as f32) * 1e-7;
    }
    tensors.tensors.insert(name.into(), w);
    index.entries.push(make_f32_entry(
        *id,
        name,
        &format!("{name}.tensor"),
        0,
        shape,
    ));
    *id += 1;
}

fn build_tiny_mla() -> (Manifest, TensorIndex, MemoryTensorSource) {
    let mut m = Manifest::tiny("mla-smoke");
    m.num_layers = 1;
    m.layers = vec![LayerSpec {
        attn_kind: AttnKind::Mla,
        q_lora_rank: 32,
        kv_lora_rank: 32,
        ..LayerSpec::default()
    }];
    let h = m.hidden_dim;
    let kv_heads = m.num_kv_heads;
    let head_dim = h / m.num_heads;
    let kv_dim = kv_heads * head_dim;
    let q_rank = 32u32;
    let kv_rank = 32u32;
    let mut index = TensorIndex::default();
    let mut id = 0u32;
    let mut tensors = MemoryTensorSource {
        tensors: Default::default(),
    };
    add_tensor(&mut tensors, &mut index, &mut id, "embed", &[m.vocab_size, h], 0.01);
    let p = "L00";
    add_tensor(&mut tensors, &mut index, &mut id, &format!("{p}.attn_norm"), &[h], 1.0);
    add_tensor(
        &mut tensors,
        &mut index,
        &mut id,
        &format!("{p}.attn_q_down"),
        &[q_rank, h],
        0.1,
    );
    add_tensor(
        &mut tensors,
        &mut index,
        &mut id,
        &format!("{p}.attn_q_up"),
        &[h, q_rank],
        0.1,
    );
    add_tensor(
        &mut tensors,
        &mut index,
        &mut id,
        &format!("{p}.attn_kv_down"),
        &[kv_rank, h],
        0.1,
    );
    add_tensor(
        &mut tensors,
        &mut index,
        &mut id,
        &format!("{p}.attn_k_up"),
        &[kv_dim, kv_rank],
        0.1,
    );
    add_tensor(
        &mut tensors,
        &mut index,
        &mut id,
        &format!("{p}.attn_v_up"),
        &[kv_dim, kv_rank],
        0.1,
    );
    add_tensor(
        &mut tensors,
        &mut index,
        &mut id,
        &format!("{p}.attn_output"),
        &[h, h],
        0.1,
    );
    add_tensor(&mut tensors, &mut index, &mut id, &format!("{p}.ffn_norm"), &[h], 1.0);
    add_tensor(
        &mut tensors,
        &mut index,
        &mut id,
        &format!("{p}.ffn_up"),
        &[m.ffn_dim, h],
        0.1,
    );
    add_tensor(
        &mut tensors,
        &mut index,
        &mut id,
        &format!("{p}.ffn_down"),
        &[h, m.ffn_dim],
        0.1,
    );
    (m, index, tensors)
}

fn build_tiny_latent_moe() -> (Manifest, TensorIndex, MemoryTensorSource) {
    let mut m = Manifest::tiny_moe("latent-smoke");
    m.num_layers = 1;
    let latent = m.moe_ffn_dim;
    let h = m.hidden_dim;
    let n_exp = m.num_experts;
    m.layers = vec![LayerSpec {
        ffn_kind: FfnKind::LatentMoe,
        num_experts: n_exp,
        num_experts_per_tok: m.num_experts_per_tok,
        moe_ffn_dim: latent,
        ..LayerSpec::default()
    }];
    let mut index = TensorIndex::default();
    let mut id = 0u32;
    let mut tensors = MemoryTensorSource {
        tensors: Default::default(),
    };
    add_tensor(&mut tensors, &mut index, &mut id, "embed", &[m.vocab_size, h], 0.01);
    let p = "L00";
    add_tensor(&mut tensors, &mut index, &mut id, &format!("{p}.attn_norm"), &[h], 1.0);
    add_tensor(
        &mut tensors,
        &mut index,
        &mut id,
        &format!("{p}.attn_q"),
        &[h, h],
        0.1,
    );
    add_tensor(
        &mut tensors,
        &mut index,
        &mut id,
        &format!("{p}.attn_k"),
        &[h, h],
        0.1,
    );
    add_tensor(
        &mut tensors,
        &mut index,
        &mut id,
        &format!("{p}.attn_v"),
        &[h, h],
        0.1,
    );
    add_tensor(
        &mut tensors,
        &mut index,
        &mut id,
        &format!("{p}.attn_output"),
        &[h, h],
        0.1,
    );
    add_tensor(&mut tensors, &mut index, &mut id, &format!("{p}.ffn_norm"), &[h], 1.0);
    add_tensor(
        &mut tensors,
        &mut index,
        &mut id,
        &format!("{p}.ffn_latent_in"),
        &[latent, h],
        0.02,
    );
    add_tensor(
        &mut tensors,
        &mut index,
        &mut id,
        &format!("{p}.ffn_latent_out"),
        &[h, latent],
        0.02,
    );
    add_tensor(
        &mut tensors,
        &mut index,
        &mut id,
        &format!("{p}.ffn_gate_inp"),
        &[n_exp, latent],
        0.01,
    );
    for e in 0..n_exp {
        let ep = format!("{p}.E{e:02}");
        add_tensor(
            &mut tensors,
            &mut index,
            &mut id,
            &format!("{ep}.ffn_gate"),
            &[latent, latent],
            0.03,
        );
        add_tensor(
            &mut tensors,
            &mut index,
            &mut id,
            &format!("{ep}.ffn_up"),
            &[latent, latent],
            0.03,
        );
        add_tensor(
            &mut tensors,
            &mut index,
            &mut id,
            &format!("{ep}.ffn_down"),
            &[latent, latent],
            0.04,
        );
    }
    (m, index, tensors)
}

#[test]
fn runtime_rechaza_flags_reservados() {
    let mut m = Manifest::tiny("flags");
    m.layers[0].flags = 1;
    assert!(m.supported_by_runtime().is_err());
}

#[test]
fn runtime_acepta_mla_con_ranks() {
    let mut m = Manifest::tiny("mla");
    m.layers = vec![LayerSpec {
        attn_kind: AttnKind::Mla,
        q_lora_rank: 32,
        kv_lora_rank: 32,
        ..LayerSpec::default()
    }];
    m.num_layers = 1;
    assert!(m.supported_by_runtime().is_ok());
}

#[test]
fn runtime_rechaza_mla_sin_ranks() {
    let mut m = Manifest::tiny("mla");
    m.layers[0].attn_kind = AttnKind::Mla;
    assert!(m.supported_by_runtime().is_err());
}

#[test]
fn mla_kv_bytes_usa_rank_latent() {
    let mut m = Manifest::tiny("mla-kv");
    m.num_layers = 1;
    m.layers = vec![LayerSpec {
        attn_kind: AttnKind::Mla,
        q_lora_rank: 64,
        kv_lora_rank: 32,
        ..LayerSpec::default()
    }];
    let bpt = kv_bytes_per_token(&m);
    assert_eq!(bpt, 32 * 2);
}

#[test]
fn mla_smoke_forward_un_token() {
    let (manifest, index, mut source) = build_tiny_mla();
    let mut rt = Runtime::new(manifest.clone(), index, 0, 0);
    rt.validate_shapes().unwrap();
    assert!(rt.kv[0].mla_latent);
    rt.embed_token(0, &mut source).unwrap();
    rt.forward_layers_range(0, manifest.num_layers, &mut source, None, &mut None)
        .unwrap();
    let logits = rt.logits(&mut source).unwrap();
    assert!(logits.iter().any(|&x| x.is_finite()));
}

#[test]
fn latent_moe_smoke_forward_un_token() {
    let (manifest, index, mut source) = build_tiny_latent_moe();
    let mut rt = Runtime::new(manifest.clone(), index, 0, 0);
    rt.validate_shapes().unwrap();
    rt.embed_token(0, &mut source).unwrap();
    rt.forward_layers_range(0, manifest.num_layers, &mut source, None, &mut None)
        .unwrap();
    let logits = rt.logits(&mut source).unwrap();
    assert!(logits.iter().any(|&x| x.is_finite()));
}

#[test]
fn classify_shared_como_tronco() {
    let mut index = TensorIndex::default();
    index.entries.push(make_f32_entry(
        0,
        "L00.S00.ffn_gate",
        "L00.S00.ffn_gate.tensor",
        0,
        &[16, 8],
    ));
    assert!(is_shared_expert_tensor("L00.S00.ffn_gate"));
    let c = classify_weight_bytes(&index);
    assert!(c.trunk_bytes > 0);
    assert_eq!(c.routed_expert_bytes, 0);
}

#[test]
fn mxfp4_roundtrip_aproximado() {
    let src: Vec<f32> = (0..64).map(|i| (i as f32 - 32.0) * 0.05).collect();
    let packed = quantize_mxfp4(&src);
    let mut out = vec![0.0f32; 64];
    dequant_mxfp4(&packed, &mut out).unwrap();
    for (a, b) in src.iter().zip(&out) {
        assert!((a - b).abs() < 0.2, "{a} vs {b}");
    }
}

#[test]
fn mxfp4_index_entry() {
    let e = make_mxfp4_entry(0, "w", "w.tensor", 0, &[64, 64]);
    assert_eq!(e.dtype, sosomodel::layout::DTYPE_MXFP4);
    assert!(e.byte_len > 0);
}

#[test]
fn thread_staged_kick_overlaps_load() {
    let raw: Vec<u8> = (0..256u32)
        .flat_map(|i| (i as f32 * 0.01).to_le_bytes())
        .collect();
    let packed = pack_shard(&raw);
    let mut mapper = MemFileMapper::new();
    mapper
        .files
        .insert("/m/shards/w.tensor".into(), packed);
    let index = TensorIndex {
        entries: vec![make_f32_entry(0, "w", "w.tensor", 0, &[64])],
    };
    let base = MmapTensorSource::new("/m/shards".into(), index, mapper);
    let mut staged = ThreadStagedSource::new(base);
    let shards = vec![String::from("w.tensor")];
    staged.kick_prefetch_shards(&shards);
    thread::sleep(std::time::Duration::from_millis(1));
    staged.wait_prefetch();
    let mut row = [0.0f32; 64];
    staged.load_f32("w", &mut row).unwrap();
    assert!(row.iter().any(|&x| x.is_finite()));
}

#[test]
fn runtime_rechaza_latent_moe_sin_tensores() {
    let mut m = Manifest::tiny_moe("latent");
    m.layers = vec![LayerSpec {
        ffn_kind: FfnKind::LatentMoe,
        num_experts: 4,
        num_experts_per_tok: 2,
        moe_ffn_dim: 32,
        ..LayerSpec::default()
    }];
    m.num_layers = 1;
    assert!(m.supported_by_runtime().is_ok());
    let rt = Runtime::new(m, TensorIndex::default(), 0, 0);
    assert!(rt.validate_shapes().is_err());
}
