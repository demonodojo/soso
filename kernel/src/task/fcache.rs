//! Caché de ficheros **por inodo** (N-001, segunda mitad).
//!
//! Antes de esto cada descriptor se llevaba su copia del contenido al abrir.
//! De ahí salían dos cosas que T33 midió: un lector abierto antes no veía lo
//! que escribía otro descriptor, y de dos escritores ganaba el último en
//! cerrar — que se llevaba por delante lo del otro.
//!
//! La cura es que el contenido viva en **un** sitio, con el inodo como clave, y
//! que los descriptores guarden sólo su posición. Así se ven entre sí por
//! construcción, y una escritura parcial no puede truncar la cola porque la
//! cola está en el búfer compartido.
//!
//! **El volcado sigue siendo una sola transacción.** Lo dice la medida del paso
//! 1 de N-001: escribir en 64 transacciones en vez de una cuesta ~75×, así que
//! lo que no se puede perder es el agrupado. Se vuelca al cerrar el **último**
//! descriptor, o cuando alguien pide `fsync`.
//!
//! **Qué no entra aquí:** los ficheros grandes (`LazyFile`, > 64 KiB) y el
//! camino de crear ficheros (`StreamWrite`, que alimenta `/var/models/` con
//! shards de cientos de MB). La caché cubre lo mismo que ya cubría la primera
//! mitad: ficheros que existen y son pequeños.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use spin::Mutex;

/// Lo que se guarda de un fichero abierto.
struct Entrada {
    data: Vec<u8>,
    /// Cuántos descriptores lo tienen abierto. A cero, se vuelca y se quita.
    refs: usize,
    /// Si hay cambios sin escribir al disco.
    sucio: bool,
    /// Dónde republicarlo: `create_file` necesita padre y nombre, no el inodo.
    dir: u64,
    name: String,
    /// Titulares del cerrojo consultivo (N-004), por pid.
    ///
    /// Va aquí porque aquí es donde vive lo **compartido**: un cerrojo sobre
    /// un fichero que cada descriptor ve por su cuenta no significaría nada, y
    /// por eso N-004 iba detrás de N-001 en el backlog.
    titulares: Vec<u64>,
    /// Si el cerrojo es exclusivo. Con `titulares` vacío no significa nada.
    exclusivo: bool,
}

static CACHE: Mutex<BTreeMap<u64, Entrada>> = Mutex::new(BTreeMap::new());

/// Registra un descriptor sobre `ino`. Si es el primero, `cargar` da el
/// contenido inicial.
///
/// El contenido se pide por closure y no por valor para no leer el fichero
/// cuando ya está en la caché — que es justo el caso que hace falta que sea
/// barato, porque es el del segundo descriptor.
pub fn abrir(ino: u64, dir: u64, name: &str, cargar: impl FnOnce() -> Vec<u8>) {
    let mut c = CACHE.lock();
    match c.get_mut(&ino) {
        Some(e) => {
            e.refs += 1;
            // Si la entrada nació sin ruta —por un clonado de descriptor, que
            // no la conoce— y ahora sí se sabe, se completa: sin ella el
            // volcado no sabría adónde escribir.
            if e.name.is_empty() && !name.is_empty() {
                e.dir = dir;
                e.name = String::from(name);
            }
        }
        None => {
            c.insert(
                ino,
                Entrada {
                    data: cargar(),
                    refs: 1,
                    sucio: false,
                    dir,
                    name: String::from(name),
                    titulares: Vec::new(),
                    exclusivo: false,
                },
            );
        }
    }
}

/// `true` si ese inodo está en la caché.
pub fn tiene(ino: u64) -> bool {
    CACHE.lock().contains_key(&ino)
}

/// Longitud actual, o `None` si no está.
pub fn largo(ino: u64) -> Option<usize> {
    CACHE.lock().get(&ino).map(|e| e.data.len())
}

/// Copia hasta `dst.len()` bytes desde `pos`. Devuelve cuántos.
pub fn leer(ino: u64, pos: usize, dst: &mut [u8]) -> Option<usize> {
    let c = CACHE.lock();
    let e = c.get(&ino)?;
    let n = dst.len().min(e.data.len().saturating_sub(pos));
    dst[..n].copy_from_slice(&e.data[pos..pos + n]);
    Some(n)
}

/// Escribe en `pos`, extendiendo **sólo** si hace falta.
///
/// Que extienda sólo si hace falta es lo que conserva la cola: escribir cinco
/// bytes al principio de un fichero de sesenta lo deja de sesenta.
pub fn escribir(ino: u64, pos: usize, datos: &[u8]) -> Option<usize> {
    let mut c = CACHE.lock();
    let e = c.get_mut(&ino)?;
    let fin = pos.checked_add(datos.len())?;
    if fin > e.data.len() {
        e.data.resize(fin, 0);
    }
    e.data[pos..fin].copy_from_slice(datos);
    e.sucio = true;
    Some(datos.len())
}

/// Deja el fichero en `len` bytes.
pub fn truncar(ino: u64, len: usize) -> Option<()> {
    let mut c = CACHE.lock();
    let e = c.get_mut(&ino)?;
    e.data.resize(len, 0);
    e.sucio = true;
    Some(())
}

/// Lo que hace falta para volcar: `(dir, name, data)`, o `None` si no hay nada
/// sucio que escribir.
///
/// Se devuelve una copia en vez de escribir aquí dentro a propósito: publicar
/// toca el VFS, y hacerlo con el candado de la caché cogido es cómo se monta un
/// abrazo mortal con cualquier otro camino que abra un fichero.
pub fn pendiente(ino: u64) -> Option<(u64, String, Vec<u8>)> {
    let c = CACHE.lock();
    let e = c.get(&ino)?;
    if !e.sucio {
        return None;
    }
    Some((e.dir, e.name.clone(), e.data.clone()))
}

/// Marca como limpio tras un volcado correcto.
pub fn limpio(ino: u64) {
    if let Some(e) = CACHE.lock().get_mut(&ino) {
        e.sucio = false;
    }
}

/// Suelta un descriptor. Devuelve `true` si era el último.
///
/// **No** vuelca: el llamante ya ha publicado con [`pendiente`] si hacía falta.
/// Separarlo evita tener el candado cogido mientras se escribe en disco.
pub fn cerrar(ino: u64) -> bool {
    let mut c = CACHE.lock();
    let Some(e) = c.get_mut(&ino) else {
        return false;
    };
    e.refs = e.refs.saturating_sub(1);
    if e.refs == 0 {
        c.remove(&ino);
        return true;
    }
    false
}

/// Cuántos descriptores quedan sobre ese inodo. Para diagnóstico.
pub fn refs(ino: u64) -> usize {
    CACHE.lock().get(&ino).map(|e| e.refs).unwrap_or(0)
}

/// Intenta tomar el cerrojo para `pid`. `Err(())` si lo tiene otro.
///
/// Reglas, que son las de `flock` y no las de `fcntl`: el cerrojo es del
/// **fichero entero**, no de un rango, y es **consultivo** — no impide leer ni
/// escribir a quien no lo pide. Tomarlo dos veces el mismo proceso cambia el
/// modo en vez de fallar, que es lo que permite subir de compartido a
/// exclusivo sin soltar por el camino.
pub fn bloquear(ino: u64, pid: u64, exclusivo: bool) -> Result<(), ()> {
    let mut c = CACHE.lock();
    let Some(e) = c.get_mut(&ino) else {
        return Err(());
    };
    let ajenos = e.titulares.iter().filter(|t| **t != pid).count();
    if ajenos > 0 && (exclusivo || e.exclusivo) {
        return Err(());
    }
    if !e.titulares.contains(&pid) {
        e.titulares.push(pid);
    }
    // Con varios titulares no puede ser exclusivo; si sólo está este, manda lo
    // que pida.
    e.exclusivo = exclusivo && e.titulares.len() == 1;
    Ok(())
}

/// Suelta el cerrojo de `pid` sobre `ino`.
pub fn desbloquear(ino: u64, pid: u64) {
    if let Some(e) = CACHE.lock().get_mut(&ino) {
        e.titulares.retain(|t| *t != pid);
        if e.titulares.is_empty() {
            e.exclusivo = false;
        }
    }
}

/// Suelta **todos** los cerrojos de `pid`, en todos los ficheros.
///
/// Se llama al morir un proceso. Sin esto, un proceso que muere con el cerrojo
/// cogido lo deja cogido para siempre mientras otro tenga el fichero abierto —
/// y un candado que sobrevive a su dueño no es un candado, es un bloqueo.
pub fn soltar_todos(pid: u64) {
    for e in CACHE.lock().values_mut() {
        e.titulares.retain(|t| *t != pid);
        if e.titulares.is_empty() {
            e.exclusivo = false;
        }
    }
}

/// Quién tiene el cerrojo y en qué modo. Para diagnóstico y pruebas.
pub fn cerrojo(ino: u64) -> Option<(usize, bool)> {
    let c = CACHE.lock();
    let e = c.get(&ino)?;
    if e.titulares.is_empty() {
        return None;
    }
    Some((e.titulares.len(), e.exclusivo))
}
