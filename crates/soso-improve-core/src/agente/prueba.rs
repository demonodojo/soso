//! Autoprueba del agente con un modelo guionizado: ejercita las **herramientas reales** sobre el
//! entorno que hospeda (host o guest) sin necesitar un modelo. Es lo que se ejecuta dentro de soso
//! para acreditar los adaptadores (archivos, procesos, plazo) con el mismo código que el agente.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use serde_json::{json, Value};

use crate::durable::Durable;
use crate::entorno::Entorno;
use crate::tiempo::Reloj;
use crate::{Error, Resultado};

use super::{correr, correr_desde, sesion, Config, Fin, Modelo, Politica};

struct Guion(Vec<Value>);

impl Modelo for Guion {
    fn completar(&mut self, _p: &str, _t: u64) -> Resultado<String> {
        if self.0.is_empty() {
            return Err(Error::entorno("el guion se acabó"));
        }
        Ok(json!({"choices": [{"message": self.0.remove(0)}]}).to_string())
    }
}

fn llamada(nombre: &str, args: Value) -> Value {
    json!({"role": "assistant", "content": "", "tool_calls": [
        {"id": "c", "type": "function", "function": {"name": nombre, "arguments": args.to_string()}}]})
}

/// Corre el escenario sobre `raiz` (se crea y se deja). `ejecutable` es una orden **que existe** en
/// el entorno, permitida por la política y de efecto nulo (p. ej. `["/bin/soso-improve","capacidades"]`).
/// Devuelve `(comprobación, ok, detalle)`.
pub fn autoprueba<E: Entorno + Durable, R: Reloj>(
    e: &mut E,
    reloj: &R,
    raiz: &str,
    ejecutable: &[&str],
) -> Resultado<Vec<(String, bool, String)>> {
    let raiz = raiz.trim_end_matches('/');
    sembrar(e, raiz)?;
    let mut pol = Politica::nueva(raiz);
    pol.ejecutables = alloc::vec![ejecutable.iter().map(|s| s.to_string()).collect()];
    pol.plazo_orden_s = 60;
    let orden_txt = ejecutable.join(" ");

    let mut m = Guion(alloc::vec![
        llamada("read", json!({"filePath": "TASK.md"})),
        llamada("glob", json!({"pattern": "*.rs"})),
        llamada("grep", json!({"pattern": "k == key"})),
        llamada("grep", json!({"pattern": "fn \\w+"})),
        llamada("read", json!({"filePath": "src/lib.rs", "offset": 4, "limit": 1})),
        llamada("reemplazar_texto", json!({"filePath": "src/lib.rs", "buscar": "k == key", "nuevo": "k.eq_ignore_ascii_case(key)"})),
        llamada("bash", json!({"command": format!("{orden_txt} ; rm -rf /")})),
        llamada("bash", json!({"command": "rm -rf src"})),
        llamada("bash", json!({"command": orden_txt})),
        llamada("read", json!({"filePath": "../fuera"})),
        json!({"role": "assistant", "content": "listo"}),
    ]);
    let mut cfg = Config::nueva("guion", "sistema");
    cfg.avisos_sin_editar = 0;
    let inf = correr(e, &mut m, reloj, &pol, &cfg, "tarea", &mut |_, _| {});

    // Resultados de herramienta, en orden.
    let salidas: Vec<String> = inf
        .mensajes
        .iter()
        .filter(|x| x["role"] == "tool")
        .map(|x| x["content"].as_str().unwrap_or("").to_string())
        .collect();
    let s = |i: usize| salidas.get(i).cloned().unwrap_or_default();
    let fuente = e.leer(&format!("{raiz}/src/lib.rs")).map(|d| String::from_utf8_lossy(&d).into_owned()).unwrap_or_default();
    let mut r: Vec<(String, bool, String)> = Vec::new();
    let mut anota = |n: &str, ok: bool, d: String| r.push((n.to_string(), ok, if ok { String::new() } else { d }));
    anota("el bucle termina", inf.fin == Fin::Terminado && inf.llamadas == 10, format!("{:?}, {} llamadas, {:?}", inf.fin, inf.llamadas, inf.error));
    anota("read numera", s(0).contains("1: Objetivo"), s(0));
    anota("glob encuentra", s(1).contains("src/lib.rs"), s(1));
    anota("grep literal", s(2).contains("src/lib.rs:4:"), s(2));
    anota("grep rechaza regex", s(3).contains("expresión regular"), s(3));
    anota("read con offset y limit", s(4).contains("4:") && !s(4).contains("5:"), s(4));
    anota("reemplazar_texto escribe", fuente.contains("k.eq_ignore_ascii_case(key)") && !fuente.contains("k == key"), fuente.clone());
    anota("bash rechaza shell", s(6).contains("sin shell"), s(6));
    anota("bash rechaza lo no permitido", s(7).contains("no está permitida") && e.existe(&format!("{raiz}/src")), s(7));
    anota("bash ejecuta lo permitido", s(8).starts_with("exit code: 0"), s(8));
    anota("read no sale del checkout", s(9).contains("sale del checkout"), s(9));

    // Sesión durable: un agente que muere tras dos llamadas y otro que la recoge.
    let dir = format!("{raiz}/.sesion");
    let mut m1 = Guion(alloc::vec![
        llamada("read", json!({"filePath": "TASK.md"})),
        llamada("grep", json!({"pattern": "eq_ignore"})),
    ]);
    let primero = correr(e, &mut m1, reloj, &pol, &cfg, "tarea", &mut |h, ms| {
        let _ = sesion::guardar(h, &dir, ms);
    });
    let previa = sesion::cargar(e, &dir)?;
    anota("la sesión sobrevive al agente", primero.fin == Fin::ErrorModelo && previa.as_ref().map(|p| p.len()) == Some(6), format!("{:?}", previa.as_ref().map(|p| p.len())));
    let mut m2 = Guion(alloc::vec![json!({"role": "assistant", "content": "retomado"})]);
    let segundo = correr_desde(e, &mut m2, reloj, &pol, &cfg, previa.unwrap_or_default(), &mut |h, ms| {
        let _ = sesion::guardar(h, &dir, ms);
    });
    let vigente = sesion::cargar(e, &dir)?.unwrap_or_default();
    anota("otro agente la retoma", segundo.fin == Fin::Terminado && segundo.respuesta_final == "retomado" && vigente.len() == 7, format!("{:?} {}", segundo.fin, vigente.len()));
    Ok(r)
}

/// Siembra un checkout mínimo con un fallo real de una línea (`src/lib.rs`) y su `TASK.md`: lo que
/// usan la autoprueba y la prueba con modelo real dentro de soso. La tarea **prohíbe ejecutar
/// órdenes**: no hay `Cargo.toml` y lo que se acredita es leer, buscar y editar.
pub fn sembrar<E: Entorno>(e: &mut E, raiz: &str) -> Resultado<()> {
    for d in [raiz.to_string(), format!("{raiz}/src")] {
        if !e.existe(&d) {
            e.crear_directorio(&d)?;
        }
    }
    e.escribir(
        &format!("{raiz}/TASK.md"),
        b"Objetivo: `header_value` en src/lib.rs debe buscar la cabecera sin distinguir mayusculas (metodo eq_ignore_ascii_case de str).\n\
Archivo que puedes cambiar: src/lib.rs (solo la comparacion `k == key`).\n\
No ejecutes ninguna orden (no hay Cargo.toml). Al terminar, informe corto de lo que cambiaste.\n",
        0o644,
    )?;
    e.escribir(
        &format!("{raiz}/src/lib.rs"),
        b"pub fn header_value<'a>(headers: &'a [(String, String)], key: &str) -> Option<&'a str> {\n    headers\n        .iter()\n        .find(|(k, _)| k == key)\n        .map(|(_, v)| v.as_str())\n}\n",
        0o644,
    )?;
    Ok(())
}
