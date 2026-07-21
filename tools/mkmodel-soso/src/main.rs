//! Genera un modelo .som sintético en un directorio host.
//!
//! Sin flags produce el modelo `tiny` de siempre; con `--hidden/--ffn/
//! --layers/--vocab/--heads/--seq` genera modelos de tamaño arbitrario
//! (decenas de GB) para medir la ruta de modelos grandes. Los shards se
//! escriben en streaming: no se materializa ningún tensor completo en RAM.

use sosomodel::index::{make_f32_entry, TensorIndex, SHARD_PAYLOAD_OFF};
use sosomodel::layout::{INDEX_FILE, MAGIC, MANIFEST_FILE, SHARDS_DIR};
use sosomodel::manifest::{LayerPrefetch, Manifest};
use sosomodel::{align_up, Crc32cDigest, BLOCK_ALIGN};
use std::fs;
use std::io::Write;
use std::path::PathBuf;

const CHUNK_ELEMS: usize = 1 << 20; // 4 MiB por pasada

/// Escribe un shard v2 (cabecera + payload alineado a 64 + padding a 4096)
/// en streaming, con dos pasadas sobre el generador (CRC y escritura).
fn write_f32_shard(path: &std::path::Path, elems: usize, fill: impl Fn(usize) -> f32) {
    let payload_len = elems * 4;
    let mut crc = Crc32cDigest::new();
    let mut buf = Vec::with_capacity(CHUNK_ELEMS * 4);
    for start in (0..elems).step_by(CHUNK_ELEMS) {
        buf.clear();
        for i in start..(start + CHUNK_ELEMS).min(elems) {
            buf.extend_from_slice(&fill(i).to_le_bytes());
        }
        crc.update(&buf);
    }
    let crc = crc.finalize();

    let mut f = std::io::BufWriter::new(fs::File::create(path).unwrap());
    let mut header = Vec::with_capacity(SHARD_PAYLOAD_OFF);
    header.extend_from_slice(&MAGIC);
    header.extend_from_slice(&crc.to_le_bytes());
    header.extend_from_slice(&2u32.to_le_bytes());
    header.extend_from_slice(&(payload_len as u64).to_le_bytes());
    header.resize(SHARD_PAYLOAD_OFF, 0);
    f.write_all(&header).unwrap();
    for start in (0..elems).step_by(CHUNK_ELEMS) {
        buf.clear();
        for i in start..(start + CHUNK_ELEMS).min(elems) {
            buf.extend_from_slice(&fill(i).to_le_bytes());
        }
        f.write_all(&buf).unwrap();
    }
    let total = SHARD_PAYLOAD_OFF + payload_len;
    let pad = align_up(total, BLOCK_ALIGN) - total;
    f.write_all(&vec![0u8; pad]).unwrap();
    f.flush().unwrap();
}

fn arg_u32(args: &[String], flag: &str, default: u32) -> u32 {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // posicional = dir de salida; el resto son parejas --flag valor
    let mut out = None;
    let mut i = 0;
    while i < args.len() {
        if args[i].starts_with("--") {
            i += 2;
        } else {
            if out.is_none() {
                out = Some(args[i].clone());
            }
            i += 1;
        }
    }
    let out = PathBuf::from(out.unwrap_or_else(|| "target/tiny-model".into()));

    let tiny = Manifest::tiny("tiny");
    let hidden = arg_u32(&args, "--hidden", tiny.hidden_dim);
    let ffn = arg_u32(&args, "--ffn", tiny.ffn_dim);
    let num_layers = arg_u32(&args, "--layers", tiny.num_layers);
    let vocab = arg_u32(&args, "--vocab", tiny.vocab_size);
    let heads = arg_u32(&args, "--heads", tiny.num_heads);
    let kv_heads = arg_u32(&args, "--kv-heads", heads);
    let max_seq = arg_u32(&args, "--seq", tiny.max_seq);

    let mut prefetch = Vec::new();
    for layer in 0..num_layers {
        prefetch.push(LayerPrefetch {
            layer,
            shards: vec![
                format!("L{layer:02}.attn_norm.tensor"),
                format!("L{layer:02}.attn_q.tensor"),
                format!("L{layer:02}.attn_k.tensor"),
                format!("L{layer:02}.attn_v.tensor"),
                format!("L{layer:02}.ffn_norm.tensor"),
                format!("L{layer:02}.ffn_up.tensor"),
                format!("L{layer:02}.ffn_down.tensor"),
            ],
        });
    }
    let manifest = Manifest {
        name: String::from("tiny"),
        vocab_size: vocab,
        hidden_dim: hidden,
        num_layers,
        num_heads: heads,
        num_kv_heads: kv_heads,
        ffn_dim: ffn,
        max_seq,
        rope_theta: 10000.0,
        rms_eps: 1e-5,
        prefetch,
    };

    let shards = out.join(SHARDS_DIR);
    fs::create_dir_all(&shards).expect("crear shards");
    fs::write(out.join(MANIFEST_FILE), manifest.serialize()).unwrap();

    let h = hidden;
    let head_dim = h / heads;
    let kv_dim = kv_heads * head_dim;
    let mut index = TensorIndex::default();
    let mut id = 0u32;
    let mut total_bytes = 0u64;

    for layer in 0..num_layers {
        let layer_u = layer;
        // convención [filas, columnas] = [out_dim, in_dim]
        let tensors: [(&str, Vec<u32>); 8] = [
            (&format!("L{layer:02}.attn_norm"), vec![h]),
            (&format!("L{layer:02}.attn_q"), vec![h, h]),
            (&format!("L{layer:02}.attn_k"), vec![kv_dim, h]),
            (&format!("L{layer:02}.attn_v"), vec![kv_dim, h]),
            (&format!("L{layer:02}.attn_output"), vec![h, h]),
            (&format!("L{layer:02}.ffn_norm"), vec![h]),
            (&format!("L{layer:02}.ffn_up"), vec![ffn, h]),
            (&format!("L{layer:02}.ffn_down"), vec![h, ffn]),
        ];
        for (base, shape) in tensors {
            let shard_name = format!("{base}.tensor");
            let elems: usize = shape.iter().map(|&d| d as usize).product();
            let is_norm = shape.len() == 1;
            write_f32_shard(&shards.join(&shard_name), elems, move |i| {
                if is_norm {
                    1.0f32
                } else {
                    ((i as u32).wrapping_mul(0x9e37_79b9) ^ layer_u) as f32 * 1e-9
                }
            });
            total_bytes += (elems * 4) as u64;
            index
                .entries
                .push(make_f32_entry(id, base, &shard_name, 0, &shape));
            id += 1;
        }
    }

    let emb_shape = [vocab, h];
    let emb_elems: usize = emb_shape.iter().map(|&d| d as usize).product();
    write_f32_shard(&shards.join("embed.tensor"), emb_elems, |i| (i as f32) * 1e-9);
    total_bytes += (emb_elems * 4) as u64;
    index
        .entries
        .push(make_f32_entry(id, "embed", "embed.tensor", 0, &emb_shape));
    id += 1;
    index
        .entries
        .push(make_f32_entry(id, "lm_head", "embed.tensor", 0, &emb_shape));

    fs::write(out.join(INDEX_FILE), index.serialize()).unwrap();
    println!(
        "mkmodel-soso: modelo en {} ({} tensores, {:.2} GiB de pesos)",
        out.display(),
        index.entries.len(),
        total_bytes as f64 / (1024.0 * 1024.0 * 1024.0)
    );
}
