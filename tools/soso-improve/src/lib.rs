//! `soso-improve` — coordinador de automejora en el host (C5).
//!
//! Sustituye a los scripts de Python de T01 y T02. La lógica vive en
//! `soso-improve-core`, que es `no_std + alloc`; aquí solo está lo que necesita
//! `std`: archivos, procesos y git. Ese reparto es el objetivo: que lo mismo
//! pueda hospedarse dentro de soso cuando tenga compilador y cargo.
//!
//! ```text
//! soso-improve capturar    --repo <r> --out <o> [--sin-herramientas]
//! soso-improve reconstruir --captura <c> --destino <d>
//! soso-improve suites      --captura <c> --arbol <a> [--solo N] [--timeout S]
//! soso-improve banco       listar|validar|sellar [--banco <d>] [--reservado <d>]
//! soso-improve verificar   programa  --caso P01 --candidato f.rs
//! soso-improve verificar   protocolo --caso Q01 --respuesta r.json
//! soso-improve verificar   repo      --caso R01 --arbol <a> [--con-referencia]
//! ```
//!
//! Códigos de salida: `0` bien, `1` no se pudo usar, `2` falla la medida,
//! `3` captura inestable, `4` alguna suite falló.

/// `println!` revienta con EPIPE cuando el lector se va (`| head`): escribe y
/// calla si el otro extremo cerró la tubería.
#[macro_export]
macro_rules! digo {
    ($($arg:tt)*) => {{
        use std::io::Write;
        let _ = writeln!(std::io::stdout(), $($arg)*);
    }};
}

/// Lo mismo para los avisos.
#[macro_export]
macro_rules! aviso {
    ($($arg:tt)*) => {{
        use std::io::Write;
        let _ = writeln!(std::io::stderr(), $($arg)*);
    }};
}

pub mod banco;
pub mod captura;
pub mod git;
pub mod modelo;
pub mod sistema;
pub mod verificar;

use std::collections::BTreeMap;

use soso_improve_core::Error;

pub const USO: &str = "\
uso: soso-improve <orden> [opciones]

  capturar    --repo <ruta> --out <ruta> [--sin-herramientas]
  reconstruir --captura <ruta> --destino <ruta>
  suites      --captura <ruta> --arbol <ruta> [--solo <nombre>] [--timeout <s>]
  banco       listar|validar|sellar [--banco <ruta>] [--reservado <ruta>]
  modelo      perfil    --modelo <dir .som> --original <dir> --out <ruta>
              comparar  --modelo <dir .som> [--fixtures <dir>] [--json <ruta>]
              detalle   --modelo <dir .som> --caso <nombre> [--desde N] [--n N]
  verificar   programa  --caso <id> --candidato <ruta> [--reservado <ruta>]
              protocolo --caso <id> --respuesta <ruta> [--reservado <ruta>]
              repo      --caso <id> --arbol <ruta> [--con-referencia]
";

/// Opciones `--clave valor` y banderas `--clave`, sin dependencias.
pub struct Opciones {
    pub valores: BTreeMap<String, Vec<String>>,
}

impl Opciones {
    pub fn parsear(args: &[String]) -> Result<Opciones, Error> {
        let mut valores: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut i = 0;
        while i < args.len() {
            let arg = &args[i];
            let clave = match arg.strip_prefix("--") {
                Some(c) => c.to_string(),
                None => return Err(Error::uso(format!("argumento suelto: {arg}"))),
            };
            let siguiente = args.get(i + 1);
            match siguiente {
                Some(v) if !v.starts_with("--") => {
                    valores.entry(clave).or_default().push(v.clone());
                    i += 2;
                }
                _ => {
                    valores.entry(clave).or_default().push(String::new());
                    i += 1;
                }
            }
        }
        Ok(Opciones { valores })
    }

    pub fn uno(&self, clave: &str) -> Option<&str> {
        self.valores
            .get(clave)
            .and_then(|v| v.first())
            .map(|s| s.as_str())
    }

    pub fn exigido(&self, clave: &str) -> Result<&str, Error> {
        self.uno(clave)
            .filter(|v| !v.is_empty())
            .ok_or_else(|| Error::uso(format!("falta --{clave}")))
    }

    pub fn bandera(&self, clave: &str) -> bool {
        self.valores.contains_key(clave)
    }

    pub fn varios(&self, clave: &str) -> Vec<String> {
        self.valores.get(clave).cloned().unwrap_or_default()
    }
}

pub fn despachar(orden: &str, args: &[String]) -> Result<i32, Error> {
    match orden {
        "capturar" => captura::capturar(&Opciones::parsear(args)?),
        "reconstruir" => captura::reconstruir(&Opciones::parsear(args)?),
        "suites" => captura::suites(&Opciones::parsear(args)?),
        "banco" => {
            let sub = args
                .first()
                .ok_or_else(|| Error::uso("banco necesita listar|validar|sellar"))?;
            banco::despachar(sub, &Opciones::parsear(&args[1..])?)
        }
        "modelo" => {
            let sub = args
                .first()
                .ok_or_else(|| Error::uso("modelo necesita un subcomando (comparar)"))?;
            modelo::despachar(sub, &Opciones::parsear(&args[1..])?)
        }
        "verificar" => {
            let sub = args
                .first()
                .ok_or_else(|| Error::uso("verificar necesita programa|protocolo|repo"))?;
            verificar::despachar(sub, &Opciones::parsear(&args[1..])?)
        }
        otro => Err(Error::uso(format!("orden desconocida: {otro}"))),
    }
}
