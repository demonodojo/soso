//! Lo único que esta lógica necesita del sistema, expresado como traits.
//!
//! El host los implementa con `std::fs` y `std::process`; el guest los
//! implementará con las llamadas que soso ya tiene (`spawn_io`, `wait`, y las
//! de ficheros). Mientras la lógica hable solo por aquí, el mismo código
//! compila para las dos partes.

use alloc::string::String;
use alloc::vec::Vec;

use crate::Resultado;

/// Qué es una entrada de directorio. `Otro` cubre enlaces y todo lo que no sea
/// archivo regular ni directorio: se registra, no se copia.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tipo {
    Archivo,
    Directorio,
    Otro,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entrada {
    /// Ruta relativa a la raíz del árbol, con `/` como separador.
    pub ruta: String,
    pub tipo: Tipo,
    pub bytes: u64,
    /// Permisos POSIX (solo los 9 bits bajos). 0 si el sistema no los expone.
    pub modo: u32,
}

/// Acceso a un árbol de archivos.
pub trait Archivos {
    fn leer(&self, ruta: &str) -> Resultado<Vec<u8>>;
    fn escribir(&mut self, ruta: &str, datos: &[u8], modo: u32) -> Resultado<()>;
    fn existe(&self, ruta: &str) -> bool;
    /// Entradas de un directorio, sin recursión y sin orden garantizado.
    fn listar(&self, ruta: &str) -> Resultado<Vec<Entrada>>;
    fn metadatos(&self, ruta: &str) -> Resultado<Entrada>;
    fn crear_directorio(&mut self, ruta: &str) -> Resultado<()>;
    fn borrar(&mut self, ruta: &str) -> Resultado<()>;
}

/// Orden que se le da a un proceso externo. `argv` sin shell: nada de
/// interpolar datos de un modelo en una línea de órdenes (C5).
#[derive(Debug, Clone)]
pub struct Orden {
    pub argv: Vec<String>,
    /// Directorio de trabajo, relativo a la raíz del árbol o absoluto.
    pub cwd: String,
    pub stdin: Vec<u8>,
    pub timeout_s: Option<u32>,
    /// Variables que se **añaden** al entorno del proceso. Nunca se vuelca el
    /// entorno entero ni se copian secretos.
    pub entorno: Vec<(String, String)>,
}

impl Orden {
    pub fn nueva(argv: &[&str], cwd: &str) -> Self {
        Orden {
            argv: argv.iter().map(|s| String::from(*s)).collect(),
            cwd: String::from(cwd),
            stdin: Vec::new(),
            timeout_s: None,
            entorno: Vec::new(),
        }
    }

    pub fn con_stdin(mut self, datos: &[u8]) -> Self {
        self.stdin = datos.to_vec();
        self
    }

    pub fn con_timeout(mut self, segundos: u32) -> Self {
        self.timeout_s = Some(segundos);
        self
    }

    pub fn con_variable(mut self, nombre: &str, valor: &str) -> Self {
        self.entorno.push((String::from(nombre), String::from(valor)));
        self
    }
}

/// Resultado de un proceso. `codigo` es `None` cuando no llegó a terminar
/// (timeout) o no se pudo lanzar; `motivo` dice cuál de las dos cosas.
#[derive(Debug, Clone)]
pub struct Salida {
    pub codigo: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub motivo: Option<String>,
}

impl Salida {
    pub fn ok(&self) -> bool {
        self.codigo == Some(0)
    }

    /// Salida unificada para los logs: primero stdout, después stderr.
    pub fn texto(&self) -> String {
        let mut t = String::from_utf8_lossy(&self.stdout).into_owned();
        t.push_str(&String::from_utf8_lossy(&self.stderr));
        t
    }
}

pub trait Procesos {
    fn ejecutar(&self, orden: &Orden) -> Resultado<Salida>;
}

/// Un entorno completo: árbol y procesos. Se pide junto porque casi todo lo
/// que hace el coordinador necesita las dos cosas.
pub trait Entorno: Archivos + Procesos {}

impl<T: Archivos + Procesos> Entorno for T {}
