//! Planificación de descarga parcial: qué tramos del `rootfs.pack` hay que pedir.
//!
//! El pack es una concatenación de ficheros y el manifest da `offset`/`size` de
//! cada uno, así que basta con pedir por HTTP `Range` los tramos que cubren los
//! ficheros que han cambiado. Como cada petición abre una conexión TLS nueva
//! (`Connection: close`), agrupamos entradas próximas en un solo tramo: bajar
//! unos KB de relleno sale más barato que otro handshake.

use alloc::vec::Vec;

use crate::manifest::FileEntry;

/// Relleno máximo que aceptamos tragar para fundir dos entradas en un tramo.
pub const GAP_MAX: u64 = 256 * 1024;
/// Tamaño máximo de un tramo que se descarga de una vez en RAM.
pub const SPAN_MAX: u64 = 8 * 1024 * 1024;

/// Un tramo contiguo `[start, end)` del pack y los índices (sobre el slice de
/// pendientes que se pasó a [`plan_spans`]) de los ficheros que contiene.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span {
    pub start: u64,
    pub end: u64,
    pub files: Vec<usize>,
}

impl Span {
    pub fn len(&self) -> u64 {
        self.end - self.start
    }

    pub fn is_empty(&self) -> bool {
        self.end == self.start
    }

    /// ¿Hay que trocear este tramo por no caber en RAM? Sólo puede pasar con un
    /// fichero suelto más grande que `span_max`.
    pub fn oversized(&self, span_max: u64) -> bool {
        self.len() > span_max
    }
}

/// Agrupa las entradas pendientes en tramos de descarga.
///
/// `pending` no necesita venir ordenado; los índices devueltos se refieren a
/// las posiciones originales.
pub fn plan_spans(pending: &[FileEntry], gap_max: u64, span_max: u64) -> Vec<Span> {
    let mut idx: Vec<usize> = (0..pending.len()).collect();
    idx.sort_by_key(|&i| (pending[i].offset, pending[i].size));

    let mut spans: Vec<Span> = Vec::new();
    for i in idx {
        let e = &pending[i];
        let (start, end) = (e.offset, e.offset + e.size);
        match spans.last_mut() {
            // Extiende el tramo abierto si el hueco es pequeño y el resultado
            // sigue cabiendo. Un tramo que ya se pasa de `span_max` (fichero
            // gigante suelto) nunca admite compañía.
            Some(last)
                if start >= last.end
                    && start - last.end <= gap_max
                    && end.saturating_sub(last.start) <= span_max =>
            {
                last.end = end.max(last.end);
                last.files.push(i);
            }
            // Solapado o contenido (no debería pasar en un pack bien formado,
            // pero no queremos pedir dos veces los mismos bytes).
            Some(last) if start < last.end && end <= last.end => {
                last.files.push(i);
            }
            _ => spans.push(Span {
                start,
                end,
                files: alloc::vec![i],
            }),
        }
    }
    spans
}

/// Bytes que habría que descargar con este plan (incluye el relleno de los huecos).
pub fn plan_bytes(spans: &[Span]) -> u64 {
    spans.iter().map(|s| s.len()).sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::String;
    use alloc::string::ToString;

    fn e(offset: u64, size: u64) -> FileEntry {
        FileEntry {
            path: String::from("x"),
            offset,
            size,
            hash_hex: "0".to_string(),
        }
    }

    #[test]
    fn vacio_no_planifica_nada() {
        assert!(plan_spans(&[], GAP_MAX, SPAN_MAX).is_empty());
    }

    #[test]
    fn entradas_contiguas_se_funden() {
        let p = [e(0, 100), e(100, 100), e(200, 50)];
        let spans = plan_spans(&p, GAP_MAX, SPAN_MAX);
        assert_eq!(spans.len(), 1);
        assert_eq!((spans[0].start, spans[0].end), (0, 250));
        assert_eq!(spans[0].files, alloc::vec![0, 1, 2]);
    }

    #[test]
    fn hueco_pequeno_se_traga_y_grande_corta() {
        let p = [e(0, 10), e(1000, 10)];
        assert_eq!(plan_spans(&p, GAP_MAX, SPAN_MAX).len(), 1);
        assert_eq!(plan_spans(&p, 100, SPAN_MAX).len(), 2);
    }

    #[test]
    fn no_supera_span_max() {
        let p = [e(0, 6_000_000), e(6_000_000, 6_000_000)];
        let spans = plan_spans(&p, GAP_MAX, SPAN_MAX);
        assert_eq!(spans.len(), 2);
        assert!(spans.iter().all(|s| s.len() <= SPAN_MAX));
    }

    #[test]
    fn fichero_gigante_va_solo_y_se_marca() {
        let p = [e(0, 64 * 1024 * 1024), e(64 * 1024 * 1024, 10)];
        let spans = plan_spans(&p, GAP_MAX, SPAN_MAX);
        assert_eq!(spans.len(), 2);
        assert!(spans[0].oversized(SPAN_MAX));
        assert_eq!(spans[0].files, alloc::vec![0]);
        assert!(!spans[1].oversized(SPAN_MAX));
    }

    #[test]
    fn desordenado_planifica_igual() {
        let p = [e(200, 50), e(0, 100), e(100, 100)];
        let spans = plan_spans(&p, GAP_MAX, SPAN_MAX);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].files, alloc::vec![1, 2, 0]);
    }

    #[test]
    fn cuenta_bytes_con_relleno() {
        let p = [e(0, 10), e(1000, 10)];
        // un solo tramo 0..1010 => 1010 bytes, relleno incluido
        assert_eq!(plan_bytes(&plan_spans(&p, GAP_MAX, SPAN_MAX)), 1010);
        assert_eq!(plan_bytes(&plan_spans(&p, 100, SPAN_MAX)), 20);
    }
}
