//! Cuantización on-disk: Q8_0 (bloques de 32, escala f32 + 32 i8) y Q4_K
//! (layout GGML exacto: superbloques de 256, 144 bytes).

use crate::f16::f16_to_f32;
use alloc::vec::Vec;
use sosomodel::layout::{Q4_K_BLOCK_BYTES, Q4_K_BLOCK_ELEMS, Q8_0_BLOCK_BYTES, Q8_0_BLOCK_ELEMS};

/// Cuantiza a bloques Q8_0; el último bloque se rellena con ceros.
pub fn quantize_q8_0(src: &[f32]) -> Vec<u8> {
    let blocks = src.len().div_ceil(Q8_0_BLOCK_ELEMS);
    let mut out = Vec::with_capacity(blocks * Q8_0_BLOCK_BYTES);
    for chunk in src.chunks(Q8_0_BLOCK_ELEMS) {
        let mut max = 0.0f32;
        for &v in chunk {
            max = max.max(v.abs());
        }
        let scale = if max > 0.0 { max / 127.0 } else { 1.0 };
        out.extend_from_slice(&scale.to_le_bytes());
        for i in 0..Q8_0_BLOCK_ELEMS {
            let v = chunk.get(i).copied().unwrap_or(0.0);
            out.push(libm::roundf(v / scale).clamp(-127.0, 127.0) as i8 as u8);
        }
    }
    out
}

/// Descuantiza `out.len()` elementos a partir del elemento `elem_off`.
/// `bytes` es el tensor Q8_0 completo (múltiplo de bloque).
pub fn dequant_q8_0_range(bytes: &[u8], elem_off: usize, out: &mut [f32]) -> Result<(), ()> {
    if bytes.len() % Q8_0_BLOCK_BYTES != 0 || out.is_empty() {
        return Err(());
    }
    let total = (bytes.len() / Q8_0_BLOCK_BYTES) * Q8_0_BLOCK_ELEMS;
    let end = elem_off.checked_add(out.len()).ok_or(())?;
    if end > total {
        return Err(());
    }
    let first = elem_off / Q8_0_BLOCK_ELEMS;
    let last = (end - 1) / Q8_0_BLOCK_ELEMS;
    for b in first..=last {
        let chunk = &bytes[b * Q8_0_BLOCK_BYTES..(b + 1) * Q8_0_BLOCK_BYTES];
        let scale = f32::from_le_bytes(chunk[..4].try_into().map_err(|_| ())?);
        let base = b * Q8_0_BLOCK_ELEMS;
        for (j, &q) in chunk[4..].iter().enumerate() {
            let e = base + j;
            if e >= elem_off && e < end {
                out[e - elem_off] = (q as i8) as f32 * scale;
            }
        }
    }
    Ok(())
}

/// Descuantiza el tensor completo.
pub fn dequant_q8_0(bytes: &[u8], out: &mut [f32]) -> Result<(), ()> {
    dequant_q8_0_range(bytes, 0, out)
}

// ---- Q4_K (layout GGML) ----
//
// Superbloque de 256 elementos = 144 bytes:
//   d: f16 | dmin: f16 | scales[12] (8 escalas + 8 mins de 6 bits) | qs[128]
// 8 sub-bloques de 32; el elemento e del par n usa el nibble bajo (primeros
// 32) o alto (siguientes 32) de qs. valor = d·sc·q − dmin·m.

/// Escala y min de 6 bits del sub-bloque `j` (0..8) — `get_scale_min_k4`.
pub(crate) fn q4k_scale_min(scales: &[u8], j: usize) -> (u8, u8) {
    if j < 4 {
        (scales[j] & 63, scales[j + 4] & 63)
    } else {
        (
            (scales[j + 4] & 0x0F) | ((scales[j - 4] >> 6) << 4),
            (scales[j + 4] >> 4) | ((scales[j] >> 6) << 4),
        )
    }
}

/// Inverso de `q4k_scale_min`: mete ocho pares (escala, mínimo) de 6 bits en 12
/// bytes. Vivía en los tests y ahora hace falta de verdad para poder CUANTIZAR;
/// sigue cubierto por `q4k_scale_min_roundtrip`, que es lo que ata las dos mitades.
pub(crate) fn q4k_pack_scales(sc: [u8; 8], m: [u8; 8]) -> [u8; 12] {
    let mut s = [0u8; 12];
    for j in 0..4 {
        s[j] = (sc[j] & 63) | ((sc[j + 4] >> 4) << 6);
        s[j + 4] = (m[j] & 63) | ((m[j + 4] >> 4) << 6);
        s[j + 8] = (sc[j + 4] & 0x0F) | ((m[j + 4] & 0x0F) << 4);
    }
    s
}

/// Cuantiza a superbloques Q4_K (256 elementos), formato GGML exacto.
///
/// **Existe para poder PROBAR el camino Q4_K.** El descuantizador está desde L2
/// porque los modelos vienen de GGUF ya cuantizados, así que no había forma de
/// sintetizar un modelo Q4_K y el camino sólo se ejercitaba con un modelo real de
/// gigabytes — o no se ejercitaba, que es lo que pasaba con el offload a GPU.
///
/// Cada sub-bloque de 32 elementos se reconstruye como `v = (d·sc)·q - (dmin·m)`
/// con `q` de 4 bits, `sc`/`m` de 6 y `d`/`dmin` en f16 compartidos por el
/// superbloque. Dos detalles que no son evidentes:
///
///  1. **El rango de cada sub-bloque se estira hasta incluir el 0.** `dmin` es uno
///     para los ocho sub-bloques y `m` no tiene signo, así que un sub-bloque cuyo
///     mínimo fuese positivo no se podría representar con el desplazamiento que le
///     toca. Forzar `lo ≤ 0 ≤ hi` cuesta precisión y siempre es representable.
///  2. **Los `q` se calculan con la escala EFECTIVA**, la que queda después de
///     redondear `d`/`dmin` a f16 y `sc`/`m` a 6 bits — no con la ideal. Con la
///     ideal, el error de redondeo de las escalas se suma al de los `q` en vez de
///     compensarse, y el round-trip empeora visiblemente.
pub fn quantize_q4_k(src: &[f32]) -> Vec<u8> {
    let blocks = src.len().div_ceil(Q4_K_BLOCK_ELEMS);
    let mut out = Vec::with_capacity(blocks * Q4_K_BLOCK_BYTES);

    for b in 0..blocks {
        let base = b * Q4_K_BLOCK_ELEMS;
        let val = |i: usize| -> f32 { src.get(base + i).copied().unwrap_or(0.0) };

        // Paso 1: escala y desplazamiento ideales de cada sub-bloque.
        let mut s_ideal = [0.0f32; 8];
        let mut o_ideal = [0.0f32; 8];
        for j in 0..8 {
            let mut lo = 0.0f32;
            let mut hi = 0.0f32;
            for l in 0..32 {
                let v = val(j * 32 + l);
                if v < lo {
                    lo = v;
                }
                if v > hi {
                    hi = v;
                }
            }
            s_ideal[j] = (hi - lo) / 15.0;
            o_ideal[j] = -lo;
        }

        // Paso 2: d y dmin, los dos factores f16 del superbloque.
        let max_s = s_ideal.iter().fold(0.0f32, |a, &v| if v > a { v } else { a });
        let max_o = o_ideal.iter().fold(0.0f32, |a, &v| if v > a { v } else { a });
        let d_bits = crate::f16::f32_to_f16(max_s / 63.0);
        let dmin_bits = crate::f16::f32_to_f16(max_o / 63.0);
        let d = crate::f16::f16_to_f32(d_bits);
        let dmin = crate::f16::f16_to_f32(dmin_bits);

        // Paso 3: sc y m de 6 bits, y las escalas EFECTIVAS que salen de ahí.
        let mut sc = [0u8; 8];
        let mut m = [0u8; 8];
        for j in 0..8 {
            sc[j] = if d > 0.0 {
                libm::roundf(s_ideal[j] / d).clamp(0.0, 63.0) as u8
            } else {
                0
            };
            m[j] = if dmin > 0.0 {
                libm::roundf(o_ideal[j] / dmin).clamp(0.0, 63.0) as u8
            } else {
                0
            };
        }

        out.extend_from_slice(&d_bits.to_le_bytes());
        out.extend_from_slice(&dmin_bits.to_le_bytes());
        out.extend_from_slice(&q4k_pack_scales(sc, m));

        // Paso 4: los nibbles. El byte `pair*32+l` lleva el elemento `pair*64+l`
        // en la mitad baja y el `pair*64+32+l` en la alta (igual que lee el
        // descuantizador; invertirlo daría un tensor con las mitades cruzadas y
        // ningún error).
        for pair in 0..4 {
            let (s1, o1) = (d * sc[2 * pair] as f32, dmin * m[2 * pair] as f32);
            let (s2, o2) = (d * sc[2 * pair + 1] as f32, dmin * m[2 * pair + 1] as f32);
            for l in 0..32 {
                let q1 = quantize_nibble(val(pair * 64 + l), s1, o1);
                let q2 = quantize_nibble(val(pair * 64 + 32 + l), s2, o2);
                out.push(q1 | (q2 << 4));
            }
        }
    }
    out
}

/// `q = round((v + o) / s)` acotado a 4 bits. `s == 0` (sub-bloque constante) da 0,
/// que reconstruye exactamente `-o`.
fn quantize_nibble(v: f32, s: f32, o: f32) -> u8 {
    if s <= 0.0 {
        return 0;
    }
    libm::roundf((v + o) / s).clamp(0.0, 15.0) as u8
}

/// Descuantiza `out.len()` elementos Q4_K a partir del elemento `elem_off`.
pub fn dequant_q4_k_range(bytes: &[u8], elem_off: usize, out: &mut [f32]) -> Result<(), ()> {
    if bytes.len() % Q4_K_BLOCK_BYTES != 0 || out.is_empty() {
        return Err(());
    }
    let total = (bytes.len() / Q4_K_BLOCK_BYTES) * Q4_K_BLOCK_ELEMS;
    let end = elem_off.checked_add(out.len()).ok_or(())?;
    if end > total {
        return Err(());
    }
    let first = elem_off / Q4_K_BLOCK_ELEMS;
    let last = (end - 1) / Q4_K_BLOCK_ELEMS;
    for b in first..=last {
        let blk = &bytes[b * Q4_K_BLOCK_BYTES..(b + 1) * Q4_K_BLOCK_BYTES];
        let d = f16_to_f32(u16::from_le_bytes([blk[0], blk[1]]));
        let dmin = f16_to_f32(u16::from_le_bytes([blk[2], blk[3]]));
        let scales = &blk[4..16];
        let qs = &blk[16..144];
        for pair in 0..4 {
            let (sc1, m1) = q4k_scale_min(scales, 2 * pair);
            let (sc2, m2) = q4k_scale_min(scales, 2 * pair + 1);
            let (d1, min1) = (d * sc1 as f32, dmin * m1 as f32);
            let (d2, min2) = (d * sc2 as f32, dmin * m2 as f32);
            let base = b * Q4_K_BLOCK_ELEMS + pair * 64;
            for l in 0..32 {
                let q = qs[pair * 32 + l];
                let e1 = base + l;
                let e2 = base + 32 + l;
                if e1 >= elem_off && e1 < end {
                    out[e1 - elem_off] = d1 * (q & 0x0F) as f32 - min1;
                }
                if e2 >= elem_off && e2 < end {
                    out[e2 - elem_off] = d2 * (q >> 4) as f32 - min2;
                }
            }
        }
    }
    Ok(())
}

/// Descuantiza el tensor Q4_K completo.
pub fn dequant_q4_k(bytes: &[u8], out: &mut [f32]) -> Result<(), ()> {
    dequant_q4_k_range(bytes, 0, out)
}

#[cfg(test)]
#[cfg(feature = "std")]
mod tests {
    use super::*;

    #[test]
    fn q8_roundtrip_aproximado() {
        let src: Vec<f32> = (0..64).map(|i| (i as f32 - 32.0) * 0.1).collect();
        let packed = quantize_q8_0(&src);
        let mut out = vec![0.0f32; 64];
        dequant_q8_0(&packed, &mut out).unwrap();
        for (a, b) in src.iter().zip(&out) {
            assert!((a - b).abs() < 0.05, "{a} vs {b}");
        }
    }

    #[test]
    fn matvec_q8_fusionado_coincide_con_referencia() {
        use crate::gemm::{matvec_f32, matvec_q8_0};
        let rows = 8;
        let cols = 64;
        let w: Vec<f32> = (0..rows * cols).map(|i| ((i % 37) as f32 - 18.0) * 0.1).collect();
        let x: Vec<f32> = (0..cols).map(|i| (i as f32 - 32.0) * 0.05).collect();
        // referencia: dequant a f32 y matvec normal
        let packed = quantize_q8_0(&w);
        let mut wq = vec![0.0f32; rows * cols];
        dequant_q8_0(&packed, &mut wq).unwrap();
        let mut expected = vec![0.0f32; rows];
        matvec_f32(&wq, rows, cols, &x, &mut expected);
        // fusionado sobre los bytes
        let mut out = vec![0.0f32; rows];
        matvec_q8_0(&packed, rows, cols, &x, &mut out).unwrap();
        for (a, b) in expected.iter().zip(&out) {
            assert!((a - b).abs() < 1e-4, "{a} vs {b}");
        }
    }

    /// El empaquetado ya no es del test: lo usa `quantize_q4_k`.
    fn pack_scales(sc: [u8; 8], m: [u8; 8]) -> [u8; 12] {
        q4k_pack_scales(sc, m)
    }

    fn q4k_block_test() -> (Vec<u8>, [u8; 8], [u8; 8], f32, f32) {
        use crate::f16::{f16_to_f32, f32_to_f16};
        let d_bits = f32_to_f16(0.5);
        let dmin_bits = f32_to_f16(0.25);
        let sc = [1u8, 2, 3, 4, 33, 34, 35, 63];
        let m = [0u8, 1, 2, 3, 48, 49, 50, 62];
        let mut blk = Vec::with_capacity(144);
        blk.extend_from_slice(&d_bits.to_le_bytes());
        blk.extend_from_slice(&dmin_bits.to_le_bytes());
        blk.extend_from_slice(&pack_scales(sc, m));
        for i in 0..128u32 {
            // nibble bajo = i%16, nibble alto = (i*7)%16
            blk.push(((i % 16) | (((i * 7) % 16) << 4)) as u8);
        }
        (blk, sc, m, f16_to_f32(d_bits), f16_to_f32(dmin_bits))
    }

    /// Round-trip Q4_K: cuantizar y descuantizar tiene que devolver algo cercano.
    ///
    /// La tolerancia no es un número bonito: con 4 bits por elemento y el rango
    /// estirado hasta el 0, el error máximo esperable es ~rango/15 por sub-bloque,
    /// y se comprueba CONTRA ESO en vez de contra una constante global — así el test
    /// sigue siendo estricto si alguien cambia los datos de prueba.
    #[test]
    fn q4k_roundtrip_aproximado() {
        // Dos superbloques con sub-bloques de rangos distintos —pero dentro de lo
        // que el formato puede representar, ver `q4k_rango_dinamico_del_formato`—,
        // incluidos uno todo-positivo, uno todo-negativo y uno constante, que son
        // los tres casos donde el desplazamiento (`m`) se comporta distinto.
        let src: Vec<f32> = (0..512)
            .map(|i| {
                let sub = (i / 32) % 8;
                let t = (i % 32) as f32 / 31.0;
                match sub {
                    0 => t * 20.0 - 10.0,     // rango ancho centrado en 0
                    1 => t * 1.5 + 0.5,       // todo positivo (fuerza lo=0)
                    2 => -t * 2.0,            // todo negativo
                    3 => 7.0,                 // constante
                    _ => (i as f32 % 13.0) - 6.0,
                }
            })
            .collect();
        let packed = quantize_q4_k(&src);
        assert_eq!(packed.len(), 2 * Q4_K_BLOCK_BYTES);
        let mut out = vec![0.0f32; src.len()];
        dequant_q4_k(&packed, &mut out).unwrap();

        for sb in 0..src.len() / 32 {
            let ini = sb * 32;
            let trozo = &src[ini..ini + 32];
            let lo = trozo.iter().fold(0.0f32, |a, &v| a.min(v));
            let hi = trozo.iter().fold(0.0f32, |a, &v| a.max(v));
            let paso = (hi - lo) / 15.0;
            for l in 0..32 {
                let (a, b) = (src[ini + l], out[ini + l]);
                // Un paso de cuantización más margen para el redondeo de las
                // escalas a f16/6 bits.
                let tope = paso * 1.5 + (hi - lo) * 0.02 + 1e-6;
                assert!(
                    (a - b).abs() <= tope,
                    "sub-bloque {sb} elem {l}: {a} vs {b} (tope {tope})"
                );
            }
        }
    }

    /// El **límite del formato**, no del cuantizador: `d` es uno por superbloque y
    /// las escalas por sub-bloque son de 6 bits, así que el rango dinámico útil
    /// entre sub-bloques de un mismo superbloque es ~63:1. Un sub-bloque 10000×
    /// más pequeño que el mayor se queda plano, y eso le pasa igual a GGML.
    ///
    /// Está escrito como test para que quien vea resultados raros con datos así
    /// encuentre la explicación en vez de buscar un bug que no existe.
    #[test]
    fn q4k_rango_dinamico_del_formato() {
        let mut src = vec![0.0f32; 256];
        for l in 0..32 {
            src[l] = (l as f32 / 31.0) * 100.0 - 50.0; // sub-bloque 0: rango 100
            src[32 + l] = (l as f32 / 31.0) * 0.01; // sub-bloque 1: rango 0.01
        }
        let packed = quantize_q4_k(&src);
        let mut out = vec![0.0f32; 256];
        dequant_q4_k(&packed, &mut out).unwrap();

        // El grande se representa bien...
        let paso = 100.0 / 15.0;
        for l in 0..32 {
            assert!(
                (src[l] - out[l]).abs() <= paso * 1.5,
                "el sub-bloque grande tendría que salir bien: {} vs {}",
                src[l],
                out[l]
            );
        }
        // ...y el diminuto se aplana. Si algún día esto deja de cumplirse será
        // porque alguien mejoró la elección de escalas, y entonces hay que
        // actualizar este test a conciencia, no borrarlo.
        let plano = out[32..64].iter().all(|&v| v == out[32]);
        assert!(
            plano,
            "el sub-bloque diminuto sale con detalle inesperado: {:?}",
            &out[32..40]
        );
    }

    /// El matvec fusionado sobre los bytes Q4_K coincide con descuantizar y
    /// multiplicar. Con Q8_0 esto ya existía; con Q4_K no se podía escribir porque
    /// no había cuantizador, y es justo el kernel que usa el modelo real.
    #[test]
    fn matvec_q4_k_fusionado_coincide_con_referencia() {
        use crate::gemm::{matvec_f32, matvec_q4_k};
        let rows = 4;
        let cols = 256;
        let w: Vec<f32> = (0..rows * cols)
            .map(|i| ((i % 71) as f32 - 35.0) * 0.03)
            .collect();
        let x: Vec<f32> = (0..cols).map(|i| ((i % 17) as f32 - 8.0) * 0.05).collect();

        let packed = quantize_q4_k(&w);
        let mut wq = vec![0.0f32; rows * cols];
        dequant_q4_k(&packed, &mut wq).unwrap();
        let mut esperado = vec![0.0f32; rows];
        matvec_f32(&wq, rows, cols, &x, &mut esperado);

        let mut out = vec![0.0f32; rows];
        matvec_q4_k(&packed, rows, cols, &x, &mut out).unwrap();
        for (a, b) in esperado.iter().zip(&out) {
            assert!((a - b).abs() < 1e-3, "{a} vs {b}");
        }
    }

    #[test]
    fn q4k_scale_min_roundtrip() {
        let sc = [1u8, 2, 3, 4, 33, 34, 35, 63];
        let m = [0u8, 1, 2, 3, 48, 49, 50, 62];
        let packed = pack_scales(sc, m);
        for j in 0..8 {
            assert_eq!(q4k_scale_min(&packed, j), (sc[j], m[j]), "sub-bloque {j}");
        }
    }

    #[test]
    fn q4k_dequant_posiciones_conocidas() {
        let (blk, sc, m, d, dmin) = q4k_block_test();
        let mut out = vec![0.0f32; 256];
        dequant_q4_k(&blk, &mut out).unwrap();
        // elemento 0: par 0, nibble bajo de qs[0] (=0), sub-bloque 0
        assert!((out[0] - (d * sc[0] as f32 * 0.0 - dmin * m[0] as f32)).abs() < 1e-4);
        // elemento 5: qs[5]&0xF = 5
        assert!((out[5] - (d * sc[0] as f32 * 5.0 - dmin * m[0] as f32)).abs() < 1e-4);
        // elemento 32: par 0, nibble ALTO de qs[0] = (0*7)%16 = 0, sub-bloque 1
        assert!((out[32] - (d * sc[1] as f32 * 0.0 - dmin * m[1] as f32)).abs() < 1e-4);
        // elemento 64: par 1, nibble bajo de qs[32] = 32%16 = 0, sub-bloque 2
        assert!((out[64] - (d * sc[2] as f32 * 0.0 - dmin * m[2] as f32)).abs() < 1e-4);
        // elemento 65: qs[33]&0xF = 33%16 = 1
        assert!((out[65] - (d * sc[2] as f32 * 1.0 - dmin * m[2] as f32)).abs() < 1e-4);
        // elemento 255: par 3, nibble alto de qs[127] = (127*7)%16 = 9, sub-bloque 7
        assert!((out[255] - (d * sc[7] as f32 * 9.0 - dmin * m[7] as f32)).abs() < 1e-4);
    }

    #[test]
    fn matvec_q4k_coincide_con_referencia() {
        use crate::gemm::{matvec_f32, matvec_q4_k};
        let (blk, ..) = q4k_block_test();
        // 2 filas: la segunda con los nibbles "girados"
        let mut blk2 = blk.clone();
        for b in blk2[16..].iter_mut() {
            *b = b.rotate_left(4);
        }
        let bytes: Vec<u8> = [blk.clone(), blk2].concat();
        let x: Vec<f32> = (0..256).map(|i| (i as f32 - 128.0) * 0.01).collect();
        let mut wq = vec![0.0f32; 512];
        dequant_q4_k(&bytes, &mut wq).unwrap();
        let mut expected = vec![0.0f32; 2];
        matvec_f32(&wq, 2, 256, &x, &mut expected);
        let mut out = vec![0.0f32; 2];
        matvec_q4_k(&bytes, 2, 256, &x, &mut out).unwrap();
        for (a, b) in expected.iter().zip(&out) {
            assert!((a - b).abs() < 1e-2, "{a} vs {b}");
        }
    }

    #[test]
    fn q4k_rango_parcial() {
        let (blk, ..) = q4k_block_test();
        let mut full = vec![0.0f32; 256];
        dequant_q4_k(&blk, &mut full).unwrap();
        let mut part = vec![0.0f32; 100];
        dequant_q4_k_range(&blk, 77, &mut part).unwrap();
        for (i, v) in part.iter().enumerate() {
            assert!((v - full[77 + i]).abs() < 1e-6);
        }
    }

    #[test]
    fn q8_rango_parcial() {
        let src: Vec<f32> = (0..96).map(|i| i as f32).collect();
        let packed = quantize_q8_0(&src);
        let mut out = vec![0.0f32; 40];
        dequant_q8_0_range(&packed, 20, &mut out).unwrap();
        for (i, v) in out.iter().enumerate() {
            assert!((v - src[20 + i]).abs() < 0.5);
        }
    }
}
