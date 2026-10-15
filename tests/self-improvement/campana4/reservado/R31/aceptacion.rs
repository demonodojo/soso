//! Aceptación reservada de R31.

use sosomfs::{Catalog, Extent, ModelEntry, ShardEntry};

fn extent(i: u64) -> Extent {
    Extent {
        volume_id: 0,
        start_lba: 100 + i,
        block_count: 8,
        segment_crc32c: 0xdead_0000 + i as u32,
        stripe_width: 1,
        stripe_index: 0,
    }
}

fn shard(path: &str, n_ext: u64) -> ShardEntry {
    ShardEntry {
        rel_path: path.to_string(),
        byte_len: 4096 * 8 * n_ext,
        extents: (0..n_ext).map(extent).collect(),
        prefetch_next_lba: 7,
        prefetch_bytes: 4096,
        cache_policy: 1,
        align_requirement: 2,
        flags: 3,
        shard_crc32c: 0xabcd_1234,
    }
}

fn modelo(name: &str, shards: Vec<ShardEntry>) -> ModelEntry {
    ModelEntry {
        name: name.to_string(),
        manifest_lba: 10,
        manifest_blocks: 1,
        index_lba: 11,
        index_blocks: 2,
        shards,
    }
}

fn catalogo() -> Catalog {
    Catalog {
        models: vec![
            modelo("uno", vec![shard("a.bin", 2), shard("b/c.bin", 0)]),
            modelo("dos", vec![shard("d.bin", 3)]),
            modelo("vacio", vec![]),
        ],
    }
}

#[test]
fn ida_y_vuelta_sigue_funcionando() {
    let bytes = catalogo().serialize();
    let c = Catalog::parse(&bytes).expect("catálogo válido");
    assert_eq!(c.models, catalogo().models);
    // El relleno de ceros que deja la lectura por bloques no molesta.
    let mut con_relleno = bytes.clone();
    con_relleno.resize(bytes.len() + 4096, 0);
    assert_eq!(Catalog::parse(&con_relleno).unwrap().models, catalogo().models);
    assert!(Catalog::parse(&Catalog { models: vec![] }.serialize()).unwrap().models.is_empty());
}

#[test]
fn cualquier_truncado_es_error_y_no_panico() {
    let bytes = catalogo().serialize();
    for n in 0..bytes.len() {
        assert!(Catalog::parse(&bytes[..n]).is_err(), "el prefijo de {n} bytes debía dar Err");
    }
}

#[test]
fn cabecera_de_modelo_justa_no_basta() {
    // Un único modelo anunciado y sólo 96 bytes tras el contador: faltan el recuento de shards y más.
    for extra in [0usize, 1, 64, 96, 100] {
        let mut b = vec![0u8; 4 + extra];
        b[0] = 1;
        assert!(Catalog::parse(&b).is_err(), "extra={extra}");
    }
}

#[test]
fn contadores_absurdos_son_error() {
    // Número de modelos desproporcionado respecto al tamaño del buffer.
    let mut b = catalogo().serialize();
    b[0..4].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(Catalog::parse(&b).is_err());

    // Número de shards desproporcionado en el primer modelo (offset 4 + 65 + 32).
    let mut b = catalogo().serialize();
    let off = 4 + 64 + 1 + 32;
    b[off..off + 4].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(Catalog::parse(&b).is_err());

    // Un shard cuyo contador de extents promete más de los que hay hasta el final.
    let c = Catalog { models: vec![modelo("m", vec![shard("x", 1)])] };
    let mut b = c.serialize();
    let off = 4 + 64 + 1 + 32 + 4 + 96 + 1 + 8;
    b[off..off + 2].copy_from_slice(&u16::MAX.to_le_bytes());
    assert!(Catalog::parse(&b).is_err());
}

#[test]
fn corrupcion_aleatoria_nunca_entra_en_panico() {
    let base = catalogo().serialize();
    let mut x: u32 = 0x1234_5678;
    for _ in 0..3000 {
        let mut b = base.clone();
        for _ in 0..3 {
            x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let i = (x >> 8) as usize % b.len();
            x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            b[i] = (x >> 16) as u8;
        }
        let cut = (x >> 4) as usize % (b.len() + 1);
        let _ = Catalog::parse(&b);
        let _ = Catalog::parse(&b[..cut]);
    }
}
