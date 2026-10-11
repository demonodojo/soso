//! Validador de candidatos y promoción de la base (T26).
//!
//! **La aprobación depende de evidencia externa, no de lo que diga el modelo.** El validador:
//!
//! 1. recibe un paquete por contenido (T50) y comprueba su **hash exacto** si se le dio uno;
//! 2. rechaza cambios fuera de las rutas editables de la tarea, sobre rutas protegidas (verificadores,
//!    casos reservados) y el borrado de pruebas;
//! 3. reconstruye la base **desde la captura** (no desde el árbol del agente) y aplica el paquete;
//! 4. ejecuta los requisitos que vienen de una especificación **fuera del alcance del candidato** y
//!    decide **sólo por códigos de salida**: ninguna palabra «OK» de una salida cuenta;
//! 5. si el paquete añade o cambia pruebas, las ejecuta **sobre la base** y exige que fallen: una
//!    prueba que ya pasa sin el arreglo no demuestra nada;
//! 6. sólo con todo en verde escribe `aceptada` y mueve la referencia de la base con una operación
//!    durable (T46) condicionada al hash anterior. Dos promociones simultáneas: gana una.
//!
//! Un check que no se pudo ejecutar (sin código: plazo, no arrancó) **bloquea**, no rechaza ni
//! acepta: «no se pudo medir» y «medí y falló» son desenlaces distintos.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use serde::{Deserialize, Serialize};

use crate::captura::{self, ruta_objeto, ArchivoBase, Manifiesto};
use crate::delta::{self, Operacion, Paquete};
use crate::durable::{self, Durable};
use crate::entorno::{Entorno, Orden};
use crate::state::{self, Autoridad, Estado, TaskSpec};
use crate::{sha256_hex, unir, Error, Resultado};

/// Un check obligatorio, con argv y cwd explícitos (nunca una línea de shell).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Requisito {
    pub nombre: String,
    pub argv: Vec<String>,
    /// Relativo a la raíz de validación.
    pub cwd: String,
    pub timeout_s: u32,
}

/// Lo que el validador exige, y que **no** sale del candidato.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Exigencias {
    pub requisitos: Vec<Requisito>,
    /// Prefijos de ruta que ningún candidato puede tocar (verificadores, casos reservados).
    pub rutas_protegidas: Vec<String>,
    /// Check de regresión: se ejecuta sobre la base con las pruebas del candidato y **debe fallar**.
    pub regresion: Option<Requisito>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Veredicto {
    Aceptado,
    /// Medido y no pasa.
    Rechazado,
    /// No se pudo medir, o la base cambió: el candidato se conserva y no se promueve.
    Bloqueado,
}

#[derive(Debug, Clone)]
pub struct Evidencia {
    pub nombre: String,
    pub codigo: Option<i32>,
    pub motivo: Option<String>,
    pub bytes_salida: usize,
}

#[derive(Debug, Clone)]
pub struct InformeValidacion {
    pub veredicto: Veredicto,
    pub razones: Vec<String>,
    pub evidencias: Vec<Evidencia>,
    pub hash_paquete: String,
    /// Generación de la referencia de base si se promovió.
    pub promovido: Option<u64>,
}

impl InformeValidacion {
    fn nuevo(hash: &str) -> Self {
        InformeValidacion { veredicto: Veredicto::Rechazado, razones: Vec::new(), evidencias: Vec::new(), hash_paquete: hash.to_string(), promovido: None }
    }
    fn termina(mut self, v: Veredicto, razon: impl ToString) -> Self {
        self.veredicto = v;
        self.razones.push(razon.to_string());
        self
    }
}

/// Referencia durable de la base de una campaña: qué huella es «la base» ahora.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BaseVigente {
    pub huella: String,
    /// Hash del paquete que la produjo (vacío para la base inicial).
    pub paquete: String,
}

/// Hash del paquete: lo que se compara con el que anunció quien lo envió.
pub fn hash_paquete(p: &Paquete) -> Resultado<String> {
    Ok(sha256_hex(&serde_json::to_vec(p).map_err(Error::formato)?))
}

/// Guarda en `almacen` los objetos que necesita el paquete, leyéndolos de la copia del agente.
/// Cada uno se comprueba contra su hash: lo que el paquete dice y lo que hay en la copia coinciden o no se envía.
pub fn empaquetar_objetos<E: Entorno>(e: &mut E, raiz_copia: &str, paquete: &Paquete, almacen: &str) -> Resultado<usize> {
    let mut n = 0;
    for op in &paquete.operaciones {
        let Some(h) = op.objeto() else { continue };
        let datos = e.leer(&unir(raiz_copia, op.ruta()))?;
        if sha256_hex(&datos) != h {
            return Err(Error::uso(format!("{} cambió después de exportar el paquete (hash distinto)", op.ruta())));
        }
        let destino = unir(almacen, &ruta_objeto(h));
        if !e.existe(&destino) {
            e.escribir(&destino, &datos, 0o644)?;
        }
        n += 1;
    }
    Ok(n)
}

/// Publica la base inicial de una campaña (una sola vez).
pub fn publicar_base<D: Durable>(d: &mut D, dir: &str, campana: &str, huella: &str) -> Resultado<u64> {
    if !d.existe(dir) {
        d.crear_directorio(dir)?;
    }
    let b = BaseVigente { huella: huella.to_string(), paquete: String::new() };
    durable::publicar(d, dir, &nombre_base(campana), &serde_json::to_vec(&b).map_err(Error::formato)?, None)
}

pub fn base_vigente<D: crate::entorno::Archivos>(d: &D, dir: &str, campana: &str) -> Resultado<Option<(BaseVigente, String)>> {
    if !d.existe(dir) {
        return Ok(None);
    }
    match durable::leer_vigente(d, dir, &nombre_base(campana))? {
        None => Ok(None),
        Some((_, datos)) => {
            let b: BaseVigente = serde_json::from_slice(&datos).map_err(Error::formato)?;
            Ok(Some((b, sha256_hex(&datos))))
        }
    }
}

fn nombre_base(campana: &str) -> String {
    format!("base-{campana}")
}

fn es_prueba(ruta: &str) -> bool {
    ruta.starts_with("tests/") || ruta.contains("/tests/") || ruta.rsplit('/').next().map_or(false, |n| n.starts_with("test_"))
}

fn bajo(ruta: &str, prefijo: &str) -> bool {
    let p = prefijo.trim_end_matches('/');
    ruta == p || ruta.starts_with(&format!("{p}/"))
}

/// Todo lo que el validador necesita saber de **dónde** está cada cosa en su propio sistema de archivos.
pub struct Sitios<'a> {
    /// Captura de la base (manifiesto + objetos).
    pub captura: &'a str,
    /// Objetos del candidato (`empaquetar_objetos`).
    pub objetos: &'a str,
    /// Directorio **vacío o inexistente** donde se reconstruye la base para validar.
    pub arbol: &'a str,
    /// Directorio de referencias durables de la campaña.
    pub referencias: &'a str,
    pub campana: &'a str,
}

/// Valida un candidato y, sólo si procede, promueve la base.
///
/// `estado` es `(directorio_de_estado, run_id)` si el candidato viene de una ejecución de T23: el
/// validador es quien escribe `aceptada` o `rechazada`.
#[allow(clippy::too_many_arguments)]
pub fn validar_candidato<V: Entorno + Durable>(
    v: &mut V,
    s: &Sitios<'_>,
    paquete: &Paquete,
    hash_esperado: Option<&str>,
    spec: &TaskSpec,
    ex: &Exigencias,
    estado: Option<(&str, &str)>,
) -> Resultado<InformeValidacion> {
    let hash = hash_paquete(paquete)?;
    let mut inf = InformeValidacion::nuevo(&hash);
    let fin = |v: &mut V, inf: InformeValidacion| -> Resultado<InformeValidacion> {
        if let Some((dir, run)) = estado {
            cerrar_estado(v, dir, run, &inf)?;
        }
        Ok(inf)
    };

    // 1. Identidad exacta del candidato.
    if let Some(h) = hash_esperado {
        if h != hash {
            let inf = inf.termina(Veredicto::Rechazado, format!("el paquete recibido es {hash}, no el anunciado {h}"));
            return fin(v, inf);
        }
    }
    // 2. Alcance.
    let mut fuera = Vec::new();
    for op in &paquete.operaciones {
        let r = op.ruta();
        if ex.rutas_protegidas.iter().any(|p| bajo(r, p)) {
            fuera.push(format!("{r}: ruta protegida (verificador o caso reservado)"));
        } else if !spec.rutas_editables.iter().any(|p| bajo(r, p)) {
            fuera.push(format!("{r}: fuera de las rutas editables de la tarea"));
        } else if matches!(op, Operacion::Borrado { .. }) && es_prueba(r) {
            fuera.push(format!("{r}: borrar una prueba no es una forma de pasarlas"));
        }
    }
    if !fuera.is_empty() {
        let mut inf = inf;
        inf.razones.extend(fuera);
        let inf = inf.termina(Veredicto::Rechazado, "el candidato toca lo que no debe");
        return fin(v, inf);
    }
    // 3. La base no se movió desde que se hizo el candidato.
    let Some((vigente, hash_ref)) = base_vigente(&*v, s.referencias, s.campana)? else {
        let inf = inf.termina(Veredicto::Bloqueado, "la campaña no tiene base publicada");
        return fin(v, inf);
    };
    if vigente.huella != paquete.base {
        let inf = inf.termina(Veredicto::Bloqueado, format!("la base cambió: el candidato parte de {}, la vigente es {}", paquete.base, vigente.huella));
        return fin(v, inf);
    }
    // 4. Reconstruir la base desde la captura y comprobar que es la que dice la referencia.
    let man: Manifiesto = captura::leer_manifiesto(&*v, s.captura)?;
    if captura::huella(&man.inventario) != vigente.huella {
        let inf = inf.termina(Veredicto::Bloqueado, "la captura no es la base vigente");
        return fin(v, inf);
    }
    if v.existe(s.arbol) && !v.listar(s.arbol)?.is_empty() {
        return Err(Error::uso(format!("{} no está vacío: el validador no reutiliza árboles", s.arbol)));
    }
    if !v.existe(s.arbol) {
        v.crear_directorio(s.arbol)?;
    }
    reconstruir_base(v, s, &man)?;

    // 5. Regresión: las pruebas del candidato, sobre la base, deben fallar.
    let toca_pruebas = paquete.operaciones.iter().any(|o| o.objeto().is_some() && es_prueba(o.ruta()));
    if toca_pruebas {
        if let Some(reg) = &ex.regresion {
            for op in &paquete.operaciones {
                if let (Some(h), true) = (op.objeto(), es_prueba(op.ruta())) {
                    let datos = v.leer(&unir(s.objetos, &ruta_objeto(h)))?;
                    v.escribir(&unir(s.arbol, op.ruta()), &datos, 0o644)?;
                }
            }
            let e = ejecutar(v, s.arbol, reg);
            let bloquea = e.codigo.is_none();
            let pasa = e.codigo == Some(0);
            inf.evidencias.push(Evidencia { nombre: format!("{} (sobre la base)", reg.nombre), ..e });
            if bloquea {
                let inf = inf.termina(Veredicto::Bloqueado, "no se pudo ejecutar la regresión sobre la base");
                return fin(v, inf);
            }
            if pasa {
                let inf = inf.termina(Veredicto::Rechazado, "las pruebas del candidato pasan sobre la base: no detectan el defecto");
                return fin(v, inf);
            }
            // Se vuelve a una base limpia para validar el candidato entero.
            limpiar(v, s.arbol)?;
            reconstruir_base(v, s, &man)?;
        } else {
            let inf = inf.termina(Veredicto::Bloqueado, "el candidato cambia pruebas y no hay check de regresión definido");
            return fin(v, inf);
        }
    }
    // 6. Aplicar el paquete (valida hashes y huella destino antes de escribir).
    if let Err(err) = aplicar_paquete(v, s, &man.inventario, paquete) {
        let inf = inf.termina(Veredicto::Rechazado, format!("el paquete no aplica sobre la base: {err}"));
        return fin(v, inf);
    }
    // 7. Requisitos independientes; sólo cuentan los códigos de salida.
    let (mut falla, mut sin_medir) = (Vec::new(), Vec::new());
    for r in &ex.requisitos {
        let e = ejecutar(v, s.arbol, r);
        match e.codigo {
            Some(0) => {}
            Some(c) => falla.push(format!("{}: salió con {c}", r.nombre)),
            None => sin_medir.push(format!("{}: {}", r.nombre, e.motivo.clone().unwrap_or_else(|| "no terminó".into()))),
        }
        inf.evidencias.push(e);
    }
    if ex.requisitos.is_empty() {
        let inf = inf.termina(Veredicto::Bloqueado, "no hay requisitos: sin comprobaciones no se puede aceptar nada");
        return fin(v, inf);
    }
    if !sin_medir.is_empty() {
        inf.razones.extend(sin_medir);
        let inf = inf.termina(Veredicto::Bloqueado, "hay checks que no se pudieron medir");
        return fin(v, inf);
    }
    if !falla.is_empty() {
        inf.razones.extend(falla);
        let inf = inf.termina(Veredicto::Rechazado, "fallan requisitos");
        return fin(v, inf);
    }
    // 8. Promoción: operación durable condicionada a la base que se vio.
    let nueva = BaseVigente { huella: paquete.destino.clone(), paquete: hash.clone() };
    let datos = serde_json::to_vec(&nueva).map_err(Error::formato)?;
    match durable::publicar(v, s.referencias, &nombre_base(s.campana), &datos, Some(&hash_ref)) {
        Ok(g) => {
            inf.promovido = Some(g);
            let inf = inf.termina(Veredicto::Aceptado, "todos los requisitos en verde; base promovida");
            fin(v, inf)
        }
        Err(err) => {
            // Otro candidato promovió antes: éste se conserva, no se pierde ni pisa nada.
            let inf = inf.termina(Veredicto::Bloqueado, format!("la base cambió durante la validación: {err}"));
            fin(v, inf)
        }
    }
}

fn ejecutar<V: Entorno>(v: &V, raiz: &str, r: &Requisito) -> Evidencia {
    let cwd = if r.cwd.is_empty() || r.cwd == "." { raiz.to_string() } else { unir(raiz, &r.cwd) };
    // `{arbol}` en el argv es la raíz de validación: así un requisito puede apuntar el verificador (que
    // vive fuera del candidato) al árbol reconstruido, sin ejecutar nada en una shell.
    let argv: Vec<String> = r.argv.iter().map(|a| a.replace("{arbol}", raiz)).collect();
    let o = Orden { argv, cwd, stdin: Vec::new(), timeout_s: Some(r.timeout_s), entorno: Vec::new() };
    match v.ejecutar(&o) {
        Ok(s) => Evidencia { nombre: r.nombre.clone(), codigo: s.codigo, motivo: s.motivo.clone(), bytes_salida: s.stdout.len() + s.stderr.len() },
        Err(e) => Evidencia { nombre: r.nombre.clone(), codigo: None, motivo: Some(e.to_string()), bytes_salida: 0 },
    }
}

fn reconstruir_base<V: Entorno>(v: &mut V, s: &Sitios<'_>, man: &Manifiesto) -> Resultado<()> {
    // Lectura y escritura sobre el mismo entorno: se leen los objetos uno a uno.
    for a in &man.inventario {
        crate::ruta_segura(&a.ruta)?;
        if man.no_almacenados.iter().any(|r| *r == a.ruta) {
            continue;
        }
        let datos = v.leer(&unir(s.captura, &ruta_objeto(&a.sha256)))?;
        if sha256_hex(&datos) != a.sha256 {
            return Err(Error::formato(format!("el objeto de {} no cuadra con su hash", a.ruta)));
        }
        v.escribir(&unir(s.arbol, &a.ruta), &datos, a.modo)?;
    }
    Ok(())
}

fn limpiar<V: Entorno>(v: &mut V, raiz: &str) -> Resultado<()> {
    fn rec<V: Entorno>(v: &mut V, dir: &str) -> Resultado<()> {
        for e in v.listar(dir)? {
            let r = unir(dir, &e.ruta);
            if e.tipo == crate::entorno::Tipo::Directorio {
                rec(v, &r)?;
            }
            v.borrar(&r)?;
        }
        Ok(())
    }
    rec(v, raiz)
}

/// Como `delta::aplicar`, pero con el almacén y el árbol en el mismo entorno (el del validador).
fn aplicar_paquete<V: Entorno>(v: &mut V, s: &Sitios<'_>, base: &[ArchivoBase], paquete: &Paquete) -> Resultado<()> {
    // Valida entero (esquema, base, objetos presentes, huella destino) antes de escribir nada.
    delta::validar(&*v, s.objetos, base, paquete)?;
    for op in &paquete.operaciones {
        let Some(h) = op.objeto() else { continue };
        let datos = v.leer(&unir(s.objetos, &ruta_objeto(h)))?;
        if sha256_hex(&datos) != h {
            return Err(Error::formato(format!("el objeto {h} no cuadra con su hash")));
        }
        let modo = match op {
            Operacion::Alta { modo, .. } | Operacion::Modificacion { modo, .. } => *modo,
            Operacion::Borrado { .. } => 0,
        };
        v.escribir(&unir(s.arbol, op.ruta()), &datos, modo)?;
    }
    for op in &paquete.operaciones {
        if let Operacion::Borrado { ruta, .. } = op {
            v.borrar(&unir(s.arbol, ruta))?;
        }
    }
    Ok(())
}

/// Escribe el desenlace del validador en el estado de la ejecución (idempotente: si ya consta, no hace nada).
pub fn marcar_veredicto<V: Entorno + Durable>(v: &mut V, dir: &str, run: &str, ver: &Veredicto) -> Resultado<()> {
    let mut inf = InformeValidacion::nuevo("");
    inf.veredicto = ver.clone();
    cerrar_estado(v, dir, run, &inf)
}

fn cerrar_estado<V: Entorno + Durable>(v: &mut V, dir: &str, run: &str, inf: &InformeValidacion) -> Resultado<()> {
    let Some((mut m, hash)) = state::cargar(&*v, dir, run)? else {
        return Err(Error::uso(format!("no hay ninguna ejecución «{run}»")));
    };
    let a = match inf.veredicto {
        Veredicto::Aceptado => Estado::Aceptada,
        Veredicto::Rechazado => Estado::Rechazada,
        Veredicto::Bloqueado => Estado::Bloqueada,
    };
    if m.estado != a {
        m.transicion(a, Autoridad::Validador)?;
        state::guardar(v, dir, &m, Some(&hash))?;
    }
    Ok(())
}

/// Línea legible por la consola serie.
pub fn resumen(i: &InformeValidacion) -> String {
    let mut t = format!("validar: {:?} (paquete {})\n", i.veredicto, i.hash_paquete);
    for e in &i.evidencias {
        t.push_str(&format!("  {} → {}\n", e.nombre, e.codigo.map_or(format!("sin código ({})", e.motivo.as_deref().unwrap_or("?")), |c| c.to_string())));
    }
    for r in &i.razones {
        t.push_str(&format!("  · {r}\n"));
    }
    if let Some(g) = i.promovido {
        t.push_str(&format!("  base promovida (generación {g})\n"));
    }
    t
}
