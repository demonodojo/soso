//! Paralelismo por filas para matvec (decode memory-bound).

/// Reparte el rango `[0, rows)` entre workers. La implementación secuencial
/// ejecuta `f(0, rows)` en el hilo llamante; un pool de hilos reparte
/// franjas y sincroniza con una barrera.
pub trait RowParallel: Sync {
    fn for_rows(&self, rows: usize, f: &(dyn Fn(usize, usize) + Sync));
}

/// Un solo hilo: todo el rango.
pub struct Sequential;

impl RowParallel for Sequential {
    fn for_rows(&self, rows: usize, f: &(dyn Fn(usize, usize) + Sync)) {
        if rows > 0 {
            f(0, rows);
        }
    }
}

/// Franja `[start, end)` del worker `i` de `n` (reparto equilibrado).
pub fn row_strip(rows: usize, i: usize, n: usize) -> (usize, usize) {
    if n == 0 || i >= n {
        return (0, 0);
    }
    let base = rows / n;
    let rem = rows % n;
    let start = i * base + core::cmp::min(i, rem);
    let len = base + if i < rem { 1 } else { 0 };
    (start, start + len)
}
