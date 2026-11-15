//! Códec experimental para filas KV en E2M1: 32 valores por bloque, escala
//! f32 y 16 bytes de datos. No comparte formato con el MXFP4 de pesos de soso.
//! Todavía no se usa en inferencia: permite medir error y espacio antes de
//! modificar `LayerKv` o activar el formato en un modelo.

use alloc::vec::Vec;

pub const ELEMENTOS: usize = 32;
pub const BYTES_BLOQUE: usize = 20;
const MAGNITUDES: [f32; 8] = [0.0, 0.5, 1.0, 1.5, 2.0, 3.0, 4.0, 6.0];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    Longitud,
    ValorNoFinito,
    EscalaInvalida,
    RellenoInvalido,
}

/// Tamaño en bytes incluida la escala y el relleno del último grupo.
pub fn bytes_para(elementos: usize) -> Option<usize> {
    elementos.checked_add(ELEMENTOS - 1)?.checked_div(ELEMENTOS)?.checked_mul(BYTES_BLOQUE)
}

/// Cuantiza sin aceptar NaN ni infinito. Cada fila debe codificarse por separado.
pub fn codificar(fila: &[f32]) -> Result<Vec<u8>, Error> {
    let mut salida = Vec::with_capacity(bytes_para(fila.len()).ok_or(Error::Longitud)?);
    for grupo in fila.chunks(ELEMENTOS) {
        let mut max = 0.0f32;
        for &x in grupo {
            if !x.is_finite() { return Err(Error::ValorNoFinito); }
            max = max.max(x.abs());
        }
        // La escala mínima evita división por cero también con subnormales.
        let escala = if max == 0.0 { 1.0 } else { (max / 6.0).max(f32::MIN_POSITIVE) };
        if !(escala * 6.0).is_finite() { return Err(Error::EscalaInvalida); }
        salida.extend_from_slice(&escala.to_le_bytes());
        let mut datos = [0u8; 16];
        for (i, &x) in grupo.iter().enumerate() {
            let objetivo = x.abs() / escala;
            let mut mejor = 0usize;
            let mut error = f32::INFINITY;
            for (j, &valor) in MAGNITUDES.iter().enumerate() {
                let distancia = (objetivo - valor).abs();
                if distancia < error { mejor = j; error = distancia; }
            }
            let signo = if x.is_sign_negative() && mejor != 0 { 8 } else { 0 };
            datos[i / 2] |= ((mejor as u8) | signo) << (4 * (i % 2));
        }
        salida.extend_from_slice(&datos);
    }
    Ok(salida)
}

/// Valida el tamaño, las escalas y el relleno antes de escribir la fila de salida.
pub fn decodificar(datos: &[u8], salida: &mut [f32]) -> Result<(), Error> {
    decodificar_rango(datos, salida.len(), 0, salida)
}

/// Lee un rango de la fila completa sin reservar un vector temporal.
pub fn decodificar_rango(datos: &[u8], elementos: usize, inicio: usize, salida: &mut [f32]) -> Result<(), Error> {
    if Some(datos.len()) != bytes_para(elementos)
        || inicio.checked_add(salida.len()).is_none_or(|fin| fin > elementos) {
        return Err(Error::Longitud);
    }
    for (b, bloque) in datos.chunks_exact(BYTES_BLOQUE).enumerate() {
        let escala = f32::from_le_bytes(bloque[..4].try_into().map_err(|_| Error::Longitud)?);
        if !escala.is_finite() || escala <= 0.0 || !(escala * 6.0).is_finite() {
            return Err(Error::EscalaInvalida);
        }
        let usados = (elementos - b * ELEMENTOS).min(ELEMENTOS);
        for i in usados..ELEMENTOS {
            let nibble = (bloque[4 + i / 2] >> (4 * (i % 2))) & 15;
            if nibble != 0 { return Err(Error::RellenoInvalido); }
        }
    }
    for (i, destino) in salida.iter_mut().enumerate() {
        let posicion = inicio + i;
        let b = posicion / ELEMENTOS;
        let j = posicion % ELEMENTOS;
        let bloque = &datos[b * BYTES_BLOQUE..(b + 1) * BYTES_BLOQUE];
        let escala = f32::from_le_bytes(bloque[..4].try_into().map_err(|_| Error::Longitud)?);
        let nibble = (bloque[4 + j / 2] >> (4 * (j % 2))) & 15;
        let valor = MAGNITUDES[(nibble & 7) as usize] * escala;
        *destino = if nibble & 8 != 0 { -valor } else { valor };
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn e2m1_representa_los_niveles_de_referencia() {
        let valores = [0.0, 0.5, -1.0, 1.5, -2.0, 3.0, -4.0, 6.0];
        let datos = codificar(&valores).unwrap();
        assert_eq!(&datos[4..8], &[0x10, 0x3a, 0x5c, 0x7e]);
        let mut out = [0.0; 8];
        decodificar(&datos, &mut out).unwrap();
        assert_eq!(out, valores);
    }

    #[test]
    fn filas_con_relleno_y_escalas_independientes() {
        let mut valores = vec![0.0; 33];
        valores[0] = 60.0;
        valores[32] = -0.006;
        let datos = codificar(&valores).unwrap();
        assert_eq!(datos.len(), 40);
        let mut out = vec![0.0; 33];
        decodificar(&datos, &mut out).unwrap();
        assert!((out[0] - valores[0]).abs() < 1e-5);
        assert!((out[32] - valores[32]).abs() < 1e-6);
        let mut cruce = [0.0; 2];
        decodificar_rango(&datos, 33, 31, &mut cruce).unwrap();
        assert_eq!(cruce, [out[31], out[32]]);
        assert_eq!(decodificar_rango(&datos, 33, 32, &mut cruce), Err(Error::Longitud));
        assert_eq!(bytes_para(usize::MAX), None);
    }

    #[test]
    fn errores_no_modifican_salida() {
        assert_eq!(codificar(&[f32::NAN]), Err(Error::ValorNoFinito));
        assert_eq!(codificar(&[f32::INFINITY]), Err(Error::ValorNoFinito));
        let mut datos = codificar(&[1.0]).unwrap();
        let mut out = [123.0];
        datos[4] |= 0xf0;
        assert_eq!(decodificar(&datos, &mut out), Err(Error::RellenoInvalido));
        assert_eq!(out, [123.0]);
        datos[4] &= 0x0f;
        datos[..4].copy_from_slice(&f32::NAN.to_le_bytes());
        assert_eq!(decodificar(&datos, &mut out), Err(Error::EscalaInvalida));
        assert_eq!(out, [123.0]);
        assert_eq!(decodificar(&datos[..19], &mut out), Err(Error::Longitud));

        let mut dos = codificar(&[1.0; 33]).unwrap();
        dos[20..24].copy_from_slice(&0.0f32.to_le_bytes());
        let mut out_dos = [123.0; 33];
        assert_eq!(decodificar(&dos, &mut out_dos), Err(Error::EscalaInvalida));
        assert_eq!(out_dos, [123.0; 33]);
    }

    #[test]
    fn error_por_grupo_acotado_y_ceros() {
        let valores: Vec<f32> = (0..96).map(|i| ((i * 37 % 101) as f32 - 50.0) / 7.0).collect();
        let datos = codificar(&valores).unwrap();
        let mut out = vec![0.0; valores.len()];
        decodificar(&datos, &mut out).unwrap();
        for (a, b) in valores.chunks(32).zip(out.chunks(32)) {
            let max = a.iter().fold(0.0f32, |m, v| m.max(v.abs()));
            for (x, y) in a.iter().zip(b) {
                assert!((x - y).abs() <= max / 6.0 + 1e-5);
            }
        }
        let datos_cero = codificar(&[0.0; 32]).unwrap();
        let mut ceros = [1.0; 32];
        decodificar(&datos_cero, &mut ceros).unwrap();
        assert_eq!(ceros, [0.0; 32]);
    }
}
