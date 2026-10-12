//! Registro de la entrada de arranque UEFI que pide el instalador.
//!
//! `soso-install` corre dentro de soso, donde ya no hay Runtime Services: el
//! kernel no puede llamar a `SetVariable` para crear un `Boot####`. Lo que sí
//! puede es dejar una nota en `SOSOBOOT.TXT`, en la ESP del pendrive. Este
//! módulo la lee en el siguiente arranque —con el firmware todavía al mando— y
//! crea la entrada apuntando a la ESP del disco recién instalado.
//!
//! Nada de esto puede impedir el arranque: cualquier fallo se anota en el
//! propio fichero y el shim sigue con el chainload.
//!
//! Formato de la petición (la escribe el instalador):
//!
//! ```text
//! SOSOBOOT v1
//! INSTALL C12A7328-F81F-11D2-BA4B-00A0C93EC93B
//! disco nvme1 id 3
//! ```
//!
//! Y la respuesta que deja este módulo: `DONE Boot0003 …` o `ERROR …`.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

use uefi::boot::{self, OpenProtocolAttributes, OpenProtocolParams, SearchType};
use uefi::proto::device_path::build::{self, DevicePathBuilder};
use uefi::proto::device_path::media::PartitionSignature;
use uefi::proto::device_path::{DevicePath, DevicePathNodeEnum};
use uefi::proto::media::file::{File, FileAttribute, FileMode};
use uefi::proto::media::fs::SimpleFileSystem;
use uefi::runtime::{self, VariableAttributes, VariableVendor};
use uefi::{CStr16, CString16, Guid, Handle, cstr16};

/// Tamaño con el que `xtask package-usb-live` reserva el fichero. Se reescribe
/// siempre entero para no dejar restos de la petición anterior ni cambiar el
/// tamaño que el kernel espera encontrar.
const FILE_SIZE: usize = 4096;
const REQ: &CStr16 = cstr16!("SOSOBOOT.TXT");
/// Ruta del cargador dentro de la ESP instalada (la imagen es un clon del USB).
const TARGET_LOADER: &CStr16 = cstr16!("\\EFI\\BOOT\\BOOTX64.EFI");
const DESC: &str = "soso";
const LOAD_OPTION_ACTIVE: u32 = 0x0000_0001;

/// Punto de entrada: atiende la petición pendiente, si la hay. Devuelve una
/// línea para el log de pantalla, o `None` si no había nada que hacer.
pub fn atender() -> Option<String> {
    let texto = leer_peticion()?;
    let guid = parse_install(&texto)?;

    let resultado = match registrar(guid) {
        Ok(num) => format!(
            "DONE Boot{num:04X} {DESC}\nesp {guid}\nEntrada de arranque registrada; \
             ya puedes quitar el USB.\n"
        ),
        Err(e) => format!("ERROR {e}\nesp {guid}\n"),
    };
    escribir_respuesta(&resultado);
    Some(resultado.lines().next().unwrap_or("").to_string())
}

/// Busca la línea `INSTALL <guid>`. Ignora peticiones ya atendidas (`DONE`).
fn parse_install(texto: &str) -> Option<Guid> {
    for line in texto.lines() {
        let line = line.trim();
        if line.starts_with("DONE") || line.starts_with("ERROR") {
            return None;
        }
        if let Some(g) = line.strip_prefix("INSTALL ") {
            return Guid::try_parse(g.trim()).ok();
        }
    }
    None
}

fn registrar(esp: Guid) -> Result<u16, String> {
    let handle = localizar_esp(esp).ok_or_else(|| {
        format!("no encuentro ninguna ESP con GUID de partición {esp}")
    })?;

    let mut dp_buf = Vec::new();
    let dp = ruta_cargador(handle, &mut dp_buf).map_err(|e| format!("device path: {e}"))?;
    let dp_bytes = dp.as_bytes();

    let mut option = Vec::with_capacity(64 + dp_bytes.len());
    option.extend_from_slice(&LOAD_OPTION_ACTIVE.to_le_bytes());
    option.extend_from_slice(&(dp_bytes.len() as u16).to_le_bytes());
    for c in DESC.encode_utf16() {
        option.extend_from_slice(&c.to_le_bytes());
    }
    option.extend_from_slice(&0u16.to_le_bytes()); // NUL de la descripción
    option.extend_from_slice(dp_bytes);

    let num = hueco_boot();
    let nombre = nombre_boot(num)?;
    runtime::set_variable(
        &nombre,
        &VariableVendor::GLOBAL_VARIABLE,
        VariableAttributes::NON_VOLATILE
            | VariableAttributes::BOOTSERVICE_ACCESS
            | VariableAttributes::RUNTIME_ACCESS,
        &option,
    )
    .map_err(|e| format!("SetVariable Boot{num:04X}: {:?}", e.status()))?;

    // Si el firmware reordena el arranque por su cuenta, la entrada sigue
    // estando en el menú: por eso un fallo aquí no invalida el registro.
    if let Err(e) = poner_primero_en_bootorder(num) {
        return Err(format!("Boot{num:04X} creada pero BootOrder falló: {e}"));
    }
    Ok(num)
}

/// Handle de la partición (con sistema de ficheros FAT) cuyo device path lleva
/// un nodo HardDrive con esta firma GPT.
fn localizar_esp(esp: Guid) -> Option<Handle> {
    let handles = boot::locate_handle_buffer(SearchType::from_proto::<SimpleFileSystem>()).ok()?;
    for &handle in handles.iter() {
        let Ok(dp) = abrir_device_path(handle) else {
            continue;
        };
        for node in dp.node_iter() {
            if let Ok(DevicePathNodeEnum::MediaHardDrive(hd)) = node.as_enum() {
                if hd.partition_signature() == PartitionSignature::Guid(esp) {
                    return Some(handle);
                }
            }
        }
    }
    None
}

fn abrir_device_path(handle: Handle) -> uefi::Result<uefi::boot::ScopedProtocol<DevicePath>> {
    // GetProtocol y no Exclusive: aquí solo se inspecciona, y abrir en
    // exclusiva desconectaría al driver que gestiona la partición.
    unsafe {
        boot::open_protocol::<DevicePath>(
            OpenProtocolParams {
                handle,
                agent: boot::image_handle(),
                controller: None,
            },
            OpenProtocolAttributes::GetProtocol,
        )
    }
}

/// Device path del disco destino + `\EFI\BOOT\BOOTX64.EFI`.
fn ruta_cargador(handle: Handle, buf: &mut Vec<u8>) -> Result<&DevicePath, String> {
    let dp = abrir_device_path(handle).map_err(|e| format!("{:?}", e.status()))?;
    let mut b = DevicePathBuilder::with_vec(buf);
    for node in dp.node_iter() {
        b = b.push(&node).map_err(|_| "sin memoria".to_string())?;
    }
    b.push(&build::media::FilePath {
        path_name: TARGET_LOADER,
    })
    .map_err(|_| "sin memoria".to_string())?
    .finalize()
    .map_err(|_| "device path inválido".to_string())
}

/// Reutiliza la `Boot####` que ya describa a soso —reinstalar no debe dejar el
/// menú lleno de entradas duplicadas— y si no, coge el número libre más bajo.
fn hueco_boot() -> u16 {
    let claves: Vec<CString16> = runtime::variable_keys()
        .filter_map(|k| k.ok())
        .filter(|k| k.vendor == VariableVendor::GLOBAL_VARIABLE)
        .map(|k| k.name)
        .collect();

    let mut usados = Vec::new();
    for nombre in &claves {
        let Some(num) = numero_boot(nombre) else {
            continue;
        };
        usados.push(num);
        if let Ok((datos, _)) = runtime::get_variable_boxed(nombre, &VariableVendor::GLOBAL_VARIABLE)
        {
            if descripcion(&datos).as_deref() == Some(DESC) {
                return num;
            }
        }
    }
    (0u16..=0xFFFF)
        .find(|n| !usados.contains(n))
        .unwrap_or(0xFFFF)
}

/// `Boot0003` → `Some(3)`; cualquier otro nombre → `None`.
fn numero_boot(nombre: &CString16) -> Option<u16> {
    let s = nombre.to_string();
    let hex = s.strip_prefix("Boot")?;
    if hex.len() != 4 {
        return None;
    }
    u16::from_str_radix(hex, 16).ok()
}

/// Descripción de un EFI_LOAD_OPTION: UTF-16 terminado en NUL tras los 6 bytes
/// de cabecera (atributos + longitud del device path).
fn descripcion(datos: &[u8]) -> Option<String> {
    if datos.len() < 8 {
        return None;
    }
    let mut units = Vec::new();
    let mut i = 6;
    while i + 1 < datos.len() {
        let c = u16::from_le_bytes([datos[i], datos[i + 1]]);
        if c == 0 {
            return String::from_utf16(&units).ok();
        }
        units.push(c);
        i += 2;
    }
    None
}

fn nombre_boot(num: u16) -> Result<CString16, String> {
    CString16::try_from(format!("Boot{num:04X}").as_str())
        .map_err(|_| "nombre de variable inválido".to_string())
}

fn poner_primero_en_bootorder(num: u16) -> Result<(), String> {
    let nombre = cstr16!("BootOrder");
    let actual = runtime::get_variable_boxed(nombre, &VariableVendor::GLOBAL_VARIABLE)
        .map(|(d, _)| d.to_vec())
        .unwrap_or_default();

    let mut orden: Vec<u16> = actual
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .filter(|&n| n != num)
        .collect();
    orden.insert(0, num);

    let mut bytes = Vec::with_capacity(orden.len() * 2);
    for n in orden {
        bytes.extend_from_slice(&n.to_le_bytes());
    }
    runtime::set_variable(
        nombre,
        &VariableVendor::GLOBAL_VARIABLE,
        VariableAttributes::NON_VOLATILE
            | VariableAttributes::BOOTSERVICE_ACCESS
            | VariableAttributes::RUNTIME_ACCESS,
        &bytes,
    )
    .map_err(|e| format!("{:?}", e.status()))
}

// -------------------------------------------------------------- el fichero

fn leer_peticion() -> Option<String> {
    let mut fs = boot::get_image_file_system(boot::image_handle()).ok()?;
    let mut root = fs.open_volume().ok()?;
    let handle = root.open(REQ, FileMode::Read, FileAttribute::empty()).ok()?;
    let mut f = handle.into_regular_file()?;
    let mut buf = vec![0u8; FILE_SIZE];
    let n = f.read(&mut buf).ok()?;
    Some(String::from_utf8_lossy(&buf[..n]).into_owned())
}

/// Reescribe el fichero completo: escribir menos dejaría cola de la petición
/// anterior, porque el protocolo de ficheros UEFI no trunca al escribir.
fn escribir_respuesta(texto: &str) {
    let Ok(mut fs) = boot::get_image_file_system(boot::image_handle()) else {
        return;
    };
    let Ok(mut root) = fs.open_volume() else {
        return;
    };
    let Ok(handle) = root.open(REQ, FileMode::ReadWrite, FileAttribute::empty()) else {
        return;
    };
    let Some(mut f) = handle.into_regular_file() else {
        return;
    };
    let mut datos = vec![b'\n'; FILE_SIZE];
    let cuerpo = format!("SOSOBOOT v1\n{texto}");
    let n = cuerpo.len().min(FILE_SIZE);
    datos[..n].copy_from_slice(&cuerpo.as_bytes()[..n]);
    let _ = f.set_position(0);
    let _ = f.write(&datos);
    let _ = f.flush();
}
