//! T27 — una ejecución que se cae en cualquier punto acaba igual que una sin cortes, o se bloquea
//! sin duplicar nada. El barrido corta **antes y después de cada escritura** de la ejecución entera.

use std::collections::{BTreeMap, BTreeSet};

use soso_improve_core::captura::{self, Git, Politica};
use soso_improve_core::durable::Durable;
use soso_improve_core::entorno::{Archivos, Entrada, Orden, Procesos, Salida, Tipo};
use soso_improve_core::recovery::*;
use soso_improve_core::state::{self, Comprobacion, Estado, Limites, Medida, TaskSpec};
use soso_improve_core::tiempo::RelojSimulado;
use soso_improve_core::validate::{self, Exigencias, Requisito, Sitios, Veredicto};
use soso_improve_core::workspace;
use soso_improve_core::{Error, Resultado, ESQUEMA};

#[derive(Clone, Default)]
struct Mem {
    f: BTreeMap<String, Vec<u8>>,
    dirs: BTreeSet<String>,
}

/// Mem que muere en la escritura número `corte` (contando escribir/crear_exclusivo/borrar).
#[derive(Clone)]
struct Cortable {
    m: Mem,
    cuenta: usize,
    corte: Option<(usize, bool)>, // (n, despues)
    /// Tras el corte el proceso **ha muerto**: ya no ejecuta ni los manejadores de error.
    muerto: bool,
}

impl Cortable {
    fn paso(&mut self) -> Result<bool, Error> {
        if self.muerto {
            return Err(Error::entorno("el proceso ya murió"));
        }
        let n = self.cuenta;
        self.cuenta += 1;
        match self.corte {
            Some((k, false)) if k == n => {
                self.muerto = true;
                Err(Error::entorno("corte antes"))
            }
            Some((k, true)) if k == n => {
                self.muerto = true;
                Ok(true)
            }
            _ => Ok(false),
        }
    }
}

fn despues(r: Resultado<()>, tras: bool) -> Resultado<()> {
    r?;
    if tras {
        Err(Error::entorno("corte después"))
    } else {
        Ok(())
    }
}

impl Archivos for Cortable {
    fn leer(&self, r: &str) -> Resultado<Vec<u8>> {
        self.m.f.get(r).cloned().ok_or_else(|| Error::entorno(format!("no existe {r}")))
    }
    fn escribir(&mut self, r: &str, d: &[u8], _: u32) -> Resultado<()> {
        let tras = self.paso()?;
        self.m.f.insert(r.into(), d.to_vec());
        despues(Ok(()), tras)
    }
    fn existe(&self, r: &str) -> bool {
        self.m.f.contains_key(r) || self.m.dirs.contains(r) || self.m.f.keys().any(|k| k.starts_with(&format!("{r}/")))
    }
    fn listar(&self, dir: &str) -> Resultado<Vec<Entrada>> {
        let p = format!("{}/", dir.trim_end_matches('/'));
        let mut v: BTreeMap<String, Entrada> = BTreeMap::new();
        for (k, d) in &self.m.f {
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
        if v.is_empty() && !self.m.dirs.contains(dir) {
            return Err(Error::entorno("no existe"));
        }
        Ok(v.into_values().collect())
    }
    fn metadatos(&self, r: &str) -> Resultado<Entrada> {
        match self.m.f.get(r) {
            Some(d) => Ok(Entrada { ruta: r.into(), tipo: Tipo::Archivo, bytes: d.len() as u64, modo: 0o644 }),
            None => Ok(Entrada { ruta: r.into(), tipo: Tipo::Directorio, bytes: 0, modo: 0o755 }),
        }
    }
    fn crear_directorio(&mut self, r: &str) -> Resultado<()> {
        self.m.dirs.insert(r.into());
        Ok(())
    }
    fn borrar(&mut self, r: &str) -> Resultado<()> {
        let tras = self.paso()?;
        self.m.f.remove(r);
        self.m.dirs.remove(r);
        despues(Ok(()), tras)
    }
}
impl Procesos for Cortable {
    fn ejecutar(&self, o: &Orden) -> Resultado<Salida> {
        let lib = self.m.f.get(&format!("{}/src/lib.rs", o.cwd)).cloned().unwrap_or_default();
        let ok = String::from_utf8_lossy(&lib).contains("eq_ignore_ascii_case");
        Ok(Salida { codigo: Some(if ok { 0 } else { 1 }), stdout: vec![], stderr: vec![], motivo: None })
    }
}
impl Durable for Cortable {
    fn crear_exclusivo(&mut self, r: &str, d: &[u8]) -> Resultado<()> {
        let tras = self.paso()?;
        if self.m.f.contains_key(r) {
            return Err(Error::uso("ya existe"));
        }
        self.m.f.insert(r.into(), d.to_vec());
        despues(Ok(()), tras)
    }
    fn sincronizar(&mut self, _: &str) -> Resultado<()> {
        Ok(())
    }
}

const BUG: &str = "pub fn f(k: &str, q: &str) -> bool {\n    k == q\n}\n";
const ARREGLO: &str = "pub fn f(k: &str, q: &str) -> bool {\n    k.eq_ignore_ascii_case(q)\n}\n";

fn spec(base: &str, intentos: u32) -> TaskSpec {
    TaskSpec {
        schema_version: ESQUEMA,
        id: "R99".into(),
        base: base.into(),
        problema: "p".into(),
        rutas_editables: vec!["src".into()],
        comprobaciones: vec![Comprobacion { argv: vec!["t".into()], cwd: ".".into(), timeout_s: 5 }],
        limites: Limites { intentos_max: intentos, ..Limites::default() },
        hashes_entrada: Default::default(),
    }
}

struct Escenario {
    e: Cortable,
    spec: TaskSpec,
    ex: Exigencias,
}

fn escenario(intentos: u32) -> Escenario {
    let mut origen = Mem::default();
    origen.f.insert("/rep/src/lib.rs".into(), BUG.as_bytes().to_vec());
    let mut alm = Cortable { m: Mem::default(), cuenta: 0, corte: None, muerto: false };
    let man = captura::capturar(&Cortable { m: origen, cuenta: 0, corte: None, muerto: false }, "/rep", &mut alm, "/val/cap", &Politica::default(), Git::default(), BTreeMap::new(), vec![], vec![], "x").unwrap();
    let huella = captura::huella(&man.inventario);
    let spec = spec(&huella, intentos);
    alm.m.dirs.insert("/val/estado".into());
    state::preparar(&mut alm, "/val/estado", &serde_json::to_vec(&spec).unwrap(), "r1", 0).unwrap();
    validate::publicar_base(&mut alm, "/val/refs", "c1", &huella).unwrap();
    let captura_ro = alm.clone();
    workspace::preparar(&captura_ro, "/val/cap", &mut alm, "/copia", &man, &spec, "r1").unwrap();
    alm.cuenta = 0;
    Escenario {
        e: alm,
        spec,
        ex: Exigencias { requisitos: vec![Requisito { nombre: "t".into(), argv: vec!["t".into()], cwd: ".".into(), timeout_s: 5 }], rutas_protegidas: vec![], regresion: None },
    }
}

fn contexto<'a>(s: &'a Escenario, adoptar: bool) -> Contexto<'a> {
    Contexto {
        dir_estado: "/val/estado",
        run_id: "r1",
        captura: "/val/cap",
        copia: "/copia",
        sitios: Sitios { captura: "/val/cap", objetos: "/val/obj", arbol: "/val/arbol", referencias: "/val/refs", campana: "c1" },
        spec: &s.spec,
        exigencias: &s.ex,
        adoptar_copia: adoptar,
    }
}

/// El «agente»: abre un intento, edita la copia y lo cierra. Hace varias escrituras durables: los cortes
/// pueden caer dentro.
fn lanzar_falso(e: &mut Cortable, intentos: u32) -> Resultado<()> {
    let (mut m, h) = state::cargar(&*e, "/val/estado", "r1")?.unwrap();
    if m.estado == Estado::Pendiente {
        m.transicion(Estado::Reproduciendo, state::Autoridad::Coordinador)?;
    }
    if m.estado == Estado::Reproduciendo {
        m.transicion(Estado::Editando, state::Autoridad::Coordinador)?;
    }
    if m.estado == Estado::Verificando {
        m.transicion(Estado::Editando, state::Autoridad::Coordinador)?;
    }
    m.abrir_intento(&Limites { intentos_max: intentos, ..Limites::default() }, 10)?;
    state::guardar(e, "/val/estado", &m, Some(&h))?;
    e.escribir("/copia/src/lib.rs", ARREGLO.as_bytes(), 0o644)?;
    let (mut m, h) = state::cargar(&*e, "/val/estado", "r1")?.unwrap();
    m.cerrar_intento(Estado::Verificando, Medida::Valor(1), Medida::desconocida("guion"), 20)?;
    m.transicion(Estado::Verificando, state::Autoridad::Coordinador)?;
    state::guardar(e, "/val/estado", &m, Some(&h))?;
    Ok(())
}

fn correr(s: &mut Escenario, adoptar: bool, intentos: u32) -> Resultado<Reanudado> {
    let cx_spec = s.spec.clone();
    let ex = s.ex.clone();
    let tmp = Escenario { e: s.e.clone(), spec: cx_spec, ex };
    let cx = contexto(&tmp, adoptar);
    avanzar(&mut s.e, &RelojSimulado::nuevo(30), &cx, &mut |e| lanzar_falso(e, intentos))
}

fn generaciones_de_base(e: &Cortable) -> usize {
    e.m.f.keys().filter(|k| k.starts_with("/val/refs/base-c1.")).count()
}

#[test]
fn una_ejecucion_sin_cortes_acepta_y_promueve_una_vez() {
    let mut s = escenario(3);
    let r = correr(&mut s, false, 3).unwrap();
    assert_eq!(r.desenlace, Desenlace::Validado(Veredicto::Aceptado));
    assert_eq!(r.ejecutadas.len(), 3);
    assert_eq!(generaciones_de_base(&s.e), 2, "la base inicial y una promoción");
    assert_eq!(state::cargar(&s.e, "/val/estado", "r1").unwrap().unwrap().0.estado, Estado::Aceptada);
    // Volver a llamar no hace nada más.
    let otra = correr(&mut s, false, 3).unwrap();
    assert_eq!(otra.desenlace, Desenlace::Validado(Veredicto::Aceptado));
    assert!(otra.ejecutadas.is_empty());
    assert_eq!(generaciones_de_base(&s.e), 2);
}

#[test]
fn cortar_antes_y_despues_de_cada_escritura_acaba_igual_o_bloquea_sin_duplicar() {
    // Cuántas escrituras tiene la ejecución entera.
    let mut medida = escenario(3);
    correr(&mut medida, false, 3).unwrap();
    let total = medida.e.cuenta;
    assert!(total >= 15, "la ejecución tiene que tener muchas escrituras que cortar: {total}");
    let mut reconocidas = 0;
    let mut bloqueos = 0;
    for k in 0..total {
        for tras in [false, true] {
            let mut s = escenario(3);
            s.e.corte = Some((k, tras));
            let primera = correr(&mut s, false, 3);
            // Un corte sobre una operación «a mejor esfuerzo» (purgar) no devuelve error pero el proceso ya murió.
            assert!(primera.is_err() || s.e.muerto, "el corte {k} ({tras}) no cortó: {primera:?}");
            s.e.corte = None;
            s.e.muerto = false;
            s.e.cuenta = 0;
            let mut r = correr(&mut s, false, 3).unwrap();
            if let Desenlace::Bloqueado { motivo, comprobacion } = &r.desenlace {
                bloqueos += 1;
                assert!(motivo.contains("se cayó") && comprobacion.contains("adoptar_copia"), "{motivo} / {comprobacion}");
                assert_eq!(state::cargar(&s.e, "/val/estado", "r1").unwrap().unwrap().0.estado, Estado::Bloqueada);
                assert_eq!(generaciones_de_base(&s.e), 1, "bloqueada: la base no se ha movido");
                r = correr(&mut s, true, 3).unwrap();
            }
            assert_eq!(r.desenlace, Desenlace::Validado(Veredicto::Aceptado), "corte {k} ({tras})");
            if r.reutilizadas.iter().any(|x| x.contains("validar")) && !r.ejecutadas.iter().any(|x| x.contains("validar")) {
                reconocidas += 1;
            }
            // Mismo resultado que sin cortes: una sola promoción, el árbol arreglado, tarea aceptada.
            assert_eq!(generaciones_de_base(&s.e), 2, "corte {k} ({tras}): promoción duplicada o perdida");
            assert_eq!(s.e.m.f["/copia/src/lib.rs"], ARREGLO.as_bytes());
            let (m, _) = state::cargar(&s.e, "/val/estado", "r1").unwrap().unwrap();
            assert_eq!(m.estado, Estado::Aceptada, "corte {k} ({tras})");
            assert!(m.intentos.len() <= 2, "corte {k} ({tras}): {} intentos; reanudar no reinicia ni infla el presupuesto", m.intentos.len());
            assert!(m.intentos.iter().all(|a| a.fin_ms.is_some() || m.estado.terminal()), "queda un intento abierto en {k}");
            let (base, _) = validate::base_vigente(&s.e, "/val/refs", "c1").unwrap().unwrap();
            assert!(!base.paquete.is_empty());
        }
    }
    assert!(reconocidas > 0, "ningún corte cayó entre la promoción y su resultado: el barrido no cubre esa ventana");
    assert!(bloqueos > 0, "ningún corte cayó dentro de lanzar con la copia ya editada");
}

#[test]
fn una_promocion_ya_aplicada_se_reconoce_por_referencia_y_hash_sin_republicar() {
    // Se corta justo después de la promoción (la escritura de la referencia de base) y antes del resultado.
    let mut medida = escenario(3);
    correr(&mut medida, false, 3).unwrap();
    let total = medida.e.cuenta;
    let mut visto = false;
    for k in 0..total {
        let mut s = escenario(3);
        s.e.corte = Some((k, true));
        let _ = correr(&mut s, false, 3);
        if generaciones_de_base(&s.e) == 2 && state::cargar(&s.e, "/val/estado", "r1").unwrap().unwrap().0.estado != Estado::Aceptada {
            visto = true;
            s.e.corte = None;
            s.e.muerto = false;
            let est = state::cargar(&s.e, "/val/estado", "r1").unwrap().unwrap().0.estado;
            let r = correr(&mut s, false, 3).unwrap_or_else(|er| panic!("corte {k}, estado {est:?}: {er}"));
            assert_eq!(r.desenlace, Desenlace::Validado(Veredicto::Aceptado));
            assert!(r.ejecutadas.iter().all(|x| !x.contains("validar")), "no se vuelve a validar: {:?}", r.ejecutadas);
            assert_eq!(generaciones_de_base(&s.e), 2, "no se vuelve a publicar");
            assert_eq!(state::cargar(&s.e, "/val/estado", "r1").unwrap().unwrap().0.estado, Estado::Aceptada);
            break;
        }
    }
    assert!(visto, "no hay un corte entre la promoción y el estado: la ventana existe y tiene que probarse");
}

#[test]
fn un_diario_truncado_no_hace_repetir_ni_pierde_lo_hecho() {
    let mut s = escenario(3);
    s.e.corte = Some((24, false));
    let _ = correr(&mut s, false, 3);
    s.e.corte = None;
    s.e.muerto = false;
    // La última generación del diario quedó a medias (corte durante la escritura).
    let ultima = s.e.m.f.keys().filter(|k| k.starts_with("/val/estado/diario-r1.")).max().cloned();
    if let Some(u) = ultima {
        let v = s.e.m.f.get_mut(&u).unwrap();
        let n = v.len() / 2;
        v.truncate(n);
    }
    let mut r = correr(&mut s, false, 3).unwrap();
    if matches!(r.desenlace, Desenlace::Bloqueado { .. }) {
        r = correr(&mut s, true, 3).unwrap();
    }
    assert_eq!(r.desenlace, Desenlace::Validado(Veredicto::Aceptado));
    assert_eq!(generaciones_de_base(&s.e), 2);
}

#[test]
fn el_presupuesto_de_intentos_no_se_reinicia_al_reanudar() {
    // Un solo intento permitido: el agente se cae a mitad (intento abierto), la copia sigue sin cambios.
    let mut s = escenario(1);
    // Cortar tras abrir el intento (segunda escritura de `lanzar_falso`) y antes de editar la copia.
    let mut medida = escenario(1);
    correr(&mut medida, false, 1).unwrap();
    let mut abierto = None;
    for k in 0..medida.e.cuenta {
        let mut t = escenario(1);
        t.e.corte = Some((k, true));
        let _ = correr(&mut t, false, 1);
        let (m, _) = state::cargar(&t.e, "/val/estado", "r1").unwrap().unwrap();
        if m.intentos.len() == 1 && m.intentos[0].fin_ms.is_none() && t.e.m.f["/copia/src/lib.rs"] == BUG.as_bytes() {
            abierto = Some(k);
            break;
        }
    }
    let k = abierto.expect("hay un corte con el intento abierto y la copia intacta");
    s.e.corte = Some((k, true));
    let _ = correr(&mut s, false, 1);
    s.e.corte = None;
    s.e.muerto = false;
    // Al reanudar hay que relanzar, pero el presupuesto ya está gastado: no se inventa otro intento.
    let r = correr(&mut s, false, 1);
    assert!(r.is_err(), "con el presupuesto agotado reanudar no puede abrir otro intento: {r:?}");
    assert!(r.unwrap_err().to_string().contains("agotados"));
    assert_eq!(generaciones_de_base(&s.e), 1, "y la base no se mueve");
    assert_eq!(state::cargar(&s.e, "/val/estado", "r1").unwrap().unwrap().0.intentos.len(), 1);
}

#[test]
fn las_claves_son_estables_y_el_diario_no_guarda_pids() {
    let mut s = escenario(3);
    correr(&mut s, false, 3).unwrap();
    let d = cargar_diario(&s.e, "/val/estado", "r1").unwrap();
    let claves: Vec<&str> = d.entradas.iter().map(|e| e.clave.as_str()).collect();
    assert_eq!(claves, ["r1/lanzar/0", "r1/lanzar/0", "r1/exportar/0", "r1/exportar/0", "r1/validar/0", "r1/validar/0"]);
    let crudo = serde_json::to_string(&d).unwrap();
    assert!(!crudo.contains("pid"), "la identidad es el run id y la clave, no un PID reciclable");
}
