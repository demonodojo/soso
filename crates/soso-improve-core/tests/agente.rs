//! Agente nativo (T35): herramientas sin silencios y bucle con modelo guionizado.

use std::collections::BTreeMap;

use serde_json::{json, Value};
use soso_improve_core::agente::{correr, ejecutar, Config, Fin, Modelo, Politica};
use soso_improve_core::entorno::{Archivos, Entrada, Orden, Procesos, Salida, Tipo};
use soso_improve_core::tiempo::RelojSimulado;
use soso_improve_core::{Error, Resultado};

#[derive(Default)]
struct Mem {
    f: BTreeMap<String, Vec<u8>>,
    ordenes: std::cell::RefCell<Vec<Vec<String>>>,
}

impl Mem {
    fn con(mut self, r: &str, t: &str) -> Self {
        self.f.insert(r.to_string(), t.as_bytes().to_vec());
        self
    }
}

impl Archivos for Mem {
    fn leer(&self, r: &str) -> Resultado<Vec<u8>> {
        self.f.get(r).cloned().ok_or_else(|| Error::entorno(format!("no existe {r}")))
    }
    fn escribir(&mut self, r: &str, d: &[u8], _m: u32) -> Resultado<()> {
        self.f.insert(r.to_string(), d.to_vec());
        Ok(())
    }
    fn existe(&self, r: &str) -> bool {
        self.f.contains_key(r) || self.f.keys().any(|k| k.starts_with(&format!("{r}/")))
    }
    fn listar(&self, dir: &str) -> Resultado<Vec<Entrada>> {
        let p = format!("{}/", dir.trim_end_matches('/'));
        let mut v: BTreeMap<String, Entrada> = BTreeMap::new();
        for (k, d) in &self.f {
            if let Some(resto) = k.strip_prefix(&p) {
                match resto.split_once('/') {
                    Some((h, _)) => {
                        v.insert(h.into(), Entrada { ruta: h.into(), tipo: Tipo::Directorio, bytes: 0, modo: 0o755 });
                    }
                    None => {
                        v.insert(resto.into(), Entrada { ruta: resto.into(), tipo: Tipo::Archivo, bytes: d.len() as u64, modo: 0o644 });
                    }
                }
            }
        }
        if v.is_empty() && dir.ends_with(".sesion") {
            return Ok(vec![]);
        }
        if v.is_empty() {
            return Err(Error::entorno("no existe"));
        }
        Ok(v.into_values().collect())
    }
    fn metadatos(&self, r: &str) -> Resultado<Entrada> {
        if let Some(d) = self.f.get(r) {
            return Ok(Entrada { ruta: r.into(), tipo: Tipo::Archivo, bytes: d.len() as u64, modo: 0o644 });
        }
        if self.existe(r) {
            return Ok(Entrada { ruta: r.into(), tipo: Tipo::Directorio, bytes: 0, modo: 0o755 });
        }
        Err(Error::entorno("no existe"))
    }
    fn crear_directorio(&mut self, _r: &str) -> Resultado<()> {
        Ok(())
    }
    fn borrar(&mut self, r: &str) -> Resultado<()> {
        self.f.remove(r);
        Ok(())
    }
}

impl Procesos for Mem {
    fn ejecutar(&self, o: &Orden) -> Resultado<Salida> {
        self.ordenes.borrow_mut().push(o.argv.clone());
        Ok(Salida { codigo: Some(0), stdout: b"test result: ok. 1 passed".to_vec(), stderr: vec![], motivo: None })
    }
}

const R: &str = "/w";

fn repo() -> Mem {
    Mem::default()
        .con("/w/src/lib.rs", "fn buscar(k: &str) {\n    if k == key {\n        hit();\n    }\n}\n")
        .con("/w/src/otro.rs", "fn otro() {}\n")
        .con("/w/README.md", "hola\n")
}

fn llama(e: &mut Mem, n: &str, a: Value) -> (String, bool) {
    let r = ejecutar(e, &Politica::nueva(R), n, &a);
    (r.texto, r.error)
}

#[test]
fn read_numera_lineas_y_lista_directorios() {
    let mut e = repo();
    let (t, err) = llama(&mut e, "read", json!({"filePath": "src/lib.rs"}));
    assert!(!err && t.contains("2:     if k == key {"), "{t}");
    let (t, _) = llama(&mut e, "read", json!({"filePath": "src"}));
    assert!(t.contains("lib.rs") && t.contains("otro.rs"), "{t}");
    let (t, err) = llama(&mut e, "read", json!({"filePath": "src/lib.rs", "offset": 99}));
    assert!(err && t.contains("pasa del final"), "{t}");
    let (t, err) = llama(&mut e, "read", json!({"filePath": "../etc/passwd"}));
    assert!(err && t.contains("sale del checkout"), "{t}");
    let (t, err) = llama(&mut e, "read", json!({"filePath": "/etc/passwd"}));
    assert!(err && t.contains("fuera del checkout"), "{t}");
    let (_, err) = llama(&mut e, "read", json!({"filePath": "/w/src/lib.rs"}));
    assert!(!err, "una ruta absoluta dentro de la raíz vale");
}

#[test]
fn grep_es_literal_y_no_calla() {
    let mut e = repo();
    let (t, err) = llama(&mut e, "grep", json!({"pattern": "k == key"}));
    assert!(!err && t.contains("src/lib.rs:2:") && t.contains("1 coincidencia"), "{t}");
    let (t, err) = llama(&mut e, "grep", json!({"pattern": "fn \\w+"}));
    assert!(err && t.contains("expresión regular"), "{t}");
    let (t, _) = llama(&mut e, "grep", json!({"pattern": "inexistente"}));
    assert!(t.contains("No hay coincidencias") && t.contains("3 archivo"), "{t}");
    let (t, _) = llama(&mut e, "grep", json!({"pattern": "fn", "include": "*.md"}));
    assert!(t.contains("No hay coincidencias") && t.contains("1 archivo"), "{t}");
    let (t, _) = llama(&mut e, "grep", json!({"pattern": "a|b"}));
    assert!(t.contains("alternativas"), "{t}");
}

#[test]
fn grep_v2_espacios_mayusculas_comodines_y_sugerencias() {
    let mut e = repo();
    let (t, _) = llama(&mut e, "grep", json!({"pattern": "if   k  ==  key"}));
    assert!(t.contains("src/lib.rs:2:"), "espacios: {t}");
    let (t, _) = llama(&mut e, "grep", json!({"pattern": "FN BUSCAR"}));
    assert!(t.contains("ignoran mayúsculas") && t.contains("src/lib.rs:1:"), "mayúsculas: {t}");
    let (t, _) = llama(&mut e, "grep", json!({"pattern": "fn .*buscar"}));
    assert!(t.contains("src/lib.rs:1:") && t.contains("interpretado"), "comodín: {t}");
    let (t, _) = llama(&mut e, "grep", json!({"pattern": "pub fn buscar"}));
    assert!(t.contains("Pero «buscar» sí aparece") || t.contains("src/lib.rs"), "palabras: {t}");
    let (t, _) = llama(&mut e, "grep", json!({"pattern": "buscar_clave"}));
    assert!(t.contains("No hay coincidencias") && t.contains("`buscar`"), "sugerencia: {t}");
}

#[test]
fn glob_encuentra_por_nombre_y_por_ruta() {
    let mut e = repo();
    let (t, _) = llama(&mut e, "glob", json!({"pattern": "*.rs"}));
    assert!(t.contains("src/lib.rs") && t.contains("src/otro.rs") && !t.contains("README"), "{t}");
    let (t, _) = llama(&mut e, "glob", json!({"pattern": "src/**/l*.rs"}));
    assert_eq!(t.trim(), "src/lib.rs");
    let (t, _) = llama(&mut e, "glob", json!({"pattern": "*.zzz"}));
    assert!(t.contains("Ningún archivo"), "{t}");
}

#[test]
fn reemplazar_ignora_espacios_exige_unicidad_y_escribe() {
    let mut e = repo();
    let (t, err) = llama(&mut e, "reemplazar_texto", json!({"filePath": "src/lib.rs", "buscar": "k==key", "nuevo": "k == *key"}));
    assert!(!err && t.contains("línea 2"), "{t}");
    assert!(String::from_utf8(e.f["/w/src/lib.rs"].clone()).unwrap().contains("if k == *key {"));
    let (t, err) = llama(&mut e, "reemplazar_texto", json!({"filePath": "src/lib.rs", "buscar": "zzz", "nuevo": "x"}));
    assert!(err && t.contains("No encuentro"), "{t}");
    let mut e2 = Mem::default().con("/w/a.rs", "x();\nx();\n");
    let (t, err) = llama(&mut e2, "reemplazar_texto", json!({"filePath": "a.rs", "buscar": "x();", "nuevo": "y();"}));
    assert!(err && t.contains("aparece 2 veces") && t.contains("línea 1") && t.contains("línea 2"), "{t}");
    assert_eq!(e2.f["/w/a.rs"], b"x();\nx();\n", "no se toca el archivo ante la duda");
}

#[test]
fn bash_rechaza_shell_y_lo_no_permitido() {
    let mut e = repo();
    for c in ["cargo test && rm -rf /", "cargo test | tee x", "cargo test > f", "cargo test $HOME", "RUSTFLAGS=x cargo test"] {
        let (t, err) = llama(&mut e, "bash", json!({"command": c}));
        assert!(err && (t.contains("sin shell") || t.contains("variables") || t.contains("sólo se admiten")), "{c}: {t}");
    }
    let (t, err) = llama(&mut e, "bash", json!({"command": "rm -rf src"}));
    assert!(err && t.contains("no está permitida"), "{t}");
    assert!(e.ordenes.borrow().is_empty(), "nada de lo rechazado llegó a ejecutarse");
    let (t, err) = llama(&mut e, "bash", json!({"command": "cargo test -p soso-http"}));
    assert!(!err && t.starts_with("exit code: 0") && t.contains("1 passed"), "{t}");
    assert_eq!(e.ordenes.borrow()[0], ["cargo", "test", "-p", "soso-http"]);
}

#[test]
fn herramienta_desconocida_y_argumentos_malos_son_respuesta() {
    let mut e = repo();
    let (t, err) = llama(&mut e, "edit", json!({}));
    assert!(err && t.contains("no existe"), "{t}");
    let (t, err) = llama(&mut e, "read", json!({}));
    assert!(err && t.contains("filePath"), "{t}");
}

struct Guion(Vec<Value>, Vec<String>);
impl Modelo for Guion {
    fn completar(&mut self, p: &str, _t: u64) -> Resultado<String> {
        self.1.push(p.to_string());
        if self.0.is_empty() {
            return Err(Error::entorno("sin más guion"));
        }
        Ok(json!({"choices": [{"message": self.0.remove(0)}]}).to_string())
    }
}

/// Configuración de los tests que no hablan del aviso «no has modificado nada»: sin él, un guion que acaba sin editar sigue acabando.
fn cfg_base() -> Config {
    let mut c = Config::nueva("m", "s");
    c.avisos_sin_editar = 0;
    c
}

fn llamada(n: &str, a: Value) -> Value {
    json!({"role": "assistant", "content": "", "tool_calls": [{"id": "c1", "type": "function", "function": {"name": n, "arguments": a.to_string()}}]})
}

#[test]
fn el_bucle_lee_edita_prueba_y_termina() {
    let mut e = repo();
    let mut m = Guion(
        vec![
            llamada("grep", json!({"pattern": "k == key"})),
            llamada("reemplazar_texto", json!({"filePath": "src/lib.rs", "buscar": "k == key", "nuevo": "k == *key"})),
            llamada("bash", json!({"command": "cargo test"})),
            json!({"role": "assistant", "content": "Listo: corregí la comparación."}),
        ],
        vec![],
    );
    let reloj = RelojSimulado::nuevo(0);
    let mut vistos = 0;
    let inf = correr(&mut e, &mut m, &reloj, &Politica::nueva(R), &{ let mut c = Config::nueva("m", "eres un agente"); c.avisos_sin_editar = 0; c }, "arregla", &mut |_, _| vistos += 1);
    assert_eq!(inf.fin, Fin::Terminado, "{:?}", inf.error);
    assert_eq!((inf.turnos, inf.llamadas, vistos), (4, 3, 4));
    assert_eq!(inf.respuesta_final, "Listo: corregí la comparación.");
    assert!(String::from_utf8(e.f["/w/src/lib.rs"].clone()).unwrap().contains("*key"));
    // El tercer turno ya lleva el resultado de la herramienta anterior.
    assert!(m.1[1].contains("tool_call_id") && m.1[1].contains("src/lib.rs:2:"));
    let ult: Value = serde_json::from_str(&m.1[3]).unwrap();
    assert_eq!(ult["temperature"], 0.0);
    assert_eq!(ult["stream"], false);
    assert_eq!(ult["tools"].as_array().unwrap().len(), 7);
}

#[test]
fn argumentos_ilegibles_se_devuelven_al_modelo() {
    let mut e = repo();
    let malo = json!({"role": "assistant", "content": "", "tool_calls": [{"id": "x", "function": {"name": "read", "arguments": "{no json"}}]});
    let mut m = Guion(vec![malo, json!({"role": "assistant", "content": "ok"})], vec![]);
    let inf = correr(&mut e, &mut m, &RelojSimulado::nuevo(0), &Politica::nueva(R), &cfg_base(), "t", &mut |_, _| {});
    assert_eq!(inf.fin, Fin::Terminado);
    assert!(m.1[1].contains("no son un objeto JSON"));
}

#[test]
fn tope_de_turnos_y_error_del_modelo() {
    let mut e = repo();
    let bucle: Vec<Value> = (0..5).map(|_| llamada("read", json!({"filePath": "README.md"}))).collect();
    let mut cfg = cfg_base();
    cfg.max_turnos = 3;
    let inf = correr(&mut e, &mut Guion(bucle, vec![]), &RelojSimulado::nuevo(0), &Politica::nueva(R), &cfg, "t", &mut |_, _| {});
    assert_eq!((inf.fin, inf.turnos), (Fin::SinTurnos, 3));
    let inf = correr(&mut e, &mut Guion(vec![], vec![]), &RelojSimulado::nuevo(0), &Politica::nueva(R), &cfg_base(), "t", &mut |_, _| {});
    assert_eq!(inf.fin, Fin::ErrorModelo);
    assert!(inf.error.unwrap().contains("sin más guion"));
}

#[test]
fn la_autoprueba_pasa_sobre_un_entorno_en_memoria() {
    let mut e = Mem::default();
    e.f.insert("/w/.keep".into(), vec![]);
    let r = soso_improve_core::agente::prueba::autoprueba(&mut e, &RelojSimulado::nuevo(0), "/w", &["cargo", "test"]).unwrap();
    let malos: Vec<_> = r.iter().filter(|(_, ok, _)| !ok).collect();
    assert!(malos.is_empty(), "{malos:?}");
    assert!(r.len() >= 11);
}

impl soso_improve_core::durable::Durable for Mem {
    fn crear_exclusivo(&mut self, r: &str, d: &[u8]) -> Resultado<()> {
        if self.f.contains_key(r) {
            return Err(Error::uso("ya existe"));
        }
        self.f.insert(r.to_string(), d.to_vec());
        Ok(())
    }
    fn sincronizar(&mut self, _r: &str) -> Resultado<()> {
        Ok(())
    }
}

#[test]
fn la_sesion_se_recupera_tras_morir_el_agente() {
    use soso_improve_core::agente::{correr_desde, sesion};
    let mut e = repo();
    // Primer agente: dos llamadas y se acaba el modelo (el proceso «muere»).
    let mut m = Guion(vec![llamada("read", json!({"filePath": "README.md"})), llamada("grep", json!({"pattern": "k == key"}))], vec![]);
    let inf = correr(&mut e, &mut m, &RelojSimulado::nuevo(0), &Politica::nueva(R), &cfg_base(), "t", &mut |h, ms| {
        sesion::guardar(h, "/w/.sesion", ms).unwrap();
    });
    assert_eq!(inf.fin, Fin::ErrorModelo);
    let previa = sesion::cargar(&e, "/w/.sesion").unwrap().expect("hay sesión");
    // 2 de arranque + 2 × (asistente + herramienta)
    assert_eq!(previa.len(), 6);
    // Otro proceso la recoge y termina sin repetir el trabajo ya hecho.
    let mut m2 = Guion(vec![json!({"role": "assistant", "content": "fin"})], vec![]);
    let inf = correr_desde(&mut e, &mut m2, &RelojSimulado::nuevo(0), &Politica::nueva(R), &cfg_base(), previa, &mut |h, ms| {
        sesion::guardar(h, "/w/.sesion", ms).unwrap();
    });
    assert_eq!((inf.fin, inf.turnos, inf.llamadas), (Fin::Terminado, 1, 0));
    assert!(m2.1[0].contains("src/lib.rs:2:"), "el modelo ve lo que ya se hizo");
    // Una sesión ya terminada no vuelve a llamar al modelo.
    let hecha = sesion::cargar(&e, "/w/.sesion").unwrap().unwrap();
    let mut m3 = Guion(vec![], vec![]);
    let inf = correr_desde(&mut e, &mut m3, &RelojSimulado::nuevo(0), &Politica::nueva(R), &cfg_base(), hecha, &mut |_, _| {});
    assert_eq!((inf.fin, inf.respuesta_final.as_str(), m3.1.len()), (Fin::Terminado, "fin", 0));
}

#[test]
fn una_llamada_sin_resultado_se_descarta_al_reanudar() {
    use soso_improve_core::agente::sesion::normalizar;
    let mut ms = vec![
        json!({"role": "system", "content": "s"}),
        json!({"role": "user", "content": "t"}),
        llamada("read", json!({"filePath": "a"})),
        json!({"role": "tool", "tool_call_id": "c", "content": "x"}),
        llamada("bash", json!({"command": "cargo test"})),
    ];
    normalizar(&mut ms);
    assert_eq!(ms.len(), 4, "la llamada sin resultado se quita, la completa se queda");
}

#[test]
fn la_herramienta_forja_existe_solo_si_se_configura_y_el_token_no_viaja_al_modelo() {
    use soso_improve_core::agente::{herramientas::definiciones, Forja};
    let sin = definiciones(&Politica::nueva(R));
    assert!(!sin.to_string().contains("\"forja\""));
    let mut e = repo();
    let r = ejecutar(&mut e, &Politica::nueva(R), "forja", &json!({"accion": "all"}));
    assert!(r.error && r.texto.contains("no existe"), "{}", r.texto);

    let mut pol = Politica::nueva(R);
    pol.forja = Some(Forja { argv: vec!["/bin/soso-forja".into()], host: "10.0.2.2".into(), token: "secreto-123".into(), plazo_s: 600 });
    let con = definiciones(&pol);
    assert!(con.to_string().contains("\"forja\"") && !con.to_string().contains("secreto-123"));
    let r = ejecutar(&mut e, &pol, "forja", &json!({"accion": "all"}));
    assert!(!r.error && r.texto.starts_with("exit code: 0"), "{}", r.texto);
    assert_eq!(e.ordenes.borrow()[0], ["/bin/soso-forja", "all", "--host", "10.0.2.2", "--token", "secreto-123"]);
    // Una acción inventada no llega a ejecutar nada.
    let n = e.ordenes.borrow().len();
    let r = ejecutar(&mut e, &pol, "forja", &json!({"accion": "apply; rm -rf /"}));
    assert!(r.error && r.texto.contains("no existe"));
    assert_eq!(e.ordenes.borrow().len(), n);
}

#[test]
fn un_no_existe_dice_que_hay_en_el_directorio_mas_cercano() {
    let mut e = repo();
    let (t, err) = llama(&mut e, "read", json!({"filePath": "src/soso/user/main.rs"}));
    assert!(err && t.contains("relativas a la raíz") && t.contains("lib.rs") && t.contains("otro.rs"), "{t}");
    let (t, err) = llama(&mut e, "reemplazar_texto", json!({"filePath": "nada/x.rs", "buscar": "a", "nuevo": "b"}));
    assert!(err && t.contains("src/") && t.contains("README.md"), "{t}");
}

#[test]
fn un_archivo_largo_avisa_de_que_no_se_recorra_por_tramos() {
    let largo: String = (1..=900).map(|i| format!("linea {i}\n")).collect();
    let mut e = repo().con("/w/src/largo.rs", &largo);
    let (t, err) = llama(&mut e, "read", json!({"filePath": "src/largo.rs"}));
    assert!(!err && t.contains("Líneas 1-300 de 900") && t.contains("usa `grep`") || t.contains("Llama a `grep`"), "{t}");
    let (t, _) = llama(&mut e, "read", json!({"filePath": "src/lib.rs"}));
    assert!(t.contains("Fin del archivo") && !t.contains("grep"), "{t}");
}

#[test]
fn un_429_ocupado_se_espera_y_se_reintenta_sin_contar_como_fallo_del_modelo() {
    use soso_improve_core::agente::http::{ModeloHttp, ESPERA_BUSY_MS};
    use soso_improve_core::tiempo::{Plazo, Reloj};
    use soso_improve_core::transporte::{Conector, Destino, TransporteSimulado};
    use std::cell::{Cell, RefCell};

    struct Conn {
        respuestas: RefCell<Vec<Vec<u8>>>,
        intentos: Cell<usize>,
    }
    impl Conector for Conn {
        type Enlace = TransporteSimulado;
        fn conectar(&self, _: &Destino, _: Plazo, _: &dyn Reloj) -> Resultado<TransporteSimulado> {
            self.intentos.set(self.intentos.get() + 1);
            let r = self.respuestas.borrow_mut().remove(0);
            Ok(TransporteSimulado::byte_a_byte(&r))
        }
    }
    thread_local! { static DORMIDO: Cell<u64> = Cell::new(0); }
    fn dormir(ms: u64) {
        DORMIDO.with(|d| d.set(d.get() + ms));
    }
    let busy = b"HTTP/1.1 429 Too Many Requests\r\nContent-Length: 2\r\n\r\n{}".to_vec();
    let ok = b"HTTP/1.1 200 OK\r\nContent-Length: 17\r\n\r\n{\"choices\":[]}  ".to_vec();
    let c = Conn { respuestas: RefCell::new(vec![busy.clone(), busy.clone(), ok]), intentos: Cell::new(0) };
    let reloj = RelojSimulado::nuevo(0);
    let mut m = ModeloHttp { conector: &c, reloj: &reloj, destino: Destino::local(1), plazo_conexion_ms: 1000, token: None, dormir };
    let r = m.completar("{}", 60_000).unwrap();
    assert!(r.contains("choices"));
    assert_eq!((c.intentos.get(), DORMIDO.with(|d| d.get())), (3, 2 * ESPERA_BUSY_MS), "dos 429 seguidos: dos esperas y un tercer intento bueno");
}

const RS: &str = "pub fn f(a: &str) -> bool {\n    let s = \"llave { en texto\";\n    // comentario con }\n    a == s\n}\n";

#[test]
fn una_edicion_que_desequilibra_llaves_no_se_escribe_y_dice_cuanto() {
    let mut e = repo().con("/w/src/b.rs", RS);
    let (t, err) = llama(&mut e, "reemplazar_texto", json!({"filePath": "src/b.rs", "buscar": "a == s", "nuevo": "a == s }"}));
    assert!(err && t.contains("No he escrito el cambio") && t.contains("llaves") && t.contains("1 cierre"), "{t}");
    assert_eq!(e.f["/w/src/b.rs"], RS.as_bytes(), "el archivo no cambia");
    // Cortar la cabecera a medias también se rechaza.
    let (t, err) = llama(&mut e, "reemplazar_texto", json!({"filePath": "src/b.rs", "buscar": "pub fn f(a: &str) -> bool {", "nuevo": "pub fn g() {}\npub fn f(a: &str) -> bool"}));
    assert!(err && t.contains("1 cierre"), "{t}");
    // Un cambio equilibrado pasa aunque haya llaves en cadenas y comentarios.
    let (t, err) = llama(&mut e, "reemplazar_texto", json!({"filePath": "src/b.rs", "buscar": "a == s", "nuevo": "a.eq_ignore_ascii_case(s)"}));
    assert!(!err, "{t}");
    // En archivos que no son Rust no se mira.
    let (_, err) = llama(&mut e, "reemplazar_texto", json!({"filePath": "README.md", "buscar": "hola", "nuevo": "hola {"}));
    assert!(!err);
}

#[test]
fn insertar_anade_bloques_completos_en_la_linea_pedida() {
    let mut e = repo().con("/w/src/b.rs", RS);
    let (t, err) = llama(&mut e, "insertar", json!({"filePath": "src/b.rs", "linea": 5, "texto": "pub fn nueva() -> u8 {\n    1\n}"}));
    assert!(!err && t.contains("3 línea"), "{t}");
    let nuevo = String::from_utf8(e.f["/w/src/b.rs"].clone()).unwrap();
    assert!(nuevo.ends_with("}\npub fn nueva() -> u8 {\n    1\n}\n"), "{nuevo:?}");
    let (_, err) = llama(&mut e, "insertar", json!({"filePath": "src/b.rs", "linea": 0, "texto": "// cabecera"}));
    assert!(!err);
    assert!(String::from_utf8(e.f["/w/src/b.rs"].clone()).unwrap().starts_with("// cabecera\npub fn f"));
    let (t, err) = llama(&mut e, "insertar", json!({"filePath": "src/b.rs", "linea": 2, "texto": "fn rota() {"}));
    assert!(err && t.contains("llaves"), "{t}");
    let (t, err) = llama(&mut e, "insertar", json!({"filePath": "src/b.rs", "linea": 99, "texto": "x"}));
    assert!(err && t.contains("línea(s)"), "{t}");
}

#[test]
fn el_modelo_que_acaba_a_medias_recibe_un_recordatorio_en_vez_de_darse_por_terminado() {
    // Cambio hecho y nunca comprobado → recordatorio → comprueba → termina.
    let edita = llamada("reemplazar_texto", json!({"filePath": "src/lib.rs", "buscar": "k == key", "nuevo": "k == *key"}));
    let mut e = repo();
    let mut m = Guion(vec![edita.clone(), json!({"role": "assistant", "content": "Continuar"}), llamada("bash", json!({"command": "cargo test"})), json!({"role": "assistant", "content": "hecho"})], vec![]);
    let inf = correr(&mut e, &mut m, &RelojSimulado::nuevo(0), &Politica::nueva(R), &cfg_base(), "t", &mut |_, _| {});
    assert_eq!((inf.fin, inf.empujones, inf.llamadas), (Fin::Terminado, 1, 2), "{:?}", inf.error);
    assert!(m.1[2].contains("Todavía no has comprobado"), "el recordatorio llega al modelo");
    // Respuesta vacía y llamada escrita como texto.
    let mut e = repo();
    let mut m = Guion(vec![json!({"role": "assistant", "content": ""}), json!({"role": "assistant", "content": "{\"name\": \"bash\", \"arguments\": {\"command\": \"x\"}}"}), json!({"role": "assistant", "content": "fin"})], vec![]);
    let inf = correr(&mut e, &mut m, &RelojSimulado::nuevo(0), &Politica::nueva(R), &cfg_base(), "t", &mut |_, _| {});
    assert_eq!((inf.fin, inf.empujones), (Fin::Terminado, 2));
    assert!(m.1[1].contains("estaba vacía") && m.1[2].contains("como texto"));
}

#[test]
fn una_comprobacion_fallida_empuja_a_corregir_y_los_recordatorios_tienen_tope() {
    struct Rojo(Mem);
    // bash siempre falla: el modelo se rinde tres veces; sólo se le empuja dos.
    impl Archivos for Rojo {
        fn leer(&self, r: &str) -> Resultado<Vec<u8>> { self.0.leer(r) }
        fn escribir(&mut self, r: &str, d: &[u8], m: u32) -> Resultado<()> { self.0.escribir(r, d, m) }
        fn existe(&self, r: &str) -> bool { self.0.existe(r) }
        fn listar(&self, r: &str) -> Resultado<Vec<Entrada>> { self.0.listar(r) }
        fn metadatos(&self, r: &str) -> Resultado<Entrada> { self.0.metadatos(r) }
        fn crear_directorio(&mut self, r: &str) -> Resultado<()> { self.0.crear_directorio(r) }
        fn borrar(&mut self, r: &str) -> Resultado<()> { self.0.borrar(r) }
    }
    impl Procesos for Rojo {
        fn ejecutar(&self, _: &Orden) -> Resultado<Salida> {
            Ok(Salida { codigo: Some(101), stdout: b"error[E0599]: algo".to_vec(), stderr: vec![], motivo: None })
        }
    }
    let mut e = Rojo(repo());
    let mut m = Guion(
        vec![llamada("bash", json!({"command": "cargo test"})), json!({"role": "assistant", "content": "no pude"}), json!({"role": "assistant", "content": "sigo sin poder"}), json!({"role": "assistant", "content": "me rindo"})],
        vec![],
    );
    let inf = correr(&mut e, &mut m, &RelojSimulado::nuevo(0), &Politica::nueva(R), &cfg_base(), "t", &mut |_, _| {});
    assert_eq!((inf.fin, inf.empujones), (Fin::Terminado, 2), "dos recordatorios y entonces se acepta que termine");
    assert!(m.1[2].contains("ha fallado"));
    assert_eq!(inf.respuesta_final, "me rindo");
}

#[test]
fn una_respuesta_mal_formada_rechazada_con_400_se_devuelve_al_modelo_con_el_motivo() {
    struct Mal400(Vec<Option<Value>>, Vec<String>);
    impl Modelo for Mal400 {
        fn completar(&mut self, p: &str, _t: u64) -> Resultado<String> {
            self.1.push(p.to_string());
            match self.0.remove(0) {
                None => Err(Error::entorno("el servidor contestó Some(400): {\"error\":{\"message\":\"varias llamadas o texto tras una llamada en un turno\",\"code\":\"invalid_request\"}}")),
                Some(m) => Ok(json!({"choices": [{"message": m}]}).to_string()),
            }
        }
    }
    let mut e = repo();
    let mut m = Mal400(vec![None, Some(llamada("read", json!({"filePath": "README.md"}))), Some(json!({"role": "assistant", "content": "fin"}))], vec![]);
    let inf = correr(&mut e, &mut m, &RelojSimulado::nuevo(0), &Politica::nueva(R), &cfg_base(), "t", &mut |_, _| {});
    assert_eq!((inf.fin, inf.formato, inf.llamadas), (Fin::Terminado, 1, 1), "{:?}", inf.error);
    assert!(m.1[1].contains("varias llamadas o texto tras una llamada") && m.1[1].contains("una sola llamada"), "el motivo del servidor llega al modelo");
    // Con el tope agotado es un error del modelo, no un bucle.
    let mut cfg = cfg_base();
    cfg.errores_de_formato = 2;
    let mut m = Mal400(vec![None, None, None, None], vec![]);
    let inf = correr(&mut repo(), &mut m, &RelojSimulado::nuevo(0), &Politica::nueva(R), &cfg, "t", &mut |_, _| {});
    assert_eq!((inf.fin, inf.formato), (Fin::ErrorModelo, 2));
}

#[test]
fn una_llamada_identica_que_ya_fallo_no_se_repite_veinte_veces() {
    let mut e = repo();
    let mala = || llamada("reemplazar_texto", json!({"filePath": "src/lib.rs", "buscar": "esto no existe en el archivo", "nuevo": "x"}));
    let mut m = Guion(vec![mala(), mala(), mala(), mala(), json!({"role": "assistant", "content": "me rindo"})], vec![]);
    let inf = correr(&mut e, &mut m, &RelojSimulado::nuevo(0), &Politica::nueva(R), &cfg_base(), "t", &mut |_, _| {});
    let salidas: Vec<String> = inf.mensajes.iter().filter(|x| x["role"] == "tool").map(|x| x["content"].as_str().unwrap().to_string()).collect();
    assert_eq!(salidas.len(), 4);
    assert!(salidas[0].starts_with("No encuentro") && !salidas[0].contains("AVISO"));
    // La segunda sí se ejecuta, pero lleva el aviso; a la tercera se la bloquea sin ejecutarla.
    assert!(salidas[1].contains("AVISO: es la misma llamada"), "{}", salidas[1]);
    assert!(salidas[2].starts_with("No he ejecutado esta llamada"), "{}", salidas[2]);
    assert!(salidas[2].contains("`read`") && salidas[3].starts_with("No he ejecutado"));
    assert_eq!(inf.repetidas, 2);
}

#[test]
fn una_llamada_distinta_o_que_funciona_reinicia_la_cuenta() {
    let mut e = repo();
    let mala = |b: &str| llamada("reemplazar_texto", json!({"filePath": "src/lib.rs", "buscar": b, "nuevo": "x"}));
    let mut m = Guion(vec![mala("nada uno"), mala("nada dos"), mala("nada uno"), mala("nada uno"), json!({"role": "assistant", "content": "fin"})], vec![]);
    let inf = correr(&mut e, &mut m, &RelojSimulado::nuevo(0), &Politica::nueva(R), &cfg_base(), "t", &mut |_, _| {});
    assert_eq!(inf.repetidas, 0, "sólo cuentan las idénticas seguidas");
}

#[test]
fn repetidas_max_cero_lo_apaga() {
    let mut e = repo();
    let mala = || llamada("reemplazar_texto", json!({"filePath": "src/lib.rs", "buscar": "no existe", "nuevo": "x"}));
    let mut cfg = cfg_base();
    cfg.repetidas_max = 0;
    let mut m = Guion(vec![mala(), mala(), mala(), json!({"role": "assistant", "content": "fin"})], vec![]);
    let inf = correr(&mut e, &mut m, &RelojSimulado::nuevo(0), &Politica::nueva(R), &cfg, "t", &mut |_, _| {});
    assert_eq!(inf.repetidas, 0);
}

#[test]
fn reemplazar_acepta_los_numeros_de_linea_de_read_y_da_una_pista_si_no_encuentra() {
    let mut e = repo();
    // El modelo copia la línea con el número que mostró `read`.
    let r = ejecutar(&mut e, &Politica::nueva(R), "reemplazar_texto", &json!({"filePath": "src/lib.rs", "buscar": "2:     if k == key {", "nuevo": "    if k.eq_ignore_ascii_case(key) {"}));
    assert!(!r.error, "{}", r.texto);
    assert!(String::from_utf8(e.f["/w/src/lib.rs"].clone()).unwrap().contains("eq_ignore_ascii_case"));
    // Un fragmento inventado que se parece a una línea real: la respuesta enseña la más parecida, con su número.
    let r = ejecutar(&mut e, &Politica::nueva(R), "reemplazar_texto", &json!({"filePath": "src/lib.rs", "buscar": "if k.eq_ignore_ascii_case(chave) {", "nuevo": "x"}));
    assert!(r.error);
    assert!(r.texto.contains("La línea más parecida del archivo es:") && r.texto.contains("2: "), "{}", r.texto);
}

#[test]
fn las_salidas_antiguas_se_recortan_antes_de_agotar_el_contexto() {
    let mut e = repo();
    let grande = "x".repeat(6000);
    e.f.insert("/w/grande.txt".into(), grande.clone().into_bytes());
    let lecturas: Vec<Value> = (0..12).map(|_| llamada("read", json!({"filePath": "grande.txt"}))).collect();
    let mut resto = lecturas;
    resto.push(json!({"role": "assistant", "content": "listo"}));
    let mut cfg = cfg_base();
    cfg.contexto_tokens = Some(6000); // ventana pequeña para forzarlo
    cfg.repetidas_max = 0; // las doce lecturas son iguales a propósito
    cfg.repetidas_ok_max = 0;
    let mut m = Guion(resto, vec![]);
    let inf = correr(&mut e, &mut m, &RelojSimulado::nuevo(0), &Politica::nueva(R), &cfg, "t", &mut |_, _| {});
    assert_eq!(inf.fin, Fin::Terminado);
    assert!(inf.recortadas > 0, "debía recortar salidas viejas");
    let tools: Vec<&str> = inf.mensajes.iter().filter(|x| x["role"] == "tool").map(|x| x["content"].as_str().unwrap()).collect();
    assert!(tools[0].contains("salida antigua recortada") && tools[0].len() < 600, "{}", tools[0].len());
    assert!(!tools.last().unwrap().contains("salida antigua recortada"), "la última salida se conserva entera");
    // Cada resultado sigue atado a su llamada.
    assert_eq!(inf.mensajes.iter().filter(|x| x["role"] == "tool").count(), 12);
}

#[test]
fn quien_intenta_terminar_sin_haber_editado_nada_recibe_un_aviso_con_tope() {
    // Lee, comprueba (en verde) y acaba sin editar: dos avisos y entonces se acepta que termine.
    let mut e = repo();
    let mut m = Guion(
        vec![llamada("read", json!({"filePath": "src/lib.rs"})), json!({"role": "assistant", "content": "ya está"}), json!({"role": "assistant", "content": "sigue sin haber nada"}), json!({"role": "assistant", "content": "de verdad, terminé"})],
        vec![],
    );
    let inf = correr(&mut e, &mut m, &RelojSimulado::nuevo(0), &Politica::nueva(R), &Config::nueva("m", "s"), "t", &mut |_, _| {});
    assert_eq!((inf.fin, inf.sin_editar, inf.empujones), (Fin::Terminado, 2, 0), "{:?}", inf.error);
    assert!(m.1[2].contains("No has modificado ningún archivo"), "el aviso llega al modelo: {:?}", m.1);
    assert_eq!(inf.respuesta_final, "de verdad, terminé");
}

#[test]
fn con_una_edicion_hecha_no_hay_aviso_de_sin_editar() {
    let edita = llamada("reemplazar_texto", json!({"filePath": "src/lib.rs", "buscar": "k == key", "nuevo": "k == *key"}));
    let mut e = repo();
    let mut m = Guion(vec![edita, llamada("bash", json!({"command": "cargo test"})), json!({"role": "assistant", "content": "hecho"})], vec![]);
    let inf = correr(&mut e, &mut m, &RelojSimulado::nuevo(0), &Politica::nueva(R), &Config::nueva("m", "s"), "t", &mut |_, _| {});
    assert_eq!((inf.fin, inf.sin_editar), (Fin::Terminado, 0), "{:?}", inf.error);
}

#[test]
fn una_llamada_identica_que_ya_salio_bien_no_se_repite_y_el_intento_se_cierra() {
    // `cargo test` sale bien (el repo en memoria lo da por pasado) y el modelo lo repite sin cesar tras una edición.
    let edita = llamada("reemplazar_texto", json!({"filePath": "src/lib.rs", "buscar": "k == key", "nuevo": "k == *key"}));
    let test = llamada("bash", json!({"command": "cargo test"}));
    let mut e = repo();
    let mut guion = vec![edita];
    for _ in 0..8 {
        guion.push(test.clone());
    }
    let mut m = Guion(guion, vec![]);
    let mut c = Config::nueva("m", "s");
    c.avisos_sin_editar = 0;
    let inf = correr(&mut e, &mut m, &RelojSimulado::nuevo(0), &Politica::nueva(R), &c, "t", &mut |_, _| {});
    // La 1ª es nueva y las repeticiones 1ª y 2ª se ejecutan; la 3ª se bloquea con aviso y la 5ª repetición (3 + 2) cierra el intento.
    assert_eq!(inf.fin, Fin::Terminado, "{:?}", inf.error);
    assert!(inf.respuesta_final.contains("intento cerrado"), "{}", inf.respuesta_final);
    assert!(inf.repetidas >= 3, "repetidas = {}", inf.repetidas);
    assert!(inf.llamadas <= 7, "no debe llegar al final del guion: {} llamadas", inf.llamadas);
    assert!(m.1.iter().any(|t| t.contains("es idéntica a la anterior")), "el aviso llega al modelo");
}

#[test]
fn una_llamada_igual_pero_con_una_edicion_en_medio_si_se_ejecuta() {
    let edita = llamada("reemplazar_texto", json!({"filePath": "src/lib.rs", "buscar": "k == key", "nuevo": "k == *key"}));
    let edita2 = llamada("reemplazar_texto", json!({"filePath": "src/lib.rs", "buscar": "k == *key", "nuevo": "*k == *key"}));
    let test = llamada("bash", json!({"command": "cargo test"}));
    let mut e = repo();
    let mut m = Guion(vec![edita, test.clone(), edita2, test.clone(), json!({"role": "assistant", "content": "listo"})], vec![]);
    let mut c = Config::nueva("m", "s");
    c.avisos_sin_editar = 0;
    let inf = correr(&mut e, &mut m, &RelojSimulado::nuevo(0), &Politica::nueva(R), &c, "t", &mut |_, _| {});
    assert_eq!((inf.fin, inf.repetidas), (Fin::Terminado, 0), "{:?}", inf.error);
}

#[test]
fn alternar_dos_comprobaciones_ya_en_verde_tambien_cuenta_como_repetir() {
    // Como R40 de la campaña 12: `cargo test` y `cargo test -- --nocapture` alternados, sin editar nada entre medias.
    let edita = llamada("reemplazar_texto", json!({"filePath": "src/lib.rs", "buscar": "k == key", "nuevo": "k == *key"}));
    let a = llamada("bash", json!({"command": "cargo test"}));
    let b = llamada("bash", json!({"command": "cargo test -- --nocapture"}));
    let mut e = repo();
    let mut guion = vec![edita];
    for i in 0..12 {
        guion.push(if i % 2 == 0 { a.clone() } else { b.clone() });
    }
    let mut m = Guion(guion, vec![]);
    let mut c = Config::nueva("m", "s");
    c.avisos_sin_editar = 0;
    let inf = correr(&mut e, &mut m, &RelojSimulado::nuevo(0), &Politica::nueva(R), &c, "t", &mut |_, _| {});
    assert_eq!(inf.fin, Fin::Terminado, "{:?}", inf.error);
    assert!(inf.respuesta_final.contains("intento cerrado"), "{}", inf.respuesta_final);
    assert!(inf.llamadas <= 8, "no debe agotar el guion: {} llamadas", inf.llamadas);
}

#[test]
fn con_exigir_pruebas_quien_edita_sin_escribir_ninguna_recibe_un_aviso_y_luego_termina() {
    let edita = llamada("reemplazar_texto", json!({"filePath": "src/lib.rs", "buscar": "k == key", "nuevo": "k == *key"}));
    let mut e = repo();
    let mut m = Guion(vec![edita, llamada("bash", json!({"command": "cargo test"})), json!({"role": "assistant", "content": "hecho"}), json!({"role": "assistant", "content": "de verdad"})], vec![]);
    let mut c = Config::nueva("m", "s");
    c.avisos_sin_editar = 0;
    c.exigir_pruebas = true;
    let inf = correr(&mut e, &mut m, &RelojSimulado::nuevo(0), &Politica::nueva(R), &c, "t", &mut |_, _| {});
    assert_eq!((inf.fin, inf.sin_pruebas), (Fin::Terminado, 1), "{:?}", inf.error);
    assert!(m.1.iter().any(|t| t.contains("No has escrito ninguna prueba propia")), "el aviso llega al modelo");
    assert_eq!(inf.respuesta_final, "de verdad");
}

#[test]
fn con_exigir_pruebas_una_prueba_insertada_cuenta_y_no_hay_aviso() {
    let prueba = llamada("insertar", json!({"filePath": "src/lib.rs", "linea": 0, "texto": "#[cfg(test)]\nmod pruebas_agente {\n    #[test]\n    fn nada() {}\n}"}));
    let mut e = repo();
    let mut m = Guion(vec![prueba, llamada("bash", json!({"command": "cargo test"})), json!({"role": "assistant", "content": "hecho"})], vec![]);
    let mut c = Config::nueva("m", "s");
    c.avisos_sin_editar = 0;
    c.exigir_pruebas = true;
    let inf = correr(&mut e, &mut m, &RelojSimulado::nuevo(0), &Politica::nueva(R), &c, "t", &mut |_, _| {});
    assert_eq!((inf.fin, inf.sin_pruebas), (Fin::Terminado, 0), "{:?}", inf.error);
}

#[test]
fn repetir_una_edicion_ya_aplicada_dice_que_ya_esta_hecha_y_enseña_como_queda() {
    let mut e = repo();
    let args = json!({"filePath": "src/lib.rs", "buscar": "k == key", "nuevo": "k.eq_ignore_ascii_case(key) && true"});
    let r = ejecutar(&mut e, &Politica::nueva(R), "reemplazar_texto", &args);
    assert!(!r.error, "{}", r.texto);
    // Segunda vez: `buscar` ya no existe, pero `nuevo` sí.
    let r = ejecutar(&mut e, &Politica::nueva(R), "reemplazar_texto", &args);
    assert!(r.error);
    assert!(r.texto.contains("YA está aplicado") && r.texto.contains("eq_ignore_ascii_case") && r.texto.contains("No repitas"), "{}", r.texto);
    // Un fragmento que no existe y cuyo `nuevo` tampoco está sigue dando la pista de siempre.
    let r = ejecutar(&mut e, &Politica::nueva(R), "reemplazar_texto", &json!({"filePath": "src/lib.rs", "buscar": "nada de esto existe aqui", "nuevo": "otra cosa distinta por completo"}));
    assert!(r.error && r.texto.contains("No encuentro"), "{}", r.texto);
}

#[test]
fn un_bloque_de_varias_lineas_que_ya_no_existe_se_responde_con_el_tramo_actual_entero() {
    let mut e = repo();
    // Primero se cambia una línea; después se pide reemplazar un bloque de 4 líneas con el texto ANTIGUO.
    let r = ejecutar(&mut e, &Politica::nueva(R), "reemplazar_texto", &json!({"filePath": "src/lib.rs", "buscar": "k == key", "nuevo": "k.eq_ignore_ascii_case(key)"}));
    assert!(!r.error, "{}", r.texto);
    let antiguo = "fn buscar(k: &str) {\n    if k == key {\n        hit();\n    }\n}";
    let r = ejecutar(&mut e, &Politica::nueva(R), "reemplazar_texto", &json!({"filePath": "src/lib.rs", "buscar": antiguo, "nuevo": "return lookup_ci(entries, key);"}));
    assert!(r.error, "{}", r.texto);
    assert!(r.texto.contains("ha cambiado desde que lo leíste") && r.texto.contains("eq_ignore_ascii_case"), "{}", r.texto);
}

fn bash(e: &mut Mem, orden: &str) -> (String, bool) {
    llama(e, "bash", json!({"command": orden}))
}

#[test]
fn cargo_test_con_las_opciones_en_el_lado_equivocado_de_los_guiones_se_reordena() {
    let mut e = repo();
    let (t, err) = bash(&mut e, "cargo test -p soso-http -- --lib");
    assert!(!err, "{t}");
    assert_eq!(e.ordenes.borrow().last().unwrap(), &["cargo", "test", "-p", "soso-http", "--lib"].map(String::from).to_vec());
    assert!(t.contains("he reordenado") && t.contains("--lib"), "{t}");
    // `--test=NOMBRE` detrás de `--` es un filtro de nombre; `--doc` y `--lib` se mueven antes.
    let (t, _) = bash(&mut e, "cargo test -p x --features std -- --lib -- --test=pruebas_agente");
    assert_eq!(e.ordenes.borrow().last().unwrap(), &["cargo", "test", "-p", "x", "--features", "std", "--lib", "--", "pruebas_agente"].map(String::from).to_vec(), "{t}");
    // Una opción de libtest sin `--` se pone detrás.
    bash(&mut e, "cargo test -p x --nocapture");
    assert_eq!(e.ordenes.borrow().last().unwrap(), &["cargo", "test", "-p", "x", "--", "--nocapture"].map(String::from).to_vec());
    // Lo que ya está bien no se toca ni se anota.
    let (t, _) = bash(&mut e, "cargo test -p x --lib -- --nocapture pruebas_agente");
    assert!(!t.contains("reordenado"), "{t}");
    assert_eq!(e.ordenes.borrow().last().unwrap(), &["cargo", "test", "-p", "x", "--lib", "--", "--nocapture", "pruebas_agente"].map(String::from).to_vec());
}

#[test]
fn bash_admite_una_tuberia_final_con_head_tail_o_grep_y_quita_2_redirigido_a_1() {
    let mut e = repo();
    let (t, err) = bash(&mut e, "cargo test -p x 2>&1 | grep passed");
    assert!(!err, "{t}");
    assert!(t.contains("1 passed") && t.contains("filtrado"), "{t}");
    assert_eq!(e.ordenes.borrow().last().unwrap(), &["cargo", "test", "-p", "x"].map(String::from).to_vec(), "2>&1 se quita");
    let (t, _) = bash(&mut e, "cargo test -p x | head -1");
    assert!(t.contains("test result: ok"), "{t}");
    let (t, _) = bash(&mut e, "cargo test -p x | grep -c ok");
    assert!(t.contains("\n1\n"), "{t}");
    // Un filtro que no existe, o varias tuberías, siguen rechazadas con un motivo.
    let (t, err) = bash(&mut e, "cargo test -p x | sort");
    assert!(err && t.contains("sólo se admiten"), "{t}");
    let (t, err) = bash(&mut e, "cargo test -p x | head -1 | tail -1");
    assert!(err && t.contains("no está disponible"), "{t}");
}

#[test]
fn una_orden_de_shell_no_permitida_dice_que_herramienta_usar() {
    let mut e = repo();
    let (t, err) = bash(&mut e, "tree crates");
    assert!(err && t.contains("`glob`"), "{t}");
    let (t, err) = bash(&mut e, "sed -n 1,5p src/lib.rs");
    assert!(err && t.contains("`read`"), "{t}");
    let (t, err) = bash(&mut e, "awk 1 src/lib.rs");
    assert!(err && t.contains("`read`"), "{t}");
    // `ls`, `cat`, `grep` y `find` no se rechazan: se resuelven con las herramientas propias.
    let (t, err) = bash(&mut e, "ls crates");
    assert!(err && t.contains("no existe"), "{t}");
}

#[test]
fn insertar_acepta_el_final_del_archivo_y_nombres_de_argumento_alternativos() {
    let mut e = repo();
    // `linea: -1` = al final; `text` en vez de `texto`.
    let (t, err) = llama(&mut e, "insertar", json!({"filePath": "src/lib.rs", "linea": -1, "text": "fn final_() {}"}));
    assert!(!err, "{t}");
    let c = String::from_utf8(e.f["/w/src/lib.rs"].clone()).unwrap();
    assert!(c.ends_with("}\nfn final_() {}\n"), "{c:?}");
    let (t, err) = llama(&mut e, "insertar", json!({"file_path": "src/lib.rs", "linea": "final", "code": "fn otra_() {}"}));
    assert!(!err, "{t}");
    assert!(String::from_utf8(e.f["/w/src/lib.rs"].clone()).unwrap().ends_with("fn final_() {}\nfn otra_() {}\n"));
    // `reemplazar_texto` con `old`/`new`.
    let (t, err) = llama(&mut e, "reemplazar_texto", json!({"path": "src/otro.rs", "old": "fn otro() {}", "new": "fn otro() { /* x */ }"}));
    assert!(!err, "{t}");
}

#[test]
fn reemplazar_texto_por_lotes_es_todo_o_nada_y_con_linea_desambigua() {
    let mut e = Mem::default().con("/w/a.rs", "fn a() {\n    x();\n}\nfn b() {\n    x();\n}\n");
    // `linea` elige la aparición.
    let (t, err) = llama(&mut e, "reemplazar_texto", json!({"filePath": "a.rs", "buscar": "x();", "nuevo": "y();", "linea": 5}));
    assert!(!err, "{t}");
    assert_eq!(e.f["/w/a.rs"], b"fn a() {\n    x();\n}\nfn b() {\n    y();\n}\n");
    // Lote que funciona: dos cambios en una llamada.
    let (t, err) = llama(&mut e, "reemplazar_texto", json!({"filePath": "a.rs", "ediciones": [{"buscar": "fn a()", "nuevo": "fn alfa()"}, {"buscar": "fn b()", "nuevo": "fn beta()"}]}));
    assert!(!err && t.contains("2 ediciones") && t.contains("1)") && t.contains("2)"), "{t}");
    let c = String::from_utf8(e.f["/w/a.rs"].clone()).unwrap();
    assert!(c.contains("fn alfa()") && c.contains("fn beta()"), "{c}");
    // Lote con una edición imposible: no se escribe NADA, ni siquiera la primera.
    let antes = e.f["/w/a.rs"].clone();
    let (t, err) = llama(&mut e, "reemplazar_texto", json!({"filePath": "a.rs", "ediciones": [{"buscar": "fn alfa()", "nuevo": "fn uno()"}, {"buscar": "esto no existe en el archivo", "nuevo": "otra cosa"}]}));
    assert!(err && t.contains("edición 2 de 2") && t.contains("NO se ha escrito"), "{t}");
    assert_eq!(e.f["/w/a.rs"], antes);
}

#[test]
fn grep_muestra_contexto_con_pocas_coincidencias_y_read_amplia_las_ventanas_pequenas() {
    let cuerpo: String = (1..=200).map(|i| format!("linea{i}\n")).collect();
    let mut e = Mem::default().con("/w/g.rs", &format!("{cuerpo}fn objetivo() {{}}\nfin\n"));
    let (t, err) = llama(&mut e, "grep", json!({"pattern": "fn objetivo"}));
    assert!(!err && t.contains("g.rs:201: fn objetivo() {}") && t.contains("g.rs-199- linea199") && t.contains("g.rs-202- fin"), "{t}");
    let (t, _) = llama(&mut e, "grep", json!({"pattern": "fn objetivo", "contexto": 0}));
    assert!(!t.contains("g.rs-199-"), "{t}");
    // read con limit 5 devuelve una ventana mínima de 80 líneas.
    let (t, err) = llama(&mut e, "read", json!({"filePath": "g.rs", "offset": 10, "limit": 5}));
    assert!(!err && t.contains("\n10: linea10") && t.contains("\n89: linea89") && !t.contains("\n90:") && t.contains("ampliado la ventana"), "{t}");
}

#[test]
fn insertar_con_ancla_de_texto_y_la_salida_de_cargo_se_compacta() {
    let mut e = Mem::default().con("/w/b.rs", "fn uno() {}\nfn dos() {}\nfn tres() {}\n");
    let (t, err) = llama(&mut e, "insertar", json!({"filePath": "b.rs", "despues_de": "fn dos()", "texto": "fn entre() {}"}));
    assert!(!err, "{t}");
    assert_eq!(e.f["/w/b.rs"], b"fn uno() {}\nfn dos() {}\nfn entre() {}\nfn tres() {}\n");
    let (t, err) = llama(&mut e, "insertar", json!({"filePath": "b.rs", "antes_de": "fn uno()", "texto": "// cabecera"}));
    assert!(!err && String::from_utf8(e.f["/w/b.rs"].clone()).unwrap().starts_with("// cabecera\nfn uno()"), "{t}");
    let (t, err) = llama(&mut e, "insertar", json!({"filePath": "b.rs", "despues_de": "fn ", "texto": "x"}));
    assert!(err && t.contains("aparece en"), "{t}");
    let (t, err) = llama(&mut e, "insertar", json!({"filePath": "b.rs", "despues_de": "no existe", "texto": "x"}));
    assert!(err && t.contains("No hay ninguna línea"), "{t}");
}

#[test]
fn la_salida_larga_de_cargo_se_compacta_sin_perder_errores_ni_resumen() {
    struct Ruidoso(Mem);
    impl Archivos for Ruidoso {
        fn leer(&self, r: &str) -> Resultado<Vec<u8>> { self.0.leer(r) }
        fn escribir(&mut self, r: &str, d: &[u8], m: u32) -> Resultado<()> { self.0.escribir(r, d, m) }
        fn existe(&self, r: &str) -> bool { self.0.existe(r) }
        fn listar(&self, r: &str) -> Resultado<Vec<Entrada>> { self.0.listar(r) }
        fn metadatos(&self, r: &str) -> Resultado<Entrada> { self.0.metadatos(r) }
        fn crear_directorio(&mut self, r: &str) -> Resultado<()> { self.0.crear_directorio(r) }
        fn borrar(&mut self, r: &str) -> Resultado<()> { self.0.borrar(r) }
    }
    impl Procesos for Ruidoso {
        fn ejecutar(&self, _: &Orden) -> Resultado<Salida> {
            let mut t = String::from("   Compiling a v0.1.0\n   Compiling b v0.1.0\n");
            for i in 0..12 {
                t.push_str(&format!("warning: unused variable: `v{i}`\n  --> src/x.rs:{i}:9\n   |\n{i} |     let v{i} = 1;\n   |         ^^ help: prefix it with an underscore\n\n"));
            }
            t.push_str("error[E0063]: missing field `y` in initializer of `P`\n  --> src/lib.rs:40:9\n   |\n40 |         P { x: 1 }\n   |         ^ missing `y`\n\n");
            for i in 0..20 {
                t.push_str(&format!("test pruebas::caso_{i} ... ok\n"));
            }
            t.push_str("test pruebas::roto ... FAILED\n\ntest result: FAILED. 20 passed; 1 failed\n");
            Ok(Salida { codigo: Some(101), stdout: t.into_bytes(), stderr: vec![], motivo: None })
        }
    }
    let mut e = Ruidoso(repo());
    let r = ejecutar(&mut e, &Politica::nueva(R), "bash", &json!({"command": "cargo test -p x"}));
    assert!(r.error);
    assert!(r.texto.contains("error[E0063]: missing field `y`") && r.texto.contains("src/lib.rs:40:9") && r.texto.contains("roto ... FAILED") && r.texto.contains("test result: FAILED"), "{}", r.texto);
    assert!(!r.texto.contains("unused variable") && !r.texto.contains("Compiling") && !r.texto.contains("caso_3 ... ok"), "{}", r.texto);
    assert!(r.texto.contains("salida compactada") && r.texto.contains("12 aviso(s)") && r.texto.contains("20 prueba(s) que pasan"), "{}", r.texto);
    // Con `| grep` se trabaja sobre la salida COMPLETA (el modelo puede querer buscar un aviso).
    let r = ejecutar(&mut e, &Politica::nueva(R), "bash", &json!({"command": "cargo test -p x | grep -c unused"}));
    assert!(r.texto.contains("\n12\n"), "{}", r.texto);
}

#[test]
fn las_llamadas_bloqueadas_por_el_guardian_no_gastan_presupuesto_de_herramientas() {
    let edita = llamada("reemplazar_texto", json!({"filePath": "src/lib.rs", "buscar": "k == key", "nuevo": "k == *key"}));
    let test = llamada("bash", json!({"command": "cargo test"}));
    let mut e = repo();
    let mut guion = vec![edita, test.clone(), test.clone(), test.clone(), test.clone()];
    guion.push(json!({"role": "assistant", "content": "listo"}));
    let mut m = Guion(guion, vec![]);
    let mut c = Config::nueva("m", "s");
    c.avisos_sin_editar = 0;
    let inf = correr(&mut e, &mut m, &RelojSimulado::nuevo(0), &Politica::nueva(R), &c, "t", &mut |_, _| {});
    assert_eq!(inf.fin, Fin::Terminado, "{:?}", inf.error);
    assert!(inf.bloqueadas >= 1, "bloqueadas = {}", inf.bloqueadas);
    // 1 edición + las comprobaciones que SÍ se ejecutaron; las bloqueadas no cuentan.
    assert_eq!(inf.llamadas + inf.bloqueadas, 5, "llamadas {} bloqueadas {}", inf.llamadas, inf.bloqueadas);
}

fn contenido(e: &mut Mem, ruta: &str) -> String {
    String::from_utf8(e.leer(ruta).unwrap()).unwrap()
}

#[test]
fn reemplazar_texto_no_duplica_la_sangria_ni_el_atributo() {
    let mut e = Mem::default().con("/w/src/a.rs", "#[derive(Clone)]\npub struct S;\n\nimpl S {\n    fn f() -> u8 {\n        1\n    }\n}\n");
    // `nuevo` trae su sangría y `buscar` no: la línea no puede acabar con ocho espacios.
    let (t, err) = llama(&mut e, "reemplazar_texto", json!({"filePath": "src/a.rs", "buscar": "fn f() -> u8 {", "nuevo": "    fn f() -> u16 {"}));
    assert!(!err, "{t}");
    assert!(contenido(&mut e, "/w/src/a.rs").contains("\n    fn f() -> u16 {\n"), "{}", contenido(&mut e, "/w/src/a.rs"));
    // El modelo repite el atributo que ya está encima.
    let (t, err) = llama(&mut e, "reemplazar_texto", json!({"filePath": "src/a.rs", "buscar": "pub struct S;", "nuevo": "#[derive(Clone)]\n#[derive(Debug)]\npub struct S;"}));
    assert!(!err, "{t}");
    let c = contenido(&mut e, "/w/src/a.rs");
    assert_eq!(c.matches("#[derive(Clone)]").count(), 1, "{c}");
    assert!(c.contains("#[derive(Debug)]\npub struct S;"), "{c}");
}

#[test]
fn reemplazar_lineas_sustituye_un_rango_y_valida() {
    let mut e = repo();
    let (t, err) = llama(&mut e, "reemplazar_lineas", json!({"filePath": "src/lib.rs", "desde": 2, "hasta": 4, "nuevo": "    if k.is_empty() {\n        return;\n    }\n    hit();"}));
    assert!(!err && t.starts_with("Hecho") && t.contains("2: ") && t.contains("Se quitó"), "{t}");
    assert_eq!(contenido(&mut e, "/w/src/lib.rs"), "fn buscar(k: &str) {\n    if k.is_empty() {\n        return;\n    }\n    hit();\n}\n");
    let (t, err) = llama(&mut e, "reemplazar_lineas", json!({"filePath": "src/lib.rs", "desde": 5, "hasta": 99, "nuevo": "x"}));
    assert!(err && t.contains("Rango inválido"), "{t}");
    // Un rango que dejaría llaves desequilibradas no se escribe.
    let antes = contenido(&mut e, "/w/src/lib.rs");
    let (t, err) = llama(&mut e, "reemplazar_lineas", json!({"filePath": "src/lib.rs", "desde": 6, "hasta": 6, "nuevo": ""}));
    assert!(err && t.contains("No he escrito"), "{t}");
    assert_eq!(contenido(&mut e, "/w/src/lib.rs"), antes);
    // Borrar líneas con `nuevo` vacío.
    let (t, err) = llama(&mut e, "reemplazar_lineas", json!({"filePath": "src/lib.rs", "desde": 5, "hasta": 5, "nuevo": ""}));
    assert!(!err, "{t}");
    assert!(!contenido(&mut e, "/w/src/lib.rs").contains("hit();"));
}

#[test]
fn un_cargo_test_sin_ninguna_prueba_avisa_de_que_no_demuestra_nada() {
    struct Cero(Mem);
    impl Archivos for Cero {
        fn leer(&self, r: &str) -> Resultado<Vec<u8>> { self.0.leer(r) }
        fn escribir(&mut self, r: &str, d: &[u8], m: u32) -> Resultado<()> { self.0.escribir(r, d, m) }
        fn existe(&self, r: &str) -> bool { self.0.existe(r) }
        fn listar(&self, r: &str) -> Resultado<Vec<Entrada>> { self.0.listar(r) }
        fn metadatos(&self, r: &str) -> Resultado<Entrada> { self.0.metadatos(r) }
        fn crear_directorio(&mut self, r: &str) -> Resultado<()> { self.0.crear_directorio(r) }
        fn borrar(&mut self, r: &str) -> Resultado<()> { self.0.borrar(r) }
    }
    impl Procesos for Cero {
        fn ejecutar(&self, o: &Orden) -> Resultado<Salida> {
            let n = if o.argv.iter().any(|a| a == "vacio") { 0 } else { 3 };
            let t = format!("running {n} tests\n\ntest result: ok. {n} passed; 0 failed\n\nrunning 0 tests\n");
            Ok(Salida { codigo: Some(0), stdout: t.into_bytes(), stderr: vec![], motivo: None })
        }
    }
    let mut e = Cero(repo());
    let sh = |e: &mut Cero, c: &str| { let r = ejecutar(e, &Politica::nueva(R), "bash", &json!({"command": c})); (r.texto, r.error) };
    let (t, err) = sh(&mut e, "cargo test -p x vacio");
    assert!(!err && t.contains("NINGUNA prueba") && t.contains("--list"), "{t}");
    let (t, _) = sh(&mut e, "cargo test -p x");
    assert!(!t.contains("NINGUNA"), "{t}");
}

#[test]
fn cargo_test_con_test_filter_o_args_usa_el_valor_como_filtro() {
    let mut e = repo();
    bash(&mut e, "cargo test -p x --lib -- --test-filter pruebas_agente");
    assert_eq!(e.ordenes.borrow().last().unwrap(), &["cargo", "test", "-p", "x", "--lib", "--", "pruebas_agente"].map(String::from).to_vec());
    bash(&mut e, "cargo test -p x -- --args=pruebas_agente");
    assert_eq!(e.ordenes.borrow().last().unwrap(), &["cargo", "test", "-p", "x", "--", "pruebas_agente"].map(String::from).to_vec());
}

#[test]
fn las_ordenes_de_shell_de_lectura_se_resuelven_con_las_herramientas_propias() {
    let mut e = repo().con("/w/crates/b/src/lib.rs", "pub struct MemDev;\n").con("/w/crates/b/Cargo.toml", "[package]\n");
    let (t, err) = bash(&mut e, "find . -name \"*.rs\" -exec grep -l \"MemDev\" {} \\;");
    assert!(!err && t.contains("crates/b/src/lib.rs") && !t.contains("src/otro.rs"), "{t}");
    let (t, _) = bash(&mut e, "find . -name lib.rs -o -name otro.rs | head -5");
    assert!(t.contains("src/otro.rs") && t.contains("crates/b/src/lib.rs"), "{t}");
    let (t, _) = bash(&mut e, "find . -path \"*/b/*\" -name \"*.rs\"");
    assert!(t.contains("crates/b/src/lib.rs") && !t.contains("src/otro.rs"), "{t}");
    let (t, err) = bash(&mut e, "find . -type d");
    assert!(err && t.contains("glob"), "{t}");
    let (t, _) = bash(&mut e, "ls crates/b/");
    assert!(t.contains("Cargo.toml") && t.contains("src"), "{t}");
    let (t, _) = bash(&mut e, "cat README.md");
    assert!(t.contains("hola"), "{t}");
    let (t, _) = bash(&mut e, "grep -rn \"fn otro\" --include=*.rs .");
    assert!(t.contains("src/otro.rs:1:"), "{t}");
    let (t, _) = bash(&mut e, "grep -rl \"MemDev\\|otro\" .");
    assert!(t.contains("crates/b/src/lib.rs") && t.contains("src/otro.rs") && !t.contains(":1:"), "{t}");
}

#[test]
fn cd_como_prefijo_cambia_el_directorio_de_trabajo() {
    struct Cwd(Mem, std::cell::RefCell<Vec<String>>);
    impl Archivos for Cwd {
        fn leer(&self, r: &str) -> Resultado<Vec<u8>> { self.0.leer(r) }
        fn escribir(&mut self, r: &str, d: &[u8], m: u32) -> Resultado<()> { self.0.escribir(r, d, m) }
        fn existe(&self, r: &str) -> bool { self.0.existe(r) }
        fn listar(&self, r: &str) -> Resultado<Vec<Entrada>> { self.0.listar(r) }
        fn metadatos(&self, r: &str) -> Resultado<Entrada> { self.0.metadatos(r) }
        fn crear_directorio(&mut self, r: &str) -> Resultado<()> { self.0.crear_directorio(r) }
        fn borrar(&mut self, r: &str) -> Resultado<()> { self.0.borrar(r) }
    }
    impl Procesos for Cwd {
        fn ejecutar(&self, o: &Orden) -> Resultado<Salida> {
            self.1.borrow_mut().push(format!("{} @ {}", o.argv.join(" "), o.cwd));
            Ok(Salida { codigo: Some(0), stdout: b"test result: ok. 1 passed".to_vec(), stderr: vec![], motivo: None })
        }
    }
    let mut e = Cwd(repo().con("/w/crates/b/src/lib.rs", "x\n"), Default::default());
    let sh = |e: &mut Cwd, c: &str| { let r = ejecutar(e, &Politica::nueva(R), "bash", &json!({"command": c})); (r.texto, r.error) };
    let (t, err) = sh(&mut e, "cd crates/b && cargo test --features std");
    assert!(!err && t.contains("dentro de crates/b"), "{t}");
    assert_eq!(e.1.borrow().last().unwrap(), "cargo test --features std @ /w/crates/b");
    let (t, err) = sh(&mut e, "cd ../.. && cargo test");
    assert!(err && t.contains("sale del checkout"), "{t}");
    let (t, err) = sh(&mut e, "cd crates/zzz && cargo test");
    assert!(err, "{t}");
}

#[test]
fn una_conversacion_larga_no_pasa_del_tope_de_mensajes_del_servidor() {
    let mut e = repo();
    let mut guion: Vec<Value> = (0..45).map(|i| llamada("read", json!({"filePath": "src/lib.rs", "offset": i + 1}))).collect();
    guion.push(json!({"role": "assistant", "content": "listo"}));
    let mut cfg = cfg_base();
    cfg.repetidas_max = 0;
    cfg.repetidas_ok_max = 0;
    cfg.max_turnos = 60;
    let mut m = Guion(guion, vec![]);
    let inf = correr(&mut e, &mut m, &RelojSimulado::nuevo(0), &Politica::nueva(R), &cfg, "tarea", &mut |_, _| {});
    assert_eq!(inf.fin, Fin::Terminado, "{:?}", inf.error);
    for p in &m.1 {
        let v: Value = serde_json::from_str(p).unwrap();
        assert!(v["messages"].as_array().unwrap().len() <= 60, "{} mensajes", v["messages"].as_array().unwrap().len());
    }
    let ult: Value = serde_json::from_str(m.1.last().unwrap()).unwrap();
    assert!(ult["messages"][1]["content"].as_str().unwrap().contains("Historial de lo que ya hiciste") && ult["messages"][1]["content"].as_str().unwrap().contains("- read src/lib.rs desde"));
    // Ningún resultado queda sin su llamada.
    let ms = ult["messages"].as_array().unwrap();
    for (i, x) in ms.iter().enumerate() {
        if x["role"] == "tool" {
            let mut j = i;
            while ms[j]["role"] == "tool" { j -= 1; }
            assert_eq!(ms[j]["role"], "assistant");
        }
    }
}

#[test]
fn una_edicion_que_no_cambia_nada_es_un_error_y_no_un_hecho() {
    let mut e = repo();
    let antes = contenido(&mut e, "/w/src/lib.rs");
    let (t, err) = llama(&mut e, "reemplazar_texto", json!({"filePath": "src/lib.rs", "buscar": "hit();", "nuevo": "hit();"}));
    assert!(err && t.contains("no cambia nada"), "{t}");
    let (t, err) = llama(&mut e, "reemplazar_lineas", json!({"filePath": "src/lib.rs", "desde": 3, "hasta": 3, "nuevo": "        hit();"}));
    assert!(err && t.contains("no cambia nada"), "{t}");
    assert_eq!(contenido(&mut e, "/w/src/lib.rs"), antes);
}

#[test]
fn una_linea_lejos_de_la_unica_aparicion_no_edita_otra_linea_en_silencio() {
    let mut cuerpo = String::from("let a = 1;\n");
    for i in 0..30 {
        cuerpo.push_str(&format!("let x{i} = {i};\n"));
    }
    cuerpo.push_str("let b = 2;\n");
    let mut e = Mem::default().con("/w/src/z.rs", &cuerpo);
    let (t, err) = llama(&mut e, "reemplazar_texto", json!({"filePath": "src/z.rs", "buscar": "let a = 1;", "nuevo": "let a = 9;", "linea": 32}));
    assert!(err && t.contains("sólo aparece en la línea 1") && t.contains("No he cambiado nada"), "{t}");
    assert_eq!(contenido(&mut e, "/w/src/z.rs"), cuerpo);
    // Una `linea` aproximada (desfase pequeño) sigue valiendo.
    let (t, err) = llama(&mut e, "reemplazar_texto", json!({"filePath": "src/z.rs", "buscar": "let a = 1;", "nuevo": "let a = 9;", "linea": 4}));
    assert!(!err, "{t}");
}

#[test]
fn anadir_tras_una_linea_dos_veces_no_duplica_lo_anadido() {
    let mut e = Mem::default().con("/w/src/g.rs", "fn f(d: &[u8]) {\n    let a = d[0];\n    let b = d[1];\n}\n");
    let args = json!({"filePath": "src/g.rs", "buscar": "let a = d[0];", "nuevo": "let a = d[0];\nif d.len() < 2 {\n    return;\n}"});
    let (t, err) = llama(&mut e, "reemplazar_texto", args.clone());
    assert!(!err, "{t}");
    let una_vez = contenido(&mut e, "/w/src/g.rs");
    let (t, err) = llama(&mut e, "reemplazar_texto", args);
    assert!(err && t.contains("YA está aplicado") && t.contains("DUPLICARÍA"), "{t}");
    assert_eq!(contenido(&mut e, "/w/src/g.rs"), una_vez);
    // Lo mismo para añadir ANTES de la línea.
    let args = json!({"filePath": "src/g.rs", "buscar": "let b = d[1];", "nuevo": "let zeta_valor = 0usize;\nlet b = d[1];"});
    let (_, err) = llama(&mut e, "reemplazar_texto", args.clone());
    assert!(!err);
    let (t, err) = llama(&mut e, "reemplazar_texto", args);
    assert!(err && t.contains("YA está aplicado"), "{t}");
}

#[test]
fn dos_llamadas_que_fallan_alternandose_tambien_se_bloquean_y_cierran_el_intento() {
    let mut e = repo();
    let a = json!({"filePath": "src/lib.rs", "buscar": "no existe uno", "nuevo": "x"});
    let b = json!({"filePath": "src/lib.rs", "buscar": "no existe dos", "nuevo": "y"});
    let mut guion: Vec<Value> = Vec::new();
    for _ in 0..30 {
        guion.push(llamada("reemplazar_texto", a.clone()));
        guion.push(llamada("reemplazar_texto", b.clone()));
    }
    guion.push(json!({"role": "assistant", "content": "listo"}));
    let mut cfg = cfg_base();
    cfg.max_turnos = 80;
    let mut m = Guion(guion, vec![]);
    let inf = correr(&mut e, &mut m, &RelojSimulado::nuevo(0), &Politica::nueva(R), &cfg, "t", &mut |_, _| {});
    assert_eq!(inf.fin, Fin::Terminado, "debe cerrarse por atasco, no agotar los turnos");
    assert!(inf.bloqueadas >= 8 && inf.repetidas > 0, "{} bloqueadas", inf.bloqueadas);
    assert!(inf.turnos < 30, "{} turnos", inf.turnos);
}
