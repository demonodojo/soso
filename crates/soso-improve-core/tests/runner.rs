//! Ejecutor con límites (T25): proceso externo simulado y agente nativo con topes en vivo.

use std::cell::RefCell;

use serde_json::{json, Value};
use soso_improve_core::agente::{Config, Modelo, Politica};
use soso_improve_core::entorno::{Archivos, Entrada, Orden, Procesos, Salida, Tipo};
use soso_improve_core::runner::{ejecutar_agente, ejecutar_proceso, Evento, Fin, LectorEventos, Limites};
use soso_improve_core::tiempo::RelojSimulado;
use soso_improve_core::{Error, Resultado};

/// Salida real de OpenCode 1.18.34 (T22, 14B, R03): 5 pasos, 4 herramientas.
const REAL: &str = include_str!("../../../tests/self-improvement/opencode/r03-intento29/transcript.jsonl");

struct Falso {
    salida: RefCell<Option<Salida>>,
    visto: RefCell<Option<Orden>>,
}

fn falso(stdout: &[u8], stderr: &[u8], codigo: Option<i32>, motivo: Option<&str>) -> Falso {
    Falso {
        salida: RefCell::new(Some(Salida { codigo, stdout: stdout.to_vec(), stderr: stderr.to_vec(), motivo: motivo.map(|s| s.to_string()) })),
        visto: RefCell::new(None),
    }
}

impl Procesos for Falso {
    fn ejecutar(&self, o: &Orden) -> Resultado<Salida> {
        *self.visto.borrow_mut() = Some(o.clone());
        self.salida.borrow_mut().take().ok_or_else(|| Error::entorno("ya se usó"))
    }
}

fn orden() -> Orden {
    Orden::nueva(&["opencode", "run", "--format", "json"], "/w")
}

#[test]
fn la_salida_real_de_opencode_se_cuenta_por_paso_sin_duplicar() {
    let p = falso(REAL.as_bytes(), b"", Some(0), None);
    let inf = ejecutar_proceso(&p, orden(), &Limites::default());
    assert_eq!(inf.fin, Fin::Completo, "{}", inf.detalle);
    assert_eq!((inf.cuenta.pasos, inf.cuenta.herramientas), (5, 4));
    // Σ input y Σ output por paso; `total` no entra (sumarlo daría 26 u 27 mil).
    assert_eq!((inf.cuenta.tokens_entrada, inf.cuenta.tokens_salida), (25_094, 127));
    assert_eq!(inf.cuenta.lineas_ilegibles, 0);
    assert!(!inf.limites_en_vivo, "el proceso sólo se juzga a posteriori y el informe lo dice");
}

#[test]
fn el_plazo_se_fija_en_la_orden_y_un_plazo_vencido_se_informa() {
    let p = falso(b"", b"", None, Some("plazo de 2 s agotado; el proceso se mató"));
    let lim = Limites { plazo_ms: 1500, ..Limites::default() };
    let inf = ejecutar_proceso(&p, orden(), &lim);
    assert_eq!(inf.fin, Fin::Plazo);
    assert_eq!(p.visto.borrow().as_ref().unwrap().timeout_s, Some(2), "1500 ms se redondea hacia arriba");
}

#[test]
fn json_partido_en_cualquier_byte_da_los_mismos_eventos() {
    let entero = {
        let mut l = LectorEventos::nuevo();
        let ev = l.empujar(REAL.as_bytes());
        (ev, l.cuenta.clone())
    };
    for corte in [1usize, 2, 3, 7, 61, 1000] {
        let mut l = LectorEventos::nuevo();
        let mut ev = Vec::new();
        for trozo in REAL.as_bytes().chunks(corte) {
            ev.extend(l.empujar(trozo));
        }
        assert!(!l.fin());
        assert_eq!((ev, l.cuenta), entero.clone(), "corte de {corte} bytes");
    }
}

#[test]
fn un_utf8_partido_a_mitad_de_caracter_no_rompe_la_linea() {
    let linea = "{\"type\":\"text\",\"part\":{\"text\":\"aó—ü\"}}\n".as_bytes();
    let mut l = LectorEventos::nuevo();
    let mut ev = Vec::new();
    for b in linea.chunks(1) {
        ev.extend(l.empujar(b));
    }
    assert_eq!(ev, vec![Evento::Texto { bytes: "aó—ü".len() }]);
}

#[test]
fn stderr_abundante_no_bloquea_ni_contamina_los_eventos() {
    let ruido = vec![b'e'; 5 * 1024 * 1024];
    let p = falso(REAL.as_bytes(), &ruido, Some(0), None);
    let inf = ejecutar_proceso(&p, orden(), &Limites::default());
    assert_eq!(inf.fin, Fin::Completo);
    assert_eq!(inf.cuenta.herramientas, 4);
    assert!(inf.detalle.len() < 1000, "el detalle recorta stderr: {}", inf.detalle.len());
}

#[test]
fn exceso_de_herramientas_y_de_pasos_se_declara() {
    let p = falso(REAL.as_bytes(), b"", Some(0), None);
    let inf = ejecutar_proceso(&p, orden(), &Limites { max_herramientas: 3, ..Limites::default() });
    assert_eq!(inf.fin, Fin::LimiteHerramientas);
    let p = falso(REAL.as_bytes(), b"", Some(0), None);
    let inf = ejecutar_proceso(&p, orden(), &Limites { max_pasos: 4, ..Limites::default() });
    assert_eq!(inf.fin, Fin::LimitePasos);
}

#[test]
fn salida_incompleta_no_cuenta_como_completa() {
    // Se corta a mitad de una línea.
    let cortada = &REAL.as_bytes()[..REAL.len() - 40];
    let inf = ejecutar_proceso(&falso(cortada, b"", Some(0), None), orden(), &Limites::default());
    assert_eq!(inf.fin, Fin::SalidaIncompleta);
    assert_eq!(inf.cuenta.lineas_ilegibles, 1);
    // Sin ningún `step_finish`: arrancó pero no cerró.
    let sin_cierre = REAL.lines().take(2).collect::<Vec<_>>().join("\n") + "\n";
    let inf = ejecutar_proceso(&falso(sin_cierre.as_bytes(), b"", Some(0), None), orden(), &Limites::default());
    assert_eq!(inf.fin, Fin::SalidaIncompleta);
}

#[test]
fn proceso_que_falla_o_no_arranca() {
    let inf = ejecutar_proceso(&falso(REAL.as_bytes(), b"boom", Some(1), None), orden(), &Limites::default());
    assert_eq!(inf.fin, Fin::ProcesoFallo);
    assert!(inf.detalle.contains("boom"));
    let inf = ejecutar_proceso(&falso(b"", b"", None, Some("no se pudo lanzar opencode")), orden(), &Limites::default());
    assert_eq!(inf.fin, Fin::SinProceso);
}

#[test]
fn una_herramienta_con_dos_eventos_cuenta_una_vez() {
    let doble = "{\"type\":\"tool_use\",\"part\":{\"tool\":\"read\",\"callID\":\"c1\",\"state\":{\"status\":\"running\"}}}\n\
{\"type\":\"tool_use\",\"part\":{\"tool\":\"read\",\"callID\":\"c1\",\"state\":{\"status\":\"completed\"}}}\n";
    let mut l = LectorEventos::nuevo();
    let ev = l.empujar(doble.as_bytes());
    assert_eq!(l.cuenta.herramientas, 1);
    assert_eq!(ev.len(), 2, "el cierre sí se registra");
}

#[test]
fn el_registro_se_recorta_pero_la_cuenta_no() {
    let p = falso(REAL.as_bytes(), b"", Some(0), None);
    let inf = ejecutar_proceso(&p, orden(), &Limites { max_log_bytes: 200, ..Limites::default() });
    assert!(inf.log_truncado);
    assert!(inf.eventos.len() < 14);
    assert_eq!(inf.cuenta.herramientas, 4, "la cuenta es completa aunque el log no lo sea");
}

// ------------------------------------------------------------------ nativo

#[derive(Default)]
struct Mem(std::collections::BTreeMap<String, Vec<u8>>);
impl Archivos for Mem {
    fn leer(&self, r: &str) -> Resultado<Vec<u8>> {
        self.0.get(r).cloned().ok_or_else(|| Error::entorno("no existe"))
    }
    fn escribir(&mut self, r: &str, d: &[u8], _: u32) -> Resultado<()> {
        self.0.insert(r.into(), d.to_vec());
        Ok(())
    }
    fn existe(&self, r: &str) -> bool {
        self.0.contains_key(r)
    }
    fn listar(&self, dir: &str) -> Resultado<Vec<Entrada>> {
        let p = format!("{}/", dir.trim_end_matches('/'));
        let mut v = std::collections::BTreeMap::new();
        for (k, d) in &self.0 {
            if let Some(resto) = k.strip_prefix(&p) {
                match resto.split_once('/') {
                    Some((h, _)) => {
                        v.insert(h.to_string(), Entrada { ruta: h.into(), tipo: Tipo::Directorio, bytes: 0, modo: 0 });
                    }
                    None => {
                        v.insert(resto.to_string(), Entrada { ruta: resto.into(), tipo: Tipo::Archivo, bytes: d.len() as u64, modo: 0 });
                    }
                }
            }
        }
        Ok(v.into_values().collect())
    }
    fn metadatos(&self, r: &str) -> Resultado<Entrada> {
        Ok(Entrada { ruta: r.into(), tipo: Tipo::Archivo, bytes: self.0.get(r).map_or(0, |d| d.len() as u64), modo: 0 })
    }
    fn crear_directorio(&mut self, _: &str) -> Resultado<()> {
        Ok(())
    }
    fn borrar(&mut self, _: &str) -> Resultado<()> {
        Ok(())
    }
}
impl Procesos for Mem {
    fn ejecutar(&self, _: &Orden) -> Resultado<Salida> {
        Ok(Salida { codigo: Some(0), stdout: vec![], stderr: vec![], motivo: None })
    }
}

struct Guion(Vec<Value>);
impl Modelo for Guion {
    fn completar(&mut self, _: &str, _: u64) -> Resultado<String> {
        if self.0.is_empty() {
            return Err(Error::entorno("fin del guion"));
        }
        let m = self.0.remove(0);
        Ok(json!({"choices": [{"message": m}], "usage": {"prompt_tokens": 100, "completion_tokens": 10}}).to_string())
    }
}
fn lee() -> Value {
    json!({"role": "assistant", "content": "", "tool_calls": [{"id": "c", "function": {"name": "read", "arguments": "{\"filePath\":\"a\"}"}}]})
}

#[test]
fn el_agente_nativo_respeta_el_tope_de_herramientas_en_vivo() {
    let mut e = Mem::default();
    e.0.insert("/w/a".into(), b"x\n".to_vec());
    let mut m = Guion(vec![lee(), lee(), lee(), lee(), json!({"role": "assistant", "content": "fin"})]);
    let lim = Limites { max_herramientas: 2, ..Limites::default() };
    let inf = ejecutar_agente(&mut e, &mut m, &RelojSimulado::nuevo(0), &Politica::nueva("/w"), &Config::nueva("m", "s"), "t", &lim);
    assert_eq!(inf.fin, Fin::LimiteHerramientas);
    assert!(inf.limites_en_vivo);
    assert_eq!(inf.cuenta.herramientas, 2, "la tercera llamada no se lanzó");
    assert_eq!(m.0.len(), 2, "y el modelo no se consultó más");
}

#[test]
fn el_uso_nativo_se_suma_por_respuesta_y_termina_completo() {
    let mut e = Mem::default();
    e.0.insert("/w/a".into(), b"x\n".to_vec());
    let mut m = Guion(vec![lee(), json!({"role": "assistant", "content": "listo"})]);
    let inf = ejecutar_agente(&mut e, &mut m, &RelojSimulado::nuevo(0), &Politica::nueva("/w"), &Config::nueva("m", "s"), "t", &Limites::default());
    assert_eq!(inf.fin, Fin::Completo);
    assert_eq!((inf.cuenta.tokens_entrada, inf.cuenta.tokens_salida), (200, 20));
    assert!(inf.eventos.contains(&Evento::Herramienta { id: "c".into(), nombre: "read".into(), completada: true }));
}

#[test]
fn un_presupuesto_de_cero_se_rechaza() {
    assert!(soso_improve_core::runner::validar(&Limites { plazo_ms: 0, ..Limites::default() }).is_err());
    assert!(soso_improve_core::runner::validar(&Limites::default()).is_ok());
}

impl soso_improve_core::durable::Durable for Mem {
    fn crear_exclusivo(&mut self, r: &str, d: &[u8]) -> Resultado<()> {
        if self.0.contains_key(r) {
            return Err(Error::uso("ya existe"));
        }
        self.0.insert(r.into(), d.to_vec());
        Ok(())
    }
    fn sincronizar(&mut self, _: &str) -> Resultado<()> {
        Ok(())
    }
}

fn spec_json(herramientas: u32) -> Vec<u8> {
    use soso_improve_core::state::{Limites as L, TaskSpec};
    serde_json::to_vec(&TaskSpec {
        schema_version: soso_improve_core::ESQUEMA,
        id: "T99".into(),
        base: "a".repeat(64),
        problema: "p".into(),
        rutas_editables: vec!["a".into()],
        comprobaciones: vec![soso_improve_core::state::Comprobacion { argv: vec!["cargo".into(), "test".into()], cwd: ".".into(), timeout_s: 60 }],
        limites: L { herramientas_max: herramientas, ..L::default() },
        hashes_entrada: Default::default(),
    })
    .unwrap()
}

fn preparado(herramientas: u32) -> Mem {
    let mut e = Mem::default();
    e.0.insert("/w/a".into(), b"x\n".to_vec());
    e.0.insert("/w/TASK.md".into(), b"arregla a\n".to_vec());
    soso_improve_core::state::preparar(&mut e, "/w/estado", &spec_json(herramientas), "r1", 0).unwrap();
    e
}

#[test]
fn un_intento_de_tarea_deja_estado_registro_y_medidas_honestas() {
    use soso_improve_core::state::{cargar, Estado, Medida};
    let mut e = preparado(30);
    let mut m = Guion(vec![lee(), json!({"role": "assistant", "content": "listo"})]);
    let inf = soso_improve_core::runner::ejecutar_intento(&mut e, &mut m, &RelojSimulado::nuevo(5), "/w/estado", "r1", "/w", &Config::nueva("m", "s"), &Politica::nueva("/w")).unwrap();
    assert_eq!(inf.fin, Fin::Completo);
    let (man, _) = cargar(&e, "/w/estado", "r1").unwrap().unwrap();
    assert_eq!(man.estado, Estado::Verificando, "el coordinador no acepta: eso es del validador");
    let a = &man.intentos[0];
    assert_eq!((a.herramientas.clone(), a.tokens.clone()), (Medida::Valor(1), Medida::Valor(220)));
    assert_eq!(a.estado, Estado::Verificando);
    assert!(a.fin_ms.is_some());
    assert!(e.0.contains_key(&a.artefactos[0]), "el registro existe y el estado sólo guarda su ruta");
}

#[test]
fn el_limite_de_herramientas_de_la_tarea_bloquea_el_intento() {
    use soso_improve_core::state::{cargar, Estado};
    let mut e = preparado(1);
    let mut m = Guion(vec![lee(), lee(), json!({"role": "assistant", "content": "x"})]);
    let inf = soso_improve_core::runner::ejecutar_intento(&mut e, &mut m, &RelojSimulado::nuevo(0), "/w/estado", "r1", "/w", &Config::nueva("m", "s"), &Politica::nueva("/w")).unwrap();
    assert_eq!(inf.fin, Fin::LimiteHerramientas);
    assert_eq!(cargar(&e, "/w/estado", "r1").unwrap().unwrap().0.estado, Estado::Bloqueada);
}

#[test]
fn un_intento_abierto_no_se_repite_a_ciegas() {
    use soso_improve_core::state::{cargar, guardar, Limites as L};
    let mut e = preparado(30);
    let (mut man, h) = cargar(&e, "/w/estado", "r1").unwrap().unwrap();
    man.abrir_intento(&L::default(), 1).unwrap();
    guardar(&mut e, "/w/estado", &man, Some(&h)).unwrap();
    let mut m = Guion(vec![]);
    let r = soso_improve_core::runner::ejecutar_intento(&mut e, &mut m, &RelojSimulado::nuevo(0), "/w/estado", "r1", "/w", &Config::nueva("m", "s"), &Politica::nueva("/w"));
    assert!(r.unwrap_err().to_string().contains("T27"));
}
