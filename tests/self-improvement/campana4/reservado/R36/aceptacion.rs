//! Aceptación reservada de R36: `Manifest::parse` rechaza versiones desconocidas y
//! `num_layers` sin respaldo en los manifests antiguos.

use sosomodel::manifest::Manifest;
use sosomodel::{pack_som, parse_som, CACHE_ALIGN};

/// Cuerpo de un manifest v1 (sin tabla de capas) con `num_layers` capas.
fn cuerpo_v1(num_layers: u32) -> Vec<u8> {
    cuerpo(1, num_layers)
}

/// Cuerpo de un manifest v1, v2 o v3 (sin tabla de capas).
fn cuerpo(version: u32, num_layers: u32) -> Vec<u8> {
    let mut b = Vec::new();
    b.extend_from_slice(b"viejo\0");
    for v in [100u32, 64, num_layers, 4, 128, 256] {
        // vocab, hidden, layers, heads, ffn, max_seq
        b.extend_from_slice(&v.to_le_bytes());
    }
    if version >= 2 {
        b.extend_from_slice(&4u32.to_le_bytes()); // num_kv_heads
        b.extend_from_slice(&10000.0f32.to_le_bytes());
        b.extend_from_slice(&1e-5f32.to_le_bytes());
    }
    if version >= 3 {
        for v in [0u32, 0, 0] {
            b.extend_from_slice(&v.to_le_bytes()); // expertos
        }
    }
    b.extend_from_slice(&0u32.to_le_bytes()); // sin prefetch
    b
}

fn con_version(bytes: &[u8], version: u32) -> Vec<u8> {
    let (_, body) = parse_som(bytes).unwrap();
    pack_som(body, version, CACHE_ALIGN)
}

#[test]
fn un_v1_normal_se_sigue_leyendo() {
    let m = Manifest::parse(&pack_som(&cuerpo_v1(12), 1, CACHE_ALIGN)).unwrap();
    assert_eq!(m.name, "viejo");
    assert_eq!(m.num_layers, 12);
    assert_eq!(m.layers.len(), 12);
    assert_eq!(m.num_kv_heads, 4);
}

#[test]
fn en_v1_a_v3_el_numero_de_capas_tiene_techo() {
    for v in 1..=3u32 {
        assert!(Manifest::parse(&pack_som(&cuerpo(v, 1024), v, CACHE_ALIGN)).is_ok(), "v{v} con 1024 capas");
        assert!(Manifest::parse(&pack_som(&cuerpo(v, 1025), v, CACHE_ALIGN)).is_err(), "v{v} con 1025 capas");
        assert!(Manifest::parse(&pack_som(&cuerpo(v, 100_000), v, CACHE_ALIGN)).is_err(), "v{v} con 100000 capas");
    }
}

#[test]
fn las_versiones_desconocidas_se_rechazan() {
    let bytes = Manifest::tiny("t").serialize();
    assert!(Manifest::parse(&bytes).is_ok());
    for v in [0u32, 7, 8, 99, u32::MAX] {
        assert!(Manifest::parse(&con_version(&bytes, v)).is_err(), "versión {v}");
    }
    assert!(Manifest::parse(&pack_som(&cuerpo_v1(2), 0, CACHE_ALIGN)).is_err());
}

#[test]
fn las_versiones_4_a_6_siguen_valiendo_y_no_tienen_el_techo_antiguo() {
    let bytes = Manifest::tiny("t").serialize();
    for v in 4..=6u32 {
        let m = Manifest::parse(&con_version(&bytes, v)).unwrap_or_else(|_| panic!("v{v}"));
        assert_eq!(m.num_layers as usize, m.layers.len());
    }
    // Con tabla de capas explícita, más de 1024 capas es legítimo.
    let mut m = Manifest::tiny("grande");
    m.num_layers = 1500;
    m.prefetch.clear();
    m.fill_layers_from_globals();
    let p = Manifest::parse(&m.serialize()).unwrap();
    assert_eq!(p.layers.len(), 1500);
}

#[test]
fn la_ida_y_vuelta_de_los_modelos_de_prueba_no_cambia() {
    for m in [Manifest::tiny("a"), Manifest::tiny_moe("b"), Manifest::tiny_asr("c")] {
        let p = Manifest::parse(&m.serialize()).unwrap();
        assert_eq!(p.name, m.name);
        assert_eq!(p.num_layers, m.num_layers);
        assert_eq!(p.layers.len(), m.layers.len());
    }
}
