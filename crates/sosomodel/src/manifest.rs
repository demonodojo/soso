//! manifest.som: arquitectura del transformer y grafo de prefetch.

use crate::{pack_som, parse_som, Reader, CACHE_ALIGN};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum AttnKind {
    Gqa = 0,
    Mla = 1,
    Kda = 2,
}

impl AttnKind {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::Gqa),
            1 => Some(Self::Mla),
            2 => Some(Self::Kda),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum FfnKind {
    Dense = 0,
    Moe = 1,
    LatentMoe = 2,
}

impl FfnKind {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::Dense),
            1 => Some(Self::Moe),
            2 => Some(Self::LatentMoe),
            _ => None,
        }
    }
}

/// Especificación por capa (manifest v4). Overrides a 0 heredan del global.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LayerSpec {
    pub attn_kind: AttnKind,
    pub ffn_kind: FfnKind,
    /// Cabezas Q; 0 = heredar `Manifest::num_heads`.
    pub num_heads: u32,
    /// Cabezas K/V; 0 = heredar `Manifest::num_kv_heads`.
    pub num_kv_heads: u32,
    /// FFN denso; 0 = heredar `Manifest::ffn_dim`.
    pub ffn_dim: u32,
    pub kv_lora_rank: u32,
    pub q_lora_rank: u32,
    pub qk_rope_head_dim: u32,
    pub qk_nope_head_dim: u32,
    pub v_head_dim: u32,
    pub num_experts: u32,
    pub num_experts_per_tok: u32,
    pub moe_ffn_dim: u32,
    pub num_shared_experts: u32,
    /// Reservado: bit0 gated MLA, bit1 AttnRes, bit2 SiTU.
    pub flags: u32,
}

impl Default for LayerSpec {
    fn default() -> Self {
        Self {
            attn_kind: AttnKind::Gqa,
            ffn_kind: FfnKind::Dense,
            num_heads: 0,
            num_kv_heads: 0,
            ffn_dim: 0,
            kv_lora_rank: 0,
            q_lora_rank: 0,
            qk_rope_head_dim: 0,
            qk_nope_head_dim: 0,
            v_head_dim: 0,
            num_experts: 0,
            num_experts_per_tok: 0,
            moe_ffn_dim: 0,
            num_shared_experts: 0,
            flags: 0,
        }
    }
}

/// Capa no soportada por el runtime actual (solo Gqa + Dense/Moe sin shared).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnsupportedLayer {
    pub layer: u32,
    pub reason: &'static str,
}

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
    pub layers: Vec<LayerSpec>,
    pub prefetch: Vec<LayerPrefetch>,
}

const LAYER_SPEC_BYTES: usize = 60;

impl LayerSpec {
    fn serialize_into(&self, body: &mut Vec<u8>) {
        body.push(self.attn_kind as u8);
        body.push(self.ffn_kind as u8);
        body.extend_from_slice(&0u16.to_le_bytes());
        body.extend_from_slice(&self.num_heads.to_le_bytes());
        body.extend_from_slice(&self.num_kv_heads.to_le_bytes());
        body.extend_from_slice(&self.ffn_dim.to_le_bytes());
        body.extend_from_slice(&self.kv_lora_rank.to_le_bytes());
        body.extend_from_slice(&self.q_lora_rank.to_le_bytes());
        body.extend_from_slice(&self.qk_rope_head_dim.to_le_bytes());
        body.extend_from_slice(&self.qk_nope_head_dim.to_le_bytes());
        body.extend_from_slice(&self.v_head_dim.to_le_bytes());
        body.extend_from_slice(&self.num_experts.to_le_bytes());
        body.extend_from_slice(&self.num_experts_per_tok.to_le_bytes());
        body.extend_from_slice(&self.moe_ffn_dim.to_le_bytes());
        body.extend_from_slice(&self.num_shared_experts.to_le_bytes());
        body.extend_from_slice(&self.flags.to_le_bytes());
    }

    fn parse(r: &mut Reader<'_>) -> Result<Self, ()> {
        let attn_kind = AttnKind::from_u8(r.u8()?).ok_or(())?;
        let ffn_kind = FfnKind::from_u8(r.u8()?).ok_or(())?;
        let _pad = u16::from_le_bytes(r.take(2)?.try_into().map_err(|_| ())?);
        Ok(Self {
            attn_kind,
            ffn_kind,
            num_heads: r.u32()?,
            num_kv_heads: r.u32()?,
            ffn_dim: r.u32()?,
            kv_lora_rank: r.u32()?,
            q_lora_rank: r.u32()?,
            qk_rope_head_dim: r.u32()?,
            qk_nope_head_dim: r.u32()?,
            v_head_dim: r.u32()?,
            num_experts: r.u32()?,
            num_experts_per_tok: r.u32()?,
            moe_ffn_dim: r.u32()?,
            num_shared_experts: r.u32()?,
            flags: r.u32()?,
        })
    }
}

impl Manifest {
    pub fn is_moe(&self) -> bool {
        self.num_experts > 0 || self.layers.iter().any(|l| Self::layer_is_moe_spec(l))
    }

    fn layer_is_moe_spec(spec: &LayerSpec) -> bool {
        matches!(spec.ffn_kind, FfnKind::Moe | FfnKind::LatentMoe) || spec.num_experts > 0
    }

    /// Dimensión del FFN por experto (MoE o denso) a nivel global.
    pub fn expert_ffn_dim(&self) -> u32 {
        if self.num_experts > 0 && self.moe_ffn_dim > 0 {
            self.moe_ffn_dim
        } else {
            self.ffn_dim
        }
    }

    pub fn layer(&self, i: u32) -> Option<&LayerSpec> {
        self.layers.get(i as usize)
    }

    pub fn attn_kind(&self, layer: u32) -> AttnKind {
        self.layer(layer)
            .map(|s| s.attn_kind)
            .unwrap_or(AttnKind::Gqa)
    }

    pub fn ffn_kind(&self, layer: u32) -> FfnKind {
        self.layer(layer)
            .map(|s| s.ffn_kind)
            .unwrap_or(FfnKind::Dense)
    }

    pub fn layer_is_moe(&self, layer: u32) -> bool {
        self.layer(layer)
            .map(Self::layer_is_moe_spec)
            .unwrap_or(self.num_experts > 0)
    }

    pub fn effective_num_heads(&self, layer: u32) -> u32 {
        self.layer(layer)
            .and_then(|s| if s.num_heads > 0 { Some(s.num_heads) } else { None })
            .unwrap_or(self.num_heads)
    }

    pub fn effective_num_kv_heads(&self, layer: u32) -> u32 {
        self.layer(layer)
            .and_then(|s| {
                if s.num_kv_heads > 0 {
                    Some(s.num_kv_heads)
                } else {
                    None
                }
            })
            .unwrap_or(self.num_kv_heads)
    }

    pub fn effective_ffn_dim(&self, layer: u32) -> u32 {
        self.layer(layer)
            .and_then(|s| if s.ffn_dim > 0 { Some(s.ffn_dim) } else { None })
            .unwrap_or(self.ffn_dim)
    }

    pub fn effective_num_experts(&self, layer: u32) -> u32 {
        self.layer(layer)
            .and_then(|s| if s.num_experts > 0 { Some(s.num_experts) } else { None })
            .unwrap_or(self.num_experts)
    }

    pub fn effective_num_experts_per_tok(&self, layer: u32) -> u32 {
        self.layer(layer)
            .and_then(|s| {
                if s.num_experts_per_tok > 0 {
                    Some(s.num_experts_per_tok)
                } else {
                    None
                }
            })
            .unwrap_or(self.num_experts_per_tok)
    }

    pub fn effective_moe_ffn_dim(&self, layer: u32) -> u32 {
        let global = self.expert_ffn_dim();
        self.layer(layer)
            .and_then(|s| if s.moe_ffn_dim > 0 { Some(s.moe_ffn_dim) } else { None })
            .unwrap_or(global)
    }

    pub fn layer_expert_ffn_dim(&self, layer: u32) -> u32 {
        if self.layer_is_moe(layer) {
            self.effective_moe_ffn_dim(layer)
        } else {
            self.effective_ffn_dim(layer)
        }
    }

    /// Máximo de expertos en router (dimensiona scratch).
    pub fn max_router_experts(&self) -> u32 {
        let mut max = self.num_experts;
        for (i, _) in self.layers.iter().enumerate().take(self.num_layers as usize) {
            max = max.max(self.effective_num_experts(i as u32));
        }
        max
    }

    /// Máximo FFN activo (dimensiona scratch).
    pub fn max_ffn_dim(&self) -> u32 {
        (0..self.num_layers)
            .map(|i| self.layer_expert_ffn_dim(i))
            .max()
            .unwrap_or(self.ffn_dim)
    }

    /// Rellena `layers` con Gqa + Dense o Gqa + Moe uniformes desde globals.
    pub fn fill_layers_from_globals(&mut self) {
        let ffn_kind = if self.num_experts > 0 {
            FfnKind::Moe
        } else {
            FfnKind::Dense
        };
        self.layers = (0..self.num_layers)
            .map(|_| LayerSpec {
                attn_kind: AttnKind::Gqa,
                ffn_kind,
                ..LayerSpec::default()
            })
            .collect();
    }

    fn synthesize_layers_from_globals(num_layers: u32, num_experts: u32) -> Vec<LayerSpec> {
        let ffn_kind = if num_experts > 0 {
            FfnKind::Moe
        } else {
            FfnKind::Dense
        };
        (0..num_layers)
            .map(|_| LayerSpec {
                attn_kind: AttnKind::Gqa,
                ffn_kind,
                ..LayerSpec::default()
            })
            .collect()
    }

    fn validate_layer_divisibility(&self) -> Result<(), ()> {
        for layer in 0..self.num_layers {
            let heads = self.effective_num_heads(layer);
            let kv_heads = self.effective_num_kv_heads(layer);
            if heads == 0
                || kv_heads == 0
                || self.hidden_dim % heads != 0
                || heads % kv_heads != 0
            {
                return Err(());
            }
        }
        Ok(())
    }

    /// Capacidades soportadas por capa (Gqa/Mla/Kda + Dense/Moe/LatentMoe + shared).
    pub fn supported_by_runtime(&self) -> Result<(), UnsupportedLayer> {
        for layer in 0..self.num_layers {
            let spec = self.layer(layer).ok_or(UnsupportedLayer {
                layer,
                reason: "missing layer spec",
            })?;
            if spec.flags != 0 {
                return Err(UnsupportedLayer {
                    layer,
                    reason: "layer flags not supported",
                });
            }
            match spec.attn_kind {
                AttnKind::Gqa | AttnKind::Mla | AttnKind::Kda => {}
            }
            match spec.ffn_kind {
                FfnKind::Dense | FfnKind::Moe | FfnKind::LatentMoe => {}
            }
            if spec.attn_kind == AttnKind::Mla {
                if spec.q_lora_rank == 0 || spec.kv_lora_rank == 0 {
                    return Err(UnsupportedLayer {
                        layer,
                        reason: "MLA ranks required",
                    });
                }
            } else if spec.kv_lora_rank > 0
                || spec.q_lora_rank > 0
                || spec.qk_rope_head_dim > 0
                || spec.qk_nope_head_dim > 0
                || spec.v_head_dim > 0
            {
                return Err(UnsupportedLayer {
                    layer,
                    reason: "MLA dims on non-MLA layer",
                });
            }
        }
        Ok(())
    }

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
        let mut m = Self {
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
            layers: Vec::new(),
            prefetch,
        };
        m.fill_layers_from_globals();
        m
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
        let mut m = Self {
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
            layers: Vec::new(),
            prefetch,
        };
        m.fill_layers_from_globals();
        m
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
        body.extend_from_slice(&self.num_kv_heads.to_le_bytes());
        body.extend_from_slice(&self.rope_theta.to_le_bytes());
        body.extend_from_slice(&self.rms_eps.to_le_bytes());
        body.extend_from_slice(&self.num_experts.to_le_bytes());
        body.extend_from_slice(&self.num_experts_per_tok.to_le_bytes());
        body.extend_from_slice(&self.moe_ffn_dim.to_le_bytes());
        // v4: tabla LayerSpec
        body.extend_from_slice(&(self.layers.len() as u32).to_le_bytes());
        for spec in &self.layers {
            spec.serialize_into(&mut body);
        }
        body.extend_from_slice(&(self.prefetch.len() as u32).to_le_bytes());
        for pf in &self.prefetch {
            body.extend_from_slice(&pf.layer.to_le_bytes());
            body.extend_from_slice(&(pf.shards.len() as u32).to_le_bytes());
            for s in &pf.shards {
                body.extend_from_slice(s.as_bytes());
                body.push(0);
            }
        }
        debug_assert_eq!(LAYER_SPEC_BYTES, 60);
        pack_som(&body, 4, CACHE_ALIGN)
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
            || hidden_dim % num_heads != 0
            || num_heads % num_kv_heads != 0
        {
            return Err(());
        }
        let layers = if version >= 4 {
            let n_specs = r.u32()? as usize;
            if n_specs != num_layers as usize {
                return Err(());
            }
            let mut specs = Vec::with_capacity(n_specs);
            for _ in 0..n_specs {
                specs.push(LayerSpec::parse(&mut r)?);
            }
            specs
        } else {
            Self::synthesize_layers_from_globals(num_layers, num_experts)
        };
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
        let manifest = Self {
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
            layers,
            prefetch,
        };
        manifest.validate_layer_divisibility()?;
        Ok(manifest)
    }
}
