//! Sonda de T59: volcar el **prompt renderizado** de una petición grabada.
//!
//! La campaña de T14 vio que el modelo emite la llamada a herramienta como
//! texto plano, sin las etiquetas `<tool_call>` que el parser de T07 busca. Eso
//! admite dos explicaciones opuestas —el render no se lo pide bien, o el modelo
//! no obedece— y arreglar el parser sin mirar cuál es sería tapar el síntoma.
//!
//! Esto no genera nada: sólo enseña lo que el modelo vería.
//!
//! ```sh
//! cargo run -p soso-llm-api --features std --example render_dump -- \
//!   target/qwen2.5-coder-3b-model tests/self-improvement/cases/visible/Q07/peticion.json
//! ```

use soso_llm_api::prepare_chat_completion;
use soso_llm_core::conversation::ModelProfile;
use soso_llm_core::tokenizer::Tokenizer;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() != 2 {
        eprintln!("uso: render_dump <dir-modelo> <peticion.json>");
        std::process::exit(2);
    }
    let (dir, peticion) = (&args[0], &args[1]);

    let datos = std::fs::read(format!("{dir}/tokenizer.som")).unwrap_or_else(|e| {
        eprintln!("no leo {dir}/tokenizer.som: {e}");
        std::process::exit(1);
    });
    let tokenizer = Tokenizer::parse(&datos).unwrap_or_else(|_| {
        eprintln!("{dir}/tokenizer.som inválido");
        std::process::exit(1);
    });

    let cuerpo = std::fs::read_to_string(peticion).unwrap_or_else(|e| {
        eprintln!("no leo {peticion}: {e}");
        std::process::exit(1);
    });
    // El `model` de la petición grabada no tiene por qué coincidir con nada:
    // aquí sólo se renderiza, así que el perfil se ajusta al documento.
    let wire: serde_json::Value = serde_json::from_str(&cuerpo).expect("petición JSON");
    let id = wire["model"].as_str().unwrap_or("modelo").to_string();

    let profile = ModelProfile {
        id,
        directory: dir.clone(),
        family: String::from("qwen2"),
        weights_sha256: String::new(),
        tokenizer_sha256: String::new(),
        template_sha256: String::new(),
        context_tokens: 32768,
        max_output_tokens: 256,
        stop_token_ids: Vec::new(),
    };

    match prepare_chat_completion(&cuerpo, &profile, &tokenizer) {
        Ok(p) => {
            let ids = soso_llm_core::conversation::render_messages(&p.input, &profile, &tokenizer)
                .expect("render");
            println!("--- {} tokens ---", ids.len());
            println!("{}", tokenizer.decode(&ids));
            println!("--- ids ---");
            // El texto decodificado **no** basta: `decode` suele ocultar el
            // EOS, así que un `<|im_end|>` ausente en la pantalla puede estar
            // perfectamente presente en el prompt. Se mira sobre los ids.
            println!("{ids:?}");
            println!("--- fin ---");
        }
        Err(e) => {
            eprintln!("prepare falló: {e:?}");
            std::process::exit(1);
        }
    }
}
