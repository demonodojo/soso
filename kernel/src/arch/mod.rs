pub mod acpi;
pub mod apic;
pub mod fpu;
pub mod gdt;
pub mod interrupts;
pub mod pit;
pub mod smp;
pub mod sse;

pub fn init() {
    gdt::init();
    interrupts::init();
}
