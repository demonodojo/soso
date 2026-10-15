//! Aceptación reservada de R35: el análisis del GGUF no se fía de las longitudes del fichero.

use gguf2som::{convert, Read, Seek, SomOut};

struct Mem {
    data: Vec<u8>,
    pos: usize,
}

impl Read for Mem {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, String> {
        let n = buf.len().min(self.data.len() - self.pos);
        buf[..n].copy_from_slice(&self.data[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}

impl Seek for Mem {
    fn seek_start(&mut self, pos: u64) -> Result<(), String> {
        self.pos = pos as usize;
        Ok(())
    }
    fn stream_position(&mut self) -> Result<u64, String> {
        Ok(self.pos as u64)
    }
}

struct Nada;

impl SomOut for Nada {
    fn mkdir(&mut self, _: &str) -> Result<(), String> {
        Ok(())
    }
    fn write(&mut self, _: &str, _: &[u8]) -> Result<(), String> {
        Ok(())
    }
}

fn cabecera(tensores: u64, kvs: u64) -> Vec<u8> {
    let mut v = b"GGUF".to_vec();
    v.extend_from_slice(&3u32.to_le_bytes());
    v.extend_from_slice(&tensores.to_le_bytes());
    v.extend_from_slice(&kvs.to_le_bytes());
    v
}

fn cadena(v: &mut Vec<u8>, s: &str) {
    v.extend_from_slice(&(s.len() as u64).to_le_bytes());
    v.extend_from_slice(s.as_bytes());
}

fn kv_str(v: &mut Vec<u8>, k: &str, val: &str) {
    cadena(v, k);
    v.extend_from_slice(&8u32.to_le_bytes());
    cadena(v, val);
}

fn convertir(data: Vec<u8>) -> Result<(), String> {
    convert(&mut Mem { data, pos: 0 }, &mut Nada, None)
}

#[test]
fn un_gguf_bien_formado_se_analiza_y_llega_a_la_comprobacion_de_arquitectura() {
    let mut v = cabecera(0, 2);
    kv_str(&mut v, "general.architecture", "inventada");
    // Una plantilla larga pero legítima (200 KB) no es un error.
    kv_str(&mut v, "tokenizer.chat_template", &"x".repeat(200_000));
    let e = convertir(v).unwrap_err();
    assert!(e.contains("arquitectura inventada"), "error inesperado: {e}");
}

#[test]
fn una_cadena_de_longitud_absurda_es_error_y_no_aborta() {
    for len in [u64::MAX, 1u64 << 62, 1u64 << 40, (16u64 << 20) + 1] {
        let mut v = cabecera(0, 1);
        cadena(&mut v, "general.architecture");
        v.extend_from_slice(&8u32.to_le_bytes());
        v.extend_from_slice(&len.to_le_bytes());
        v.extend_from_slice(b"corto");
        let e = convertir(v).unwrap_err();
        assert!(e.contains("cadena"), "len={len}: {e}");
    }
}

#[test]
fn la_clave_de_un_metadato_tambien_esta_acotada() {
    let mut v = cabecera(0, 1);
    v.extend_from_slice(&u64::MAX.to_le_bytes());
    let e = convertir(v).unwrap_err();
    assert!(e.contains("cadena"), "{e}");
}

#[test]
fn el_nombre_de_un_tensor_tambien_esta_acotado() {
    let mut v = cabecera(1, 0);
    v.extend_from_slice(&(1u64 << 50).to_le_bytes());
    let e = convertir(v).unwrap_err();
    assert!(e.contains("cadena"), "{e}");
}

#[test]
fn un_tensor_con_demasiadas_dimensiones_se_rechaza() {
    let mut v = cabecera(1, 1);
    kv_str(&mut v, "general.architecture", "inventada");
    cadena(&mut v, "blk.0.attn_q.weight");
    v.extend_from_slice(&9u32.to_le_bytes());
    for d in 1..=9u64 {
        v.extend_from_slice(&d.to_le_bytes());
    }
    v.extend_from_slice(&0u32.to_le_bytes()); // tipo
    v.extend_from_slice(&0u64.to_le_bytes()); // offset
    let e = convertir(v).unwrap_err();
    assert!(e.contains("dimensiones"), "{e}");
    assert!(e.contains("blk.0.attn_q.weight"), "{e}");
}

#[test]
fn un_numero_de_dimensiones_enorme_es_error_sin_reservar() {
    let mut v = cabecera(1, 1);
    kv_str(&mut v, "general.architecture", "inventada");
    cadena(&mut v, "t");
    v.extend_from_slice(&u32::MAX.to_le_bytes());
    let e = convertir(v).unwrap_err();
    assert!(e.contains("dimensiones"), "{e}");
}

#[test]
fn ocho_dimensiones_siguen_siendo_validas() {
    let mut v = cabecera(1, 1);
    kv_str(&mut v, "general.architecture", "inventada");
    cadena(&mut v, "t");
    v.extend_from_slice(&8u32.to_le_bytes());
    for _ in 0..8 {
        v.extend_from_slice(&1u64.to_le_bytes());
    }
    v.extend_from_slice(&0u32.to_le_bytes());
    v.extend_from_slice(&0u64.to_le_bytes());
    let e = convertir(v).unwrap_err();
    assert!(e.contains("arquitectura inventada"), "{e}");
}

#[test]
fn una_cadena_truncada_sigue_siendo_error() {
    let mut v = cabecera(0, 1);
    cadena(&mut v, "general.architecture");
    v.extend_from_slice(&8u32.to_le_bytes());
    v.extend_from_slice(&100_000u64.to_le_bytes());
    v.extend_from_slice(&[b'a'; 10]);
    assert!(convertir(v).is_err());
}
