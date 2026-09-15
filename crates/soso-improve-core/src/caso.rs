//! Casos del banco: tipos, partición reproducible y sellado.
//!
//! El formato en disco no cambia con el port a Rust —los 25 JSON del banco se
//! leen tal cual—, porque los datos ya eran neutros: lo que se porta es la
//! lógica, no los casos.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use serde::{Deserialize, Serialize};

use crate::entorno::Archivos;
use crate::{sha256_hex, unir, Error, Resultado};

/// Sal de la partición. Fija y publicada: cambiarla rehace el reparto entero.
pub const SAL_PARTICION: &str = "soso-banco-v1";
pub const MODULO_PARTICION: u32 = 5;
pub const CORTE_RESERVADO: u32 = 2;

pub const CLASES: &[&str] = &["programacion", "protocolo", "repo"];
pub const DISTRIBUCION: &[(&str, usize)] = &[
    ("programacion", 10),
    ("protocolo", 10),
    ("repo", 5),
];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Entrada {
    pub enunciado: String,
    #[serde(default)]
    pub archivos: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Comprobador {
    pub argv: Vec<String>,
    pub timeout_s: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Recursos {
    pub cpu: u32,
    pub memoria_mb: u32,
    pub red: bool,
    pub escritura: Vec<String>,
    #[serde(default)]
    pub herramientas: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Base {
    pub commit: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rama: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub captura: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Aceptacion {
    pub descripcion: String,
    pub argv: Vec<String>,
    pub prueba_reservada: String,
    pub destino_prueba: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub referencia: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Caso {
    pub schema_version: u32,
    pub id: String,
    pub clase: String,
    pub particion: String,
    pub titulo: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub etiqueta: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origen: Option<String>,
    pub entrada: Entrada,
    pub resultado_observable: String,
    pub comprobador: Comprobador,
    pub recursos: Recursos,
    pub reservado: Vec<String>,
    pub hashes_entrada: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<Base>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rutas_editables: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aceptacion: Option<Aceptacion>,
}

impl Caso {
    /// Rutas que el lanzador puede copiar al espacio del agente.
    pub fn paquete_visible(&self) -> Resultado<Vec<String>> {
        let mut rutas = Vec::with_capacity(1 + self.entrada.archivos.len());
        rutas.push(self.entrada.enunciado.clone());
        rutas.extend(self.entrada.archivos.iter().cloned());
        for r in &rutas {
            crate::ruta_segura(r)?;
            if !r.starts_with("visible/") {
                return Err(Error::uso(format!(
                    "{}: entrada fuera de visible/: {r}",
                    self.id
                )));
            }
        }
        Ok(rutas)
    }
}

/// Partición reproducible: sale del identificador, no de una lista.
///
/// Elegir a mano qué caso se reserva es la forma más fácil de acabar midiendo
/// con los casos que mejor salen. Esto lo hace imposible.
pub fn particion_de(id: &str) -> &'static str {
    let hash = sha256_hex(format!("{SAL_PARTICION}|{id}").as_bytes());
    let valor = u32::from_str_radix(&hash[..8], 16).unwrap_or(0);
    if valor % MODULO_PARTICION < CORTE_RESERVADO {
        "reservado"
    } else {
        "desarrollo"
    }
}

/// Un caso cargado junto con los bytes de su archivo, que son los que sella la
/// huella del banco.
#[derive(Debug, Clone)]
pub struct CasoCargado {
    pub caso: Caso,
    pub archivo: String,
    pub bytes: Vec<u8>,
}

/// Huella del banco entero: identifica la distribución de una campaña.
pub fn huella(casos: &[CasoCargado]) -> String {
    let mut acumulado = Vec::new();
    for c in casos {
        acumulado.extend_from_slice(c.caso.id.as_bytes());
        acumulado.push(0);
        acumulado.extend_from_slice(sha256_hex(&c.bytes).as_bytes());
        acumulado.push(0);
    }
    sha256_hex(&acumulado)
}

/// Carga los 25 casos desde el directorio del banco.
pub fn cargar(arbol: &dyn Archivos, raiz: &str) -> Resultado<Vec<CasoCargado>> {
    let mut casos: Vec<CasoCargado> = Vec::new();
    for clase in CLASES {
        let directorio = unir(raiz, clase);
        if !arbol.existe(&directorio) {
            continue;
        }
        for entrada in arbol.listar(&directorio)? {
            if !entrada.ruta.ends_with(".json") {
                continue;
            }
            let relativo = format!("{clase}/{}", entrada.ruta);
            let bytes = arbol.leer(&unir(raiz, &relativo))?;
            let caso: Caso = serde_json::from_slice(&bytes)
                .map_err(|e| Error::formato(format!("{relativo}: {e}")))?;
            if &caso.clase != clase {
                return Err(Error::formato(format!(
                    "{relativo}: clase {} en el directorio {clase}",
                    caso.clase
                )));
            }
            casos.push(CasoCargado {
                caso,
                archivo: relativo,
                bytes,
            });
        }
    }
    casos.sort_by(|a, b| a.caso.id.cmp(&b.caso.id));
    for par in casos.windows(2) {
        if par[0].caso.id == par[1].caso.id {
            return Err(Error::formato(format!(
                "id repetido {}: {} y {}",
                par[0].caso.id, par[0].archivo, par[1].archivo
            )));
        }
    }
    Ok(casos)
}

pub fn por_id<'a>(casos: &'a [CasoCargado], id: &str) -> Resultado<&'a Caso> {
    casos
        .iter()
        .find(|c| c.caso.id == id)
        .map(|c| &c.caso)
        .ok_or_else(|| Error::uso(format!("caso desconocido: {id}")))
}

/// Hashes de las entradas visibles, recalculados desde el contenido.
pub fn hashes_entrada(
    arbol: &dyn Archivos,
    raiz: &str,
    caso: &Caso,
) -> Resultado<BTreeMap<String, String>> {
    let mut fuera = BTreeMap::new();
    for r in caso.paquete_visible()? {
        let datos = arbol.leer(&unir(raiz, &r))?;
        fuera.insert(r, sha256_hex(&datos));
    }
    Ok(fuera)
}

/// Comprobaciones estructurales del banco. Devuelve la lista de problemas.
pub fn comprobar(
    arbol: &dyn Archivos,
    raiz: &str,
    reservado: &str,
    casos: &[CasoCargado],
) -> Resultado<Vec<String>> {
    let mut problemas = Vec::new();
    for (clase, esperados) in DISTRIBUCION {
        let hay = casos.iter().filter(|c| &c.caso.clase == clase).count();
        if hay != *esperados {
            problemas.push(format!("clase {clase}: {hay} casos, se exigen {esperados}"));
        }
    }
    for c in casos {
        let caso = &c.caso;
        if caso.particion != particion_de(&caso.id) {
            problemas.push(format!(
                "{}: partición {} no sale de la regla ({})",
                caso.id,
                caso.particion,
                particion_de(&caso.id)
            ));
        }
        match caso.paquete_visible() {
            Err(e) => problemas.push(format!("{}: {e}", caso.id)),
            Ok(rutas) => {
                for r in rutas {
                    if !arbol.existe(&unir(raiz, &r)) {
                        problemas.push(format!("{}: falta la entrada {r}", caso.id));
                    }
                }
            }
        }
        for r in &caso.reservado {
            if crate::ruta_segura(r).is_err() || !r.starts_with("reservado/") {
                problemas.push(format!("{}: reservado con ruta rara: {r}", caso.id));
                continue;
            }
            let destino = unir(reservado, r.trim_start_matches("reservado/"));
            if !arbol.existe(&destino) {
                problemas.push(format!("{}: falta el material reservado {r}", caso.id));
            }
        }
        match hashes_entrada(arbol, raiz, caso) {
            Err(e) => problemas.push(format!("{}: {e}", caso.id)),
            Ok(esperados) => {
                if esperados != caso.hashes_entrada {
                    problemas.push(format!(
                        "{}: hashes_entrada no coincide con el contenido",
                        caso.id
                    ));
                }
            }
        }
        for arg in &caso.comprobador.argv {
            if arg.starts_with("reservado/") {
                problemas.push(format!("{}: el argv del comprobador expone {arg}", caso.id));
            }
        }
    }
    Ok(problemas)
}

/// Manifiesto del banco (`banco.json`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifiesto {
    pub schema_version: u32,
    pub distribucion: BTreeMap<String, usize>,
    pub huella: String,
    #[serde(flatten)]
    pub resto: BTreeMap<String, serde_json::Value>,
}

pub fn leer_manifiesto(arbol: &dyn Archivos, raiz: &str) -> Resultado<Manifiesto> {
    let datos = arbol.leer(&unir(raiz, "banco.json"))?;
    serde_json::from_slice(&datos).map_err(Error::formato)
}

/// Clasificación de un caso para los informes: clase y partición.
pub fn resumen(casos: &[CasoCargado]) -> BTreeMap<String, (usize, usize)> {
    let mut fuera: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    for c in casos {
        let e = fuera.entry(c.caso.clase.clone()).or_insert((0, 0));
        e.0 += 1;
        if c.caso.particion == "reservado" {
            e.1 += 1;
        }
    }
    fuera
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn la_particion_es_la_misma_que_fijo_el_banco() {
        // Estos son los repartos que ya tiene el banco en disco: si la regla
        // cambiara, los 25 casos dejarían de validar.
        assert_eq!(particion_de("P02"), "reservado");
        assert_eq!(particion_de("P04"), "reservado");
        assert_eq!(particion_de("P06"), "reservado");
        assert_eq!(particion_de("P01"), "desarrollo");
        assert_eq!(particion_de("Q01"), "reservado");
        assert_eq!(particion_de("Q06"), "reservado");
        assert_eq!(particion_de("R02"), "reservado");
        assert_eq!(particion_de("R01"), "desarrollo");
    }

    #[test]
    fn la_particion_no_depende_del_contenido() {
        for id in ["P01", "Q05", "R03"] {
            assert_eq!(particion_de(id), particion_de(id));
        }
    }
}
