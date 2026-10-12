//! Emisión streaming de shards F32.

use sosomodel::index::{make_f32_entry, TensorIndex, SHARD_PAYLOAD_OFF};
use sosomodel::{align_up, Crc32cDigest, BLOCK_ALIGN};
use soso_llm_core::f16::f16_to_f32;
use std::fs::{self, File, OpenOptions};
use std::io::{BufWriter, Read, Seek, SeekFrom, Write};
use std::path::Path;

const CHUNK: usize = 64 * 1024;

pub struct Emitter {
    shards: std::path::PathBuf,
    index: TensorIndex,
    next_id: u32,
    total_bytes: u64,
}

impl Emitter {
    pub fn new(out: &Path) -> Self {
        let shards = out.join("shards");
        fs::create_dir_all(&shards).expect("shards dir");
        Self {
            shards,
            index: TensorIndex::default(),
            next_id: 0,
            total_bytes: 0,
        }
    }

    pub fn finish(self, _out: &Path) -> (TensorIndex, u64) {
        (self.index, self.total_bytes)
    }

    fn write_f32_payload(
        path: &Path,
        mut write_payload: impl FnMut(&mut dyn Write) -> Result<(), String>,
    ) -> Result<(), String> {
        let mut crc = Crc32cDigest::new();
        let mut f = BufWriter::new(File::create(path).map_err(|e| e.to_string())?);
        let mut header = vec![0u8; SHARD_PAYLOAD_OFF];
        header[..8].copy_from_slice(b"SOMODL01");
        f.write_all(&header).map_err(|e| e.to_string())?;

        let mut crc_writer = CrcCountWriter {
            inner: &mut f,
            crc: &mut crc,
            len: 0,
        };
        write_payload(&mut crc_writer)?;
        let payload_len = crc_writer.len;
        let crc_val = crc.finalize();

        let pad = align_up(payload_len, BLOCK_ALIGN) - payload_len;
        if pad > 0 {
            f.write_all(&vec![0u8; pad]).map_err(|e| e.to_string())?;
        }
        f.flush().map_err(|e| e.to_string())?;
        drop(f);

        let mut hdr = OpenOptions::new()
            .write(true)
            .open(path)
            .map_err(|e| e.to_string())?;
        hdr.seek(SeekFrom::Start(8)).map_err(|e| e.to_string())?;
        hdr.write_all(&crc_val.to_le_bytes())
            .map_err(|e| e.to_string())?;
        hdr.write_all(&2u32.to_le_bytes())
            .map_err(|e| e.to_string())?;
        hdr.write_all(&(payload_len as u64).to_le_bytes())
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn emit_f32_from_reader(
        &mut self,
        som_name: &str,
        shape: &[u32],
        reader: &mut File,
        raw_bytes: u64,
        ttype: i32,
        transpose_2d: bool,
    ) -> Result<(), String> {
        let shard_path = self.shards.join(format!("{som_name}.tensor"));
        let elems: u64 = shape.iter().map(|&d| d as u64).product();

        Self::write_f32_payload(&shard_path, |w| {
            match (ttype, shape.len(), transpose_2d) {
                (1, 2, true) => {
                    let cols = shape[1] as usize;
                    let rows = shape[0] as usize;
                    let mut row_raw = vec![0u8; cols * 2];
                    let mut row_f32 = vec![0u8; cols * 4];
                    for _r in 0..rows {
                        reader.read_exact(&mut row_raw).map_err(|e| e.to_string())?;
                        for (c, chunk) in row_f32.chunks_mut(4).enumerate() {
                            let bits = u16::from_le_bytes([row_raw[c * 2], row_raw[c * 2 + 1]]);
                            chunk.copy_from_slice(&f16_to_f32(bits).to_le_bytes());
                        }
                        w.write_all(&row_f32).map_err(|e| e.to_string())?;
                    }
                    Ok(())
                }
                (1, _, false) => {
                    let mut left = raw_bytes as usize;
                    let mut buf = vec![0u8; CHUNK.min(left.max(1))];
                    let mut out = Vec::new();
                    while left > 0 {
                        let n = left.min(CHUNK);
                        buf.resize(n, 0);
                        reader.read_exact(&mut buf).map_err(|e| e.to_string())?;
                        out.clear();
                        out.reserve(n * 2);
                        for i in 0..(n / 2) {
                            let bits = u16::from_le_bytes([buf[i * 2], buf[i * 2 + 1]]);
                            out.extend_from_slice(&f16_to_f32(bits).to_le_bytes());
                        }
                        w.write_all(&out).map_err(|e| e.to_string())?;
                        left -= n;
                    }
                    Ok(())
                }
                (0, _, false) => {
                    let mut left = raw_bytes as usize;
                    let mut buf = vec![0u8; CHUNK.min(left.max(1))];
                    while left > 0 {
                        let n = left.min(CHUNK);
                        buf.resize(n, 0);
                        reader.read_exact(&mut buf).map_err(|e| e.to_string())?;
                        w.write_all(&buf).map_err(|e| e.to_string())?;
                        left -= n;
                    }
                    Ok(())
                }
                _ => Err(format!(
                    "tipo/shape no soportado: ttype={ttype} shape={shape:?} transpose={transpose_2d}"
                )),
            }
        })?;

        let payload_len = elems * 4;
        self.total_bytes += payload_len;
        let shard_name = format!("{som_name}.tensor");
        self.index.entries.push(make_f32_entry(
            self.next_id,
            som_name,
            &shard_name,
            0,
            shape,
        ));
        self.next_id += 1;
        Ok(())
    }

    pub fn emit_pos_or_embed(
        &mut self,
        som_name: &str,
        rows: u32,
        cols: u32,
        reader: &mut File,
        ttype: i32,
    ) -> Result<(), String> {
        let shard_path = self.shards.join(format!("{som_name}.tensor"));
        Self::write_f32_payload(&shard_path, |w| {
            let ggml_cols = cols as usize;
            let mut col_raw = vec![0u8; ggml_cols * 2];
            let mut col_f32 = vec![0u8; ggml_cols * 4];
            for _r in 0..rows {
                match ttype {
                    1 => {
                        reader.read_exact(&mut col_raw).map_err(|e| e.to_string())?;
                        for (i, chunk) in col_f32.chunks_mut(4).enumerate() {
                            let bits = u16::from_le_bytes([col_raw[i * 2], col_raw[i * 2 + 1]]);
                            chunk.copy_from_slice(&f16_to_f32(bits).to_le_bytes());
                        }
                        w.write_all(&col_f32).map_err(|e| e.to_string())?;
                    }
                    0 => {
                        col_f32.resize(ggml_cols * 4, 0);
                        reader.read_exact(&mut col_f32).map_err(|e| e.to_string())?;
                        w.write_all(&col_f32).map_err(|e| e.to_string())?;
                    }
                    _ => return Err("pos/embed: solo F16/F32".into()),
                }
            }
            Ok(())
        })?;
        let elems = (rows as u64) * (cols as u64);
        self.total_bytes += elems * 4;
        let shard_name = format!("{som_name}.tensor");
        self.index.entries.push(make_f32_entry(
            self.next_id,
            som_name,
            &shard_name,
            0,
            &[rows, cols],
        ));
        self.next_id += 1;
        Ok(())
    }
}

struct CrcCountWriter<'a, W: Write> {
    inner: &'a mut W,
    crc: &'a mut Crc32cDigest,
    len: usize,
}

impl<W: Write> Write for CrcCountWriter<'_, W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.crc.update(buf);
        self.len += buf.len();
        self.inner.write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}
