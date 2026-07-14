pub mod gdt;
pub mod interrupts;
pub mod pit;
pub mod sse;

pub fn init() {
    gdt::init();
    interrupts::init();
}
