//! Logs nativos en sosofs: `/var/log/{kernel,aplicaciones,actualizaciones}.log`.
//!
//! Entrega U1 de `docs/PLAN-ACTUALIZACIONES.md`. Sustituye a `fatlog` como
//! destino persistente en una instalación: la ESP deja de ser el sitio donde
//! vive el log y pasa a ser sólo el medio de arranque y recuperación. El live
//! conserva su `SOSOLOG.TXT`, porque es el único canal cuando sosofs no monta.
//!
//! Reglas que gobiernan este módulo, por orden de importancia:
//!
//! 1. **El logger no puede impedir arrancar ni recuperarse.** Cualquier error
//!    de FS suspende el flujo y se contabiliza; consola y ring siguen vivos.
//! 2. **Nada de E/S con el candado del ring tomado.** El lote se copia y el
//!    candado se suelta antes de tocar el disco.
//! 3. **Nada de escrituras recursivas.** Lo que el propio escritor imprima
//!    mientras vuelca no se captura (`logbuf::run_without_capture`).
//! 4. **Nunca desde una IRQ.** Sólo desde el bucle del planificador, el kshell
//!    o una petición explícita.

use alloc::vec;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use spin::Mutex;

use soso_log_core::flujo::{Estado, Flujo, LOTE_MAX};
use soso_log_core::rotacion::{self, Accion, Paso};
use soso_log_core::texto::{self, Cabecera};
use soso_log_core::{DIR_LOG, FLUJO_APPS, FLUJO_KERNEL, FLUJO_OTA};

/// Cadencia del volcado diferido.
const POLL_MS: u64 = 2000;
/// Tope de pasadas de un drenado, para que un ring que crece mientras se
/// vuelca no deje al planificador dando vueltas aquí.
const MAX_PASADAS: usize = 64;

/// De qué ring se alimenta cada flujo.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Fuente {
    Kernel,
    Apps,
    Ota,
}

impl Fuente {
    fn escritos(self) -> u64 {
        match self {
            Fuente::Kernel => crate::drivers::logbuf::escritos(),
            Fuente::Apps => crate::drivers::applog::escritos(),
            Fuente::Ota => crate::drivers::otalog::escritos(),
        }
    }
    fn cursor_minimo(self) -> u64 {
        match self {
            Fuente::Kernel => crate::drivers::logbuf::cursor_minimo(),
            Fuente::Apps => crate::drivers::applog::cursor_minimo(),
            Fuente::Ota => crate::drivers::otalog::cursor_minimo(),
        }
    }
    fn leer_desde(self, cursor: u64, out: &mut [u8]) -> soso_log_core::ring::Lectura {
        match self {
            Fuente::Kernel => crate::drivers::logbuf::leer_desde(cursor, out),
            Fuente::Apps => crate::drivers::applog::leer_desde(cursor, out),
            Fuente::Ota => crate::drivers::otalog::leer_desde(cursor, out),
        }
    }
}

struct Canal {
    fuente: Fuente,
    flujo: Flujo,
    /// Inodo del fichero activo.
    ino: u64,
}

struct Escritor {
    dir: u64,
    canales: [Canal; 3],
    /// Buffer del lote, reservado una vez: el volcado no reserva memoria.
    lote: Vec<u8>,
}

static ESCRITOR: Mutex<Option<Escritor>> = Mutex::new(None);
static ACTIVO: AtomicBool = AtomicBool::new(false);
/// Escritor en pausa: nadie escribe en sosofs desde aquí.
static PAUSADO: AtomicBool = AtomicBool::new(false);
static ULTIMO_POLL_MS: AtomicU64 = AtomicU64::new(0);
static BOOT_ID: AtomicU64 = AtomicU64::new(0);

pub fn activo() -> bool {
    ACTIVO.load(Ordering::Relaxed) && !PAUSADO.load(Ordering::Relaxed)
}

/// Deja de escribir en sosofs hasta nuevo aviso, tras vaciar lo pendiente.
///
/// Lo pide `soso-install` mientras clona: a partir de U1 este módulo escribe en
/// el rootfs cada dos segundos, y un commit de sosofs a mitad del clon deja en
/// el destino un superbloque que apunta a bloques que todavía no se habían
/// copiado. El ring sigue capturando; se vuelca al reanudar.
pub fn pausar() {
    if !activo() {
        PAUSADO.store(true, Ordering::Relaxed);
        return;
    }
    drenar_todo();
    PAUSADO.store(true, Ordering::Relaxed);
    crate::println!("logfs: en pausa (clon en curso); el log se queda en RAM");
}

pub fn reanudar() {
    if !PAUSADO.swap(false, Ordering::Relaxed) {
        return;
    }
    if ACTIVO.load(Ordering::Relaxed) {
        crate::println!("logfs: reanudado");
        drenar_todo();
    }
}

pub fn pausado() -> bool {
    PAUSADO.load(Ordering::Relaxed)
}

/// Arranca los logs nativos. Se llama **justo después de montar sosofs y antes
/// del bring-up de firmware**: lo capturado en RAM desde el primer `println!`
/// se vuelca aquí, que es lo que hace útil el fichero cuando el arranque
/// siguiente no llega tan lejos.
pub fn init() {
    if crate::fs::FS.get().is_none() {
        return;
    }
    let Some(dir) = asegurar_dir() else {
        crate::println!("logfs: sin {DIR_LOG}; los logs se quedan en RAM");
        return;
    };
    BOOT_ID.store(boot_id(), Ordering::Relaxed);

    let mut canales = [
        Canal { fuente: Fuente::Kernel, flujo: Flujo::nuevo(FLUJO_KERNEL), ino: 0 },
        Canal { fuente: Fuente::Apps, flujo: Flujo::nuevo(FLUJO_APPS), ino: 0 },
        Canal { fuente: Fuente::Ota, flujo: Flujo::nuevo(FLUJO_OTA), ino: 0 },
    ];
    let mut abiertos = 0;
    for c in &mut canales {
        match abrir(dir, c.flujo.nombre) {
            Some((ino, size)) => {
                c.ino = ino;
                c.flujo.activar(size);
                abiertos += 1;
            }
            None => crate::println!("logfs: no pude abrir {}/{}", DIR_LOG, c.flujo.nombre),
        }
    }
    if abiertos == 0 {
        return;
    }

    *ESCRITOR.lock() = Some(Escritor { dir, canales, lote: vec![0u8; LOTE_MAX] });
    ACTIVO.store(true, Ordering::Relaxed);
    cabeceras();
    drenar_todo();
    crate::println!("logfs: {DIR_LOG} activo ({abiertos}/3 flujos)");
}

/// Volcado diferido, ~cada 2 s. Se llama desde el bucle del planificador.
pub fn poll() {
    if !activo() {
        return;
    }
    // TSC y no PIT: el PIT subcuenta durante el polling de disco, que es
    // exactamente cuando este módulo está trabajando.
    let ahora = crate::arch::tsc::uptime_ms();
    let prev = ULTIMO_POLL_MS.load(Ordering::Relaxed);
    if ahora.saturating_sub(prev) < POLL_MS {
        return;
    }
    ULTIMO_POLL_MS.store(ahora, Ordering::Relaxed);
    pasada_global(1);
}

/// Vuelca todo lo pendiente. Para `init`, `halt`, `dmesg save` y checkpoints.
pub fn drenar_todo() {
    if !activo() {
        return;
    }
    pasada_global(MAX_PASADAS);
}

fn pasada_global(max_pasadas: usize) {
    // `try_lock`: si otro core ya está volcando, esta vuelta no hace nada. El
    // log nunca es motivo para que el planificador se quede esperando.
    let Some(mut guard) = ESCRITOR.try_lock() else {
        return;
    };
    let Some(esc) = guard.as_mut() else { return };
    let ahora = crate::arch::tsc::uptime_ms();
    for i in 0..esc.canales.len() {
        for _ in 0..max_pasadas {
            if !pasada_canal(esc, i, ahora) {
                break;
            }
        }
    }
}

/// Una pasada de un flujo. Devuelve `true` si queda material pendiente.
fn pasada_canal(esc: &mut Escritor, idx: usize, ahora_ms: u64) -> bool {
    let dir = esc.dir;
    let fuente = esc.canales[idx].fuente;
    if !esc.canales[idx].flujo.disponible(ahora_ms) {
        return false;
    }
    let Some(trabajo) = esc.canales[idx]
        .flujo
        .planificar(fuente.escritos(), fuente.cursor_minimo())
    else {
        return false;
    };

    // Copiar el lote y soltar el candado del ring antes de tocar el disco.
    let cursor = esc.canales[idx].flujo.cursor;
    let lectura = fuente.leer_desde(cursor, &mut esc.lote[..trabajo.bytes]);

    // La marca lleva lo que se perdió **en esta lectura**, no lo que se estimó
    // al planificar: entre una cosa y otra el ring puede haber dado otra vuelta.
    let mut marca = [0u8; 96];
    let marca_len = if lectura.perdidos > 0 {
        texto::marca_perdida(&mut marca, lectura.perdidos)
    } else {
        0
    };
    let total = marca_len + lectura.copiados;
    if total == 0 {
        return false;
    }

    let rotar = trabajo.accion == Accion::RotarYAnexar;
    // Todo lo que sigue imprime por consola si falla, pero no se captura: un
    // error del escritor que acabara en el ring que estamos vaciando sería una
    // escritura recursiva.
    let resultado = crate::drivers::logbuf::run_without_capture(|| {
        if rotar {
            let nombre = esc.canales[idx].flujo.nombre;
            rotar_flujo(dir, nombre)?;
            let (ino, _) = abrir(dir, nombre).ok_or(())?;
            esc.canales[idx].ino = ino;
        }
        let ino = esc.canales[idx].ino;
        let mtime = crate::time::wall_secs();
        if marca_len > 0 {
            crate::vfs::append_file(ino, &marca[..marca_len], mtime).map_err(|_| ())?;
        }
        if lectura.copiados > 0 {
            crate::vfs::append_file(ino, &esc.lote[..lectura.copiados], mtime).map_err(|_| ())?;
        }
        Ok::<(), ()>(())
    });

    match resultado {
        Ok(()) => {
            esc.canales[idx].flujo.exito(&lectura, total as u64, rotar);
            trabajo.mas_pendiente
        }
        Err(()) => {
            let c = &mut esc.canales[idx];
            c.flujo.fallo(ahora_ms, total);
            let mut aviso = [0u8; 96];
            let n = texto::marca_fallo(&mut aviso, c.flujo.nombre, c.flujo.fallos_totales);
            if let Ok(s) = core::str::from_utf8(&aviso[..n]) {
                crate::print!("{s}");
            }
            false
        }
    }
}

/// Corre el histórico un puesto: `.3` fuera, `.2`→`.3`, `.1`→`.2`, activo→`.1`.
fn rotar_flujo(dir: u64, nombre: &str) -> Result<(), ()> {
    let mtime = crate::time::wall_secs();
    for paso in rotacion::pasos() {
        match paso {
            Paso::Borrar(n) => {
                let mut buf = [0u8; 64];
                let viejo = ruta_en(&mut buf, nombre, n);
                // Que no exista es lo normal en las primeras rotaciones.
                let _ = crate::vfs::unlink(dir, viejo);
            }
            Paso::Renombrar { de, a } => {
                let mut b1 = [0u8; 64];
                let mut b2 = [0u8; 64];
                let (origen, destino) = (ruta_en(&mut b1, nombre, de), ruta_en(&mut b2, nombre, a));
                if crate::vfs::lookup(dir, origen).is_ok() {
                    crate::vfs::rename(dir, origen, dir, destino, mtime).map_err(|_| ())?;
                }
            }
        }
    }
    // El activo se acaba de renombrar: hace falta uno nuevo y vacío.
    crate::vfs::create_file(dir, nombre, &[], mtime).map_err(|_| ())?;
    Ok(())
}

/// `kernel.log` → `kernel.log.2`. Los nombres de flujo son constantes ASCII,
/// así que el recorte nunca parte un carácter; si alguna vez dejara de serlo,
/// devolver vacío hace que la operación falle en vez de tocar otro fichero.
fn ruta_en<'a>(buf: &'a mut [u8; 64], nombre: &str, indice: u8) -> &'a str {
    let n = nombre.len().min(buf.len() - 4);
    buf[..n].copy_from_slice(&nombre.as_bytes()[..n]);
    let mut sufijo = [0u8; 4];
    let s = rotacion::sufijo(indice, &mut sufijo);
    buf[n..n + s].copy_from_slice(&sufijo[..s]);
    core::str::from_utf8(&buf[..n + s]).unwrap_or("")
}

/// Abre (creando si hace falta) el fichero activo de un flujo.
fn abrir(dir: u64, nombre: &str) -> Option<(u64, u64)> {
    let mtime = crate::time::wall_secs();
    let ino = match crate::vfs::lookup(dir, nombre) {
        Ok(ino) => ino,
        Err(_) => crate::vfs::create_file(dir, nombre, &[], mtime).ok()?,
    };
    let size = crate::vfs::stat_inode(ino).ok()?.size.get();
    Some((ino, size))
}

/// `/var/log`, creando `/var` y `/var/log` si faltan.
fn asegurar_dir() -> Option<u64> {
    if let Ok(ino) = crate::vfs::resolve(DIR_LOG) {
        return Some(ino);
    }
    let mtime = crate::time::wall_secs();
    let raiz = crate::vfs::resolve("/").ok()?;
    let var = match crate::vfs::lookup(raiz, "var") {
        Ok(ino) => ino,
        Err(_) => crate::vfs::mkdir(raiz, "var", mtime).ok()?,
    };
    match crate::vfs::lookup(var, "log") {
        Ok(ino) => Some(ino),
        Err(_) => crate::vfs::mkdir(var, "log", mtime).ok(),
    }
}

/// Cabecera de arranque en cada flujo: sin ella, un fichero acumulado no deja
/// distinguir qué línea es de qué arranque.
fn cabeceras() {
    let boot_id = BOOT_ID.load(Ordering::Relaxed);
    let uptime = crate::arch::tsc::uptime_ms();
    let fecha = crate::arch::rtc::fecha_iso();
    let Some(mut guard) = ESCRITOR.try_lock() else { return };
    let Some(esc) = guard.as_mut() else { return };
    let dir_mtime = crate::time::wall_secs();
    for c in &mut esc.canales {
        if c.flujo.estado == Estado::Inactivo {
            continue;
        }
        let mut buf = [0u8; 192];
        let n = texto::cabecera(
            &mut buf,
            &Cabecera {
                flujo: c.flujo.nombre,
                boot_id,
                version: crate::version::version(),
                uptime_ms: uptime,
                fecha: fecha.as_ref().map(|f| f.as_str()),
            },
        );
        let ok = crate::drivers::logbuf::run_without_capture(|| {
            crate::vfs::append_file(c.ino, &buf[..n], dir_mtime).is_ok()
        });
        if ok {
            c.flujo.activo_bytes += n as u64;
            c.flujo.escritos += n as u64;
        }
    }
}

/// Identificador del arranque. El TSC al arrancar basta para distinguir
/// arranques dentro de una instalación, que es para lo que sirve.
fn boot_id() -> u64 {
    let ns = crate::time::monotonic_ns();
    let wall = crate::time::wall_secs();
    ns ^ (wall << 20) ^ 0x5050_5f4c_4f47_5f31
}

/// Resumen para el kshell.
pub fn resumen() {
    if pausado() {
        crate::println!("logfs: en pausa (clon en curso); el log se acumula en RAM");
        return;
    }
    if !activo() {
        crate::println!("logfs: inactivo (sin sosofs o sin {DIR_LOG})");
        return;
    }
    let Some(guard) = ESCRITOR.try_lock() else {
        crate::println!("logfs: ocupado");
        return;
    };
    let Some(esc) = guard.as_ref() else { return };
    for c in &esc.canales {
        let f = &c.flujo;
        let estado = match f.estado {
            Estado::Inactivo => "inactivo",
            Estado::Activo => "activo",
            Estado::Suspendido { .. } => "suspendido",
        };
        crate::println!(
            "logfs: {} {} activo={}B escritos={}B perdidos={}B fallos={} descartados={}B",
            f.nombre,
            estado,
            f.activo_bytes,
            f.escritos,
            f.perdidos,
            f.fallos_totales,
            f.descartados
        );
    }
}

/// Intento único desde un camino de emergencia (panic, muerte de un proceso).
///
/// Sólo escribe si el candado del FS está libre: forzar su desbloqueo para
/// dejar constancia del panic partiría el árbol CoW a medio commit, y perder
/// la cola del log es mucho más barato que perder el sistema de ficheros.
/// Con un corte brusco, la cola no persistida se pierde: es el límite asumido.
pub fn drenar_si_seguro() {
    if !activo() || !crate::vfs::fs_disponible() {
        return;
    }
    pasada_global(MAX_PASADAS);
}
