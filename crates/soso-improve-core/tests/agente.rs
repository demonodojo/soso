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
    assert!(t.contains("Aviso"), "{t}");
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
    assert!(err && t.contains("más de una vez") && t.contains("1 y 2"), "{t}");
    assert_eq!(e2.f["/w/a.rs"], b"x();\nx();\n", "no se toca el archivo ante la duda");
}

#[test]
fn bash_rechaza_shell_y_lo_no_permitido() {
    let mut e = repo();
    for c in ["cargo test && rm -rf /", "cargo test | tee x", "cargo test > f", "cargo test $HOME", "RUSTFLAGS=x cargo test"] {
        let (t, err) = llama(&mut e, "bash", json!({"command": c}));
        assert!(err && (t.contains("sin shell") || t.contains("variables")), "{c}: {t}");
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
    let inf = correr(&mut e, &mut m, &reloj, &Politica::nueva(R), &Config::nueva("m", "eres un agente"), "arregla", &mut |_, _| vistos += 1);
    assert_eq!(inf.fin, Fin::Terminado, "{:?}", inf.error);
    assert_eq!((inf.turnos, inf.llamadas, vistos), (4, 3, 4));
    assert_eq!(inf.respuesta_final, "Listo: corregí la comparación.");
    assert!(String::from_utf8(e.f["/w/src/lib.rs"].clone()).unwrap().contains("*key"));
    // El tercer turno ya lleva el resultado de la herramienta anterior.
    assert!(m.1[1].contains("tool_call_id") && m.1[1].contains("src/lib.rs:2:"));
    let ult: Value = serde_json::from_str(&m.1[3]).unwrap();
    assert_eq!(ult["temperature"], 0.0);
    assert_eq!(ult["stream"], false);
    assert_eq!(ult["tools"].as_array().unwrap().len(), 5);
}

#[test]
fn argumentos_ilegibles_se_devuelven_al_modelo() {
    let mut e = repo();
    let malo = json!({"role": "assistant", "content": "", "tool_calls": [{"id": "x", "function": {"name": "read", "arguments": "{no json"}}]});
    let mut m = Guion(vec![malo, json!({"role": "assistant", "content": "ok"})], vec![]);
    let inf = correr(&mut e, &mut m, &RelojSimulado::nuevo(0), &Politica::nueva(R), &Config::nueva("m", "s"), "t", &mut |_, _| {});
    assert_eq!(inf.fin, Fin::Terminado);
    assert!(m.1[1].contains("no son un objeto JSON"));
}

#[test]
fn tope_de_turnos_y_error_del_modelo() {
    let mut e = repo();
    let bucle: Vec<Value> = (0..5).map(|_| llamada("read", json!({"filePath": "README.md"}))).collect();
    let mut cfg = Config::nueva("m", "s");
    cfg.max_turnos = 3;
    let inf = correr(&mut e, &mut Guion(bucle, vec![]), &RelojSimulado::nuevo(0), &Politica::nueva(R), &cfg, "t", &mut |_, _| {});
    assert_eq!((inf.fin, inf.turnos), (Fin::SinTurnos, 3));
    let inf = correr(&mut e, &mut Guion(vec![], vec![]), &RelojSimulado::nuevo(0), &Politica::nueva(R), &Config::nueva("m", "s"), "t", &mut |_, _| {});
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
    let inf = correr(&mut e, &mut m, &RelojSimulado::nuevo(0), &Politica::nueva(R), &Config::nueva("m", "s"), "t", &mut |h, ms| {
        sesion::guardar(h, "/w/.sesion", ms).unwrap();
    });
    assert_eq!(inf.fin, Fin::ErrorModelo);
    let previa = sesion::cargar(&e, "/w/.sesion").unwrap().expect("hay sesión");
    // 2 de arranque + 2 × (asistente + herramienta)
    assert_eq!(previa.len(), 6);
    // Otro proceso la recoge y termina sin repetir el trabajo ya hecho.
    let mut m2 = Guion(vec![json!({"role": "assistant", "content": "fin"})], vec![]);
    let inf = correr_desde(&mut e, &mut m2, &RelojSimulado::nuevo(0), &Politica::nueva(R), &Config::nueva("m", "s"), previa, &mut |h, ms| {
        sesion::guardar(h, "/w/.sesion", ms).unwrap();
    });
    assert_eq!((inf.fin, inf.turnos, inf.llamadas), (Fin::Terminado, 1, 0));
    assert!(m2.1[0].contains("src/lib.rs:2:"), "el modelo ve lo que ya se hizo");
    // Una sesión ya terminada no vuelve a llamar al modelo.
    let hecha = sesion::cargar(&e, "/w/.sesion").unwrap().unwrap();
    let mut m3 = Guion(vec![], vec![]);
    let inf = correr_desde(&mut e, &mut m3, &RelojSimulado::nuevo(0), &Politica::nueva(R), &Config::nueva("m", "s"), hecha, &mut |_, _| {});
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
