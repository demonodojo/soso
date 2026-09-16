//! Pruebas de los fixtures de referencia y su comparador (ficha T03).
//!
//! Los fixtures los genera una herramienta externa offline
//! (`tools/tokenizer-ref`), así que lo que se comprueba aquí es que **el
//! proyecto los lee bien**: que sus hashes cuadran, que sus IDs caben en el
//! vocabulario declarado y que la comparación clasifica lo que encuentra.
//!
//! La comparación contra el modelo real solo corre si el `.som` está en
//! `target/` —son 2,2 GB que no viajan en Git—. Cuando está, no se afirma un
//! resultado concreto: T53 va a cambiarlo. Se exige que cada divergencia quede
//! clasificada y que ningún ID de la referencia se salga del vocabulario.

use std::path::{Path, PathBuf};

use soso_improve::sistema::Host;
use soso_improve_core::entorno::Archivos;
use soso_improve_core::referencia::{self, Tokeniza};

fn raiz_repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

fn fixtures() -> String {
    raiz_repo()
        .join("tests/self-improvement/reference")
        .to_string_lossy()
        .into_owned()
}

const ESPERADOS: &[&str] = &[
    "simple",
    "system",
    "historial",
    "herramienta-esquema",
    "herramienta-resultado",
];

#[test]
fn estan_las_cinco_conversaciones_que_pide_la_ficha() {
    let indice = referencia::leer_indice(&Host, &fixtures()).unwrap();
    let nombres: Vec<&str> = indice.fixtures.iter().map(|f| f.nombre.as_str()).collect();
    for esperado in ESPERADOS {
        assert!(nombres.contains(esperado), "falta el fixture {esperado}: {nombres:?}");
    }
    assert_eq!(indice.fixtures.len(), ESPERADOS.len());
    assert_eq!(indice.schema_version, 1);
}

#[test]
fn la_referencia_declara_de_donde_sale() {
    let indice = referencia::leer_indice(&Host, &fixtures()).unwrap();
    // Sin revisión exacta, el fixture no vale como referencia: el mismo
    // repositorio puede cambiar plantilla o tokenizer.
    assert_eq!(indice.revision.len(), 40, "la revisión debe ser un commit completo");
    assert!(indice.modelo.contains('/'), "modelo: {}", indice.modelo);
    assert!(indice.vocab_size > 1000);

    for entrada in &indice.fixtures {
        let f = referencia::leer_fixture(&Host, &fixtures(), &entrada.archivo).unwrap();
        assert_eq!(f.revision, indice.revision, "{}", entrada.nombre);
        assert_eq!(f.modelo, indice.modelo, "{}", entrada.nombre);
    }
}

/// `leer_fixture` recalcula los hashes: un fixture editado a mano se detecta.
#[test]
fn los_hashes_del_fixture_cuadran_con_su_contenido() {
    let indice = referencia::leer_indice(&Host, &fixtures()).unwrap();
    for entrada in &indice.fixtures {
        let f = referencia::leer_fixture(&Host, &fixtures(), &entrada.archivo)
            .unwrap_or_else(|e| panic!("{}: {e}", entrada.archivo));
        assert_eq!(f.ids.len(), entrada.tokens, "{}", entrada.nombre);
        assert_eq!(f.texto_sha256, entrada.texto_sha256);
        assert_eq!(f.ids_sha256, entrada.ids_sha256);
        assert!(!f.texto.is_empty());
        assert!(!f.ids.is_empty());
    }
}

#[test]
fn un_fixture_manipulado_se_rechaza() {
    let tmp = std::env::temp_dir().join(format!("t03-fixture-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap();
    let destino = tmp.to_string_lossy().into_owned();
    let mut f = referencia::leer_fixture(&Host, &fixtures(), "simple.json").unwrap();
    f.texto.push_str(" y una coma de más");
    let mut host = Host;
    host.escribir(
        &format!("{destino}/simple.json"),
        serde_json::to_string(&f).unwrap().as_bytes(),
        0o644,
    )
    .unwrap();
    let error = referencia::leer_fixture(&Host, &destino, "simple.json").unwrap_err();
    assert!(format!("{error}").contains("texto_sha256"), "{error}");
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn todos_los_ids_caben_en_el_vocabulario_declarado() {
    let indice = referencia::leer_indice(&Host, &fixtures()).unwrap();
    for entrada in &indice.fixtures {
        let f = referencia::leer_fixture(&Host, &fixtures(), &entrada.archivo).unwrap();
        for id in &f.ids {
            assert!(
                *id < indice.vocab_size,
                "{}: id {id} fuera de vocab_size {}",
                entrada.nombre,
                indice.vocab_size
            );
        }
    }
}

/// Las conversaciones con herramientas tienen que traer de verdad el formato de
/// la familia; si no, el fixture no prueba nada sobre llamadas.
#[test]
fn los_fixtures_de_herramienta_traen_el_formato_de_la_familia() {
    let esquema = referencia::leer_fixture(&Host, &fixtures(), "herramienta-esquema.json").unwrap();
    assert!(esquema.texto.contains("<tools>"), "falta la declaración de herramientas");
    assert!(esquema.texto.contains("<tool_call>"), "falta el formato de llamada");

    let resultado =
        referencia::leer_fixture(&Host, &fixtures(), "herramienta-resultado.json").unwrap();
    assert!(resultado.texto.contains("<tool_response>"), "falta el turno de resultado");
    // La plantilla de esta familia inserta un turno de sistema aunque no se le
    // dé ninguno: conviene que el fixture lo demuestre.
    let simple = referencia::leer_fixture(&Host, &fixtures(), "simple.json").unwrap();
    assert!(simple.texto.starts_with("<|im_start|>system"), "{:?}", &simple.texto[..40]);
    assert!(simple.texto.ends_with("<|im_start|>assistant\n"));
}

// --- comparación contra el modelo real (solo si está convertido) -------------

struct TokenizerSoso(soso_llm_core::tokenizer::Tokenizer);

impl Tokeniza for TokenizerSoso {
    fn encode(&self, texto: &str) -> Vec<u32> {
        self.0.encode(texto)
    }
    fn vocab_size(&self) -> u32 {
        self.0.vocab_len() as u32
    }
    fn pieza(&self, id: u32) -> Option<String> {
        let mut bytes = Vec::new();
        self.0.token_bytes(id, &mut bytes);
        (!bytes.is_empty()).then(|| String::from_utf8_lossy(&bytes).into_owned())
    }
}

fn modelo_convertido() -> Option<TokenizerSoso> {
    let ruta = raiz_repo().join("target/qwen2.5-coder-3b-model/tokenizer.som");
    let datos = std::fs::read(ruta).ok()?;
    let tok = soso_llm_core::tokenizer::Tokenizer::parse(&datos).ok()?;
    tok.tiene_vocabulario().then_some(TokenizerSoso(tok))
}

#[test]
fn la_comparacion_clasifica_lo_que_encuentra() {
    let Some(tokenizer) = modelo_convertido() else {
        eprintln!("sin target/qwen2.5-coder-3b-model: se omite la comparación real");
        return;
    };
    let informe = referencia::comparar_todos(&Host, &fixtures(), &tokenizer).unwrap();
    assert_eq!(informe.comparaciones.len(), ESPERADOS.len());
    assert_eq!(informe.iguales + informe.divergentes, ESPERADOS.len());

    for c in &informe.comparaciones {
        // No se afirma el resultado: T53 lo va a cambiar. Se afirma que, haya
        // lo que haya, quede dicho qué es y dónde.
        match (&c.divergencia, c.iguales) {
            (None, true) => {}
            (Some(d), false) => {
                assert!(d.posicion <= c.tokens_referencia, "{}", c.fixture);
                assert!(!d.explicacion.is_empty(), "{}", c.fixture);
            }
            otro => panic!("{}: divergencia e 'iguales' no concuerdan: {otro:?}", c.fixture),
        }
        assert!(
            c.fuera_de_vocabulario.is_empty(),
            "{}: la referencia usa {} id(s) que soso no tiene",
            c.fixture,
            c.fuera_de_vocabulario.len()
        );
    }
    // El informe sale con 0 o 2, nunca con otra cosa.
    assert!(informe.codigo() == 0 || informe.codigo() == 2);
}
