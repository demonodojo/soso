//! Tokeniza un fichero de texto con el `tokenizer.som` de un modelo y escribe los ids separados por comas: `tokenizar <dir-modelo> <texto.txt> [max]`.
use soso_llm_core::tokenizer::Tokenizer;

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let tk = Tokenizer::parse(&std::fs::read(format!("{}/tokenizer.som", a[0])).expect("tokenizer.som")).expect("tokenizer válido");
    let texto = std::fs::read_to_string(&a[1]).expect("texto");
    let mut ids = tk.encode(&texto);
    if let Some(m) = a.get(2).and_then(|s| s.parse::<usize>().ok()) {
        ids.truncate(m);
    }
    println!("{}", ids.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(","));
}
