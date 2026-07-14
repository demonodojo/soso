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
