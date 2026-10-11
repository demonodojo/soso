//! T28 — aprender sólo lo verificado, y entregarlo acotado, determinista y sin lo reservado.

use std::collections::BTreeMap;

use soso_improve_core::captura::ArchivoBase;
use soso_improve_core::entorno::{Archivos, Entrada, Tipo};
use soso_improve_core::durable::Durable;
use soso_improve_core::knowledge::*;
use soso_improve_core::sha256_hex;
use soso_improve_core::state::{Limites, TaskSpec};
use soso_improve_core::validate::BaseVigente;
use soso_improve_core::{Error, Resultado, ESQUEMA};

#[derive(Default)]
struct Mem(BTreeMap<String, Vec<u8>>);
impl Archivos for Mem {
    fn leer(&self, r: &str) -> Resultado<Vec<u8>> {
        self.0.get(r).cloned().ok_or_else(|| Error::entorno("no existe"))
    }
    fn escribir(&mut self, r: &str, d: &[u8], _: u32) -> Resultado<()> {
        self.0.insert(r.into(), d.to_vec());
        Ok(())
    }
    fn existe(&self, r: &str) -> bool {
        self.0.contains_key(r) || self.0.keys().any(|k| k.starts_with(&format!("{r}/")))
    }
    fn listar(&self, dir: &str) -> Resultado<Vec<Entrada>> {
        let p = format!("{}/", dir.trim_end_matches('/'));
        Ok(self.0.keys().filter_map(|k| k.strip_prefix(&p).filter(|r| !r.contains('/')).map(|r| Entrada { ruta: r.into(), tipo: Tipo::Archivo, bytes: 0, modo: 0 })).collect())
    }
    fn metadatos(&self, r: &str) -> Resultado<Entrada> {
        Ok(Entrada { ruta: r.into(), tipo: Tipo::Archivo, bytes: 0, modo: 0 })
    }
    fn crear_directorio(&mut self, _: &str) -> Resultado<()> {
        Ok(())
    }
    fn borrar(&mut self, r: &str) -> Resultado<()> {
        self.0.remove(r);
        Ok(())
    }
}
impl Durable for Mem {
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

fn base(h_lib: &str) -> Vec<ArchivoBase> {
    vec![
        ArchivoBase { ruta: "crates/soso-http/src/lib.rs".into(), sha256: h_lib.into(), bytes: 1, modo: 0o644 },
        ArchivoBase { ruta: "crates/soso-abi/src/lib.rs".into(), sha256: "b".repeat(64), bytes: 1, modo: 0o644 },
    ]
}

fn spec(problema: &str, rutas: &[&str]) -> TaskSpec {
    TaskSpec {
        schema_version: ESQUEMA,
        id: "T".into(),
        base: "a".repeat(64),
        problema: problema.into(),
        rutas_editables: rutas.iter().map(|s| s.to_string()).collect(),
        comprobaciones: vec![],
        limites: Limites::default(),
        hashes_entrada: Default::default(),
    }
}

fn nota(prob: &str, sol: &str, rutas: &[&str], paquete: &str, fallido: bool) -> Nota {
    candidata(prob, sol, &rutas.iter().map(|s| s.to_string()).collect::<Vec<_>>(), &["cargo test -p soso-http".to_string()], &base(&"a".repeat(64)), &"c".repeat(64), paquete, vec![], fallido)
}

fn con_evidencia(mut n: Nota, mem: &mut Mem) -> Nota {
    mem.0.insert("/ev/informe.txt".into(), b"verificador exit 0".to_vec());
    n.evidencias = vec![Evidencia { ruta: "/ev/informe.txt".into(), sha256: sha256_hex(b"verificador exit 0") }];
    n
}

fn vigente(paquete: &str) -> BaseVigente {
    BaseVigente { huella: "d".repeat(64), paquete: paquete.into() }
}

#[test]
fn una_nota_nace_como_hipotesis_y_un_intento_fallido_como_limitacion() {
    let ok = nota("header_value distingue mayúsculas", "usar eq_ignore_ascii_case", &["crates/soso-http/src/lib.rs"], "p1", false);
    assert_eq!(ok.certeza, Certeza::Hipotesis);
    let mal = nota("probé regex en grep", "no funciona: el modelo la busca literal", &["crates/soso-http/src/lib.rs"], "", true);
    assert_eq!(mal.certeza, Certeza::Limitacion);
    assert!(mal.vigente && mal.hashes.contains_key("crates/soso-http/src/lib.rs"));
    let mut m = Mem::default();
    assert!(confirmar(&mut con_evidencia(mal, &mut m), &m, Some(&vigente(""))).is_err(), "una limitación no se confirma como hecho");
}

#[test]
fn solo_el_validador_convierte_una_hipotesis_en_hecho() {
    let mut m = Mem::default();
    let n = con_evidencia(nota("p", "s", &["crates/soso-http/src/lib.rs"], "paquete-bueno", false), &mut m);
    // Sin base de campaña, o con otro paquete, o sin evidencia: no.
    assert!(confirmar(&mut n.clone(), &m, None).is_err());
    let e = confirmar(&mut n.clone(), &m, Some(&vigente("otro-paquete"))).unwrap_err().to_string();
    assert!(e.contains("no respalda"), "{e}");
    let sin_ev = nota("p", "s", &["x"], "paquete-bueno", false);
    assert!(confirmar(&mut sin_ev.clone(), &m, Some(&vigente("paquete-bueno"))).unwrap_err().to_string().contains("sin evidencia"));
    // Evidencia que cambió o desapareció.
    let mut roto = n.clone();
    roto.evidencias[0].sha256 = "0".repeat(64);
    assert!(confirmar(&mut roto, &m, Some(&vigente("paquete-bueno"))).unwrap_err().to_string().contains("hash distinto"));
    let mut falta = n.clone();
    falta.evidencias[0].ruta = "/ev/no-esta.txt".into();
    assert!(confirmar(&mut falta, &m, Some(&vigente("paquete-bueno"))).unwrap_err().to_string().contains("falta la evidencia"));
    // Con todo en orden sí.
    let mut bueno = n;
    confirmar(&mut bueno, &m, Some(&vigente("paquete-bueno"))).unwrap();
    assert_eq!(bueno.certeza, Certeza::Hecho);
    assert_eq!(bueno.historial.len(), 2);
}

fn hecho(prob: &str, sol: &str, rutas: &[&str], paquete: &str) -> Nota {
    let mut m = Mem::default();
    let mut n = con_evidencia(nota(prob, sol, rutas, paquete, false), &mut m);
    confirmar(&mut n, &m, Some(&vigente(paquete))).unwrap();
    n
}

#[test]
fn se_recupera_por_ruta_y_palabras_con_orden_estable() {
    let a = hecho("cabeceras HTTP distinguen mayúsculas", "comparar con eq_ignore_ascii_case", &["crates/soso-http/src/lib.rs"], "pa");
    let b = hecho("el parser de respuestas pierde cabeceras repetidas", "acumular en vez de sustituir", &["crates/soso-http/src/lib.rs"], "pb");
    let c = hecho("el montón del kernel se fragmenta", "cambiar el asignador", &["kernel/src/heap.rs"], "pc");
    let ajena = hecho("algo de la ABI", "otra cosa", &["crates/soso-abi/src/lib.rs"], "pd");
    let todas = vec![c.clone(), b.clone(), a.clone(), ajena];
    let s = spec("las cabeceras HTTP deben compararse sin distinguir mayúsculas", &["crates/soso-http/src"]);
    let r1 = recuperar(&todas, &s, &Reservado::default(), 100_000);
    let ids: Vec<&str> = r1.notas.iter().map(|x| x.id.as_str()).collect();
    assert_eq!(ids.len(), 2, "sólo las pertinentes por ruta: {ids:?}");
    assert!(!ids.contains(&c.id.as_str()), "la del kernel no es pertinente");
    assert_eq!(ids[0], a.id, "la que además comparte palabras puntúa más");
    // Orden estable: mismo resultado se barajen como se barajen.
    let mut al_reves = todas.clone();
    al_reves.reverse();
    assert_eq!(r1.notas, recuperar(&al_reves, &s, &Reservado::default(), 100_000).notas);
    assert!(r1.notas[0].texto.contains("HECHO verificado") && r1.notas[0].texto.contains("vigente"));
}

#[test]
fn el_presupuesto_trunca_y_lo_dice() {
    let notas: Vec<Nota> = (0..6).map(|i| hecho(&format!("problema número {i} de cabeceras"), "solución de cabeceras", &["crates/soso-http/src/lib.rs"], &format!("p{i}"))).collect();
    let s = spec("cabeceras", &["crates/soso-http"]);
    let todo = recuperar(&notas, &s, &Reservado::default(), 1_000_000);
    assert_eq!(todo.notas.len(), 6);
    let una = todo.notas[0].texto.len();
    let r = recuperar(&notas, &s, &Reservado::default(), una * 2 + 10);
    assert_eq!(r.notas.len(), 2);
    assert_eq!(r.fuera_de_presupuesto, 4, "lo que no cabe se cuenta, no desaparece sin avisar");
    assert!(r.notas.iter().map(|x| x.texto.len()).sum::<usize>() <= una * 2 + 10);
}

#[test]
fn las_hipotesis_y_lo_caducado_no_llegan_al_agente_y_el_cambio_de_base_caduca_con_historial() {
    let mut notas = vec![hecho("cabeceras", "arreglo", &["crates/soso-http/src/lib.rs"], "p1"), nota("cabeceras", "sin confirmar", &["crates/soso-http/src/lib.rs"], "p2", false)];
    let s = spec("cabeceras", &["crates/soso-http"]);
    assert_eq!(recuperar(&notas, &s, &Reservado::default(), 100_000).notas.len(), 1, "la hipótesis no es contexto");
    // La base cambia: el archivo de la nota tiene otro hash.
    let n = invalidar_por_base(&mut notas, &base(&"9".repeat(64)), &"e".repeat(64));
    assert_eq!(n, 2);
    assert!(notas.iter().all(|x| !x.vigente));
    assert!(notas[0].historial.last().unwrap().que.contains("desactualizada: cambió crates/soso-http/src/lib.rs"));
    assert_eq!(notas[0].historial.len(), 3, "se conserva el historial");
    assert!(recuperar(&notas, &s, &Reservado::default(), 100_000).notas.is_empty());
    // Si la base no cambió lo que la nota menciona, sigue vigente.
    let mut otras = vec![hecho("cabeceras", "arreglo", &["crates/soso-http/src/lib.rs"], "p1")];
    assert_eq!(invalidar_por_base(&mut otras, &base(&"a".repeat(64)), &"e".repeat(64)), 0);
    assert!(otras[0].vigente);
}

#[test]
fn lo_reservado_los_secretos_y_los_transcritos_no_viajan() {
    let reservado = Reservado { prefijos: vec!["tests/self-improvement/cases/reservado".into()] };
    let s = spec("cabeceras HTTP", &["crates/soso-http", "tests"]);
    let con_caso = hecho("cabeceras HTTP", "mira el caso", &["tests/self-improvement/cases/reservado/R03.json"], "pr");
    let mut con_secreto = hecho("cabeceras HTTP", "usa Bearer abc123 para entrar", &["crates/soso-http/src/lib.rs"], "ps");
    con_secreto.id = "secreto".into();
    let con_env = hecho("cabeceras HTTP", "lee la clave", &["crates/soso-http/.env"], "pe");
    let normal = hecho("cabeceras HTTP", "arreglo normal", &["crates/soso-http/src/lib.rs"], "pn");
    let r = recuperar(&[con_caso, con_secreto, con_env, normal.clone()], &s, &reservado, 100_000);
    assert_eq!(r.notas.len(), 1);
    assert_eq!(r.notas[0].id, normal.id);
    assert_eq!(r.excluidas.len(), 3);
    assert!(r.excluidas.iter().any(|(_, m)| m.contains("reservado")) && r.excluidas.iter().any(|(_, m)| m.contains("secreto")));
    // Un transcrito entero no cabe en un campo.
    let enorme = "línea de transcrito\n".repeat(500);
    let n = nota(&enorme, &enorme, &["x"], "p", false);
    assert!(n.problema.len() <= CAMPO_MAX + 20 && n.problema.ends_with("(acotado)"));
}

#[test]
fn las_notas_se_guardan_por_generaciones_y_no_se_duplican() {
    let mut m = Mem::default();
    assert!(cargar(&m, "/k").unwrap().is_empty());
    let mut notas = Vec::new();
    let n = hecho("p", "s", &["crates/soso-http/src/lib.rs"], "p1");
    assert!(añadir(&mut notas, n.clone()));
    assert!(!añadir(&mut notas, n), "mismo id = misma nota");
    guardar(&mut m, "/k", &notas).unwrap();
    notas[0].vigente = false;
    guardar(&mut m, "/k", &notas).unwrap();
    let leidas = cargar(&m, "/k").unwrap();
    assert_eq!(leidas.len(), 1);
    assert!(!leidas[0].vigente);
}
