//! Búsqueda sin reservas ni candados para el diagnóstico de un heap dañado.

/// Índice de la primera palabra igual a `valor`.
///
/// # Safety
/// Las `n` palabras desde `base` deben ser RAM legible y estar alineadas a 8.
/// No sigue punteros almacenados en ellas. Puede observar escrituras de otros
/// cores: es una lectura de diagnóstico, no una instantánea coherente.
pub(super) unsafe fn buscar(base: *const u64, n: usize, valor: u64) -> Option<usize> {
    if n == 0 {
        return None;
    }
    let mut cursor = base;
    let igual: u8;
    // Evita millones de llamadas a read_volatile sin optimizar en kernel dev.
    unsafe {
        core::arch::asm!(
            "cld",
            "repne scasq",
            "sete {igual}",
            inout("rdi") cursor,
            inout("rcx") n => _,
            in("rax") valor,
            igual = lateout(reg_byte) igual,
            options(nostack, readonly),
        );
    }
    if igual != 0 {
        Some((cursor as usize - base as usize) / 8 - 1)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::buscar;

    #[test]
    fn limites_y_primera_coincidencia() {
        let datos = [11u64, 22, 33, 22, 44];
        for (valor, esperado) in [(11, Some(0)), (22, Some(1)), (44, Some(4)), (55, None)] {
            assert_eq!(unsafe { buscar(datos.as_ptr(), datos.len(), valor) }, esperado);
        }
        // No lee el último elemento fuera del intervalo solicitado.
        assert_eq!(unsafe { buscar(datos.as_ptr(), 4, 44) }, None);
        assert_eq!(unsafe { buscar(core::ptr::null(), 0, 0) }, None);
    }
}
