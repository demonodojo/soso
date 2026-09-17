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
use soso_update_core::txn::punto::Punto;
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
        Recuperacion::Aplicar(id) => {
            let ok = ejecutar(&dir, diario, true);
            if ok {
                // Paso 4 del orden de aplicación: la pareja nueva queda a
                // prueba. Si el arranque no llega a confirmarse, el siguiente
                // encontrará esto y la deshará.
                publicar(&esp, Decision::Probando, id);
            }
            ok
        }
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
        Recuperacion::Rescatar(punto) => rescatar(&esp, punto, diario, &dir),
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

/// Restaura un **punto retenido** (U5d): la vuelta atrás pedida desde fuera del
/// sistema actualizado, que no depende de la operación en curso.
///
/// No lleva diario de progreso y no le hace falta: cada paso es idempotente, así
/// que un corte a mitad se arregla repitiéndolo entero en el arranque siguiente
/// —el registro sigue diciendo `rescatar` hasta que termina—.
fn rescatar(
    esp: &EstadoEsp,
    punto: soso_update_core::txn::TxnId,
    diario: EstadoJournal,
    // Directorio de la **operación** (el del registro de arranque), que no
    // tiene por qué ser el del punto: se rescata un punto retenido, y el
    // diario que hay que cerrar es el de la operación en curso.
    dir_op: &str,
) -> bool {
    let dir = punto.dir();
    let ruta = format!("{DIR_BASE}/{dir}/punto.rec");
    let datos = crate::vfs::resolve(&ruta)
        .ok()
        .and_then(|i| crate::vfs::read_file(i).ok());
    let Some(datos) = datos else {
        crate::println!("txn: rescate pedido pero no encuentro el punto en {ruta}");
        crate::otalog!("arranque: rescate sin punto en disco");
        return false;
    };
    let p = match Punto::parse(&datos) {
        Ok(p) => p,
        Err(e) => {
            crate::println!("txn: el punto de rescate está ilegible ({e:?})");
            return false;
        }
    };
    let mut sis = Vfs { dir: String::from(&dir) };
    // Se comprueba **entero antes** de escribir nada: restaurar a medias desde
    // un punto roto deja una pareja mezclada, que es lo que se quiere evitar.
    if let Err(e) = p.verificar(&mut sis) {
        crate::println!("txn: el punto de rescate no está completo ({e:?}); no lo uso");
        crate::otalog!("arranque: punto de rescate incompleto {e:?}");
        return false;
    }
    match aplicador::restaurar(&p.entradas, &mut sis) {
        Ok(()) => {
            crate::println!(
                "txn: restaurada la versión {} desde el punto ({} entradas)",
                p.version,
                p.entradas.len()
            );
            crate::otalog!("arranque: restaurada {} desde punto retenido", p.version);
            // El diario de la operación deshecha se cierra **antes** de publicar
            // la decisión: si se corta en medio, el registro sigue diciendo
            // `rescatar` y el arranque siguiente repite el rescate entero, que
            // es idempotente. Al revés quedaría una pareja incoherente.
            if let EstadoJournal::Diario(mut j) = diario {
                if soso_update_core::txn::rescate::cerrar_diario(&mut j) {
                    if escribir_diario(dir_op, &j).is_err() {
                        crate::println!("txn: no pude cerrar el diario tras el rescate");
                        crate::otalog!("arranque: diario abierto tras rescatar");
                        return false;
                    }
                }
            }
            // A prueba: falta que este arranque llegue a init. Si no llega, el
            // siguiente lo verá y lo dirá en vez de repetir la restauración.
            publicar(esp, Decision::RestauradoAPrueba, punto);
            true
        }
        Err(e) => {
            crate::println!("txn: fallo restaurando el punto ({e:?})");
            crate::otalog!("arranque: fallo restaurando punto {e:?}");
            false
        }
    }
}

/// Confirma la pareja: primero la evidencia en sosofs, después la decisión en
/// la ESP. Si se corta entre las dos, `reconcile` lo completa al arrancar.
pub fn confirmar() -> bool {
    let esp = leer_bootrec();
    // Sin registro no hay nada que acreditar. Se calla: en una máquina sin
    // hueco en la ESP esto pasa en cada invocación del cliente.
    let EstadoEsp::Registro(rec) = &esp else {
        return true;
    };
    match rec.decision {
        // La pareja nueva se acredita.
        Decision::Probando => {
            let dir = rec.dir.clone();
            if let EstadoJournal::Diario(mut j) = leer_diario(&dir) {
                if j.avanzar(soso_update_core::txn::TxnEvent::PruebaSuperada).is_ok() {
                    let _ = escribir_diario(&dir, &j);
                }
            }
            publicar(&esp, Decision::Confirmado, rec.id);
            crate::println!("txn: pareja confirmada ({})", rec.version_nueva);
            crate::otalog!("arranque: pareja confirmada");
        }
        // La versión restaurada sí arranca: la vuelta atrás queda cerrada.
        Decision::RestauradoAPrueba => {
            publicar(&esp, Decision::Revertido, rec.id);
            crate::println!("txn: versión restaurada acreditada ({})", rec.version_anterior);
            crate::otalog!("arranque: versión restaurada acreditada");
        }
        // Ya cerrado: acreditar es idempotente y el cliente lo salda en cada
        // orden, así que aquí no hay nada que decir.
        _ => {}
    }
    true
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
