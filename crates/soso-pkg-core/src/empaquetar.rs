//! Construcción de `paquete.pack` + manifiesto en el host (`std`).

use alloc::string::String;
use alloc::vec::Vec;

use soso_update_core::hex_sha256;

use crate::manifest::Manifest;
use crate::rutas::{nombre_valido, ruta_valida};

#[derive(Debug)]
pub enum EmpaquetarError {
    Nombre(String),
    Ruta(String),
    Vacio(String),
}

impl core::fmt::Display for EmpaquetarError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Nombre(e) => write!(f, "{e}"),
            Self::Ruta(p) => write!(f, "ruta no permitida: {p}"),
            Self::Vacio(p) => write!(f, "fichero vacío: {p}"),
        }
    }
}

/// `(manifiesto texto, blob pack, Manifest parseado)`.
pub fn empaquetar(
    nombre: &str,
    version: &str,
    abi: Option<u32>,
    mut files: Vec<(String, Vec<u8>)>,
) -> Result<(String, Vec<u8>, Manifest), EmpaquetarError> {
    nombre_valido(nombre).map_err(|e| EmpaquetarError::Nombre(format!("{e}")))?;
    files.sort_by(|a, b| a.0.cmp(&b.0));
    let mut blob = Vec::new();
    let mut lines = String::from("SOSOPKG v1\n");
    lines.push_str(&format!("nombre={nombre}\n"));
    lines.push_str(&format!("version={version}\n"));
    if let Some(a) = abi {
        lines.push_str(&format!("abi={a}\n"));
    }
    for (path, data) in &files {
        ruta_valida(path).map_err(|_| EmpaquetarError::Ruta(path.clone()))?;
        if data.is_empty() {
            return Err(EmpaquetarError::Vacio(path.clone()));
        }
        let hash = hex_sha256(data);
        let size = data.len() as u64;
        lines.push_str(&format!("f {hash} {size} {path}\n"));
        blob.extend_from_slice(data);
    }
    let man = Manifest::parse(&lines).expect("manifiesto recién generado");
    Ok((lines, blob, man))
}
