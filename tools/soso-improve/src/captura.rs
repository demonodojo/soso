//! Órdenes de base: `capturar`, `reconstruir` y `suites`.

use std::collections::BTreeMap;
use std::path::Path;

use soso_improve_core::captura::{
    self as base, Politica, Suite, Variable, VARIABLES,
};
use soso_improve_core::entorno::Archivos;
use soso_improve_core::ignorar::Reglas;
use soso_improve_core::{unir, Error, Resultado};

use crate::git;
use crate::sistema::Host;
use crate::Opciones;

/// Marca de tiempo ISO-8601 en UTC, sin dependencias de calendario ajenas.
fn ahora() -> String {
    let segundos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0) as i64;
    let dias = segundos.div_euclid(86_400);
    let resto = segundos.rem_euclid(86_400);
    // Algoritmo civil de Howard Hinnant: días desde la época a fecha.
    let z = dias + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        resto / 3600,
        (resto % 3600) / 60,
        resto % 60
    )
}

/// El destino debe ser nuevo y estar fuera de los archivos fuente.
///
/// «Fuera de los archivos fuente» se decide con la misma política que excluye
/// del inventario: si el destino cae dentro del árbol, su ruta relativa tiene
/// que estar excluida (`target/…`). Así la regla no depende de git y vale
/// igual dentro de soso.
fn validar_destino(raiz: &str, salida: &str, politica: &Politica) -> Resultado<()> {
    let raiz_abs = std::fs::canonicalize(raiz)
        .map_err(|e| Error::uso(format!("no se puede resolver {raiz}: {e}")))?;
    let salida_p = Path::new(salida);
    if salida_p.exists() {
        if !salida_p.is_dir() {
            return Err(Error::uso(format!(
                "el destino existe y no es un directorio: {salida}"
            )));
        }
        let vacio = std::fs::read_dir(salida_p)
            .map_err(|e| Error::uso(format!("no se puede leer {salida}: {e}")))?
            .next()
            .is_none();
        if !vacio {
            return Err(Error::uso(format!(
                "el destino existe y no está vacío: {salida}"
            )));
        }
    }
    let salida_abs = if salida_p.exists() {
        std::fs::canonicalize(salida_p)
            .map_err(|e| Error::uso(format!("no se puede resolver {salida}: {e}")))?
    } else {
        let padre = salida_p.parent().unwrap_or(Path::new("."));
        let padre_abs = std::fs::canonicalize(padre)
            .map_err(|e| Error::uso(format!("no existe el directorio padre de {salida}: {e}")))?;
        padre_abs.join(salida_p.file_name().unwrap_or_default())
    };
    if salida_abs == raiz_abs {
        return Err(Error::uso("el destino no puede ser el propio árbol"));
    }
    if raiz_abs.starts_with(&salida_abs) {
        return Err(Error::uso("el destino no puede contener al árbol"));
    }
    if let Ok(relativa) = salida_abs.strip_prefix(&raiz_abs) {
        let texto = relativa.to_string_lossy().replace('\\', "/");
        if politica.excluye(&texto).is_none() {
            return Err(Error::uso(format!(
                "destino dentro de los archivos fuente: {texto} (usa una ruta excluida, p. ej. target/…)"
            )));
        }
    }
    Ok(())
}

fn variables() -> BTreeMap<String, Variable> {
    let mut fuera = BTreeMap::new();
    for nombre in VARIABLES {
        let definida = std::env::var_os(nombre).is_some();
        if base::parece_secreto(nombre) {
            fuera.insert(
                nombre.to_string(),
                Variable {
                    definida,
                    valor: None,
                    motivo: Some("nombre con aspecto de secreto".to_string()),
                },
            );
        } else {
            fuera.insert(
                nombre.to_string(),
                Variable {
                    definida,
                    valor: std::env::var(nombre).ok(),
                    motivo: None,
                },
            );
        }
    }
    fuera
}

/// Política con las reglas del `.gitignore` del árbol, si lo hay.
///
/// El repositorio ya declara ahí qué es desechable; sin leerlo, una captura se
/// traga `rootfs/var/` (137 MB de artefactos) y `lxdde/reference/` (6600
/// archivos de fuentes ajenas).
fn politica_de(raiz: &str) -> Resultado<Politica> {
    let ruta = unir(raiz, ".gitignore");
    if !Host.existe(&ruta) {
        return Ok(Politica::default());
    }
    let texto = String::from_utf8_lossy(&Host.leer(&ruta)?).into_owned();
    Ok(Politica::default().con_ignorados(Reglas::parsear(&texto)?))
}

pub fn capturar(opciones: &Opciones) -> Resultado<i32> {
    let raiz = opciones.exigido("repo")?.to_string();
    let salida = opciones.exigido("out")?.to_string();
    let politica = politica_de(&raiz)?;
    validar_destino(&raiz, &salida, &politica)?;

    let mut host = Host;
    host.crear_directorio(&salida)?;
    let logs = unir(&salida, "logs");
    host.crear_directorio(&logs)?;

    let herramientas = if opciones.bandera("sin-herramientas") {
        Vec::new()
    } else {
        let mut guardar = |nombre: &str, texto: &str| -> Resultado<String> {
            let relativa = format!("logs/{nombre}.log");
            let mut h = Host;
            h.escribir(&unir(&salida, &relativa), texto.as_bytes(), 0o644)?;
            Ok(relativa)
        };
        base::sondar_herramientas(&Host, &raiz, &mut guardar)
    };

    let manifiesto = base::capturar(
        &Host,
        &raiz,
        &mut host,
        &salida,
        &politica,
        git::describir(&raiz),
        variables(),
        herramientas,
        base::suites_declaradas(),
        &ahora(),
    )?;

    crate::digo!(
        "captura en {salida}: {} archivo(s), {} excluido(s), huella {}",
        manifiesto.inventario.len(),
        manifiesto.excluidos.len(),
        &manifiesto.estabilidad.huella[..16]
    );
    if !manifiesto.completa() {
        crate::digo!(
            "  {} archivo(s) declarados sin contenido guardado",
            manifiesto.no_almacenados.len()
        );
    }
    Ok(0)
}

pub fn reconstruir(opciones: &Opciones) -> Resultado<i32> {
    let captura = opciones.exigido("captura")?.to_string();
    let destino = opciones.exigido("destino")?.to_string();
    let destino_p = Path::new(&destino);
    if destino_p.exists()
        && std::fs::read_dir(destino_p)
            .map(|mut d| d.next().is_some())
            .unwrap_or(false)
    {
        return Err(Error::uso(format!(
            "el destino existe y no está vacío: {destino}"
        )));
    }

    let manifiesto = base::leer_manifiesto(&Host, &captura)?;
    let mut host = Host;
    host.crear_directorio(&destino)?;
    let escritos = base::reconstruir(&Host, &captura, &mut host, &destino, &manifiesto)?;
    let verificacion = base::verificar(&Host, &destino, &manifiesto)?;

    let informe = serde_json::to_string_pretty(&verificacion)
        .map_err(Error::formato)?;
    host.escribir(
        &unir(&captura, "reconstruccion.json"),
        informe.as_bytes(),
        0o644,
    )?;

    if verificacion.ok {
        crate::digo!("reconstrucción verificada en {destino}: {escritos} archivo(s)");
        if !verificacion.no_reproducidos.is_empty() {
            crate::digo!(
                "  {} archivo(s) declarados y no almacenados: {}",
                verificacion.no_reproducidos.len(),
                verificacion.no_reproducidos.join(", ")
            );
        }
        return Ok(0);
    }
    crate::aviso!(
        "reconstrucción con {} problema(s)",
        verificacion.problemas.len()
    );
    for p in verificacion.problemas.iter().take(20) {
        crate::aviso!("  {}: esperado {} obtenido {}", p.ruta, p.esperado, p.obtenido);
    }
    Ok(2)
}

pub fn suites(opciones: &Opciones) -> Resultado<i32> {
    let captura = opciones.exigido("captura")?.to_string();
    let arbol = opciones.exigido("arbol")?.to_string();
    let timeout: u32 = opciones
        .uno("timeout")
        .and_then(|t| t.parse().ok())
        .unwrap_or(5400);
    let solo = opciones.varios("solo");

    let mut manifiesto = base::leer_manifiesto(&Host, &captura)?;
    let mut host = Host;
    let mut fallos = 0;
    let mut resultados: Vec<Suite> = Vec::new();

    for suite in base::suites_declaradas() {
        if !solo.is_empty() && !solo.contains(&suite.nombre) {
            resultados.push(Suite {
                estado: "omitida".to_string(),
                ..suite
            });
            continue;
        }
        let cwd = if suite.cwd == "." {
            arbol.clone()
        } else {
            unir(&arbol, &suite.cwd)
        };
        let orden = soso_improve_core::entorno::Orden {
            argv: suite.argv.clone(),
            cwd,
            stdin: Vec::new(),
            timeout_s: Some(timeout),
            entorno: Vec::new(),
        };
        let salida = soso_improve_core::entorno::Procesos::ejecutar(&Host, &orden)?;
        let relativa = format!("logs/suite-{}.log", suite.nombre);
        host.escribir(&unir(&captura, &relativa), salida.texto().as_bytes(), 0o644)?;
        let estado = if salida.ok() {
            "ok"
        } else if salida.codigo.is_none() {
            "no ejecutada"
        } else {
            fallos += 1;
            "fallo"
        };
        crate::digo!("suites: {} → {estado} (exit {:?})", suite.nombre, salida.codigo);
        resultados.push(Suite {
            estado: estado.to_string(),
            exit_code: salida.codigo,
            log: Some(relativa),
            motivo: salida.motivo,
            ..suite
        });
    }

    manifiesto.suites_base = resultados;
    base::escribir_manifiesto(&mut host, &captura, &manifiesto)?;
    Ok(if fallos > 0 { 4 } else { 0 })
}
