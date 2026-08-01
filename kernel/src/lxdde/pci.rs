//! PCI shim sobre drivers/pci.rs de soso.

use crate::drivers::pci;
use crate::mm;
use alloc::string::String;
use alloc::vec::Vec;
use core::ffi::c_void;
use spin::Mutex;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct LxPciDeviceId {
    pub vendor: u32,
    pub device: u32,
    pub subvendor: u32,
    pub subdevice: u32,
    pub class: u32,
    pub class_mask: u32,
    pub driver_data: u64,
}

#[repr(C)]
pub struct LxPciDev {
    bus: u8,
    device: u8,
    function: u8,
    vendor_id: u16,
    device_id: u16,
    bar0: u64,
    bar0_size: u64,
    /// Apertura de FB (BAR1) y su tamaño, capturados AQUÍ y no cuando se piden.
    /// Dimensionar un BAR exige escribirle unos y devolverlo: en QEMU lo emula
    /// vfio-pci y es inocuo, pero sobre un dispositivo vivo en metal desnudo no,
    /// y el bring-up del GSP lo pide con la tarjeta ya en marcha. RM tampoco
    /// sirve: `sriovCaps.bar1Size` vino a 0 en la GB205 (silicio, 2026-08-01).
    bar1: u64,
    bar1_size: u64,
    data: usize,
    mmio: u64,
    irq_vectors: Vec<u8>,
}

struct DriverReg {
    name: String,
    ids: Vec<LxPciDeviceId>,
    probe: extern "C" fn(*mut LxPciDev, *const LxPciDeviceId) -> i32,
    remove: Option<extern "C" fn(*mut LxPciDev)>,
}

static DRIVERS: Mutex<Vec<DriverReg>> = Mutex::new(Vec::new());
static DEVICES: Mutex<Vec<LxPciDev>> = Mutex::new(Vec::new());

pub fn init() {
    for drv in DRIVERS.lock().iter() {
        let devs = pci::enumerate();
        for d in devs {
            if !match_id(&drv.ids, d.vendor_id, d.device_id) {
                continue;
            }
            let (bar0, bar0_size) = pci::bar_info(d.bus, d.device, d.function, 0).unwrap_or((0, 0));
            let (bar1, bar1_size) =
                pci::bar_info(d.bus, d.device, d.function, 1).unwrap_or((0, 0));
            let mut lx = LxPciDev {
                bus: d.bus,
                device: d.device,
                function: d.function,
                vendor_id: d.vendor_id,
                device_id: d.device_id,
                bar0,
                bar0_size,
                bar1,
                bar1_size,
                data: 0,
                mmio: 0,
                irq_vectors: Vec::new(),
            };
            let id = LxPciDeviceId {
                vendor: d.vendor_id as u32,
                device: d.device_id as u32,
                subvendor: 0,
                subdevice: 0,
                class: 0,
                class_mask: 0,
                driver_data: 0,
            };
            let ptr = &mut lx as *mut LxPciDev;
            DEVICES.lock().push(lx);
            let dev_ptr = DEVICES.lock().last_mut().unwrap() as *mut LxPciDev;
            let r = (drv.probe)(dev_ptr, &id);
            if r != 0 {
                DEVICES.lock().pop();
            } else {
                let _ = ptr;
                crate::println!("lxdde-pci: {} probe {:04x}:{:04x} ok", drv.name, d.vendor_id, d.device_id);
            }
        }
    }
}

fn match_id(ids: &[LxPciDeviceId], vendor: u16, device: u16) -> bool {
    ids.iter().any(|id| {
        id.vendor == 0 && id.device == 0
            || (id.vendor as u16 == vendor && (id.device == 0 || id.device as u16 == device))
    }) && !ids.is_empty()
}

#[unsafe(no_mangle)]
pub extern "C" fn lx_pci_register_driver(
    name: *const u8,
    ids: *const LxPciDeviceId,
    probe: extern "C" fn(*mut LxPciDev, *const LxPciDeviceId) -> i32,
    remove: extern "C" fn(*mut LxPciDev),
) -> i32 {
    if name.is_null() || ids.is_null() {
        return -1;
    }
    let mut id_list = Vec::new();
    unsafe {
        let mut p = ids;
        loop {
            let id = *p;
            if id.vendor == 0 && id.device == 0 {
                break;
            }
            id_list.push(id);
            p = p.add(1);
        }
    }
    let name_str = unsafe {
        let mut len = 0usize;
        while *name.add(len) != 0 {
            len += 1;
        }
        let slice = core::slice::from_raw_parts(name, len);
        String::from_utf8_lossy(slice).into_owned()
    };
    DRIVERS.lock().push(DriverReg {
        name: name_str,
        ids: id_list,
        probe,
        remove: Some(remove),
    });
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn lx_pci_enable_device(dev: *mut LxPciDev) -> i32 {
    if dev.is_null() {
        return -1;
    }
    unsafe {
        let d = &*dev;
        let mut cmd = pci::read16(d.bus, d.device, d.function, 0x04);
        cmd |= 0x6;
        pci::write16(d.bus, d.device, d.function, 0x04, cmd);
    }
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn lx_pci_disable_device(_dev: *mut LxPciDev) {}

#[unsafe(no_mangle)]
pub extern "C" fn lx_pci_set_master(dev: *mut LxPciDev) {
    if dev.is_null() {
        return;
    }
    unsafe {
        let d = &*dev;
        let mut cmd = pci::read16(d.bus, d.device, d.function, 0x04);
        cmd |= 0x4;
        pci::write16(d.bus, d.device, d.function, 0x04, cmd);
    }
}

/// Quita `Bus Master Enable` (bit 2 de COMMAND). Sin él el dispositivo no puede
/// iniciar transacciones: es lo que deja la GPU incapaz de hacer DMA antes de
/// que el host le pase un reset por encima (ver `gsp_fini.c`).
#[unsafe(no_mangle)]
pub extern "C" fn lx_pci_clear_master(dev: *mut LxPciDev) {
    if dev.is_null() {
        return;
    }
    unsafe {
        let d = &*dev;
        let cmd = pci::read16(d.bus, d.device, d.function, 0x04);
        pci::write16(d.bus, d.device, d.function, 0x04, cmd & !0x4);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn lx_pci_iomap(dev: *mut LxPciDev, bar: i32, _max_len: u64) -> *mut c_void {
    if dev.is_null() || bar != 0 {
        return core::ptr::null_mut();
    }
    unsafe {
        let d = &mut *dev;
        let size = d.bar0_size.max(0x20000);
        if d.vendor_id == 0x10de {
            let va = mm::map_dma_wc(d.bar0, size);
            d.mmio = d.bar0;
            return va.as_mut_ptr::<u8>() as *mut c_void;
        }
        mm::ensure_mmio_mapped(d.bar0, size);
        d.mmio = d.bar0;
        mm::phys_to_virt(d.bar0).as_mut_ptr::<u8>() as *mut c_void
    }
}

/// Base de la apertura de FB (BAR1) y, por `size_out`, su tamaño. Los dos salen
/// de la enumeración, no de un sondeo en caliente (ver el campo `bar1`).
/// Devuelve 0 si el dispositivo no tiene BAR1 utilizable.
#[unsafe(no_mangle)]
pub extern "C" fn lx_pci_bar1(dev: *mut LxPciDev, size_out: *mut u64) -> u64 {
    if dev.is_null() {
        return 0;
    }
    unsafe {
        let d = &*dev;
        if !size_out.is_null() {
            *size_out = d.bar1_size;
        }
        d.bar1
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn lx_pci_iounmap(_dev: *mut LxPciDev, _addr: *mut c_void) {}

#[unsafe(no_mangle)]
pub extern "C" fn lx_pci_read_config(dev: *mut LxPciDev, offset: i32, size: i32) -> u32 {
    if dev.is_null() {
        return 0;
    }
    unsafe {
        let d = &*dev;
        match size {
            1 => {
                let v = pci::read32(d.bus, d.device, d.function, (offset & !3) as u8);
                ((v >> ((offset & 3) * 8)) & 0xff) as u32
            }
            2 => pci::read16(d.bus, d.device, d.function, offset as u8) as u32,
            _ => pci::read32(d.bus, d.device, d.function, offset as u8),
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn lx_pci_write_config(dev: *mut LxPciDev, offset: i32, val: u32, size: i32) {
    if dev.is_null() {
        return;
    }
    unsafe {
        let d = &*dev;
        match size {
            1 => {
                let off = (offset & !3) as u8;
                let shift = (offset & 3) * 8;
                let old = pci::read32(d.bus, d.device, d.function, off);
                let mask = !(0xffu32 << shift);
                pci::write32(d.bus, d.device, d.function, off, (old & mask) | ((val & 0xff) << shift));
            }
            2 => pci::write16(d.bus, d.device, d.function, offset as u8, val as u16),
            _ => pci::write32(d.bus, d.device, d.function, offset as u8, val),
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn lx_pci_alloc_irq_vectors(dev: *mut LxPciDev, min: u32, max: u32, _flags: u32) -> i32 {
    if dev.is_null() {
        return -1;
    }
    let count = min.max(1).min(max.max(1));
    unsafe {
        let d = &mut *dev;
        d.irq_vectors.clear();
        if let Some(info) = pci::find_msix(d.bus, d.device, d.function) {
            if let Some(stub) = super::irq::stub_for(0x42) {
                if let Some(v) = crate::arch::irq::allocate(stub) {
                    let _ = pci::msix_setup(&info, 0, v, pci::msix_default_dest());
                    d.irq_vectors.push(v);
                    return count as i32;
                }
            }
        }
        if let Some(stub) = super::irq::stub_for(0x42) {
            if let Some(v) = crate::arch::irq::allocate(stub) {
                let (gsi, _) = crate::arch::ioapic::irq_to_gsi(11);
                let _ = crate::arch::ioapic::route_isa(11, v, gsi);
                d.irq_vectors.push(v);
                return count as i32;
            }
        }
    }
    -1
}

#[unsafe(no_mangle)]
pub extern "C" fn lx_pci_free_irq_vectors(dev: *mut LxPciDev) {
    if dev.is_null() {
        return;
    }
    unsafe {
        (*dev).irq_vectors.clear();
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn lx_pci_irq_vector(dev: *mut LxPciDev, nr: u32) -> i32 {
    if dev.is_null() {
        return -1;
    }
    unsafe {
        let d = &*dev;
        d.irq_vectors.get(nr as usize).copied().map(|v| v as i32).unwrap_or(-1)
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn lx_pci_set_drvdata(dev: *mut LxPciDev, data: *mut c_void) {
    if !dev.is_null() {
        unsafe { (*dev).data = data as usize; }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn lx_pci_get_drvdata(dev: *mut LxPciDev) -> *mut c_void {
    if dev.is_null() {
        return core::ptr::null_mut();
    }
    unsafe { (*dev).data as *mut c_void }
}

#[unsafe(no_mangle)]
/// `pci_dev_id()` de Linux: bus en los bits altos, device/function en los bajos.
/// GSP-RM lo quiere en `GspSystemInfo.nvDomainBusDeviceFunc`.
#[unsafe(no_mangle)]
pub extern "C" fn lx_pci_bdf(dev: *mut LxPciDev) -> u32 {
    if dev.is_null() {
        return 0;
    }
    let d = unsafe { &*dev };
    ((d.bus as u32) << 8) | ((d.device as u32) << 3) | (d.function as u32)
}

#[unsafe(no_mangle)]
pub extern "C" fn lx_pci_device_id(dev: *mut LxPciDev) -> u16 {
    if dev.is_null() {
        return 0;
    }
    unsafe { (*dev).device_id }
}
