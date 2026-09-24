//! Copia de tarea y exportación de su parche (T24).
//!
//! El coordinador nunca deja que un candidato toque el árbol de trabajo. Lo
//! que hace es **reconstruir la base en otro sitio** desde la captura por
//! contenido de [T01](crate::captura) —sin Git, porque soso no lo tiene—,
//! sembrar ahí el enunciado, y al terminar exportar lo que cambió como un
//! paquete de [T50](crate::delta) aplicable sobre el árbol original.
//!
//! Esto **no es un sandbox**. Copiar archivos no aísla procesos: un candidato
//! con permiso para ejecutar puede salirse por donde quiera. El aislamiento se
//! configura en T25, y confundir una cosa con la otra es la clase de error que
//! parece resuelto hasta que deja de estarlo.
//!
//! Dos decisiones que conviene entender antes de tocar nada:
//!
//! - **Lo que sembramos nosotros no es un cambio del intento.** El `TASK.md`
//!   lo escribe el coordinador; si se exportara contra la base aparecería como
//!   un alta del candidato y acabaría aplicándose al repositorio de verdad.
//!   Por eso la copia registra qué plantó, y al exportar se descuenta.
//! - **Un enlace simbólico no se sigue jamás.** El recorrido lo marca como
//!   «no es archivo regular» y aquí se **informa**, no se descarta en silencio:
//!   un enlace a `/etc/passwd` que desaparece del informe es exactamente cómo
//!   se cuela su contenido en un parche sin que nadie lo vea.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use serde::{Deserialize, Serialize};

use crate::captura::{self, ArchivoBase, Excluido, Manifiesto, Politica};
use crate::delta;
use crate::entorno::Archivos;
use crate::state::TaskSpec;
use crate::{unir, Error, Resultado, ESQUEMA};

/// El enunciado, tal como lo ve el candidato.
pub const TASK_MD: &str = "TASK.md";

/// Marca de la copia. Sirve para no borrar ni reutilizar el directorio de otro.
pub const MARCA: &str = ".soso-workspace.json";

/// Lo que el coordinador guarda de una copia.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Copia {
    pub schema_version: u32,
    /// Raíz de la copia, tal como se pidió.
    pub ruta: String,
    /// Huella del inventario base (T01): lo que el paquete espera encontrar.
    pub base: String,
    pub task_id: String,
    pub run_id: String,
    /// Lo que plantó el coordinador. **No** son cambios del intento.
    pub sembrados: Vec<ArchivoBase>,
    /// Hash del `TASK.md` materializado, para saber qué se le dio exactamente.
    pub paquete_tarea: String,
    pub archivos: usize,
}

/// Lo que sale de un intento.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Exportacion {
    /// Aplicable sobre el árbol original: su `base` es la de la captura.
    pub paquete: delta::Paquete,
    /// Rutas que plantó el coordinador y que se han descontado del paquete.
    pub previos: Vec<String>,
    /// Lo que el recorrido no pudo tratar como archivo: enlaces simbólicos,
    /// dispositivos… Se informa siempre, aunque esté vacío.
    pub rechazados: Vec<Excluido>,
}

impl Exportacion {
    /// `true` si el intento no cambió nada del repositorio.
    pub fn vacia(&self) -> bool {
        self.paquete.operaciones.is_empty()
    }
}

/// El enunciado en Markdown, con **sólo lo que el candidato puede ver**.
///
/// Quedan fuera a propósito: la huella base y los hashes de entrada (son
/// contabilidad del coordinador) y cualquier criterio reservado, que por C5 ni
/// siquiera viaja en el `TaskSpec`. Sí entran las comprobaciones: son lo que el
/// candidato tiene que hacer pasar, y ocultárselas no lo mediría mejor, sólo
/// haría el encargo imposible de entender.
pub fn texto_tarea(spec: &TaskSpec) -> String {
    let mut s = format!("# {}\n\n{}\n\n## Qué puedes tocar\n\n", spec.id, spec.problema);
    for r in &spec.rutas_editables {
        s.push_str(&format!("- `{r}`\n"));
    }
    s.push_str("\nCualquier otra ruta queda fuera: un cambio ahí no se aplica.\n");
    s.push_str("\n## Qué tiene que pasar\n\n");
    for c in &spec.comprobaciones {
        s.push_str(&format!(
            "- `{}` (en `{}`, máximo {} s)\n",
            c.argv.join(" "),
            c.cwd,
            c.timeout_s
        ));
    }
    s.push_str(&format!(
        "\n## Límites\n\n- Intentos: {}\n- Herramientas: {}\n",
        spec.limites.intentos_max, spec.limites.herramientas_max
    ));
    if let Some(t) = spec.limites.tokens_max {
        s.push_str(&format!("- Tokens: {t}\n"));
    }
    if let Some(s2) = spec.limites.segundos_max {
        s.push_str(&format!("- Segundos: {s2}\n"));
    }
    s
}

/// Reconstruye la base en `raiz` y siembra el enunciado.
///
/// No toca el árbol activo: lee de la captura y escribe en el destino, que son
/// dos `Archivos` distintos precisamente para que no puedan confundirse.
pub fn preparar(
    captura_arbol: &dyn Archivos,
    salida: &str,
    destino: &mut dyn Archivos,
    raiz: &str,
    manifiesto: &Manifiesto,
    spec: &TaskSpec,
    run_id: &str,
) -> Resultado<Copia> {
    spec.validar()?;
    let base = captura::huella(&manifiesto.inventario);
    if base != spec.base {
        return Err(Error::uso(format!(
            "la tarea {} se escribió contra la base {} y la captura es {base}",
            spec.id, spec.base
        )));
    }
    comprobar_destino(destino, raiz, run_id)?;

    let archivos = captura::reconstruir(captura_arbol, salida, destino, raiz, manifiesto)?;

    // El enunciado se siembra **después** de reconstruir: si la base trajera un
    // TASK.md propio, el nuestro manda, y así queda claro cuál se le dio.
    let texto = texto_tarea(spec);
    destino.escribir(&unir(raiz, TASK_MD), texto.as_bytes(), 0o644)?;
    let paquete_tarea = crate::sha256_hex(texto.as_bytes());
    let sembrados = alloc::vec![ArchivoBase {
        ruta: TASK_MD.to_string(),
        sha256: paquete_tarea.clone(),
        bytes: texto.len() as u64,
        modo: 0o644,
    }];

    let copia = Copia {
        schema_version: ESQUEMA,
        ruta: raiz.to_string(),
        base,
        task_id: spec.id.clone(),
        run_id: run_id.to_string(),
        sembrados,
        paquete_tarea,
        archivos,
    };
    let marca = serde_json::to_vec(&copia)
        .map_err(|e| Error::formato(format!("no pude escribir la marca: {e}")))?;
    destino.escribir(&unir(raiz, MARCA), &marca, 0o644)?;
    Ok(copia)
}

/// Rechaza un destino que ya existe y no es nuestro.
///
/// «Nuestro» significa con la marca de **esta misma ejecución**: reutilizar la
/// copia de otra mezclaría dos intentos en el mismo parche, y borrar un
/// directorio ajeno porque estorba es como se pierde el trabajo de alguien.
fn comprobar_destino(destino: &dyn Archivos, raiz: &str, run_id: &str) -> Resultado<()> {
    let ruta_marca = unir(raiz, MARCA);
    if let Ok(datos) = destino.leer(&ruta_marca) {
        let previa: Copia = serde_json::from_slice(&datos)
            .map_err(|e| Error::formato(format!("marca ilegible en {raiz}: {e}")))?;
        if previa.run_id != run_id {
            return Err(Error::uso(format!(
                "{raiz} ya es la copia de {}: no se reutiliza ni se borra",
                previa.run_id
            )));
        }
        return Ok(());
    }
    // Sin marca: sólo vale si está vacío o no existe. Un directorio con cosas
    // dentro es de alguien.
    match destino.listar(raiz) {
        Err(_) => Ok(()),
        Ok(e) if e.is_empty() => Ok(()),
        Ok(e) => Err(Error::uso(format!(
            "{raiz} existe, tiene {} entrada(s) y no lleva marca de copia",
            e.len()
        ))),
    }
}

/// Lee la marca de una copia.
pub fn leer_marca(arbol: &dyn Archivos, raiz: &str) -> Resultado<Copia> {
    let datos = arbol.leer(&unir(raiz, MARCA))?;
    let c: Copia = serde_json::from_slice(&datos)
        .map_err(|e| Error::formato(format!("marca ilegible en {raiz}: {e}")))?;
    if c.schema_version != ESQUEMA {
        return Err(Error::formato(format!(
            "marca de esquema {}; esta herramienta habla {ESQUEMA}",
            c.schema_version
        )));
    }
    Ok(c)
}

/// Exporta lo que cambió en la copia como paquete aplicable al árbol original.
pub fn exportar(
    arbol: &dyn Archivos,
    copia: &Copia,
    manifiesto: &Manifiesto,
    politica: &Politica,
) -> Resultado<Exportacion> {
    let base = captura::huella(&manifiesto.inventario);
    if base != copia.base {
        return Err(Error::uso(format!(
            "la copia se hizo sobre {} y el manifiesto es {base}",
            copia.base
        )));
    }
    let (inventario, rechazados) = escanear(arbol, &copia.ruta, politica)?;

    // Lo que plantó el coordinador se descuenta, aunque el candidato lo haya
    // modificado: sigue sin ser un cambio del repositorio.
    let mut previos: Vec<String> = copia.sembrados.iter().map(|a| a.ruta.clone()).collect();
    previos.push(MARCA.to_string());
    previos.sort();
    let candidato: Vec<ArchivoBase> = inventario
        .into_iter()
        .filter(|a| !previos.iter().any(|p| *p == a.ruta))
        .collect();

    let paquete = delta::exportar(&manifiesto.inventario, &candidato);
    Ok(Exportacion {
        paquete,
        previos,
        rechazados,
    })
}

/// Inventario de un árbol de trabajo, con lo que no es archivo regular aparte.
///
/// Los enlaces simbólicos salen en `rechazados`, nunca en el inventario, y no
/// se leen: seguir uno sería copiar el contenido de lo que apunte, que puede
/// estar fuera del árbol entero.
pub fn escanear(
    arbol: &dyn Archivos,
    raiz: &str,
    politica: &Politica,
) -> Resultado<(Vec<ArchivoBase>, Vec<Excluido>)> {
    let (entradas, excluidos) = captura::recorrer(arbol, raiz, politica)?;
    let mut inventario = Vec::with_capacity(entradas.len());
    for e in entradas {
        crate::ruta_segura(&e.ruta)?;
        let datos = arbol.leer(&unir(raiz, &e.ruta))?;
        inventario.push(ArchivoBase {
            sha256: crate::sha256_hex(&datos),
            bytes: datos.len() as u64,
            modo: e.modo,
            ruta: e.ruta,
        });
    }
    inventario.sort_by(|a, b| a.ruta.cmp(&b.ruta));
    let rechazados = excluidos
        .into_iter()
        .filter(|x| x.motivo == captura::NO_REGULAR)
        .collect();
    Ok((inventario, rechazados))
}

/// Borra una copia, y **sólo** si es la de esta ejecución.
///
/// Se comprueba la marca antes de tocar nada: un `rm -rf` sobre una ruta que
/// resultó no ser la nuestra no se deshace. Los intentos fallidos se conservan
/// —ver [`conservar`]— porque son lo único que queda para saber qué pasó.
pub fn limpiar(arbol: &mut dyn Archivos, copia: &Copia) -> Resultado<usize> {
    let presente = leer_marca(arbol, &copia.ruta)?;
    if presente.run_id != copia.run_id {
        return Err(Error::uso(format!(
            "{} es la copia de {}, no la de {}",
            copia.ruta, presente.run_id, copia.run_id
        )));
    }
    borrar_recursivo(arbol, &copia.ruta)
}

/// Si una copia hay que conservarla para diagnóstico.
///
/// Un intento que salió mal es justo el que hay que poder mirar después. Uno
/// aceptado ya dejó su paquete, y su copia sólo ocupa sitio.
pub fn conservar(estado: crate::state::Estado) -> bool {
    !matches!(estado, crate::state::Estado::Aceptada)
}

fn borrar_recursivo(arbol: &mut dyn Archivos, raiz: &str) -> Resultado<usize> {
    let mut borrados = 0;
    let entradas = match arbol.listar(raiz) {
        Ok(e) => e,
        Err(_) => return Ok(0),
    };
    for e in entradas {
        let ruta = unir(raiz, &e.ruta);
        match e.tipo {
            crate::entorno::Tipo::Directorio => borrados += borrar_recursivo(arbol, &ruta)?,
            // Un enlace se borra, no se sigue: borrar su destino sería salirse
            // del árbol por el mismo camino que no se le deja usar al candidato.
            _ => {
                arbol.borrar(&ruta)?;
                borrados += 1;
            }
        }
    }
    let _ = arbol.borrar(raiz);
    Ok(borrados)
}
