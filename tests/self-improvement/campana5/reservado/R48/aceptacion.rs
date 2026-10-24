//! Aceptación reservada de R48: `CachePolicy` y consultas sobre `ShardEntry` / `ModelEntry`.

use sosomfs::{
    CachePolicy, Catalog, Extent, ModelEntry, ParseCachePolicyError, ShardEntry, CACHE_NORMAL,
    CACHE_PIN, CACHE_STREAM,
};

fn ext(n: u64) -> Extent {
    Extent { volume_id: 0, start_lba: 100, block_count: n, segment_crc32c: 0, stripe_width: 1, stripe_index: 0 }
}

fn shard(path: &str, len: u64, policy: u8, bloques: &[u64]) -> ShardEntry {
    ShardEntry {
        rel_path: path.to_string(),
        byte_len: len,
        extents: bloques.iter().map(|&n| ext(n)).collect(),
        prefetch_next_lba: 0,
        prefetch_bytes: 0,
        cache_policy: policy,
        align_requirement: 0,
        flags: 0,
        shard_crc32c: 0,
    }
}

fn modelo() -> ModelEntry {
    ModelEntry {
        name: "m".to_string(),
        manifest_lba: 1,
        manifest_blocks: 1,
        index_lba: 2,
        index_blocks: 1,
        shards: vec![
            shard("config.json", 10, CACHE_PIN, &[1]),
            shard("weights/a.bin", 1000, CACHE_STREAM, &[3, 4]),
            shard("weights/b.bin", 2000, CACHE_NORMAL, &[]),
            shard("weights-extra/c.bin", 5, 9, &[2]),
            shard("weights/sub/d.bin", 7, CACHE_STREAM, &[1, 1, 1]),
            shard("weights", 3, CACHE_NORMAL, &[1]),
            shard("config.json", 99, CACHE_NORMAL, &[8]),
        ],
    }
}

#[test]
fn try_from_u8_y_vuelta() {
    assert_eq!(CachePolicy::try_from(CACHE_NORMAL), Ok(CachePolicy::Normal));
    assert_eq!(CachePolicy::try_from(CACHE_STREAM), Ok(CachePolicy::Stream));
    assert_eq!(CachePolicy::try_from(CACHE_PIN), Ok(CachePolicy::Pin));
    assert_eq!(CachePolicy::try_from(3u8), Err(3u8));
    assert_eq!(CachePolicy::try_from(255u8), Err(255u8));
    assert_eq!(u8::from(CachePolicy::Normal), CACHE_NORMAL);
    assert_eq!(u8::from(CachePolicy::Stream), CACHE_STREAM);
    assert_eq!(u8::from(CachePolicy::Pin), CACHE_PIN);
}

#[test]
fn display_y_from_str() {
    assert_eq!(CachePolicy::Normal.to_string(), "normal");
    assert_eq!(CachePolicy::Stream.to_string(), "stream");
    assert_eq!(CachePolicy::Pin.to_string(), "pin");
    for p in [CachePolicy::Normal, CachePolicy::Stream, CachePolicy::Pin] {
        assert_eq!(p.to_string().parse::<CachePolicy>(), Ok(p));
    }
    for malo in ["", "Pin", "PIN", " pin", "pin ", "streaming", "0"] {
        assert_eq!(malo.parse::<CachePolicy>(), Err(ParseCachePolicyError), "{malo:?}");
    }
}

#[test]
fn policy_del_shard() {
    let m = modelo();
    assert_eq!(m.shards[0].policy(), Some(CachePolicy::Pin));
    assert_eq!(m.shards[1].policy(), Some(CachePolicy::Stream));
    assert_eq!(m.shards[2].policy(), Some(CachePolicy::Normal));
    assert_eq!(m.shards[3].policy(), None);
}

#[test]
fn total_blocks_del_shard() {
    let m = modelo();
    assert_eq!(m.shards[1].total_blocks(), 7);
    assert_eq!(m.shards[2].total_blocks(), 0);
    assert_eq!(m.shards[4].total_blocks(), 3);
    let enorme = shard("x", 0, 0, &[u64::MAX, 5]);
    assert_eq!(enorme.total_blocks(), u64::MAX);
}

#[test]
fn total_bytes_del_modelo() {
    let m = modelo();
    assert_eq!(m.total_bytes(), 10 + 1000 + 2000 + 5 + 7 + 3 + 99);
    let vacio = ModelEntry { shards: vec![], ..modelo() };
    assert_eq!(vacio.total_bytes(), 0);
    let enorme = ModelEntry { shards: vec![shard("a", u64::MAX, 0, &[]), shard("b", 1, 0, &[])], ..modelo() };
    assert_eq!(enorme.total_bytes(), u64::MAX);
}

#[test]
fn shard_por_ruta_exacta() {
    let m = modelo();
    assert_eq!(m.shard("weights/a.bin").unwrap().byte_len, 1000);
    // Repetido: el primero.
    assert_eq!(m.shard("config.json").unwrap().byte_len, 10);
    // Exacta: ni prefijos ni mayúsculas ni barra final.
    assert!(m.shard("weights/a").is_none());
    assert!(m.shard("Weights/a.bin").is_none());
    assert!(m.shard("weights/").is_none());
    assert!(m.shard("").is_none());
    assert_eq!(m.shard("weights").unwrap().byte_len, 3);
}

#[test]
fn shards_bajo_un_directorio() {
    let m = modelo();
    let rutas = |d: &str| -> Vec<String> { m.shards_under(d).map(|s| s.rel_path.clone()).collect() };
    assert_eq!(rutas("weights"), ["weights/a.bin", "weights/b.bin", "weights/sub/d.bin"]);
    assert_eq!(rutas("weights/sub"), ["weights/sub/d.bin"]);
    assert_eq!(rutas("weights-extra"), ["weights-extra/c.bin"]);
    // `weights-extra/...` no está bajo `weights`, y el fichero `weights` tampoco está bajo sí mismo.
    assert_eq!(rutas("config.json"), Vec::<String>::new());
    assert_eq!(rutas("weight"), Vec::<String>::new());
    assert_eq!(rutas("nada"), Vec::<String>::new());
    // `""` devuelve todos, en el orden del modelo.
    let todos = rutas("");
    assert_eq!(todos.len(), 7);
    assert_eq!(todos[0], "config.json");
    assert_eq!(todos[6], "config.json");
    assert_eq!(todos[5], "weights");
}

#[test]
fn la_api_anterior_sigue_igual() {
    let m = modelo();
    let cat = Catalog { models: vec![m.clone()] };
    assert!(!cat.serialize().is_empty());
    assert_eq!(cat.find_model("m").unwrap().shards.len(), 7);
    assert!(cat.find_model("otro").is_none());
    assert_eq!(cat.find_shard("m", "weights/b.bin").unwrap().byte_len, 2000);
    assert!(cat.find_shard("m", "weights/zzz").is_none());
    // Los campos públicos siguen siendo los de siempre.
    assert_eq!(m.shards[3].cache_policy, 9);
    assert_eq!(CACHE_NORMAL, 0);
    assert_eq!(CACHE_STREAM, 1);
    assert_eq!(CACHE_PIN, 2);
}
