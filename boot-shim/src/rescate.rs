//! Entrada UEFI de rescate: «volver a la versión anterior» sin init ni red.
//!
//! La vía normal para deshacer una actualización es `soso-update revertir`,
//! que necesita que el sistema arranque y que sosh responda. Esta es la vía
//! para cuando **no** arranca: una segunda entrada `Boot####` que apunta al
//! mismo cargador y trae `rescatar` en su OptionalData. El firmware la ofrece
//! en su menú, el shim la reconoce aquí, y lo único que hace es **pedir** el
//! rescate en el registro de arranque de la ESP. La restauración la ejecuta el
//! kernel en ese mismo arranque, que es quien puede leer sosofs y comprobar el
//! punto entero antes de tocar nada.
//!
//! Nada de esto puede impedir el arranque: si falla, se anota y se sigue.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use soso_update_core::record::SLOT_SIZE;
use soso_update_core::txn::bootrec::{BootRecord, BOOTREC_SIZE};
use soso_update_core::txn::rescate::{es_senal, planear, Plan, Sin};
use uefi::boot;
use uefi::proto::loaded_image::LoadedImage;
use uefi::proto::media::file::{File, FileAttribute, FileMode, RegularFile};
use uefi::{cstr16, CStr16};

const TXN: &CStr16 = cstr16!("SOSOTXN.BIN");
/// Lo que lleva la entrada de rescate en su OptionalData. La decisión de qué
/// hacer con ella vive en `soso-update-core`, donde se puede probar en el host;
/// aquí sólo está la E/S de la ESP.
pub use soso_update_core::txn::rescate::SENAL;

/// ¿Nos han arrancado por la entrada de rescate?
pub fn pedido() -> bool {
    let Ok(li) = boot::open_protocol_exclusive::<LoadedImage>(boot::image_handle()) else {
        return false;
    };
    // El firmware entrega los datos tal cual se guardaron. La entrada la
    // escribimos nosotros en UTF-16, pero un firmware que los pase en ASCII no
    // debería dejar sin rescate a quien lo necesita: se miran las dos formas.
    if let Ok(s) = li.load_options_as_cstr16() {
        if es_senal(&s.to_string()) {
            return true;
        }
    }
    match li.load_options_as_bytes() {
        Some(b) => es_senal(&String::from_utf8_lossy(b)),
        None => false,
    }
}

/// Atiende la petición de rescate, si la hay. Devuelve una línea para el log.
pub fn atender() -> Option<String> {
    if !pedido() {
        return None;
    }
    Some(match pedir_rescate() {
        Ok(linea) => linea,
        Err(e) => format!("ERROR {e}"),
    })
}

fn pedir_rescate() -> Result<String, String> {
    let raw = leer(TXN, BOOTREC_SIZE)
        .ok_or_else(|| "no encuentro SOSOTXN.BIN en la ESP".to_string())?;
    let rec = BootRecord::pick(&raw)
        .map_err(|e| format!("registro de arranque ilegible ({e:?})"))?;

    let (registro, desde, hasta) = match planear(&rec) {
        Plan::Pedir {
            registro,
            desde,
            hasta,
        } => (registro, desde, hasta),
        Plan::YaPedido { hasta } => {
            return Ok(format!(
                "rescate: ya estaba pedido; volverás a {hasta} en este arranque"
            ))
        }
        Plan::No(Sin::FormatoAntiguo) => {
            return Err("el registro de arranque no sabe de copias guardadas (formato antiguo)".into())
        }
        Plan::No(Sin::SinPunto) => {
            return Err("no consta ninguna copia guardada a la que volver".into())
        }
        Plan::No(Sin::YaRestaurado { version }) => {
            return Err(format!(
                "ya estás en {version}: esa vuelta atrás ya se hizo y no hay otra copia"
            ))
        }
    };

    let bytes = registro
        .format()
        .map_err(|e| format!("no pude formar el registro ({e:?})"))?;
    escribir_en(TXN, (registro.ranura() * SLOT_SIZE) as u64, &bytes)
        .map_err(|e| format!("no pude escribir SOSOTXN.BIN: {e}"))?;

    // La pareja vuelve entera o no vuelve: el rootfs lo restaura el kernel
    // leyendo el punto, y el kernel se restaura desde el hueco de la ESP por
    // el buzón de siempre, en este mismo arranque.
    let kernel = if crate::actualiza::hay_copia_de_kernel() {
        match crate::actualiza::pedir_revertir_kernel() {
            Ok(()) => "kernel incluido",
            Err(e) => {
                return Err(format!(
                    "registrado el rootfs pero no pude pedir el kernel ({e}): no arranques hasta arreglarlo"
                ))
            }
        }
    } else {
        "sólo rootfs: no consta copia del kernel"
    };

    Ok(format!(
        "rescate: pedido {desde} → {hasta} ({kernel}); el kernel comprobará la copia antes de restaurar"
    ))
}

fn leer(path: &CStr16, max: usize) -> Option<Vec<u8>> {
    let mut fs = boot::get_image_file_system(boot::image_handle()).ok()?;
    let mut root = fs.open_volume().ok()?;
    let handle = root.open(path, FileMode::Read, FileAttribute::empty()).ok()?;
    let mut f = handle.into_regular_file()?;
    let mut data = Vec::new();
    let mut buf = [0u8; 4096];
    while data.len() < max {
        let n = f.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        let cabe = n.min(max - data.len());
        data.extend_from_slice(&buf[..cabe]);
    }
    Some(data)
}

/// Escribe en su sitio, sin tocar el resto ni el tamaño: las otras ranuras del
/// fichero son justamente las que hay que no perder.
fn escribir_en(path: &CStr16, off: u64, data: &[u8]) -> Result<(), &'static str> {
    let mut fs = boot::get_image_file_system(boot::image_handle()).map_err(|_| "sin filesystem")?;
    let mut root = fs.open_volume().map_err(|_| "open_volume")?;
    let handle = root
        .open(path, FileMode::ReadWrite, FileAttribute::empty())
        .map_err(|_| "open")?;
    let mut f: RegularFile = handle.into_regular_file().ok_or("no es un fichero")?;
    f.set_position(off).map_err(|_| "set_position")?;
    f.write(data).map_err(|_| "write")?;
    f.flush().map_err(|_| "flush")
}
