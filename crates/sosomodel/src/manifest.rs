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
    /// Atención completa con puerta (Qwen3.5/3.8): Q+gate fusionados, QK-norm.
    Gated = 3,
    /// Gated DeltaNet (atención lineal recurrente, Qwen3.5/3.8).
    Gdn = 4,
    /// Atención bidireccional (encoder Whisper).
    Bidirectional = 5,
    /// Cross-attention decoder→encoder (Whisper).
    Cross = 6,
}

impl AttnKind {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::Gqa),
            1 => Some(Self::Mla),
            2 => Some(Self::Kda),
            3 => Some(Self::Gated),
            4 => Some(Self::Gdn),
            5 => Some(Self::Bidirectional),
            6 => Some(Self::Cross),
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
    /// MLP con GELU (Whisper).
    GeluMlp = 3,
}

impl FfnKind {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::Dense),
            1 => Some(Self::Moe),
            2 => Some(Self::LatentMoe),
            3 => Some(Self::GeluMlp),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum NormKind {
    Rms = 0,
    Layer = 1,
}

impl NormKind {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::Rms),
            1 => Some(Self::Layer),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum ModelKind {
    Decoder = 0,
    AsrEncoderDecoder = 1,
}

impl ModelKind {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::Decoder),
            1 => Some(Self::AsrEncoderDecoder),
            _ => None,
        }
    }
}

/// Parámetros de audio/texto para modelos ASR (manifest v6).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AudioSpec {
    pub n_mels: u32,
    pub n_audio_ctx: u32,
    pub n_audio_state: u32,
    pub n_audio_layer: u32,
    pub n_audio_head: u32,
    pub n_text_ctx: u32,
    pub n_text_state: u32,
    pub n_text_layer: u32,
    pub n_text_head: u32,
}

impl Default for AudioSpec {
    fn default() -> Self {
        Self {
            n_mels: 80,
            n_audio_ctx: 1500,
            n_audio_state: 384,
            n_audio_layer: 4,
            n_audio_head: 6,
            n_text_ctx: 448,
            n_text_state: 384,
            n_text_layer: 4,
            n_text_head: 6,
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
    /// Plantilla de chat del modelo (v5), con los marcadores de
    /// `soso_llm_core::chat`: `{prompt}` y `{eos}`. Vacía = el modelo no es de
    /// chat o no se supo traducir su plantilla al convertirlo.
    ///
    /// Viaja en el modelo y no en la imagen a propósito: la plantilla es del
    /// modelo, así que un `soso-hf pull` de otra familia trae la suya y
    /// `/etc/llm.conf` sólo hace falta para pisarla.
    pub chat_template: String,
    /// Tipo de modelo (v6).
    pub model_kind: ModelKind,
    /// Parámetros ASR cuando `model_kind == AsrEncoderDecoder`.
    pub audio: AudioSpec,
    /// Norma por capa (v6); RMS para decoders clásicos.
    pub norm_kind: NormKind,
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

    /// Dimensión por cabeza. Si `v_head_dim > 0` (Gated/GDN/MLA) no es `hidden/heads`.
    pub fn effective_head_dim(&self, layer: u32) -> u32 {
        if let Some(s) = self.layer(layer) {
            if s.v_head_dim > 0 {
                return s.v_head_dim;
            }
        }
        let heads = self.effective_num_heads(layer);
        if heads == 0 {
            0
        } else {
            self.hidden_dim / heads
        }
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
            let kind = self.attn_kind(layer);
            if heads == 0 || kv_heads == 0 {
                return Err(());
            }
            match kind {
                AttnKind::Gdn => {
                    // n_k / n_v: las cabezas V son múltiplo de las QK.
                    if kv_heads % heads != 0 || self.effective_head_dim(layer) == 0 {
                        return Err(());
                    }
                }
                AttnKind::Gated => {
                    if heads % kv_heads != 0 || self.effective_head_dim(layer) == 0 {
                        return Err(());
                    }
                }
                AttnKind::Mla => {
                    if heads % kv_heads != 0 {
                        return Err(());
                    }
                }
                AttnKind::Gqa | AttnKind::Kda => {
                    if self.hidden_dim % heads != 0 || heads % kv_heads != 0 {
                        return Err(());
                    }
                }
                AttnKind::Bidirectional | AttnKind::Cross => {
                    if self.hidden_dim % heads != 0 || heads % kv_heads != 0 {
                        return Err(());
                    }
                }
            }
        }
        Ok(())
    }

    /// Capacidades soportadas por capa (Gqa/Mla/Kda/Gated/Gdn + Dense/Moe/LatentMoe + shared).
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
                AttnKind::Gqa
                | AttnKind::Mla
                | AttnKind::Kda
                | AttnKind::Gated
                | AttnKind::Gdn
                | AttnKind::Bidirectional
                | AttnKind::Cross => {}
            }
            match spec.ffn_kind {
                FfnKind::Dense | FfnKind::Moe | FfnKind::LatentMoe | FfnKind::GeluMlp => {}
            }
            match spec.attn_kind {
                AttnKind::Mla => {
                    if spec.q_lora_rank == 0 || spec.kv_lora_rank == 0 {
                        return Err(UnsupportedLayer {
                            layer,
                            reason: "MLA ranks required",
                        });
                    }
                }
                AttnKind::Gated | AttnKind::Gdn => {
                    if spec.v_head_dim == 0 {
                        return Err(UnsupportedLayer {
                            layer,
                            reason: "Gated/GDN requieren v_head_dim",
                        });
                    }
                }
                AttnKind::Gqa | AttnKind::Kda => {
                    if spec.kv_lora_rank > 0
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
                AttnKind::Bidirectional | AttnKind::Cross => {}
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
            // Sintético: no es un modelo de chat y su tokenizador es el
            // byte-level de reserva, donde `<|user|>` sólo son bytes.
            chat_template: String::new(),
            model_kind: ModelKind::Decoder,
            audio: AudioSpec::default(),
            norm_kind: NormKind::Rms,
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
            // Sintético: no es un modelo de chat y su tokenizador es el
            // byte-level de reserva, donde `<|user|>` sólo son bytes.
            chat_template: String::new(),
            model_kind: ModelKind::Decoder,
            audio: AudioSpec::default(),
            norm_kind: NormKind::Rms,
        };
        m.fill_layers_from_globals();
        m
    }

    /// Modelo ASR diminuto para tests (1 capa encoder + 1 decoder).
    pub fn tiny_asr(name: &str) -> Self {
        let audio = AudioSpec {
            n_mels: 80,
            n_audio_ctx: 50,
            n_audio_state: 64,
            n_audio_layer: 1,
            n_audio_head: 2,
            n_text_ctx: 32,
            n_text_state: 64,
            n_text_layer: 1,
            n_text_head: 2,
        };
        let layers = alloc::vec![
            LayerSpec {
                attn_kind: AttnKind::Bidirectional,
                ffn_kind: FfnKind::GeluMlp,
                num_heads: audio.n_audio_head,
                ..LayerSpec::default()
            },
            LayerSpec {
                attn_kind: AttnKind::Gqa,
                ffn_kind: FfnKind::GeluMlp,
                num_heads: audio.n_text_head,
                ..LayerSpec::default()
            },
        ];
        let prefetch = alloc::vec![
            LayerPrefetch {
                layer: 0,
                shards: alloc::vec![
                    String::from("conv1.weight.tensor"),
                    String::from("conv1.bias.tensor"),
                    String::from("conv2.weight.tensor"),
                    String::from("conv2.bias.tensor"),
                    String::from("pos_embed.tensor"),
                    String::from("E00.attn_ln.weight.tensor"),
                    String::from("E00.attn_ln.bias.tensor"),
                    String::from("E00.attn_q.weight.tensor"),
                    String::from("E00.attn_q.bias.tensor"),
                    String::from("E00.attn_k.weight.tensor"),
                    String::from("E00.attn_v.weight.tensor"),
                    String::from("E00.attn_out.weight.tensor"),
                    String::from("E00.attn_out.bias.tensor"),
                    String::from("E00.mlp_ln.weight.tensor"),
                    String::from("E00.mlp_ln.bias.tensor"),
                    String::from("E00.mlp_fc1.weight.tensor"),
                    String::from("E00.mlp_fc1.bias.tensor"),
                    String::from("E00.mlp_fc2.weight.tensor"),
                    String::from("E00.mlp_fc2.bias.tensor"),
                ],
            },
            LayerPrefetch {
                layer: 1,
                shards: alloc::vec![
                    String::from("token_embed.tensor"),
                    String::from("D00.attn_ln.weight.tensor"),
                    String::from("D00.attn_ln.bias.tensor"),
                    String::from("D00.attn_q.weight.tensor"),
                    String::from("D00.attn_q.bias.tensor"),
                    String::from("D00.attn_k.weight.tensor"),
                    String::from("D00.attn_v.weight.tensor"),
                    String::from("D00.attn_out.weight.tensor"),
                    String::from("D00.attn_out.bias.tensor"),
                    String::from("D00.cross_ln.weight.tensor"),
                    String::from("D00.cross_ln.bias.tensor"),
                    String::from("D00.cross_q.weight.tensor"),
                    String::from("D00.cross_q.bias.tensor"),
                    String::from("D00.cross_k.weight.tensor"),
                    String::from("D00.cross_v.weight.tensor"),
                    String::from("D00.cross_out.weight.tensor"),
                    String::from("D00.cross_out.bias.tensor"),
                    String::from("D00.mlp_ln.weight.tensor"),
                    String::from("D00.mlp_ln.bias.tensor"),
                    String::from("D00.mlp_fc1.weight.tensor"),
                    String::from("D00.mlp_fc1.bias.tensor"),
                    String::from("D00.mlp_fc2.weight.tensor"),
                    String::from("D00.mlp_fc2.bias.tensor"),
                ],
            },
        ];
        Self {
            name: String::from(name),
            vocab_size: 256,
            hidden_dim: audio.n_text_state,
            num_layers: 2,
            num_heads: audio.n_text_head,
            num_kv_heads: audio.n_text_head,
            ffn_dim: 128,
            max_seq: audio.n_text_ctx,
            rope_theta: 10000.0,
            rms_eps: 1e-5,
            num_experts: 0,
            num_experts_per_tok: 0,
            moe_ffn_dim: 0,
            layers,
            prefetch,
            chat_template: String::new(),
            model_kind: ModelKind::AsrEncoderDecoder,
            audio,
            norm_kind: NormKind::Layer,
        }
    }

    pub fn is_asr(&self) -> bool {
        self.model_kind == ModelKind::AsrEncoderDecoder
    }

    /// Manifest Whisper tiny/base (4+4 capas, AudioSpec::default).
    pub fn whisper_asr(name: &str, vocab_size: u32) -> Self {
        let audio = AudioSpec::default();
        let d = audio.n_text_state;
        let ffn = 4 * d;
        let mut layers = Vec::new();
        let mut prefetch = Vec::new();
        for i in 0..audio.n_audio_layer {
            layers.push(LayerSpec {
                attn_kind: AttnKind::Bidirectional,
                ffn_kind: FfnKind::GeluMlp,
                num_heads: audio.n_audio_head,
                ..LayerSpec::default()
            });
            let p = format!("E{i:02}");
            prefetch.push(LayerPrefetch {
                layer: i,
                shards: encoder_layer_shards(&p),
            });
        }
        for i in 0..audio.n_text_layer {
            layers.push(LayerSpec {
                attn_kind: AttnKind::Gqa,
                ffn_kind: FfnKind::GeluMlp,
                num_heads: audio.n_text_head,
                ..LayerSpec::default()
            });
            let p = format!("D{i:02}");
            prefetch.push(LayerPrefetch {
                layer: audio.n_audio_layer + i,
                shards: decoder_layer_shards(&p),
            });
        }
        Self {
            name: String::from(name),
            vocab_size,
            hidden_dim: d,
            num_layers: audio.n_audio_layer + audio.n_text_layer,
            num_heads: audio.n_text_head,
            num_kv_heads: audio.n_text_head,
            ffn_dim: ffn,
            max_seq: audio.n_text_ctx,
            rope_theta: 10000.0,
            rms_eps: 1e-5,
            num_experts: 0,
            num_experts_per_tok: 0,
            moe_ffn_dim: 0,
            layers,
            prefetch,
            chat_template: String::new(),
            model_kind: ModelKind::AsrEncoderDecoder,
            audio,
            norm_kind: NormKind::Layer,
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
        // v5: plantilla de chat, al final y con NUL. Va detrás de todo lo demás
        // para que un lector de v4 llegue a su fin natural sin verla.
        body.extend_from_slice(self.chat_template.as_bytes());
        body.push(0);
        // v6: tipo ASR + AudioSpec + NormKind
        body.push(self.model_kind as u8);
        body.push(self.norm_kind as u8);
        body.extend_from_slice(&0u16.to_le_bytes());
        body.extend_from_slice(&self.audio.n_mels.to_le_bytes());
        body.extend_from_slice(&self.audio.n_audio_ctx.to_le_bytes());
        body.extend_from_slice(&self.audio.n_audio_state.to_le_bytes());
        body.extend_from_slice(&self.audio.n_audio_layer.to_le_bytes());
        body.extend_from_slice(&self.audio.n_audio_head.to_le_bytes());
        body.extend_from_slice(&self.audio.n_text_ctx.to_le_bytes());
        body.extend_from_slice(&self.audio.n_text_state.to_le_bytes());
        body.extend_from_slice(&self.audio.n_text_layer.to_le_bytes());
        body.extend_from_slice(&self.audio.n_text_head.to_le_bytes());
        debug_assert_eq!(LAYER_SPEC_BYTES, 60);
        let version = if self.model_kind == ModelKind::AsrEncoderDecoder {
            6
        } else {
            5
        };
        pack_som(&body, version, CACHE_ALIGN)
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
        if num_heads == 0 || num_kv_heads == 0 {
            return Err(());
        }
        // v4+ puede traer head_dim explícito (Qwen3.8: hidden % heads ≠ 0).
        // En v1–v3 la divisibilidad clásica sigue siendo contrato.
        if version < 4
            && (hidden_dim % num_heads != 0 || num_heads % num_kv_heads != 0)
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
        // v5: si el modelo es anterior, no hay plantilla y el texto va crudo, que
        // es exactamente lo que hacía todo antes de esta versión.
        let chat_template = if version >= 5 {
            String::from(r.cstr()?)
        } else {
            String::new()
        };
        let (model_kind, norm_kind, audio) = if version >= 6 {
            let mk = ModelKind::from_u8(r.u8()?).ok_or(())?;
            let nk = NormKind::from_u8(r.u8()?).ok_or(())?;
            let _pad = u16::from_le_bytes(r.take(2)?.try_into().map_err(|_| ())?);
            let audio = AudioSpec {
                n_mels: r.u32()?,
                n_audio_ctx: r.u32()?,
                n_audio_state: r.u32()?,
                n_audio_layer: r.u32()?,
                n_audio_head: r.u32()?,
                n_text_ctx: r.u32()?,
                n_text_state: r.u32()?,
                n_text_layer: r.u32()?,
                n_text_head: r.u32()?,
            };
            (mk, nk, audio)
        } else {
            (ModelKind::Decoder, NormKind::Rms, AudioSpec::default())
        };
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
            chat_template,
            model_kind,
            audio,
            norm_kind,
        };
        manifest.validate_layer_divisibility()?;
        Ok(manifest)
    }
}

fn encoder_layer_shards(prefix: &str) -> Vec<String> {
    alloc::vec![
        format!("{prefix}.attn_ln.weight.tensor"),
        format!("{prefix}.attn_ln.bias.tensor"),
        format!("{prefix}.attn_q.weight.tensor"),
        format!("{prefix}.attn_q.bias.tensor"),
        format!("{prefix}.attn_k.weight.tensor"),
        format!("{prefix}.attn_v.weight.tensor"),
        format!("{prefix}.attn_v.bias.tensor"),
        format!("{prefix}.attn_out.weight.tensor"),
        format!("{prefix}.attn_out.bias.tensor"),
        format!("{prefix}.mlp_ln.weight.tensor"),
        format!("{prefix}.mlp_ln.bias.tensor"),
        format!("{prefix}.mlp_fc1.weight.tensor"),
        format!("{prefix}.mlp_fc1.bias.tensor"),
        format!("{prefix}.mlp_fc2.weight.tensor"),
        format!("{prefix}.mlp_fc2.bias.tensor"),
    ]
}

fn decoder_layer_shards(prefix: &str) -> Vec<String> {
    alloc::vec![
        format!("{prefix}.attn_ln.weight.tensor"),
        format!("{prefix}.attn_ln.bias.tensor"),
        format!("{prefix}.attn_q.weight.tensor"),
        format!("{prefix}.attn_q.bias.tensor"),
        format!("{prefix}.attn_k.weight.tensor"),
        format!("{prefix}.attn_v.weight.tensor"),
        format!("{prefix}.attn_v.bias.tensor"),
        format!("{prefix}.attn_out.weight.tensor"),
        format!("{prefix}.attn_out.bias.tensor"),
        format!("{prefix}.cross_ln.weight.tensor"),
        format!("{prefix}.cross_ln.bias.tensor"),
        format!("{prefix}.cross_q.weight.tensor"),
        format!("{prefix}.cross_q.bias.tensor"),
        format!("{prefix}.cross_k.weight.tensor"),
        format!("{prefix}.cross_v.weight.tensor"),
        format!("{prefix}.cross_out.weight.tensor"),
        format!("{prefix}.cross_out.bias.tensor"),
        format!("{prefix}.mlp_ln.weight.tensor"),
        format!("{prefix}.mlp_ln.bias.tensor"),
        format!("{prefix}.mlp_fc1.weight.tensor"),
        format!("{prefix}.mlp_fc1.bias.tensor"),
        format!("{prefix}.mlp_fc2.weight.tensor"),
        format!("{prefix}.mlp_fc2.bias.tensor"),
    ]
}
