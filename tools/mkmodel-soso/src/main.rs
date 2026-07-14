//! Genera un modelo .som de prueba en un directorio host.

use sosomodel::index::{make_f32_entry, pack_shard, TensorIndex};
use sosomodel::layout::{INDEX_FILE, MANIFEST_FILE, SHARDS_DIR};
use sosomodel::manifest::Manifest;
use std::fs;
use std::path::PathBuf;

fn main() {
    let out = std::env::args().nth(1).unwrap_or_else(|| "target/tiny-model".into());
    let out = PathBuf::from(out);
    let shards = out.join(SHARDS_DIR);
    fs::create_dir_all(&shards).expect("crear shards");

    let manifest = Manifest::tiny("tiny");
    fs::write(out.join(MANIFEST_FILE), manifest.serialize()).unwrap();

    let h = manifest.hidden_dim;
    let ffn = manifest.ffn_dim;
    let mut index = TensorIndex::default();
    let mut id = 0u32;

    for layer in 0..manifest.num_layers {
        let tensors = [
            (format!("L{layer:02}.attn_q"), vec![h, h]),
            (format!("L{layer:02}.attn_k"), vec![h, h]),
            (format!("L{layer:02}.attn_v"), vec![h, h]),
            (format!("L{layer:02}.ffn_up"), vec![h, ffn]),
            (format!("L{layer:02}.ffn_down"), vec![ffn, h]),
        ];
        for (base, shape) in tensors {
            let shard_name = format!("{base}.tensor");
            let elems: usize = shape.iter().map(|&d| d as usize).product();
            let mut raw = vec![0u8; elems * 4];
            for (i, chunk) in raw.chunks_mut(4).enumerate() {
                let v = ((i as u32).wrapping_mul(0x9e37_79b9) ^ layer) as f32 * 1e-4;
                chunk.copy_from_slice(&v.to_le_bytes());
            }
            let packed = pack_shard(&raw);
            let offset = 0u64;
            index.entries.push(make_f32_entry(
                id,
                &base,
                &shard_name,
                offset,
                &shape,
            ));
            id += 1;
            fs::write(shards.join(&shard_name), &packed).unwrap();
        }
    }

    // Embedding y lm_head
    let emb_shape = [manifest.vocab_size, h];
    let emb_elems: usize = emb_shape.iter().map(|&d| d as usize).product();
    let mut emb = vec![0u8; emb_elems * 4];
    for (i, c) in emb.chunks_mut(4).enumerate() {
        let v = (i as f32) * 1e-5;
        c.copy_from_slice(&v.to_le_bytes());
    }
    fs::write(shards.join("embed.tensor"), pack_shard(&emb)).unwrap();
    index.entries.push(make_f32_entry(id, "embed", "embed.tensor", 0, &emb_shape));
    id += 1;
    index
        .entries
        .push(make_f32_entry(id, "lm_head", "embed.tensor", 0, &emb_shape));

    fs::write(out.join(INDEX_FILE), index.serialize()).unwrap();
    println!(
        "mkmodel-soso: modelo tiny en {} ({} tensores)",
        out.display(),
        index.entries.len()
    );
}
