//! Exclusión de escritores de la actualización (U5).
//!
//! Entre que `soso-update` respalda las rutas administradas y el arranque
//! siguiente las aplica, **nadie más puede escribirlas**. No es una manía de
//! coherencia: el punto de vuelta atrás guarda los ficheros tal como estaban en
//! ese instante, así que si otro programa reescribe `/bin/sosh` después del
//! respaldo, revertir no devolvería el sistema a un estado que existió — lo
//! machacaría con uno anterior, perdiendo lo que aquel programa hizo.
//!
//! Tres detalles del diseño:
//!
//! - **Leer no se toca.** Los procesos que sólo leen siguen usando los ficheros
//!   viejos, que es justo lo que se quiere mientras la nueva versión no está.
//! - **La exclusión sobrevive al proceso.** Al armar deja de tener dueño y dura
//!   hasta el reinicio, porque la ventana que hay que proteger va del respaldo
//!   al reinicio, no de un `main` a su `return`.
//! - **Un dueño muerto no deja la máquina de solo lectura.** Si el proceso que
//!   la tomó se cae antes de armar, la exclusión se suelta sola: se comprueba
//!   que siga vivo cada vez que se consulta.

use spin::Mutex;

/// Quién manda sobre las rutas administradas ahora mismo.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Estado {
    Libre,
    /// La tiene un proceso que está preparando la operación.
    Dueño(u64),
    /// La operación está armada: sin dueño y hasta el reinicio.
    Armada,
}

struct Exclusion {
    estado: Estado,
    /// Bloques de sosofs que la restauración necesitará. Los demás escritores
    /// no pueden bajar de aquí: gastarse la reserva ahora es quedarse sin
    /// vuelta atrás luego, cuando ya no hay margen para negociar.
    reserva: u64,
}

static EXCL: Mutex<Exclusion> = Mutex::new(Exclusion {
    estado: Estado::Libre,
    reserva: 0,
});

/// Estado actual, soltando de paso una exclusión cuyo dueño ya no vive.
fn vigente(e: &mut Exclusion) -> Estado {
    if let Estado::Dueño(pid) = e.estado {
        if !crate::task::exists(pid) {
            e.estado = Estado::Libre;
            e.reserva = 0;
            crate::println!("txn: exclusión soltada (el pid {pid} que la tenía ya no está)");
        }
    }
    e.estado
}

/// La toma para `pid`. `Err(EBUSY)` si ya hay otra operación en marcha: dos a
/// la vez sobre las mismas rutas no tienen arreglo posible.
pub fn tomar(pid: u64, reserva: u64) -> Result<(), i64> {
    let mut e = EXCL.lock();
    match vigente(&mut e) {
        Estado::Libre => {
            e.estado = Estado::Dueño(pid);
            e.reserva = reserva;
            Ok(())
        }
        Estado::Dueño(d) if d == pid => {
            e.reserva = reserva;
            Ok(())
        }
        _ => Err(-soso_abi::EBUSY),
    }
}

/// La operación quedó armada: la exclusión pierde dueño y dura hasta reiniciar.
pub fn armado(pid: u64) -> Result<(), i64> {
    let mut e = EXCL.lock();
    match vigente(&mut e) {
        Estado::Dueño(d) if d == pid => {
            e.estado = Estado::Armada;
            Ok(())
        }
        Estado::Armada => Ok(()),
        _ => Err(-soso_abi::EBUSY),
    }
}

/// La suelta el dueño porque canceló. Una operación **armada** no se suelta
/// desde userspace: ya está publicada y la aplica el arranque.
pub fn soltar(pid: u64) -> Result<(), i64> {
    let mut e = EXCL.lock();
    match vigente(&mut e) {
        Estado::Dueño(d) if d == pid => {
            e.estado = Estado::Libre;
            e.reserva = 0;
            Ok(())
        }
        Estado::Libre => Ok(()),
        _ => Err(-soso_abi::EBUSY),
    }
}

/// La toma el propio kernel al arrancar, cuando la pareja está **aplicada pero
/// sin acreditar**: hasta que este arranque se confirme, el siguiente puede
/// tener que deshacerla desde los respaldos, y esos respaldos describen el
/// sistema tal como estaba. Sin reserva: lo que hay que proteger aquí son las
/// rutas, no el espacio —la copia ya está hecha—.
pub fn tomar_por_el_kernel() {
    let mut e = EXCL.lock();
    e.estado = Estado::Armada;
    e.reserva = 0;
}

/// La suelta el kernel: la pareja quedó acreditada o deshecha, así que ya no
/// hay ninguna decisión pendiente que unos ficheros cambiados pudieran
/// estropear.
pub fn liberar_del_kernel() {
    let mut e = EXCL.lock();
    e.estado = Estado::Libre;
    e.reserva = 0;
}

pub fn estado(pid: u64) -> u64 {
    let mut e = EXCL.lock();
    match vigente(&mut e) {
        Estado::Libre => soso_abi::TXN_LOCK_LIBRE,
        Estado::Dueño(d) if d == pid => soso_abi::TXN_LOCK_MIA,
        Estado::Dueño(_) => soso_abi::TXN_LOCK_AJENA,
        Estado::Armada => soso_abi::TXN_LOCK_ARMADA,
    }
}

/// ¿Puede `pid` escribir en `ruta`? Es el único sitio donde se decide.
///
/// Devuelve `Err(EROFS)` para las rutas administradas —el mensaje no es «no
/// tienes permiso» sino «ahora mismo eso no se toca»— y `Err(ENOSPC)` cuando
/// el escritor se comería la reserva de la restauración.
pub fn comprobar_escritura(pid: u64, ruta: &str) -> Result<(), i64> {
    let (estado, reserva) = {
        let mut e = EXCL.lock();
        (vigente(&mut e), e.reserva)
    };
    let dueño = match estado {
        Estado::Libre => return Ok(()),
        Estado::Dueño(d) => Some(d),
        Estado::Armada => None,
    };
    if dueño == Some(pid) {
        return Ok(());
    }
    if soso_update_core::es_administrada(ruta) {
        return Err(-soso_abi::EROFS);
    }
    // Fuera de las rutas administradas se puede escribir, pero no hasta
    // dejar sin sitio a la vuelta atrás.
    if reserva > 0 && libres() < reserva {
        return Err(-soso_abi::ENOSPC);
    }
    Ok(())
}

/// ¿Puede este proceso publicar ahora mismo un fichero **administrado**? Es la
/// misma regla que `comprobar_escritura`, para cuando la ruta ya no está a
/// mano y sólo se sabe que lo era.
pub fn comprobar_ruta_administrada(pid: u64) -> Result<(), i64> {
    let estado = {
        let mut e = EXCL.lock();
        vigente(&mut e)
    };
    match estado {
        Estado::Libre => Ok(()),
        Estado::Dueño(d) if d == pid => Ok(()),
        _ => Err(-soso_abi::EROFS),
    }
}

fn libres() -> u64 {
    crate::fs::FS
        .get()
        .map(|fs| fs.lock().free_blocks())
        .unwrap_or(u64::MAX)
}
