//! Órdenes del banco: `listar`, `validar` y `sellar`.

use soso_improve_core::cli::Codigo;
use soso_improve_core::caso::{self, CasoCargado};
use soso_improve_core::entorno::Archivos;
use soso_improve_core::{unir, Error, Resultado};

use crate::sistema::Host;
use crate::Opciones;

pub const BANCO_POR_DEFECTO: &str = "tests/self-improvement/cases";
pub const ENV_RESERVADO: &str = "SOSO_BANCO_RESERVADO";

pub fn raiz_banco(opciones: &Opciones) -> String {
    opciones
        .uno("banco")
        .filter(|s| !s.is_empty())
        .unwrap_or(BANCO_POR_DEFECTO)
        .to_string()
}

/// Dónde vive el material reservado.
///
/// Por defecto, dentro del banco. En una campaña real se mueve fuera del
/// checkout que ve el agente y se apunta con `--reservado` o la variable de
/// entorno; los verificadores no necesitan nada más.
pub fn raiz_reservada(opciones: &Opciones, banco: &str) -> String {
    if let Some(r) = opciones.uno("reservado").filter(|s| !s.is_empty()) {
        return r.to_string();
    }
    if let Ok(r) = std::env::var(ENV_RESERVADO) {
        if !r.is_empty() {
            return r;
        }
    }
    unir(banco, "reservado")
}

pub fn cargar(banco: &str) -> Resultado<Vec<CasoCargado>> {
    caso::cargar(&Host, banco)
}

pub fn despachar(sub: &str, opciones: &Opciones) -> Resultado<i32> {
    let banco = raiz_banco(opciones);
    let reservado = raiz_reservada(opciones, &banco);
    match sub {
        "listar" => {
            for c in cargar(&banco)? {
                crate::digo!(
                    "{}  {:12} {:10} {}",
                    c.caso.id, c.caso.clase, c.caso.particion, c.caso.titulo
                );
            }
            Ok(0)
        }
        "validar" => {
            let casos = cargar(&banco)?;
            let mut problemas = caso::comprobar(&Host, &banco, &reservado, &casos)?;
            match caso::leer_manifiesto(&Host, &banco) {
                Err(e) => problemas.push(format!("banco.json: {e}")),
                Ok(m) => {
                    if m.huella != caso::huella(&casos) {
                        problemas.push(
                            "banco.json: la huella no corresponde a los casos actuales".to_string(),
                        );
                    }
                }
            }
            for p in &problemas {
                crate::aviso!("problema: {p}");
            }
            crate::digo!("{} casos, {} problema(s)", casos.len(), problemas.len());
            // Un banco inválido es una **medida que falla**, no un «no se pudo
            // usar»: le toca el 2. Devolvía 1, así que host y guest no sólo
            // tenían sintaxis distinta, también criterio distinto (T45).
            Ok(if problemas.is_empty() {
                Codigo::Exito.como_i32()
            } else {
                Codigo::Verificacion.como_i32()
            })
        }
        "sellar" => sellar(&banco, &reservado),
        otro => Err(Error::uso(format!("banco: subcomando desconocido {otro}"))),
    }
}

/// Recalcula partición, hashes de entrada y huella.
///
/// Solo reescribe un caso si algo cambia: así el sellado no mueve bytes —y con
/// ellos la huella— por gusto.
fn sellar(banco: &str, reservado: &str) -> Resultado<i32> {
    let mut casos = cargar(banco)?;
    let mut tocados = 0;
    for c in casos.iter_mut() {
        let particion = caso::particion_de(&c.caso.id).to_string();
        let hashes = caso::hashes_entrada(&Host, banco, &c.caso)?;
        if c.caso.particion == particion && c.caso.hashes_entrada == hashes {
            continue;
        }
        c.caso.particion = particion;
        c.caso.hashes_entrada = hashes;
        let texto = serde_json::to_string_pretty(&c.caso).map_err(Error::formato)?;
        let mut bytes = texto.into_bytes();
        bytes.push(b'\n');
        let mut host = Host;
        host.escribir(&unir(banco, &c.archivo), &bytes, 0o644)?;
        c.bytes = bytes;
        tocados += 1;
    }

    let casos = cargar(banco)?;
    let huella = caso::huella(&casos);
    let resumen = caso::resumen(&casos);
    let mut distribucion = serde_json::Map::new();
    let mut particiones = serde_json::Map::new();
    for (clase, (total, reservados)) in &resumen {
        distribucion.insert(clase.clone(), serde_json::json!(total));
        particiones.insert(
            clase.clone(),
            serde_json::json!({"desarrollo": total - reservados, "reservado": reservados}),
        );
    }
    let manifiesto = serde_json::json!({
        "schema_version": 1,
        "distribucion": distribucion,
        "distribucion_exigida": caso::DISTRIBUCION.iter()
            .map(|(c, n)| (c.to_string(), *n))
            .collect::<std::collections::BTreeMap<_, _>>(),
        "particion": {
            "regla": format!(
                "sha256('{}|' + id)[:8] % {} < {} -> reservado",
                caso::SAL_PARTICION, caso::MODULO_PARTICION, caso::CORTE_RESERVADO),
            "conteo": particiones,
        },
        "umbrales": {
            "estado": "no fijados",
            "fijar_en": "T14",
            "nota": "T02 no mide nada: fijar un umbral sin ejecución sería inventarlo. \
                     T14 los escribe aquí con su evidencia y desde entonces son \
                     inmutables durante la campaña.",
            "por_clase": caso::CLASES.iter()
                .map(|c| (c.to_string(), serde_json::Value::Null))
                .collect::<std::collections::BTreeMap<_, _>>(),
        },
        "huella": huella,
        "reservado": reservado,
        "casos": casos.iter().map(|c| serde_json::json!({
            "id": c.caso.id,
            "clase": c.caso.clase,
            "particion": c.caso.particion,
            "titulo": c.caso.titulo,
            "archivo": c.archivo,
        })).collect::<Vec<_>>(),
    });
    let mut texto = serde_json::to_string_pretty(&manifiesto).map_err(Error::formato)?;
    texto.push('\n');
    let mut host = Host;
    host.escribir(&unir(banco, "banco.json"), texto.as_bytes(), 0o644)?;
    crate::digo!(
        "sellados {} casos ({tocados} reescrito(s)); huella {}",
        casos.len(),
        &huella[..16]
    );
    Ok(0)
}
