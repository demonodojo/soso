//! PAT (Page Attribute Table): tipo de memoria **write-combining** para el
//! framebuffer GOP.
//!
//! POR QUÉ EXISTE. El bootloader mapea el framebuffer con PTEs limpios (índice
//! PAT 0 = WB) y el tipo efectivo lo decide entonces la MTRR que cubra la
//! apertura de la GPU, que en las placas es **UC**: cada store al GOP es una
//! transacción de bus serializada. En la ROG (GOP de la iGPU AMD `1002:1638`,
//! 1920×1080×4 = 8 MiB por pantalla, sin serie) un scroll costaba ~100 ms sólo
//! en volcar la sombra, y las trazas de arranque se arrastraban línea a línea.
//!
//! Con PAT = WC la CPU combina los stores en búferes de 64 bytes y los
//! escribe en ráfaga: es lo que hace Linux (`ioremap_wc`) para efifb/amdgpu,
//! y funciona sobre una MTRR UC tanto en Intel (SDM tabla 11-7) como en AMD
//! (APM 7.8.2): PAT WC + MTRR UC → WC.
//!
//! Sólo se toca la entrada **1** (PWT=1, PCD=0), que de fábrica es WT y aquí
//! nadie usa: `NO_CACHE|WRITE_THROUGH` (índice 3 = UC) y `NO_CACHE` (índice 2
//! = UC-) siguen igual. Así el bit del PTE es el mismo en páginas de 4 KiB y
//! de 2 MiB (el bit PAT cambia de sitio entre niveles; PWT no).
//!
//! Todos los cores deben llevar el mismo PAT (SDM 11.12.4): la BSP lo fija
//! antes de tocar el framebuffer y cada AP en `ap_entry`.

use core::sync::atomic::{AtomicBool, Ordering};
use x86_64::registers::control::{Cr0, Cr0Flags};
use x86_64::registers::model_specific::Msr;
use x86_64::structures::paging::PageTableFlags;

const IA32_PAT: u32 = 0x277;
/// Tipo WC en la codificación del PAT.
const PAT_WC: u64 = 0x01;
/// Entrada que pasa a WC: índice 1 → PWT=1, PCD=0, PAT=0.
const WC_ENTRY: u32 = 1;

/// Flags de PTE que seleccionan la entrada WC del PAT.
pub const WC_FLAGS: PageTableFlags = PageTableFlags::WRITE_THROUGH;

static WC_LISTO: AtomicBool = AtomicBool::new(false);

fn soportado() -> bool {
    // CPUID.01H:EDX[16] = PAT.
    let f = core::arch::x86_64::__cpuid(1);
    f.edx & (1 << 16) != 0
}

/// ¿Está programada la entrada WC? Si no, `WC_FLAGS` significaría WT.
pub fn wc_disponible() -> bool {
    WC_LISTO.load(Ordering::Relaxed)
}

/// Programa la entrada WC del PAT en **este** core. Idempotente. Llamar con
/// paginación activa, una vez por CPU (BSP en el arranque, AP en `ap_entry`).
pub fn init_cpu() {
    if !soportado() {
        return;
    }
    let mut pat = Msr::new(IA32_PAT);
    let actual = unsafe { pat.read() };
    let shift = WC_ENTRY * 8;
    let nuevo = (actual & !(0xffu64 << shift)) | (PAT_WC << shift);
    if nuevo != actual {
        // Secuencia del SDM 11.12.4 (y APM 7.8.2): sin interrupciones, caché
        // en modo no-fill, vaciar caché y TLB, escribir el MSR y volver a
        // vaciar. En el arranque de la BSP casi todo esto es redundante (la
        // entrada 1 aún no la usa ninguna página), pero es barato y es el
        // protocolo que garantizan los fabricantes.
        x86_64::instructions::interrupts::without_interrupts(|| unsafe {
            let cr0 = Cr0::read();
            let mut nofill = cr0;
            nofill.insert(Cr0Flags::CACHE_DISABLE);
            nofill.remove(Cr0Flags::NOT_WRITE_THROUGH);
            Cr0::write(nofill);
            core::arch::asm!("wbinvd", options(nostack, preserves_flags));
            x86_64::instructions::tlb::flush_all();
            pat.write(nuevo);
            x86_64::instructions::tlb::flush_all();
            core::arch::asm!("wbinvd", options(nostack, preserves_flags));
            Cr0::write(cr0);
        });
    }
    WC_LISTO.store(true, Ordering::Relaxed);
}
