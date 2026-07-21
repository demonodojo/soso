use sosomodel::index::{pack_shard, TensorIndex, make_f32_entry};
use sosomodel::manifest::Manifest;

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
