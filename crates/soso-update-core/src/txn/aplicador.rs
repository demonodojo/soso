//! Ejecución de la transacción: aplicar y deshacer, de forma idempotente.
//!
//! Entrega U5. `reconcile` (U0) decide **qué** hay que hacer al arrancar; esto
//! lo hace. La propiedad que tiene que cumplir, y que el banco comprueba
//! cortando en cada paso, es que **cualquier corte deja una pareja completa**:
//! o la anterior entera, o la nueva entera, nunca media.
//!
//! Aquí no hay E/S: el kernel y el cliente aportan un `Sistema`. Así el banco
//! puede cortar donde quiera, que dentro del kernel no hay forma de hacerlo.

use alloc::string::String;
use alloc::vec::Vec;

use crate::hash::hex_sha256;
use crate::txn::journal::{Accion, Journal, Progreso};
use crate::txn::{TxnEvent, TxnState};

/// De dónde sale un contenido.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum De {
    /// Área de preparación: lo que trae la release.
    Preparado,
    /// Respaldo: lo que había antes de aplicar.
    Respaldo,
}

/// Lo que el aplicador necesita del sistema.
pub trait Sistema {
    fn leer(&mut self, de: De, ruta: &str) -> Option<Vec<u8>>;
    fn escribir(&mut self, ruta: &str, datos: &[u8]) -> Result<(), ()>;
    /// Borrar algo que ya no está no es un error.
    fn borrar(&mut self, ruta: &str) -> Result<(), ()>;
    /// Hace durable el diario. Es el único punto de sincronización: hasta que
    /// esto vuelve, el progreso no cuenta.
    fn guardar_diario(&mut self, j: &Journal) -> Result<(), ()>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Fallo {
    /// Falta en el área de preparación lo que la release debía traer.
    FaltaPreparado(String),
    /// Falta el respaldo de algo que hay que deshacer. Es lo que convierte una
    /// reversión en un diagnóstico: sin la copia no se puede volver atrás.
    FaltaRespaldo(String),
    /// El contenido no cuadra con el hash que declara el diario.
    HashDistinto(String),
    Escritura(String),
    Borrado(String),
    /// No se pudo hacer durable el progreso: seguir sería avanzar a ciegas.
    Diario,
}

/// Aplica lo que falte. Repetirlo tras un corte no repite trabajo ni rompe
/// nada: cada entrada se salta si su progreso ya lo dice.
pub fn aplicar<S: Sistema>(j: &mut Journal, s: &mut S) -> Result<(), Fallo> {
    // Ya está hecho. Volver a pedirlo es normal: el recuperador entra en cada
    // arranque y no sabe si el anterior llegó a terminar.
    if matches!(j.estado, TxnState::Probando | TxnState::Confirmado) {
        return Ok(());
    }
    if j.estado != TxnState::Aplicando {
        let _ = j.avanzar(TxnEvent::AplicacionIniciada);
        s.guardar_diario(j).map_err(|_| Fallo::Diario)?;
    }
    for i in 0..j.entradas.len() {
        if j.entradas[i].progreso == Progreso::Aplicado {
            continue;
        }
        paso_aplicar(j, i, s)?;
        j.seq += 1;
        s.guardar_diario(j).map_err(|_| Fallo::Diario)?;
    }
    if j.estado != TxnState::Probando {
        j.avanzar(TxnEvent::AplicacionCompleta).map_err(|_| Fallo::Diario)?;
        s.guardar_diario(j).map_err(|_| Fallo::Diario)?;
    }
    Ok(())
}

fn paso_aplicar<S: Sistema>(j: &mut Journal, i: usize, s: &mut S) -> Result<(), Fallo> {
    let e = j.entradas[i].clone();
    match e.accion {
        Accion::Crear | Accion::Reemplazar => {
            let nuevo = e.nuevo.as_ref().ok_or_else(|| Fallo::FaltaPreparado(e.path.clone()))?;
            let datos = s
                .leer(De::Preparado, &e.path)
                .ok_or_else(|| Fallo::FaltaPreparado(e.path.clone()))?;
            // Se verifica **antes** de escribir: el área de preparación es un
            // fichero más del disco y puede haberse estropeado desde que se
            // descargó.
            if datos.len() as u64 != nuevo.size || hex_sha256(&datos) != nuevo.hash {
                return Err(Fallo::HashDistinto(e.path.clone()));
            }
            s.escribir(&e.path, &datos)
                .map_err(|_| Fallo::Escritura(e.path.clone()))?;
        }
        Accion::Borrar => {
            s.borrar(&e.path).map_err(|_| Fallo::Borrado(e.path.clone()))?;
        }
    }
    j.entradas[i].progreso = Progreso::Aplicado;
    Ok(())
}

/// Deshace la operación entera, **mire lo que mire el progreso**.
///
/// Es deliberado y costó un fallo encontrarlo: entre escribir un fichero y
/// anotar que se escribió hay una ventana, y un corte ahí deja el diario
/// diciendo «pendiente» sobre un fichero que ya es el nuevo. Deshacer sólo lo
/// marcado dejaba entonces una **pareja mezclada**, que es justo lo que este
/// contrato existe para impedir. Restaurar todo es idempotente —reescribir un
/// fichero intacto con su propio respaldo no lo cambia— y cuesta E/S, no
/// correción.
pub fn revertir<S: Sistema>(j: &mut Journal, s: &mut S) -> Result<(), Fallo> {
    if j.estado == TxnState::Revertido {
        return Ok(());
    }
    if j.estado != TxnState::Revirtiendo {
        let _ = j.avanzar(TxnEvent::ReversionSolicitada);
        s.guardar_diario(j).map_err(|_| Fallo::Diario)?;
    }
    for i in 0..j.entradas.len() {
        if j.entradas[i].progreso == Progreso::Restaurado {
            continue;
        }
        paso_revertir(j, i, s)?;
        j.seq += 1;
        s.guardar_diario(j).map_err(|_| Fallo::Diario)?;
    }
    if j.estado != TxnState::Revertido {
        j.avanzar(TxnEvent::ReversionCompleta).map_err(|_| Fallo::Diario)?;
        s.guardar_diario(j).map_err(|_| Fallo::Diario)?;
    }
    Ok(())
}

fn paso_revertir<S: Sistema>(j: &mut Journal, i: usize, s: &mut S) -> Result<(), Fallo> {
    let e = j.entradas[i].clone();
    match e.accion {
        // No existía antes de la actualización: deshacer es quitarlo.
        Accion::Crear => {
            s.borrar(&e.path).map_err(|_| Fallo::Borrado(e.path.clone()))?;
        }
        Accion::Reemplazar | Accion::Borrar => {
            let resp = e
                .respaldo
                .as_ref()
                .ok_or_else(|| Fallo::FaltaRespaldo(e.path.clone()))?;
            let datos = s
                .leer(De::Respaldo, &e.path)
                .ok_or_else(|| Fallo::FaltaRespaldo(e.path.clone()))?;
            if datos.len() as u64 != resp.size || hex_sha256(&datos) != resp.hash {
                return Err(Fallo::HashDistinto(e.path.clone()));
            }
            s.escribir(&e.path, &datos)
                .map_err(|_| Fallo::Escritura(e.path.clone()))?;
        }
    }
    j.entradas[i].progreso = Progreso::Restaurado;
    Ok(())
}

/// ¿Se puede deshacer esta operación entera con lo que hay guardado?
///
/// Se comprueba **antes** de armar: publicar el registro de arranque sin poder
/// deshacer es exactamente lo que este contrato evita.
pub fn se_puede_deshacer<S: Sistema>(j: &Journal, s: &mut S) -> Result<(), Fallo> {
    for e in &j.entradas {
        if !e.accion.exige_respaldo() {
            continue;
        }
        let resp = e
            .respaldo
            .as_ref()
            .ok_or_else(|| Fallo::FaltaRespaldo(e.path.clone()))?;
        let datos = s
            .leer(De::Respaldo, &e.path)
            .ok_or_else(|| Fallo::FaltaRespaldo(e.path.clone()))?;
        if datos.len() as u64 != resp.size || hex_sha256(&datos) != resp.hash {
            return Err(Fallo::HashDistinto(e.path.clone()));
        }
    }
    Ok(())
}
