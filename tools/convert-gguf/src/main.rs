//! Convierte un fichero GGUF (llama) al layout sosomodel (.som).
//!
//! Soporta tensores F32, F16 (→F32) y Q8_0 (repack a bloques Q8_0 propios),
//! GQA, ffn_gate/output_norm y exporta el vocabulario a tokenizer.som.

use soso_llm_core::quant::quantize_q8_0;
use soso_llm_core::tokenizer::{VocabTokenizer, NO_TOKEN};
use sosomodel::index::{make_f32_entry, make_q4_k_entry, make_q8_0_entry, pack_shard, TensorIndex};
use sosomodel::layout::{
    DTYPE_F32, DTYPE_Q4_K, DTYPE_Q8_0, INDEX_FILE, MANIFEST_FILE, Q4_K_BLOCK_BYTES,
    SHARDS_DIR, TOKENIZER_FILE,
};
use sosomodel::manifest::{LayerPrefetch, Manifest};
use std::collections::HashMap;
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

const GGML_F32: u32 = 0;
const GGML_F16: u32 = 1;
const GGML_Q8_0: u32 = 8;
const GGML_Q4_K: u32 = 12;
const GGML_Q6_K: u32 = 14;
/// Bloque GGML Q6_K: ql[128] + qh[64] + scales[16 i8] + d f16 = 210 bytes.
const Q6_K_BLOCK_BYTES: usize = 210;

fn main() {
    let mut args = std::env::args().skip(1);
    let gguf = args.next().unwrap_or_else(|| {
        eprintln!("uso: convert-gguf <modelo.gguf> [dir_salida] [--name nombre]");
        std::process::exit(2);
    });
    let mut out = PathBuf::from("target/converted-model");
    let mut name = None;
    let mut rest: Vec<String> = args.collect();
    let mut i = 0;
    while i < rest.len() {
        if rest[i] == "--name" {
            name = rest.get(i + 1).cloned();
            rest.remove(i);
            rest.remove(i);
            continue;
        }
        if !rest[i].starts_with('-') && out == PathBuf::from("target/converted-model") {
            out = PathBuf::from(&rest[i]);
            rest.remove(i);
            continue;
        }
        i += 1;
    }
    if let Err(e) = convert(&gguf, &out, name.as_deref()) {
        eprintln!("convert-gguf: {e}");
        std::process::exit(1);
    }
    println!("convert-gguf: modelo escrito en {}", out.display());
}

fn convert(gguf_path: &str, out_dir: &Path, name: Option<&str>) -> Result<(), String> {
    let mut file = fs::File::open(gguf_path).map_err(|e| format!("abrir {gguf_path}: {e}"))?;
    let gguf = parse_gguf(&mut file)?;

    if let Some(MetaValue::Str(arch)) = gguf.meta.get("general.architecture") {
        if arch != "llama" {
            return Err(format!("arquitectura {arch} no soportada (solo llama)"));
        }
    }

    let hidden = gguf.meta_u32("llama.embedding_length")?;
    let num_layers = gguf.meta_u32("llama.block_count")?;
    let ffn_dim = gguf.meta_u32("llama.feed_forward_length")?;
    let num_heads = gguf.meta_u32("llama.attention.head_count")?;
    let num_kv_heads = gguf
        .meta_u32("llama.attention.head_count_kv")
        .unwrap_or(num_heads);
    let rope_theta = gguf.meta_f32("llama.rope.freq_base").unwrap_or(10000.0);
    let rms_eps = gguf
        .meta_f32("llama.attention.layer_norm_rms_epsilon")
        .unwrap_or(1e-5);
    let tokens = match gguf.meta.get("tokenizer.ggml.tokens") {
        Some(MetaValue::StrArray(v)) => Some(v.clone()),
        _ => None,
    };
    let vocab = gguf
        .meta_u32("llama.vocab_size")
        .ok()
        .or_else(|| tokens.as_ref().map(|t| t.len() as u32))
        .unwrap_or(256);
    let max_seq = gguf.meta_u32("llama.context_length").unwrap_or(2048);

    let model_name = name
        .map(String::from)
        .or_else(|| {
            Path::new(gguf_path)
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| String::from("model"));

    let shards_dir = out_dir.join(SHARDS_DIR);
    fs::create_dir_all(&shards_dir).map_err(|e| e.to_string())?;

    // tensores por capa (GGUF → nombre .som)
    let layer_parts = [
        ("attn_norm", "attn_norm"),
        ("attn_q", "attn_q"),
        ("attn_k", "attn_k"),
        ("attn_v", "attn_v"),
        ("attn_output", "attn_output"),
        ("ffn_norm", "ffn_norm"),
        ("ffn_up", "ffn_up"),
        ("ffn_gate", "ffn_gate"),
        ("ffn_down", "ffn_down"),
    ];
    let globals = [
        ("token_embd.weight", "embed"),
        ("output.weight", "lm_head"),
        ("output_norm.weight", "output_norm"),
    ];

    let mut index = TensorIndex::default();
    let mut id = 0u32;
    let mut prefetch = Vec::new();

    let emit = |file: &mut fs::File,
                    index: &mut TensorIndex,
                    id: &mut u32,
                    gguf_name: &str,
                    som_name: &str|
     -> Result<bool, String> {
        let Some(t) = gguf.tensors.get(gguf_name) else {
            return Ok(false);
        };
        let (payload, dtype) = read_tensor(file, &gguf, t)?;
        let shard_name = format!("{som_name}.tensor");
        fs::write(shards_dir.join(&shard_name), pack_shard(&payload))
            .map_err(|e| e.to_string())?;
        // GGUF guarda ne[] con la dimensión rápida primero; sosomodel usa
        // row-major [filas, columnas], así que se invierte.
        let shape: Vec<u32> = t.shape.iter().rev().map(|&d| d as u32).collect();
        index.entries.push(match dtype {
            DTYPE_Q8_0 => make_q8_0_entry(*id, som_name, &shard_name, 0, &shape),
            DTYPE_Q4_K => make_q4_k_entry(*id, som_name, &shard_name, 0, &shape),
            _ => make_f32_entry(*id, som_name, &shard_name, 0, &shape),
        });
        *id += 1;
        Ok(true)
    };

    for layer in 0..num_layers {
        let mut shards = Vec::new();
        for (gguf_part, som_part) in layer_parts {
            let gguf_name = format!("blk.{layer}.{gguf_part}.weight");
            let som_name = format!("L{layer:02}.{som_part}");
            if emit(&mut file, &mut index, &mut id, &gguf_name, &som_name)? {
                shards.push(format!("{som_name}.tensor"));
            }
        }
        prefetch.push(LayerPrefetch { layer, shards });
    }
    for (gguf_name, som_name) in globals {
        emit(&mut file, &mut index, &mut id, gguf_name, som_name)?;
    }

    if index.find("embed").is_none() {
        return Err("el GGUF no contiene token_embd.weight".into());
    }

    let manifest = Manifest {
        name: model_name,
        vocab_size: vocab,
        hidden_dim: hidden,
        num_layers,
        num_heads,
        num_kv_heads,
        ffn_dim,
        max_seq,
        rope_theta,
        rms_eps,
        prefetch,
    };
    fs::write(out_dir.join(MANIFEST_FILE), manifest.serialize()).map_err(|e| e.to_string())?;
    fs::write(out_dir.join(INDEX_FILE), index.serialize()).map_err(|e| e.to_string())?;

    if let Some(pieces) = tokens {
        let bos = gguf.meta_u32("tokenizer.ggml.bos_token_id").unwrap_or(NO_TOKEN);
        let eos = gguf.meta_u32("tokenizer.ggml.eos_token_id").unwrap_or(NO_TOKEN);
        fs::write(
            out_dir.join(TOKENIZER_FILE),
            VocabTokenizer::serialize(&pieces, bos, eos),
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

struct TensorInfo {
    shape: Vec<u64>,
    ggml_type: u32,
    offset: u64,
}

struct GgufFile {
    meta: HashMap<String, MetaValue>,
    tensors: HashMap<String, TensorInfo>,
    data_offset: u64,
}

#[derive(Clone, Debug)]
enum MetaValue {
    Int(u64),
    F32(f32),
    Str(String),
    StrArray(Vec<String>),
    Other,
}

impl GgufFile {
    fn meta_u32(&self, key: &str) -> Result<u32, String> {
        match self.meta.get(key) {
            Some(MetaValue::Int(v)) if *v <= u32::MAX as u64 => Ok(*v as u32),
            Some(MetaValue::Int(_)) => Err(format!("meta {key} demasiado grande")),
            Some(_) => Err(format!("meta {key} no es entero")),
            None => Err(format!("meta {key} ausente")),
        }
    }

    fn meta_f32(&self, key: &str) -> Result<f32, String> {
        match self.meta.get(key) {
            Some(MetaValue::F32(v)) => Ok(*v),
            Some(MetaValue::Int(v)) => Ok(*v as f32),
            Some(_) => Err(format!("meta {key} no es float")),
            None => Err(format!("meta {key} ausente")),
        }
    }
}

fn parse_gguf(file: &mut fs::File) -> Result<GgufFile, String> {
    let mut magic = [0u8; 4];
    file.read_exact(&mut magic).map_err(|e| e.to_string())?;
    if &magic != b"GGUF" {
        return Err("magic GGUF inválido".into());
    }
    let version = read_u32(file)?;
    if version < 2 {
        return Err(format!("versión GGUF {version} no soportada"));
    }
    let tensor_count = read_u64(file)?;
    let kv_count = read_u64(file)?;

    let mut meta = HashMap::new();
    for _ in 0..kv_count {
        let key = read_string(file)?;
        let vtype = read_u32(file)?;
        let val = read_meta_value(file, vtype)?;
        meta.insert(key, val);
    }

    let mut tensors = HashMap::new();
    for _ in 0..tensor_count {
        let name = read_string(file)?;
        let n_dims = read_u32(file)? as usize;
        let mut shape = Vec::with_capacity(n_dims);
        for _ in 0..n_dims {
            shape.push(read_u64(file)?);
        }
        let ggml_type = read_u32(file)?;
        let offset = read_u64(file)?;
        tensors.insert(
            name,
            TensorInfo {
                shape,
                ggml_type,
                offset,
            },
        );
    }

    let alignment = match meta.get("general.alignment") {
        Some(MetaValue::Int(v)) if *v > 0 && v.is_power_of_two() => *v,
        _ => 32,
    };
    let pos = file.stream_position().map_err(|e| e.to_string())?;
    let data_offset = pos.next_multiple_of(alignment);

    Ok(GgufFile {
        meta,
        tensors,
        data_offset,
    })
}

/// Lee un tensor y devuelve (payload .som, dtype). F16 se expande a F32;
/// Q8_0 se reempaqueta (escala f16 → f32) sin pérdida.
fn read_tensor(
    file: &mut fs::File,
    gguf: &GgufFile,
    t: &TensorInfo,
) -> Result<(Vec<u8>, u8), String> {
    let elems: usize = t.shape.iter().product::<u64>() as usize;
    file.seek(SeekFrom::Start(gguf.data_offset + t.offset))
        .map_err(|e| e.to_string())?;
    match t.ggml_type {
        GGML_F32 => {
            let mut out = vec![0u8; elems * 4];
            file.read_exact(&mut out).map_err(|e| e.to_string())?;
            Ok((out, DTYPE_F32))
        }
        GGML_F16 => {
            let mut raw = vec![0u8; elems * 2];
            file.read_exact(&mut raw).map_err(|e| e.to_string())?;
            let mut out = vec![0u8; elems * 4];
            for (i, chunk) in out.chunks_mut(4).enumerate() {
                let bits = u16::from_le_bytes([raw[i * 2], raw[i * 2 + 1]]);
                chunk.copy_from_slice(&f16_to_f32(bits).to_le_bytes());
            }
            Ok((out, DTYPE_F32))
        }
        GGML_Q8_0 => {
            // bloque GGML: d f16 + 32 i8 (34 bytes) → bloque .som: f32 + 32 i8
            if elems % 32 != 0 {
                return Err(format!("tensor Q8_0 con {elems} elems (no múltiplo de 32)"));
            }
            let blocks = elems / 32;
            let mut raw = vec![0u8; blocks * 34];
            file.read_exact(&mut raw).map_err(|e| e.to_string())?;
            let mut out = Vec::with_capacity(blocks * 36);
            for b in raw.chunks_exact(34) {
                let d = f16_to_f32(u16::from_le_bytes([b[0], b[1]]));
                out.extend_from_slice(&d.to_le_bytes());
                out.extend_from_slice(&b[2..]);
            }
            Ok((out, DTYPE_Q8_0))
        }
        GGML_Q4_K => {
            // el layout .som de Q4_K es el de GGML tal cual: passthrough
            if elems % 256 != 0 {
                return Err(format!("tensor Q4_K con {elems} elems (no múltiplo de 256)"));
            }
            let mut out = vec![0u8; (elems / 256) * Q4_K_BLOCK_BYTES];
            file.read_exact(&mut out).map_err(|e| e.to_string())?;
            Ok((out, DTYPE_Q4_K))
        }
        GGML_Q6_K => {
            // sin soporte nativo Q6_K: descuantizar y re-cuantizar a Q8_0
            // (más precisión que el original; típico en output.weight de
            // los Q4_K_M)
            if elems % 256 != 0 {
                return Err(format!("tensor Q6_K con {elems} elems (no múltiplo de 256)"));
            }
            let blocks = elems / 256;
            let mut raw = vec![0u8; blocks * Q6_K_BLOCK_BYTES];
            file.read_exact(&mut raw).map_err(|e| e.to_string())?;
            let mut f = vec![0.0f32; elems];
            for (b, blk) in raw.chunks_exact(Q6_K_BLOCK_BYTES).enumerate() {
                dequant_q6_k_block(blk, &mut f[b * 256..(b + 1) * 256]);
            }
            Ok((quantize_q8_0(&f), DTYPE_Q8_0))
        }
        other => Err(format!(
            "dtype GGML {other} no soportado (solo F32/F16/Q8_0/Q4_K/Q6_K)"
        )),
    }
}

/// Descuantiza un bloque GGML Q6_K (256 elementos) — `dequantize_row_q6_K`.
fn dequant_q6_k_block(blk: &[u8], out: &mut [f32]) {
    let ql = &blk[0..128];
    let qh = &blk[128..192];
    let sc = &blk[192..208];
    let d = f16_to_f32(u16::from_le_bytes([blk[208], blk[209]]));
    for n in 0..2 {
        let ql = &ql[n * 64..];
        let qh = &qh[n * 32..];
        let sc = &sc[n * 8..];
        let y = &mut out[n * 128..];
        for l in 0..32 {
            let is = l / 16;
            let q1 = ((ql[l] & 0x0F) | ((qh[l] & 3) << 4)) as i32 - 32;
            let q2 = ((ql[l + 32] & 0x0F) | (((qh[l] >> 2) & 3) << 4)) as i32 - 32;
            let q3 = ((ql[l] >> 4) | (((qh[l] >> 4) & 3) << 4)) as i32 - 32;
            let q4 = ((ql[l + 32] >> 4) | (((qh[l] >> 6) & 3) << 4)) as i32 - 32;
            y[l] = d * (sc[is] as i8) as f32 * q1 as f32;
            y[l + 32] = d * (sc[is + 2] as i8) as f32 * q2 as f32;
            y[l + 64] = d * (sc[is + 4] as i8) as f32 * q3 as f32;
            y[l + 96] = d * (sc[is + 6] as i8) as f32 * q4 as f32;
        }
    }
}

use soso_llm_core::f16::f16_to_f32;

// Tipos de valor de metadatos GGUF:
// 0 u8, 1 i8, 2 u16, 3 i16, 4 u32, 5 i32, 6 f32, 7 bool(1B),
// 8 string, 9 array (elem_type u32 + count u64 + elems), 10 u64, 11 i64, 12 f64.
fn read_meta_value(file: &mut fs::File, vtype: u32) -> Result<MetaValue, String> {
    Ok(match vtype {
        0 | 1 | 7 => MetaValue::Int(read_bytes_le(file, 1)?),
        2 | 3 => MetaValue::Int(read_bytes_le(file, 2)?),
        4 | 5 => MetaValue::Int(read_bytes_le(file, 4)?),
        10 | 11 => MetaValue::Int(read_bytes_le(file, 8)?),
        6 => MetaValue::F32(f32::from_le_bytes(
            (read_bytes_le(file, 4)? as u32).to_le_bytes(),
        )),
        12 => {
            let bits = read_bytes_le(file, 8)?;
            MetaValue::F32(f64::from_le_bytes(bits.to_le_bytes()) as f32)
        }
        8 => MetaValue::Str(read_string(file)?),
        9 => {
            let elem_type = read_u32(file)?;
            let count = read_u64(file)?;
            if elem_type == 8 {
                let mut v = Vec::with_capacity(count.min(1 << 24) as usize);
                for _ in 0..count {
                    v.push(read_string(file)?);
                }
                MetaValue::StrArray(v)
            } else {
                for _ in 0..count {
                    skip_meta_value(file, elem_type)?;
                }
                MetaValue::Other
            }
        }
        other => return Err(format!("tipo meta desconocido {other}")),
    })
}

fn skip_meta_value(file: &mut fs::File, vtype: u32) -> Result<(), String> {
    read_meta_value(file, vtype).map(|_| ())
}

/// Lee `n` bytes little-endian como entero sin signo.
fn read_bytes_le(file: &mut fs::File, n: usize) -> Result<u64, String> {
    let mut b = [0u8; 8];
    file.read_exact(&mut b[..n]).map_err(|e| e.to_string())?;
    Ok(u64::from_le_bytes(b))
}

fn read_u32(file: &mut fs::File) -> Result<u32, String> {
    Ok(read_bytes_le(file, 4)? as u32)
}

fn read_u64(file: &mut fs::File) -> Result<u64, String> {
    read_bytes_le(file, 8)
}

fn read_string(file: &mut fs::File) -> Result<String, String> {
    let len = read_u64(file)? as usize;
    let mut buf = vec![0u8; len];
    file.read_exact(&mut buf).map_err(|e| e.to_string())?;
    String::from_utf8(buf).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn f16_conversion() {
        assert!((f16_to_f32(0) - 0.0).abs() < 1e-6);
        assert!((f16_to_f32(0x3c00) - 1.0).abs() < 1e-3);
        assert!((f16_to_f32(0xbc00) + 1.0).abs() < 1e-3); // -1.0
        assert!((f16_to_f32(0x3800) - 0.5).abs() < 1e-3);
        assert!(f16_to_f32(0x7c00).is_infinite());
        assert!(f16_to_f32(0x7e00).is_nan());
        // subnormal mínimo: 2^-24
        assert!((f16_to_f32(0x0001) - 2f32.powi(-24)).abs() < 1e-10);
    }

    // Construye un GGUF v3 mínimo (llama de 1 capa) y lo convierte.
    fn gguf_string(out: &mut Vec<u8>, s: &str) {
        out.extend_from_slice(&(s.len() as u64).to_le_bytes());
        out.extend_from_slice(s.as_bytes());
    }

    fn gguf_kv_u32(out: &mut Vec<u8>, key: &str, v: u32) {
        gguf_string(out, key);
        out.extend_from_slice(&4u32.to_le_bytes());
        out.extend_from_slice(&v.to_le_bytes());
    }

    fn gguf_kv_f32(out: &mut Vec<u8>, key: &str, v: f32) {
        gguf_string(out, key);
        out.extend_from_slice(&6u32.to_le_bytes());
        out.extend_from_slice(&v.to_le_bytes());
    }

    fn gguf_kv_bool(out: &mut Vec<u8>, key: &str, v: bool) {
        gguf_string(out, key);
        out.extend_from_slice(&7u32.to_le_bytes());
        out.push(v as u8);
    }

    fn gguf_kv_str_array(out: &mut Vec<u8>, key: &str, vals: &[&str]) {
        gguf_string(out, key);
        out.extend_from_slice(&9u32.to_le_bytes());
        out.extend_from_slice(&8u32.to_le_bytes());
        out.extend_from_slice(&(vals.len() as u64).to_le_bytes());
        for v in vals {
            gguf_string(out, v);
        }
    }

    #[test]
    fn q4k_passthrough_y_q6k_a_q8() {
        use soso_llm_core::f16::f32_to_f16;
        use soso_llm_core::quant::dequant_q8_0;

        let dir = std::env::temp_dir().join("convert-gguf-quant-test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        // Q4_K: un superbloque arbitrario debe pasar tal cual
        let q4: Vec<u8> = (0..Q4_K_BLOCK_BYTES).map(|i| (i * 13 % 251) as u8).collect();
        let p = dir.join("q4.bin");
        fs::File::create(&p).unwrap().write_all(&q4).unwrap();
        let gguf = GgufFile {
            meta: HashMap::new(),
            tensors: HashMap::new(),
            data_offset: 0,
        };
        let t = TensorInfo {
            shape: vec![256],
            ggml_type: GGML_Q4_K,
            offset: 0,
        };
        let mut f = fs::File::open(&p).unwrap();
        let (out, dtype) = read_tensor(&mut f, &gguf, &t).unwrap();
        assert_eq!(dtype, DTYPE_Q4_K);
        assert_eq!(out, q4);

        // Q6_K: ql/qh a cero y escalas 1 → todos los q = -32; con d = 1/32
        // cada elemento vale exactamente -1.0 (y sobrevive el paso a Q8_0)
        let mut q6 = vec![0u8; Q6_K_BLOCK_BYTES];
        for s in &mut q6[192..208] {
            *s = 1; // scales i8 = 1
        }
        q6[208..210].copy_from_slice(&f32_to_f16(1.0 / 32.0).to_le_bytes());
        let p6 = dir.join("q6.bin");
        fs::File::create(&p6).unwrap().write_all(&q6).unwrap();
        let t6 = TensorInfo {
            shape: vec![256],
            ggml_type: GGML_Q6_K,
            offset: 0,
        };
        let mut f6 = fs::File::open(&p6).unwrap();
        let (out6, dtype6) = read_tensor(&mut f6, &gguf, &t6).unwrap();
        assert_eq!(dtype6, DTYPE_Q8_0);
        let mut vals = vec![0.0f32; 256];
        dequant_q8_0(&out6, &mut vals).unwrap();
        for v in &vals {
            assert!((v + 1.0).abs() < 0.02, "esperaba -1.0, got {v}");
        }
    }

    #[test]
    fn convierte_gguf_sintetico() {
        const H: usize = 8;
        const FFN: usize = 16;
        const KV: usize = 4; // 1 cabeza kv de dim 4 (2 heads, head_dim 4)
        const VOCAB: usize = 6;

        let tensors: Vec<(&str, Vec<u64>)> = vec![
            // ne[]: dimensión rápida primero (in_dim, out_dim)
            ("token_embd.weight", vec![H as u64, VOCAB as u64]),
            ("output_norm.weight", vec![H as u64]),
            ("blk.0.attn_norm.weight", vec![H as u64]),
            ("blk.0.attn_q.weight", vec![H as u64, H as u64]),
            ("blk.0.attn_k.weight", vec![H as u64, KV as u64]),
            ("blk.0.attn_v.weight", vec![H as u64, KV as u64]),
            ("blk.0.attn_output.weight", vec![H as u64, H as u64]),
            ("blk.0.ffn_norm.weight", vec![H as u64]),
            ("blk.0.ffn_up.weight", vec![H as u64, FFN as u64]),
            ("blk.0.ffn_gate.weight", vec![H as u64, FFN as u64]),
            ("blk.0.ffn_down.weight", vec![FFN as u64, H as u64]),
        ];

        let mut g: Vec<u8> = Vec::new();
        g.extend_from_slice(b"GGUF");
        g.extend_from_slice(&3u32.to_le_bytes());
        g.extend_from_slice(&(tensors.len() as u64).to_le_bytes());
        g.extend_from_slice(&10u64.to_le_bytes()); // kv_count

        gguf_string(&mut g, "general.architecture");
        g.extend_from_slice(&8u32.to_le_bytes());
        gguf_string(&mut g, "llama");
        gguf_kv_u32(&mut g, "llama.embedding_length", H as u32);
        gguf_kv_u32(&mut g, "llama.block_count", 1);
        gguf_kv_u32(&mut g, "llama.feed_forward_length", FFN as u32);
        gguf_kv_u32(&mut g, "llama.attention.head_count", 2);
        gguf_kv_u32(&mut g, "llama.attention.head_count_kv", 1);
        gguf_kv_f32(&mut g, "llama.attention.layer_norm_rms_epsilon", 1e-6);
        // bool intercalado: el parser antiguo desincronizaba aquí
        gguf_kv_bool(&mut g, "tokenizer.ggml.add_bos_token", true);
        gguf_kv_str_array(
            &mut g,
            "tokenizer.ggml.tokens",
            &["<s>", "</s>", "\u{2581}a", "\u{2581}b", "c", "d"],
        );
        gguf_kv_u32(&mut g, "tokenizer.ggml.eos_token_id", 1);

        // tabla de tensores
        let mut offset = 0u64;
        for (name, shape) in &tensors {
            gguf_string(&mut g, name);
            g.extend_from_slice(&(shape.len() as u32).to_le_bytes());
            for d in shape {
                g.extend_from_slice(&d.to_le_bytes());
            }
            g.extend_from_slice(&GGML_F32.to_le_bytes());
            g.extend_from_slice(&offset.to_le_bytes());
            let elems: u64 = shape.iter().product();
            offset = (offset + elems * 4).next_multiple_of(32);
        }
        // datos alineados a 32
        while g.len() % 32 != 0 {
            g.push(0);
        }
        for (i, (_, shape)) in tensors.iter().enumerate() {
            let elems: u64 = shape.iter().product();
            for e in 0..elems {
                g.extend_from_slice(&((i as f32) + (e as f32) * 1e-3).to_le_bytes());
            }
            while g.len() % 32 != 0 {
                g.push(0);
            }
        }

        let dir = std::env::temp_dir().join("convert-gguf-test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let gguf_path = dir.join("mini.gguf");
        fs::File::create(&gguf_path)
            .unwrap()
            .write_all(&g)
            .unwrap();

        let out = dir.join("out");
        convert(gguf_path.to_str().unwrap(), &out, Some("mini")).unwrap();

        let manifest =
            Manifest::parse(&fs::read(out.join(MANIFEST_FILE)).unwrap()).unwrap();
        assert_eq!(manifest.hidden_dim, H as u32);
        assert_eq!(manifest.num_kv_heads, 1);
        assert_eq!(manifest.vocab_size, VOCAB as u32);
        assert!((manifest.rms_eps - 1e-6).abs() < 1e-9);

        let index = TensorIndex::parse(&fs::read(out.join(INDEX_FILE)).unwrap()).unwrap();
        // shapes invertidas a [filas, columnas]
        let embed = index.find("embed").unwrap();
        assert_eq!(embed.shape, vec![VOCAB as u32, H as u32]);
        let up = index.find("L00.ffn_up").unwrap();
        assert_eq!(up.shape, vec![FFN as u32, H as u32]);
        let k = index.find("L00.attn_k").unwrap();
        assert_eq!(k.shape, vec![KV as u32, H as u32]);
        assert!(index.find("L00.ffn_gate").is_some());
        assert!(index.find("output_norm").is_some());

        // el runtime valida las shapes del modelo convertido
        let rt = soso_llm_core::runtime::Runtime::new(manifest, index, 0, 0);
        rt.validate_shapes().expect("shapes coherentes");

        // tokenizer exportado y parseable
        let tok = soso_llm_core::tokenizer::Tokenizer::parse(
            &fs::read(out.join(TOKENIZER_FILE)).unwrap(),
        )
        .unwrap();
        assert_eq!(tok.eos(), Some(1));
    }
}
