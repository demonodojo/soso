//! Llamadas a herramientas restringidas por gramática (idea de «structured outputs» de DeepInfer).
//!
//! Sin modelo: un «modelo» de logits sintéticos que **quiere** escribir una llamada rota (nombre que no existe,
//! JSON sin cerrar, clave desconocida, falta una requerida) y el muestreador restringido, que sólo puede elegir
//! tokens que el autómata de la familia acepta. Lo que sale se pasa por el parser de verdad.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::json;
use soso_llm_core::conversation::gramatica::restriccion_de_herramientas;
use soso_llm_core::conversation::{
    parse_assistant_output, parse_assistant_output_familia, ChatError, ChatInput, Message, ToolChoice,
    ToolDefinition,
};
use soso_llm_core::restriccion::{Restriccion, TablaBytes};
use soso_llm_core::sample::Sampler;
use soso_llm_core::tokenizer::Tokenizer;

// ------------------------------------------------------------------------------------------------ vocabulario

/// Piezas del vocabulario sintético: un token por byte ASCII imprimible (más `\n`), y las piezas de marcador y
/// de sintaxis que usan las familias. El último id es un fin de secuencia que no emite bytes.
fn piezas() -> Vec<Vec<u8>> {
    let mut v: Vec<Vec<u8>> = (0x20u8..0x7f).map(|b| vec![b]).collect();
    v.push(b"\n".to_vec());
    v.push(b"\t".to_vec());
    for p in [
        "<tool_call>", "</tool_call>", "<|tool_call>", "<tool_call|>", "<|\"|>", "<think>", "</think>",
        "<|channel>", "<channel|>", "<function=", "</function>", "<parameter=", "</parameter>", "call:",
        "\"name\"", "\"arguments\"", "{\"", "\"}", "true", "false", "null", "name", "arguments", "ruta",
        "comando", "leer_fichero", "ejecutar", "ping",
    ] {
        v.push(p.as_bytes().to_vec());
    }
    v.push(Vec::new()); // eos
    v
}

fn tabla() -> Arc<TablaBytes> {
    let p = piezas();
    Arc::new(TablaBytes::desde_piezas(p.iter().map(|x| x.as_slice())))
}

fn eos() -> u32 {
    (piezas().len() - 1) as u32
}

fn bytes_de(tabla: &TablaBytes, ids: &[u32]) -> Vec<u8> {
    ids.iter().flat_map(|&t| tabla.bytes(t).to_vec()).collect()
}

/// Tokeniza `texto` por la pieza más larga que encaje (lo que «querría escribir» el modelo).
fn tokenizar(texto: &str) -> Vec<u32> {
    let p = piezas();
    let b = texto.as_bytes();
    let mut i = 0;
    let mut out = Vec::new();
    while i < b.len() {
        let (id, len) = p
            .iter()
            .enumerate()
            .filter(|(_, x)| !x.is_empty() && b[i..].starts_with(x))
            .map(|(id, x)| (id as u32, x.len()))
            .max_by_key(|&(_, l)| l)
            .unwrap_or_else(|| panic!("byte sin pieza en {texto:?}"));
        out.push(id);
        i += len;
    }
    out
}

// ------------------------------------------------------------------------------------------------ el «modelo»

struct Ruido(u64);

impl Ruido {
    fn siguiente(&mut self) -> f32 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        (x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 40) as f32 / (1u64 << 24) as f32
    }
}

fn lcp(a: &[u8], b: &[u8]) -> usize {
    a.iter().zip(b).take_while(|(x, y)| x == y).count()
}

/// Logits que empujan hacia escribir `querido` (por lo que se parezca cada token a lo que falta), con ruido
/// para desempatar. Si se desvía de lo querido (la gramática se lo impide) sigue pidiendo lo mismo.
///
/// Si la gramática lo desvía de lo querido y se alarga demasiado, acaba prefiriendo los cierres (comillas,
/// llaves, etiquetas de cierre): un modelo real también termina lo que empieza. Sin esto el «modelo» podría
/// quedarse repitiendo la misma letra dentro de una cadena sin fin, que no prueba nada de la gramática.
fn logits(tabla: &TablaBytes, querido: &[u8], escrito: &[u8], ruido: &mut Ruido) -> Vec<f32> {
    // Cierres de cadena (preferidos) y de estructura.
    const CIERRES_CADENA: [&[u8]; 3] = [b"\"", b"<|\"|>", b"</parameter>"];
    const CIERRES: [&[u8]; 8] = [b":", b">", b"}", b"\"}", b"</tool_call>", b"<tool_call|>", b"</function>", b"]"];
    let p = lcp(escrito, querido);
    let resto = &querido[p..];
    let cansado = escrito.len() > 200;
    (0..tabla.len() as u32)
        .map(|t| {
            let b = tabla.bytes(t);
            let afinidad = if b.is_empty() { 0 } else { lcp(b, resto) };
            let cierre = if !cansado {
                0.0
            } else if CIERRES_CADENA.contains(&b) {
                60.0
            } else if CIERRES.contains(&b) {
                50.0
            } else {
                0.0
            };
            10.0 * afinidad as f32 + cierre + ruido.siguiente()
        })
        .collect()
}

/// Genera con el muestreador restringido hasta que la llamada se abre y se cierra (o se agotan los pasos).
/// Devuelve el texto generado y si la restricción llegó a cerrar la llamada.
fn generar(
    mut r: Box<dyn Restriccion>,
    tabla: &TablaBytes,
    querido: &str,
    semilla: u64,
    temperatura: f32,
) -> (String, bool) {
    let mut ruido = Ruido(semilla | 1);
    let mut sampler = Sampler::new(temperatura, 0.95, semilla);
    let mut ids: Vec<u32> = Vec::new();
    let mut estuvo_activa = false;
    for _ in 0..4000 {
        let l = logits(tabla, querido.as_bytes(), &bytes_de(tabla, &ids), &mut ruido);
        let t = sampler.sample_restringido(&l, r.as_mut());
        r.avanza(t);
        ids.push(t);
        if r.activa() {
            estuvo_activa = true;
        } else if estuvo_activa {
            return (String::from_utf8_lossy(&bytes_de(tabla, &ids)).into_owned(), true);
        }
    }
    (String::from_utf8_lossy(&bytes_de(tabla, &ids)).into_owned(), false)
}

// ------------------------------------------------------------------------------------------------ herramientas

fn herramientas() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition::nueva(
            "leer_fichero",
            Some(String::from("Lee un fichero")),
            json!({
                "type": "object",
                "properties": {
                    "ruta": {"type": "string"},
                    "limite": {"type": "integer"},
                    "literal": {"type": "boolean"}
                },
                "required": ["ruta"]
            }),
        ),
        ToolDefinition::nueva(
            "ejecutar",
            None,
            json!({
                "type": "object",
                "properties": {
                    "comando": {"type": "string"},
                    "entorno": {"type": "object"},
                    "args": {"type": "array"}
                },
                "required": ["comando"]
            }),
        ),
        ToolDefinition::nueva(
            "ping",
            None,
            json!({"type": "object", "properties": {"eco": {"type": "string"}}}),
        ),
    ]
}

fn entrada(choice: ToolChoice) -> ChatInput {
    ChatInput::nuevo(vec![Message::user("haz algo")]).con_herramientas(herramientas(), choice)
}

fn gramatica(familia: &str, choice: ToolChoice) -> Box<dyn Restriccion> {
    restriccion_de_herramientas(familia, &entrada(choice), tabla(), false).expect("hay restricción")
}

// Llamadas rotas que el «modelo» querría escribir, por familia: (descripción, texto).
fn rotas(familia: &str) -> Vec<(&'static str, &'static str)> {
    match familia {
        "qwen2" | "mellum" => vec![
            ("nombre inexistente", "<tool_call>\n{\"name\": \"borrar_todo\", \"arguments\": {\"ruta\": \"/\"}}\n</tool_call>"),
            ("JSON sin cerrar", "<tool_call>\n{\"name\": \"leer_fichero\", \"arguments\": {\"ruta\": \"a.rs\""),
            ("clave desconocida", "<tool_call>\n{\"name\": \"leer_fichero\", \"arguments\": {\"path\": \"a.rs\"}}\n</tool_call>"),
            ("falta la requerida", "<tool_call>\n{\"name\": \"leer_fichero\", \"arguments\": {}}\n</tool_call>"),
            ("tipo equivocado", "<tool_call>\n{\"name\": \"leer_fichero\", \"arguments\": {\"ruta\": 5, \"limite\": \"mucho\"}}\n</tool_call>"),
        ],
        "gemma4" | "gemma4-pensar" => vec![
            ("nombre inexistente", "<|tool_call>call:borrar_todo{ruta:<|\"|>/<|\"|>}<tool_call|>"),
            ("sin cerrar", "<|tool_call>call:leer_fichero{ruta:<|\"|>a.rs<|\"|>"),
            ("clave desconocida", "<|tool_call>call:leer_fichero{path:<|\"|>a.rs<|\"|>}<tool_call|>"),
            ("falta la requerida", "<|tool_call>call:leer_fichero{}<tool_call|>"),
            ("tipo equivocado", "<|tool_call>call:leer_fichero{ruta:5,limite:<|\"|>mucho<|\"|>}<tool_call|>"),
        ],
        _ => vec![
            ("nombre inexistente", "<tool_call>\n<function=borrar_todo>\n<parameter=ruta>\n/\n</parameter>\n</function>\n</tool_call>"),
            ("sin cerrar", "<tool_call>\n<function=leer_fichero>\n<parameter=ruta>\na.rs\n</parameter>"),
            ("clave desconocida", "<tool_call>\n<function=leer_fichero>\n<parameter=path>\na.rs\n</parameter>\n</function>\n</tool_call>"),
            ("falta la requerida", "<tool_call>\n<function=leer_fichero>\n</function>\n</tool_call>"),
            ("antes del cierre", "<tool_call>\n<function=leer_fichero>\n<parameter=ruta>\na.rs"),
        ],
    }
}

const FAMILIAS: [&str; 4] = ["qwen2", "mellum", "gemma4", "qwen3coder"];

/// Lo que tiene que cumplir una llamada generada: la lee el parser, con su validación contra el esquema.
fn comprobar_llamada(familia: &str, texto: &str, motivo: &str) {
    let e = entrada(ToolChoice::Auto);
    let turno = parse_assistant_output_familia(&e, texto, 1, familia)
        .unwrap_or_else(|err| panic!("[{familia}] {motivo}: no se parsea ({err}): {texto:?}"));
    let llamada = turno.tool_call.unwrap_or_else(|| panic!("[{familia}] {motivo}: sin llamada: {texto:?}"));
    // Y la versión estricta, que valida los argumentos contra el esquema de la herramienta.
    let estricto = parse_assistant_output(&e, texto, 1)
        .unwrap_or_else(|err| panic!("[{familia}] {motivo}: no valida contra el esquema ({err}): {texto:?}"));
    assert_eq!(estricto.tool_call.as_ref().map(|c| &c.name), Some(&llamada.name));
    let def = e.herramienta(&llamada.name).expect("herramienta declarada");
    let args: serde_json::Value = serde_json::from_str(&llamada.arguments).expect("arguments es JSON");
    let obj = args.as_object().expect("arguments es un objeto");
    let props = def.parameters["properties"].as_object().unwrap();
    for k in obj.keys() {
        assert!(props.is_empty() || props.contains_key(k), "[{familia}] {motivo}: clave {k:?} fuera del esquema: {texto:?}");
    }
    for r in def.parameters["required"].as_array().into_iter().flatten() {
        assert!(obj.contains_key(r.as_str().unwrap()), "[{familia}] {motivo}: falta la requerida {r}: {texto:?}");
    }
}

// ------------------------------------------------------------------------------------------------ pruebas

#[test]
fn lo_restringido_sale_parseable_en_las_cuatro_familias() {
    let t = tabla();
    for familia in FAMILIAS {
        for (motivo, querido) in rotas(familia) {
            for semilla in 1u64..=25 {
                let (texto, cerrada) = generar(gramatica(familia, ToolChoice::Auto), &t, querido, semilla, 0.0);
                assert!(cerrada, "[{familia}] {motivo}/{semilla}: la llamada no se cerró: {texto:?}");
                comprobar_llamada(familia, &texto, motivo);
            }
        }
    }
}

#[test]
fn con_temperatura_tambien() {
    let t = tabla();
    for familia in FAMILIAS {
        for (motivo, querido) in rotas(familia) {
            for semilla in 1u64..=15 {
                let (texto, cerrada) = generar(gramatica(familia, ToolChoice::Auto), &t, querido, semilla, 0.7);
                assert!(cerrada, "[{familia}] {motivo}/{semilla}: la llamada no se cerró: {texto:?}");
                comprobar_llamada(familia, &texto, motivo);
            }
        }
    }
}

#[test]
fn sin_restriccion_esas_mismas_llamadas_salen_rotas() {
    // Control: sin el autómata el «modelo» escribe lo que quiere, y eso no pasa el parser. Si esto fallara, las
    // pruebas de arriba no demostrarían nada.
    let t = tabla();
    for familia in ["qwen2", "gemma4", "qwen3coder"] {
        let (motivo, querido) = rotas(familia)[0];
        let ids = tokenizar(querido);
        let texto = String::from_utf8_lossy(&bytes_de(&t, &ids)).into_owned();
        assert_eq!(texto, querido);
        let r = parse_assistant_output(&entrada(ToolChoice::Auto), &texto, 1);
        assert!(
            matches!(r, Err(ChatError::HerramientaDesconocida { .. })),
            "[{familia}] {motivo}: esperaba herramienta desconocida y salió {r:?}"
        );
    }
}

#[test]
fn el_nombre_es_siempre_el_de_una_herramienta_declarada() {
    let t = tabla();
    for familia in FAMILIAS {
        let (_, querido) = rotas(familia)[0];
        for semilla in 1..=20u64 {
            let (texto, _) = generar(gramatica(familia, ToolChoice::Auto), &t, querido, semilla, 0.0);
            let turno = parse_assistant_output_familia(&entrada(ToolChoice::Auto), &texto, 1, familia).unwrap();
            let nombre = turno.tool_call.unwrap().name;
            assert!(["leer_fichero", "ejecutar", "ping"].contains(&nombre.as_str()), "[{familia}] {nombre}");
        }
    }
}

#[test]
fn con_una_herramienta_concreta_solo_sale_esa() {
    let t = tabla();
    for familia in FAMILIAS {
        let (_, querido) = rotas(familia)[0];
        for semilla in 1..=10u64 {
            let (texto, _) =
                generar(gramatica(familia, ToolChoice::Named(String::from("ejecutar"))), &t, querido, semilla, 0.0);
            let turno = parse_assistant_output_familia(&entrada(ToolChoice::Auto), &texto, 1, familia)
                .unwrap_or_else(|e| panic!("[{familia}/{semilla}] {e:?}: {texto:?}"));
            assert_eq!(turno.tool_call.unwrap().name, "ejecutar", "[{familia}]");
        }
    }
}

#[test]
fn es_perezosa_no_restringe_hasta_abrir_la_llamada() {
    let t = tabla();
    for (familia, abre, cierra) in [
        ("qwen2", "<tool_call>", "</tool_call>"),
        ("gemma4", "<|tool_call>", "<tool_call|>"),
        ("qwen3coder", "<tool_call>", "</tool_call>"),
    ] {
        let mut r = gramatica(familia, ToolChoice::Required);
        assert!(!r.activa());
        // Texto libre: todo cabe y nada se activa.
        for id in tokenizar("Voy a leer el fichero: {x} </y> hola!") {
            assert!(r.permite(id));
            r.avanza(id);
        }
        assert!(!r.activa());
        // El marcador la activa; a partir de ahí no cabe cualquier cosa (ni un fin de secuencia).
        for id in tokenizar(abre) {
            r.avanza(id);
        }
        assert!(r.activa(), "{familia}");
        assert!(!r.permite(eos()), "{familia}: el fin de secuencia no cabe a mitad de una llamada");
        assert!(!r.permite(tokenizar("z")[0]), "{familia}: una letra suelta no abre nada");
        // Una llamada válida la cierra y la suelta.
        let valida = match familia {
            "qwen2" => "\n{\"name\": \"ping\", \"arguments\": {}}\n",
            "gemma4" => "call:ping{}",
            _ => "\n<function=ping>\n</function>\n",
        };
        for id in tokenizar(valida).into_iter().chain(tokenizar(cierra)) {
            assert!(r.permite(id), "{familia}: {:?} debería caber", t.bytes(id));
            r.avanza(id);
        }
        assert!(!r.activa(), "{familia}: cerrada la llamada se suelta");
        assert!(r.permite(eos()));
    }
}

#[test]
fn un_marcador_dentro_del_razonamiento_no_abre_la_llamada() {
    for (familia, abre_p, cierra_p, abre) in [
        ("qwen2", "<think>", "</think>", "<tool_call>"),
        ("gemma4", "<|channel>", "<channel|>", "<|tool_call>"),
    ] {
        let mut r = gramatica(familia, ToolChoice::Auto);
        for id in tokenizar(abre_p).into_iter().chain(tokenizar(abre)) {
            r.avanza(id);
        }
        assert!(!r.activa(), "{familia}: dentro del pensamiento no se activa");
        for id in tokenizar(cierra_p).into_iter().chain(tokenizar(abre)) {
            r.avanza(id);
        }
        assert!(r.activa(), "{familia}: cerrado el pensamiento, sí");
    }
    // Si el aviso de generación ya dejó el razonamiento abierto.
    let mut r = restriccion_de_herramientas("qwen2", &entrada(ToolChoice::Auto), tabla(), true).unwrap();
    for id in tokenizar("<tool_call>") {
        r.avanza(id);
    }
    assert!(!r.activa());
    for id in tokenizar("</think>").into_iter().chain(tokenizar("<tool_call>")) {
        r.avanza(id);
    }
    assert!(r.activa());
}

#[test]
fn sin_salida_se_suelta_y_devuelve_lo_de_siempre() {
    // La única herramienta se llama con una letra que este vocabulario no sabe escribir.
    let tools = vec![ToolDefinition::nueva("año", None, json!({"type": "object", "properties": {}}))];
    let e = ChatInput::nuevo(vec![Message::user("x")]).con_herramientas(tools, ToolChoice::Auto);
    let t = tabla();
    let mut r = restriccion_de_herramientas("qwen2", &e, t.clone(), false).unwrap();
    for id in tokenizar("<tool_call>{\"name\": \"") {
        r.avanza(id);
    }
    assert!(r.activa());
    // El nombre empieza por `a` (cabe) y sigue por `ñ`, que no existe en el vocabulario: tras `a`, nada cabe.
    r.avanza(tokenizar("a")[0]);
    let mut sampler = Sampler::greedy();
    let l: Vec<f32> = (0..t.len()).map(|i| i as f32).collect();
    let libre = {
        let mut s = Sampler::greedy();
        s.sample(&l)
    };
    let elegido = sampler.sample_restringido(&l, r.as_mut());
    assert_eq!(elegido, libre, "sin salida devuelve el argmax sin restringir");
    assert!(!r.activa(), "y se suelta");
}

#[test]
fn herramienta_con_unica_clave_no_se_repite() {
    // Con una sola clave, tras escribirla ya no se puede abrir otra: sólo `}`.
    let t = tabla();
    let tools = vec![ToolDefinition::nueva(
        "uno",
        None,
        json!({"type": "object", "properties": {"a": {"type": "string"}}, "required": ["a"]}),
    )];
    let e = ChatInput::nuevo(vec![Message::user("x")]).con_herramientas(tools, ToolChoice::Auto);
    let mut r = restriccion_de_herramientas("qwen2", &e, t.clone(), false).unwrap();
    for id in tokenizar("<tool_call>{\"name\": \"uno\", \"arguments\": {\"a\": \"v\"") {
        assert!(r.permite(id), "{:?}", t.bytes(id));
        r.avanza(id);
    }
    assert!(!r.permite(tokenizar(",")[0]), "no quedan claves por escribir");
    assert!(r.permite(tokenizar("}")[0]));
}

#[test]
fn las_cadenas_json_no_admiten_saltos_de_linea_sin_escapar() {
    let t = tabla();
    let mut r = gramatica("qwen2", ToolChoice::Auto);
    for id in tokenizar("<tool_call>{\"name\": \"leer_fichero\", \"arguments\": {\"ruta\": \"a") {
        r.avanza(id);
    }
    assert!(!r.permite(tokenizar("\n")[0]), "un salto crudo dentro de una cadena no es JSON");
    let barra = tokenizar("\\")[0];
    assert!(r.permite(barra));
    r.avanza(barra);
    assert!(r.permite(tokenizar("n")[0]), "pero `\\n` sí");
    assert!(!r.permite(tokenizar("q")[0]), "`\\q` no es un escape");
    let _ = t;
}

#[test]
fn sin_properties_las_claves_son_libres() {
    // Una herramienta que no declara propiedades acepta cualquier clave; lo que escribe un modelo bien portado
    // para ella cabe entero.
    let t = tabla();
    let tools = vec![ToolDefinition::nueva("libre", None, json!({"type": "object"}))];
    let e = ChatInput::nuevo(vec![Message::user("x")]).con_herramientas(tools, ToolChoice::Auto);
    for (familia, llamada) in [
        ("qwen2", "<tool_call>\n{\"name\": \"libre\", \"arguments\": {\"x\": 1, \"y\": \"z\", \"x\": [true, null]}}\n</tool_call>"),
        ("gemma4", "<|tool_call>call:libre{x:1,y:<|\"|>z<|\"|>,w:[true,null]}<tool_call|>"),
        ("qwen3coder", "<tool_call>\n<function=libre>\n<parameter=x>\n1\n</parameter>\n<parameter=y>\nz\n</parameter>\n</function>\n</tool_call>"),
    ] {
        let mut r = restriccion_de_herramientas(familia, &e, t.clone(), false).unwrap();
        for id in tokenizar(llamada) {
            assert!(r.permite(id), "[{familia}] {:?} rechazado en {llamada:?}", String::from_utf8_lossy(t.bytes(id)));
            r.avanza(id);
        }
        assert!(!r.activa(), "[{familia}] la llamada completa cierra la gramática");
    }
}

#[test]
fn en_qwen3coder_los_valores_no_textuales_se_escriben_como_json() {
    let t = tabla();
    let mut r = gramatica("qwen3coder", ToolChoice::Auto);
    for id in tokenizar("<tool_call>\n<function=leer_fichero>\n<parameter=limite>\n") {
        r.avanza(id);
    }
    // `limite` es un entero: una letra no cabe, un dígito sí.
    assert!(!r.permite(tokenizar("a")[0]));
    assert!(r.permite(tokenizar("7")[0]));
    for id in tokenizar("42\n</parameter>") {
        assert!(r.permite(id), "{:?}", String::from_utf8_lossy(t.bytes(id)));
        r.avanza(id);
    }
}

// ------------------------------------------------------------------------------------------------ vocabulario real

fn tokenizer_real() -> Tokenizer {
    let ruta: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/self-improvement/reference/tokenizer.som");
    let datos = std::fs::read(&ruta).unwrap_or_else(|e| panic!("falta {ruta:?}: {e}"));
    Tokenizer::parse(&datos).expect("tokenizer.som")
}

#[test]
fn la_tokenizacion_normal_de_una_llamada_valida_cabe_en_la_gramatica() {
    // Con el vocabulario de Qwen2.5 (BPE real), los tokens que produciría un modelo bien portado para una
    // llamada válida tienen que ser todos aceptables: si no, la gramática le estorbaría a quien acierta.
    let tok = tokenizer_real();
    let tabla = Arc::new(tok.tabla_bytes());
    let llamadas = [
        "<tool_call>\n{\"name\": \"leer_fichero\", \"arguments\": {\"ruta\": \"src/main.rs\", \"limite\": 200}}\n</tool_call>",
        "<tool_call>\n{\"name\": \"ejecutar\", \"arguments\": {\"comando\": \"cargo test -p x\\n\", \"entorno\": {\"A\": [1, 2.5e3, null, true]}}}\n</tool_call>",
        "<tool_call>\n{\"name\": \"ping\", \"arguments\": {\"eco\": \"cosa\"}}\n</tool_call>",
    ];
    for llamada in llamadas {
        let mut r = restriccion_de_herramientas("qwen2", &entrada(ToolChoice::Auto), tabla.clone(), false).unwrap();
        let ids = tok.encode_trozo(llamada, false, false);
        for (i, &id) in ids.iter().enumerate() {
            assert!(
                r.permite(id),
                "token {i} ({:?}) rechazado en {llamada:?}",
                String::from_utf8_lossy(tabla.bytes(id))
            );
            r.avanza(id);
        }
        assert!(!r.activa(), "la llamada completa cierra la gramática: {llamada:?}");
    }
}

#[test]
fn la_tabla_de_bytes_coincide_con_el_texto_de_los_marcadores() {
    let tok = tokenizer_real();
    let tabla = tok.tabla_bytes();
    let ids = tok.encode_trozo("<tool_call>", false, false);
    assert_eq!(ids.len(), 1, "el marcador es un token");
    assert_eq!(tabla.bytes(ids[0]), b"<tool_call>");
    assert_eq!(tok.bytes_token(ids[0]), b"<tool_call>");
    assert_eq!(tabla.len(), tok.vocab_len());
    assert!(tabla.bytes(u32::MAX - 1).is_empty());
}

#[test]
fn con_el_vocabulario_real_el_barrido_completo_funciona_y_es_rapido() {
    let tok = tokenizer_real();
    let tabla = Arc::new(tok.tabla_bytes());
    let mut r = restriccion_de_herramientas("qwen2", &entrada(ToolChoice::Auto), tabla.clone(), false).unwrap();
    for &id in &tok.encode_trozo("<tool_call>\n{\"name\": \"", false, false) {
        r.avanza(id);
    }
    // Logits que quieren otra cosa: el mejor permitido está fuera de los candidatos rápidos.
    let n = tok.vocab_len();
    let l: Vec<f32> = (0..n).map(|i| (i % 1000) as f32 * 0.01).collect();
    let inicio = std::time::Instant::now();
    let elegido = Sampler::greedy().sample_restringido(&l, r.as_mut());
    let dt = inicio.elapsed();
    let texto = String::from_utf8_lossy(tabla.bytes(elegido)).into_owned();
    assert!(
        ["leer_fichero", "ejecutar", "ping"].iter().any(|n| n.starts_with(texto.as_str()) && !texto.is_empty()),
        "{texto:?} no empieza ningún nombre"
    );
    // Cota generosa: el barrido de ~150 k tokens con el autómata cabe de sobra en un token de decode.
    assert!(dt < std::time::Duration::from_secs(2), "{dt:?}");
}
