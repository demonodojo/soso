//! Tests arquitecturas extendidas: shared experts, MLA, LatentMoE, MXFP4, flags.
//!
//! Requiere `--features std`.

use soso_llm_core::arch::{forward_mla_attn, mla_head_dims};
use soso_llm_core::gemm::{rmsnorm, rope_inplace};
use soso_llm_core::kv::LayerKv;
use soso_llm_core::layer::{matvec_view, LayerScratch, TensorSource};
use soso_llm_core::parallel::Sequential;
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

fn add_tensor_identity_out(
    tensors: &mut MemoryTensorSource,
    index: &mut TensorIndex,
    id: &mut u32,
    name: &str,
    h: u32,
) {
    let mut w = vec![0.0f32; h as usize * h as usize];
    for i in 0..h as usize {
        w[i * h as usize + i] = 1.0;
    }
    tensors.tensors.insert(name.into(), w);
    index.entries.push(make_f32_entry(
        *id,
        name,
        &format!("{name}.tensor"),
        0,
        &[h, h],
    ));
    *id += 1;
}

fn build_tiny_mla_parity() -> (Manifest, TensorIndex, MemoryTensorSource) {
    let mut m = Manifest::tiny("mla-parity");
    m.hidden_dim = 64;
    m.num_heads = 2;
    m.num_kv_heads = 1;
    m.num_layers = 1;
    let h = m.hidden_dim;
    let heads = m.num_heads;
    let kv_heads = m.num_kv_heads;
    let head_dim = h / heads;
    let qk_nope = head_dim / 2;
    let qk_rope = head_dim - qk_nope;
    let q_rank = 16u32;
    let kv_rank = 16u32;
    let kv_qk = kv_heads * (qk_nope + qk_rope);
    let kv_v = kv_heads * head_dim;
    m.layers = vec![LayerSpec {
        attn_kind: AttnKind::Mla,
        q_lora_rank: q_rank,
        kv_lora_rank: kv_rank,
        qk_nope_head_dim: qk_nope as u32,
        qk_rope_head_dim: qk_rope as u32,
        v_head_dim: head_dim,
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
        &[kv_qk, kv_rank],
        0.1,
    );
    add_tensor(
        &mut tensors,
        &mut index,
        &mut id,
        &format!("{p}.attn_v_up"),
        &[kv_v, kv_rank],
        0.1,
    );
    add_tensor_identity_out(&mut tensors, &mut index, &mut id, &format!("{p}.attn_output"), h);
    (m, index, tensors)
}

fn mla_oracle_attn_out(
    spec: &LayerSpec,
    manifest: &Manifest,
    source: &mut MemoryTensorSource,
    hidden: &[f32],
    c_history: &[Vec<f32>],
    pos: usize,
) -> Vec<f32> {
    let h = manifest.hidden_dim as usize;
    let heads = manifest.num_heads as usize;
    let kv_heads = manifest.num_kv_heads as usize;
    let head_dim = h / heads;
    let dims = mla_head_dims(spec, head_dim, kv_heads);
    let q_rank = spec.q_lora_rank as usize;
    let kv_rank = spec.kv_lora_rank as usize;
    let theta = manifest.rope_theta;
    let group = heads / kv_heads;
    let n_tokens = pos + 1;

    let mut x = hidden.to_vec();
    let mut norm_w = vec![0.0; h];
    source.load_f32("L00.attn_norm", &mut norm_w).unwrap();
    rmsnorm(&mut x, &norm_w, manifest.rms_eps);

    let mut q_lat = vec![0.0; q_rank];
    let mut q = vec![0.0; h];
    matvec_view(
        &source.tensor_view("L00.attn_q_down").unwrap(),
        q_rank,
        h,
        &x,
        &mut q_lat,
    )
    .unwrap();
    matvec_view(
        &source.tensor_view("L00.attn_q_up").unwrap(),
        h,
        q_rank,
        &q_lat,
        &mut q,
    )
    .unwrap();
    for head in 0..heads {
        let base = head * head_dim;
        if dims.qk_rope > 0 {
            rope_inplace(
                &mut q[base + dims.qk_nope..base + dims.qk_nope + dims.qk_rope],
                pos,
                theta,
            );
        }
    }

    let inv = 1.0 / libm::sqrtf(dims.qk_per_head as f32);
    let mut attn_out = vec![0.0; h];
    let mut k_full = vec![0.0; dims.kv_qk_dim];
    let mut v_full = vec![0.0; dims.kv_v_dim];
    let mut logits = vec![0.0f32; n_tokens];

    for head in 0..heads {
        let q_h = &q[head * head_dim..(head + 1) * head_dim];
        let kv_head = head / group;
        let k_off = kv_head * dims.qk_per_head;
        let v_off = kv_head * dims.v_dim;
        logits.fill(0.0);

        for t in 0..n_tokens {
            matvec_view(
                &source.tensor_view("L00.attn_k_up").unwrap(),
                dims.kv_qk_dim,
                kv_rank,
                &c_history[t],
                &mut k_full,
            )
            .unwrap();
            if dims.qk_rope > 0 {
                rope_inplace(
                    &mut k_full[k_off + dims.qk_nope..k_off + dims.qk_nope + dims.qk_rope],
                    t,
                    theta,
                );
            }
            let q_len = dims.qk_per_head.min(q_h.len());
            let mut dot = 0.0f32;
            for d in 0..q_len {
                dot += q_h[d] * k_full[k_off + d];
            }
            logits[t] = dot * inv;
        }

        let mut max_l = logits[0];
        for t in 1..n_tokens {
            if logits[t] > max_l {
                max_l = logits[t];
            }
        }
        let mut sum = 0.0f32;
        for t in 0..n_tokens {
            logits[t] = libm::expf(logits[t] - max_l);
            sum += logits[t];
        }
        if sum > 0.0 {
            for t in 0..n_tokens {
                logits[t] /= sum;
            }
        }

        let mut head_out = vec![0.0; head_dim];
        for t in 0..n_tokens {
            matvec_view(
                &source.tensor_view("L00.attn_v_up").unwrap(),
                dims.kv_v_dim,
                kv_rank,
                &c_history[t],
                &mut v_full,
            )
            .unwrap();
            for d in 0..head_dim {
                if d < dims.v_dim {
                    head_out[d] += logits[t] * v_full[v_off + d];
                }
            }
        }
        attn_out[head * head_dim..(head + 1) * head_dim].copy_from_slice(&head_out);
    }
    attn_out
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
fn mla_latent_rope_golden_parity() {
    let (manifest, _index, mut source) = build_tiny_mla_parity();
    let spec = manifest.layers[0].clone();
    let h = manifest.hidden_dim as usize;
    let kv_rank = spec.kv_lora_rank as usize;
    let mut hidden = vec![0.0; h];
    for (i, v) in hidden.iter_mut().enumerate() {
        *v = 0.01 * (i as f32 + 1.0);
    }
    let mut kv = LayerKv::with_capacity_mla(manifest.max_seq as usize, kv_rank);
    let mut scratch = LayerScratch::new(&manifest);
    const T: usize = 3;

    for pos in 0..T {
        for i in 0..h {
            hidden[i] += pos as f32 * 0.001;
        }
        let before = hidden.clone();
        forward_mla_attn(
            &spec,
            &manifest,
            0,
            pos,
            "L00",
            &mut hidden,
            &mut scratch,
            &mut kv,
            &mut source,
            &mut None,
            false,
            &Sequential,
            None,
            None,
        )
        .unwrap();
        let attn_impl: Vec<f32> = before
            .iter()
            .zip(hidden.iter())
            .map(|(b, a)| a - b)
            .collect();

        if pos > 0 {
            let mut c_history = Vec::new();
            for t in 0..=pos {
                let mut c = vec![0.0; kv_rank];
                kv.load_latent_token(t, kv_rank, &mut c);
                c_history.push(c);
            }
            let oracle =
                mla_oracle_attn_out(&spec, &manifest, &mut source, &before, &c_history, pos);
            for i in 0..h {
                let diff = (attn_impl[i] - oracle[i]).abs();
                assert!(
                    diff < 1e-4,
                    "pos={pos} dim={i}: impl={} oracle={} diff={diff}",
                    attn_impl[i],
                    oracle[i]
                );
            }
        }
    }
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

fn add_dense_ffn(
    tensors: &mut MemoryTensorSource,
    index: &mut TensorIndex,
    id: &mut u32,
    p: &str,
    h: u32,
    ffn: u32,
) {
    add_tensor(tensors, index, id, &format!("{p}.ffn_norm"), &[h], 1.0);
    add_tensor(tensors, index, id, &format!("{p}.ffn_up"), &[ffn, h], 0.05);
    add_tensor(tensors, index, id, &format!("{p}.ffn_gate"), &[ffn, h], 0.05);
    add_tensor(tensors, index, id, &format!("{p}.ffn_down"), &[h, ffn], 0.05);
}

fn build_tiny_qwen38() -> (Manifest, TensorIndex, MemoryTensorSource) {
    let h = 16u32;
    let ffn = 32u32;
    let gated_heads = 2u32;
    let gated_kv = 1u32;
    let gated_hd = 8u32;
    let n_k = 2u32;
    let n_v = 4u32;
    let d = 4u32;
    let kernel = 2u32;
    let qkv = n_k * d * 2 + n_v * d;
    let mut m = Manifest::tiny("qwen38-smoke");
    m.hidden_dim = h;
    m.num_heads = gated_heads;
    m.num_kv_heads = gated_kv;
    m.ffn_dim = ffn;
    m.num_layers = 2;
    m.max_seq = 8;
    m.layers = vec![
        LayerSpec {
            attn_kind: AttnKind::Gdn,
            num_heads: n_k,
            num_kv_heads: n_v,
            v_head_dim: d,
            qk_rope_head_dim: kernel,
            ..LayerSpec::default()
        },
        LayerSpec {
            attn_kind: AttnKind::Gated,
            num_heads: gated_heads,
            num_kv_heads: gated_kv,
            v_head_dim: gated_hd,
            qk_rope_head_dim: 2,
            ..LayerSpec::default()
        },
    ];
    let mut index = TensorIndex::default();
    let mut id = 0u32;
    let mut tensors = MemoryTensorSource {
        tensors: Default::default(),
    };
    add_tensor(&mut tensors, &mut index, &mut id, "embed", &[m.vocab_size, h], 0.01);
    add_tensor(&mut tensors, &mut index, &mut id, "output_norm", &[h], 1.0);
    let p0 = "L00";
    add_tensor(&mut tensors, &mut index, &mut id, &format!("{p0}.attn_norm"), &[h], 1.0);
    add_tensor(&mut tensors, &mut index, &mut id, &format!("{p0}.attn_qkv"), &[qkv, h], 0.02);
    add_tensor(&mut tensors, &mut index, &mut id, &format!("{p0}.attn_gate"), &[n_v * d, h], 0.02);
    add_tensor(&mut tensors, &mut index, &mut id, &format!("{p0}.ssm_conv1d"), &[qkv, kernel], 0.1);
    add_tensor(&mut tensors, &mut index, &mut id, &format!("{p0}.ssm_dt"), &[n_v], 0.1);
    add_tensor(&mut tensors, &mut index, &mut id, &format!("{p0}.ssm_a"), &[n_v], -0.2);
    add_tensor(&mut tensors, &mut index, &mut id, &format!("{p0}.ssm_beta"), &[n_v, h], 0.02);
    add_tensor(&mut tensors, &mut index, &mut id, &format!("{p0}.ssm_alpha"), &[n_v, h], 0.02);
    add_tensor(&mut tensors, &mut index, &mut id, &format!("{p0}.ssm_norm"), &[d], 1.0);
    add_tensor(&mut tensors, &mut index, &mut id, &format!("{p0}.ssm_out"), &[h, n_v * d], 0.05);
    add_dense_ffn(&mut tensors, &mut index, &mut id, p0, h, ffn);
    let p1 = "L01";
    add_tensor(&mut tensors, &mut index, &mut id, &format!("{p1}.attn_norm"), &[h], 1.0);
    add_tensor(
        &mut tensors,
        &mut index,
        &mut id,
        &format!("{p1}.attn_q"),
        &[gated_heads * gated_hd * 2, h],
        0.03,
    );
    add_tensor(
        &mut tensors,
        &mut index,
        &mut id,
        &format!("{p1}.attn_k"),
        &[gated_kv * gated_hd, h],
        0.03,
    );
    add_tensor(
        &mut tensors,
        &mut index,
        &mut id,
        &format!("{p1}.attn_v"),
        &[gated_kv * gated_hd, h],
        0.03,
    );
    add_tensor(
        &mut tensors,
        &mut index,
        &mut id,
        &format!("{p1}.attn_output"),
        &[h, gated_heads * gated_hd],
        0.03,
    );
    add_tensor(&mut tensors, &mut index, &mut id, &format!("{p1}.attn_q_norm"), &[gated_hd], 1.0);
    add_tensor(&mut tensors, &mut index, &mut id, &format!("{p1}.attn_k_norm"), &[gated_hd], 1.0);
    add_dense_ffn(&mut tensors, &mut index, &mut id, p1, h, ffn);
    (m, index, tensors)
}

#[test]
fn qwen38_smoke_gdn_y_gated() {
    let (manifest, index, mut source) = build_tiny_qwen38();
    assert_eq!(manifest.attn_kind(0), AttnKind::Gdn);
    assert_eq!(manifest.attn_kind(1), AttnKind::Gated);
    assert!(manifest.supported_by_runtime().is_ok());
    let mut rt = Runtime::new(manifest.clone(), index, 0, 0);
    rt.validate_shapes().unwrap();
    assert!(!rt.kv[0].gdn_s.is_empty());
    rt.embed_token(1, &mut source).unwrap();
    rt.forward_layers_range(0, manifest.num_layers, &mut source, None, &mut None)
        .unwrap();
    let logits = rt.logits(&mut source).unwrap();
    assert!(logits.iter().all(|x| x.is_finite()));
    rt.advance_pos();
    rt.embed_token(2, &mut source).unwrap();
    rt.forward_layers_range(0, manifest.num_layers, &mut source, None, &mut None)
        .unwrap();
    let logits2 = rt.logits(&mut source).unwrap();
    assert!(logits2.iter().all(|x| x.is_finite()));
}

#[test]
fn gdn_kv_bytes_no_crece_con_secuencia() {
    let (m, _, _) = build_tiny_qwen38();
    let bpt = kv_bytes_per_token(&m);
    // Solo la capa gated (1 kv head × 8 dim × K y V × f16).
    assert_eq!(bpt, 8 * 2 * 2);
}
