//! Índice `/var/paquetes/comandos` para sosh.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::manifest::Manifest;
use crate::rutas::nombre_bin;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntradaComando {
    pub nombre: String,
    pub ruta: String,
}

#[derive(Debug)]
pub struct IndiceComandos {
    map: BTreeMap<String, String>,
}

impl IndiceComandos {
    pub fn new() -> Self {
        Self {
            map: BTreeMap::new(),
        }
    }

    pub fn insertar(&mut self, cmd: String, ruta_abs: String) {
        self.map.insert(cmd, ruta_abs);
    }

    pub fn resolver(&self, cmd: &str) -> Option<&str> {
        self.map.get(cmd).map(|s| s.as_str())
    }

    pub fn format(&self) -> String {
        let mut out = String::new();
        for (k, v) in &self.map {
            out.push_str(&format!("{k} {v}\n"));
        }
        out
    }

    pub fn parse(text: &str) -> Self {
        let mut map = BTreeMap::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut it = line.split_whitespace();
            let Some(nombre) = it.next() else { continue };
            let Some(ruta) = it.next() else { continue };
            map.insert(nombre.into(), ruta.into());
        }
        Self { map }
    }
}

impl Default for IndiceComandos {
    fn default() -> Self {
        Self::new()
    }
}

/// Entradas `bin/` de un manifiesto instalado bajo `/var/paquetes/<nombre>/`.
pub fn entradas_de_manifiesto(man: &Manifest, prefijo_inst: &str) -> Vec<EntradaComando> {
    let base = prefijo_inst.trim_end_matches('/');
    let mut out = Vec::new();
    for f in &man.files {
        if let Some(cmd) = nombre_bin(&f.path) {
            out.push(EntradaComando {
                nombre: cmd,
                ruta: format!("{base}/{}", f.path),
            });
        }
    }
    out
}

/// Reconstruye el índice global a partir de manifiestos instalados.
pub fn reconstruir(
    paquetes: &[(String, Manifest)],
    bin_sistema: &[&str],
) -> Result<IndiceComandos, String> {
    let mut idx = IndiceComandos::new();
    let mut vistos = BTreeMap::new();
    for b in bin_sistema {
        vistos.insert((*b).to_string(), String::from("/bin"));
    }
    for (nombre_pkg, man) in paquetes {
        let pref = format!("/var/paquetes/{nombre_pkg}");
        for e in entradas_de_manifiesto(man, &pref) {
            if vistos.contains_key(&e.nombre) {
                let origen = vistos.get(&e.nombre).cloned().unwrap_or_default();
                return Err(format!(
                    "comando «{}» ya existe ({origen} y paquete {nombre_pkg})",
                    e.nombre
                ));
            }
            vistos.insert(e.nombre.clone(), format!("paquete {nombre_pkg}"));
            idx.insertar(e.nombre, e.ruta);
        }
    }
    Ok(idx)
}

pub fn entrada_comando(line: &str) -> Option<EntradaComando> {
    let mut it = line.split_whitespace();
    let nombre = it.next()?.into();
    let ruta = it.next()?.into();
    Some(EntradaComando { nombre, ruta })
}
