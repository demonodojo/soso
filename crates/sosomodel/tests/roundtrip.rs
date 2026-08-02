use sosomodel::index::{pack_shard, TensorIndex, make_f32_entry};
use sosomodel::manifest::{AttnKind, FfnKind, LayerSpec, Manifest};

#[test]
fn manifest_v4_roundtrip_layer_specs() {
    let mut m = Manifest::tiny("v4");
    m.num_layers = 2;
    m.prefetch.truncate(2);
    m.layers = vec![
        LayerSpec {
            attn_kind: AttnKind::Kda,
            ffn_kind: FfnKind::LatentMoe,
            num_experts: 896,
            num_experts_per_tok: 16,
            num_shared_experts: 2,
            moe_ffn_dim: 3072,
            ..LayerSpec::default()
        },
        LayerSpec {
            attn_kind: AttnKind::Mla,
            ffn_kind: FfnKind::LatentMoe,
            kv_lora_rank: 512,
            q_lora_rank: 1536,
            qk_rope_head_dim: 64,
            qk_nope_head_dim: 128,
            v_head_dim: 128,
            num_experts: 896,
            num_experts_per_tok: 16,
            ..LayerSpec::default()
        },
    ];
    let bytes = m.serialize();
    let parsed = Manifest::parse(&bytes).unwrap();
    assert_eq!(parsed.layers.len(), 2);
    assert_eq!(parsed.layers[0].attn_kind, AttnKind::Kda);
    assert_eq!(parsed.layers[1].attn_kind, AttnKind::Mla);
    assert_eq!(parsed.layers[0].num_shared_experts, 2);
    assert_eq!(parsed.layers[1].kv_lora_rank, 512);
}

#[test]
fn manifest_v3_synthesizes_layers() {
    let mut m = Manifest::tiny("v3");
    m.layers.clear();
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
    body.extend_from_slice(&m.num_experts.to_le_bytes());
    body.extend_from_slice(&m.num_experts_per_tok.to_le_bytes());
    body.extend_from_slice(&m.moe_ffn_dim.to_le_bytes());
    body.extend_from_slice(&(m.prefetch.len() as u32).to_le_bytes());
    for pf in &m.prefetch {
        body.extend_from_slice(&pf.layer.to_le_bytes());
        body.extend_from_slice(&(pf.shards.len() as u32).to_le_bytes());
        for s in &pf.shards {
            body.extend_from_slice(s.as_bytes());
            body.push(0);
        }
    }
    let v3_bytes = sosomodel::pack_som(&body, 3, sosomodel::CACHE_ALIGN);
    let parsed = Manifest::parse(&v3_bytes).unwrap();
    assert_eq!(parsed.layers.len(), 4);
    assert_eq!(parsed.layers[0].attn_kind, AttnKind::Gqa);
    assert_eq!(parsed.layers[0].ffn_kind, FfnKind::Dense);
}

#[test]
fn manifest_max_ffn_dim_moe() {
    let m = Manifest::tiny_moe("moe");
    assert_eq!(m.max_ffn_dim(), m.moe_ffn_dim);
}

#[test]
fn manifest_roundtrip() {
    let m = Manifest::tiny("test");
    let bytes = m.serialize();
    let parsed = Manifest::parse(&bytes).unwrap();
    assert_eq!(parsed.name, "test");
    assert_eq!(parsed.num_layers, 4);
}

#[test]
fn shard_crc_ok() {
    let data = [1u8, 2, 3, 4];
    let packed = pack_shard(&data);
    let payload = sosomodel::index::verify_shard(&packed).unwrap();
    assert_eq!(payload, &data);
}

#[test]
fn shard_crc_detecta_corrupcion() {
    let data = [1u8, 2, 3, 4];
    let mut packed = pack_shard(&data);
    packed[0] ^= 0xff;
    assert!(sosomodel::index::verify_shard(&packed).is_err());
}

#[test]
fn index_roundtrip() {
    let mut idx = TensorIndex::default();
    idx.entries.push(make_f32_entry(0, "w", "s.tensor", 0, &[4, 4]));
    let bytes = idx.serialize();
    let parsed = TensorIndex::parse(&bytes).unwrap();
    assert_eq!(parsed.entries.len(), 1);
}

#[test]
fn manifest_v2_roundtrip_campos_gqa_rope() {
    let mut m = Manifest::tiny("test");
    m.num_kv_heads = 2;
    m.rope_theta = 500000.0;
    m.rms_eps = 1e-6;
    let parsed = Manifest::parse(&m.serialize()).unwrap();
    assert_eq!(parsed.num_kv_heads, 2);
    assert!((parsed.rope_theta - 500000.0).abs() < 1.0);
    assert!((parsed.rms_eps - 1e-6).abs() < 1e-9);
}

#[test]
fn shard_crc_terminado_en_cero_no_se_recorta() {
    // Regresión: el verify antiguo quitaba todos los ceros finales y podía
    // comerse bytes del propio CRC (~1/256 de los shards). Con la cabecera
    // explícita cualquier payload debe verificar, sea cual sea su CRC.
    for seed in 0u32..600 {
        let data: Vec<u8> = (0..37).map(|i| (seed.wrapping_mul(31).wrapping_add(i) % 251) as u8).collect();
        let packed = pack_shard(&data);
        let payload = sosomodel::index::verify_shard(&packed)
            .unwrap_or_else(|_| panic!("shard con seed {seed} rechazado"));
        assert_eq!(payload, &data[..]);
    }
}

#[test]
fn shard_v2_payload_alineado_a_64() {
    let data: Vec<u8> = (0..100u8).collect();
    let packed = pack_shard(&data);
    let payload = sosomodel::index::verify_shard(&packed).unwrap();
    let off = payload.as_ptr() as usize - packed.as_ptr() as usize;
    assert_eq!(off, sosomodel::index::SHARD_PAYLOAD_OFF);
    assert_eq!(off % 64, 0);
    assert_eq!(payload, &data[..]);
    assert_eq!(packed.len() % 4096, 0);
}

#[test]
fn shard_payload_con_ceros_finales_se_conserva() {
    let data = [1u8, 2, 3, 0, 0, 0];
    let packed = pack_shard(&data);
    let payload = sosomodel::index::verify_shard(&packed).unwrap();
    assert_eq!(payload, &data);
}

#[test]
fn parse_truncado_no_hace_panic() {
    let m = Manifest::tiny("t").serialize();
    let idx = {
        let mut i = TensorIndex::default();
        i.entries.push(make_f32_entry(0, "w", "s.tensor", 0, &[4]));
        i.serialize()
    };
    for data in [&m, &idx] {
        for cut in 0..data.len() {
            // ninguna longitud truncada debe hacer panic
            let _ = Manifest::parse(&data[..cut]);
            let _ = TensorIndex::parse(&data[..cut]);
        }
    }
    // payload_len corrupto (enorme) tampoco
    let mut evil = m.clone();
    evil[16..24].copy_from_slice(&u64::MAX.to_le_bytes());
    assert!(Manifest::parse(&evil).is_err());
}
