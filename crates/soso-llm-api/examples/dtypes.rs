//! Diagnóstico: cuenta tensores por (rol, dtype, forma) de un modelo convertido.
use sosomodel::TensorIndex;
use std::collections::BTreeMap;
fn main() {
    let dir = std::env::args().nth(1).expect("dir");
    let idx = TensorIndex::parse(&std::fs::read(format!("{dir}/index.som")).unwrap()).unwrap();
    let mut m: BTreeMap<String, usize> = BTreeMap::new();
    for t in &idx.entries {
        let rol: String = t
            .name
            .split('.')
            .filter(|p| {
                !(p.starts_with('L') && p[1..].chars().all(|c| c.is_ascii_digit()))
                    && !(p.starts_with('E') && p[1..].chars().all(|c| c.is_ascii_digit()))
            })
            .collect::<Vec<_>>()
            .join(".");
        *m.entry(format!("{rol} dtype={} shape={:?}", t.dtype, t.shape))
            .or_default() += 1;
    }
    for (k, v) in m {
        println!("{v:5} {k}");
    }
}
