//! Spike G1: detección NVIDIA por PCI y lectura NV_PMC_BOOT_0 (chipset id).

use crate::drivers::pci;
use crate::mm;
use spin::Once;

const VENDOR_NVIDIA: u16 = 0x10de;
#[allow(dead_code)]
const NV_PMC_BOOT_0: u64 = 0x0000;
const PCI_CMD_MEMORY: u16 = 0x2;
const PCI_CMD_MASTER: u16 = 0x4;

static NVIDIA_CHIPSET: Once<Option<u32>> = Once::new();
static NVIDIA_DEVICE: Once<Option<u16>> = Once::new();

fn pci_enable_mem_master(gpu: &pci::PciDevice) {
    let cmd = pci::read16(gpu.bus, gpu.device, gpu.function, 0x04);
    let want = cmd | PCI_CMD_MEMORY | PCI_CMD_MASTER;
    if want != cmd {
        pci::write16(gpu.bus, gpu.device, gpu.function, 0x04, want);
    }
}

pub fn init() {
    let devs = pci::enumerate();
    let Some(gpu) = devs.iter().find(|d| d.vendor_id == VENDOR_NVIDIA && d.class == 0x03)
    else {
        crate::println!("nvidia: sin GPU NVIDIA en PCI");
        NVIDIA_CHIPSET.call_once(|| None);
        NVIDIA_DEVICE.call_once(|| None);
        return;
    };
    if gpu.bar0 == 0 || gpu.bar0_size == 0 {
        crate::println!(
            "nvidia: GPU {:04x}:{:04x} bus {:02x}:{:02x}.{} sin BAR0",
            gpu.vendor_id,
            gpu.device_id,
            gpu.bus,
            gpu.device,
            gpu.function
        );
        NVIDIA_CHIPSET.call_once(|| None);
        NVIDIA_DEVICE.call_once(|| None);
        return;
    }
    pci_enable_mem_master(gpu);
    mm::ensure_mmio_mapped(gpu.bar0, gpu.bar0_size.min(16 * 1024 * 1024));
    let boot0 = unsafe { mm::phys_to_virt(gpu.bar0).as_ptr::<u32>().read_volatile() };
    if boot0 == 0xffffffff {
        crate::println!(
            "nvidia: GPU {:04x}:{:04x} NV_PMC_BOOT_0=0xffffffff (fuera del bus / sin D0)",
            gpu.vendor_id,
            gpu.device_id
        );
    } else {
        crate::println!(
            "nvidia: GPU {:04x}:{:04x} NV_PMC_BOOT_0=0x{:08x}",
            gpu.vendor_id,
            gpu.device_id,
            boot0
        );
    }
    #[cfg(feature = "lxdde")]
    crate::lxdde::notify_boot0(boot0, gpu.device_id);
    NVIDIA_CHIPSET.call_once(|| Some(boot0));
    NVIDIA_DEVICE.call_once(|| Some(gpu.device_id));
}

pub fn chipset_id() -> Option<u32> {
    *NVIDIA_CHIPSET.get().unwrap_or(&None)
}

#[allow(dead_code)]
pub fn device_id() -> Option<u16> {
    *NVIDIA_DEVICE.get().unwrap_or(&None)
}

pub fn present() -> bool {
    chipset_id().is_some()
}
