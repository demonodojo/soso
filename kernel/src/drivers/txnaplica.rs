//! Recuperador temprano de actualizaciones (entrega U5).
//!
//! Corre **justo después de montar sosofs y antes de cargar firmware de `/lib`
//! y de arrancar `/bin/init`**. Ese sitio no es casual: si una actualización se
//! quedó a medias, nadie puede consumir un `/lib` mezclado ni ejecutar un
//! `/bin/init` que quizá sea el nuevo con el resto viejo.
//!
//! Aquí sólo está la E/S. La decisión la toma `reconcile` (U0) y la ejecución
//! `aplicador` (U5), los dos en `soso-update-core` y probados en host, cortando
//! en cada paso.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use soso_update_core::txn::aplicador::{self, De, Sistema};
use soso_update_core::txn::bootrec::{BootRecError, BootRecord, Decision, BOOTREC_SIZE};
use soso_update_core::txn::journal::Journal;
use soso_update_core::txn::reconcile::{reconcile, EstadoEsp, EstadoJournal, Recuperacion};
use soso_update_core::RecordError;

use crate::drivers::espfat;

const DIR_BASE: &str = "/var/lib/soso-update";

/// Ejecuta lo que toque. Devuelve `false` si el arranque **no** debe seguir
/// como normal (pareja incoherente: hace falta diagnóstico desde el live).
pub fn recuperar() -> bool {
    if crate::fs::FS.get().is_none() {
        return true;
    }
    let esp = leer_bootrec();
    // Sin registro en la ESP no hay ninguna operación armada: el camino normal.
    let dir = match &esp {
        EstadoEsp::Registro(r) => r.dir.clone(),
        _ => match buscar_dir_unico() {
            Some(d) => d,
            None => return true,
        },
    };
    let diario = leer_diario(&dir);
    if matches!(esp, EstadoEsp::Ausente) && matches!(diario, EstadoJournal::Ausente) {
        return true;
    }

    let accion = reconcile(&esp, &diario);
    crate::println!("txn: {accion:?}");
    crate::otalog!("arranque: reconciliación → {accion:?}");

    match accion {
        Recuperacion::Normal => true,
        Recuperacion::Descartar(_) => true,
        Recuperacion::RetrocederAPreparado(_) => true,
        Recuperacion::Aplicar(_) => ejecutar(&dir, diario, true),
        Recuperacion::Revertir(_) => ejecutar(&dir, diario, false),
        Recuperacion::PublicarProbando(id) => {
            publicar(&esp, Decision::Probando, id);
            true
        }
        Recuperacion::CompletarConfirmacion(id) => {
            publicar(&esp, Decision::Confirmado, id);
            true
        }
        Recuperacion::CompletarReversion(id) => {
            publicar(&esp, Decision::Revertido, id);
            true
        }
        Recuperacion::Diagnostico(motivo) => {
            crate::println!(
                "txn: PAREJA INCOHERENTE ({motivo:?}) — no arranco así; \
                 recupera desde el USB live"
            );
            crate::otalog!("arranque: pareja incoherente ({motivo:?})");
            false
        }
    }
}

fn ejecutar(dir: &str, diario: EstadoJournal, aplicar: bool) -> bool {
    let EstadoJournal::Diario(mut j) = diario else {
        return false;
    };
    let mut sis = Vfs { dir: String::from(dir) };
    let r = if aplicar {
        aplicador::aplicar(&mut j, &mut sis)
    } else {
        aplicador::revertir(&mut j, &mut sis)
    };
    match r {
        Ok(()) => {
            let verbo = if aplicar { "aplicada" } else { "revertida" };
            crate::println!("txn: actualización {verbo} ({} entradas)", j.entradas.len());
            crate::otalog!("arranque: operación {verbo}, {} entradas", j.entradas.len());
            true
        }
        Err(e) => {
            crate::println!("txn: fallo {e:?} — no arranco con una pareja a medias");
            crate::otalog!("arranque: fallo del aplicador {e:?}");
            false
        }
    }
}

/// El sistema, visto por el aplicador. Las rutas del diario son relativas a la
/// raíz; el área de preparación y el respaldo cuelgan del directorio de la
/// operación.
struct Vfs {
    dir: String,
}

impl Vfs {
    fn ruta(&self, de: De, rel: &str) -> String {
        match de {
            De::Preparado => format!("{DIR_BASE}/{}/etapa/{rel}", self.dir),
            De::Respaldo => format!("{DIR_BASE}/{}/respaldo/{rel}", self.dir),
        }
    }
}

impl Sistema for Vfs {
    fn leer(&mut self, de: De, rel: &str) -> Option<Vec<u8>> {
        let ruta = self.ruta(de, rel);
        let ino = crate::vfs::resolve(&ruta).ok()?;
        crate::vfs::read_file(ino).ok()
    }

    fn escribir(&mut self, rel: &str, datos: &[u8]) -> Result<(), ()> {
        let destino = format!("/{rel}");
        crear_arbol(parent_of(&destino));
        let (dir, nombre) = split_padre(&destino).ok_or(())?;
        let padre = crate::vfs::resolve(dir).map_err(|_| ())?;
        let mtime = crate::time::wall_secs();
        let _ = crate::vfs::unlink(padre, nombre);
        crate::vfs::create_file(padre, nombre, datos, mtime)
            .map(|_| ())
            .map_err(|_| ())
    }

    fn borrar(&mut self, rel: &str) -> Result<(), ()> {
        let destino = format!("/{rel}");
        let Some((dir, nombre)) = split_padre(&destino) else {
            return Ok(());
        };
        let Ok(padre) = crate::vfs::resolve(dir) else {
            return Ok(());
        };
        // Que ya no esté no es un error: la operación es idempotente.
        let _ = crate::vfs::unlink(padre, nombre);
        Ok(())
    }

    fn guardar_diario(&mut self, j: &Journal) -> Result<(), ()> {
        escribir_diario(&self.dir, j)
    }
}

fn parent_of(path: &str) -> &str {
    path.rfind('/').filter(|&i| i > 0).map_or("/", |i| &path[..i])
}

fn split_padre(path: &str) -> Option<(&str, &str)> {
    let i = path.rfind('/')?;
    let nombre = &path[i + 1..];
    if nombre.is_empty() {
        return None;
    }
    Some((if i == 0 { "/" } else { &path[..i] }, nombre))
}

fn crear_arbol(dir: &str) {
    let mtime = crate::time::wall_secs();
    let mut actual = match crate::vfs::resolve("/") {
        Ok(r) => r,
        Err(_) => return,
    };
    for parte in dir.trim_start_matches('/').split('/') {
        if parte.is_empty() {
            continue;
        }
        actual = match crate::vfs::lookup(actual, parte) {
            Ok(i) => i,
            Err(_) => match crate::vfs::mkdir(actual, parte, mtime) {
                Ok(i) => i,
                Err(_) => return,
            },
        };
    }
}

// ── Registros ────────────────────────────────────────────────────────────

fn leer_bootrec() -> EstadoEsp {
    let Some(slot) = espfat::locate(b"SOSOTXN ", b"BIN", BOOTREC_SIZE) else {
        return EstadoEsp::Ausente;
    };
    let mut buf = alloc::vec![0u8; BOOTREC_SIZE];
    for (i, trozo) in buf.chunks_mut(espfat::SECTOR).enumerate() {
        if espfat::read(slot.data_lba + i as u64, trozo).is_err() {
            return EstadoEsp::Roto;
        }
    }
    match BootRecord::pick(&buf) {
        Ok(r) => EstadoEsp::Registro(r),
        Err(BootRecError::Registro(RecordError::Vacia)) => EstadoEsp::Ausente,
        Err(_) => EstadoEsp::Roto,
    }
}

/// Publica una decisión en la ESP, por turnos de ranura.
fn publicar(esp: &EstadoEsp, decision: Decision, id: soso_update_core::txn::TxnId) {
    let EstadoEsp::Registro(actual) = esp else {
        return;
    };
    let mut nuevo = actual.clone();
    nuevo.decision = decision;
    nuevo.id = id;
    nuevo.seq = actual.seq + 1;
    let Ok(bytes) = nuevo.format() else { return };
    let Some(slot) = espfat::locate(b"SOSOTXN ", b"BIN", BOOTREC_SIZE) else {
        return;
    };
    let base = slot.data_lba + (nuevo.ranura() * soso_update_core::SLOT_SIZE / espfat::SECTOR) as u64;
    for (i, trozo) in bytes.chunks(espfat::SECTOR).enumerate() {
        if espfat::write(base + i as u64, trozo).is_err() {
            crate::println!("txn: no pude publicar {decision:?} en SOSOTXN.BIN");
            return;
        }
    }
    crate::otalog!("arranque: registro de arranque → {decision:?}");
}

fn copias(dir: &str) -> (String, String) {
    (
        format!("{DIR_BASE}/{dir}/diario.0"),
        format!("{DIR_BASE}/{dir}/diario.1"),
    )
}

fn leer_diario(dir: &str) -> EstadoJournal {
    let (a, b) = copias(dir);
    let leer = |p: &str| {
        crate::vfs::resolve(p)
            .ok()
            .and_then(|i| crate::vfs::read_file(i).ok())
            .unwrap_or_default()
    };
    let (da, db) = (leer(&a), leer(&b));
    if da.is_empty() && db.is_empty() {
        return EstadoJournal::Ausente;
    }
    match Journal::pick(&[&da, &db]) {
        Ok(j) => EstadoJournal::Diario(j),
        Err(_) => EstadoJournal::Roto,
    }
}

/// Escribe el diario en la copia que le toca por secuencia: una escritura
/// cortada no puede llevarse por delante la anterior.
fn escribir_diario(dir: &str, j: &Journal) -> Result<(), ()> {
    let (a, b) = copias(dir);
    let destino = if j.seq % 2 == 0 { a } else { b };
    let (padre, nombre) = split_padre(&destino).ok_or(())?;
    let ino_padre = crate::vfs::resolve(padre).map_err(|_| ())?;
    let mtime = crate::time::wall_secs();
    let datos = j.format();
    let _ = crate::vfs::unlink(ino_padre, nombre);
    crate::vfs::create_file(ino_padre, nombre, &datos, mtime)
        .map(|_| ())
        .map_err(|_| ())
}

/// Sin registro en la ESP, busca si hay una única operación en sosofs. Sirve
/// para el caso «la ESP se borró con el rootfs a medias», que `reconcile`
/// resuelve deshaciendo.
fn buscar_dir_unico() -> Option<String> {
    let ino = crate::vfs::resolve(DIR_BASE).ok()?;
    let entradas = crate::vfs::read_dir(ino).ok()?;
    let mut it = entradas
        .into_iter()
        .map(|(n, _)| n)
        .filter(|n| n != "." && n != "..");
    let primero = it.next()?;
    if it.next().is_some() {
        return None;
    }
    Some(primero)
}
