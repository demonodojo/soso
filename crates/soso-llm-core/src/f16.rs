//! Conversión f16 (IEEE 754 binary16) ↔ f32, sin hardware F16C.
//! Usada por el KV cache en f16 y por la importación de GGUF.

/// f16 → f32. La implementación vive en `sosomodel::dequant` porque el **kernel**
/// también la necesita (el dispositivo software descuantiza para `MATVQ`) y no
/// enlaza este crate. Reexportada aquí para no tocar sus llamantes.
pub use sosomodel::dequant::f16_to_f32;

/// Redondeo al más cercano (empates hacia arriba, suficiente para KV cache).
pub fn f32_to_f16(v: f32) -> u16 {
    let bits = v.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exp = ((bits >> 23) & 0xff) as i32;
    let frac = bits & 0x7f_ffff;
    if exp == 255 {
        // inf / NaN
        return sign | 0x7c00 | if frac != 0 { 0x200 } else { 0 };
    }
    let e = exp - 127 + 15;
    if e >= 31 {
        return sign | 0x7c00; // desborda → inf
    }
    if e <= 0 {
        if e < -10 {
            return sign; // demasiado pequeño → ±0
        }
        // subnormal f16
        let frac = frac | 0x80_0000;
        let shift = (14 - e) as u32;
        let sub = (frac >> shift) as u16;
        let round = ((frac >> (shift - 1)) & 1) as u16;
        return sign | (sub + round);
    }
    let half = ((e as u32) << 10) | (frac >> 13);
    let round = ((frac >> 12) & 1) as u16;
    (sign | half as u16).wrapping_add(round)
}

#[cfg(test)]
#[cfg(feature = "std")]
mod tests {
    use super::*;

    #[test]
    fn casos_basicos() {
        assert_eq!(f16_to_f32(0x3c00), 1.0);
        assert_eq!(f16_to_f32(0xbc00), -1.0);
        assert_eq!(f16_to_f32(0x3800), 0.5);
        assert_eq!(f16_to_f32(0), 0.0);
        assert!(f16_to_f32(0x7c00).is_infinite());
        assert!(f16_to_f32(0x7e00).is_nan());
        assert!((f16_to_f32(0x0001) - 2f32.powi(-24)).abs() < 1e-10);
    }

    #[test]
    fn roundtrip_aproximado() {
        for i in -1000..1000 {
            let v = i as f32 * 0.37;
            let rt = f16_to_f32(f32_to_f16(v));
            let err = (v - rt).abs();
            // f16 tiene ~3 decimales de precisión relativa
            assert!(err <= v.abs() * 1e-3 + 1e-4, "{v} → {rt} (err {err})");
        }
    }

    #[test]
    fn extremos() {
        assert_eq!(f32_to_f16(0.0), 0);
        assert!(f16_to_f32(f32_to_f16(1e10)).is_infinite());
        assert_eq!(f16_to_f32(f32_to_f16(1e-10)), 0.0);
        assert!(f16_to_f32(f32_to_f16(f32::NAN)).is_nan());
    }
}
