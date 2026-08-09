//! Shim UEFI de diagnóstico para el arranque en hardware real.
//!
//! Se instala como `efi/boot/bootx64.efi` y deja constancia en `BOOTMARK.TXT`
//! (raíz de la ESP) de que el firmware llegó a ejecutar nuestro código, con
//! datos del firmware. Después chainloadea el bootloader real
//! (`efi/boot/bootsoso.efi`). Si el USB vuelve a Linux con BOOTMARK.TXT
//! intacto, el firmware nunca nos arrancó; si tiene la marca pero SOSOLOG.TXT
//! está vacío, el fallo es del kernel (mira los checkpoints `boot:` en
//! pantalla).

#![no_main]
#![no_std]

extern crate alloc;

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::time::Duration;

use uefi::boot::{self, LoadImageSource};
use uefi::prelude::*;
use uefi::proto::device_path::build::{self, DevicePathBuilder};
use uefi::proto::device_path::DevicePath;
use uefi::proto::loaded_image::LoadedImage;
use uefi::proto::media::file::{File, FileAttribute, FileMode, RegularFile};
use uefi::{cstr16, system, CStr16};

const REAL_LOADER: &CStr16 = cstr16!("efi\\boot\\bootsoso.efi");
const REAL_LOADER_ABS: &CStr16 = cstr16!("\\efi\\boot\\bootsoso.efi");
const MARK: &CStr16 = cstr16!("BOOTMARK.TXT");

#[entry]
fn main() -> Status {
    let _ = uefi::helpers::init();
    uefi::println!("soso-shim: UEFI alcanzado; cargando bootsoso.efi…");

    let vendor: String = system::firmware_vendor().to_string();
    let rev = system::firmware_revision();
    let uefi_rev = system::uefi_revision();
    let head = format!(
        "soso-shim: UEFI alcanzado\nfirmware: {vendor} rev {rev:#x} (UEFI {}.{})\n",
        uefi_rev.major(),
        uefi_rev.minor(),
    );
    write_mark(&head);

    let loader = match read_loader() {
        Ok(data) => data,
        Err(st) => {
            return fail(&format!("{head}ERROR leyendo efi\\boot\\bootsoso.efi: {st:?}\n"), st);
        }
    };
    write_mark(&format!(
        "{head}bootsoso.efi leído ({} bytes); saltando al bootloader\n",
        loader.len()
    ));

    // Device path completo del loader: sin él, el LoadedImage del bootloader
    // no tendría DeviceHandle y no encontraría kernel-x86_64 en la ESP.
    let mut dp_buf = Vec::new();
    let file_path = match loader_device_path(&mut dp_buf) {
        Ok(p) => p,
        Err(st) => {
            return fail(&format!("{head}ERROR construyendo device path: {st:?}\n"), st);
        }
    };

    let image = match boot::load_image(
        boot::image_handle(),
        LoadImageSource::FromBuffer {
            buffer: &loader,
            file_path: Some(file_path),
        },
    ) {
        Ok(h) => h,
        Err(e) => {
            return fail(&format!("{head}ERROR en LoadImage: {e:?}\n"), e.status());
        }
    };
    if let Err(e) = boot::start_image(image) {
        return fail(&format!("{head}ERROR en StartImage: {e:?}\n"), e.status());
    }
    // El bootloader no debería devolver el control.
    write_mark(&format!("{head}aviso: el bootloader devolvió el control\n"));
    Status::SUCCESS
}

/// Escribe el error en BOOTMARK.TXT y en pantalla, y da tiempo a leerlo.
fn fail(msg: &str, st: Status) -> Status {
    write_mark(msg);
    uefi::println!("soso-shim: {msg}");
    boot::stall(Duration::from_secs(30));
    st
}

/// Sobrescribe BOOTMARK.TXT desde el principio (se crea si no existe).
/// Best-effort: un fallo aquí no debe impedir el arranque.
fn write_mark(text: &str) {
    let Ok(mut fs) = boot::get_image_file_system(boot::image_handle()) else {
        return;
    };
    let Ok(mut root) = fs.open_volume() else {
        return;
    };
    let Ok(handle) = root.open(MARK, FileMode::CreateReadWrite, FileAttribute::empty()) else {
        return;
    };
    let Some(mut f) = handle.into_regular_file() else {
        return;
    };
    let _ = f.set_position(0);
    let _ = f.write(text.as_bytes());
    let _ = f.flush();
}

/// Device path completo del loader real: los nodos del dispositivo del que
/// nos cargaron + un nodo FilePath con `\efi\boot\bootsoso.efi`.
fn loader_device_path(buf: &mut Vec<u8>) -> Result<&DevicePath, Status> {
    let li = boot::open_protocol_exclusive::<LoadedImage>(boot::image_handle())
        .map_err(|e| e.status())?;
    let device = li.device().ok_or(Status::NOT_FOUND)?;
    let dp = boot::open_protocol_exclusive::<DevicePath>(device).map_err(|e| e.status())?;
    let mut b = DevicePathBuilder::with_vec(buf);
    for node in dp.node_iter() {
        b = b.push(&node).map_err(|_| Status::OUT_OF_RESOURCES)?;
    }
    b.push(&build::media::FilePath {
        path_name: REAL_LOADER_ABS,
    })
    .map_err(|_| Status::OUT_OF_RESOURCES)?
    .finalize()
    .map_err(|_| Status::OUT_OF_RESOURCES)
}

fn read_loader() -> Result<Vec<u8>, Status> {
    let mut fs =
        boot::get_image_file_system(boot::image_handle()).map_err(|e| e.status())?;
    let mut root = fs.open_volume().map_err(|e| e.status())?;
    let handle = root
        .open(REAL_LOADER, FileMode::Read, FileAttribute::empty())
        .map_err(|e| e.status())?;
    let mut f: RegularFile = handle.into_regular_file().ok_or(Status::NOT_FOUND)?;
    let mut data = Vec::new();
    let mut buf = [0u8; 32 * 1024];
    loop {
        let n = f.read(&mut buf).map_err(|e| e.status())?;
        if n == 0 {
            break;
        }
        data.extend_from_slice(&buf[..n]);
    }
    Ok(data)
}
