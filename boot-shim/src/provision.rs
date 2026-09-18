//! Transición de una instalación antigua (U6): crear en su ESP lo que le falta.
//!
//! **Por qué aquí.** Los huecos de la ESP son ficheros FAT pre-creados que el
//! kernel sobrescribe por LBA; el kernel no sabe crear entradas de directorio
//! ni asignar clusters, así que no puede provisionarlos él. Bajo UEFI sí hay un
//! driver FAT completo: el shim del live puede abrir la ESP **del disco
//! instalado** por GUID y crear allí lo que haga falta. Es la misma vía por la
//! que se registra la entrada de arranque tras instalar.
//!
//! **Qué no hace.** No toca el rootfs, ni sosomfs, ni la configuración. No es
//! un instalador: es la parte de arranque de una instalación que ya existe.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

use soso_update_core::migracion::HUECOS;
use uefi::boot::{self, ScopedProtocol};
use uefi::proto::media::file::{Directory, File, FileAttribute, FileInfo, FileMode, RegularFile};
use uefi::proto::media::fs::SimpleFileSystem;
use uefi::{cstr16, CStr16, CString16, Guid};

/// Nuestro propio fichero en la ESP del live y en la del destino.
const SHIM: &CStr16 = cstr16!("efi\\boot\\bootx64.efi");

/// Provisiona la ESP con este GUID. Devuelve el detalle para la respuesta.
pub fn provisionar(esp: Guid) -> Result<String, String> {
    let handle = crate::bootentry::localizar_esp(esp)
        .ok_or_else(|| format!("no encuentro ninguna ESP con GUID de partición {esp}"))?;
    let mut fs: ScopedProtocol<SimpleFileSystem> =
        boot::open_protocol_exclusive(handle).map_err(|e| format!("abrir la ESP: {:?}", e.status()))?;
    let mut raiz = fs.open_volume().map_err(|e| format!("open_volume: {:?}", e.status()))?;

    let mut detalle = String::new();
    for h in HUECOS {
        let nombre = CString16::try_from(h.nombre).map_err(|_| "nombre inválido".to_string())?;
        match asegurar_hueco(&mut raiz, &nombre, h.tamano) {
            Ok(Accion::YaEstaba) => detalle.push_str(&format!("{} ya estaba\n", h.nombre)),
            Ok(Accion::Creado) => {
                detalle.push_str(&format!("{} creado ({} B)\n", h.nombre, h.tamano))
            }
            Err(e) => return Err(format!("{}: {e}", h.nombre)),
        }
    }

    // Un hueco de identidad en blanco es peor que no tenerlo: el kernel lo lee
    // como «sin registro» y trata la máquina como un live —con su log en la
    // ESP y sin retirar nada—. Así que al provisionar hay que **declarar** lo
    // que es, con el GUID de su propia ESP.
    match declarar_instalada(&mut raiz, esp) {
        Ok(true) => detalle.push_str("SOSOMODE.TXT: declarada instalada\n"),
        Ok(false) => detalle.push_str("SOSOMODE.TXT: ya estaba declarada\n"),
        Err(e) => return Err(format!("declarar la identidad: {e}")),
    }

    // El shim viejo no entiende la entrada de rescate, así que registrarla sin
    // sustituirlo dejaría una opción en el menú que no hace nada. Se copia
    // **antes** de registrar nada, y verificándolo.
    match copiar_shim(&mut raiz) {
        Ok(n) => detalle.push_str(&format!("bootx64.efi actualizado ({n} B)\n")),
        Err(e) => return Err(format!("bootx64.efi: {e}")),
    }

    let normal = crate::bootentry::registrar(esp, crate::bootentry::DESC, None)
        .map_err(|e| format!("entrada de arranque: {e}"))?;
    detalle.push_str(&format!("Boot{normal:04X} {}\n", crate::bootentry::DESC));
    match crate::bootentry::registrar(
        esp,
        crate::bootentry::DESC_RESCATE,
        Some(crate::rescate::SENAL),
    ) {
        Ok(r) => detalle.push_str(&format!("rescate Boot{r:04X}\n")),
        // Igual que al instalar: sin la de rescate la instalación sigue siendo
        // buena, y lo que se pierde es una comodidad, no el arranque.
        Err(e) => detalle.push_str(&format!("rescate NO registrado: {e}\n")),
    }
    Ok(detalle)
}

enum Accion {
    YaEstaba,
    Creado,
}

const MODO: &CStr16 = cstr16!("SOSOMODE.TXT");

/// Escribe la identidad `installed` si no hay ninguna legible. Devuelve si hizo
/// falta escribirla.
///
/// No se pisa una identidad que ya esté: si la máquina ya se declaró, quien lo
/// hizo sabía más que nosotros —por ejemplo el propio instalador, con su fecha.
fn declarar_instalada(raiz: &mut Directory, esp: Guid) -> Result<bool, String> {
    use soso_update_core::identity::{BootMode, ModeRecord};

    let actual = leer_de(raiz, MODO).unwrap_or_default();
    let guid = format!("{esp}");
    if let soso_update_core::Identidad::Explicita(r) =
        soso_update_core::resolver_identidad(Some(&actual), &guid)
    {
        if r.modo == BootMode::Installed {
            return Ok(false);
        }
    }
    let rec = ModeRecord::nuevo(BootMode::Installed, "", &guid, 1);
    let bytes = rec
        .format()
        .map_err(|e| format!("formatear el registro: {e:?}"))?;
    // Por ranuras, como todos los registros durables: se escribe la que toca
    // por secuencia y el resto se queda como estaba.
    let mut datos = actual;
    datos.resize(soso_update_core::UPD_MODE_SIZE, 0);
    let off = rec.ranura() * soso_update_core::SLOT_SIZE;
    datos[off..off + bytes.len()].copy_from_slice(&bytes);
    escribir_exacto(raiz, MODO, &datos)?;
    Ok(true)
}

/// Deja el hueco con **exactamente** el tamaño que el kernel espera. Uno de
/// otro tamaño no le sirve —los localiza por LBA exigiendo la medida—, así que
/// se rehace entero en vez de dejarlo «casi bien».
fn asegurar_hueco(raiz: &mut Directory, nombre: &CStr16, tamano: usize) -> Result<Accion, String> {
    if let Ok(handle) = raiz.open(nombre, FileMode::Read, FileAttribute::empty()) {
        if let Some(mut f) = handle.into_regular_file() {
            let info = f
                .get_boxed_info::<FileInfo>()
                .map_err(|e| format!("get_info: {:?}", e.status()))?;
            if info.file_size() == tamano as u64 {
                return Ok(Accion::YaEstaba);
            }
        }
        // Existe con otro tamaño: se borra para que el driver FAT vuelva a
        // asignarle clusters desde cero, que es la única forma de tener
        // alguna posibilidad de que queden consecutivos.
        let handle = raiz
            .open(nombre, FileMode::ReadWrite, FileAttribute::empty())
            .map_err(|e| format!("abrir para borrar: {:?}", e.status()))?;
        if let Some(f) = handle.into_regular_file() {
            f.delete().map_err(|e| format!("borrar: {:?}", e.status()))?;
        }
    }

    let handle = raiz
        .open(nombre, FileMode::CreateReadWrite, FileAttribute::empty())
        .map_err(|e| format!("crear: {:?}", e.status()))?;
    let mut f: RegularFile = handle.into_regular_file().ok_or("no es un fichero regular")?;
    // A ceros y por trozos: 64 MiB de una vez no caben cómodamente en el heap
    // del firmware, y el driver FAT va asignando clusters según escribe.
    let cero = vec![0u8; 64 * 1024];
    let mut escrito = 0usize;
    while escrito < tamano {
        let n = cero.len().min(tamano - escrito);
        f.write(&cero[..n])
            .map_err(|e| format!("escribir: {:?}", e.status()))?;
        escrito += n;
    }
    f.flush().map_err(|e| format!("flush: {:?}", e.status()))?;
    Ok(Accion::Creado)
}

/// Copia este mismo shim a la ESP del destino, verificándolo, y devolviendo los
/// bytes viejos a su sitio si la copia no se relee igual: dejar a medias el
/// fichero que arranca la máquina es la única avería de aquí que no tiene
/// vuelta desde el propio sistema.
fn copiar_shim(raiz: &mut Directory) -> Result<usize, String> {
    let nuevo = leer_propio(SHIM)?;
    let viejo = leer_de(raiz, SHIM).unwrap_or_default();
    escribir_exacto(raiz, SHIM, &nuevo)?;
    let releido = leer_de(raiz, SHIM).ok_or("no pude releer bootx64.efi")?;
    if releido != nuevo {
        if !viejo.is_empty() {
            let _ = escribir_exacto(raiz, SHIM, &viejo);
        }
        return Err("la copia no se relee igual; dejé el anterior".into());
    }
    Ok(nuevo.len())
}

fn leer_propio(ruta: &CStr16) -> Result<Vec<u8>, String> {
    let mut fs = boot::get_image_file_system(boot::image_handle())
        .map_err(|e| format!("mi propia ESP: {:?}", e.status()))?;
    let mut raiz = fs
        .open_volume()
        .map_err(|e| format!("open_volume: {:?}", e.status()))?;
    leer_de(&mut raiz, ruta).ok_or_else(|| "no encuentro mi propio bootx64.efi".to_string())
}

fn leer_de(raiz: &mut Directory, ruta: &CStr16) -> Option<Vec<u8>> {
    let handle = raiz.open(ruta, FileMode::Read, FileAttribute::empty()).ok()?;
    let mut f = handle.into_regular_file()?;
    let mut datos = Vec::new();
    let mut buf = [0u8; 32 * 1024];
    loop {
        let n = f.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        datos.extend_from_slice(&buf[..n]);
    }
    Some(datos)
}

fn escribir_exacto(raiz: &mut Directory, ruta: &CStr16, datos: &[u8]) -> Result<(), String> {
    let handle = raiz
        .open(ruta, FileMode::CreateReadWrite, FileAttribute::empty())
        .map_err(|e| format!("abrir: {:?}", e.status()))?;
    let mut f: RegularFile = handle.into_regular_file().ok_or("no es un fichero regular")?;
    f.set_position(0)
        .map_err(|e| format!("set_position: {:?}", e.status()))?;
    f.write(datos)
        .map_err(|e| format!("escribir: {:?}", e.status()))?;
    f.flush().map_err(|e| format!("flush: {:?}", e.status()))?;
    truncar(&mut f, datos.len() as u64)
}

fn truncar(f: &mut RegularFile, size: u64) -> Result<(), String> {
    let info = f
        .get_boxed_info::<FileInfo>()
        .map_err(|e| format!("get_info: {:?}", e.status()))?;
    if info.file_size() == size {
        return Ok(());
    }
    let mut buf = vec![0u8; 512];
    let nuevo = FileInfo::new(
        &mut buf,
        size,
        info.physical_size(),
        *info.create_time(),
        *info.last_access_time(),
        *info.modification_time(),
        info.attribute(),
        info.file_name(),
    )
    .map_err(|_| "new_info".to_string())?;
    f.set_info(nuevo)
        .map_err(|e| format!("set_info: {:?}", e.status()))
}
