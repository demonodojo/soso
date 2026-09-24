//! Lógica host de `next` en hueco libre (misma región que el kernel).

pub fn next_en_rango(next: u64, inicio: u64, fin: u64) -> bool {
    next == 0 || (next >= inicio && next < fin)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_nulo_o_dentro_del_heap() {
        let inicio = 0x4444_4444_0000;
        let fin = inicio + 512 * 1024 * 1024;
        assert!(next_en_rango(0, inicio, fin));
        assert!(next_en_rango(inicio + 0x1000, inicio, fin));
        assert!(!next_en_rango(0x5500_0000_0000, inicio, fin));
    }
}
