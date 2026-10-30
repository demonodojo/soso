pub mod acpi;
pub mod acpi_hid_i2c;
pub mod apic;
pub mod cpu_hardening;
pub mod fpu;
pub mod gdt;
pub mod hwbp;
pub mod interrupts;
pub mod ioapic;
pub mod iommu;
pub mod irq;
pub mod pat;
pub mod percpu;
pub mod power;
pub mod pit;
pub mod rtc;
pub mod smp;
pub mod tsc;
pub mod sse;

pub fn init() {
    gdt::init();
    // GS de la BSP: cpu 0, con el KSTACK de siempre. Antes de esto nadie
    // puede leer/escribir `percpu::current_pid()` etc.
    percpu::init(0, gdt::kstack_top().as_u64());
    interrupts::init();
    cpu_hardening::enable_smep();
    cpu_hardening::enable_smap();
    cpu_hardening::log_line();
}
