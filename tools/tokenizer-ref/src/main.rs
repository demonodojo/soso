//! Generador offline de fixtures de referencia del modelo (ficha T03).
//!
//! Produce, para cinco conversaciones fijas, **el texto renderizado y los token
//! IDs según el modelo original**, usando su plantilla de chat y su tokenizer
//! tal y como los publica Hugging Face en una revisión concreta.
//!
//! Por qué existe separado del proyecto:
//!
//! - La referencia tiene que ser **independiente** de lo que se va a probar. Si
//!   la generase el renderer de soso, estaríamos comprobando que un programa
//!   coincide consigo mismo.
//! - `tokenizers` y `minijinja` son la implementación oficial (la misma que
//!   lee `tokenizer.json`) y un motor Jinja: buenos para preparar fixtures,
//!   malos como dependencia permanente de un sistema operativo que quiere
//!   compilarse a sí mismo. Por eso esta crate está **fuera del workspace**.
//!
//! Uso:
//!
//! ```text
//! cargo run --manifest-path tools/tokenizer-ref/Cargo.toml -- \
//!     --modelo target/self-improvement/modelo \
//!     --salida tests/self-improvement/reference \
//!     --revision <sha>
//! ```

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use minijinja::{context, Environment};
use serde::Serialize;
use sha2::{Digest, Sha256};
use tokenizers::Tokenizer;

/// Las cinco conversaciones que exige la ficha.
fn conversaciones() -> Vec<(&'static str, serde_json::Value, Option<serde_json::Value>)> {
    let herramienta = serde_json::json!({
        "type": "function",
        "function": {
            "name": "leer_archivo",
            "description": "Lee las primeras líneas de un archivo del repositorio",
            "parameters": {
                "type": "object",
                "properties": {
                    "ruta": {"type": "string", "description": "Ruta relativa"},
                    "lineas": {"type": "integer", "description": "Cuántas líneas"}
                },
                "required": ["ruta", "lineas"]
            }
        }
    });
    vec![
        (
            "simple",
            serde_json::json!([{"role": "user", "content": "Di la palabra LISTO y nada más."}]),
            None,
        ),
        (
            "system",
            serde_json::json!([
                {"role": "system", "content": "Responde siempre con una sola palabra en mayúsculas."},
                {"role": "user", "content": "¿Estás preparado? Contesta LISTO."}
            ]),
            None,
        ),
        (
            "historial",
            serde_json::json!([
                {"role": "user", "content": "Mi número favorito es el 7. Apúntalo."},
                {"role": "assistant", "content": "Apuntado."},
                {"role": "user", "content": "¿Cuál es mi número favorito? Responde solo con la cifra."}
            ]),
            None,
        ),
        (
            "herramienta-esquema",
            serde_json::json!([
                {"role": "user", "content": "Enséñame las primeras 20 líneas de kernel/src/main.rs"}
            ]),
            Some(serde_json::json!([herramienta])),
        ),
        (
            "herramienta-resultado",
            serde_json::json!([
                {"role": "user", "content": "Enséñame las primeras 20 líneas de kernel/src/main.rs"},
                {"role": "assistant", "content": "", "tool_calls": [{
                    "type": "function",
                    "function": {"name": "leer_archivo",
                                 "arguments": {"ruta": "kernel/src/main.rs", "lineas": 20}}
                }]},
                {"role": "tool", "content": "fn main() { /* … */ }"}
            ]),
            Some(serde_json::json!([herramienta])),
        ),
    ]
}

#[derive(Serialize)]
struct Fixture {
    schema_version: u32,
    nombre: String,
    /// Modelo y revisión exactos de los que sale esta referencia.
    modelo: String,
    revision: String,
    /// Entrada tal y como se le pasó a la plantilla.
    mensajes: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    herramientas: Option<serde_json::Value>,
    add_generation_prompt: bool,
    /// Lo que produce la plantilla oficial.
    texto: String,
    texto_sha256: String,
    /// Lo que produce el tokenizer oficial sobre ese texto.
    ids: Vec<u32>,
    ids_sha256: String,
    /// Tokens especiales presentes, por si la divergencia está ahí.
    especiales: BTreeMap<String, u32>,
    generador: Generador,
}

#[derive(Serialize, Clone)]
struct Generador {
    herramienta: String,
    tokenizers: String,
    minijinja: String,
    tokenizer_json_sha256: String,
    tokenizer_config_sha256: String,
}

fn sha256_hex(datos: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(datos);
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

fn arg(nombre: &str) -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    args.iter()
        .position(|a| a == nombre)
        .and_then(|i| args.get(i + 1).cloned())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let modelo = PathBuf::from(arg("--modelo").unwrap_or_else(|| {
        "target/self-improvement/modelo".to_string()
    }));
    let salida = PathBuf::from(arg("--salida").unwrap_or_else(|| {
        "tests/self-improvement/reference".to_string()
    }));
    let revision = arg("--revision").unwrap_or_else(|| "desconocida".to_string());
    let nombre_modelo = arg("--nombre").unwrap_or_else(|| "Qwen/Qwen2.5-Coder-3B-Instruct".into());

    let config_bytes = std::fs::read(modelo.join("tokenizer_config.json"))?;
    let tokenizer_bytes = std::fs::read(modelo.join("tokenizer.json"))?;
    let config: serde_json::Value = serde_json::from_slice(&config_bytes)?;
    let plantilla = config["chat_template"]
        .as_str()
        .ok_or("tokenizer_config.json sin chat_template")?
        .to_string();

    let tokenizer = Tokenizer::from_file(modelo.join("tokenizer.json"))
        .map_err(|e| format!("tokenizer.json: {e}"))?;

    let mut entorno = Environment::new();
    // `transformers` expone esto a las plantillas; sin ello, las que validan
    // entradas fallan al compilar.
    entorno.add_function("raise_exception", |msg: String| -> Result<(), minijinja::Error> {
        Err(minijinja::Error::new(minijinja::ErrorKind::InvalidOperation, msg))
    });
    entorno.add_template_owned("chat", plantilla.clone())?;
    let chat = entorno.get_template("chat")?;

    let generador = Generador {
        herramienta: format!("tokenizer-ref {}", env!("CARGO_PKG_VERSION")),
        tokenizers: "tokenizers 0.20".to_string(),
        minijinja: "minijinja 2".to_string(),
        tokenizer_json_sha256: sha256_hex(&tokenizer_bytes),
        tokenizer_config_sha256: sha256_hex(&config_bytes),
    };

    std::fs::create_dir_all(&salida)?;
    let mut indice = Vec::new();

    for (nombre, mensajes, herramientas) in conversaciones() {
        let texto: String = match &herramientas {
            Some(t) => chat.render(context! {
                messages => mensajes.clone(),
                tools => t.clone(),
                add_generation_prompt => true,
            })?,
            None => chat.render(context! {
                messages => mensajes.clone(),
                add_generation_prompt => true,
            })?,
        };
        // Los tokens especiales del texto renderizado tienen que reconocerse
        // como tales: por eso `add_special_tokens = false` (la plantilla ya los
        // pone) pero el tokenizer sí los segmenta como piezas propias.
        let codificado = tokenizer
            .encode(texto.as_str(), false)
            .map_err(|e| format!("{nombre}: {e}"))?;
        let ids = codificado.get_ids().to_vec();

        let mut especiales = BTreeMap::new();
        for pieza in ["<|im_start|>", "<|im_end|>", "<|endoftext|>"] {
            if let Some(id) = tokenizer.token_to_id(pieza) {
                especiales.insert(pieza.to_string(), id);
            }
        }

        let fixture = Fixture {
            schema_version: 1,
            nombre: nombre.to_string(),
            modelo: nombre_modelo.clone(),
            revision: revision.clone(),
            mensajes,
            herramientas,
            add_generation_prompt: true,
            texto_sha256: sha256_hex(texto.as_bytes()),
            ids_sha256: sha256_hex(
                &ids.iter()
                    .flat_map(|i| i.to_le_bytes())
                    .collect::<Vec<u8>>(),
            ),
            texto,
            ids,
            especiales,
            generador: generador.clone(),
        };

        let ruta = salida.join(format!("{nombre}.json"));
        let mut texto_json = serde_json::to_string_pretty(&fixture)?;
        texto_json.push('\n');
        std::fs::write(&ruta, texto_json)?;
        println!(
            "{nombre}: {} tokens, texto {} bytes → {}",
            fixture.ids.len(),
            fixture.texto.len(),
            ruta.display()
        );
        indice.push(serde_json::json!({
            "nombre": nombre,
            "archivo": format!("{nombre}.json"),
            "tokens": fixture.ids.len(),
            "texto_sha256": fixture.texto_sha256,
            "ids_sha256": fixture.ids_sha256,
        }));
    }

    let resumen = serde_json::json!({
        "schema_version": 1,
        "modelo": nombre_modelo,
        "revision": revision,
        "vocab_size": tokenizer.get_vocab_size(true),
        "generador": generador,
        "fixtures": indice,
    });
    let mut texto = serde_json::to_string_pretty(&resumen)?;
    texto.push('\n');
    std::fs::write(salida.join("index.json"), texto)?;
    println!("índice en {}", salida.join("index.json").display());
    let _ = Path::new(&modelo);
    Ok(())
}
