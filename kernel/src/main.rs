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

#[cfg(feature = "lxdde")]
mod lxdde;

use alloc::vec::Vec;
use bootloader_api::config::{BootloaderConfig, Mapping};
use bootloader_api::info::FrameBufferInfo;
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

    // Copiar RSDP y framebuffer antes de que mm::init tome boot_info.
    let rsdp = match boot_info.rsdp_addr {
        bootloader_api::info::Optional::Some(r) => Some(r),
        _ => None,
    };
    let fb: Option<(u64, FrameBufferInfo)> = match &mut boot_info.framebuffer {
        bootloader_api::info::Optional::Some(fb) => {
            let info = fb.info();
            let start = fb.buffer_mut().as_mut_ptr() as u64;
            Some((start, info))
        }
        _ => None,
    };

    arch::init();
    mm::init(boot_info);

    if let Some((start, info)) = fb {
        drivers::fb::init(start, info);
        if let Some((w, h, stride, bpp)) = drivers::fb::info_log() {
            println!("fb: {w}x{h} stride={stride} bpp={bpp}");
        }
    }

    println!(
        "memoria: {} MiB libres tras el heap",
        mm::FRAME_ALLOC.get().unwrap().lock().free_frames() * 4096 / (1024 * 1024)
    );

    mm::memtest::run();

    // Autotests rápidos de arranque: heap e IDT.
    let cuadrados: Vec<u64> = (1..=10).map(|n| n * n).collect();
    assert_eq!(cuadrados.last(), Some(&100));
    x86_64::instructions::interrupts::int3();

    // ACPI (MADT/MCFG) → IOAPIC → APs. Sin RSDP: monocore + ECAM fallback.
    match rsdp {
        Some(r) => {
            arch::acpi::init(r);
            arch::ioapic::init();
            arch::smp::init(r);
        }
        None => println!("smp: sin RSDP del bootloader; monocore"),
    }

    drivers::pci::init_ecam();
    drivers::pci::init();
    drivers::nvme::init();
    drivers::virtio_blk::init();
    drivers::usb_storage::init();
    drivers::live_disk::init();
    #[cfg(feature = "lxdde")]
    {
        let mode = lxdde_mode();
        if mode != lxdde::LxddeMode::Off {
            lxdde::init(mode);
        }
        if mode != lxdde::LxddeMode::E1000e {
            let _ = drivers::e1000e::init();
        }
        if mode == lxdde::LxddeMode::Nouveau {
            drivers::nvidia_probe::init();
        }
    }
    #[cfg(not(feature = "lxdde"))]
    let _ = drivers::e1000e::init();
    drivers::gpu::init();
    drivers::nvidia_probe::init();
    drivers::nvidia_compute::init();
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

#[cfg(feature = "lxdde")]
fn lxdde_mode() -> lxdde::LxddeMode {
    match option_env!("SOSO_LXDDE_MODE").unwrap_or("") {
        "spike" => lxdde::LxddeMode::Spike,
        "testdrv" => lxdde::LxddeMode::TestDrv,
        "e1000e" => lxdde::LxddeMode::E1000e,
        "nouveau" => lxdde::LxddeMode::Nouveau,
        _ => lxdde::LxddeMode::Off,
    }
}

extern "sysv64" fn panic_print_shim(info: u64, _b: u64) -> u64 {
    let info = unsafe { &*(info as *const PanicInfo<'_>) };
    println!("\n!!! panic: {info}");
    0
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    use core::sync::atomic::{AtomicBool, Ordering};
    static EN_PANICO: AtomicBool = AtomicBool::new(false);
    x86_64::instructions::interrupts::disable();
    // Panic anidado (p. ej. fallo formateando el mensaje): salir sin imprimir
    // para no girar sobre un lock ya tomado.
    if EN_PANICO.swap(true, Ordering::SeqCst) {
        qemu::exit(qemu::ExitCode::Failed);
    }
    // El lock de la serie puede estar tomado por el contexto interrumpido.
    unsafe { drivers::serial::SERIAL1.force_unlock() };
    // Imprimir con rsp realineado: un panic desde un handler x86-interrupt
    // con código de error llega con rsp%16==8 y el fmt puede hacer movaps.
    arch::interrupts::con_rsp_alineado(
        panic_print_shim,
        info as *const PanicInfo<'_> as u64,
        0,
    );
    qemu::exit(qemu::ExitCode::Failed);
}
