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
    assert_eq!(ult["tools"].as_array().unwrap().len(), 9);
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
fn llama_server_500_por_json_de_herramienta_cortado_se_devuelve_al_modelo_con_el_consejo_de_acortar() {
    struct Mal500(Vec<Option<Value>>, Vec<String>);
    impl Modelo for Mal500 {
        fn completar(&mut self, p: &str, _t: u64) -> Resultado<String> {
            self.1.push(p.to_string());
            match self.0.remove(0) {
                None => Err(Error::entorno("el servidor contestó Some(500): {\"error\":{\"code\":500,\"message\":\"Failed to parse tool call arguments as JSON: [json.exception.parse_error.101] parse error at line 1, column 9301: syntax error while parsing value - invalid string: missing closing quote\",\"type\":\"server_error\"}}")),
                Some(m) => Ok(json!({"choices": [{"message": m}]}).to_string()),
            }
        }
    }
    let mut m = Mal500(vec![None, Some(llamada("read", json!({"filePath": "README.md"}))), Some(json!({"role": "assistant", "content": "fin"}))], vec![]);
    let inf = correr(&mut repo(), &mut m, &RelojSimulado::nuevo(0), &Politica::nueva(R), &cfg_base(), "t", &mut |_, _| {});
    assert_eq!((inf.fin, inf.formato), (Fin::Terminado, 1), "{:?}", inf.error);
    assert!(m.1[1].contains("Failed to parse tool call") && m.1[1].contains("MÁS CORTA"), "el consejo de acortar llega a la primera: {}", m.1[1]);
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
    c.avisos_sin_pruebas = 1;
    let inf = correr(&mut e, &mut m, &RelojSimulado::nuevo(0), &Politica::nueva(R), &c, "t", &mut |_, _| {});
    assert_eq!((inf.fin, inf.sin_pruebas), (Fin::Terminado, 1), "{:?}", inf.error);
    assert!(m.1.iter().any(|t| t.contains("No has escrito ninguna prueba propia")), "el aviso llega al modelo");
    assert_eq!(inf.respuesta_final, "de verdad");
}

#[test]
fn con_exigir_pruebas_una_prueba_insertada_cuenta_y_no_hay_aviso() {
    let prueba = llamada("insertar", json!({"filePath": "src/lib.rs", "linea": 0, "texto": "#[cfg(test)]\nmod pruebas_agente {\n    #[test]\n    fn nada() {}\n}"}));
    let mut e = repo();
    let mut m = Guion(vec![prueba, llamada("bash", json!({"command": "cargo test -p x -- pruebas_agente"})), json!({"role": "assistant", "content": "hecho"})], vec![]);
    let mut c = Config::nueva("m", "s");
    c.avisos_sin_editar = 0;
    c.exigir_pruebas = true;
    let inf = correr(&mut e, &mut m, &RelojSimulado::nuevo(0), &Politica::nueva(R), &c, "t", &mut |_, _| {});
    assert_eq!((inf.fin, inf.sin_pruebas, inf.sin_ejecutar), (Fin::Terminado, 0, 0), "{:?}", inf.error);
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
fn una_edicion_plegada_sigue_contando_como_edicion() {
    let mut e = repo();
    let mut guion: Vec<Value> = vec![llamada("reemplazar_texto", json!({"filePath": "src/lib.rs", "buscar": "hit();", "nuevo": "hit();\n        hit();"}))];
    for i in 0..40 {
        guion.push(llamada("grep", json!({"pattern": format!("zzz{i}")})));
    }
    guion.push(json!({"role": "assistant", "content": "listo"}));
    let mut cfg = cfg_base();
    cfg.max_turnos = 80;
    cfg.mensajes_max = Some(12);
    let mut m = Guion(guion, vec![]);
    let inf = correr(&mut e, &mut m, &RelojSimulado::nuevo(0), &Politica::nueva(R), &cfg, "t", &mut |_, _| {});
    let primera = inf.mensajes[1]["content"].as_str().unwrap_or("");
    assert!(primera.contains("→ Hecho"), "el historial debe conservar la edición: {primera}");
    assert!(soso_improve_core::agente::hubo_edicion(&inf.mensajes));
}

#[test]
fn las_ediciones_sin_efecto_seguidas_suben_la_temperatura_y_una_edicion_buena_la_baja() {
    let mut e = repo();
    let igual = json!({"filePath": "src/lib.rs", "buscar": "hit();", "nuevo": "hit();"});
    let mut guion: Vec<Value> = (0..4).map(|_| llamada("reemplazar_texto", igual.clone())).collect();
    guion.push(llamada("reemplazar_texto", json!({"filePath": "src/lib.rs", "buscar": "hit();", "nuevo": "hit();\n        hit();"})));
    guion.push(json!({"role": "assistant", "content": "listo"}));
    let mut cfg = cfg_base();
    cfg.max_turnos = 20;
    let mut m = Guion(guion, vec![]);
    let _ = correr(&mut e, &mut m, &RelojSimulado::nuevo(0), &Politica::nueva(R), &cfg, "t", &mut |_, _| {});
    let temps: Vec<f64> = m.1.iter().map(|p| serde_json::from_str::<Value>(p).unwrap()["temperature"].as_f64().unwrap()).collect();
    assert_eq!(temps[0], 0.0);
    assert_eq!(temps[1], 0.0, "con una sola edición sin efecto aún no sube");
    assert!(temps[2] > 0.0 && temps[3] > temps[2], "{temps:?}");
    assert_eq!(temps[5], 0.0, "tras una edición buena vuelve a 0: {temps:?}");
}

#[test]
fn los_prompts_dicen_donde_va_una_comprobacion_de_entrada() {
    for p in [soso_improve_core::agente::SISTEMA, soso_improve_core::agente::SISTEMA_PRUEBAS] {
        assert!(p.contains("va al PRINCIPIO de la función"), "{p}");
    }
}

/// Entorno que contesta a todo con un error de compilación por falta del preludio de `std` (crate `no_std`).
struct ConErrorNoStd(Mem);
impl Archivos for ConErrorNoStd {
    fn leer(&self, r: &str) -> Resultado<Vec<u8>> { self.0.leer(r) }
    fn escribir(&mut self, r: &str, d: &[u8], m: u32) -> Resultado<()> { self.0.escribir(r, d, m) }
    fn existe(&self, r: &str) -> bool { self.0.existe(r) }
    fn listar(&self, r: &str) -> Resultado<Vec<Entrada>> { self.0.listar(r) }
    fn metadatos(&self, r: &str) -> Resultado<Entrada> { self.0.metadatos(r) }
    fn crear_directorio(&mut self, r: &str) -> Resultado<()> { self.0.crear_directorio(r) }
    fn borrar(&mut self, r: &str) -> Resultado<()> { self.0.borrar(r) }
}
impl Procesos for ConErrorNoStd {
    fn ejecutar(&self, o: &Orden) -> Resultado<Salida> {
        self.0.ordenes.borrow_mut().push(o.argv.clone());
        Ok(Salida { codigo: Some(101), stdout: b"error: cannot find macro `vec` in this scope\n --> src/lib.rs:9:5\n".to_vec(), stderr: vec![], motivo: None })
    }
}

#[test]
fn un_error_por_falta_de_vec_en_un_crate_no_std_trae_la_pista_de_alloc() {
    let mut e = ConErrorNoStd(repo());
    let r = ejecutar(&mut e, &Politica::nueva(R), "bash", &json!({"command": "cargo test -q -p x"}));
    assert!(r.error && r.texto.contains("es `no_std`") && r.texto.contains("extern crate alloc"), "{}", r.texto);
}

#[test]
fn una_edicion_fuera_de_lo_que_la_tarea_deja_tocar_se_rechaza_con_la_lista_de_permitidos() {
    let task = "# T\n\n## Qué puedes tocar\n\n- `src/lib.rs`\n\nCualquier otra ruta queda fuera.\n\n## Qué tiene que pasar\n\n- `cargo test`\n";
    let mut e = repo().con("/w/TASK.md", task);
    for (n, a) in [
        ("reemplazar_texto", json!({"filePath": "src/otro.rs", "buscar": "fn otro() {}", "nuevo": "fn otro() { 1; }"})),
        ("insertar", json!({"filePath": "tests/roundtrip.rs", "linea": 1, "texto": "fn x() {}"})),
        ("reemplazar_funcion", json!({"filePath": "src/otro.rs", "nombre": "otro", "nuevo": "fn otro() { 2; }"})),
    ] {
        let (t, err) = llama(&mut e, n, a);
        assert!(err && t.contains("fuera de lo que esta tarea deja tocar") && t.contains("`src/lib.rs`"), "{n}: {t}");
    }
    // El archivo permitido sí se edita, y otra herramienta de lectura sobre cualquier ruta no se toca.
    let (t, err) = llama(&mut e, "reemplazar_texto", json!({"filePath": "src/lib.rs", "buscar": "hit();", "nuevo": "hit();\n        hit();"}));
    assert!(!err, "{t}");
    let (_, err) = llama(&mut e, "read", json!({"filePath": "src/otro.rs"}));
    assert!(!err);
    // Sin sección «Qué puedes tocar» no hay restricción.
    let mut libre = repo();
    let (_, err) = llama(&mut libre, "reemplazar_texto", json!({"filePath": "src/otro.rs", "buscar": "fn otro() {}", "nuevo": "fn otro() { 1; }"}));
    assert!(!err);
}

#[test]
fn quitar_o_cambiar_la_firma_de_un_item_publico_se_avisa_antes_de_terminar_y_cambiar_el_cuerpo_no() {
    let fuente = "pub fn a(x: u32) -> u32 {\n    x\n}\n\npub fn b() {\n    hit();\n}\n";
    let mut cfg = cfg_base();
    cfg.proteger_api = true;
    // 1) Cambia el tipo de retorno de `a` y borra `b`: aviso con los dos.
    let mut e = repo().con("/w/src/lib.rs", fuente);
    let mut m = Guion(vec![
        llamada("reemplazar_texto", json!({"filePath": "src/lib.rs", "buscar": "pub fn a(x: u32) -> u32 {", "nuevo": "pub fn a(x: u32) -> Option<u32> {"})),
        llamada("reemplazar_texto", json!({"filePath": "src/lib.rs", "buscar": "pub fn b() {\n    hit();\n}", "nuevo": "fn b() {\n    hit();\n}"})),
        json!({"role": "assistant", "content": "listo"}),
        llamada("bash", json!({"command": "cargo test -q"})),
        json!({"role": "assistant", "content": "listo"}),
        json!({"role": "assistant", "content": "listo otra vez"}),
    ], vec![]);
    let inf = correr(&mut e, &mut m, &RelojSimulado::nuevo(0), &Politica::nueva(R), &cfg, "t", &mut |_, _| {});
    let aviso = inf.mensajes.iter().find(|x| x["role"] == "user" && x["content"].as_str().unwrap_or("").contains("ítems PÚBLICOS")).unwrap_or_else(|| panic!("falta el aviso: {:?}", inf.mensajes.iter().map(|x| x["content"].as_str().unwrap_or("").chars().take(150).collect::<String>()).collect::<Vec<_>>()));
    let t = aviso["content"].as_str().unwrap();
    assert!(t.contains("`pub fn a(x: u32) -> u32`") && t.contains("`pub fn b()`"), "{t}");
    // 2) Cambiar sólo el cuerpo (y añadir un ítem público nuevo) no avisa.
    let mut e = repo().con("/w/src/lib.rs", fuente);
    let mut m = Guion(vec![
        llamada("reemplazar_texto", json!({"filePath": "src/lib.rs", "buscar": "    x\n}", "nuevo": "    x + 1\n}\n\npub fn c() {}"})),
        llamada("bash", json!({"command": "cargo test -q"})),
        json!({"role": "assistant", "content": "listo"}),
    ], vec![]);
    let inf = correr(&mut e, &mut m, &RelojSimulado::nuevo(0), &Politica::nueva(R), &cfg, "t", &mut |_, _| {});
    assert!(!inf.mensajes.iter().any(|x| x["role"] == "user" && x["content"].as_str().unwrap_or("").contains("ítems PÚBLICOS")));
}

#[test]
fn un_modulo_de_pruebas_vacio_no_cuenta_como_editar_y_el_atasco_se_redirige() {
    let mut e = repo();
    let mut guion: Vec<Value> = vec![llamada("insertar", json!({"filePath": "src/lib.rs", "linea": -1, "texto": "#[cfg(test)]\nmod pruebas_agente {\n    // pendiente\n}\n"}))];
    let a = json!({"filePath": "src/lib.rs", "buscar": "no existe uno", "nuevo": "x"});
    let b = json!({"filePath": "src/lib.rs", "buscar": "no existe dos", "nuevo": "y"});
    for _ in 0..30 {
        guion.push(llamada("reemplazar_texto", a.clone()));
        guion.push(llamada("reemplazar_texto", b.clone()));
    }
    guion.push(json!({"role": "assistant", "content": "listo"}));
    let mut cfg = cfg_base();
    cfg.max_turnos = 80;
    let mut m = Guion(guion, vec![]);
    let inf = correr(&mut e, &mut m, &RelojSimulado::nuevo(0), &Politica::nueva(R), &cfg, "t", &mut |_, _| {});
    assert!(inf.redirecciones >= 1, "{} redirecciones", inf.redirecciones);
    assert!(inf.mensajes.iter().any(|x| x["role"] == "user" && x["content"].as_str().unwrap_or("").contains("módulo de pruebas vacío")), "{:?}", inf.mensajes.iter().take(4).collect::<Vec<_>>());
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
    // Sin ninguna edición buena, el atasco se redirige dos veces antes de cerrar el intento.
    assert!(inf.redirecciones == 2 && inf.repetidas > 0, "{} redirecciones, {} repetidas", inf.redirecciones, inf.repetidas);
    assert!(inf.turnos < 75, "{} turnos", inf.turnos);
}

#[test]
fn revisar_enseña_el_diff_propio_y_los_requisitos_de_la_tarea() {
    let mut e = repo().con("/w/TASK.md", "# T\n\n## Qué hay hoy\nalgo\n\n## Qué se pide\n- el menor índice en un empate\n- los NaN nunca se eligen\n\n## Alcance\nsólo src/lib.rs\n");
    let r = ejecutar(&mut e, &Politica::nueva(R), "revisar", &json!({}));
    // `ejecutar` no lleva estado: la revisión la resuelve el bucle del agente.
    assert!(r.error, "{}", r.texto);
    let mut m = Guion(
        vec![
            llamada("reemplazar_texto", json!({"filePath": "src/lib.rs", "buscar": "hit();", "nuevo": "hit();\nmiss();"})),
            llamada("revisar", json!({})),
            llamada("bash", json!({"command": "cargo test"})),
            json!({"role": "assistant", "content": "Listo."}),
        ],
        vec![],
    );
    let inf = correr(&mut e, &mut m, &RelojSimulado::nuevo(0), &Politica::nueva(R), &cfg_base(), "t", &mut |_, _| {});
    assert_eq!(inf.fin, Fin::Terminado, "{:?}", inf.error);
    let rev = inf.mensajes.iter().filter(|x| x["role"] == "tool").map(|x| x["content"].as_str().unwrap().to_string()).find(|c| c.contains("## Tus cambios")).expect("falta la salida de revisar");
    assert!(rev.contains("--- src/lib.rs") && rev.contains("+ miss();") && rev.contains("  hit();"), "{rev}");
    assert!(rev.contains("el menor índice en un empate") && rev.contains("los NaN nunca se eligen") && !rev.contains("sólo src/lib.rs"), "{rev}");
}

#[test]
fn con_revision_el_agente_ve_su_diff_una_sola_vez_antes_de_terminar() {
    let mut e = repo().con("/w/TASK.md", "# T\n\n## Qué se pide\n- cubrir el caso A\n");
    let mut cfg = cfg_base();
    cfg.revision = true;
    let mut m = Guion(
        vec![
            llamada("reemplazar_texto", json!({"filePath": "src/lib.rs", "buscar": "hit();", "nuevo": "hit();\nmiss();"})),
            llamada("bash", json!({"command": "cargo test"})),
            json!({"role": "assistant", "content": "Listo, terminé."}),
            json!({"role": "assistant", "content": "Revisado: todo cubierto."}),
        ],
        vec![],
    );
    let inf = correr(&mut e, &mut m, &RelojSimulado::nuevo(0), &Politica::nueva(R), &cfg, "t", &mut |_, _| {});
    assert_eq!((inf.fin, inf.revisiones), (Fin::Terminado, 1), "{:?}", inf.error);
    assert_eq!(inf.respuesta_final, "Revisado: todo cubierto.");
    let aviso = inf.mensajes.iter().find(|x| x["role"] == "user" && x["content"].as_str().unwrap_or("").contains("revisa lo que has hecho")).expect("falta la revisión");
    let t = aviso["content"].as_str().unwrap();
    assert!(t.contains("+ miss();") && t.contains("cubrir el caso A"), "{t}");
    // Sin ediciones no hay nada que revisar.
    let mut e2 = repo();
    let mut m2 = Guion(vec![json!({"role": "assistant", "content": "nada"})], vec![]);
    let inf2 = correr(&mut e2, &mut m2, &RelojSimulado::nuevo(0), &Politica::nueva(R), &cfg, "t", &mut |_, _| {});
    assert_eq!((inf2.fin, inf2.revisiones), (Fin::Terminado, 0));
}

#[test]
fn reemplazar_lineas_que_se_queda_corto_amplia_el_rango_al_bloque_entero() {
    let src = "fn a() {\n    let x = 1;\n    if x > 0 {\n        f();\n    }\n    g();\n}\n\nfn b() {}\n";
    let mut e = Mem::default().con("/w/src/c.rs", src);
    // La función `a` ocupa 1–7; el modelo reemplaza 1–6 con una función completa: sin ajuste quedaría una `}` de más.
    let (t, err) = llama(&mut e, "reemplazar_lineas", json!({"filePath": "src/c.rs", "desde": 1, "hasta": 6, "nuevo": "fn a() {\n    h();\n}"}));
    assert!(!err && t.contains("AJUSTE") && t.contains("1–7"), "{t}");
    assert_eq!(contenido(&mut e, "/w/src/c.rs"), "fn a() {\n    h();\n}\n\nfn b() {}\n");
    // Con un `nuevo` que de verdad está mal (no equilibrado) no se ajusta nada y se enseña el balance de cada parte.
    let (t, err) = llama(&mut e, "reemplazar_lineas", json!({"filePath": "src/c.rs", "desde": 1, "hasta": 3, "nuevo": "fn a() {\n    h();"}));
    assert!(err && t.contains("balance de tu `nuevo`"), "{t}");
}

#[test]
fn reemplazar_lineas_que_empieza_dentro_de_un_bloque_se_ajusta_hacia_atras() {
    let src = "fn a() {\n    let x = 1;\n    if x > 0 {\n        f();\n    }\n    g();\n}\n";
    let mut e = Mem::default().con("/w/src/c.rs", src);
    // Reemplaza 2–7 (desde dentro de `a` hasta su cierre) con una función entera: sobra una `{` abierta. Se amplía al inicio del bloque.
    let (t, err) = llama(&mut e, "reemplazar_lineas", json!({"filePath": "src/c.rs", "desde": 2, "hasta": 7, "nuevo": "fn a() {\n    h();\n}"}));
    assert!(!err && t.contains("AJUSTE") && t.contains("1–7"), "{t}");
    assert_eq!(contenido(&mut e, "/w/src/c.rs"), "fn a() {\n    h();\n}\n");
}

#[test]
fn el_revisor_independiente_ve_solo_diff_y_requisitos_y_su_critica_vuelve_al_agente() {
    let mut e = repo().con("/w/TASK.md", "# T\n\n## Qué se pide\n- cubrir el caso A\n");
    let mut cfg = cfg_base();
    cfg.revision = true;
    cfg.revisor = true;
    let mut m = Guion(
        vec![
            llamada("reemplazar_texto", json!({"filePath": "src/lib.rs", "buscar": "hit();", "nuevo": "hit();\nmiss();"})),
            llamada("bash", json!({"command": "cargo test"})),
            json!({"role": "assistant", "content": "Listo."}),
            // respuesta del revisor (petición aparte)
            json!({"role": "assistant", "content": "1. Requisito «cubrir el caso A»: con entrada A el diff devuelve B."}),
            json!({"role": "assistant", "content": "Corregido: lo explico."}),
        ],
        vec![],
    );
    let inf = correr(&mut e, &mut m, &RelojSimulado::nuevo(0), &Politica::nueva(R), &cfg, "t", &mut |_, _| {});
    assert_eq!((inf.fin, inf.revisiones), (Fin::Terminado, 1), "{:?}", inf.error);
    assert_eq!(inf.respuesta_final, "Corregido: lo explico.");
    // La petición del revisor no lleva herramientas ni la conversación del agente.
    let pet: Value = serde_json::from_str(m.1.iter().find(|p| p.contains("revisor de código estricto")).expect("falta la petición del revisor")).unwrap();
    assert!(pet.get("tools").is_none() && pet["messages"].as_array().unwrap().len() == 2, "{pet}");
    let u = pet["messages"][1]["content"].as_str().unwrap();
    assert!(u.contains("+ miss();") && u.contains("cubrir el caso A") && !u.contains("Resuelve"), "{u}");
    // La crítica llega al agente como mensaje de usuario.
    assert!(inf.mensajes.iter().any(|x| x["role"] == "user" && x["content"].as_str().unwrap_or("").contains("un revisor independiente") && x["content"].as_str().unwrap().contains("el diff devuelve B")));
}

#[test]
fn si_el_revisor_no_ve_fallos_el_agente_termina_sin_mas_avisos() {
    let mut e = repo().con("/w/TASK.md", "# T\n\n## Qué se pide\n- cubrir el caso A\n");
    let mut cfg = cfg_base();
    cfg.revision = true;
    cfg.revisor = true;
    let mut m = Guion(
        vec![
            llamada("reemplazar_texto", json!({"filePath": "src/lib.rs", "buscar": "hit();", "nuevo": "hit();\nmiss();"})),
            llamada("bash", json!({"command": "cargo test"})),
            json!({"role": "assistant", "content": "Todo hecho."}),
            json!({"role": "assistant", "content": "SIN FALLOS"}),
        ],
        vec![],
    );
    let inf = correr(&mut e, &mut m, &RelojSimulado::nuevo(0), &Politica::nueva(R), &cfg, "t", &mut |_, _| {});
    assert_eq!((inf.fin, inf.revisiones), (Fin::Terminado, 1), "{:?}", inf.error);
    assert_eq!(inf.respuesta_final, "Todo hecho.");
}

fn cfg_equipo() -> Config {
    let mut c = cfg_base();
    c.multiagente = true;
    c.ciclos_max = 3;
    c.max_turnos = 60;
    c
}

#[test]
fn el_equipo_planifica_programa_y_el_revisor_da_el_visto_bueno() {
    let mut e = repo().con("/w/TASK.md", "# T\n\n## Qué se pide\n- cubrir el caso A\n");
    let mut m = Guion(
        vec![
            json!({"role": "assistant", "content": "PLAN: en src/lib.rs añadir miss() tras hit()."}), // planificador
            llamada("reemplazar_texto", json!({"filePath": "src/lib.rs", "buscar": "hit();", "nuevo": "hit();\nmiss();"})), // programador
            llamada("bash", json!({"command": "cargo test"})),
            json!({"role": "assistant", "content": "Hecho."}),
            json!({"role": "assistant", "content": "SIN FALLOS"}), // revisor
        ],
        vec![],
    );
    let inf = correr(&mut e, &mut m, &RelojSimulado::nuevo(0), &Politica::nueva(R), &cfg_equipo(), "Resuelve TASK.md", &mut |_, _| {});
    assert_eq!((inf.fin, inf.ciclos), (Fin::Terminado, 1), "{:?}", inf.error);
    assert_eq!(inf.respuesta_final, "Hecho.");
    assert!(contenido(&mut e, "/w/src/lib.rs").contains("miss();"));
    // El planificador sólo ve herramientas de lectura y su sistema es el suyo.
    let p0: Value = serde_json::from_str(&m.1[0]).unwrap();
    let nombres: Vec<&str> = p0["tools"].as_array().unwrap().iter().map(|t| t["function"]["name"].as_str().unwrap()).collect();
    assert_eq!(nombres, ["read", "grep", "glob"]);
    assert!(p0["messages"][0]["content"].as_str().unwrap().contains("PLANIFICADOR"));
    // El programador recibe el plan en el encargo y sí tiene las herramientas de edición.
    let p1: Value = serde_json::from_str(&m.1[1]).unwrap();
    assert!(p1["messages"][1]["content"].as_str().unwrap().contains("PLAN: en src/lib.rs añadir miss()"));
    assert!(p1["tools"].as_array().unwrap().len() > 3);
}

#[test]
fn si_el_revisor_encuentra_fallos_el_planificador_replantea_y_el_programador_vuelve_a_empezar_limpio() {
    let mut e = repo().con("/w/TASK.md", "# T\n\n## Qué se pide\n- cubrir el caso A\n");
    let mut m = Guion(
        vec![
            json!({"role": "assistant", "content": "PLAN 1"}),
            llamada("reemplazar_texto", json!({"filePath": "src/lib.rs", "buscar": "hit();", "nuevo": "hit();\nmiss();"})),
            llamada("bash", json!({"command": "cargo test"})),
            json!({"role": "assistant", "content": "Hecho 1."}),
            json!({"role": "assistant", "content": "1. Requisito A: con entrada X el diff devuelve Y."}), // revisor: fallos
            json!({"role": "assistant", "content": "PLAN 2: corregir el caso X."}),                        // planificador, ciclo 2
            llamada("reemplazar_texto", json!({"filePath": "src/lib.rs", "buscar": "miss();", "nuevo": "miss_x();"})),
            llamada("bash", json!({"command": "cargo test"})),
            json!({"role": "assistant", "content": "Hecho 2."}),
            json!({"role": "assistant", "content": "SIN FALLOS"}),
        ],
        vec![],
    );
    let inf = correr(&mut e, &mut m, &RelojSimulado::nuevo(0), &Politica::nueva(R), &cfg_equipo(), "Resuelve TASK.md", &mut |_, _| {});
    assert_eq!((inf.fin, inf.ciclos), (Fin::Terminado, 2), "{:?}", inf.error);
    assert_eq!(inf.respuesta_final, "Hecho 2.");
    assert!(contenido(&mut e, "/w/src/lib.rs").contains("miss_x();"));
    // El planificador del ciclo 2 recibe el diff acumulado y los problemas del revisor.
    let p: Value = serde_json::from_str(&m.1[5]).unwrap();
    let u = p["messages"][1]["content"].as_str().unwrap();
    assert!(u.contains("+ miss();") && u.contains("con entrada X el diff devuelve Y"), "{u}");
    // El programador del ciclo 2 arranca con una conversación nueva (sistema + encargo) con el plan 2 y la nota de corrección.
    let g: Value = serde_json::from_str(&m.1[6]).unwrap();
    assert_eq!(g["messages"].as_array().unwrap().len(), 2);
    let enc = g["messages"][1]["content"].as_str().unwrap();
    assert!(enc.contains("PLAN 2") && enc.contains("CICLO DE CORRECCIÓN"), "{enc}");
}

#[test]
fn si_la_comprobacion_del_programador_falla_se_replantea_sin_pasar_por_el_revisor() {
    struct Rojo2(Mem, std::cell::Cell<u32>);
    impl Archivos for Rojo2 {
        fn leer(&self, r: &str) -> Resultado<Vec<u8>> { self.0.leer(r) }
        fn escribir(&mut self, r: &str, d: &[u8], m: u32) -> Resultado<()> { self.0.escribir(r, d, m) }
        fn existe(&self, r: &str) -> bool { self.0.existe(r) }
        fn listar(&self, r: &str) -> Resultado<Vec<Entrada>> { self.0.listar(r) }
        fn metadatos(&self, r: &str) -> Resultado<Entrada> { self.0.metadatos(r) }
        fn crear_directorio(&mut self, r: &str) -> Resultado<()> { self.0.crear_directorio(r) }
        fn borrar(&mut self, r: &str) -> Resultado<()> { self.0.borrar(r) }
    }
    impl Procesos for Rojo2 {
        fn ejecutar(&self, _: &Orden) -> Resultado<Salida> {
            // La primera comprobación falla; la segunda pasa.
            let n = self.1.get();
            self.1.set(n + 1);
            let (codigo, t) = if n == 0 { (101, "error[E0308]: mismatched types") } else { (0, "test result: ok. 1 passed") };
            Ok(Salida { codigo: Some(codigo), stdout: t.as_bytes().to_vec(), stderr: vec![], motivo: None })
        }
    }
    let mut e = Rojo2(repo().con("/w/TASK.md", "# T\n\n## Qué se pide\n- cubrir el caso A\n"), std::cell::Cell::new(0));
    let mut m = Guion(
        vec![
            json!({"role": "assistant", "content": "PLAN 1"}),
            llamada("reemplazar_texto", json!({"filePath": "src/lib.rs", "buscar": "hit();", "nuevo": "hit();\nmiss();"})),
            llamada("bash", json!({"command": "cargo test"})),
            json!({"role": "assistant", "content": "Hecho 1."}),
            // no hay llamada al revisor: la comprobación roja basta para replantear
            json!({"role": "assistant", "content": "PLAN 2: arreglar el tipo."}),
            llamada("bash", json!({"command": "cargo test"})),
            json!({"role": "assistant", "content": "Hecho 2."}),
            json!({"role": "assistant", "content": "SIN FALLOS"}),
        ],
        vec![],
    );
    // Sin los empujones propios del programador, para ver la decisión del orquestador ante una comprobación roja.
    let mut cfg = cfg_equipo();
    cfg.empujones = 0;
    let inf = correr(&mut e, &mut m, &RelojSimulado::nuevo(0), &Politica::nueva(R), &cfg, "Resuelve TASK.md", &mut |_, _| {});
    assert_eq!((inf.fin, inf.ciclos), (Fin::Terminado, 2), "{:?}", inf.error);
    let p: Value = serde_json::from_str(&m.1[4]).unwrap();
    assert!(p["messages"][1]["content"].as_str().unwrap().contains("comprobación del programador FALLA"), "{}", p["messages"][1]["content"]);
}

#[test]
fn alternar_una_edicion_entre_dos_versiones_se_avisa_y_acaba_cerrando_el_intento() {
    let mut e = repo();
    let mut guion: Vec<Value> = Vec::new();
    for _ in 0..8 {
        guion.push(llamada("reemplazar_texto", json!({"filePath": "src/lib.rs", "buscar": "hit();", "nuevo": "hot();"})));
        guion.push(llamada("reemplazar_texto", json!({"filePath": "src/lib.rs", "buscar": "hot();", "nuevo": "hit();"})));
    }
    guion.push(json!({"role": "assistant", "content": "listo"}));
    let mut cfg = cfg_base();
    cfg.max_turnos = 60;
    let mut m = Guion(guion, vec![]);
    let inf = correr(&mut e, &mut m, &RelojSimulado::nuevo(0), &Politica::nueva(R), &cfg, "t", &mut |_, _| {});
    assert_eq!(inf.fin, Fin::Terminado, "{:?}", inf.error);
    assert!(inf.respuesta_final.contains("intento cerrado"), "{}", inf.respuesta_final);
    assert!(inf.turnos < 16, "{} turnos", inf.turnos);
    assert!(inf.mensajes.iter().any(|x| x["role"] == "tool" && x["content"].as_str().unwrap_or("").contains("devuelve el archivo a un estado en el que ya estuvo")));
}

const MOD_RS: &str = "use x;\n\n/// Suma.\n#[inline]\npub fn suma(a: u32, b: u32) -> u32 {\n    a + b\n}\n\nstruct P {\n    x: u32,\n}\n\nimpl P {\n    fn nuevo() -> Self {\n        P { x: 0 }\n    }\n\n    fn doble(&self) -> u32 {\n        self.x * 2\n    }\n}\n\nfn repetida() {}\nmod interno {\n    pub fn repetida() {}\n}\n";

#[test]
fn read_esquema_y_simbolo_enseñan_la_estructura_con_rangos_exactos() {
    let mut e = Mem::default().con("/w/src/m.rs", MOD_RS);
    let (t, err) = llama(&mut e, "read", json!({"filePath": "src/m.rs", "esquema": true}));
    assert!(!err && t.contains("5–7  pub fn suma") && t.contains("9–11  struct P") && t.contains("13–21  impl P") && t.contains("14–16  fn nuevo"), "{t}");
    let (t, err) = llama(&mut e, "read", json!({"filePath": "src/m.rs", "simbolo": "doble"}));
    assert!(!err && t.contains("18: ") && t.contains("self.x * 2") && t.contains("(líneas 18–20)"), "{t}");
    let (t, err) = llama(&mut e, "read", json!({"filePath": "src/m.rs", "simbolo": "dobles"}));
    assert!(err && t.contains("`doble`"), "{t}");
    let (t, err) = llama(&mut e, "read", json!({"filePath": "src/m.rs", "simbolo": "repetida"}));
    assert!(err && t.contains("2 veces"), "{t}");
}

#[test]
fn reemplazar_funcion_sustituye_por_nombre_y_no_duplica_atributos() {
    let mut e = Mem::default().con("/w/src/m.rs", MOD_RS);
    // El modelo repite el atributo y la documentación que ya estaban encima.
    let (t, err) = llama(&mut e, "reemplazar_funcion", json!({"filePath": "src/m.rs", "nombre": "suma", "nuevo": "/// Suma.\n#[inline]\npub fn suma(a: u32, b: u32) -> u32 {\n    a.saturating_add(b)\n}"}));
    assert!(!err && t.starts_with("Hecho") && t.contains("Era fn «suma»"), "{t}");
    let c = contenido(&mut e, "/w/src/m.rs");
    assert_eq!(c.matches("#[inline]").count(), 1, "{c}");
    assert!(c.contains("a.saturating_add(b)") && !c.contains("a + b"), "{c}");
    // Un método dentro de un impl también se encuentra por nombre.
    let (t, err) = llama(&mut e, "reemplazar_funcion", json!({"filePath": "src/m.rs", "nombre": "doble", "nuevo": "    fn doble(&self) -> u32 {\n        self.x.wrapping_mul(2)\n    }"}));
    assert!(!err, "{t}");
    assert!(contenido(&mut e, "/w/src/m.rs").contains("wrapping_mul(2)"));
    // Nombre repetido: pide `linea`; con `linea` elige el que la contiene.
    let (t, err) = llama(&mut e, "reemplazar_funcion", json!({"filePath": "src/m.rs", "nombre": "repetida", "nuevo": "fn repetida() { 1; }"}));
    assert!(err && t.contains("2 veces"), "{t}");
    let l = contenido(&mut e, "/w/src/m.rs").lines().position(|x| x.starts_with("fn repetida")).unwrap() + 1;
    let (t, err) = llama(&mut e, "reemplazar_funcion", json!({"filePath": "src/m.rs", "nombre": "repetida", "linea": l, "nuevo": "fn repetida() { 1; }"}));
    assert!(!err, "{t}");
    // Un nombre que no existe sugiere los parecidos.
    let (t, err) = llama(&mut e, "reemplazar_funcion", json!({"filePath": "src/m.rs", "nombre": "sumar", "nuevo": "fn sumar() {}"}));
    assert!(err && t.contains("`suma`"), "{t}");
}

#[test]
fn reemplazar_texto_e_insertar_enseñan_como_queda_el_tramo_con_su_numeracion() {
    let mut e = repo();
    let (t, err) = llama(&mut e, "reemplazar_texto", json!({"filePath": "src/lib.rs", "buscar": "hit();", "nuevo": "hit();\nmiss();"}));
    assert!(!err && t.contains("Así queda ahora") && t.contains("4: miss();"), "{t}");
    let (t, err) = llama(&mut e, "insertar", json!({"filePath": "src/lib.rs", "linea": 1, "texto": "// nota"}));
    assert!(!err && t.contains("Así queda ahora") && t.contains("2: // nota"), "{t}");
}

#[test]
fn el_esquema_acierta_con_archivos_reales_del_repo() {
    // Para cada función/tipo que el esquema encuentra, reemplazarlo por su propio texto debe ser «no cambia nada»: el rango es exacto.
    let raiz = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut total = 0;
    let mut malos: Vec<String> = Vec::new();
    for rel in ["crates/sosofs/src/fs.rs", "crates/sosomfs/src/catalog.rs", "crates/soso-llm-core/src/sample.rs", "crates/soso-improve-core/src/agente/revision.rs", "crates/gguf2som/src/lib.rs"] {
        let Ok(src) = std::fs::read_to_string(raiz.join(rel)) else { continue };
        let ruta = format!("/w/{rel}");
        let mut e = Mem::default().con(&ruta, &src);
        let (t, err) = llama(&mut e, "read", json!({"filePath": rel, "esquema": true}));
        assert!(!err, "{rel}: {t}");
        let lineas: Vec<&str> = src.lines().collect();
        for l in t.lines() {
            let l = l.trim_start();
            let Some((rango, _)) = l.split_once("  ") else { continue };
            let Some((a, b)) = rango.split_once('–') else { continue };
            let (Ok(a), Ok(b)) = (a.parse::<usize>(), b.parse::<usize>()) else { continue };
            total += 1;
            if a < 1 || b > lineas.len() || b < a {
                malos.push(format!("{rel}: rango fuera de archivo {a}–{b}"));
                continue;
            }
            let texto = lineas[a - 1..b].join("\n");
            let (r, _) = llama(&mut e, "reemplazar_lineas", json!({"filePath": rel, "desde": a, "hasta": b, "nuevo": texto}));
            if !r.contains("no cambia nada") {
                malos.push(format!("{rel} {a}–{b}: {}", r.lines().next().unwrap_or("")));
            }
        }
    }
    assert!(total > 50, "el esquema encontró sólo {total} elementos");
    assert!(malos.is_empty(), "{} de {total} rangos inexactos: {:?}", malos.len(), &malos[..malos.len().min(8)]);
}

/// Entorno en memoria cuyo `ejecutar` es el de un `rustfmt` de verdad (si existe en el PATH), alimentado por la entrada estándar.
struct ConRustfmt(Mem);
impl Archivos for ConRustfmt {
    fn leer(&self, r: &str) -> Resultado<Vec<u8>> { self.0.leer(r) }
    fn escribir(&mut self, r: &str, d: &[u8], m: u32) -> Resultado<()> { self.0.escribir(r, d, m) }
    fn existe(&self, r: &str) -> bool { self.0.existe(r) }
    fn listar(&self, r: &str) -> Resultado<Vec<Entrada>> { self.0.listar(r) }
    fn metadatos(&self, r: &str) -> Resultado<Entrada> { self.0.metadatos(r) }
    fn crear_directorio(&mut self, r: &str) -> Resultado<()> { self.0.crear_directorio(r) }
    fn borrar(&mut self, r: &str) -> Resultado<()> { self.0.borrar(r) }
}
impl Procesos for ConRustfmt {
    fn ejecutar(&self, o: &Orden) -> Resultado<Salida> {
        use std::io::Write;
        let mut hijo = std::process::Command::new(&o.argv[0])
            .args(&o.argv[1..])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| Error::entorno(format!("no pude lanzar {}: {e}", o.argv[0])))?;
        hijo.stdin.take().unwrap().write_all(&o.stdin).map_err(|e| Error::entorno(e.to_string()))?;
        let s = hijo.wait_with_output().map_err(|e| Error::entorno(e.to_string()))?;
        Ok(Salida { codigo: s.status.code(), stdout: s.stdout, stderr: s.stderr, motivo: None })
    }
}

fn hay_rustfmt() -> bool {
    std::process::Command::new("rustfmt").arg("--version").output().map(|o| o.status.success()).unwrap_or(false)
}

#[test]
fn una_edicion_que_rompe_la_sintaxis_se_rechaza_con_el_error_de_rustfmt_y_no_se_escribe() {
    if !hay_rustfmt() {
        eprintln!("rustfmt no está en el PATH: se salta");
        return;
    }
    let src = "fn a() {\n    let x = 1;\n    let y = 2;\n}\n";
    let mut e = ConRustfmt(Mem::default().con("/w/src/s.rs", src).con("/w/NOTAS.md", "hola\n"));
    let mut pol = Politica::nueva(R);
    pol.comprobar_sintaxis = Some(soso_improve_core::agente::herramientas::orden_sintaxis_rustfmt());
    let llama2 = |e: &mut ConRustfmt, n: &str, a: Value| { let r = ejecutar(e, &pol, n, &a); (r.texto, r.error) };
    // Falta el `;`: el recuento de llaves no lo ve, rustfmt sí.
    let (t, err) = llama2(&mut e, "reemplazar_texto", json!({"filePath": "src/s.rs", "buscar": "let x = 1;", "nuevo": "let x = 1"}));
    assert!(err && t.contains("error de SINTAXIS") && t.contains("expected `;`") && t.contains("alrededor de la línea") && t.contains("3:     let y = 2;"), "{t}");
    assert_eq!(e.0.f["/w/src/s.rs"], src.as_bytes(), "el archivo no cambia");
    let (t, err) = llama2(&mut e, "reemplazar_lineas", json!({"filePath": "src/s.rs", "desde": 2, "hasta": 2, "nuevo": "    let x = ;"}));
    assert!(err && t.contains("error de SINTAXIS"), "{t}");
    let (t, err) = llama2(&mut e, "insertar", json!({"filePath": "src/s.rs", "linea": 3, "texto": "    let z = (1 +;"}));
    assert!(err && (t.contains("SINTAXIS") || t.contains("desequilibrados")), "{t}");
    // Un cambio válido pasa aunque esté mal formateado (eso es un `Diff`, no un `error:`).
    let (t, err) = llama2(&mut e, "reemplazar_texto", json!({"filePath": "src/s.rs", "buscar": "let x = 1;", "nuevo": "let   x=2 ;"}));
    assert!(!err, "{t}");
    // Un archivo que no es Rust no se comprueba.
    let (_, err) = llama2(&mut e, "reemplazar_texto", json!({"filePath": "NOTAS.md", "buscar": "hola", "nuevo": "fn (((("}));
    assert!(!err);
}

#[test]
fn si_el_archivo_original_ya_estaba_roto_no_se_culpa_a_la_edicion() {
    if !hay_rustfmt() {
        return;
    }
    let roto = "fn a() {\n    let x = 1\n    let y = 2;\n}\n";
    let mut e = ConRustfmt(Mem::default().con("/w/src/r.rs", roto));
    let mut pol = Politica::nueva(R);
    pol.comprobar_sintaxis = Some(soso_improve_core::agente::herramientas::orden_sintaxis_rustfmt());
    let r = ejecutar(&mut e, &pol, "reemplazar_texto", &json!({"filePath": "src/r.rs", "buscar": "let y = 2;", "nuevo": "let y = 3;"}));
    assert!(!r.error, "{}", r.texto);
}

#[test]
fn sin_comprobador_o_sin_el_programa_las_ediciones_siguen_funcionando() {
    // Sin `comprobar_sintaxis` (valor por defecto) y con un programa que no existe: no se comprueba y no falla.
    let mut e = ConRustfmt(Mem::default().con("/w/src/s.rs", "fn a() {\n    let x = 1;\n}\n"));
    let pol = Politica::nueva(R);
    let r = ejecutar(&mut e, &pol, "reemplazar_texto", &json!({"filePath": "src/s.rs", "buscar": "let x = 1;", "nuevo": "let x = 1"}));
    assert!(!r.error, "{}", r.texto);
    let mut e = ConRustfmt(Mem::default().con("/w/src/s.rs", "fn a() {\n    let x = 1;\n}\n"));
    let mut pol = Politica::nueva(R);
    pol.comprobar_sintaxis = Some(vec!["programa-que-no-existe-xyz".to_string()]);
    let r = ejecutar(&mut e, &pol, "reemplazar_texto", &json!({"filePath": "src/s.rs", "buscar": "let x = 1;", "nuevo": "let x = 1"}));
    assert!(!r.error, "{}", r.texto);
}

#[test]
fn con_pruebas_exigidas_hay_que_ejecutarlas_en_verde_tras_el_ultimo_cambio() {
    let mut e = repo();
    let mut cfg = cfg_base();
    cfg.exigir_pruebas = true;
    let prueba = "#[cfg(test)]\nmod pruebas_agente {\n    #[test]\n    fn caso_a() {}\n}";
    let mut m = Guion(
        vec![
            llamada("insertar", json!({"filePath": "src/lib.rs", "linea": -1, "texto": prueba})),
            llamada("bash", json!({"command": "cargo test -p x"})), // sin el filtro: no demuestra que SUS pruebas pasen
            json!({"role": "assistant", "content": "Terminé."}),
            // el aviso las manda ejecutar con el filtro
            llamada("bash", json!({"command": "cargo test -p x -- pruebas_agente"})),
            json!({"role": "assistant", "content": "Ahora sí."}),
        ],
        vec![],
    );
    let inf = correr(&mut e, &mut m, &RelojSimulado::nuevo(0), &Politica::nueva(R), &cfg, "t", &mut |_, _| {});
    assert_eq!((inf.fin, inf.sin_ejecutar), (Fin::Terminado, 1), "{:?}", inf.error);
    assert_eq!(inf.respuesta_final, "Ahora sí.");
    assert!(inf.mensajes.iter().any(|x| x["role"] == "user" && x["content"].as_str().unwrap_or("").contains("no las has ejecutado en verde")));
}

#[test]
fn un_segundo_modulo_de_pruebas_se_rechaza_y_dice_donde_añadir_las_pruebas() {
    let src = "fn f() {}\n\n#[cfg(test)]\nmod pruebas_agente {\n    #[test]\n    fn a() {}\n}\n";
    let mut e = Mem::default().con("/w/src/p.rs", src);
    let nuevo = "#[cfg(test)]\nmod pruebas_agente {\n    #[test]\n    fn b() {}\n}";
    let (t, err) = llama(&mut e, "insertar", json!({"filePath": "src/p.rs", "linea": -1, "texto": nuevo}));
    assert!(err && t.contains("DOS `mod pruebas_agente`") && t.contains("reemplazar_funcion"), "{t}");
    assert_eq!(e.f["/w/src/p.rs"], src.as_bytes());
    // Con `reemplazar_funcion` por su nombre sí se sustituye el módulo existente.
    let (t, err) = llama(&mut e, "reemplazar_funcion", json!({"filePath": "src/p.rs", "nombre": "pruebas_agente", "nuevo": "mod pruebas_agente {\n    #[test]\n    fn b() {}\n}"}));
    assert!(!err, "{t}");
}

#[test]
fn una_edicion_que_se_lleva_por_delante_codigo_ajeno_pide_confirmacion() {
    let src = "fn parse() {\n    let a = 1;\n    let alineamiento = calcular();\n    let desplazamiento = alinear(alineamiento);\n    let usado = desplazamiento + a;\n    guardar(usado);\n    registrar(usado);\n}\n";
    let mut e = Mem::default().con("/w/src/p.rs", src);
    // Pretende añadir una comprobación y sustituye 6 líneas por 2: borra el cálculo del alineamiento.
    let (t, err) = llama(&mut e, "reemplazar_lineas", json!({"filePath": "src/p.rs", "desde": 2, "hasta": 7, "nuevo": "    comprobar();\n    guardar(1);"}));
    assert!(err && t.contains("BORRARÍA") && t.contains("alineamiento") && t.contains("confirmar_borrado"), "{t}");
    assert_eq!(e.f["/w/src/p.rs"], src.as_bytes());
    // Confirmado: se aplica.
    let (t, err) = llama(&mut e, "reemplazar_lineas", json!({"filePath": "src/p.rs", "desde": 2, "hasta": 7, "nuevo": "    comprobar();\n    guardar(1);", "confirmar_borrado": true}));
    assert!(!err, "{t}");
    // Reescribir una función entera con `reemplazar_funcion` no lo pide (es lo que se pretende).
    let mut e = Mem::default().con("/w/src/p.rs", src);
    let (t, err) = llama(&mut e, "reemplazar_funcion", json!({"filePath": "src/p.rs", "nombre": "parse", "nuevo": "fn parse() {\n    nuevo();\n}"}));
    assert!(!err, "{t}");
    // Cambiar líneas conservando las demás no molesta.
    let mut e = Mem::default().con("/w/src/p.rs", src);
    let (_, err) = llama(&mut e, "reemplazar_texto", json!({"filePath": "src/p.rs", "buscar": "let a = 1;", "nuevo": "let a = 2;"}));
    assert!(!err);
}

#[test]
fn al_terminar_el_arnes_ejecuta_la_comprobacion_de_la_tarea_y_si_falla_se_la_devuelve() {
    struct Rojo3(Mem, std::cell::Cell<u32>);
    impl Archivos for Rojo3 {
        fn leer(&self, r: &str) -> Resultado<Vec<u8>> { self.0.leer(r) }
        fn escribir(&mut self, r: &str, d: &[u8], m: u32) -> Resultado<()> { self.0.escribir(r, d, m) }
        fn existe(&self, r: &str) -> bool { self.0.existe(r) }
        fn listar(&self, r: &str) -> Resultado<Vec<Entrada>> { self.0.listar(r) }
        fn metadatos(&self, r: &str) -> Resultado<Entrada> { self.0.metadatos(r) }
        fn crear_directorio(&mut self, r: &str) -> Resultado<()> { self.0.crear_directorio(r) }
        fn borrar(&mut self, r: &str) -> Resultado<()> { self.0.borrar(r) }
    }
    impl Procesos for Rojo3 {
        fn ejecutar(&self, _: &Orden) -> Resultado<Salida> {
            // La comprobación falla la primera vez (no compila) y pasa la segunda.
            let n = self.1.get();
            self.1.set(n + 1);
            let (c, t) = if n == 0 { (101, "error[E0425]: cannot find function `decompress_gzip` in this scope") } else { (0, "test result: ok. 3 passed") };
            Ok(Salida { codigo: Some(c), stdout: t.as_bytes().to_vec(), stderr: vec![], motivo: None })
        }
    }
    let task = "# T\n\n## Qué se pide\n- algo\n\n## Qué tiene que pasar\n\n- `cargo test -q -p x --features std` (en `.`, máximo 900 s)\n";
    let mut e = Rojo3(repo().con("/w/TASK.md", task), std::cell::Cell::new(0));
    let mut cfg = cfg_base();
    cfg.comprobar_al_terminar = true;
    cfg.empujones = 0;
    let mut m = Guion(
        vec![
            llamada("reemplazar_texto", json!({"filePath": "src/lib.rs", "buscar": "hit();", "nuevo": "hit();\nmiss();"})),
            json!({"role": "assistant", "content": "Listo (sin haber ejecutado nada)."}),
            // el arnés ejecuta la comprobación, falla y se la devuelve: el modelo corrige y vuelve a terminar
            llamada("reemplazar_texto", json!({"filePath": "src/lib.rs", "buscar": "miss();", "nuevo": "miss_bien();"})),
            json!({"role": "assistant", "content": "Corregido."}),
        ],
        vec![],
    );
    let inf = correr(&mut e, &mut m, &RelojSimulado::nuevo(0), &Politica::nueva(R), &cfg, "t", &mut |_, _| {});
    assert_eq!((inf.fin, inf.comprobaciones_rojas), (Fin::Terminado, 1), "{:?}", inf.error);
    assert_eq!(inf.respuesta_final, "Corregido.");
    let aviso = inf.mensajes.iter().find(|x| x["role"] == "user" && x["content"].as_str().unwrap_or("").contains("he ejecutado yo la comprobación")).expect("falta el aviso");
    let t = aviso["content"].as_str().unwrap();
    assert!(t.contains("$ cargo test -q -p x --features std") && t.contains("decompress_gzip"), "{t}");
}

#[test]
fn un_cambio_que_es_casi_todo_comentarios_se_rechaza() {
    let mut e = repo();
    let coment: String = (0..9).map(|i| format!("        // Wait, {i}: how do we know the parent?\n")).collect();
    let (t, err) = llama(&mut e, "reemplazar_texto", json!({"filePath": "src/lib.rs", "buscar": "hit();", "nuevo": format!("{coment}        hit();")}));
    assert!(err && t.contains("son comentarios") && t.contains("no implementa nada"), "{t}");
    // Un comentario normal no molesta.
    let (_, err) = llama(&mut e, "reemplazar_texto", json!({"filePath": "src/lib.rs", "buscar": "hit();", "nuevo": "// aviso\nhit();"}));
    assert!(!err);
}

#[test]
fn un_monologo_de_dudas_entre_codigo_se_rechaza() {
    let mut e = repo();
    let nuevo = "        // Espera, ¿cómo sabemos el padre?\n        // Si no hay parent, ¿qué hacemos?\n        // Revisemos stat_inode.\n        // Quizás lookup lo permita?\n        let a = 1;\n        let b = 2;\n        let c = 3;\n        let d = 4;\n        hit();";
    let (t, err) = llama(&mut e, "reemplazar_texto", json!({"filePath": "src/lib.rs", "buscar": "hit();", "nuevo": nuevo}));
    assert!(err && t.contains("dudas o monólogo"), "{t}");
}

/// Entorno que contesta a `cargo clippy` con avisos en formato corto y a lo demás con una comprobación verde.
struct ConClippy(Mem);
impl Archivos for ConClippy {
    fn leer(&self, r: &str) -> Resultado<Vec<u8>> { self.0.leer(r) }
    fn escribir(&mut self, r: &str, d: &[u8], m: u32) -> Resultado<()> { self.0.escribir(r, d, m) }
    fn existe(&self, r: &str) -> bool { self.0.existe(r) }
    fn listar(&self, r: &str) -> Resultado<Vec<Entrada>> { self.0.listar(r) }
    fn metadatos(&self, r: &str) -> Resultado<Entrada> { self.0.metadatos(r) }
    fn crear_directorio(&mut self, r: &str) -> Resultado<()> { self.0.crear_directorio(r) }
    fn borrar(&mut self, r: &str) -> Resultado<()> { self.0.borrar(r) }
}
impl Procesos for ConClippy {
    fn ejecutar(&self, o: &Orden) -> Resultado<Salida> {
        self.0.ordenes.borrow_mut().push(o.argv.clone());
        let t = if o.argv.iter().any(|a| a == "clippy") {
            "src/lib.rs:4:17: warning: indexing may panic\nsrc/lib.rs:2:8: warning: arithmetic operation that can potentially result in unexpected side-effects\nsrc/lib.rs:9:5: warning: slicing may panic\nsrc/otro.rs:1:1: warning: indexing may panic\n"
        } else {
            "test result: ok. 3 passed"
        };
        Ok(Salida { codigo: Some(0), stdout: t.as_bytes().to_vec(), stderr: vec![], motivo: None })
    }
}

#[test]
fn si_la_tarea_pide_no_entrar_en_panico_clippy_marca_las_lineas_del_agente_y_solo_esas() {
    let task = "# T\n\n## Qué se pide\n- que no entre en pánico con datos truncados\n\n## Qué tiene que pasar\n\n- `cargo test -q -p x --features std` (en `.`, máximo 900 s)\n";
    let dos = "fn buscar(k: &str) {\n    if k == key {\n        hit();\n    }\n}\n\nfn otra(d: &[u8]) {\n    let a = 1;\n    let b = &d[..a];\n}\n";
    let mut e = ConClippy(repo().con("/w/src/lib.rs", dos).con("/w/TASK.md", task));
    let mut cfg = cfg_base();
    cfg.alertas_panico = true;
    cfg.avisos_panico = 1;
    cfg.empujones = 0;
    let mut m = Guion(
        vec![
            llamada("reemplazar_texto", json!({"filePath": "src/lib.rs", "buscar": "hit();", "nuevo": "hit();\nlet z = d[0];"})),
            llamada("bash", json!({"command": "cargo test -q -p x --features std"})),
            json!({"role": "assistant", "content": "Listo."}),
            llamada("reemplazar_texto", json!({"filePath": "src/lib.rs", "buscar": "let z = d[0];", "nuevo": "let z = d.first();"})),
            json!({"role": "assistant", "content": "Corregido."}),
        ],
        vec![],
    );
    let inf = correr(&mut e, &mut m, &RelojSimulado::nuevo(0), &Politica::nueva(R), &cfg, "t", &mut |_, _| {});
    assert_eq!((inf.fin, inf.alertas_panico), (Fin::Terminado, 1), "{:?}", inf.error);
    let aviso = inf.mensajes.iter().find(|x| x["role"] == "user" && x["content"].as_str().unwrap_or("").contains("Clippy")).expect("falta el aviso de Clippy");
    let t = aviso["content"].as_str().unwrap();
    assert!(t.contains("src/lib.rs:4") && t.contains("indexing may panic"), "{t}");
    // Vale toda la función que tocó (la línea 2 está en ella aunque no la cambiara), no otras funciones ni otros archivos.
    assert!(t.contains("src/lib.rs:2") && !t.contains("src/lib.rs:9") && !t.contains("otro.rs"), "{t}");
    // La orden construida lleva el crate y las features de la comprobación de la tarea y los dos lints.
    let o = e.0.ordenes.borrow().iter().find(|a| a.iter().any(|x| x == "clippy")).cloned().unwrap();
    assert!(o.join(" ").contains("-p x --features std") && o.join(" ").contains("clippy::indexing_slicing") && o.join(" ").contains("clippy::arithmetic_side_effects"), "{o:?}");
}

#[test]
fn si_la_tarea_no_habla_de_panico_no_se_pasa_clippy() {
    let task = "# T\n\n## Qué se pide\n- que ordene bien\n\n## Qué tiene que pasar\n\n- `cargo test -q -p x`\n";
    let mut e = ConClippy(repo().con("/w/TASK.md", task));
    let mut cfg = cfg_base();
    cfg.alertas_panico = true;
    cfg.empujones = 0;
    let mut m = Guion(
        vec![
            llamada("reemplazar_texto", json!({"filePath": "src/lib.rs", "buscar": "hit();", "nuevo": "hit();\nlet z = d[0];"})),
            llamada("bash", json!({"command": "cargo test -q -p x"})),
            json!({"role": "assistant", "content": "Listo."}),
        ],
        vec![],
    );
    let inf = correr(&mut e, &mut m, &RelojSimulado::nuevo(0), &Politica::nueva(R), &cfg, "t", &mut |_, _| {});
    assert_eq!((inf.fin, inf.alertas_panico), (Fin::Terminado, 0));
    assert!(!e.0.ordenes.borrow().iter().any(|a| a.iter().any(|x| x == "clippy")));
}

#[test]
fn con_pruebas_exigidas_menos_pruebas_que_puntos_pide_las_que_faltan() {
    let task = "# T\n\n## Qué se pide\n- punto uno\n- punto dos\n- punto tres\n\n## Alcance\nnada\n";
    let mut e = repo().con("/w/TASK.md", task);
    let mut cfg = cfg_base();
    cfg.exigir_pruebas = true;
    cfg.empujones = 0;
    let una = "#[cfg(test)]\nmod pruebas_agente {\n    #[test]\n    fn caso_a() {}\n}";
    let mut m = Guion(
        vec![
            llamada("insertar", json!({"filePath": "src/lib.rs", "linea": -1, "texto": una})),
            llamada("bash", json!({"command": "cargo test -p x -- pruebas_agente"})),
            json!({"role": "assistant", "content": "Listo."}),
            json!({"role": "assistant", "content": "Sigo igual."}),
        ],
        vec![],
    );
    let inf = correr(&mut e, &mut m, &RelojSimulado::nuevo(0), &Politica::nueva(R), &cfg, "t", &mut |_, _| {});
    assert_eq!((inf.fin, inf.pocas_pruebas), (Fin::Terminado, 1), "{:?}", inf.error);
    assert!(inf.mensajes.iter().any(|x| x["role"] == "user" && x["content"].as_str().unwrap_or("").contains("tiene 3 puntos y has escrito 1")));
}

#[test]
fn releer_lo_mismo_sin_editar_redirige_a_escribir_en_vez_de_cerrar_el_intento() {
    let mut e = repo();
    let mut guion: Vec<Value> = (0..6).map(|_| llamada("read", json!({"filePath": "src/lib.rs"}))).collect();
    // Tras la redirección el modelo por fin edita y termina.
    guion.push(llamada("reemplazar_texto", json!({"filePath": "src/lib.rs", "buscar": "hit();", "nuevo": "hit();\nmiss();"})));
    guion.push(llamada("bash", json!({"command": "cargo test"})));
    guion.push(json!({"role": "assistant", "content": "Hecho."}));
    let mut cfg = cfg_base();
    cfg.max_turnos = 60;
    let mut m = Guion(guion, vec![]);
    let inf = correr(&mut e, &mut m, &RelojSimulado::nuevo(0), &Politica::nueva(R), &cfg, "t", &mut |_, _| {});
    assert_eq!((inf.fin, inf.redirecciones), (Fin::Terminado, 1), "{:?}", inf.error);
    assert_eq!(inf.respuesta_final, "Hecho.");
    assert!(contenido(&mut e, "/w/src/lib.rs").contains("miss();"));
    assert!(inf.mensajes.iter().any(|x| x["role"] == "user" && x["content"].as_str().unwrap_or("").contains("NINGUNA edición")));
}

#[test]
fn read_simbolo_acepta_tipo_metodo_y_el_ambiguo_trae_la_llamada_exacta() {
    let mut e = Mem::default().con("/w/src/m.rs", MOD_RS);
    let (t, err) = llama(&mut e, "read", json!({"filePath": "src/m.rs", "simbolo": "P::doble"}));
    assert!(!err && t.contains("self.x * 2") && t.contains("(líneas 18–20)"), "{t}");
    let (t, err) = llama(&mut e, "read", json!({"filePath": "src/m.rs", "simbolo": "P"}));
    assert!(err && t.contains("read offset=9 limit=3") && t.contains("read offset=13 limit=9"), "{t}");
    // `simbolo: true` (un booleano) no rompe el read normal.
    let (t, err) = llama(&mut e, "read", json!({"filePath": "src/m.rs", "simbolo": true, "offset": 5}));
    assert!(!err && t.contains("5: pub fn suma"), "{t}");
}

#[test]
fn las_respuestas_mal_formadas_seguidas_suben_la_temperatura_y_piden_llamadas_mas_cortas() {
    struct Mal400b(Vec<Option<Value>>, Vec<String>);
    impl Modelo for Mal400b {
        fn completar(&mut self, p: &str, _t: u64) -> Resultado<String> {
            self.1.push(p.to_string());
            match self.0.remove(0) {
                None => Err(Error::entorno("el servidor contestó Some(400): {\"error\":{\"message\":\"argumentos que no son un objeto JSON: call_3\",\"code\":\"invalid_request\"}}")),
                Some(m) => Ok(json!({"choices": [{"message": m}]}).to_string()),
            }
        }
    }
    let mut e = repo();
    let mut m = Mal400b(vec![None, None, None, Some(llamada("read", json!({"filePath": "README.md"}))), Some(json!({"role": "assistant", "content": "fin"}))], vec![]);
    let inf = correr(&mut e, &mut m, &RelojSimulado::nuevo(0), &Politica::nueva(R), &cfg_base(), "t", &mut |_, _| {});
    assert_eq!((inf.fin, inf.formato), (Fin::Terminado, 3), "{:?}", inf.error);
    let temp = |i: usize| serde_json::from_str::<Value>(&m.1[i]).unwrap()["temperature"].as_f64().unwrap();
    assert_eq!(temp(0), 0.0, "la primera petición va a T=0");
    assert!(temp(1) > 0.0 && temp(2) > temp(1), "cada reintento sube la temperatura: {} {}", temp(1), temp(2));
    assert!(!m.1[1].contains("MÁS CORTA") && m.1[2].contains("MÁS CORTA"), "el consejo llega desde el segundo fallo seguido");
    // Tras una respuesta buena la temperatura vuelve a la configurada.
    assert_eq!(temp(4), 0.0);
}

#[test]
fn la_tercera_ejecucion_con_las_mismas_pruebas_rojas_avisa_de_que_reescribir_no_las_mueve() {
    struct Rojas(Mem);
    impl Archivos for Rojas {
        fn leer(&self, r: &str) -> Resultado<Vec<u8>> { self.0.leer(r) }
        fn escribir(&mut self, r: &str, d: &[u8], m: u32) -> Resultado<()> { self.0.escribir(r, d, m) }
        fn existe(&self, r: &str) -> bool { self.0.existe(r) }
        fn listar(&self, r: &str) -> Resultado<Vec<Entrada>> { self.0.listar(r) }
        fn metadatos(&self, r: &str) -> Resultado<Entrada> { self.0.metadatos(r) }
        fn crear_directorio(&mut self, r: &str) -> Resultado<()> { self.0.crear_directorio(r) }
        fn borrar(&mut self, r: &str) -> Resultado<()> { self.0.borrar(r) }
    }
    impl Procesos for Rojas {
        fn ejecutar(&self, _: &Orden) -> Resultado<Salida> {
            Ok(Salida { codigo: Some(101), stdout: b"running 2 tests\ntest pruebas_agente::caso_a ... FAILED\ntest pruebas_agente::caso_b ... ok\n".to_vec(), stderr: vec![], motivo: None })
        }
    }
    let mut e = Rojas(repo());
    let mut guion: Vec<Value> = Vec::new();
    for i in 0..3 {
        guion.push(llamada("reemplazar_texto", json!({"filePath": "src/lib.rs", "buscar": if i == 0 { "hit();" } else { "x();" }, "nuevo": "x();"})));
        guion.push(llamada("bash", json!({"command": format!("cargo test -p x -- pruebas_agente {i}")})));
    }
    guion.push(json!({"role": "assistant", "content": "fin"}));
    let mut cfg = cfg_base();
    cfg.empujones = 0;
    let mut m = Guion(guion, vec![]);
    let inf = correr(&mut e, &mut m, &RelojSimulado::nuevo(0), &Politica::nueva(R), &cfg, "t", &mut |_, _| {});
    let avisos: Vec<&str> = inf.mensajes.iter().filter(|x| x["role"] == "tool").filter_map(|x| x["content"].as_str()).filter(|c| c.contains("tercera ejecución seguida")).collect();
    assert_eq!(avisos.len(), 1, "sólo a la tercera, una vez");
    assert!(avisos[0].contains("MISMAS pruebas en rojo (pruebas_agente::caso_a)"), "{}", avisos[0]);
}

#[test]
fn un_intento_que_agota_los_turnos_habiendo_editado_se_da_por_completo_para_validarlo() {
    use soso_improve_core::runner::{ejecutar_agente, Fin as FinRunner, Limites};
    let edita = |n: &str| llamada("reemplazar_texto", json!({"filePath": "src/lib.rs", "buscar": n, "nuevo": "hot();"}));
    let guion = || Guion(vec![edita("hit();"), llamada("read", json!({"filePath": "src/lib.rs"})), llamada("read", json!({"filePath": "src/otro.rs"})), llamada("read", json!({"filePath": "README.md"}))], vec![]);
    let mut cfg = cfg_base();
    cfg.max_turnos = 3;
    let lim = Limites { max_pasos: 3, ..Limites::default() };
    // Sin la opción: límite superado (no se valida).
    let inf = ejecutar_agente(&mut repo(), &mut guion(), &RelojSimulado::nuevo(0), &Politica::nueva(R), &cfg, "t", &lim);
    assert_eq!(inf.fin, FinRunner::LimitePasos);
    // Con la opción y habiendo editado: completo.
    cfg.validar_al_agotar = true;
    let inf = ejecutar_agente(&mut repo(), &mut guion(), &RelojSimulado::nuevo(0), &Politica::nueva(R), &cfg, "t", &lim);
    assert_eq!(inf.fin, FinRunner::Completo);
    // Con la opción pero SIN haber editado nada: sigue siendo límite superado.
    let sin_editar = Guion(vec![llamada("read", json!({"filePath": "src/lib.rs"})), llamada("read", json!({"filePath": "src/otro.rs"})), llamada("read", json!({"filePath": "README.md"})), llamada("glob", json!({"pattern": "*.rs"}))], vec![]);
    let mut m = sin_editar;
    let inf = ejecutar_agente(&mut repo(), &mut m, &RelojSimulado::nuevo(0), &Politica::nueva(R), &cfg, "t", &lim);
    assert_eq!(inf.fin, FinRunner::LimitePasos);
}

#[test]
fn al_cerrar_se_eliminan_las_pruebas_propias_que_siguen_en_rojo_y_solo_esas() {
    struct Poda(Mem);
    impl Archivos for Poda {
        fn leer(&self, r: &str) -> Resultado<Vec<u8>> { self.0.leer(r) }
        fn escribir(&mut self, r: &str, d: &[u8], m: u32) -> Resultado<()> { self.0.escribir(r, d, m) }
        fn existe(&self, r: &str) -> bool { self.0.existe(r) }
        fn listar(&self, r: &str) -> Resultado<Vec<Entrada>> { self.0.listar(r) }
        fn metadatos(&self, r: &str) -> Resultado<Entrada> { self.0.metadatos(r) }
        fn crear_directorio(&mut self, r: &str) -> Resultado<()> { self.0.crear_directorio(r) }
        fn borrar(&mut self, r: &str) -> Resultado<()> { self.0.borrar(r) }
    }
    impl Procesos for Poda {
        fn ejecutar(&self, _: &Orden) -> Resultado<Salida> {
            // En rojo mientras `roja` siga en el archivo.
            let src = String::from_utf8_lossy(&self.0.f["/w/src/lib.rs"]).into_owned();
            let (c, t) = if src.contains("fn roja") { (101, "running 2 tests\ntest pruebas_agente::verde ... ok\ntest pruebas_agente::roja ... FAILED\n") } else { (0, "running 1 tests\ntest pruebas_agente::verde ... ok\n") };
            Ok(Salida { codigo: Some(c), stdout: t.as_bytes().to_vec(), stderr: vec![], motivo: None })
        }
    }
    let task = "# T\n\n## Qué se pide\n- algo\n\n## Qué tiene que pasar\n\n- `cargo test -q -p x`\n";
    let modulo = "#[cfg(test)]\nmod pruebas_agente {\n    #[test]\n    fn verde() {\n        assert_eq!(1, 1);\n    }\n\n    #[test]\n    #[should_panic]\n    fn roja() {\n        assert_eq!(1, 2);\n    }\n}";
    let mut e = Poda(repo().con("/w/TASK.md", task));
    let mut cfg = cfg_base();
    cfg.exigir_pruebas = true;
    cfg.podar_pruebas_rojas = true;
    cfg.empujones = 0;
    cfg.avisos_sin_pruebas = 0;
    cfg.avisos_sin_ejecutar = 0;
    cfg.avisos_pocas_pruebas = 0;
    let mut m = Guion(
        vec![
            llamada("insertar", json!({"filePath": "src/lib.rs", "linea": -1, "texto": modulo})),
            llamada("bash", json!({"command": "cargo test -q -p x -- pruebas_agente"})),
            json!({"role": "assistant", "content": "Listo."}),
        ],
        vec![],
    );
    let inf = correr(&mut e, &mut m, &RelojSimulado::nuevo(0), &Politica::nueva(R), &cfg, "t", &mut |_, _| {});
    assert_eq!(inf.fin, Fin::Terminado, "{:?}", inf.error);
    assert_eq!(inf.podadas, vec!["roja".to_string()]);
    let src = String::from_utf8(e.0.f["/w/src/lib.rs"].clone()).unwrap();
    assert!(!src.contains("fn roja") && !src.contains("should_panic"), "{src}");
    assert!(src.contains("fn verde") && src.contains("mod pruebas_agente") && src.contains("fn buscar"), "{src}");
}

#[test]
fn la_poda_tambien_quita_las_pruebas_propias_que_no_compilan_y_nunca_el_codigo_de_produccion() {
    struct Compila(Mem);
    impl Archivos for Compila {
        fn leer(&self, r: &str) -> Resultado<Vec<u8>> { self.0.leer(r) }
        fn escribir(&mut self, r: &str, d: &[u8], m: u32) -> Resultado<()> { self.0.escribir(r, d, m) }
        fn existe(&self, r: &str) -> bool { self.0.existe(r) }
        fn listar(&self, r: &str) -> Resultado<Vec<Entrada>> { self.0.listar(r) }
        fn metadatos(&self, r: &str) -> Resultado<Entrada> { self.0.metadatos(r) }
        fn crear_directorio(&mut self, r: &str) -> Resultado<()> { self.0.crear_directorio(r) }
        fn borrar(&mut self, r: &str) -> Resultado<()> { self.0.borrar(r) }
    }
    impl Procesos for Compila {
        fn ejecutar(&self, _: &Orden) -> Resultado<Salida> {
            // Un error de compilación por cada línea que contiene `NO_EXISTE`, con su ubicación real.
            let src = String::from_utf8_lossy(&self.0.f["/w/src/lib.rs"]).into_owned();
            let mut t = String::new();
            for (i, l) in src.lines().enumerate() {
                if l.contains("NO_EXISTE") {
                    t.push_str(&format!("error[E0425]: cannot find function `NO_EXISTE` in this scope\n  --> src/lib.rs:{}:9\n   |\n", i + 1));
                }
            }
            let (c, t) = if t.is_empty() { (0, "test result: ok. 1 passed".to_string()) } else { (101, t) };
            Ok(Salida { codigo: Some(c), stdout: t.into_bytes(), stderr: vec![], motivo: None })
        }
    }
    let task = "# T\n\n## Qué se pide\n- algo\n\n## Qué tiene que pasar\n\n- `cargo test -q -p x`\n";
    let base = "fn buscar(k: &str) {\n    if k == key {\n        hit();\n    }\n}\n";
    // 1) una prueba propia dentro del módulo no compila: se va ella, el módulo y las demás se quedan.
    let modulo = "#[cfg(test)]\nmod pruebas_agente {\n    use super::*;\n\n    #[test]\n    fn buena() {}\n\n    #[test]\n    fn mala() {\n        NO_EXISTE(1);\n    }\n}";
    let mut cfg = cfg_base();
    cfg.exigir_pruebas = true;
    cfg.podar_pruebas_rojas = true;
    cfg.empujones = 0;
    cfg.avisos_sin_pruebas = 0;
    cfg.avisos_sin_ejecutar = 0;
    cfg.avisos_pocas_pruebas = 0;
    let mut e = Compila(Mem::default().con("/w/src/lib.rs", base).con("/w/TASK.md", task));
    let mut m = Guion(vec![llamada("insertar", json!({"filePath": "src/lib.rs", "linea": -1, "texto": modulo})), json!({"role": "assistant", "content": "Listo."})], vec![]);
    let inf = correr(&mut e, &mut m, &RelojSimulado::nuevo(0), &Politica::nueva(R), &cfg, "t", &mut |_, _| {});
    assert_eq!(inf.podadas, vec!["mala".to_string()], "{:?}", inf.error);
    let src = String::from_utf8(e.0.f["/w/src/lib.rs"].clone()).unwrap();
    assert!(!src.contains("NO_EXISTE") && src.contains("fn buena") && src.contains("mod pruebas_agente"), "{src}");
    // 2) el error está en un ayudante del módulo, fuera de cualquier prueba: se va el módulo entero.
    let modulo2 = "#[cfg(test)]\nmod pruebas_agente {\n    fn ayuda() -> u8 {\n        NO_EXISTE()\n    }\n\n    #[test]\n    fn usa_ayuda() {\n        ayuda();\n    }\n}";
    let mut e = Compila(Mem::default().con("/w/src/lib.rs", base).con("/w/TASK.md", task));
    let mut m = Guion(vec![llamada("insertar", json!({"filePath": "src/lib.rs", "linea": -1, "texto": modulo2})), json!({"role": "assistant", "content": "Listo."})], vec![]);
    let inf = correr(&mut e, &mut m, &RelojSimulado::nuevo(0), &Politica::nueva(R), &cfg, "t", &mut |_, _| {});
    assert_eq!(inf.podadas, vec!["pruebas_agente".to_string()]);
    assert_eq!(String::from_utf8(e.0.f["/w/src/lib.rs"].clone()).unwrap(), base);
    // 3) el error está en código de producción: no se toca nada.
    let mut e = Compila(Mem::default().con("/w/src/lib.rs", base).con("/w/TASK.md", task));
    let mut m = Guion(vec![llamada("reemplazar_texto", json!({"filePath": "src/lib.rs", "buscar": "hit();", "nuevo": "NO_EXISTE();"})), json!({"role": "assistant", "content": "Listo."})], vec![]);
    let inf = correr(&mut e, &mut m, &RelojSimulado::nuevo(0), &Politica::nueva(R), &cfg, "t", &mut |_, _| {});
    assert!(inf.podadas.is_empty());
    assert!(String::from_utf8(e.0.f["/w/src/lib.rs"].clone()).unwrap().contains("NO_EXISTE();"));
}

#[test]
fn al_sustituir_las_lineas_que_no_cambian_conservan_su_texto_original_aunque_el_modelo_comprima_los_espacios() {
    let src = "fn a(x: &str) -> &str {\n    // Qwen3.8: hidden % heads\n    let m = \"hola mundo cruel\";\n    if x == m {\n        return m;\n    }\n    x\n}\n";
    let mut e = Mem::default().con("/w/src/c.rs", src);
    // El modelo reescribe la función con los espacios comprimidos y cambia UNA línea (`return m;` → `return x;`).
    let nuevo = "fn a(x:&str) -> &str {\n    // Qwen3.8:hidden % heads\n    let m = \"holamundocruel\";\n    if x == m {\n        return x;\n    }\n    x\n}";
    let (t, err) = llama(&mut e, "reemplazar_funcion", json!({"filePath": "src/c.rs", "nombre": "a", "nuevo": nuevo}));
    assert!(!err, "{t}");
    let c = contenido(&mut e, "/w/src/c.rs");
    assert!(c.contains("// Qwen3.8: hidden % heads") && c.contains("\"hola mundo cruel\"") && c.contains("fn a(x: &str)"), "{c}");
    assert!(c.contains("return x;") && !c.contains("return m;"), "{c}");
    // El mismo caso con `reemplazar_texto` sobre varias líneas.
    let mut e = Mem::default().con("/w/src/c.rs", src);
    let (t, err) = llama(&mut e, "reemplazar_texto", json!({"filePath": "src/c.rs", "buscar": "let m = \"hola mundo cruel\";\n    if x == m {\n        return m;", "nuevo": "let m = \"holamundocruel\";\n    if x == m {\n        return x;"}));
    assert!(!err, "{t}");
    let c = contenido(&mut e, "/w/src/c.rs");
    assert!(c.contains("\"hola mundo cruel\"") && c.contains("return x;"), "{c}");
}
