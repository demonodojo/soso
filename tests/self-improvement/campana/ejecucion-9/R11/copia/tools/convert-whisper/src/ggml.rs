//! Parser streaming del formato `.bin` ggml de whisper.cpp.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

pub const GGML_MAGIC: u32 = 0x6767_6d6c;

pub const GGML_F32: i32 = 0;
pub const GGML_F16: i32 = 1;

#[derive(Clone, Debug)]
pub struct HParams {
    pub n_vocab: i32,
    pub n_audio_ctx: i32,
    pub n_audio_state: i32,
    pub n_audio_head: i32,
    pub n_audio_layer: i32,
    pub n_text_ctx: i32,
    pub n_text_state: i32,
    pub n_text_head: i32,
    pub n_text_layer: i32,
    pub n_mels: i32,
    pub ftype: i32,
}

#[derive(Clone, Debug)]
pub struct TensorMeta {
    pub name: String,
    pub ne: Vec<i32>,
    pub ttype: i32,
    pub offset: u64,
    pub nbytes: u64,
}

pub struct WhisperBin {
    file: File,
    pub hparams: HParams,
    pub vocab: Vec<String>,
    pub tensors: Vec<TensorMeta>,
}

fn read_i32(r: &mut File) -> io::Result<i32> {
    let mut b = [0u8; 4];
    r.read_exact(&mut b)?;
    Ok(i32::from_le_bytes(b))
}

fn tensor_nbytes(ttype: i32, elems: u64) -> u64 {
    match ttype {
        GGML_F32 => elems * 4,
        GGML_F16 => elems * 2,
        8 => (elems / 32) * 34, // Q8_0
        _ => elems * 2,
    }
}

impl WhisperBin {
    pub fn open(path: &Path) -> Result<Self, String> {
        let mut file = File::open(path).map_err(|e| e.to_string())?;
        let magic = read_i32(&mut file).map_err(|e| e.to_string())? as u32;
        if magic != GGML_MAGIC {
            return Err(format!("magic ggml inválido: {magic:#x}"));
        }
        let hparams = HParams {
            n_vocab: read_i32(&mut file).map_err(|e| e.to_string())?,
            n_audio_ctx: read_i32(&mut file).map_err(|e| e.to_string())?,
            n_audio_state: read_i32(&mut file).map_err(|e| e.to_string())?,
            n_audio_head: read_i32(&mut file).map_err(|e| e.to_string())?,
            n_audio_layer: read_i32(&mut file).map_err(|e| e.to_string())?,
            n_text_ctx: read_i32(&mut file).map_err(|e| e.to_string())?,
            n_text_state: read_i32(&mut file).map_err(|e| e.to_string())?,
            n_text_head: read_i32(&mut file).map_err(|e| e.to_string())?,
            n_text_layer: read_i32(&mut file).map_err(|e| e.to_string())?,
            n_mels: read_i32(&mut file).map_err(|e| e.to_string())?,
            ftype: read_i32(&mut file).map_err(|e| e.to_string())?,
        };
        let n_mel = read_i32(&mut file).map_err(|e| e.to_string())?;
        let n_fft = read_i32(&mut file).map_err(|e| e.to_string())?;
        file.seek(SeekFrom::Current((n_mel as i64) * (n_fft as i64) * 4))
            .map_err(|e| e.to_string())?;

        let n_vocab_file = read_i32(&mut file).map_err(|e| e.to_string())?;
        let mut vocab = Vec::with_capacity(n_vocab_file as usize);
        for _ in 0..n_vocab_file {
            let len = read_i32(&mut file).map_err(|e| e.to_string())?;
            let mut buf = vec![0u8; len as usize];
            file.read_exact(&mut buf).map_err(|e| e.to_string())?;
            vocab.push(String::from_utf8_lossy(&buf).into_owned());
        }

        let mut tensors = Vec::new();
        loop {
            let nd = match read_i32(&mut file) {
                Ok(v) => v,
                Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => break,
                Err(e) => return Err(e.to_string()),
            };
            let name_len = read_i32(&mut file).map_err(|e| e.to_string())?;
            let ttype = read_i32(&mut file).map_err(|e| e.to_string())?;
            let mut ne = Vec::with_capacity(nd as usize);
            for _ in 0..nd {
                ne.push(read_i32(&mut file).map_err(|e| e.to_string())?);
            }
            let mut name_buf = vec![0u8; name_len as usize];
            file.read_exact(&mut name_buf)
                .map_err(|e| e.to_string())?;
            let name = String::from_utf8_lossy(&name_buf).into_owned();
            let elems: u64 = ne.iter().map(|&d| d as u64).product();
            let nbytes = tensor_nbytes(ttype, elems);
            let offset = file.stream_position().map_err(|e| e.to_string())?;
            file.seek(SeekFrom::Current(nbytes as i64))
                .map_err(|e| e.to_string())?;
            tensors.push(TensorMeta {
                name,
                ne,
                ttype,
                offset,
                nbytes,
            });
        }

        Ok(Self {
            file,
            hparams,
            vocab,
            tensors,
        })
    }

    pub fn tensor_data(&mut self, meta: &TensorMeta) -> Result<&mut File, String> {
        self.file
            .seek(SeekFrom::Start(meta.offset))
            .map_err(|e| e.to_string())?;
        Ok(&mut self.file)
    }
}
