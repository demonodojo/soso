//! Instantánea portable para KV f16 completo de CPU. El fingerprint debe
//! identificar los pesos reales, tokenizador y configuración de inferencia;
//! el índice del modelo por sí solo no identifica el contenido de los shards.

use alloc::vec::Vec;
use sha2::{Digest, Sha256};

const MAGIC: &[u8; 8] = b"SOSOKV01";
const CABECERA: usize = 8 + 32 + 4 + 4 + 4;
const DIGEST: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    Incompatible,
    Longitud,
    Integridad,
    ValorNoFinito,
    EstadoNoCompatible,
}

/// Una capa con K y V f16; posiciones consecutivas desde cero.
pub struct Capa {
    pub k: Vec<u16>,
    pub v: Vec<u16>,
}

pub struct Snapshot {
    pub tokens: Vec<u32>,
    pub hidden: Vec<f32>,
    pub capas: Vec<Capa>,
}

pub fn bytes_necesarios(tokens: usize, hidden: usize, dimensiones: &[usize]) -> Option<usize> {
    let ancho = dimensiones.iter().try_fold(0usize, |s, d| s.checked_add(*d))?;
    CABECERA.checked_add(tokens.checked_mul(4)?)?
        .checked_add(dimensiones.len().checked_mul(4)?)?
        .checked_add(hidden.checked_mul(4)?)?
        .checked_add(tokens.checked_mul(ancho)?.checked_mul(4)?)?.checked_add(DIGEST)
}

fn finito(v: u16) -> bool { v & 0x7c00 != 0x7c00 }

/// Codifica la imagen completa. No serializa estado recurrente.
pub fn codificar(s: &Snapshot, perfil: &[u8; 32], dimensiones: &[usize], max_bytes: usize) -> Result<Vec<u8>, Error> {
    let n = s.tokens.len();
    let bytes = bytes_necesarios(n, s.hidden.len(), dimensiones).ok_or(Error::Longitud)?;
    if n == 0 || n > u32::MAX as usize || s.hidden.is_empty() || dimensiones.is_empty()
        || s.hidden.len() > u32::MAX as usize || s.capas.len() != dimensiones.len()
        || dimensiones.len() > u32::MAX as usize || bytes > max_bytes {
        return Err(Error::Longitud);
    }
    if s.hidden.iter().any(|x| !x.is_finite()) { return Err(Error::ValorNoFinito); }
    for (capa, &dim) in s.capas.iter().zip(dimensiones) {
        let len = n.checked_mul(dim).ok_or(Error::Longitud)?;
        if dim == 0 || dim > u32::MAX as usize || capa.k.len() != len || capa.v.len() != len { return Err(Error::Longitud); }
        if capa.k.iter().chain(&capa.v).any(|&x| !finito(x)) { return Err(Error::ValorNoFinito); }
    }
    let mut out = Vec::with_capacity(bytes);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(perfil);
    out.extend_from_slice(&(n as u32).to_le_bytes());
    out.extend_from_slice(&(dimensiones.len() as u32).to_le_bytes());
    out.extend_from_slice(&(s.hidden.len() as u32).to_le_bytes());
    for &d in dimensiones { out.extend_from_slice(&(d as u32).to_le_bytes()); }
    for &t in &s.tokens { out.extend_from_slice(&t.to_le_bytes()); }
    for &h in &s.hidden { out.extend_from_slice(&h.to_le_bytes()); }
    for capa in &s.capas {
        for &v in capa.k.iter().chain(&capa.v) { out.extend_from_slice(&v.to_le_bytes()); }
    }
    let digest = Sha256::digest(&out);
    out.extend_from_slice(&digest);
    Ok(out)
}

/// Comprueba límites, identidad y checksum antes de reservar o aceptar contenido.
pub fn decodificar(bytes: &[u8], perfil: &[u8; 32], dimensiones: &[usize], max_tokens: usize,
                   vocabulario: usize, hidden_dim: usize, max_bytes: usize) -> Result<Snapshot, Error> {
    if bytes.len() < CABECERA + DIGEST || bytes.len() > max_bytes { return Err(Error::Longitud); }
    if &bytes[..8] != MAGIC || &bytes[8..40] != perfil { return Err(Error::Incompatible); }
    let n = u32::from_le_bytes(bytes[40..44].try_into().map_err(|_| Error::Longitud)?) as usize;
    let capas = u32::from_le_bytes(bytes[44..48].try_into().map_err(|_| Error::Longitud)?) as usize;
    let hidden_guardado = u32::from_le_bytes(bytes[48..52].try_into().map_err(|_| Error::Longitud)?) as usize;
    if n == 0 || n > max_tokens || hidden_dim == 0 || dimensiones.is_empty()
        || capas != dimensiones.len() || dimensiones.iter().any(|&d| d == 0)
        || hidden_guardado != hidden_dim
        || bytes_necesarios(n, hidden_dim, dimensiones) != Some(bytes.len()) {
        return Err(Error::Longitud);
    }
    let fin = bytes.len() - DIGEST;
    if Sha256::digest(&bytes[..fin]).as_slice() != &bytes[fin..] { return Err(Error::Integridad); }
    let mut offset = CABECERA;
    for &dim in dimensiones {
        let d = u32::from_le_bytes(bytes[offset..offset + 4].try_into().map_err(|_| Error::Longitud)?) as usize;
        if d != dim { return Err(Error::Incompatible); }
        offset += 4;
    }
    let mut tokens = Vec::with_capacity(n);
    for _ in 0..n {
        let t = u32::from_le_bytes(bytes[offset..offset + 4].try_into().map_err(|_| Error::Longitud)?);
        if t as usize >= vocabulario { return Err(Error::Incompatible); }
        tokens.push(t);
        offset += 4;
    }
    let mut hidden = Vec::with_capacity(hidden_dim);
    for _ in 0..hidden_dim {
        let x = f32::from_le_bytes(bytes[offset..offset + 4].try_into().map_err(|_| Error::Longitud)?);
        if !x.is_finite() { return Err(Error::ValorNoFinito); }
        hidden.push(x);
        offset += 4;
    }
    let mut salida = Vec::with_capacity(capas);
    for &dim in dimensiones {
        let len = n * dim; // Ya validado por bytes_necesarios().
        let mut k = Vec::with_capacity(len);
        let mut v = Vec::with_capacity(len);
        for destino in [&mut k, &mut v] {
            for _ in 0..len {
                let x = u16::from_le_bytes([bytes[offset], bytes[offset + 1]]);
                if !finito(x) { return Err(Error::ValorNoFinito); }
                destino.push(x);
                offset += 2;
            }
        }
        salida.push(Capa { k, v });
    }
    Ok(Snapshot { tokens, hidden, capas: salida })
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn ejemplo() -> Vec<u8> {
        codificar(&Snapshot { tokens: vec![1, 2], hidden: vec![0.5],
            capas: vec![Capa { k: vec![0x3c00; 4], v: vec![0x4000; 4] }] },
            &[9; 32], &[2], 1024).unwrap()
    }

    fn recalc(bytes: &mut [u8]) {
        let n = bytes.len() - DIGEST;
        let hash = Sha256::digest(&bytes[..n]);
        bytes[n..].copy_from_slice(&hash);
    }

    #[test]
    fn rechaza_cambios_y_longitudes_sin_reservas_excesivas() {
        let bytes = ejemplo();
        let leer = |b: &[u8], perfil: &[u8; 32], dims: &[usize], max| {
            decodificar(b, perfil, dims, 10, 100, 1, max).map(|_| ())
        };
        assert_eq!(leer(&bytes, &[8; 32], &[2], 1024), Err(Error::Incompatible));
        assert_eq!(leer(&bytes, &[9; 32], &[3], 1024), Err(Error::Longitud));
        assert_eq!(leer(&bytes, &[9; 32], &[2], bytes.len() - 1), Err(Error::Longitud));
        for corte in 0..bytes.len() {
            assert!(leer(&bytes[..corte], &[9; 32], &[2], 1024).is_err());
        }
        let mut corrupto = bytes.clone();
        corrupto[56] ^= 1;
        assert_eq!(leer(&corrupto, &[9; 32], &[2], 1024), Err(Error::Integridad));
        corrupto[40..44].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(leer(&corrupto, &[9; 32], &[2], 1024), Err(Error::Longitud));
    }

    #[test]
    fn rechaza_valores_invalidos_incluso_con_checksum_correcto() {
        let mut bytes = ejemplo();
        bytes[56..60].copy_from_slice(&100u32.to_le_bytes());
        recalc(&mut bytes);
        assert_eq!(decodificar(&bytes, &[9; 32], &[2], 10, 100, 1, 1024).err(), Some(Error::Incompatible));
        let mut bytes = ejemplo();
        bytes[64..68].copy_from_slice(&f32::NAN.to_le_bytes());
        recalc(&mut bytes);
        assert_eq!(decodificar(&bytes, &[9; 32], &[2], 10, 100, 1, 1024).err(), Some(Error::ValorNoFinito));
        let mut bytes = ejemplo();
        bytes[68..70].copy_from_slice(&0x7c00u16.to_le_bytes());
        recalc(&mut bytes);
        assert_eq!(decodificar(&bytes, &[9; 32], &[2], 10, 100, 1, 1024).err(), Some(Error::ValorNoFinito));
    }

    #[test]
    fn rechaza_geometria_distinta_aunque_tenga_el_mismo_tamano() {
        let s = Snapshot { tokens: vec![1], hidden: vec![0.0], capas: vec![
            Capa { k: vec![0; 1], v: vec![0; 1] },
            Capa { k: vec![0; 3], v: vec![0; 3] },
        ] };
        let b = codificar(&s, &[4; 32], &[1, 3], 1024).unwrap();
        assert_eq!(decodificar(&b, &[4; 32], &[2, 2], 10, 100, 1, 1024).err(), Some(Error::Incompatible));
    }
}
