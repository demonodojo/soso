//! Tests arquitecturas extendidas: shared experts, MLA, LatentMoE, MXFP4, flags.

use soso_llm_core::layer::TensorSource;
use soso_llm_core::plan::{classify_weight_bytes, is_shared_expert_tensor};
use soso_llm_core::quant::{dequant_mxfp4, quantize_mxfp4};
use soso_llm_core::runtime::Runtime;
use soso_llm_core::source::host::{MemFileMapper, ThreadStagedSource};
use soso_llm_core::source::MmapTensorSource;
use sosomodel::index::{make_f32_entry, make_mxfp4_entry, pack_shard, TensorIndex};
use sosomodel::manifest::{AttnKind, FfnKind, LayerSpec, Manifest};
use std::thread;

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
