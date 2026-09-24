//! Watchpoints hardware (DR0–DR3) para cazar quién escribe en `bins` de talc.
//!
//! El #PF recurrente del live USB viene de que `bins[0]` (`HEAP_START+8`)
//! amanece con un `u16` pisado en sus bytes +4/+5. Ningún centinela lo ve
//! porque no es una reserva: es la tabla de cabeceras de talc. Un punto de
//! ruptura de datos sobre esos 8 bytes dispara `#DB` en la instrucción que
//! escribe, con su `rip`; si el valor cambia sin `#DB`, lo escribió DMA.
//!
//! Los registros de depuración son por CPU: la BSP fija las direcciones al
//! armar y cada AP las copia al arrancar (`armar_en_este_cpu`).

use core::arch::asm;
use core::sync::atomic::{AtomicU64, Ordering};

/// Dirección vigilada por DRi (0 = libre). Siempre 8 bytes, sólo escritura.
static VIGILADAS: [AtomicU64; 4] = [
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
];

/// Vigila escrituras sobre los 8 bytes alineados en `addr` con DR`idx`.
pub fn vigilar_escritura(idx: usize, addr: u64) {
    if idx >= VIGILADAS.len() || addr % 8 != 0 {
        return;
    }
    VIGILADAS[idx].store(addr, Ordering::SeqCst);
    armar_en_este_cpu();
}

pub fn vigilada(idx: usize) -> u64 {
    VIGILADAS.get(idx).map_or(0, |a| a.load(Ordering::Relaxed))
}

/// Carga DR0–DR3 y DR7 en la CPU actual con las direcciones fijadas.
pub fn armar_en_este_cpu() {
    // bit 10 se lee siempre a 1; LE/GE (8/9) piden reporte exacto.
    let mut dr7: u64 = (1 << 10) | (1 << 8) | (1 << 9);
    for (i, slot) in VIGILADAS.iter().enumerate() {
        let a = slot.load(Ordering::Relaxed);
        if a == 0 {
            continue;
        }
        unsafe {
            match i {
                0 => asm!("mov dr0, {}", in(reg) a, options(nostack, preserves_flags)),
                1 => asm!("mov dr1, {}", in(reg) a, options(nostack, preserves_flags)),
                2 => asm!("mov dr2, {}", in(reg) a, options(nostack, preserves_flags)),
                _ => asm!("mov dr3, {}", in(reg) a, options(nostack, preserves_flags)),
            }
        }
        dr7 |= 1 << (2 * i + 1); // Gi: global, sobrevive a cambios de tarea
        dr7 |= 0b01 << (16 + 4 * i); // RWi = 01: sólo escritura de datos
        dr7 |= 0b10 << (18 + 4 * i); // LENi = 10: 8 bytes
    }
    unsafe {
        asm!("mov dr6, {}", in(reg) 0u64, options(nostack, preserves_flags));
        asm!("mov dr7, {}", in(reg) dr7, options(nostack, preserves_flags));
    }
}

pub fn leer_dr6() -> u64 {
    let v: u64;
    unsafe { asm!("mov {}, dr6", out(reg) v, options(nostack, preserves_flags)) };
    v
}

pub fn limpiar_dr6() {
    unsafe { asm!("mov dr6, {}", in(reg) 0u64, options(nostack, preserves_flags)) };
}
