//! Aceptación reservada de R22.

use std::collections::BTreeMap;

use sosomodel::index::{make_f32_entry, make_mxfp4_entry, make_q4_k_entry, make_q8_0_entry, pack_shard};
use sosomodel::validate::ModelBundle;
use sosomodel::{Manifest, TensorEntry, TensorIndex, DTYPE_F32};

fn manifiesto() -> Manifest {
    Manifest::tiny("r22")
}

/// Devuelve el shard empaquetado con `len` bytes de payload.
fn shard(len: usize) -> Vec<u8> {
    pack_shard(&vec![7u8; len])
}

fn validar(entries: Vec<TensorEntry>, shard_len: usize) -> Result<(), ()> {
    let s = shard(shard_len);
    let mut shards = BTreeMap::new();
    shards.insert("s0", s.as_slice());
    let b = ModelBundle { manifest: manifiesto(), index: TensorIndex { entries }, shards };
    b.validate()
}

#[test]
fn entradas_coherentes_de_todos_los_dtype_validan() {
    let e = vec![
        make_f32_entry(0, "a", "s0", 0, &[4, 3]),
        make_q8_0_entry(1, "b", "s0", 64, &[2, 32]),
        make_q4_k_entry(2, "c", "s0", 256, &[512]),
        make_mxfp4_entry(3, "d", "s0", 1024, &[64]),
    ];
    assert_eq!(validar(e, 4096), Ok(()));
}

#[test]
fn byte_len_que_no_cuadra_con_la_forma_se_rechaza() {
    for delta in [-4i64, -1, 1, 4, 36] {
        let mut e = make_f32_entry(0, "a", "s0", 0, &[4, 3]);
        e.byte_len = (e.byte_len as i64 + delta) as u64;
        assert_eq!(validar(vec![e], 4096), Err(()), "f32 delta {delta}");
    }
    let mut q8 = make_q8_0_entry(0, "b", "s0", 0, &[2, 32]);
    q8.byte_len += 36;
    assert_eq!(validar(vec![q8], 4096), Err(()));
    let mut q4 = make_q4_k_entry(0, "c", "s0", 0, &[256]);
    q4.byte_len -= 1;
    assert_eq!(validar(vec![q4], 4096), Err(()));
    let mut mx = make_mxfp4_entry(0, "d", "s0", 0, &[32]);
    mx.byte_len = 0;
    assert_eq!(validar(vec![mx], 4096), Err(()));
}

#[test]
fn la_forma_manda_aunque_el_byte_len_cabria_en_el_shard() {
    // Forma de 100 elementos f32 = 400 bytes, pero el índice dice 40.
    let mut e = make_f32_entry(0, "a", "s0", 0, &[10, 10]);
    e.byte_len = 40;
    assert_eq!(validar(vec![e], 4096), Err(()));
}

#[test]
fn dtype_desconocido_se_rechaza() {
    let mut e = make_f32_entry(0, "a", "s0", 0, &[4]);
    e.dtype = 99;
    assert_eq!(validar(vec![e], 4096), Err(()));
    let mut e = make_f32_entry(0, "a", "s0", 0, &[4]);
    e.dtype = 0;
    assert_eq!(validar(vec![e], 4096), Err(()));
    let _ = DTYPE_F32;
}

#[test]
fn forma_que_desborda_se_rechaza_sin_panic() {
    let mut e = make_f32_entry(0, "a", "s0", 0, &[4]);
    e.shape = vec![u32::MAX, u32::MAX, u32::MAX];
    assert_eq!(validar(vec![e], 4096), Err(()));
}

#[test]
fn nombres_duplicados_se_rechazan() {
    let e = vec![
        make_f32_entry(0, "w", "s0", 0, &[4]),
        make_f32_entry(1, "w", "s0", 64, &[4]),
    ];
    assert_eq!(validar(e, 4096), Err(()));
    let e = vec![
        make_f32_entry(0, "w", "s0", 0, &[4]),
        make_f32_entry(1, "w2", "s0", 64, &[4]),
    ];
    assert_eq!(validar(e, 4096), Ok(()));
}

#[test]
fn se_mantienen_las_comprobaciones_previas() {
    // Fuera del payload.
    let e = vec![make_f32_entry(0, "a", "s0", 4090, &[4])];
    assert_eq!(validar(e, 4096), Err(()));
    // Shard inexistente.
    let e = vec![make_f32_entry(0, "a", "otro", 0, &[4])];
    assert_eq!(validar(e, 4096), Err(()));
    // Prefetch que no coincide con num_layers.
    let s = shard(64);
    let mut shards = BTreeMap::new();
    shards.insert("s0", s.as_slice());
    let mut m = manifiesto();
    m.prefetch.pop();
    let b = ModelBundle { manifest: m, index: TensorIndex { entries: vec![] }, shards };
    assert_eq!(b.validate(), Err(()));
}
