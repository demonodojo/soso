//! Verificador de los casos de programación.
//!
//! Compila el archivo candidato y lo ejecuta contra cada vector reservado en un
//! directorio de trabajo desechable. Un caso pasa solo si **todos** sus vectores
//! coinciden byte a byte en stdout y en código de salida.
//!
//! Ni el compilador ni el ejecutor son cosa de este módulo: entran por los
//! traits de `entorno`. En el host eso es `rustc` + `std::process`; dentro de
//! soso será `soso-rustc` + `spawn_io`, cuando T40 lo haga real.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use serde::{Deserialize, Serialize};

use crate::entorno::{Archivos, Orden, Procesos};
use crate::{unir, Error, Resultado};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Vector {
    pub nombre: String,
    pub stdin: String,
    pub stdout: String,
    #[serde(default)]
    pub exit: i32,
    /// Archivos que se crean en el directorio de trabajo antes de ejecutar.
    #[serde(default)]
    pub archivos: alloc::collections::BTreeMap<String, String>,
    #[serde(default)]
    pub directorios: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Vectores {
    pub vectores: Vec<Vector>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResultadoVector {
    pub vector: String,
    pub paso: bool,
    pub esperado: String,
    pub obtenido: String,
    pub exit_esperado: i32,
    pub exit: Option<i32>,
    pub motivo: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Informe {
    pub caso: String,
    pub estado: String,
    pub pasados: usize,
    pub total: usize,
    pub vectores: Vec<ResultadoVector>,
    pub motivo: Option<String>,
    pub compilacion: Option<String>,
}

impl Informe {
    /// `0` pasa, `2` falla, `1` no se pudo usar. La diferencia importa: un
    /// candidato que no compila no es lo mismo que uno que compila y responde
    /// mal.
    pub fn codigo(&self) -> i32 {
        match self.estado.as_str() {
            "ok" => 0,
            "fallo" => 2,
            _ => 1,
        }
    }
}

fn error(caso: &str, motivo: String, compilacion: Option<String>) -> Informe {
    Informe {
        caso: caso.to_string(),
        estado: "error".to_string(),
        pasados: 0,
        total: 0,
        vectores: Vec::new(),
        motivo: Some(motivo),
        compilacion,
    }
}

/// Compila y ejecuta un candidato contra los vectores del caso.
///
/// `trabajo` es un directorio desechable donde se deja el fuente, el binario y
/// el directorio de cada vector. `compilador` recibe (fuente, destino) y
/// devuelve el argv con el que compilar.
pub fn verificar(
    caso: &str,
    vectores: &Vectores,
    candidato: &[u8],
    arbol: &mut dyn Archivos,
    procesos: &dyn Procesos,
    trabajo: &str,
    compilador: &dyn Fn(&str, &str) -> Vec<String>,
    timeout_s: u32,
) -> Resultado<Informe> {
    let fuente = unir(trabajo, "candidato.rs");
    let binario = unir(trabajo, "candidato");
    arbol.escribir(&fuente, candidato, 0o644)?;

    let argv = compilador(&fuente, &binario);
    let orden = Orden {
        argv,
        cwd: trabajo.to_string(),
        stdin: Vec::new(),
        timeout_s: Some(timeout_s * 4),
        entorno: Vec::new(),
    };
    let compilacion = procesos.ejecutar(&orden)?;
    if !compilacion.ok() {
        return Ok(error(
            caso,
            "no compila".to_string(),
            Some(compilacion.texto()),
        ));
    }

    let mut resultados = Vec::new();
    for (i, vector) in vectores.vectores.iter().enumerate() {
        let dir = unir(trabajo, &format!("vector-{i}"));
        arbol.crear_directorio(&dir)?;
        for d in &vector.directorios {
            crate::ruta_segura(d)?;
            arbol.crear_directorio(&unir(&dir, d))?;
        }
        for (ruta, contenido) in &vector.archivos {
            crate::ruta_segura(ruta)?;
            arbol.escribir(&unir(&dir, ruta), contenido.as_bytes(), 0o644)?;
        }
        let orden = Orden {
            argv: alloc::vec![binario.clone()],
            cwd: dir,
            stdin: vector.stdin.as_bytes().to_vec(),
            timeout_s: Some(timeout_s),
            entorno: Vec::new(),
        };
        let salida = procesos.ejecutar(&orden)?;
        let obtenido = String::from_utf8_lossy(&salida.stdout).into_owned();
        let paso = salida.motivo.is_none()
            && obtenido == vector.stdout
            && salida.codigo == Some(vector.exit);
        resultados.push(ResultadoVector {
            vector: vector.nombre.clone(),
            paso,
            esperado: vector.stdout.clone(),
            obtenido,
            exit_esperado: vector.exit,
            exit: salida.codigo,
            motivo: salida.motivo,
        });
    }

    let pasados = resultados.iter().filter(|r| r.paso).count();
    Ok(Informe {
        caso: caso.to_string(),
        estado: if pasados == resultados.len() {
            "ok".to_string()
        } else {
            "fallo".to_string()
        },
        pasados,
        total: resultados.len(),
        vectores: resultados,
        motivo: None,
        compilacion: None,
    })
}

pub fn leer_vectores(arbol: &dyn Archivos, ruta: &str) -> Resultado<Vectores> {
    let datos = arbol.leer(ruta)?;
    serde_json::from_slice(&datos).map_err(Error::formato)
}
