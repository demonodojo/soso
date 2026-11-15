//! Transferencia acotada de una imagen KV sobre un transporte dedicado.
//! El SHA-256 detecta corrupción, no autentica el emisor ni cifra los datos.
//! La conexión debe estar autenticada y autorizada por el adaptador antes de
//! usar este módulo; no se mezcla con los mensajes del pipeline existente.

use alloc::vec::Vec;
use sha2::{Digest, Sha256};
use crate::pipeline::Transport;

const MAGIC: &[u8; 8] = b"SOSOKVT1";
const CABECERA: usize = 8 + 8 + 32;
const TROZO: usize = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error { Transporte, Longitud, Version, Integridad }

/// Envía una imagen ya validada; no requiere una trama del tamaño de la imagen.
pub fn enviar<T: Transport>(tx: &mut T, imagen: &[u8], max_bytes: usize) -> Result<(), Error> {
    if imagen.is_empty() || imagen.len() > max_bytes { return Err(Error::Longitud); }
    let mut cab = [0u8; CABECERA];
    cab[..8].copy_from_slice(MAGIC);
    cab[8..16].copy_from_slice(&(imagen.len() as u64).to_le_bytes());
    cab[16..].copy_from_slice(&Sha256::digest(imagen));
    tx.send_all(&cab).map_err(|_| Error::Transporte)?;
    for parte in imagen.chunks(TROZO) { tx.send_all(parte).map_err(|_| Error::Transporte)?; }
    Ok(())
}

fn leer_exacto<T: Transport>(tx: &mut T, mut dst: &mut [u8]) -> Result<(), Error> {
    while !dst.is_empty() {
        let n = tx.recv_some(dst).map_err(|_| Error::Transporte)?;
        if n == 0 || n > dst.len() { return Err(Error::Transporte); }
        dst = &mut dst[n..];
    }
    Ok(())
}

/// La cabecera se verifica antes de reservar. El llamante valida después el
/// perfil, la geometría y los tokens con `Runtime::importar_kv`.
pub fn recibir<T: Transport>(tx: &mut T, max_bytes: usize) -> Result<Vec<u8>, Error> {
    let mut cab = [0u8; CABECERA];
    leer_exacto(tx, &mut cab)?;
    if &cab[..8] != MAGIC { return Err(Error::Version); }
    let len64 = u64::from_le_bytes(cab[8..16].try_into().map_err(|_| Error::Longitud)?);
    let len = usize::try_from(len64).map_err(|_| Error::Longitud)?;
    if len == 0 || len > max_bytes { return Err(Error::Longitud); }
    let mut datos = alloc::vec![0u8; len];
    for parte in datos.chunks_mut(TROZO) { leer_exacto(tx, parte)?; }
    if Sha256::digest(&datos).as_slice() != &cab[16..] { return Err(Error::Integridad); }
    Ok(datos)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Default)]
    struct Mem { bytes: Vec<u8>, pos: usize, fragmento: usize }
    impl Transport for Mem {
        fn send_all(&mut self, data: &[u8]) -> Result<(), ()> { self.bytes.extend_from_slice(data); Ok(()) }
        fn recv_some(&mut self, buf: &mut [u8]) -> Result<usize, ()> {
            let n = buf.len().min(self.fragmento.max(1)).min(self.bytes.len() - self.pos);
            buf[..n].copy_from_slice(&self.bytes[self.pos..self.pos + n]);
            self.pos += n;
            Ok(n)
        }
    }

    #[test]
    fn fragmentacion_limites_y_corrupcion() {
        let payload: Vec<u8> = (0..150_000).map(|i| (i % 251) as u8).collect();
        let mut m = Mem { fragmento: 17, ..Default::default() };
        enviar(&mut m, &payload, 200_000).unwrap();
        assert_eq!(recibir(&mut m, 200_000).unwrap(), payload);
        m.pos = 0;
        assert_eq!(recibir(&mut m, 100).err(), Some(Error::Longitud));
        m.pos = 0;
        m.bytes[CABECERA + 3] ^= 1;
        assert_eq!(recibir(&mut m, 200_000).err(), Some(Error::Integridad));
        m.pos = 0;
        m.bytes.truncate(CABECERA + 10);
        assert_eq!(recibir(&mut m, 200_000).err(), Some(Error::Transporte));
        assert_eq!(enviar(&mut m, &[], 10), Err(Error::Longitud));
    }
}
