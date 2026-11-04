//! Hasta dónde llega lo ocupado de un volumen, leído en crudo.
//!
//! Sirve para clonar un sosofs sin copiar su cola libre: en el live el volumen
//! crece hasta llenar la partición (`grow_to`) y casi todo lo de detrás está
//! libre. Lo que haya en esos bloques da igual, porque sosofs no lee un bloque
//! libre sin haberlo escrito antes.

use crate::layout::Superblock;
use block_dev::{BLOCK_SIZE, Block};

/// Número de bloques desde el principio que hay que copiar para no perder
/// nada de lo que referencia cualquiera de los dos superbloques válidos: el
/// último bloque marcado en sus bitmaps, más uno (como mínimo los superbloques
/// y el propio bitmap).
///
/// `leer(bloque, buf)` devuelve `false` si la lectura falla. Devuelve `None`
/// si falla una lectura o no hay ningún superbloque válido.
pub fn bloques_en_uso(mut leer: impl FnMut(u64, &mut Block) -> bool) -> Option<u64> {
    let mut buf: Block = [0; BLOCK_SIZE];
    let mut fin: Option<u64> = None;
    for slot in 0..2u64 {
        if !leer(slot, &mut buf) {
            return None;
        }
        let Some(sb) = Superblock::parse(&buf) else {
            continue;
        };
        let total = sb.block_count.get();
        let inicio = sb.bitmap_start.get();
        let n = sb.bitmap_blocks.get() as u64;
        let mut hasta = 2u64.max(inicio + n);
        for i in 0..n {
            if !leer(inicio + i, &mut buf) {
                return None;
            }
            let base = i * BLOCK_SIZE as u64 * 8;
            if let Some(j) = buf.iter().rposition(|&b| b != 0) {
                let bit = 7 - buf[j].leading_zeros() as u64;
                let ultimo = base + j as u64 * 8 + bit;
                hasta = hasta.max((ultimo + 1).min(total));
            }
        }
        fin = Some(fin.map_or(hasta, |f| f.max(hasta)));
    }
    fin
}
