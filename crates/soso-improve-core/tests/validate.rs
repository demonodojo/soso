//! T26 — el validador acepta por evidencia externa y sólo él mueve la base.

use std::collections::{BTreeMap, BTreeSet};

use soso_improve_core::captura::{self, ArchivoBase, Git, Politica};
use soso_improve_core::delta;
use soso_improve_core::durable::Durable;
use soso_improve_core::entorno::{Archivos, Entrada, Orden, Procesos, Salida, Tipo};
use soso_improve_core::state::{self, Autoridad, Comprobacion, Estado, Limites, TaskSpec};
use soso_improve_core::validate::*;
use soso_improve_core::{sha256_hex, Error, Resultado, ESQUEMA};

type Juez = fn(&Mem, &Orden) -> Salida;

#[derive(Clone)]
struct Mem {
    f: BTreeMap<String, Vec<u8>>,
    dirs: BTreeSet<String>,
    juez: Juez,
}

fn ok() -> Salida {
    Salida { codigo: Some(0), stdout: b"test result: ok".to_vec(), stderr: vec![], motivo: None }
}

/// `cargo test` de mentira que **lee el árbol**: pasa si el arreglo está en `src/lib.rs`.
fn juez_realista(m: &Mem, o: &Orden) -> Salida {
    let lib = m.f.get(&format!("{}/src/lib.rs", o.cwd)).cloned().unwrap_or_default();
    let arreglado = String::from_utf8_lossy(&lib).contains("eq_ignore_ascii_case");
    // Un test nuevo del candidato que sólo falla si falta el arreglo.
    let nuevo = m.f.get(&format!("{}/tests/nuevo.rs", o.cwd)).is_some();
    let pasa = if nuevo && o.argv.iter().any(|a| a == "regresion") { arreglado } else { arreglado };
    Salida { codigo: Some(if pasa { 0 } else { 1 }), stdout: if pasa { b"test result: ok".to_vec() } else { b"test result: FAILED".to_vec() }, stderr: vec![], motivo: None }
}

impl Mem {
    fn nuevo(juez: Juez) -> Self {
        Mem { f: BTreeMap::new(), dirs: BTreeSet::new(), juez }
    }
    fn pon(&mut self, r: &str, t: &str) {
        self.f.insert(r.to_string(), t.as_bytes().to_vec());
    }
}

impl Archivos for Mem {
    fn leer(&self, r: &str) -> Resultado<Vec<u8>> {
        self.f.get(r).cloned().ok_or_else(|| Error::entorno(format!("no existe {r}")))
    }
    fn escribir(&mut self, r: &str, d: &[u8], _: u32) -> Resultado<()> {
        self.f.insert(r.to_string(), d.to_vec());
        Ok(())
    }
    fn existe(&self, r: &str) -> bool {
        self.f.contains_key(r) || self.dirs.contains(r) || self.f.keys().any(|k| k.starts_with(&format!("{r}/")))
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
        if v.is_empty() && !self.dirs.contains(dir) {
            return Err(Error::entorno("no existe"));
        }
        Ok(v.into_values().collect())
    }
    fn metadatos(&self, r: &str) -> Resultado<Entrada> {
        match self.f.get(r) {
            Some(d) => Ok(Entrada { ruta: r.into(), tipo: Tipo::Archivo, bytes: d.len() as u64, modo: 0o644 }),
            None => Ok(Entrada { ruta: r.into(), tipo: Tipo::Directorio, bytes: 0, modo: 0o755 }),
        }
    }
    fn crear_directorio(&mut self, r: &str) -> Resultado<()> {
        self.dirs.insert(r.to_string());
        Ok(())
    }
    fn borrar(&mut self, r: &str) -> Resultado<()> {
        self.f.remove(r);
        self.dirs.remove(r);
        Ok(())
    }
}
impl Procesos for Mem {
    fn ejecutar(&self, o: &Orden) -> Resultado<Salida> {
        Ok((self.juez)(self, o))
    }
}
impl Durable for Mem {
    fn crear_exclusivo(&mut self, r: &str, d: &[u8]) -> Resultado<()> {
        if self.f.contains_key(r) {
            return Err(Error::uso("ya existe"));
        }
        self.f.insert(r.into(), d.to_vec());
        Ok(())
    }
    fn sincronizar(&mut self, _: &str) -> Resultado<()> {
        Ok(())
    }
}

const BUG: &str = "pub fn header_value(h: &[(String,String)], key: &str) -> Option<usize> {\n    h.iter().position(|(k, _)| k == key)\n}\n";
const ARREGLO: &str = "pub fn header_value(h: &[(String,String)], key: &str) -> Option<usize> {\n    h.iter().position(|(k, _)| k.eq_ignore_ascii_case(key))\n}\n";

struct Banco {
    v: Mem,
    man: captura::Manifiesto,
    spec: TaskSpec,
}

fn spec(base: &str) -> TaskSpec {
    TaskSpec {
        schema_version: ESQUEMA,
        id: "R03".into(),
        base: base.into(),
        problema: "p".into(),
        rutas_editables: vec!["src".into(), "tests".into()],
        comprobaciones: vec![Comprobacion { argv: vec!["cargo".into(), "test".into()], cwd: ".".into(), timeout_s: 60 }],
        limites: Limites::default(),
        hashes_entrada: Default::default(),
    }
}

fn banco(juez: Juez) -> Banco {
    let mut origen = Mem::nuevo(juez);
    origen.pon("/rep/src/lib.rs", BUG);
    origen.pon("/rep/tests/viejo.rs", "#[test] fn viejo() {}\n");
    origen.pon("/rep/verificador/caso.json", "{\"reservado\":true}\n");
    let mut v = Mem::nuevo(juez);
    let man = captura::capturar(&origen, "/rep", &mut v, "/val/cap", &Politica::default(), Git::default(), BTreeMap::new(), vec![], vec![], "2026-10-11T00:00:00Z").unwrap();
    let huella = captura::huella(&man.inventario);
    publicar_base(&mut v, "/val/refs", "c1", &huella).unwrap();
    let spec = spec(&huella);
    Banco { v, man, spec }
}

fn sitios<'a>(arbol: &'a str) -> Sitios<'a> {
    Sitios { captura: "/val/cap", objetos: "/val/obj", arbol, referencias: "/val/refs", campana: "c1" }
}

fn exigencias() -> Exigencias {
    Exigencias {
        requisitos: vec![Requisito { nombre: "cargo test".into(), argv: vec!["cargo".into(), "test".into()], cwd: ".".into(), timeout_s: 60 }],
        rutas_protegidas: vec!["verificador".into()],
        regresion: Some(Requisito { nombre: "regresion".into(), argv: vec!["cargo".into(), "test".into(), "regresion".into()], cwd: ".".into(), timeout_s: 60 }),
    }
}

/// Un candidato: parte de la base, aplica `cambios` (ruta, contenido o None=borrar) y devuelve su paquete
/// con los objetos ya enviados al validador.
fn candidato(b: &mut Banco, cambios: &[(&str, Option<&str>)]) -> delta::Paquete {
    let mut copia = b.v.clone();
    // La copia del agente: base reconstruida en /cand y editada.
    for a in &b.man.inventario {
        let d = b.v.leer(&format!("/val/cap/{}", captura::ruta_objeto(&a.sha256))).unwrap();
        copia.f.insert(format!("/cand/{}", a.ruta), d);
    }
    for (r, c) in cambios {
        match c {
            Some(t) => copia.pon(&format!("/cand/{r}"), t),
            None => {
                copia.f.remove(&format!("/cand/{r}"));
            }
        }
    }
    let inv: Vec<ArchivoBase> = copia
        .f
        .iter()
        .filter_map(|(k, d)| k.strip_prefix("/cand/").map(|r| ArchivoBase { ruta: r.into(), sha256: sha256_hex(d), bytes: d.len() as u64, modo: 0o644 }))
        .collect();
    let p = delta::exportar(&b.man.inventario, &inv);
    empaquetar_objetos(&mut copia, "/cand", &p, "/val/obj").unwrap();
    for (k, d) in &copia.f {
        if k.starts_with("/val/obj") {
            b.v.f.insert(k.clone(), d.clone());
        }
    }
    p
}

fn referencia(b: &Banco) -> BaseVigente {
    base_vigente(&b.v, "/val/refs", "c1").unwrap().unwrap().0
}

#[test]
fn un_arreglo_con_todo_en_verde_se_acepta_y_mueve_la_base() {
    let mut b = banco(juez_realista);
    let p = candidato(&mut b, &[("src/lib.rs", Some(ARREGLO))]);
    let antes = referencia(&b);
    let h = hash_paquete(&p).unwrap();
    let inf = validar_candidato(&mut b.v, &sitios("/val/arbol1"), &p, Some(&h), &b.spec, &exigencias(), None).unwrap();
    assert_eq!(inf.veredicto, Veredicto::Aceptado, "{}", resumen(&inf));
    let despues = referencia(&b);
    assert_ne!(antes, despues);
    assert_eq!((despues.huella.as_str(), despues.paquete.as_str()), (p.destino.as_str(), h.as_str()));
    assert_eq!(inf.evidencias[0].codigo, Some(0));
}

#[test]
fn un_falso_ok_con_exit_1_se_rechaza_aunque_la_salida_diga_ok() {
    fn miente(_: &Mem, _: &Orden) -> Salida {
        Salida { codigo: Some(1), stdout: b"test result: ok. 12 passed; OK OK OK".to_vec(), stderr: vec![], motivo: None }
    }
    let mut b = banco(miente);
    let p = candidato(&mut b, &[("src/lib.rs", Some(ARREGLO))]);
    let antes = referencia(&b);
    let inf = validar_candidato(&mut b.v, &sitios("/val/a"), &p, None, &b.spec, &exigencias(), None).unwrap();
    assert_eq!(inf.veredicto, Veredicto::Rechazado);
    assert_eq!(referencia(&b), antes, "sólo un candidato aceptado cambia la referencia");
}

#[test]
fn borrar_una_prueba_o_tocar_lo_protegido_o_salirse_del_alcance_se_rechaza() {
    for (cambio, pista) in [
        (("tests/viejo.rs", None), "borrar una prueba"),
        (("verificador/caso.json", Some("{}")), "protegida"),
        (("Cargo.toml", Some("[package]")), "fuera de las rutas editables"),
    ] {
        let mut b = banco(juez_realista);
        let p = candidato(&mut b, &[("src/lib.rs", Some(ARREGLO)), cambio]);
        let antes = referencia(&b);
        let inf = validar_candidato(&mut b.v, &sitios("/val/a"), &p, None, &b.spec, &exigencias(), None).unwrap();
        assert_eq!(inf.veredicto, Veredicto::Rechazado, "{pista}");
        assert!(inf.razones.iter().any(|r| r.contains(pista)), "{pista}: {:?}", inf.razones);
        assert_eq!(referencia(&b), antes);
        assert!(inf.evidencias.is_empty(), "no se ejecuta nada de un candidato que ya se sabe fuera de alcance");
    }
}

#[test]
fn un_paquete_distinto_del_anunciado_se_rechaza() {
    let mut b = banco(juez_realista);
    let p = candidato(&mut b, &[("src/lib.rs", Some(ARREGLO))]);
    let inf = validar_candidato(&mut b.v, &sitios("/val/a"), &p, Some(&"0".repeat(64)), &b.spec, &exigencias(), None).unwrap();
    assert_eq!(inf.veredicto, Veredicto::Rechazado);
    assert!(inf.razones[0].contains("no el anunciado"));
}

#[test]
fn un_check_que_no_se_pudo_medir_bloquea_no_acepta() {
    fn qemu_muerto(_: &Mem, _: &Orden) -> Salida {
        Salida { codigo: None, stdout: vec![], stderr: vec![], motivo: Some("plazo de 60 s agotado; el proceso se mató".into()) }
    }
    let mut b = banco(qemu_muerto);
    let p = candidato(&mut b, &[("src/lib.rs", Some(ARREGLO))]);
    let antes = referencia(&b);
    let inf = validar_candidato(&mut b.v, &sitios("/val/a"), &p, None, &b.spec, &exigencias(), None).unwrap();
    assert_eq!(inf.veredicto, Veredicto::Bloqueado);
    assert_eq!(referencia(&b), antes);
}

#[test]
fn dos_promociones_sobre_la_misma_base_gana_una_y_la_otra_se_conserva() {
    let mut b = banco(juez_realista);
    let p1 = candidato(&mut b, &[("src/lib.rs", Some(ARREGLO))]);
    let p2 = candidato(&mut b, &[("src/lib.rs", Some(&format!("{ARREGLO}// otro\n")))]);
    let a = validar_candidato(&mut b.v, &sitios("/val/a1"), &p1, None, &b.spec, &exigencias(), None).unwrap();
    let c = validar_candidato(&mut b.v, &sitios("/val/a2"), &p2, None, &b.spec, &exigencias(), None).unwrap();
    assert_eq!((a.veredicto, c.veredicto.clone()), (Veredicto::Aceptado, Veredicto::Bloqueado));
    assert!(c.razones.iter().any(|r| r.contains("la base cambió")));
    assert_eq!(referencia(&b).paquete, hash_paquete(&p1).unwrap(), "la base es la del primero");
    assert!(b.v.f.keys().any(|k| k.starts_with("/val/obj/")), "el candidato perdedor sigue disponible");
}

#[test]
fn una_prueba_nueva_que_ya_pasa_sobre_la_base_no_demuestra_nada() {
    // El juez da 0 sobre la base para cualquier cosa: la prueba del candidato no detecta el defecto.
    fn todo_pasa(_: &Mem, _: &Orden) -> Salida {
        ok()
    }
    let mut b = banco(todo_pasa);
    let p = candidato(&mut b, &[("src/lib.rs", Some(ARREGLO)), ("tests/nuevo.rs", Some("#[test] fn n() {}\n"))]);
    let inf = validar_candidato(&mut b.v, &sitios("/val/a"), &p, None, &b.spec, &exigencias(), None).unwrap();
    assert_eq!(inf.veredicto, Veredicto::Rechazado);
    assert!(inf.razones.iter().any(|r| r.contains("no detectan el defecto")));
}

#[test]
fn una_prueba_nueva_que_falla_sobre_la_base_y_pasa_con_el_arreglo_se_acepta() {
    let mut b = banco(juez_realista);
    let p = candidato(&mut b, &[("src/lib.rs", Some(ARREGLO)), ("tests/nuevo.rs", Some("#[test] fn n() {}\n"))]);
    let inf = validar_candidato(&mut b.v, &sitios("/val/a"), &p, None, &b.spec, &exigencias(), None).unwrap();
    assert_eq!(inf.veredicto, Veredicto::Aceptado, "{}", resumen(&inf));
    assert_eq!(inf.evidencias.len(), 2, "regresión sobre la base + requisito sobre el candidato");
}

#[test]
fn el_validador_escribe_el_estado_aceptada_y_no_el_coordinador() {
    let mut b = banco(juez_realista);
    let json = serde_json::to_vec(&b.spec).unwrap();
    b.v.crear_directorio("/val/estado").unwrap();
    state::preparar(&mut b.v, "/val/estado", &json, "r1", 0).unwrap();
    // El coordinador llevó el intento hasta `verificando`.
    let (mut m, h) = state::cargar(&b.v, "/val/estado", "r1").unwrap().unwrap();
    for e in [Estado::Reproduciendo, Estado::Editando, Estado::Verificando] {
        m.transicion(e, Autoridad::Coordinador).unwrap();
    }
    state::guardar(&mut b.v, "/val/estado", &m, Some(&h)).unwrap();
    assert!(m.clone().transicion(Estado::Aceptada, Autoridad::Coordinador).is_err());

    let p = candidato(&mut b, &[("src/lib.rs", Some(ARREGLO))]);
    let inf = validar_candidato(&mut b.v, &sitios("/val/a"), &p, None, &b.spec, &exigencias(), Some(("/val/estado", "r1"))).unwrap();
    assert_eq!(inf.veredicto, Veredicto::Aceptado);
    assert_eq!(state::cargar(&b.v, "/val/estado", "r1").unwrap().unwrap().0.estado, Estado::Aceptada);
}

#[test]
fn sin_requisitos_o_con_el_arbol_ocupado_no_se_acepta_nada() {
    let mut b = banco(juez_realista);
    let p = candidato(&mut b, &[("src/lib.rs", Some(ARREGLO))]);
    let ex = Exigencias { requisitos: vec![], ..exigencias() };
    let inf = validar_candidato(&mut b.v, &sitios("/val/a"), &p, None, &b.spec, &ex, None).unwrap();
    assert_eq!(inf.veredicto, Veredicto::Bloqueado);
    b.v.pon("/val/ocupado/x", "y");
    assert!(validar_candidato(&mut b.v, &sitios("/val/ocupado"), &p, None, &b.spec, &exigencias(), None).is_err());
}
