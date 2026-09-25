//! `soso-improve receta comprobar` — qué parches del bootstrap faltan en un
//! vendor, **sin tocarlo**.
//!
//! La política está en `soso_improve_core::receta`, que es portable; aquí sólo
//! se pone el sistema de ficheros y se imprime.
//!
//! Existe porque el vendor se quedaba a medias en silencio: al escribir
//! [T39](../../../docs/self-improvement/T39-bootstrap-libstd.md),
//! `library/std/src/os/mod.rs` no tenía ni rastro de soso mientras los otros
//! seis parches sí estaban, y no había forma de verlo salvo abriendo ficheros
//! uno por uno.

use soso_improve_core::receta::{self, Resultado};
use soso_improve_core::Error;

use crate::sistema::Host;
use crate::Opciones;

/// Reparte entre los subcomandos de `receta`.
pub fn despachar(sub: &str, op: &Opciones) -> Result<i32, Error> {
    match sub {
        "comprobar" => comprobar(op),
        otro => Err(Error::uso(format!("receta necesita comprobar, no «{otro}»"))),
    }
}

pub fn comprobar(op: &Opciones) -> Result<i32, Error> {
    let vendor = op.exigido("vendor")?;
    let plantillas = op.exigido("plantillas")?;
    // La ruta de `soso-rt` sólo afecta al texto que se añadiría al
    // `Cargo.toml`; comprobar no escribe, así que basta con una coherente.
    let soso_rt = format!("{}/../../crates/soso-rt", plantillas.trim_end_matches('/'));

    let pasos = receta::pasos_libstd(plantillas, &soso_rt);
    let informes = receta::comprobar(&Host, vendor, &pasos);

    let mut faltan = 0usize;
    let mut rotos = 0usize;
    for i in &informes {
        let (marca, detalle) = match &i.resultado {
            Resultado::YaEstaba => ("ok   ", String::new()),
            Resultado::Falta => {
                faltan += 1;
                ("FALTA", String::new())
            }
            Resultado::Fallo(m) => {
                rotos += 1;
                ("ROTO ", format!(" — {m}"))
            }
            Resultado::Aplicado => ("?    ", String::from(" — comprobar no aplica nada")),
        };
        println!("{marca} {}{detalle}", i.nombre);
    }

    println!();
    println!(
        "receta: {} pasos · {faltan} por aplicar · {rotos} con el ancla rota",
        informes.len()
    );
    // Un ancla rota y un parche pendiente **no** son lo mismo: el segundo se
    // arregla ejecutando la preparación, el primero no se arregla
    // ejecutándola más veces. Por eso salen con códigos distintos.
    if rotos > 0 {
        println!("receta: un ancla rota no se arregla repitiendo la preparación:");
        println!("        el fichero del vendor cambió y el paso hay que reescribirlo.");
        return Ok(2);
    }
    Ok(if faltan > 0 { 1 } else { 0 })
}
