//! IOMMU AMD-Vi: detección por IVRS y apagado si el firmware lo dejó activo.
//!
//! Motivo: soso programa DMA físico directo (NVMe, xHCI, WiFi). Si el firmware
//! entrega la máquina con el IOMMU traduciendo y sus propias tablas de
//! dispositivo, cada transferencia que programemos se aborta y el dispositivo
//! queda mudo — indistinguible de un driver mal escrito, y sin serie en la
//! Steam Deck no hay forma de distinguirlos. Así que se mira siempre y se deja
//! escrito en el log qué se ha encontrado.
//!
//! Referencia: `iommu_disable()` en
//! `lxdde/linux/drivers/iommu/amd/init.c:470`. Linux limpia en orden el buffer
//! de comandos, el log de eventos, GA y PPR, y sólo al final `IOMMU_EN`. El
//! parseo de la tabla vive en `soso-hw` para poder probarlo en host.

use soso_hw::ivrs::{self, Iommu};
use spin::Once;

use crate::mm;
use crate::println;

/// Los AMD-Vi de una APU son uno o dos; el ROG y la Deck, uno.
const MAX_IOMMU: usize = 4;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Estado {
    /// No hay IVRS: máquina sin AMD-Vi (Intel, QEMU sin `-device amd-iommu`).
    SinIvrs,
    /// IVRS presente pero ilegible.
    IvrsInvalida,
    /// Encontrado y ya venía apagado por el firmware.
    Apagado,
    /// Venía activo y lo hemos apagado nosotros.
    Desactivado,
    /// Venía activo y **no** se pudo apagar: el DMA directo es sospechoso.
    SigueActivo,
}

impl Estado {
    pub fn texto(self) -> &'static str {
        match self {
            Estado::SinIvrs => "sin-ivrs",
            Estado::IvrsInvalida => "ivrs-invalida",
            Estado::Apagado => "apagado-por-firmware",
            Estado::Desactivado => "desactivado-por-soso",
            Estado::SigueActivo => "SIGUE-ACTIVO",
        }
    }
}

static ESTADO: Once<Estado> = Once::new();

/// Estado del IOMMU tras `init`. Antes de llamarlo, `SinIvrs`.
pub fn estado() -> Estado {
    ESTADO.get().copied().unwrap_or(Estado::SinIvrs)
}

unsafe fn leer64(phys: u64) -> u64 {
    unsafe { core::ptr::read_volatile(mm::phys_to_virt(phys).as_ptr::<u64>()) }
}

unsafe fn escribir64(phys: u64, v: u64) {
    unsafe { core::ptr::write_volatile(mm::phys_to_virt(phys).as_mut_ptr::<u64>(), v) };
}

/// Detecta los IOMMU AMD-Vi y los apaga si están traduciendo.
///
/// Debe ir **antes** de `pci::init` y de cualquier driver que programe DMA.
pub fn init() {
    let estado = ESTADO.call_once(detectar_y_apagar);
    println!("iommu: {}", estado.texto());
}

fn detectar_y_apagar() -> Estado {
    let Some(tabla) = crate::arch::acpi::tabla_bytes(b"IVRS") else {
        return Estado::SinIvrs;
    };
    let mut encontrados = [Iommu {
        ivhd_type: 0,
        devid: 0,
        cap_ptr: 0,
        mmio_phys: 0,
        pci_seg: 0,
    }; MAX_IOMMU];
    let n = match ivrs::parse(tabla, &mut encontrados) {
        Ok(n) => n,
        Err(e) => {
            println!("iommu: IVRS ilegible ({e:?})");
            return Estado::IvrsInvalida;
        }
    };
    if n == 0 {
        println!("iommu: IVRS sin IVHD utilizable");
        return Estado::IvrsInvalida;
    }

    let mut peor = Estado::Apagado;
    for iommu in &encontrados[..n] {
        // El bloque de registros del IOMMU son 0x4000 bytes (el log de
        // comandos vive en 0x2000); mapear de menos dejaría fuera lecturas
        // futuras y no cuesta nada mapear la página entera.
        mm::ensure_mmio_mapped(iommu.mmio_phys, 0x4000);
        let ctrl_phys = iommu.mmio_phys + ivrs::MMIO_CONTROL_OFFSET;
        let ctrl = unsafe { leer64(ctrl_phys) };
        println!(
            "iommu: ivhd{:#x} devid {:04x} mmio {:#x} control {:#x}",
            iommu.ivhd_type, iommu.devid, iommu.mmio_phys, ctrl
        );
        if !ivrs::habilitado(ctrl) {
            continue;
        }
        let nuevo = ivrs::control_apagado(ctrl);
        unsafe { escribir64(ctrl_phys, nuevo) };
        let confirmado = unsafe { leer64(ctrl_phys) };
        if ivrs::habilitado(confirmado) {
            println!(
                "iommu: NO se pudo apagar (control {confirmado:#x}); el DMA directo puede fallar"
            );
            peor = Estado::SigueActivo;
        } else {
            println!("iommu: estaba activo; apagado (control {confirmado:#x})");
            if peor != Estado::SigueActivo {
                peor = Estado::Desactivado;
            }
        }
    }
    peor
}
