//! Aceptación reservada de R33.

use soso_audio::wav::{parse_pcm16_mono, WavError};

fn chunk(id: &[u8; 4], decl: u32, cuerpo: &[u8]) -> Vec<u8> {
    let mut v = id.to_vec();
    v.extend_from_slice(&decl.to_le_bytes());
    v.extend_from_slice(cuerpo);
    if cuerpo.len() % 2 == 1 && decl as usize == cuerpo.len() {
        v.push(0);
    }
    v
}

fn fmt(canales: u16, rate: u32, bits: u16, formato: u16) -> Vec<u8> {
    let mut b = Vec::new();
    b.extend_from_slice(&formato.to_le_bytes());
    b.extend_from_slice(&canales.to_le_bytes());
    b.extend_from_slice(&rate.to_le_bytes());
    b.extend_from_slice(&(rate * canales as u32 * bits as u32 / 8).to_le_bytes());
    b.extend_from_slice(&(canales * bits / 8).to_le_bytes());
    b.extend_from_slice(&bits.to_le_bytes());
    chunk(b"fmt ", 16, &b)
}

fn bytes(m: &[i16]) -> Vec<u8> {
    m.iter().flat_map(|s| s.to_le_bytes()).collect()
}

fn riff(chunks: &[Vec<u8>]) -> Vec<u8> {
    let cuerpo: Vec<u8> = chunks.concat();
    let mut v = b"RIFF".to_vec();
    v.extend_from_slice(&(4 + cuerpo.len() as u32).to_le_bytes());
    v.extend_from_slice(b"WAVE");
    v.extend_from_slice(&cuerpo);
    v
}

fn data(m: &[i16]) -> Vec<u8> {
    let b = bytes(m);
    chunk(b"data", b.len() as u32, &b)
}

#[test]
fn mono_normal_sigue_igual() {
    let w = riff(&[fmt(1, 16_000, 16, 1), data(&[1, -2, 300, i16::MIN])]);
    assert_eq!(parse_pcm16_mono(&w), Ok((16_000, vec![1, -2, 300, i16::MIN])));
}

#[test]
fn estereo_se_promedia() {
    let w = riff(&[fmt(2, 44_100, 16, 1), data(&[10, 20, -5, 4, 7, 8, -7, -8, 32767, 32767])]);
    let (r, pcm) = parse_pcm16_mono(&w).unwrap();
    assert_eq!(r, 44_100);
    assert_eq!(pcm, vec![15, 0, 7, -7, 32767]);
    // La división es la entera de Rust (hacia cero) y no desborda i16.
    let w = riff(&[fmt(2, 8_000, 16, 1), data(&[1, 0, -1, 0, i16::MIN, i16::MIN, -3, 0])]);
    assert_eq!(parse_pcm16_mono(&w).unwrap().1, vec![0, 0, i16::MIN, -1]);
}

#[test]
fn estereo_con_trama_incompleta_la_ignora() {
    let mut b = bytes(&[2, 4, 6, 8]);
    b.extend_from_slice(&[0x55, 0x66, 0x77]); // media trama estéreo y un byte
    let w = riff(&[fmt(2, 16_000, 16, 1), chunk(b"data", b.len() as u32, &b)]);
    assert_eq!(parse_pcm16_mono(&w).unwrap().1, vec![3, 7]);
}

#[test]
fn otros_formatos_no_se_admiten() {
    let d = data(&[1, 2, 3, 4]);
    assert_eq!(parse_pcm16_mono(&riff(&[fmt(3, 16_000, 16, 1), d.clone()])), Err(WavError::Unsupported));
    assert_eq!(parse_pcm16_mono(&riff(&[fmt(0, 16_000, 16, 1), d.clone()])), Err(WavError::Unsupported));
    assert_eq!(parse_pcm16_mono(&riff(&[fmt(2, 16_000, 8, 1), d.clone()])), Err(WavError::Unsupported));
    assert_eq!(parse_pcm16_mono(&riff(&[fmt(1, 16_000, 24, 1), d.clone()])), Err(WavError::Unsupported));
    assert_eq!(parse_pcm16_mono(&riff(&[fmt(1, 16_000, 16, 3), d])), Err(WavError::Unsupported));
}

#[test]
fn data_con_tamano_excesivo_llega_al_final() {
    let b = bytes(&[5, 6, 7, 8, 9]);
    // Streaming: tamaño máximo.
    let w = riff(&[fmt(1, 16_000, 16, 1), chunk(b"data", u32::MAX, &b)]);
    assert_eq!(parse_pcm16_mono(&w).unwrap().1, vec![5, 6, 7, 8, 9]);
    // Cortado: declara 100 bytes y quedan 10 + 1 suelto.
    let mut b2 = b.clone();
    b2.push(0xAA);
    let w = riff(&[fmt(1, 16_000, 16, 1), chunk(b"data", 100, &b2)]);
    assert_eq!(parse_pcm16_mono(&w).unwrap().1, vec![5, 6, 7, 8, 9]);
    // Estéreo cortado.
    let w = riff(&[fmt(2, 16_000, 16, 1), chunk(b"data", 4000, &bytes(&[2, 4, 10, 20]))]);
    assert_eq!(parse_pcm16_mono(&w).unwrap().1, vec![3, 15]);
}

#[test]
fn un_chunk_ajeno_cortado_no_aporta_muestras() {
    let w = riff(&[fmt(1, 16_000, 16, 1), data(&[1, 2]), chunk(b"LIST", 500, &[1, 2, 3, 4])]);
    assert_eq!(parse_pcm16_mono(&w).unwrap().1, vec![1, 2]);
}

#[test]
fn data_antes_de_fmt() {
    let w = riff(&[data(&[4, 6, 8, 10]), fmt(2, 22_050, 16, 1)]);
    assert_eq!(parse_pcm16_mono(&w), Ok((22_050, vec![5, 9])));
    let w = riff(&[data(&[1, 2, 3]), fmt(1, 16_000, 16, 1)]);
    assert_eq!(parse_pcm16_mono(&w), Ok((16_000, vec![1, 2, 3])));
}

#[test]
fn varios_data_se_concatenan_y_los_ajenos_se_saltan() {
    let w = riff(&[
        fmt(1, 16_000, 16, 1),
        data(&[1, 2]),
        chunk(b"LIST", 3, &[9, 9, 9]),
        data(&[3, 4]),
    ]);
    assert_eq!(parse_pcm16_mono(&w).unwrap().1, vec![1, 2, 3, 4]);
}

#[test]
fn sin_fmt_o_sin_muestras_es_badfmt() {
    assert_eq!(parse_pcm16_mono(&riff(&[data(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12])])), Err(WavError::BadFmt));
    assert_eq!(parse_pcm16_mono(&riff(&[fmt(1, 16_000, 16, 1), data(&[])])), Err(WavError::BadFmt));
    let w = riff(&[fmt(2, 16_000, 16, 1), chunk(b"data", 2, &[1, 0])]);
    assert_eq!(parse_pcm16_mono(&w), Err(WavError::BadFmt));
    // Rellenos para pasar el mínimo de 44 bytes sin chunk data.
    let w = riff(&[fmt(1, 16_000, 16, 1), chunk(b"LIST", 8, &[0; 8])]);
    assert_eq!(parse_pcm16_mono(&w), Err(WavError::BadFmt));
}

#[test]
fn cabeceras_invalidas_no_cambian() {
    assert_eq!(parse_pcm16_mono(&[0u8; 10]), Err(WavError::TooShort));
    let mut w = riff(&[fmt(1, 16_000, 16, 1), data(&[1, 2, 3])]);
    w[0] = b'X';
    assert_eq!(parse_pcm16_mono(&w), Err(WavError::BadRiff));
}
