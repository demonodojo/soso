//! Histograma de formatos de peso de un modelo `.som` (bytes por dtype).
//! `cargo run --release -p soso-llm-core --features std --example hist_dtypes -- target/qwen2.5-coder-14b-model`
use std::collections::BTreeMap;

use sosomodel::index::TensorIndex;

fn main() {
    let dir = std::env::args().nth(1).expect("dir del modelo");
    let datos = std::fs::read(format!("{dir}/index.som")).expect("index.som");
    let idx = TensorIndex::parse(&datos).expect("índice");
    let mut por: BTreeMap<u8, (usize, u64)> = BTreeMap::new();
    let mut ejemplos: BTreeMap<u8, Vec<String>> = BTreeMap::new();
    for e in &idx.entries {
        let x = por.entry(e.dtype).or_default();
        x.0 += 1;
        x.1 += e.byte_len;
        let v = ejemplos.entry(e.dtype).or_default();
        if v.len() < 4 && !e.name.starts_with("L0") || v.len() < 2 {
            v.push(e.name.clone());
        }
    }
    let total: u64 = por.values().map(|x| x.1).sum();
    for (d, (n, b)) in &por {
        println!("dtype {d}: {n} tensores, {:.2} GiB ({:.1}%)  p.ej. {:?}", *b as f64 / 1073741824.0, 100.0 * *b as f64 / total as f64, ejemplos[d]);
    }
}
