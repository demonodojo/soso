//! Los tokens que cierran un turno de Qwen2.5 son dos: `<|im_end|>` y `<|endoftext|>`. El perfil
//! del guest sólo paraba en `tokenizer.eos()` y, cuando el modelo escribía `<|endoftext|>`, seguía
//! generando (T22, 2026-10-10: inventaba una respuesta de usuario y la salida de `cargo test`).

use soso_llm_core::tokenizer::Tokenizer;

fn tokenizer() -> Tokenizer {
    let ruta = concat!(env!("CARGO_MANIFEST_DIR"), "/../../target/qwen2.5-coder-7b-model/tokenizer.som");
    let datos = std::fs::read(ruta).unwrap_or_else(|e| panic!("falta {ruta}: {e}"));
    Tokenizer::parse(&datos).expect("tokenizer.som")
}

#[test]
fn los_dos_tokens_de_parada_se_codifican_a_un_solo_id() {
    let t = tokenizer();
    assert_eq!(t.encode_trozo("<|im_end|>", false, false), vec![151_645]);
    assert_eq!(t.encode_trozo("<|endoftext|>", false, false), vec![151_643]);
    assert_eq!(t.eos(), Some(151_645), "el eos del tokenizer es `<|im_end|>`: el segundo hay que añadirlo");
}
