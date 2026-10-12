//! Segmentación de Gemma 4 igual que llama.cpp (G5 de `docs/PLAN-GEMMA4.md`).
//!
//! `fixtures/gemma4_tokenize.json` lleva, por texto, los ids que devolvió `llama-server` en `/tokenize`
//! (`add_special: true`, `parse_special: true`). Usa el `tokenizer.som` del modelo convertido; sin él
//! falla con la ruta, de ahí el `#[ignore]`.
//!
//! ```sh
//! cargo test --release -p soso-llm-core --features std --test tokenizer_gemma4 -- --ignored
//! ```
#![cfg(feature = "std")]

use soso_llm_core::tokenizer::{Segmentacion, Tokenizer};

#[test]
#[ignore = "necesita target/gemma4-12b-model/tokenizer.som"]
fn gemma4_segmenta_igual_que_llama_cpp() {
    let raiz = env!("CARGO_MANIFEST_DIR");
    let ruta = format!("{raiz}/../../target/gemma4-12b-model/tokenizer.som");
    let bytes = std::fs::read(&ruta).unwrap_or_else(|e| panic!("falta {ruta}: {e}"));
    let tok = Tokenizer::parse(&bytes).expect("tokenizer.som válido");
    let Tokenizer::Vocab(v) = &tok else { panic!("sin vocabulario") };
    assert_eq!(v.segmentacion(), Segmentacion::BpeSpm);

    let fixture = std::fs::read_to_string(format!("{raiz}/tests/fixtures/gemma4_tokenize.json")).unwrap();
    let casos: serde_json::Value = serde_json::from_str(&fixture).unwrap();
    let mut fallos = Vec::new();
    for c in casos.as_array().unwrap() {
        let texto = c["texto"].as_str().unwrap();
        let esperado: Vec<u32> = c["ids"].as_array().unwrap().iter().map(|x| x.as_u64().unwrap() as u32).collect();
        let obtenido = tok.encode(texto);
        if obtenido != esperado {
            let i = obtenido.iter().zip(&esperado).position(|(a, b)| a != b).unwrap_or(obtenido.len().min(esperado.len()));
            fallos.push(format!(
                "{:?}: primera diferencia en {i}: soso {:?} vs llama {:?} (largos {} / {})",
                &texto.chars().take(40).collect::<String>(),
                obtenido.get(i.saturating_sub(1)..(i + 3).min(obtenido.len())),
                esperado.get(i.saturating_sub(1)..(i + 3).min(esperado.len())),
                obtenido.len(),
                esperado.len()
            ));
        } else {
            // ida y vuelta: decodificar devuelve el texto (sin el BOS)
            assert_eq!(tok.decode(&obtenido), texto, "ida y vuelta de {:?}", &texto.chars().take(40).collect::<String>());
        }
    }
    assert!(fallos.is_empty(), "{} caso(s) distintos de llama.cpp:\n{}", fallos.len(), fallos.join("\n"));
}

/// Vocabulario de juguete con la forma de Gemma 4: sin modelo, corre siempre.
#[test]
fn spm_sintetico_ida_y_vuelta_v3() {
    use soso_llm_core::tokenizer::VocabTokenizer;
    let piezas: Vec<String> = ["<pad>", "<bos>", "<turn>", "a", "b", "\u{2581}", "ab", "\u{2581}ab", "\n", "<0xC3>", "<0xA9>"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    // fusiones: a+b → ab (rango 0), ▁+ab → ▁ab (rango 1)
    let merges = [(3u32, 4u32), (5, 6)];
    let som = VocabTokenizer::serialize_spm(&piezas, 1, 1, &merges, &[0, 1, 2]);
    let tok = Tokenizer::parse(&som).expect("v3 válido");
    let Tokenizer::Vocab(v) = &tok else { panic!() };
    assert_eq!(v.segmentacion(), Segmentacion::BpeSpm);
    // BOS, «▁ab» fusionado, salto de línea, token especial literal, «é» sin pieza → bytes de reserva
    assert_eq!(tok.encode(" ab\n<turn>é"), vec![1, 7, 8, 2, 9, 10]);
    assert_eq!(tok.decode(&[7, 8, 3]), " ab\na");
}
