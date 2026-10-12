//! Paridad numérica de Gemma 4 12B QAT Q4_0 contra llama.cpp (G3 de `docs/PLAN-GEMMA4.md`).
//!
//! Los valores de referencia salen de `llama-server` (`/tokenize` + `/completion` con `n_probs`,
//! `temperature: 0`) sobre los mismos ids. Necesita `target/gemma4-12b-model` (7 GB); sin él falla
//! con ruta clara, de ahí el `#[ignore]`:
//!
//! ```sh
//! HILOS=12 cargo test --release -p soso-llm-api --features std --test gemma_paridad -- --ignored
//! ```
//! Tolerancia: el mismo top-1 y el mismo top-2, y la probabilidad del top-1 a ±0,12 (llama.cpp
//! referenció en GPU y soso requantiza token_embd a Q8_0 y guarda el KV en f16).

#![cfg(feature = "std")]

use soso_llm_api::{HostPool, OwnedMapper};
use soso_llm_core::parallel::RowParallel;
use soso_llm_core::runtime::Runtime;
use soso_llm_core::source::MmapTensorSource;
use sosomodel::{Manifest, TensorIndex};

struct Caso {
    nombre: &'static str,
    ids: &'static [u32],
    top1: u32,
    p1: f64,
    top2: u32,
}

const CASOS: &[Caso] = &[
    Caso { nombre: "capital", ids: &[2, 818, 5279, 529, 7001, 563], top1: 236771, p1: 0.3176, top2: 236770 },
    Caso {
        nombre: "paris",
        ids: &[2, 818, 5279, 529, 7001, 563, 9079, 236761, 669, 5279, 529, 9405, 563],
        top1: 9079,
        p1: 0.9926,
        top2: 1390,
    },
    Caso {
        nombre: "fibonacci",
        ids: &[
            2, 2063, 10779, 78113, 236769, 236749, 1473, 107, 140, 584, 538, 655, 236743, 236778, 236787, 107, 144, 2060, 538,
            107, 140, 2060, 10779, 78113, 236769, 236749, 753, 236743, 236770, 236768, 900,
        ],
        top1: 10779,
        p1: 0.6700,
        top2: 568,
    },
    Caso {
        nombre: "numeros",
        ids: &[
            2, 2805, 236764, 4164, 236764, 14783, 236764, 28474, 236764, 30079, 236764, 42079, 236764, 64582, 236764, 79304, 236764,
            100969, 236764,
        ],
        top1: 115272,
        p1: 0.6711,
        top2: 64201,
    },
];

#[test]
#[ignore = "necesita target/gemma4-12b-model (7 GB)"]
fn gemma4_coincide_con_llama_cpp() {
    let dir = format!("{}/../../target/gemma4-12b-model", env!("CARGO_MANIFEST_DIR"));
    let leer = |f: &str| std::fs::read(format!("{dir}/{f}")).unwrap_or_else(|e| panic!("falta {dir}/{f}: {e}"));
    let manifest = Manifest::parse(&leer("manifest.som")).expect("manifest válido");
    let index = TensorIndex::parse(&leer("index.som")).expect("index válido");
    let mut rt = Runtime::new(manifest, index.clone(), 0, 0);
    rt.validate_shapes().expect("formas coherentes");
    let mut source = MmapTensorSource::new(format!("{dir}/shards"), index, OwnedMapper::default());
    let pool = std::env::var("HILOS").ok().and_then(|v| v.parse::<usize>().ok()).filter(|&n| n > 1).map(HostPool::new);
    let par: Option<&dyn RowParallel> = pool.as_ref().map(|p| p as &dyn RowParallel);

    for c in CASOS {
        rt.reset_sequence();
        rt.prefill_prompt(&mut source, c.ids, par, &mut None, None).expect("prefill");
        let logits = rt.logits_par(&mut source, par).expect("logits").to_vec();
        let max = logits.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        let suma: f64 = logits.iter().map(|&l| ((l - max) as f64).exp()).sum();
        let mut orden: Vec<usize> = (0..logits.len()).collect();
        orden.sort_by(|&a, &b| logits[b].partial_cmp(&logits[a]).unwrap());
        let p1 = ((logits[orden[0]] - max) as f64).exp() / suma;
        assert_eq!(orden[0] as u32, c.top1, "{}: top-1", c.nombre);
        assert_eq!(orden[1] as u32, c.top2, "{}: top-2", c.nombre);
        assert!((p1 - c.p1).abs() < 0.12, "{}: p(top-1) {p1:.4} vs referencia {:.4}", c.nombre, c.p1);
    }
}
