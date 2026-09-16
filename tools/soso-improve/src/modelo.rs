//! Orden `modelo`: perfil del modelo y comparación de fixtures (T03).
//!
//! El tokenizer de soso se carga del `.som` del modelo y se envuelve en el
//! trait `Tokeniza` del core. Toda la política —comparar, encontrar la primera
//! divergencia y clasificarla— vive allí, así que la misma comparación se hace
//! dentro de soso sin cambiar una línea.

use soso_improve_core::entorno::Archivos;
use soso_improve_core::referencia::{self, Tokeniza};
use soso_improve_core::{unir, Error, Resultado};
use soso_llm_core::tokenizer::{Segmentacion, Tokenizer};
use sosomodel::manifest::Manifest;

use crate::sistema::Host;
use crate::Opciones;

/// El tokenizer del `.som`, con lo que pide el comparador.
struct TokenizerSoso(Tokenizer);

impl TokenizerSoso {
    /// Qué algoritmo pide el vocabulario y cuántas fusiones trae. Un
    /// vocabulario byte-level sin fusiones no puede reproducir la segmentación
    /// oficial (T52/T53), y eso tiene que verse en el perfil.
    fn segmentacion(&self) -> &'static str {
        match &self.0 {
            Tokenizer::Vocab(v) => match v.segmentacion() {
                Segmentacion::BpeByteLevel => "bpe-bytelevel",
                Segmentacion::PiezaMasLarga => "pieza-mas-larga",
            },
            Tokenizer::ByteLevel => "bytes",
        }
    }

    fn merges(&self) -> usize {
        match &self.0 {
            Tokenizer::Vocab(v) => v.merges().len(),
            Tokenizer::ByteLevel => 0,
        }
    }
}

impl Tokeniza for TokenizerSoso {
    fn encode(&self, texto: &str) -> Vec<u32> {
        self.0.encode(texto)
    }

    fn vocab_size(&self) -> u32 {
        self.0.vocab_len() as u32
    }

    fn pieza(&self, id: u32) -> Option<String> {
        // La pieza tal cual está en el vocabulario: `decode` traduce bytes y se
        // come los marcadores de parada, que son los que más interesa ver.
        match &self.0 {
            Tokenizer::Vocab(v) => v.pieza_de(id).map(String::from),
            Tokenizer::ByteLevel => None,
        }
    }
}

fn cargar_tokenizer(modelo: &str) -> Resultado<TokenizerSoso> {
    let ruta = unir(modelo, "tokenizer.som");
    let datos = Host.leer(&ruta)?;
    let tok = Tokenizer::parse(&datos)
        .map_err(|_| Error::formato(format!("{ruta}: tokenizer.som inválido")))?;
    if !tok.tiene_vocabulario() {
        return Err(Error::uso(format!(
            "{ruta} no trae vocabulario: comparar contra un tokenizer de bytes no dice nada"
        )));
    }
    Ok(TokenizerSoso(tok))
}

pub fn despachar(sub: &str, opciones: &Opciones) -> Resultado<i32> {
    match sub {
        "comparar" => comparar(opciones),
        "detalle" => detalle(opciones),
        "coste" => coste(opciones),
        "perfil" => perfil(opciones),
        otro => Err(Error::uso(format!(
            "modelo: subcomando desconocido {otro} (usa: perfil | comparar | detalle | coste)"
        ))),
    }
}

/// Enseña las dos secuencias lado a lado alrededor de un punto.
///
/// Sin esto, «segmentación en el token 18» no le dice nada a nadie: hace falta
/// ver qué pieza puso cada uno para saber si el problema es el vocabulario, el
/// algoritmo o los tokens especiales.
fn detalle(opciones: &Opciones) -> Resultado<i32> {
    let fixtures = opciones
        .uno("fixtures")
        .filter(|s| !s.is_empty())
        .unwrap_or("tests/self-improvement/reference")
        .to_string();
    let modelo = opciones.exigido("modelo")?.to_string();
    let caso = opciones.exigido("caso")?.to_string();
    let desde: usize = opciones.uno("desde").and_then(|v| v.parse().ok()).unwrap_or(0);
    let cuantos: usize = opciones.uno("n").and_then(|v| v.parse().ok()).unwrap_or(14);

    let tokenizer = cargar_tokenizer(&modelo)?;
    let fixture = referencia::leer_fixture(&Host, &fixtures, &format!("{caso}.json"))?;
    let obtenidos = tokenizer.encode(&fixture.texto);

    crate::digo!("{caso}: referencia {} tokens, soso {} tokens",
                 fixture.ids.len(), obtenidos.len());
    crate::digo!("{:>5}  {:>8} {:<18}  {:>8} {:<18}", "#", "ref", "pieza", "soso", "pieza");
    for i in desde..(desde + cuantos).min(fixture.ids.len().max(obtenidos.len())) {
        let r = fixture.ids.get(i).copied();
        let o = obtenidos.get(i).copied();
        let marca = if r == o { " " } else { "≠" };
        crate::digo!(
            "{marca}{i:>4}  {:>8} {:<18}  {:>8} {:<18}",
            r.map(|v| v.to_string()).unwrap_or_default(),
            r.and_then(|v| tokenizer.pieza(v)).map(|p| format!("{p:?}")).unwrap_or_default(),
            o.map(|v| v.to_string()).unwrap_or_default(),
            o.and_then(|v| tokenizer.pieza(v)).map(|p| format!("{p:?}")).unwrap_or_default(),
        );
    }
    Ok(0)
}

fn comparar(opciones: &Opciones) -> Resultado<i32> {
    let fixtures = opciones
        .uno("fixtures")
        .filter(|s| !s.is_empty())
        .unwrap_or("tests/self-improvement/reference")
        .to_string();
    let modelo = opciones.exigido("modelo")?.to_string();

    let tokenizer = cargar_tokenizer(&modelo)?;
    let informe = referencia::comparar_todos(&Host, &fixtures, &tokenizer)?;

    if let Some(ruta) = opciones.uno("json").filter(|s| !s.is_empty()) {
        let mut texto = serde_json::to_string_pretty(&informe).map_err(Error::formato)?;
        texto.push('\n');
        let mut host = Host;
        host.escribir(ruta, texto.as_bytes(), 0o644)?;
    }

    crate::digo!(
        "{} @ {}: {} fixture(s) iguales, {} con divergencia (vocab referencia {}, soso {})",
        informe.modelo,
        &informe.revision[..informe.revision.len().min(12)],
        informe.iguales,
        informe.divergentes,
        informe.vocab_referencia,
        informe.vocab_soso
    );
    for c in &informe.comparaciones {
        match &c.divergencia {
            None => crate::digo!("  {:22} igual ({} tokens)", c.fixture, c.tokens_referencia),
            Some(d) => {
                crate::digo!(
                    "  {:22} {} en el token {} de {} (soso: {})",
                    c.fixture,
                    d.clase.como_texto(),
                    d.posicion,
                    c.tokens_referencia,
                    c.tokens_soso
                );
                crate::aviso!(
                    "      esperado {:?} {:?} · obtenido {:?} {:?}",
                    d.esperado,
                    d.pieza_esperada,
                    d.obtenido,
                    d.pieza_obtenida
                );
            }
        }
        if !c.fuera_de_vocabulario.is_empty() {
            crate::aviso!(
                "      {} id(s) de la referencia fuera del vocabulario de soso; el primero: {}",
                c.fuera_de_vocabulario.len(),
                c.fuera_de_vocabulario[0]
            );
        }
    }
    Ok(informe.codigo())
}

/// Escribe el `model-lock.json` que fija qué modelo se está usando (T03).
///
/// Junta las dos mitades: lo que publica el origen (revisión, plantilla y
/// tokenizer originales) y lo que hay convertido en soso (manifiesto, índice y
/// tokenizer `.som`), cada cosa con su hash. Sin esto, «el modelo» es un
/// directorio que cambia sin avisar.
fn perfil(opciones: &Opciones) -> Resultado<i32> {
    let modelo = opciones.exigido("modelo")?.to_string();
    let original = opciones.exigido("original")?.to_string();
    let salida = opciones.exigido("out")?.to_string();
    let revision = opciones.uno("revision").unwrap_or("desconocida").to_string();
    let origen = opciones
        .uno("origen")
        .unwrap_or("Qwen/Qwen2.5-Coder-3B-Instruct")
        .to_string();

    let manifiesto_bytes = Host.leer(&unir(&modelo, "manifest.som"))?;
    let manifiesto = Manifest::parse(&manifiesto_bytes)
        .map_err(|_| Error::formato(format!("{modelo}/manifest.som inválido")))?;
    let tokenizer = cargar_tokenizer(&modelo)?;

    let mut hashes = serde_json::Map::new();
    for (etiqueta, base, archivo) in [
        ("som.manifest", modelo.as_str(), "manifest.som"),
        ("som.index", modelo.as_str(), "index.som"),
        ("som.tokenizer", modelo.as_str(), "tokenizer.som"),
        ("original.tokenizer_json", original.as_str(), "tokenizer.json"),
        ("original.tokenizer_config", original.as_str(), "tokenizer_config.json"),
        ("original.config", original.as_str(), "config.json"),
        ("original.generation_config", original.as_str(), "generation_config.json"),
    ] {
        let ruta = unir(base, archivo);
        match Host.leer(&ruta) {
            Ok(datos) => {
                hashes.insert(
                    etiqueta.to_string(),
                    serde_json::json!({
                        "ruta": ruta,
                        "bytes": datos.len(),
                        "sha256": soso_improve_core::sha256_hex(&datos),
                    }),
                );
            }
            Err(_) => {
                hashes.insert(etiqueta.to_string(), serde_json::Value::Null);
            }
        }
    }

    let config: serde_json::Value = serde_json::from_slice(
        &Host.leer(&unir(&original, "config.json")).unwrap_or_default(),
    )
    .unwrap_or(serde_json::Value::Null);
    let generacion: serde_json::Value = serde_json::from_slice(
        &Host.leer(&unir(&original, "generation_config.json")).unwrap_or_default(),
    )
    .unwrap_or(serde_json::Value::Null);
    let tokenizer_config: serde_json::Value = serde_json::from_slice(
        &Host.leer(&unir(&original, "tokenizer_config.json")).unwrap_or_default(),
    )
    .unwrap_or(serde_json::Value::Null);

    let lock = serde_json::json!({
        "schema_version": 1,
        "origen": {
            "repositorio": origen,
            "revision": revision,
            "familia": config.get("model_type").cloned().unwrap_or(serde_json::Value::Null),
            "arquitecturas": config.get("architectures").cloned().unwrap_or(serde_json::Value::Null),
            "vocab_size": config.get("vocab_size").cloned().unwrap_or(serde_json::Value::Null),
            "max_position_embeddings": config.get("max_position_embeddings").cloned()
                .unwrap_or(serde_json::Value::Null),
            "eos_token": tokenizer_config.get("eos_token").cloned().unwrap_or(serde_json::Value::Null),
            "pad_token": tokenizer_config.get("pad_token").cloned().unwrap_or(serde_json::Value::Null),
            "add_bos_token": tokenizer_config.get("add_bos_token").cloned()
                .unwrap_or(serde_json::Value::Null),
            "eos_token_id_generacion": generacion.get("eos_token_id").cloned()
                .unwrap_or(serde_json::Value::Null),
            "chat_template_sha256": tokenizer_config.get("chat_template")
                .and_then(|t| t.as_str())
                .map(|t| serde_json::Value::String(soso_improve_core::sha256_hex(t.as_bytes())))
                .unwrap_or(serde_json::Value::Null),
        },
        "som": {
            "directorio": modelo,
            "nombre": manifiesto.name,
            "vocab_size": manifiesto.vocab_size,
            "hidden_dim": manifiesto.hidden_dim,
            "num_layers": manifiesto.num_layers,
            "num_heads": manifiesto.num_heads,
            "num_kv_heads": manifiesto.num_kv_heads,
            "ffn_dim": manifiesto.ffn_dim,
            "max_seq": manifiesto.max_seq,
            "rope_theta": manifiesto.rope_theta,
            "num_experts": manifiesto.num_experts,
            "chat_template": manifiesto.chat_template,
            "chat_template_sha256": soso_improve_core::sha256_hex(
                manifiesto.chat_template.as_bytes()),
            "tokenizer_vocab_len": tokenizer.vocab_size(),
            "tokenizer_segmentacion": tokenizer.segmentacion(),
            "tokenizer_merges": tokenizer.merges(),
            "capas": manifiesto.layers.len(),
        },
        "hashes": hashes,
        "nota": "La cuantización no la declara el manifiesto .som: viaja por tensor en \
                 el índice. Se registra el hash del índice, que es lo que identifica \
                 los pesos convertidos.",
    });
    let mut texto = serde_json::to_string_pretty(&lock).map_err(Error::formato)?;
    texto.push('\n');
    let mut host = Host;
    host.escribir(&salida, texto.as_bytes(), 0o644)?;
    crate::digo!("perfil en {salida}: {} ({} capas, vocab .som {})",
                 origen, manifiesto.layers.len(), manifiesto.vocab_size);
    Ok(0)
}

/// Mide cuánto cuesta segmentar (lo exige T53).
///
/// El bucle de fusión no puede ser cuadrático: aquí entran prompts de miles de
/// tokens y esto corre también dentro de soso, sin `std`.
fn coste(opciones: &Opciones) -> Resultado<i32> {
    let modelo = opciones.exigido("modelo")?.to_string();
    let fixtures = opciones
        .uno("fixtures")
        .filter(|s| !s.is_empty())
        .unwrap_or("tests/self-improvement/reference")
        .to_string();
    let tokenizer = cargar_tokenizer(&modelo)?;
    let fixture = referencia::leer_fixture(&Host, &fixtures, "herramienta-resultado.json")?;

    // `decode` suprime el token de parada a propósito, y lo hace desde siempre:
    // el bucle de generación no quiere imprimirlo. Para juzgar la ida y vuelta
    // se compara contra el texto sin él.
    let parada = tokenizer
        .0
        .eos()
        .and_then(|id| tokenizer.pieza(id))
        .unwrap_or_default();
    // El texto del fixture trae el marcador literal; `decode` no lo devuelve.

    for (nombre, veces) in [("fixture", 1usize), ("x16", 16), ("x64", 64)] {
        let texto = fixture.texto.repeat(veces);
        let inicio = std::time::Instant::now();
        let ids = tokenizer.encode(&texto);
        let ms = inicio.elapsed().as_secs_f64() * 1000.0;
        let vuelta = tokenizer.0.decode(&ids);
        let esperado = if parada.is_empty() {
            texto.clone()
        } else {
            texto.replace(&parada, "")
        };
        let igual = vuelta == esperado;
        crate::digo!(
            "{nombre:8} {:>8} bytes → {:>6} tokens en {ms:7.1} ms · ida y vuelta {}",
            texto.len(),
            ids.len(),
            if igual { "idéntica" } else { "DIFIERE" }
        );
        if !igual && veces == 1 {
            let corte = esperado
                .char_indices()
                .zip(vuelta.char_indices())
                .find(|((_, a), (_, b))| a != b)
                .map(|((i, _), _)| i)
                .unwrap_or(esperado.len().min(vuelta.len()));
            crate::aviso!(
                "      primera diferencia en el byte {corte}: esperado {:?} · obtenido {:?}",
                &esperado[corte..(corte + 30).min(esperado.len())],
                &vuelta[corte..(corte + 30).min(vuelta.len())]
            );
        }
    }
    Ok(0)
}
