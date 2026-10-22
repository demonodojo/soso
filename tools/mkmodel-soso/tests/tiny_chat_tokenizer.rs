//! El `tiny` de pruebas trae vocabulario para la API de chat (familia qwen2).

use soso_llm_core::conversation::render_messages;
use soso_llm_core::conversation::{ChatInput, Message, ModelProfile, Role};
use soso_llm_core::tokenizer::Tokenizer;
use sosomodel::layout::TOKENIZER_FILE;

#[test]
fn tiny_chat_tokenizer_renderiza_di_hola() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/tiny-model");
    if !root.join(TOKENIZER_FILE).is_file() {
        eprintln!("skip: falta {}; corre mkmodel-soso antes", root.display());
        return;
    }
    let som = std::fs::read(root.join(TOKENIZER_FILE)).expect("tokenizer.som");
    let tok = Tokenizer::parse(&som).expect("parse");
    assert!(tok.tiene_vocabulario());
    let profile = ModelProfile {
        id: String::from("tiny"),
        directory: root.to_string_lossy().into_owned(),
        family: String::from("qwen2"),
        weights_sha256: String::new(),
        tokenizer_sha256: String::new(),
        template_sha256: String::new(),
        context_tokens: 128,
        max_output_tokens: 64,
        stop_token_ids: vec![tok.eos().unwrap_or(4)],
    };
    let input = ChatInput::nuevo(vec![Message::texto(Role::User, "di hola")]);
    let ids = render_messages(&input, &profile, &tok).expect("render");
    assert!(!ids.is_empty());
}
