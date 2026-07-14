#![no_std]
#![no_main]
#![feature(abi_x86_interrupt)]

extern crate alloc;

mod arch;
mod drivers;
mod fs;
mod vfs;
mod kshell;
mod mm;
mod net;
mod qemu;
mod task;

use alloc::vec::Vec;
use bootloader_api::config::{BootloaderConfig, Mapping};
use bootloader_api::{BootInfo, entry_point};
use core::panic::PanicInfo;

// Necesitamos toda la memoria física mapeada para recorrer tablas de páginas.
static BOOTLOADER_CONFIG: BootloaderConfig = {
    let mut config = BootloaderConfig::new_default();
    config.mappings.physical_memory = Some(Mapping::Dynamic);
    config
};

// El bootloader entra con `push 0; jmp _start`, es decir con la pila ya en
// la convención post-`call` (rsp%16==8), así que la alineación que exige
// SSE en el camino normal la da `entry_point!` sin más. Los trampolines en
// asm que fijan rsp a mano (schedule_landing) sí deben cuidarla.
entry_point!(kernel_main, config = &BOOTLOADER_CONFIG);

fn kernel_main(boot_info: &'static mut BootInfo) -> ! {
    // Lo PRIMERO: habilitar SSE por hardware. El kernel se compila con
    // +sse2 (lo exige la cripto de sunset) y el bootloader lo deja
    // deshabilitado; cualquier instrucción XMM antes de esto falla.
    arch::sse::enable();

    println!("soso 0.1");

    arch::init();
    mm::init(boot_info);

    println!(
        "memoria: {} MiB libres tras el heap",
        mm::FRAME_ALLOC.get().unwrap().lock().free_frames() * 4096 / (1024 * 1024)
    );

    // Autotests rápidos de arranque: heap e IDT.
    let cuadrados: Vec<u64> = (1..=10).map(|n| n * n).collect();
    assert_eq!(cuadrados.last(), Some(&100));
    x86_64::instructions::interrupts::int3();

    drivers::virtio_blk::init();
    drivers::pci::init();
    drivers::gpu::init();
    fs::init();
    net::init();
    task::init();

    // Si hay un init de usuario, arranca en ring 3; si no, kernel-shell.
    let hay_init = crate::vfs::resolve("/bin/init").is_ok();
    if hay_init {
        match task::spawn("/bin/init", "", 0) {
            Ok(pid) => {
                println!("task: /bin/init lanzado (pid {pid})");
                task::schedule();
            }
            Err(e) => println!("task: fallo lanzando /bin/init (errno {e})"),
        }
    }
    kshell::run();
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    x86_64::instructions::interrupts::disable();
    println!("\n!!! panic: {info}");
    qemu::exit(qemu::ExitCode::Failed);
}
