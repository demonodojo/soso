//! Conocimiento verificado entre tareas (T28).
//!
//! La siguiente tarea recibe **sólo notas pertinentes, con procedencia y estado de vigencia**. Tres reglas:
//!
//! - una nota nace como *hipótesis* (o *limitación*, si el intento falló); sólo pasa a *hecho* si el
//!   **validador confirma la evidencia que cita**: el paquete que dice haber producido tiene que ser el que
//!   la referencia de base de la campaña registra, y cada evidencia citada tiene que existir con su hash;
//! - una nota **caduca** cuando cambia alguno de los archivos de los que habla (se compara el hash que
//!   tenía el archivo al escribirla con el de la base actual); se conserva con su historial, no se borra;
//! - el paquete de contexto **no lleva** verificadores reservados, secretos ni transcripciones: lo que
//!   toca rutas protegidas o huele a credencial se descarta y todo campo se acota.
//!
//! Recuperar es determinista (puntuación por rutas y palabras, empate por id) y respeta un presupuesto de
//! bytes. No hay embeddings ni base vectorial.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use serde::{Deserialize, Serialize};

use crate::captura::ArchivoBase;
use crate::durable::{self, Durable};
use crate::entorno::Archivos;
use crate::state::TaskSpec;
use crate::validate::BaseVigente;
use crate::{sha256_hex, Error, Resultado};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Certeza {
    /// Confirmada por el validador con la evidencia citada.
    Hecho,
    /// Lo que se cree tras un intento, sin confirmar.
    Hipotesis,
    /// Lo que **no** funcionó o lo que no se pudo probar.
    Limitacion,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Evidencia {
    /// Archivo (ruta) que contiene la evidencia.
    pub ruta: String,
    /// Su sha256 cuando se citó.
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Evento {
    pub que: String,
    /// Huella de la base en ese momento.
    pub base: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Nota {
    pub id: String,
    pub problema: String,
    pub solucion: String,
    /// Rutas (y, tras `::`, símbolos) de las que habla.
    pub rutas: Vec<String>,
    pub pruebas: Vec<String>,
    pub certeza: Certeza,
    /// Huella de la base sobre la que se escribió.
    pub base: String,
    /// Hash del paquete candidato del que sale (vacío si no hubo).
    pub candidato: String,
    /// Hash que tenía cada ruta al escribir la nota: sirve para saber si caducó.
    pub hashes: BTreeMap<String, String>,
    pub evidencias: Vec<Evidencia>,
    pub vigente: bool,
    pub historial: Vec<Evento>,
}

/// Campo máximo: una nota es un resumen, no un registro (impide colar transcripciones).
pub const CAMPO_MAX: usize = 600;

fn acotar(t: &str) -> String {
    if t.len() <= CAMPO_MAX {
        return t.to_string();
    }
    let mut c = CAMPO_MAX;
    while !t.is_char_boundary(c) {
        c -= 1;
    }
    format!("{}… (acotado)", &t[..c])
}

/// Candidata a nota a partir de un intento. **Nunca** nace como hecho.
pub fn candidata(
    problema: &str,
    solucion: &str,
    rutas: &[String],
    pruebas: &[String],
    base: &[ArchivoBase],
    huella_base: &str,
    candidato: &str,
    evidencias: Vec<Evidencia>,
    intento_fallido: bool,
) -> Nota {
    let hashes: BTreeMap<String, String> = rutas
        .iter()
        .filter_map(|r| {
            let ruta = r.split("::").next().unwrap_or(r);
            base.iter().find(|a| a.ruta == ruta).map(|a| (ruta.to_string(), a.sha256.clone()))
        })
        .collect();
    let id = sha256_hex(format!("{huella_base}\0{candidato}\0{problema}\0{solucion}").as_bytes())[..16].to_string();
    Nota {
        id,
        problema: acotar(problema),
        solucion: acotar(solucion),
        rutas: rutas.to_vec(),
        pruebas: pruebas.iter().map(|p| acotar(p)).collect(),
        certeza: if intento_fallido { Certeza::Limitacion } else { Certeza::Hipotesis },
        base: huella_base.to_string(),
        candidato: candidato.to_string(),
        hashes,
        evidencias,
        vigente: true,
        historial: alloc::vec![Evento { que: if intento_fallido { "propuesta tras intento fallido".into() } else { "propuesta tras intento".into() }, base: huella_base.to_string() }],
    }
}

/// ¿Puede esta nota pasar a hecho? Sólo si el validador respalda lo que cita.
///
/// `vigente` es la referencia de base de la campaña (la escribe el validador, T26). La nota debe hablar del
/// paquete que **esa referencia** registra y cada evidencia debe existir con el hash citado.
pub fn confirmar<A: Archivos>(n: &mut Nota, a: &A, vigente: Option<&BaseVigente>) -> Resultado<()> {
    if n.certeza == Certeza::Limitacion {
        return Err(Error::uso("una limitación no se confirma como hecho: documenta lo que no funcionó"));
    }
    let Some(v) = vigente else {
        return Err(Error::uso("no hay base de campaña: nada respalda la nota"));
    };
    if n.candidato.is_empty() || v.paquete != n.candidato {
        return Err(Error::uso(format!(
            "el validador no respalda esta nota: la base vigente la produjo el paquete «{}», la nota cita «{}»",
            v.paquete, n.candidato
        )));
    }
    if n.evidencias.is_empty() {
        return Err(Error::uso("una nota sin evidencia citada no es un hecho"));
    }
    for e in &n.evidencias {
        let datos = a.leer(&e.ruta).map_err(|_| Error::uso(format!("falta la evidencia citada: {}", e.ruta)))?;
        if sha256_hex(&datos) != e.sha256 {
            return Err(Error::uso(format!("la evidencia {} ya no es la que se citó (hash distinto)", e.ruta)));
        }
    }
    n.certeza = Certeza::Hecho;
    n.historial.push(Evento { que: "confirmada por el validador".into(), base: v.huella.clone() });
    Ok(())
}

/// Marca como desactualizadas las notas cuyos archivos cambiaron en la base actual. Devuelve cuántas.
pub fn invalidar_por_base(notas: &mut [Nota], base_actual: &[ArchivoBase], huella_actual: &str) -> usize {
    let mut n = 0;
    for nota in notas.iter_mut().filter(|x| x.vigente) {
        let cambiada = nota.hashes.iter().find(|(ruta, h)| match base_actual.iter().find(|a| &a.ruta == *ruta) {
            None => true,
            Some(a) => &a.sha256 != *h,
        });
        if let Some((ruta, _)) = cambiada {
            nota.vigente = false;
            nota.historial.push(Evento { que: format!("desactualizada: cambió {ruta}"), base: huella_actual.to_string() });
            n += 1;
        }
    }
    n
}

/// Rutas que nunca viajan al contexto de un agente.
#[derive(Debug, Clone, Default)]
pub struct Reservado {
    pub prefijos: Vec<String>,
}

const HUELLAS_SECRETO: [&str; 8] = ["BEGIN PRIVATE KEY", "BEGIN RSA", "Bearer ", "api_key", "apikey", "secret", "password", "token="];

fn huele_a_secreto(t: &str) -> bool {
    let b = t.to_ascii_lowercase();
    HUELLAS_SECRETO.iter().any(|h| b.contains(&h.to_ascii_lowercase()))
}

/// Nombres de archivo que típicamente guardan credenciales.
fn archivo_secreto(n: &str) -> bool {
    let m = n.to_ascii_lowercase();
    m.starts_with(".env") || m.ends_with(".pem") || m.ends_with(".key") || m.ends_with(".p12") || m.contains("id_rsa") || m.contains("id_ed25519") || m.contains("credentials") || m == ".netrc"
}

fn permitida(n: &Nota, r: &Reservado) -> core::result::Result<(), &'static str> {
    let todo = [&n.problema, &n.solucion];
    if todo.iter().any(|t| huele_a_secreto(t)) || n.pruebas.iter().any(|t| huele_a_secreto(t)) {
        return Err("parece contener un secreto");
    }
    for ruta in &n.rutas {
        let r0 = ruta.split("::").next().unwrap_or(ruta);
        if archivo_secreto(r0.rsplit('/').next().unwrap_or(r0)) {
            return Err("habla de un archivo secreto");
        }
        if r.prefijos.iter().any(|p| r0 == p.trim_end_matches('/') || r0.starts_with(&format!("{}/", p.trim_end_matches('/')))) {
            return Err("habla de un verificador o caso reservado");
        }
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Seleccion {
    pub id: String,
    pub certeza: Certeza,
    pub vigente: bool,
    pub texto: String,
    pub puntuacion: usize,
}

#[derive(Debug, Clone, Default)]
pub struct Recuperado {
    pub notas: Vec<Seleccion>,
    /// Descartadas por reservadas o secretas, con el motivo.
    pub excluidas: Vec<(String, String)>,
    /// Pertinentes que no cupieron en el presupuesto.
    pub fuera_de_presupuesto: usize,
}

fn palabras(t: &str) -> Vec<String> {
    let mut v: Vec<String> = t
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|w| w.len() >= 4)
        .map(|w| w.to_ascii_lowercase())
        .collect();
    v.sort();
    v.dedup();
    v
}

fn bajo(ruta: &str, prefijo: &str) -> bool {
    let p = prefijo.trim_end_matches('/');
    ruta == p || ruta.starts_with(&format!("{p}/"))
}

/// Notas pertinentes para `spec`, en orden determinista y dentro de `presupuesto` bytes.
///
/// Entran los *hechos vigentes* y las *limitaciones vigentes*; las hipótesis y lo caducado no llegan al
/// agente (se pueden listar aparte, con su estado, pero no como contexto).
pub fn recuperar(notas: &[Nota], spec: &TaskSpec, reservado: &Reservado, presupuesto: usize) -> Recuperado {
    let clave = palabras(&spec.problema);
    let mut puntuadas: Vec<(usize, &Nota)> = Vec::new();
    let mut r = Recuperado::default();
    for n in notas {
        if !n.vigente || n.certeza == Certeza::Hipotesis {
            continue;
        }
        if let Err(m) = permitida(n, reservado) {
            r.excluidas.push((n.id.clone(), m.to_string()));
            continue;
        }
        let mut p = 0usize;
        for ruta in &n.rutas {
            let r0 = ruta.split("::").next().unwrap_or(ruta);
            if spec.rutas_editables.iter().any(|e| bajo(r0, e) || bajo(e, r0)) {
                p += 10;
            }
        }
        let texto = palabras(&format!("{} {}", n.problema, n.solucion));
        p += clave.iter().filter(|w| texto.contains(*w)).count();
        if p > 0 {
            puntuadas.push((p, n));
        }
    }
    puntuadas.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.id.cmp(&b.1.id)));
    let mut usado = 0usize;
    for (p, n) in puntuadas {
        let texto = format!(
            "[{}] {} · {} (base {}, paquete {})\nProblema: {}\nSolución: {}\nArchivos: {}\nPruebas: {}",
            n.id,
            match n.certeza {
                Certeza::Hecho => "HECHO verificado",
                Certeza::Limitacion => "LIMITACIÓN",
                Certeza::Hipotesis => "HIPÓTESIS",
            },
            if n.vigente { "vigente" } else { "desactualizada" },
            &n.base[..n.base.len().min(12)],
            if n.candidato.is_empty() { "-" } else { &n.candidato[..n.candidato.len().min(12)] },
            n.problema,
            n.solucion,
            n.rutas.join(", "),
            n.pruebas.join("; ")
        );
        if usado + texto.len() > presupuesto {
            r.fuera_de_presupuesto += 1;
            continue;
        }
        usado += texto.len();
        r.notas.push(Seleccion { id: n.id.clone(), certeza: n.certeza, vigente: n.vigente, texto, puntuacion: p });
    }
    r
}

// ----------------------------------------------------------------------------- persistencia

const NOMBRE: &str = "conocimiento";

pub fn cargar<D: Archivos>(d: &D, dir: &str) -> Resultado<Vec<Nota>> {
    if !d.existe(dir) {
        return Ok(Vec::new());
    }
    match durable::leer_vigente(d, dir, NOMBRE)? {
        None => Ok(Vec::new()),
        Some((_, v)) => serde_json::from_slice(&v).map_err(|e| Error::formato(format!("notas ilegibles: {e}"))),
    }
}

/// Publica el conjunto de notas por generaciones (nunca se pierde el anterior hasta tener el nuevo).
pub fn guardar<D: Durable>(d: &mut D, dir: &str, notas: &[Nota]) -> Resultado<u64> {
    if !d.existe(dir) {
        d.crear_directorio(dir)?;
    }
    let datos = serde_json::to_vec(notas).map_err(Error::formato)?;
    let actual = durable::leer_vigente(&*d, dir, NOMBRE)?.map(|(_, v)| sha256_hex(&v));
    let g = durable::publicar(d, dir, NOMBRE, &datos, actual.as_deref())?;
    let _ = durable::purgar(d, dir, NOMBRE, g.saturating_sub(3));
    Ok(g)
}

/// Añade una nota si no estaba (mismo id = misma nota).
pub fn añadir(notas: &mut Vec<Nota>, n: Nota) -> bool {
    if notas.iter().any(|x| x.id == n.id) {
        return false;
    }
    notas.push(n);
    true
}
