//! Convierte GGUF (llama) al layout sosomodel (.som).

#![cfg_attr(not(feature = "std"), no_std)]
#[cfg(feature = "std")]
extern crate std;

extern crate alloc;
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

mod io;
pub use io::{Read, ReadSeek, Seek};

use soso_llm_core::quant::quantize_q8_0;
use soso_llm_core::tokenizer::{VocabTokenizer, NO_TOKEN};
use sosomodel::index::{make_f32_entry, make_mxfp4_entry, make_q4_k_entry, make_q8_0_entry, pack_shard, TensorIndex};
use sosomodel::layout::{
    DTYPE_F32, DTYPE_MXFP4, DTYPE_Q4_K, DTYPE_Q8_0, INDEX_FILE, MANIFEST_FILE, Q4_K_BLOCK_BYTES,
    SHARDS_DIR, TOKENIZER_FILE,
};
use sosomodel::manifest::{AttnKind, LayerPrefetch, Manifest};

pub trait SomOut {
    fn mkdir(&mut self, path: &str) -> Result<(), String>;
    fn write(&mut self, rel: &str, data: &[u8]) -> Result<(), String>;
}

/// Opciones de conversión (layout de shards).
#[derive(Clone, Copy, Debug, Default)]
pub struct ConvertOptions {
    /// Empaqueta attn+FFN denso+router por capa en `Lxx.trunk.tensor`.
    pub pack_trunk: bool,
}

impl ConvertOptions {
    pub fn with_pack_trunk(mut self, on: bool) -> Self {
        self.pack_trunk = on;
        self
    }
}

#[cfg(feature = "std")]
pub fn convert_path(gguf_path: &str, out_dir: &std::path::Path, name: Option<&str>) -> Result<(), String> {
    use crate::io::std_file::File as IoFile;
    struct StdOut {
        root: std::path::PathBuf,
    }
    impl SomOut for StdOut {
        fn mkdir(&mut self, path: &str) -> Result<(), String> {
            std::fs::create_dir_all(self.root.join(path)).map_err(|e| e.to_string())
        }
        fn write(&mut self, rel: &str, data: &[u8]) -> Result<(), String> {
            let p = self.root.join(rel);
            if let Some(parent) = p.parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            std::fs::write(p, data).map_err(|e| e.to_string())
        }
    }
    let mut f = IoFile(std::fs::File::open(gguf_path).map_err(|e| format!("abrir {gguf_path}: {e}"))?);
    let stem = name.map(String::from).or_else(|| {
        std::path::Path::new(gguf_path)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
    });
    let mut out = StdOut {
        root: out_dir.to_path_buf(),
    };
    convert_with_options(&mut f, &mut out, stem.as_deref(), ConvertOptions::default())
}

#[cfg(feature = "std")]
pub fn convert_path_with_options(
    gguf_path: &str,
    out_dir: &std::path::Path,
    name: Option<&str>,
    options: ConvertOptions,
) -> Result<(), String> {
    use crate::io::std_file::File as IoFile;
    struct StdOut {
        root: std::path::PathBuf,
    }
    impl SomOut for StdOut {
        fn mkdir(&mut self, path: &str) -> Result<(), String> {
            std::fs::create_dir_all(self.root.join(path)).map_err(|e| e.to_string())
        }
        fn write(&mut self, rel: &str, data: &[u8]) -> Result<(), String> {
            let p = self.root.join(rel);
            if let Some(parent) = p.parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            std::fs::write(p, data).map_err(|e| e.to_string())
        }
    }
    let mut f = IoFile(std::fs::File::open(gguf_path).map_err(|e| format!("abrir {gguf_path}: {e}"))?);
    let stem = name.map(String::from).or_else(|| {
        std::path::Path::new(gguf_path)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
    });
    let mut out = StdOut {
        root: out_dir.to_path_buf(),
    };
    convert_with_options(&mut f, &mut out, stem.as_deref(), options)
}

const GGML_F32: u32 = 0;
const GGML_F16: u32 = 1;
const GGML_Q8_0: u32 = 8;
const GGML_Q4_K: u32 = 12;
const GGML_Q6_K: u32 = 14;
/// MXFP4 en GGUF reciente (llama.cpp); passthrough sin re-cuantizar.
const GGML_MXFP4: u32 = 39;
/// Bloque GGML Q6_K: ql[128] + qh[64] + scales[16 i8] + d f16 = 210 bytes.
const Q6_K_BLOCK_BYTES: usize = 210;

pub fn convert<R: Read + Seek>(
    file: &mut R,
    out: &mut dyn SomOut,
    name: Option<&str>,
) -> Result<(), String> {
    convert_with_options(file, out, name, ConvertOptions::default())
}

pub fn convert_with_options<R: Read + Seek>(
    file: &mut R,
    out: &mut dyn SomOut,
    name: Option<&str>,
    options: ConvertOptions,
) -> Result<(), String> {
    let gguf = parse_gguf(file)?;

    let arch_name = gguf
        .meta
        .get("general.architecture")
        .and_then(|v| match v {
            MetaValue::Str(s) => Some(s.as_str()),
            _ => None,
        })
        .unwrap_or("llama");
    let is_mla = arch_name == "deepseek2";
    if arch_name != "llama" && !is_mla {
        return Err(format!(
            "arquitectura {arch_name} no soportada (llama | deepseek2)"
        ));
    }
    let meta_prefix = if is_mla { "deepseek2" } else { "llama" };

    let hidden = gguf.meta_u32(&format!("{meta_prefix}.embedding_length"))?;
    let num_layers = gguf.meta_u32(&format!("{meta_prefix}.block_count"))?;
    let ffn_dim = gguf.meta_u32(&format!("{meta_prefix}.feed_forward_length"))?;
    let num_heads = gguf.meta_u32(&format!("{meta_prefix}.attention.head_count"))?;
    let num_kv_heads = gguf
        .meta_u32(&format!("{meta_prefix}.attention.head_count_kv"))
        .unwrap_or(num_heads);
    let rope_theta = gguf
        .meta_f32(&format!("{meta_prefix}.rope.freq_base"))
        .unwrap_or(10000.0);
    let rms_eps = gguf
        .meta_f32(&format!("{meta_prefix}.attention.layer_norm_rms_epsilon"))
        .unwrap_or(1e-5);
    let q_lora_rank = if is_mla {
        gguf.meta_u32(&format!("{meta_prefix}.attention.q_lora_rank"))
            .unwrap_or(hidden / 4)
    } else {
        0
    };
    let kv_lora_rank = if is_mla {
        gguf.meta_u32(&format!("{meta_prefix}.attention.kv_lora_rank"))
            .unwrap_or(hidden / 4)
    } else {
        0
    };
    let tokens = match gguf.meta.get("tokenizer.ggml.tokens") {
        Some(MetaValue::StrArray(v)) => Some(v.clone()),
        _ => None,
    };
    let vocab = gguf
        .meta_u32(&format!("{meta_prefix}.vocab_size"))
        .or_else(|_| gguf.meta_u32("llama.vocab_size"))
        .ok()
        .or_else(|| tokens.as_ref().map(|t| t.len() as u32))
        .unwrap_or(256);
    let max_seq = gguf
        .meta_u32(&format!("{meta_prefix}.context_length"))
        .or_else(|_| gguf.meta_u32("llama.context_length"))
        .unwrap_or(2048);
    let num_experts = gguf
        .meta_u32(&format!("{meta_prefix}.expert_count"))
        .or_else(|_| gguf.meta_u32("llama.expert_count"))
        .unwrap_or(0);
    let num_experts_per_tok = gguf
        .meta_u32(&format!("{meta_prefix}.expert_used_count"))
        .or_else(|_| gguf.meta_u32("llama.expert_used_count"))
        .unwrap_or(if num_experts > 0 { 2 } else { 0 });
    let is_moe = num_experts > 0;
    let moe_ffn_dim = if is_moe { ffn_dim } else { 0 };

    let model_name = name.map(String::from).unwrap_or_else(|| String::from("model"));

    out.mkdir(SHARDS_DIR)?;

    // tensores por capa (GGUF → nombre .som)
    const LLAMA_LAYER_PARTS: &[(&str, &str)] = &[
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
    const MLA_LAYER_PARTS: &[(&str, &str)] = &[
        ("attn_norm", "attn_norm"),
        ("attn_q_a_proj", "attn_q_down"),
        ("attn_q_b_proj", "attn_q_up"),
        ("attn_kv_a_proj", "attn_kv_down"),
        ("attn_k_b_proj", "attn_k_up"),
        ("attn_v_b_proj", "attn_v_up"),
        ("attn_output", "attn_output"),
        ("ffn_norm", "ffn_norm"),
        ("ffn_up", "ffn_up"),
        ("ffn_gate", "ffn_gate"),
        ("ffn_down", "ffn_down"),
    ];
    let layer_parts = if is_mla {
        MLA_LAYER_PARTS
    } else {
        LLAMA_LAYER_PARTS
    };
    let globals = [
        ("token_embd.weight", "embed"),
        ("output.weight", "lm_head"),
        ("output_norm.weight", "output_norm"),
    ];

    let mut index = TensorIndex::default();
    let mut id = 0u32;
    let mut prefetch = Vec::new();
    let mut shared_by_layer: BTreeMap<u32, u32> = BTreeMap::new();

    struct TrunkPiece {
        som_name: String,
        payload: Vec<u8>,
        dtype: u8,
        shape: Vec<u32>,
    }

    let write_trunk_pack = |layer: u32,
                            pieces: &[TrunkPiece],
                            index: &mut TensorIndex,
                            id: &mut u32,
                            out: &mut dyn SomOut|
     -> Result<Vec<String>, String> {
        if pieces.is_empty() {
            return Ok(Vec::new());
        }
        let shard_name = format!("L{layer:02}.trunk.tensor");
        let mut blob = Vec::new();
        let mut offset = 0u64;
        for piece in pieces {
            index.entries.push(match piece.dtype {
                DTYPE_Q8_0 => make_q8_0_entry(
                    *id,
                    &piece.som_name,
                    &shard_name,
                    offset,
                    &piece.shape,
                ),
                DTYPE_Q4_K => make_q4_k_entry(
                    *id,
                    &piece.som_name,
                    &shard_name,
                    offset,
                    &piece.shape,
                ),
                _ => make_f32_entry(*id, &piece.som_name, &shard_name, offset, &piece.shape),
            });
            *id += 1;
            blob.extend_from_slice(&piece.payload);
            offset = offset.saturating_add(piece.payload.len() as u64);
        }
        out.write(
            &format!("{SHARDS_DIR}/{shard_name}"),
            &pack_shard(&blob),
        )?;
        Ok(alloc::vec![shard_name])
    };

    let emit = |file: &mut R,
                    index: &mut TensorIndex,
                    id: &mut u32,
                    gguf_name: &str,
                    som_name: &str,
                    out: &mut dyn SomOut|
     -> Result<bool, String> {
        let Some(t) = gguf.tensors.get(gguf_name) else {
            return Ok(false);
        };
        let (payload, dtype) = read_tensor(file, &gguf, t)?;
        let shard_name = format!("{som_name}.tensor");
        out.write(&format!("{SHARDS_DIR}/{shard_name}"), &pack_shard(&payload))?;
        // GGUF guarda ne[] con la dimensión rápida primero; sosomodel usa
        // row-major [filas, columnas], así que se invierte.
        let shape: Vec<u32> = t.shape.iter().rev().map(|&d| d as u32).collect();
        index.entries.push(match dtype {
            DTYPE_Q8_0 => make_q8_0_entry(*id, som_name, &shard_name, 0, &shape),
            DTYPE_Q4_K => make_q4_k_entry(*id, som_name, &shard_name, 0, &shape),
            DTYPE_MXFP4 => make_mxfp4_entry(*id, som_name, &shard_name, 0, &shape),
            _ => make_f32_entry(*id, som_name, &shard_name, 0, &shape),
        });
        *id += 1;
        Ok(true)
    };

    for layer in 0..num_layers {
        let mut shards = Vec::new();
        let mut trunk_pieces: Vec<TrunkPiece> = Vec::new();
        if is_moe {
            let attn_parts = [
                ("attn_norm", "attn_norm"),
                ("attn_q", "attn_q"),
                ("attn_k", "attn_k"),
                ("attn_v", "attn_v"),
                ("attn_output", "attn_output"),
                ("ffn_norm", "ffn_norm"),
            ];
            for (gguf_part, som_part) in attn_parts {
                let gguf_name = format!("blk.{layer}.{gguf_part}.weight");
                let som_name = format!("L{layer:02}.{som_part}");
                if options.pack_trunk {
                    if let Some(t) = gguf.tensors.get(&gguf_name) {
                        let (payload, dtype) = read_tensor(file, &gguf, t)?;
                        let shape: Vec<u32> = t.shape.iter().rev().map(|&d| d as u32).collect();
                        trunk_pieces.push(TrunkPiece {
                            som_name,
                            payload,
                            dtype,
                            shape,
                        });
                    }
                } else if emit(file, &mut index, &mut id, &gguf_name, &som_name, out)? {
                    shards.push(format!("{som_name}.tensor"));
                }
            }
            let router_gguf = format!("blk.{layer}.ffn_gate_inp.weight");
            let router_som = format!("L{layer:02}.ffn_gate_inp");
            if options.pack_trunk {
                if let Some(t) = gguf.tensors.get(&router_gguf) {
                    let (payload, dtype) = read_tensor(file, &gguf, t)?;
                    let shape: Vec<u32> = t.shape.iter().rev().map(|&d| d as u32).collect();
                    trunk_pieces.push(TrunkPiece {
                        som_name: router_som,
                        payload,
                        dtype,
                        shape,
                    });
                }
            } else if emit(
                file,
                &mut index,
                &mut id,
                &router_gguf,
                &router_som,
                out,
            )? {
                shards.push(format!("{router_som}.tensor"));
            }
            if options.pack_trunk {
                shards.extend(write_trunk_pack(
                    layer,
                    &trunk_pieces,
                    &mut index,
                    &mut id,
                    out,
                )?);
            }
            for (fused_suffix, split_part, som_suffix) in [
                ("ffn_gate_exps", "ffn_gate", "ffn_gate"),
                ("ffn_up_exps", "ffn_up", "ffn_up"),
                ("ffn_down_exps", "ffn_down", "ffn_down"),
            ] {
                let fused = emit_moe_experts(
                    file,
                    &gguf,
                    &mut index,
                    &mut id,
                    &format!("blk.{layer}.{fused_suffix}.weight"),
                    layer,
                    num_experts,
                    som_suffix,
                    out,
                )?;
                if fused {
                    continue;
                }
                let split = emit_moe_experts_split(
                    file,
                    &gguf,
                    &mut index,
                    &mut id,
                    layer,
                    num_experts,
                    split_part,
                    som_suffix,
                    out,
                )?;
                if !split {
                    return Err(format!(
                        "capa {layer}: faltan expertos {som_suffix} (ni blk.{layer}.{fused_suffix}.weight ni blk.{layer}.{split_part}.N.weight)"
                    ));
                }
            }
            for (gguf_suffix, som_suffix) in [
                ("ffn_gate_shexp", "ffn_gate"),
                ("ffn_up_shexp", "ffn_up"),
                ("ffn_down_shexp", "ffn_down"),
            ] {
                let n_shared = emit_moe_shared_experts(
                    file,
                    &gguf,
                    &mut index,
                    &mut id,
                    &format!("blk.{layer}.{gguf_suffix}.weight"),
                    layer,
                    som_suffix,
                    out,
                )?;
                if n_shared > 0 {
                    shared_by_layer.insert(layer, n_shared);
                }
            }
        } else {
            for (gguf_part, som_part) in layer_parts {
                let gguf_name = format!("blk.{layer}.{gguf_part}.weight");
                let som_name = format!("L{layer:02}.{som_part}");
                if options.pack_trunk {
                    if let Some(t) = gguf.tensors.get(&gguf_name) {
                        let (payload, dtype) = read_tensor(file, &gguf, t)?;
                        let shape: Vec<u32> = t.shape.iter().rev().map(|&d| d as u32).collect();
                        trunk_pieces.push(TrunkPiece {
                            som_name,
                            payload,
                            dtype,
                            shape,
                        });
                    }
                } else if emit(file, &mut index, &mut id, &gguf_name, &som_name, out)? {
                    shards.push(format!("{som_name}.tensor"));
                }
            }
            if options.pack_trunk {
                shards.extend(write_trunk_pack(
                    layer,
                    &trunk_pieces,
                    &mut index,
                    &mut id,
                    out,
                )?);
            }
        }
        prefetch.push(LayerPrefetch { layer, shards });
    }
    for (gguf_name, som_name) in globals {
        emit(file, &mut index, &mut id, gguf_name, som_name, out)?;
    }

    if index.find("embed").is_none() {
        return Err("el GGUF no contiene token_embd.weight".into());
    }

    let chat_template = plantilla_de_chat(&gguf);
    let mut manifest = Manifest {
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
        num_experts,
        num_experts_per_tok,
        moe_ffn_dim,
        layers: Vec::new(),
        prefetch,
        chat_template,
    };
    manifest.fill_layers_from_globals();
    if is_mla {
        for spec in manifest.layers.iter_mut() {
            spec.attn_kind = AttnKind::Mla;
            spec.q_lora_rank = q_lora_rank;
            spec.kv_lora_rank = kv_lora_rank;
        }
    }
    for (layer, n) in shared_by_layer {
        if let Some(spec) = manifest.layers.get_mut(layer as usize) {
            spec.num_shared_experts = n;
        }
    }
    out.write(MANIFEST_FILE, &manifest.serialize())?;
    out.write(INDEX_FILE, &index.serialize())?;

    if let Some(pieces) = tokens {
        let bos = gguf.meta_u32("tokenizer.ggml.bos_token_id").unwrap_or(NO_TOKEN);
        let eos = gguf.meta_u32("tokenizer.ggml.eos_token_id").unwrap_or(NO_TOKEN);
        out.write(
            TOKENIZER_FILE,
            &VocabTokenizer::serialize(&pieces, bos, eos),
        )?;
    }
    Ok(())
}

/// Trocea un tensor 3D MoE `[n_expert, rows, cols]` en shards 2D por experto.
/// Devuelve `false` si el tensor fusionado no está (GGUF Mixtral antiguo).
fn emit_moe_experts<R: Read + Seek>(
    file: &mut R,
    gguf: &GgufFile,
    index: &mut TensorIndex,
    id: &mut u32,
    gguf_name: &str,
    layer: u32,
    num_experts: u32,
    som_suffix: &str,
    out: &mut dyn SomOut,
) -> Result<bool, String> {
    let Some(t) = gguf.tensors.get(gguf_name) else {
        return Ok(false);
    };
    if t.shape.len() != 3 {
        return Err(format!("{gguf_name}: se esperaban 3 dimensiones MoE"));
    }
    let (payload, dtype) = read_tensor(file, gguf, t)?;
    // GGUF ne[] rápido primero → sosomodel [n_expert, rows, cols].
    let shape: Vec<u32> = t.shape.iter().rev().map(|&d| d as u32).collect();
    let n_exp = shape[0] as usize;
    let rows = shape[1] as usize;
    let cols = shape[2] as usize;
    if n_exp as u32 != num_experts {
        return Err(format!(
            "{gguf_name}: {n_exp} expertos en tensor vs {num_experts} en meta"
        ));
    }
    for expert in 0..num_experts {
        let slice = slice_expert_3d(&payload, dtype, rows, cols, expert as usize)?;
        let som_name = format!("L{layer:02}.E{expert:02}.{som_suffix}");
        let shard_name = format!("{som_name}.tensor");
        out.write(&format!("{SHARDS_DIR}/{shard_name}"), &pack_shard(&slice))?;
        let exp_shape = vec![rows as u32, cols as u32];
        index.entries.push(match dtype {
            DTYPE_Q8_0 => make_q8_0_entry(*id, &som_name, &shard_name, 0, &exp_shape),
            DTYPE_Q4_K => make_q4_k_entry(*id, &som_name, &shard_name, 0, &exp_shape),
            _ => make_f32_entry(*id, &som_name, &shard_name, 0, &exp_shape),
        });
        *id += 1;
    }
    Ok(true)
}

/// Expertos Mixtral del GGUF antiguo: `blk.L.ffn_{gate,up,down}.E.weight` (2D).
fn emit_moe_experts_split<R: Read + Seek>(
    file: &mut R,
    gguf: &GgufFile,
    index: &mut TensorIndex,
    id: &mut u32,
    layer: u32,
    num_experts: u32,
    gguf_part: &str,
    som_suffix: &str,
    out: &mut dyn SomOut,
) -> Result<bool, String> {
    let first = format!("blk.{layer}.{gguf_part}.0.weight");
    if !gguf.tensors.contains_key(&first) {
        return Ok(false);
    }
    for expert in 0..num_experts {
        let gguf_name = format!("blk.{layer}.{gguf_part}.{expert}.weight");
        let Some(t) = gguf.tensors.get(&gguf_name) else {
            return Err(format!(
                "{gguf_name}: falta (se esperaban {num_experts} expertos)"
            ));
        };
        if t.shape.len() != 2 {
            return Err(format!("{gguf_name}: se esperaban 2 dimensiones"));
        }
        let (payload, dtype) = read_tensor(file, gguf, t)?;
        let shape: Vec<u32> = t.shape.iter().rev().map(|&d| d as u32).collect();
        let som_name = format!("L{layer:02}.E{expert:02}.{som_suffix}");
        let shard_name = format!("{som_name}.tensor");
        out.write(&format!("{SHARDS_DIR}/{shard_name}"), &pack_shard(&payload))?;
        index.entries.push(match dtype {
            DTYPE_Q8_0 => make_q8_0_entry(*id, &som_name, &shard_name, 0, &shape),
            DTYPE_Q4_K => make_q4_k_entry(*id, &som_name, &shard_name, 0, &shape),
            _ => make_f32_entry(*id, &som_name, &shard_name, 0, &shape),
        });
        *id += 1;
    }
    Ok(true)
}

/// Trocea expertos compartidos MoE (`ffn_*_shexp`) en `Lxx.Syy.*`.
fn emit_moe_shared_experts<R: Read + Seek>(
    file: &mut R,
    gguf: &GgufFile,
    index: &mut TensorIndex,
    id: &mut u32,
    gguf_name: &str,
    layer: u32,
    som_suffix: &str,
    out: &mut dyn SomOut,
) -> Result<u32, String> {
    let Some(t) = gguf.tensors.get(gguf_name) else {
        return Ok(0);
    };
    if t.shape.len() != 3 {
        return Err(format!("{gguf_name}: se esperaban 3 dimensiones shared MoE"));
    }
    let (payload, dtype) = read_tensor(file, gguf, t)?;
    let shape: Vec<u32> = t.shape.iter().rev().map(|&d| d as u32).collect();
    let n_exp = shape[0] as usize;
    let rows = shape[1] as usize;
    let cols = shape[2] as usize;
    for shared in 0..n_exp {
        let slice = slice_expert_3d(&payload, dtype, rows, cols, shared)?;
        let som_name = format!("L{layer:02}.S{shared:02}.{som_suffix}");
        let shard_name = format!("{som_name}.tensor");
        out.write(&format!("{SHARDS_DIR}/{shard_name}"), &pack_shard(&slice))?;
        let exp_shape = vec![rows as u32, cols as u32];
        index.entries.push(match dtype {
            DTYPE_Q8_0 => make_q8_0_entry(*id, &som_name, &shard_name, 0, &exp_shape),
            DTYPE_Q4_K => make_q4_k_entry(*id, &som_name, &shard_name, 0, &exp_shape),
            _ => make_f32_entry(*id, &som_name, &shard_name, 0, &exp_shape),
        });
        *id += 1;
    }
    Ok(n_exp as u32)
}

fn slice_expert_3d(
    payload: &[u8],
    dtype: u8,
    rows: usize,
    cols: usize,
    expert: usize,
) -> Result<Vec<u8>, String> {
    match dtype {
        DTYPE_F32 => {
            let row_bytes = cols * 4;
            let expert_bytes = rows * row_bytes;
            let off = expert * expert_bytes;
            let end = off + expert_bytes;
            payload
                .get(off..end)
                .map(|s| s.to_vec())
                .ok_or_else(|| "slice experto F32 fuera de rango".into())
        }
        DTYPE_Q8_0 => {
            if cols % 32 != 0 {
                return Err(format!("Q8_0 MoE: cols={cols} no múltiplo de 32"));
            }
            let row_bytes = (cols / 32) * 36;
            let expert_bytes = rows * row_bytes;
            let off = expert * expert_bytes;
            payload
                .get(off..off + expert_bytes)
                .map(|s| s.to_vec())
                .ok_or_else(|| "slice experto Q8_0 fuera de rango".into())
        }
        DTYPE_Q4_K => {
            if cols % 256 != 0 {
                return Err(format!("Q4_K MoE: cols={cols} no múltiplo de 256"));
            }
            let row_bytes = (cols / 256) * Q4_K_BLOCK_BYTES;
            let expert_bytes = rows * row_bytes;
            let off = expert * expert_bytes;
            payload
                .get(off..off + expert_bytes)
                .map(|s| s.to_vec())
                .ok_or_else(|| "slice experto Q4_K fuera de rango".into())
        }
        other => Err(format!("dtype {other} no soportado en slice MoE")),
    }
}

struct TensorInfo {
    shape: Vec<u64>,
    ggml_type: u32,
    offset: u64,
}

struct GgufFile {
    meta: BTreeMap<String, MetaValue>,
    tensors: BTreeMap<String, TensorInfo>,
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

    fn meta_str(&self, key: &str) -> Option<&str> {
        match self.meta.get(key) {
            Some(MetaValue::Str(s)) => Some(s.as_str()),
            _ => None,
        }
    }
}

/// Traduce la plantilla de chat del GGUF a la forma con marcadores que entiende
/// `soso_llm_core::chat` (`{prompt}` y `{eos}`).
///
/// El GGUF trae la plantilla **en Jinja** (`tokenizer.chat_template`: 410 B en el
/// de TinyLlama). Aquí no se evalúa Jinja ni se pretende: se reconoce a qué
/// familia pertenece por los marcadores que usa —que es lo que de verdad
/// distingue una de otra— y se emite la forma equivalente de un solo turno con
/// el prompt de generación al final.
///
/// Lo que no se reconoce vuelve vacío, y entonces el modelo se usa con el texto
/// crudo (lo de siempre). Se avisa al convertir, que es el único momento en que
/// alguien puede hacer algo al respecto; sin el aviso, un modelo de chat nuevo
/// contestaría ensalada de palabras sin una sola pista de por qué.
fn traducir_plantilla(jinja: &str) -> Option<&'static str> {
    if jinja.contains("<|im_start|>") {
        // ChatML (Qwen, muchos finetunes). `<|im_end|>` es un token añadido y
        // está en la lista de tokens del GGUF, así que el emparejado más largo
        // del tokenizador lo encuentra entero.
        return Some("<|im_start|>user\n{prompt}<|im_end|>\n<|im_start|>assistant\n");
    }
    if jinja.contains("<|user|>") && jinja.contains("<|assistant|>") {
        // Zephyr / TinyLlama-Chat. El fin de turno del usuario es el token EOS.
        return Some("<|user|>\n{prompt}{eos}\n<|assistant|>\n");
    }
    if jinja.contains("[INST]") {
        // Llama-2 chat.
        return Some("[INST] {prompt} [/INST]");
    }
    None
}

fn plantilla_de_chat(gguf: &GgufFile) -> String {
    let Some(jinja) = gguf.meta_str("tokenizer.chat_template") else {
        return String::new();
    };
    // El aviso sólo en el host: este crate también se compila `no_std` para el
    // `soso-hf pull` de dentro de soso, y ahí no hay stderr. Quien convierta en la
    // placa lo ve por otro lado: `soso-llm run --chat` dice si no hay plantilla.
    match traducir_plantilla(jinja) {
        Some(p) => {
            #[cfg(feature = "std")]
            std::eprintln!("gguf2som: plantilla de chat reconocida → {p:?}");
            p.to_string()
        }
        None => {
            #[cfg(feature = "std")]
            std::eprintln!(
                "gguf2som: el modelo trae plantilla de chat pero no reconozco su \
                 familia; se usará el texto crudo. Ponla a mano en /etc/llm.conf \
                 (plantilla=…) con los marcadores {{prompt}} y {{eos}}"
            );
            String::new()
        }
    }
}

fn parse_gguf<R: Read + Seek>(file: &mut R) -> Result<GgufFile, String> {
    let mut magic = [0u8; 4];
    file.read_exact(&mut magic)?;
    if &magic != b"GGUF" {
        return Err("magic GGUF inválido".into());
    }
    let version = read_u32(file)?;
    if version < 2 {
        return Err(format!("versión GGUF {version} no soportada"));
    }
    let tensor_count = read_u64(file)?;
    let kv_count = read_u64(file)?;

    let mut meta = BTreeMap::new();
    for _ in 0..kv_count {
        let key = read_string(file)?;
        let vtype = read_u32(file)?;
        let val = read_meta_value(file, vtype)?;
        meta.insert(key, val);
    }

    let mut tensors = BTreeMap::new();
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
    let pos = file.stream_position()?;
    let data_offset = pos.next_multiple_of(alignment);

    Ok(GgufFile {
        meta,
        tensors,
        data_offset,
    })
}

/// Lee un tensor y devuelve (payload .som, dtype). F16 se expande a F32;
/// Q8_0 se reempaqueta (escala f16 → f32) sin pérdida.
fn read_tensor<R: Read + Seek>(
    file: &mut R,
    gguf: &GgufFile,
    t: &TensorInfo,
) -> Result<(Vec<u8>, u8), String> {
    let elems: usize = t.shape.iter().product::<u64>() as usize;
    file.seek_start(gguf.data_offset + t.offset as u64)
        .map_err(|e| e.to_string())?;
    match t.ggml_type {
        GGML_F32 => {
            let mut out = vec![0u8; elems * 4];
            file.read_exact(&mut out)?;
            Ok((out, DTYPE_F32))
        }
        GGML_F16 => {
            let mut raw = vec![0u8; elems * 2];
            file.read_exact(&mut raw)?;
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
            file.read_exact(&mut raw)?;
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
            file.read_exact(&mut out)?;
            Ok((out, DTYPE_Q4_K))
        }
        GGML_MXFP4 => {
            use sosomodel::layout::{MXFP4_BLOCK_BYTES, MXFP4_BLOCK_ELEMS};
            if elems % MXFP4_BLOCK_ELEMS != 0 {
                return Err(format!(
                    "tensor MXFP4 con {elems} elems (no múltiplo de {MXFP4_BLOCK_ELEMS})"
                ));
            }
            let mut out = vec![0u8; (elems / MXFP4_BLOCK_ELEMS) * MXFP4_BLOCK_BYTES];
            file.read_exact(&mut out)?;
            Ok((out, DTYPE_MXFP4))
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
            file.read_exact(&mut raw)?;
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
fn read_meta_value<R: Read + Seek>(file: &mut R, vtype: u32) -> Result<MetaValue, String> {
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

fn skip_meta_value<R: Read + Seek>(file: &mut R, vtype: u32) -> Result<(), String> {
    read_meta_value(file, vtype).map(|_| ())
}

/// Lee `n` bytes little-endian como entero sin signo.
fn read_bytes_le<R: Read + Seek>(file: &mut R, n: usize) -> Result<u64, String> {
    let mut b = [0u8; 8];
    file.read_exact(&mut b[..n])?;
    Ok(u64::from_le_bytes(b))
}

fn read_u32<R: Read + Seek>(file: &mut R) -> Result<u32, String> {
    Ok(read_bytes_le(file, 4)? as u32)
}

fn read_u64<R: Read + Seek>(file: &mut R) -> Result<u64, String> {
    read_bytes_le(file, 8)
}

fn read_string<R: Read + Seek>(file: &mut R) -> Result<String, String> {
    let len = read_u64(file)? as usize;
    let mut buf = vec![0u8; len];
    file.read_exact(&mut buf)?;
    String::from_utf8(buf).map_err(|e| e.to_string())
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use alloc::collections::BTreeMap;
    use super::*;
    use std::fs;
    use std::io::Write;
    use crate::io::std_file::File as IoFile;

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

    fn gguf_kv_str(out: &mut Vec<u8>, key: &str, v: &str) {
        gguf_string(out, key);
        out.extend_from_slice(&8u32.to_le_bytes());
        gguf_string(out, v);
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
            meta: BTreeMap::new(),
            tensors: BTreeMap::new(),
            data_offset: 0,
        };
        let t = TensorInfo {
            shape: vec![256],
            ggml_type: GGML_Q4_K,
            offset: 0,
        };
        let mut f = IoFile(fs::File::open(&p).unwrap());
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
        let mut f6 = IoFile(fs::File::open(&p6).unwrap());
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
        g.extend_from_slice(&11u64.to_le_bytes()); // kv_count

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
        // La plantilla de chat viaja en el GGUF y tiene que acabar en el
        // manifiesto: es lo que hace que `ask` no mande el texto pelado a un
        // modelo de chat. Ésta es la de TinyLlama recortada a lo que se reconoce.
        gguf_kv_str(
            &mut g,
            "tokenizer.chat_template",
            "{% if message['role'] == 'user' %}{{ '<|user|>\n' + message['content'] \
             + eos_token }}{% elif message['role'] == 'assistant' %}{{ '<|assistant|>\n' }}",
        );

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
        convert_path(gguf_path.to_str().unwrap(), &out, Some("mini")).unwrap();

        let manifest =
            Manifest::parse(&fs::read(out.join(MANIFEST_FILE)).unwrap()).unwrap();
        assert_eq!(manifest.hidden_dim, H as u32);
        assert_eq!(manifest.num_kv_heads, 1);
        assert_eq!(manifest.vocab_size, VOCAB as u32);
        assert!((manifest.rms_eps - 1e-6).abs() < 1e-9);
        assert_eq!(
            manifest.chat_template, "<|user|>\n{prompt}{eos}\n<|assistant|>\n",
            "la plantilla del GGUF tiene que llegar al manifiesto"
        );

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

    #[test]
    fn familias_de_plantilla_reconocidas() {
        // La de TinyLlama, tal cual sale de su GGUF (410 B de Jinja).
        let tinyllama = "{% for message in messages %}\n{% if message['role'] == 'user' %}\n\
            {{ '<|user|>\n' + message['content'] + eos_token }}\n{% elif message['role'] \
            == 'system' %}\n{{ '<|system|>\n' + message['content'] + eos_token }}\n{% elif \
            message['role'] == 'assistant' %}\n{{ '<|assistant|>\n'  + message['content'] \
            + eos_token }}\n{% endif %}\n{% if loop.last and add_generation_prompt %}\n\
            {{ '<|assistant|>' }}\n{% endif %}\n{% endfor %}";
        assert_eq!(
            traducir_plantilla(tinyllama),
            Some("<|user|>\n{prompt}{eos}\n<|assistant|>\n")
        );
        assert_eq!(
            traducir_plantilla("{{ '<|im_start|>' + message['role'] }}"),
            Some("<|im_start|>user\n{prompt}<|im_end|>\n<|im_start|>assistant\n")
        );
        assert_eq!(
            traducir_plantilla("{{ '[INST] ' + message['content'] + ' [/INST]' }}"),
            Some("[INST] {prompt} [/INST]")
        );
        // Familia desconocida: vacía y a texto crudo, nunca una plantilla
        // inventada — meterle marcadores que el modelo no vio es peor que nada.
        assert_eq!(traducir_plantilla("{{ raro }}"), None);
    }

    #[test]
    fn convierte_deepseek2_mla_sintetico() {
        const H: usize = 8;
        const FFN: usize = 16;
        const RANK: usize = 4;
        const KV_DIM: usize = 4;
        const VOCAB: usize = 6;

        let tensors: Vec<(&str, Vec<u64>)> = vec![
            ("token_embd.weight", vec![H as u64, VOCAB as u64]),
            ("output_norm.weight", vec![H as u64]),
            ("blk.0.attn_norm.weight", vec![H as u64]),
            ("blk.0.attn_q_a_proj.weight", vec![H as u64, RANK as u64]),
            ("blk.0.attn_q_b_proj.weight", vec![RANK as u64, H as u64]),
            ("blk.0.attn_kv_a_proj.weight", vec![H as u64, RANK as u64]),
            ("blk.0.attn_k_b_proj.weight", vec![RANK as u64, KV_DIM as u64]),
            ("blk.0.attn_v_b_proj.weight", vec![RANK as u64, KV_DIM as u64]),
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
        g.extend_from_slice(&12u64.to_le_bytes());

        gguf_string(&mut g, "general.architecture");
        g.extend_from_slice(&8u32.to_le_bytes());
        gguf_string(&mut g, "deepseek2");
        gguf_kv_u32(&mut g, "deepseek2.embedding_length", H as u32);
        gguf_kv_u32(&mut g, "deepseek2.block_count", 1);
        gguf_kv_u32(&mut g, "deepseek2.feed_forward_length", FFN as u32);
        gguf_kv_u32(&mut g, "deepseek2.attention.head_count", 2);
        gguf_kv_u32(&mut g, "deepseek2.attention.head_count_kv", 1);
        gguf_kv_u32(&mut g, "deepseek2.attention.q_lora_rank", RANK as u32);
        gguf_kv_u32(&mut g, "deepseek2.attention.kv_lora_rank", RANK as u32);
        gguf_kv_f32(&mut g, "deepseek2.attention.layer_norm_rms_epsilon", 1e-6);
        gguf_kv_bool(&mut g, "tokenizer.ggml.add_bos_token", true);
        gguf_kv_str_array(
            &mut g,
            "tokenizer.ggml.tokens",
            &["<s>", "</s>", "a", "b", "c", "d"],
        );
        gguf_kv_u32(&mut g, "tokenizer.ggml.eos_token_id", 1);

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

        let dir = std::env::temp_dir().join("convert-gguf-ds2-test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let gguf_path = dir.join("ds2.gguf");
        fs::File::create(&gguf_path)
            .unwrap()
            .write_all(&g)
            .unwrap();

        let out = dir.join("out");
        convert_path(gguf_path.to_str().unwrap(), &out, Some("ds2-mla")).unwrap();

        let manifest =
            Manifest::parse(&fs::read(out.join(MANIFEST_FILE)).unwrap()).unwrap();
        assert_eq!(manifest.layers[0].attn_kind, AttnKind::Mla);
        assert_eq!(manifest.layers[0].q_lora_rank, RANK as u32);
        assert_eq!(manifest.layers[0].kv_lora_rank, RANK as u32);

        let index = TensorIndex::parse(&fs::read(out.join(INDEX_FILE)).unwrap()).unwrap();
        assert!(index.find("L00.attn_q_down").is_some());
        assert!(index.find("L00.attn_kv_down").is_some());
        let rt = soso_llm_core::runtime::Runtime::new(manifest.clone(), index.clone(), 0, 0);
        rt.validate_shapes().expect("deepseek2 MLA shapes");

        use soso_llm_core::source::{FileMapper, MappedShard, MmapTensorSource};

        struct DirMapper {
            root: std::path::PathBuf,
        }

        impl FileMapper for DirMapper {
            fn map_file(&mut self, path: &str) -> Result<MappedShard, ()> {
                let data = std::fs::read(self.root.join(path)).map_err(|_| ())?;
                let len = data.len();
                let ptr = Box::leak(data.into_boxed_slice()).as_ptr();
                Ok(MappedShard {
                    addr: ptr as u64,
                    len,
                })
            }

            fn unmap_file(&mut self, _shard: &MappedShard) {}
        }

        let shard_root = out.join("shards");
        let base = shard_root.to_string_lossy().into_owned();
        let mut source = MmapTensorSource::new(
            base,
            index.clone(),
            DirMapper {
                root: shard_root,
            },
        );
        let mut rt = soso_llm_core::runtime::Runtime::new(manifest, index, 0, 0);
        rt.embed_token(0, &mut source).expect("embed");
        rt.forward_layers_range(0, rt.manifest.num_layers, &mut source, None, &mut None)
            .expect("forward 1 token");
        let logits = rt.logits(&mut source).expect("logits");
        assert!(logits.iter().all(|x| x.is_finite()));
    }

    fn pack_gguf_v3(
        kv_count: u64,
        write_kv: impl FnOnce(&mut Vec<u8>),
        tensors: &[(&str, Vec<u64>)],
    ) -> Vec<u8> {
        let mut g: Vec<u8> = Vec::new();
        g.extend_from_slice(b"GGUF");
        g.extend_from_slice(&3u32.to_le_bytes());
        g.extend_from_slice(&(tensors.len() as u64).to_le_bytes());
        g.extend_from_slice(&kv_count.to_le_bytes());
        write_kv(&mut g);
        let mut offset = 0u64;
        for (name, shape) in tensors {
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
        g
    }

    fn llama_moe_kv(g: &mut Vec<u8>, h: u32, ffn: u32, n_exp: u32) {
        gguf_kv_str(g, "general.architecture", "llama");
        gguf_kv_u32(g, "llama.embedding_length", h);
        gguf_kv_u32(g, "llama.block_count", 1);
        gguf_kv_u32(g, "llama.feed_forward_length", ffn);
        gguf_kv_u32(g, "llama.attention.head_count", 2);
        gguf_kv_u32(g, "llama.attention.head_count_kv", 1);
        gguf_kv_f32(g, "llama.attention.layer_norm_rms_epsilon", 1e-6);
        gguf_kv_u32(g, "llama.expert_count", n_exp);
        gguf_kv_u32(g, "llama.expert_used_count", 2);
        gguf_kv_str_array(
            g,
            "tokenizer.ggml.tokens",
            &["<s>", "</s>", "a", "b", "c", "d"],
        );
        gguf_kv_u32(g, "tokenizer.ggml.eos_token_id", 1);
    }

    const MOE_KV: u64 = 11;

    fn moe_trunk(h: u64, kv: u64, n_exp: u64) -> Vec<(&'static str, Vec<u64>)> {
        vec![
            ("token_embd.weight", vec![h, 6]),
            ("output_norm.weight", vec![h]),
            ("blk.0.attn_norm.weight", vec![h]),
            ("blk.0.attn_q.weight", vec![h, h]),
            ("blk.0.attn_k.weight", vec![h, kv]),
            ("blk.0.attn_v.weight", vec![h, kv]),
            ("blk.0.attn_output.weight", vec![h, h]),
            ("blk.0.ffn_norm.weight", vec![h]),
            ("blk.0.ffn_gate_inp.weight", vec![h, n_exp]),
        ]
    }

    #[test]
    fn convierte_gguf_mixtral_expertos_split() {
        // TheBloke Mixtral: blk.L.ffn_{gate,up,down}.E.weight (2D), no ffn_*_exps.
        const H: u64 = 8;
        const FFN: u64 = 16;
        const KV: u64 = 4;
        let mut tensors = moe_trunk(H, KV, 2);
        tensors.extend_from_slice(&[
            ("blk.0.ffn_gate.0.weight", vec![H, FFN]),
            ("blk.0.ffn_up.0.weight", vec![H, FFN]),
            ("blk.0.ffn_down.0.weight", vec![FFN, H]),
            ("blk.0.ffn_gate.1.weight", vec![H, FFN]),
            ("blk.0.ffn_up.1.weight", vec![H, FFN]),
            ("blk.0.ffn_down.1.weight", vec![FFN, H]),
        ]);
        let g = pack_gguf_v3(
            MOE_KV,
            |g| llama_moe_kv(g, H as u32, FFN as u32, 2),
            &tensors,
        );

        let dir = std::env::temp_dir().join("convert-gguf-moe-split");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let gguf_path = dir.join("mixtral-old.gguf");
        fs::File::create(&gguf_path).unwrap().write_all(&g).unwrap();
        let out = dir.join("out");
        convert_path(gguf_path.to_str().unwrap(), &out, Some("mixtral")).unwrap();

        let manifest = Manifest::parse(&fs::read(out.join(MANIFEST_FILE)).unwrap()).unwrap();
        assert_eq!(manifest.num_experts, 2);
        assert_eq!(manifest.moe_ffn_dim, FFN as u32);
        let index = TensorIndex::parse(&fs::read(out.join(INDEX_FILE)).unwrap()).unwrap();
        let gate = index.find("L00.E00.ffn_gate").unwrap();
        assert_eq!(gate.shape, vec![FFN as u32, H as u32]);
        let down = index.find("L00.E01.ffn_down").unwrap();
        assert_eq!(down.shape, vec![H as u32, FFN as u32]);
        let rt = soso_llm_core::runtime::Runtime::new(manifest, index, 0, 0);
        rt.validate_shapes().expect("shapes Mixtral split");
    }

    #[test]
    fn convierte_gguf_moe_fused_exps() {
        const H: u64 = 8;
        const FFN: u64 = 16;
        const KV: u64 = 4;
        const N_EXP: u64 = 2;
        let mut tensors = moe_trunk(H, KV, N_EXP);
        tensors.extend_from_slice(&[
            ("blk.0.ffn_gate_exps.weight", vec![H, FFN, N_EXP]),
            ("blk.0.ffn_up_exps.weight", vec![H, FFN, N_EXP]),
            ("blk.0.ffn_down_exps.weight", vec![FFN, H, N_EXP]),
        ]);
        let g = pack_gguf_v3(
            MOE_KV,
            |g| llama_moe_kv(g, H as u32, FFN as u32, N_EXP as u32),
            &tensors,
        );

        let dir = std::env::temp_dir().join("convert-gguf-moe-fused");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let gguf_path = dir.join("mixtral-fused.gguf");
        fs::File::create(&gguf_path).unwrap().write_all(&g).unwrap();
        let out = dir.join("out");
        convert_path(gguf_path.to_str().unwrap(), &out, Some("mixtral")).unwrap();

        let manifest = Manifest::parse(&fs::read(out.join(MANIFEST_FILE)).unwrap()).unwrap();
        let index = TensorIndex::parse(&fs::read(out.join(INDEX_FILE)).unwrap()).unwrap();
        assert!(index.find("L00.E00.ffn_gate").is_some());
        assert!(index.find("L00.E01.ffn_up").is_some());
        let rt = soso_llm_core::runtime::Runtime::new(manifest, index, 0, 0);
        rt.validate_shapes().expect("shapes MoE fused");
    }

    #[test]
    fn moe_sin_expertos_no_convierte_en_silencio() {
        const H: u64 = 8;
        let tensors = moe_trunk(H, 4, 2);
        let g = pack_gguf_v3(MOE_KV, |g| llama_moe_kv(g, H as u32, 16, 2), &tensors);
        let dir = std::env::temp_dir().join("convert-gguf-moe-missing");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let gguf_path = dir.join("broken.gguf");
        fs::File::create(&gguf_path).unwrap().write_all(&g).unwrap();
        let out = dir.join("out");
        let err = convert_path(gguf_path.to_str().unwrap(), &out, Some("mixtral")).unwrap_err();
        assert!(
            err.contains("faltan expertos"),
            "tenía que abortar, no convertir a medias: {err}"
        );
    }
}
