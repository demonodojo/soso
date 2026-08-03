//! Genera un modelo .som sintético en un directorio host.
//!
//! Sin flags produce el modelo `tiny` de siempre; con `--hidden/--ffn/
//! --layers/--vocab/--heads/--seq` genera modelos de tamaño arbitrario
//! (decenas de GB) para medir la ruta de modelos grandes. Los shards se
//! escriben en streaming: no se materializa ningún tensor completo en RAM.
//!
//! `--quant q8_0` o `--quant q4_k` generan los tensores 2D cuantizados (los `norm`
//! de una dimensión se quedan en F32, como en un modelo real). Sirve para probar el
//! camino de pesos cuantizados —el offload a GPU sólo admitía F32 y con un modelo
//! cuantizado no se usaba— y **rompe el streaming**: cuantizar necesita el tensor
//! entero en RAM, así que este modo es para modelos de prueba, no para los de
//! decenas de GB.

use sosomodel::index::{
    make_f32_entry, make_q4_k_entry, make_q8_0_entry, pack_shard, TensorIndex, SHARD_PAYLOAD_OFF,
};
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

/// Escribe un shard v2 cuyo payload ya está en memoria (el camino cuantizado).
fn write_bytes_shard(path: &std::path::Path, payload: &[u8]) {
    let mut crc = Crc32cDigest::new();
    crc.update(payload);
    let crc = crc.finalize();

    let mut f = std::io::BufWriter::new(fs::File::create(path).unwrap());
    let mut header = Vec::with_capacity(SHARD_PAYLOAD_OFF);
    header.extend_from_slice(&MAGIC);
    header.extend_from_slice(&crc.to_le_bytes());
    header.extend_from_slice(&2u32.to_le_bytes());
    header.extend_from_slice(&(payload.len() as u64).to_le_bytes());
    header.resize(SHARD_PAYLOAD_OFF, 0);
    f.write_all(&header).unwrap();
    f.write_all(payload).unwrap();
    let total = SHARD_PAYLOAD_OFF + payload.len();
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

fn arg_string(args: &[String], flag: &str, default: &str) -> String {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .cloned()
        .unwrap_or_else(|| default.into())
}

fn arg_bool(args: &[String], flag: &str) -> bool {
    args.iter().any(|a| a == flag)
}

fn emit_tensor_f32(
    shards: &std::path::Path,
    index: &mut TensorIndex,
    id: &mut u32,
    base: &str,
    shape: &[u32],
    layer_u: u32,
    expert_u: u32,
    total_bytes: &mut u64,
) {
    let shard_name = format!("{base}.tensor");
    let elems: usize = shape.iter().map(|&d| d as usize).product();
    let is_norm = shape.len() == 1;
    write_f32_shard(&shards.join(&shard_name), elems, move |i| {
        if is_norm {
            1.0f32
        } else {
            ((i as u32)
                .wrapping_mul(0x9e37_79b9)
                .wrapping_add(layer_u.wrapping_mul(0x100))
                .wrapping_add(expert_u.wrapping_mul(0x10))) as f32
                * 1e-9
        }
    });
    *total_bytes += (elems * 4) as u64;
    index
        .entries
        .push(make_f32_entry(*id, base, &shard_name, 0, shape));
    *id += 1;
}

fn f32_fill(layer_u: u32, expert_u: u32, is_norm: bool) -> impl Fn(usize) -> f32 {
    move |i: usize| {
        if is_norm {
            1.0f32
        } else {
            ((i as u32)
                .wrapping_mul(0x9e37_79b9)
                .wrapping_add(layer_u.wrapping_mul(0x100))
                .wrapping_add(expert_u.wrapping_mul(0x10))) as f32
                * 1e-9
        }
    }
}

fn append_f32_to_trunk(
    blob: &mut Vec<u8>,
    index: &mut TensorIndex,
    id: &mut u32,
    base: &str,
    shard: &str,
    shape: &[u32],
    layer_u: u32,
    expert_u: u32,
) -> u64 {
    let elems: usize = shape.iter().map(|&d| d as usize).product();
    let offset = blob.len() as u64;
    let is_norm = shape.len() == 1;
    let fill = f32_fill(layer_u, expert_u, is_norm);
    for i in 0..elems {
        blob.extend_from_slice(&fill(i).to_le_bytes());
    }
    index
        .entries
        .push(make_f32_entry(*id, base, shard, offset, shape));
    *id += 1;
    elems as u64 * 4
}

fn write_trunk_shard(path: &std::path::Path, blob: &[u8]) {
    write_bytes_shard(path, &pack_shard(blob));
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // posicional = dir de salida; el resto son parejas --flag valor
    let mut out = None;
    let mut i = 0;
    while i < args.len() {
        if args[i].starts_with("--") {
        if args[i] == "--moe" || args[i] == "--pack-trunk" {
            i += 1;
        } else if args[i] == "--attn" || args[i] == "--ffn-kind" {
            i += 2;
            } else {
                i += 2;
            }
        } else {
            if out.is_none() {
                out = Some(args[i].clone());
            }
            i += 1;
        }
    }
    let out = PathBuf::from(out.unwrap_or_else(|| {
        if arg_bool(&args, "--moe") {
            "target/tiny-moe-model".into()
        } else {
            "target/tiny-model".into()
        }
    }));

    let moe = arg_bool(&args, "--moe");
    let pack_trunk = arg_bool(&args, "--pack-trunk");
    let tiny = if moe {
        Manifest::tiny_moe("tiny-moe")
    } else {
        Manifest::tiny("tiny")
    };
    let default_name = if moe { "tiny-moe" } else { "tiny" };
    let name = arg_string(&args, "--name", default_name);
    let hidden = arg_u32(&args, "--hidden", tiny.hidden_dim);
    let ffn = arg_u32(&args, "--ffn", tiny.ffn_dim);
    let num_layers = arg_u32(&args, "--layers", tiny.num_layers);
    let vocab = arg_u32(&args, "--vocab", tiny.vocab_size);
    let heads = arg_u32(&args, "--heads", tiny.num_heads);
    let kv_heads = arg_u32(&args, "--kv-heads", heads);
    let max_seq = arg_u32(&args, "--seq", tiny.max_seq);
    let num_experts = if moe {
        arg_u32(&args, "--experts", tiny.num_experts)
    } else {
        0
    };
    let num_experts_per_tok = if moe {
        arg_u32(&args, "--experts-per-tok", tiny.num_experts_per_tok)
    } else {
        0
    };
    let moe_ffn = if moe {
        arg_u32(&args, "--moe-ffn", tiny.moe_ffn_dim)
    } else {
        0
    };
    let num_shared = if moe {
        arg_u32(&args, "--shared-experts", 0)
    } else {
        0
    };
    let attn_kind = arg_string(&args, "--attn", "gqa");
    let ffn_kind = arg_string(&args, "--ffn-kind", if moe { "moe" } else { "dense" });
    let quant = arg_string(&args, "--quant", "f32");
    if quant != "f32" && quant != "q8_0" && quant != "q4_k" {
        eprintln!("mkmodel-soso: --quant {quant} no soportado (f32 | q8_0 | q4_k)");
        std::process::exit(2);
    }
    let cuantizado = quant != "f32";
    // Q4_K empaqueta superbloques de 256 elementos y el matvec fusionado exige que
    // cada FILA sea un número entero de superbloques (`cols % 256 == 0`). Con
    // hidden=128 el modelo se genera sin protestar y luego la inferencia falla con
    // un `Err(())` mudo, así que se rechaza aquí y se dice por qué.
    let ffn_check = if moe { moe_ffn } else { ffn };
    if quant == "q4_k" && (hidden % 256 != 0 || ffn_check % 256 != 0) {
        eprintln!(
            "mkmodel-soso: --quant q4_k exige hidden y ffn/moe-ffn múltiplos de 256 \
             (hidden={hidden}, ffn={ffn_check})"
        );
        std::process::exit(2);
    }

    if pack_trunk && cuantizado {
        eprintln!("mkmodel-soso: --pack-trunk sólo con pesos f32 (sin --quant)");
        std::process::exit(2);
    }

    let mut prefetch = Vec::new();
    for layer in 0..num_layers {
        let shards = if pack_trunk {
            vec![format!("L{layer:02}.trunk.tensor")]
        } else {
            let mut shards = vec![
                format!("L{layer:02}.attn_norm.tensor"),
                format!("L{layer:02}.attn_q.tensor"),
                format!("L{layer:02}.attn_k.tensor"),
                format!("L{layer:02}.attn_v.tensor"),
                format!("L{layer:02}.attn_output.tensor"),
                format!("L{layer:02}.ffn_norm.tensor"),
            ];
            if moe {
                shards.push(format!("L{layer:02}.ffn_gate_inp.tensor"));
            } else {
                shards.push(format!("L{layer:02}.ffn_up.tensor"));
                shards.push(format!("L{layer:02}.ffn_down.tensor"));
            }
            shards
        };
        prefetch.push(LayerPrefetch { layer, shards });
    }
    let mut manifest = Manifest {
        name,
        vocab_size: vocab,
        hidden_dim: hidden,
        num_layers,
        num_heads: heads,
        num_kv_heads: kv_heads,
        ffn_dim: ffn,
        max_seq,
        rope_theta: 10000.0,
        rms_eps: 1e-5,
        num_experts,
        num_experts_per_tok,
        moe_ffn_dim: if moe { moe_ffn } else { 0 },
        layers: Vec::new(),
        prefetch,
    };
    manifest.fill_layers_from_globals();
    if num_shared > 0 {
        for layer in manifest.layers.iter_mut() {
            layer.num_shared_experts = num_shared;
        }
    }
    if attn_kind == "mla" {
        for layer in manifest.layers.iter_mut() {
            layer.attn_kind = sosomodel::AttnKind::Mla;
            layer.q_lora_rank = hidden / 4;
            layer.kv_lora_rank = hidden / 4;
        }
    } else if attn_kind == "kda" {
        for layer in manifest.layers.iter_mut() {
            layer.attn_kind = sosomodel::AttnKind::Kda;
        }
    }
    if ffn_kind == "latent-moe" {
        for layer in manifest.layers.iter_mut() {
            layer.ffn_kind = sosomodel::FfnKind::LatentMoe;
        }
    }

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
        let q_rank = hidden / 4;
        let kv_rank = hidden / 4;
        if pack_trunk {
            let shard_name = format!("L{layer:02}.trunk.tensor");
            let mut blob = Vec::new();
            if attn_kind == "mla" {
                for (base, shape) in [
                    (&format!("L{layer:02}.attn_norm"), vec![h]),
                    (&format!("L{layer:02}.attn_q_down"), vec![q_rank, h]),
                    (&format!("L{layer:02}.attn_q_up"), vec![h, q_rank]),
                    (&format!("L{layer:02}.attn_kv_down"), vec![kv_rank, h]),
                    (&format!("L{layer:02}.attn_k_up"), vec![kv_dim, kv_rank]),
                    (&format!("L{layer:02}.attn_v_up"), vec![kv_dim, kv_rank]),
                    (&format!("L{layer:02}.attn_output"), vec![h, h]),
                    (&format!("L{layer:02}.ffn_norm"), vec![h]),
                ] {
                    total_bytes += append_f32_to_trunk(
                        &mut blob, &mut index, &mut id, base, &shard_name, &shape, layer_u, 0,
                    );
                }
            } else {
                let attn_tensors: [(&str, Vec<u32>); 6] = [
                    (&format!("L{layer:02}.attn_norm"), vec![h]),
                    (&format!("L{layer:02}.attn_q"), vec![h, h]),
                    (&format!("L{layer:02}.attn_k"), vec![kv_dim, h]),
                    (&format!("L{layer:02}.attn_v"), vec![kv_dim, h]),
                    (&format!("L{layer:02}.attn_output"), vec![h, h]),
                    (&format!("L{layer:02}.ffn_norm"), vec![h]),
                ];
                for (base, shape) in attn_tensors {
                    total_bytes += append_f32_to_trunk(
                        &mut blob, &mut index, &mut id, base, &shard_name, &shape, layer_u, 0,
                    );
                }
            }
            if moe {
                total_bytes += append_f32_to_trunk(
                    &mut blob,
                    &mut index,
                    &mut id,
                    &format!("L{layer:02}.ffn_gate_inp"),
                    &shard_name,
                    &[num_experts, h],
                    layer_u,
                    0,
                );
            } else {
                for (base, shape) in [
                    (&format!("L{layer:02}.ffn_up"), vec![ffn, h]),
                    (&format!("L{layer:02}.ffn_down"), vec![h, ffn]),
                ] {
                    total_bytes += append_f32_to_trunk(
                        &mut blob,
                        &mut index,
                        &mut id,
                        base,
                        &shard_name,
                        &shape,
                        layer_u,
                        0,
                    );
                }
            }
            write_trunk_shard(&shards.join(&shard_name), &blob);
        } else if attn_kind == "mla" {
            for (base, shape) in [
                (&format!("L{layer:02}.attn_norm"), vec![h]),
                (&format!("L{layer:02}.attn_q_down"), vec![q_rank, h]),
                (&format!("L{layer:02}.attn_q_up"), vec![h, q_rank]),
                (&format!("L{layer:02}.attn_kv_down"), vec![kv_rank, h]),
                (&format!("L{layer:02}.attn_k_up"), vec![kv_dim, kv_rank]),
                (&format!("L{layer:02}.attn_v_up"), vec![kv_dim, kv_rank]),
                (&format!("L{layer:02}.attn_output"), vec![h, h]),
                (&format!("L{layer:02}.ffn_norm"), vec![h]),
            ] {
                emit_tensor_f32(
                    &shards, &mut index, &mut id, base, &shape, layer_u, 0, &mut total_bytes,
                );
            }
        } else {
            let attn_tensors: [(&str, Vec<u32>); 6] = [
                (&format!("L{layer:02}.attn_norm"), vec![h]),
                (&format!("L{layer:02}.attn_q"), vec![h, h]),
                (&format!("L{layer:02}.attn_k"), vec![kv_dim, h]),
                (&format!("L{layer:02}.attn_v"), vec![kv_dim, h]),
                (&format!("L{layer:02}.attn_output"), vec![h, h]),
                (&format!("L{layer:02}.ffn_norm"), vec![h]),
            ];
            for (base, shape) in attn_tensors {
                emit_tensor_f32(
                    &shards,
                    &mut index,
                    &mut id,
                    base,
                    &shape,
                    layer_u,
                    0,
                    &mut total_bytes,
                );
            }
        }
        if moe {
            if !pack_trunk {
                emit_tensor_f32(
                    &shards,
                    &mut index,
                    &mut id,
                    &format!("L{layer:02}.ffn_gate_inp"),
                    &[num_experts, h],
                    layer_u,
                    0,
                    &mut total_bytes,
                );
            }
            for expert in 0..num_experts {
                let ep = format!("L{layer:02}.E{expert:02}");
                emit_tensor_f32(
                    &shards,
                    &mut index,
                    &mut id,
                    &format!("{ep}.ffn_gate"),
                    &[moe_ffn, h],
                    layer_u,
                    expert,
                    &mut total_bytes,
                );
                emit_tensor_f32(
                    &shards,
                    &mut index,
                    &mut id,
                    &format!("{ep}.ffn_up"),
                    &[moe_ffn, h],
                    layer_u,
                    expert,
                    &mut total_bytes,
                );
                emit_tensor_f32(
                    &shards,
                    &mut index,
                    &mut id,
                    &format!("{ep}.ffn_down"),
                    &[h, moe_ffn],
                    layer_u,
                    expert,
                    &mut total_bytes,
                );
            }
            for shared in 0..num_shared {
                let sp = format!("L{layer:02}.S{shared:02}");
                emit_tensor_f32(
                    &shards,
                    &mut index,
                    &mut id,
                    &format!("{sp}.ffn_gate"),
                    &[moe_ffn, h],
                    layer_u,
                    shared,
                    &mut total_bytes,
                );
                emit_tensor_f32(
                    &shards,
                    &mut index,
                    &mut id,
                    &format!("{sp}.ffn_up"),
                    &[moe_ffn, h],
                    layer_u,
                    shared,
                    &mut total_bytes,
                );
                emit_tensor_f32(
                    &shards,
                    &mut index,
                    &mut id,
                    &format!("{sp}.ffn_down"),
                    &[h, moe_ffn],
                    layer_u,
                    shared,
                    &mut total_bytes,
                );
            }
        } else if !pack_trunk {
            let dense: [(&str, Vec<u32>); 2] = [
                (&format!("L{layer:02}.ffn_up"), vec![ffn, h]),
                (&format!("L{layer:02}.ffn_down"), vec![h, ffn]),
            ];
            for (base, shape) in dense {
                let shard_name = format!("{base}.tensor");
                let elems: usize = shape.iter().map(|&d| d as usize).product();
                let valor = move |i: usize| {
                    ((i as u32).wrapping_mul(0x9e37_79b9) ^ layer_u) as f32 * 1e-9
                };
                if cuantizado {
                    let plano: Vec<f32> = (0..elems).map(valor).collect();
                    let (bytes, entrada) = if quant == "q8_0" {
                        (
                            soso_llm_core::quant::quantize_q8_0(&plano),
                            make_q8_0_entry(id, base, &shard_name, 0, &shape),
                        )
                    } else {
                        (
                            soso_llm_core::quant::quantize_q4_k(&plano),
                            make_q4_k_entry(id, base, &shard_name, 0, &shape),
                        )
                    };
                    total_bytes += bytes.len() as u64;
                    write_bytes_shard(&shards.join(&shard_name), &bytes);
                    index.entries.push(entrada);
                } else {
                    write_f32_shard(&shards.join(&shard_name), elems, valor);
                    total_bytes += (elems * 4) as u64;
                    index
                        .entries
                        .push(make_f32_entry(id, base, &shard_name, 0, &shape));
                }
                id += 1;
            }
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
