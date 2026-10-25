//! Tramos consecutivos de sectores que difieren entre dos instantáneas del mismo tamaño.

/// `[offset, offset+len)` dentro del búfer, alineado a `sector`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tramo {
    pub offset: usize,
    pub len: usize,
}

/// Recorre cada tramo donde `anterior` y `nuevo` difieren, comparando por sectores.
///
/// `anterior` y `nuevo` deben tener la misma longitud (múltiplo de `sector`).
pub fn por_cada_tramo_cambiado(
    anterior: &[u8],
    nuevo: &[u8],
    sector: usize,
    mut f: impl FnMut(Tramo),
) {
    if sector == 0 || anterior.len() != nuevo.len() || anterior.is_empty() {
        return;
    }
    debug_assert_eq!(anterior.len() % sector, 0);
    let nsect = anterior.len() / sector;
    let mut i = 0usize;
    while i < nsect {
        let off = i * sector;
        let end = off + sector;
        if anterior[off..end] == nuevo[off..end] {
            i += 1;
            continue;
        }
        let start = off;
        while i < nsect {
            let o = i * sector;
            let e = o + sector;
            if anterior[o..e] != nuevo[o..e] {
                i += 1;
            } else {
                break;
            }
        }
        f(Tramo {
            offset: start,
            len: i * sector - start,
        });
    }
}

#[cfg(feature = "std")]
pub fn tramos_cambiados(anterior: &[u8], nuevo: &[u8], sector: usize) -> std::vec::Vec<Tramo> {
    let mut v = std::vec::Vec::new();
    por_cada_tramo_cambiado(anterior, nuevo, sector, |t| v.push(t));
    v
}
