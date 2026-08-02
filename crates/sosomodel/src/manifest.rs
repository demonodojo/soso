//! manifest.som: arquitectura del transformer y grafo de prefetch.

use crate::{pack_som, parse_som, Reader, CACHE_ALIGN};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

#[derive(Clone, Debug)]
pub struct LayerPrefetch {
    pub layer: u32,
    pub shards: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct Manifest {
    pub name: String,
    pub vocab_size: u32,
    pub hidden_dim: u32,
    pub num_layers: u32,
    pub num_heads: u32,
    /// Cabezas K/V (GQA). En MHA clásico coincide con `num_heads`.
    pub num_kv_heads: u32,
    pub ffn_dim: u32,
    pub max_seq: u32,
    /// Base de frecuencias RoPE (10000.0 en llama clásico).
    pub rope_theta: f32,
    /// Épsilon del RMSNorm.
    pub rms_eps: f32,
    /// Número de expertos MoE (0 = modelo denso clásico).
    pub num_experts: u32,
    /// Expertos activos por token (top-k del router).
    pub num_experts_per_tok: u32,
    /// Dimensión FFN por experto; 0 = usar `ffn_dim`.
    pub moe_ffn_dim: u32,
    pub prefetch: Vec<LayerPrefetch>,
}

impl Manifest {
    pub fn is_moe(&self) -> bool {
        self.num_experts > 0
    }

    /// Dimensión del FFN por experto (MoE o denso).
    pub fn expert_ffn_dim(&self) -> u32 {
        if self.is_moe() && self.moe_ffn_dim > 0 {
            self.moe_ffn_dim
        } else {
            self.ffn_dim
        }
    }
}

impl Manifest {
    pub fn tiny(name: &str) -> Self {
        let mut prefetch = Vec::new();
        for layer in 0..4u32 {
            prefetch.push(LayerPrefetch {
                layer,
                shards: alloc::vec![
                    format!("L{layer:02}.attn_norm.tensor"),
                    format!("L{layer:02}.attn_q.tensor"),
                    format!("L{layer:02}.attn_k.tensor"),
                    format!("L{layer:02}.attn_v.tensor"),
                    format!("L{layer:02}.attn_output.tensor"),
                    format!("L{layer:02}.ffn_norm.tensor"),
                    format!("L{layer:02}.ffn_up.tensor"),
                    format!("L{layer:02}.ffn_down.tensor"),
                ],
            });
        }
        Self {
            name: String::from(name),
            vocab_size: 256,
            hidden_dim: 128,
            num_layers: 4,
            num_heads: 4,
            num_kv_heads: 4,
            ffn_dim: 256,
            max_seq: 128,
            rope_theta: 10000.0,
            rms_eps: 1e-5,
            num_experts: 0,
            num_experts_per_tok: 0,
            moe_ffn_dim: 0,
            prefetch,
        }
    }

    /// Modelo MoE diminuto para tests (4 expertos, top-2, 2 capas).
    pub fn tiny_moe(name: &str) -> Self {
        let num_layers = 2u32;
        let num_experts = 4u32;
        let mut prefetch = Vec::new();
        for layer in 0..num_layers {
            prefetch.push(LayerPrefetch {
                layer,
                shards: alloc::vec![
                    format!("L{layer:02}.attn_norm.tensor"),
                    format!("L{layer:02}.attn_q.tensor"),
                    format!("L{layer:02}.attn_k.tensor"),
                    format!("L{layer:02}.attn_v.tensor"),
                    format!("L{layer:02}.attn_output.tensor"),
                    format!("L{layer:02}.ffn_norm.tensor"),
                    format!("L{layer:02}.ffn_gate_inp.tensor"),
                ],
            });
        }
        Self {
            name: String::from(name),
            vocab_size: 64,
            hidden_dim: 64,
            num_layers,
            num_heads: 4,
            num_kv_heads: 4,
            ffn_dim: 128,
            max_seq: 64,
            rope_theta: 10000.0,
            rms_eps: 1e-5,
            num_experts,
            num_experts_per_tok: 2,
            moe_ffn_dim: 32,
            prefetch,
        }
    }

    pub fn serialize(&self) -> Vec<u8> {
        let mut body = Vec::new();
        body.extend_from_slice(self.name.as_bytes());
        body.push(0);
        body.extend_from_slice(&self.vocab_size.to_le_bytes());
        body.extend_from_slice(&self.hidden_dim.to_le_bytes());
        body.extend_from_slice(&self.num_layers.to_le_bytes());
        body.extend_from_slice(&self.num_heads.to_le_bytes());
        body.extend_from_slice(&self.ffn_dim.to_le_bytes());
        body.extend_from_slice(&self.max_seq.to_le_bytes());
        // v2: GQA + RoPE + eps
        body.extend_from_slice(&self.num_kv_heads.to_le_bytes());
        body.extend_from_slice(&self.rope_theta.to_le_bytes());
        body.extend_from_slice(&self.rms_eps.to_le_bytes());
        // v3: MoE (Mixtral / Qwen3-MoE estilo llama+expert_count)
        body.extend_from_slice(&self.num_experts.to_le_bytes());
        body.extend_from_slice(&self.num_experts_per_tok.to_le_bytes());
        body.extend_from_slice(&self.moe_ffn_dim.to_le_bytes());
        body.extend_from_slice(&(self.prefetch.len() as u32).to_le_bytes());
        for pf in &self.prefetch {
            body.extend_from_slice(&pf.layer.to_le_bytes());
            body.extend_from_slice(&(pf.shards.len() as u32).to_le_bytes());
            for s in &pf.shards {
                body.extend_from_slice(s.as_bytes());
                body.push(0);
            }
        }
        pack_som(&body, 3, CACHE_ALIGN)
    }

    pub fn parse(data: &[u8]) -> Result<Self, ()> {
        let (version, body) = parse_som(data)?;
        let mut r = Reader::new(body);
        let name = String::from(r.cstr()?);
        let vocab_size = r.u32()?;
        let hidden_dim = r.u32()?;
        let num_layers = r.u32()?;
        let num_heads = r.u32()?;
        let ffn_dim = r.u32()?;
        let max_seq = r.u32()?;
        let (num_kv_heads, rope_theta, rms_eps) = if version >= 2 {
            (r.u32()?, r.f32()?, r.f32()?)
        } else {
            (num_heads, 10000.0, 1e-5)
        };
        let (num_experts, num_experts_per_tok, moe_ffn_dim) = if version >= 3 {
            (r.u32()?, r.u32()?, r.u32()?)
        } else {
            (0, 0, 0)
        };
        if num_heads == 0
            || num_kv_heads == 0
            || hidden_dim as usize % num_heads as usize != 0
            || num_heads % num_kv_heads != 0
        {
            return Err(());
        }
        let n_pf = r.u32()? as usize;
        let mut prefetch = Vec::new();
        for _ in 0..n_pf {
            let layer = r.u32()?;
            let n_shards = r.u32()? as usize;
            let mut shards = Vec::new();
            for _ in 0..n_shards {
                shards.push(String::from(r.cstr()?));
            }
            prefetch.push(LayerPrefetch { layer, shards });
        }
        Ok(Self {
            name,
            vocab_size,
            hidden_dim,
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
            prefetch,
        })
    }
}
