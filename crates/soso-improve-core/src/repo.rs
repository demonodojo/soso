//! Verificador de los casos de repo.
//!
//! Deja caer la prueba de aceptación reservada en el árbol, ejecuta el comando
//! de aceptación y **devuelve el árbol como estaba**. Un verificador que deja
//! restos convierte la siguiente medida en basura.
//!
//! El candidato nunca ve ni la prueba ni el parche de referencia: viven en la
//! raíz reservada, que puede estar fuera del checkout.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use serde::{Deserialize, Serialize};

use crate::caso::Caso;
use crate::entorno::{Archivos, Orden, Procesos};
use crate::{unir, Error, Resultado};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Informe {
    pub caso: String,
    pub arbol: String,
    pub base_declarada: String,
    pub con_referencia: bool,
    pub argv: Vec<String>,
    pub estado: String,
    pub exit_code: Option<i32>,
    pub motivo: Option<String>,
    pub salida: String,
}

impl Informe {
    pub fn codigo(&self) -> i32 {
        match self.estado.as_str() {
            "ok" => 0,
            "fallo" => 2,
            _ => 1,
        }
    }
}

/// Aplica (o revierte) el parche de referencia. Se pasa como cierre porque
/// aplicar un parche es cosa del entorno: en el host es `git apply`, y dentro
/// de soso será otra cosa mientras `soso-git` no sepa hacerlo.
pub type AplicarParche<'a> = &'a dyn Fn(&str, &str, bool) -> Resultado<()>;

/// Mide un caso de repo sobre `arbol`.
///
/// `con_referencia` aplica la solución reservada antes de medir: es el control
/// que demuestra que la aceptación distingue una solución correcta de la base,
/// no una forma de resolver el caso.
#[allow(clippy::too_many_arguments)]
pub fn verificar(
    caso: &Caso,
    arbol_raiz: &str,
    arbol: &mut dyn Archivos,
    procesos: &dyn Procesos,
    prueba: &[u8],
    parche: Option<&str>,
    aplicar: AplicarParche<'_>,
    timeout_s: u32,
) -> Resultado<Informe> {
    let aceptacion = caso
        .aceptacion
        .as_ref()
        .ok_or_else(|| Error::uso(format!("{}: no es un caso de repo", caso.id)))?;
    let base = caso
        .base
        .as_ref()
        .map(|b| b.commit.clone())
        .unwrap_or_default();
    crate::ruta_segura(&aceptacion.destino_prueba)?;
    let destino = unir(arbol_raiz, &aceptacion.destino_prueba);

    let mut informe = Informe {
        caso: caso.id.clone(),
        arbol: arbol_raiz.to_string(),
        base_declarada: base,
        con_referencia: parche.is_some(),
        argv: aceptacion.argv.clone(),
        estado: "error".to_string(),
        exit_code: None,
        motivo: None,
        salida: String::new(),
    };

    if arbol.existe(&destino) {
        informe.motivo = Some(format!(
            "ya existe {} en el árbol; el verificador no sobrescribe nada",
            aceptacion.destino_prueba
        ));
        return Ok(informe);
    }

    let mut parche_aplicado = false;
    if let Some(p) = parche {
        if let Err(e) = aplicar(arbol_raiz, p, false) {
            informe.motivo = Some(format!("el parche de referencia no aplica: {e}"));
            return Ok(informe);
        }
        parche_aplicado = true;
    }

    arbol.escribir(&destino, prueba, 0o644)?;
    let orden = Orden {
        argv: aceptacion.argv.clone(),
        cwd: arbol_raiz.to_string(),
        stdin: Vec::new(),
        timeout_s: Some(timeout_s),
        entorno: Vec::new(),
    };
    let salida = procesos.ejecutar(&orden);

    // El árbol se restaura aunque la aceptación reviente.
    let _ = arbol.borrar(&destino);
    if parche_aplicado {
        let _ = aplicar(arbol_raiz, parche.unwrap(), true);
    }

    match salida {
        Err(e) => {
            informe.motivo = Some(format!("no se pudo ejecutar la aceptación: {e}"));
            Ok(informe)
        }
        Ok(s) => {
            informe.exit_code = s.codigo;
            informe.motivo = s.motivo.clone();
            let texto = s.texto();
            let desde = texto.len().saturating_sub(8000);
            informe.salida = texto[desde..].to_string();
            informe.estado = if s.ok() { "ok" } else { "fallo" }.to_string();
            Ok(informe)
        }
    }
}
