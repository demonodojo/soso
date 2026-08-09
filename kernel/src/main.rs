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
        if let Some((w, h, mapped_h, stride, bpp, scale)) = drivers::fb::info_log() {
            println!("fb: {w}x{h} mapped_h={mapped_h} stride={stride} bpp={bpp} scale={scale}");
        }
    }

    // En placa sin COM1 la consola es sólo el framebuffer y el teclado PS/2.
    if drivers::serial::present() {
        println!("serial: COM1 0x3F8 presente");
    } else {
        println!("serial: COM1 0x3F8 ausente (consola = framebuffer + PS/2)");
    }

    println!(
        "memoria: {} MiB libres tras el heap",
        mm::FRAME_ALLOC.get().unwrap().lock().free_frames() * 4096 / (1024 * 1024)
    );

    // Checkpoints «boot:»: en hardware real sin serie, la pantalla es la única
    // traza; el último marcador visible acota dónde se colgó el arranque.
    println!("boot: memtest");
    mm::memtest::run();

    // Autotests rápidos de arranque: heap e IDT.
    let cuadrados: Vec<u64> = (1..=10).map(|n| n * n).collect();
    assert_eq!(cuadrados.last(), Some(&100));
    x86_64::instructions::interrupts::int3();

    // ACPI (MADT/MCFG) → IOAPIC → APs. Sin RSDP: monocore + ECAM fallback.
    println!("boot: acpi/smp");
    match rsdp {
        Some(r) => {
            arch::acpi::init(r);
            arch::ioapic::init();
            arch::smp::init(r);
        }
        None => println!("smp: sin RSDP del bootloader; monocore"),
    }

    // Reloj fino, antes de que nadie mida o espere. El tick va a 100 Hz, así que
    // sin esto la unidad más pequeña de tiempo del kernel son 10 ms: los bucles
    // de espera de la GPU pagaban un tick entero por lanzamiento y el matvec
    // residente salía a 137 ms/capa (2026-08-02).
    println!("boot: tsc");
    arch::tsc::calibrate();

    println!("boot: pci");
    drivers::pci::init_ecam();
    drivers::pci::init();
    #[cfg(feature = "drv-nvme")]
    {
        println!("boot: nvme");
        drivers::nvme::init();
    }
    #[cfg(feature = "drv-virtio-blk")]
    {
        println!("boot: virtio-blk");
        drivers::virtio_blk::init();
    }
    #[cfg(feature = "drv-usb")]
    {
        println!("boot: usb");
        drivers::usb_storage::init();
    }
    #[cfg(feature = "drv-live-disk")]
    {
        println!("boot: live-disk");
        drivers::live_disk::init();
        drivers::fatlog::init();
        drivers::drvlog::init();
    }
    println!("boot: kbd");
    drivers::kbd::init();
    // Antes que lxdde: el bring-up GSP pide sus blobs por VFS
    // (`lx_request_firmware` → `/lib/firmware/…`) y sin montar falla en fw_loading.
    println!("boot: fs");
    fs::init();
    println!("boot: ethernet");
    #[cfg(feature = "lxdde")]
    {
        let mode = lxdde_mode();
        if mode != lxdde::LxddeMode::Off {
            lxdde::init(mode);
        }
        if mode == lxdde::LxddeMode::Nouveau {
            drivers::nvidia_probe::init();
        }
        if mode != lxdde::LxddeMode::E1000e && mode != lxdde::LxddeMode::Iwlwifi {
            #[cfg(feature = "drv-e1000e")]
            let _ = drivers::e1000e::init();
        }
    }
    #[cfg(all(not(feature = "lxdde"), feature = "drv-e1000e"))]
    let _ = drivers::e1000e::init();
    #[cfg(feature = "drv-gpu-nvidia")]
    {
        println!("boot: gpu");
        drivers::gpu::init();
        drivers::nvidia_probe::init();
        drivers::nvidia_compute::init();
    }
    println!("boot: red");
    net::init();
    #[cfg(feature = "lxdde")]
    if lxdde_mode() == lxdde::LxddeMode::Iwlwifi {
        let rc = net::wifi_wpa::autoconnect_from_config();
        if rc == 0 {
            println!("wifi: conectado desde /etc/wifi.conf");
        }
    }
    // Autodescubrimiento: informe parseable en serie; en live también en ESP.
    if drivers::registry::missing_drivers() {
        drivers::registry::print_hwscan();
        #[cfg(feature = "drv-live-disk")]
        let _ = drivers::drvlog::flush();
    }
    println!("boot: task");
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
    } else {
        println!("init: sin /bin/init — ¿live sin montar? (busca «live:» / «usb:» arriba)");
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
        "iwlwifi" => lxdde::LxddeMode::Iwlwifi,
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
    // Los locks de consola pueden estar tomados por el contexto interrumpido;
    // sin soltarlos el panic no llegaría ni a la serie ni a la pantalla.
    unsafe { drivers::serial::SERIAL1.force_unlock() };
    unsafe { drivers::fb::force_unlock() };
    unsafe { drivers::logbuf::force_unlock() };
    // Imprimir con rsp realineado: un panic desde un handler x86-interrupt
    // con código de error llega con rsp%16==8 y el fmt puede hacer movaps.
    arch::interrupts::con_rsp_alineado(
        panic_print_shim,
        info as *const PanicInfo<'_> as u64,
        0,
    );
    #[cfg(feature = "drv-live-disk")]
    let _ = drivers::fatlog::flush();
    qemu::exit(qemu::ExitCode::Failed);
}
