//! Runtime de inferencia: bucle de generación de tokens.

use crate::generation::{
    GenCancel, GenCheckpoint, GenerationLedger, GenerationObserver, GenerationOptions,
    GenerationReport, LegacyStreamObserver, NoCancelObserver, stops_from_eos,
};
use crate::gemm::rmsnorm;
use crate::kv::LayerKv;
use crate::layer::{
    clear_infer_op, matvec_view_par, note_infer_op, BlockScratch, LayerExecutor, LayerScratch,
    TensorSource, TensorView,
};
use crate::parallel::{RowParallel, Sequential};
use crate::plan::{ExecDest, ResourcePlanner};
use crate::tier::TierManager;
use alloc::boxed::Box;
use alloc::string::String;
use crate::restriccion::Restriccion;
use alloc::vec;
use alloc::vec::Vec;
use crate::pipeline::PipelineRole;
use sosomodel::index::TensorIndex;
use sosomodel::manifest::{AttnKind, Manifest};

mod decode_gpu;

fn kv_storage_dim(manifest: &Manifest, layer: u32) -> usize {
    let spec = manifest.layer(layer).cloned().unwrap_or_default();
    if spec.attn_kind == AttnKind::Mla && spec.kv_lora_rank > 0 {
        spec.kv_lora_rank as usize
    } else if spec.attn_kind == AttnKind::Gdn {
        0
    } else {
        let head_dim = manifest.effective_head_dim(layer) as usize;
        manifest.effective_num_kv_heads(layer) as usize * head_dim
    }
}

fn make_layer_kv(
    manifest: &Manifest,
    layer: u32,
    kv_cap: usize,
    dtype: crate::kv::KvDtype,
) -> LayerKv {
    let spec = manifest.layer(layer).cloned().unwrap_or_default();
    if spec.attn_kind == AttnKind::Mla && spec.kv_lora_rank > 0 {
        LayerKv::with_capacity_mla_dtype(kv_cap, spec.kv_lora_rank as usize, dtype)
    } else if spec.attn_kind == AttnKind::Gdn {
        let mut kv = LayerKv::with_capacity_dtype(0, 0, dtype);
        let n_v = manifest.effective_num_kv_heads(layer) as usize;
        let d = manifest.effective_head_dim(layer) as usize;
        let kernel = spec.qk_rope_head_dim.max(1) as usize;
        let n_k = manifest.effective_num_heads(layer) as usize;
        let conv_dim = n_k.saturating_mul(d).saturating_mul(2) + n_v.saturating_mul(d);
        kv.ensure_gdn(n_v, d, kernel.saturating_sub(1).saturating_mul(conv_dim));
        kv
    } else {
        let head_dim = manifest.effective_head_dim(layer) as usize;
        let kv_dim = manifest.effective_num_kv_heads(layer) as usize * head_dim;
        // Una capa local de Gemma sólo guarda su ventana (más la holgura de expulsión).
        let ventana = manifest.layer_window(layer) as usize;
        let cap = if ventana > 0 {
            kv_cap.min(ventana + crate::gemma::EXPULSION_DISPARO + Runtime::BLOQUE_PREFILL_MOE_GPU)
        } else {
            kv_cap
        };
        LayerKv::with_capacity_dtype(cap, kv_dim, dtype)
    }
}

pub struct MemoryTensorSource {
    pub tensors: alloc::collections::BTreeMap<String, Vec<f32>>,
}

impl TensorSource for MemoryTensorSource {
    fn load_f32(&mut self, name: &str, out: &mut [f32]) -> Result<(), ()> {
        let data = self.tensors.get(name).ok_or(())?;
        if data.len() != out.len() {
            return Err(());
        }
        out.copy_from_slice(data);
        Ok(())
    }

    fn load_f32_range(&mut self, name: &str, elem_off: usize, out: &mut [f32]) -> Result<(), ()> {
        let data = self.tensors.get(name).ok_or(())?;
        let end = elem_off.checked_add(out.len()).ok_or(())?;
        if end > data.len() {
            return Err(());
        }
        out.copy_from_slice(&data[elem_off..end]);
        Ok(())
    }

    fn tensor_view(&mut self, name: &str) -> Result<TensorView<'_>, ()> {
        let data = self.tensors.get(name).ok_or(())?;
        let bytes = unsafe {
            core::slice::from_raw_parts(data.as_ptr() as *const u8, data.len() * 4)
        };
        Ok(TensorView {
            bytes,
            dtype: sosomodel::layout::DTYPE_F32,
            elems: data.len(),
        })
    }
}

/// Avisa a la restricción (si hay) de un token aceptado.
fn sync_cluster_prefijo(
    cluster: &mut Option<&mut crate::remoto::ClusterGen<'_>>,
    ya: usize,
) -> Result<(), ()> {
    if let Some(crate::remoto::ClusterGen { remoto, .. }) = cluster.as_deref_mut() {
        if ya == 0 {
            remoto.reiniciar()?;
        } else {
            remoto.truncar(ya)?;
        }
    }
    Ok(())
}

fn avanzar_restriccion(restr: &mut Option<Box<dyn Restriccion>>, token: u32) {
    if let Some(r) = restr.as_deref_mut() {
        r.avanza(token);
    }
}

/// Recorta el borrador al prefijo que la restricción permite. Con la restricción activa los tokens aceptados
/// se verifican contra el argmax **sin** restringir: un borrador permitido que coincide con él coincide
/// también con el argmax restringido, así que aceptarlo es correcto (rechazar de más sólo pierde velocidad).
fn filtrar_borrador(restr: &Option<Box<dyn Restriccion>>, drafts: &mut Vec<u32>) {
    let Some(r) = restr.as_deref() else {
        return;
    };
    if !r.activa() && drafts.is_empty() {
        return;
    }
    let mut copia = r.duplicar();
    let mut validos = 0;
    for &d in drafts.iter() {
        if !copia.permite(d) {
            break;
        }
        copia.avanza(d);
        validos += 1;
    }
    drafts.truncate(validos);
}

/// Resultado de intentar verificar un borrador con un bloque.
enum Especulativo {
    /// No se pudo (modelo o estado no aptos): el bucle sigue por el camino token a token.
    NoAplica,
    /// Hecho: el estado está en el último token aceptado.
    Hecho { ofrecidos: usize, aceptados: usize, replanificado: bool },
    /// Parada (token de parada aceptado o cancelación): el bucle termina.
    Parar,
}

/// Prefijo compartido entre preguntas Kev (estado + caché recurrente).
#[derive(Clone)]
pub struct KevPrefixSnapshot {
    pub pos: usize,
    pub hidden: Vec<f32>,
    pub kv: Vec<LayerKv>,
}

/// Estado de las capas GDN tras procesar `tokens` (el prompt de una petición).
#[derive(Clone)]
struct InstantaneaGdn {
    tokens: Vec<u32>,
    capas: Vec<(usize, Vec<f32>, Vec<f32>)>,
}

pub struct Runtime {
    pub manifest: Manifest,
    pub index: TensorIndex,
    pub tiers: TierManager,
    pub hidden: Vec<f32>,
    pub kv: Vec<LayerKv>,
    pub pos: usize,
    /// Tokens cuyo K/V está en la caché, en orden: el prefijo que una petición
    /// nueva puede reutilizar sin repetir el prefill (T22). Vacío cuando la caché
    /// no es fiable (cancelación, planificador, otra generación en curso).
    cached_tokens: Vec<u32>,
    /// Instantáneas del estado de las capas Gated DeltaNet al final de cada prompt (las más recientes primero). El estado
    /// recurrente no se puede recortar como un KV: para reutilizar un prefijo hay que volver a un punto guardado.
    gdn_instantaneas: Vec<InstantaneaGdn>,
    /// Posición a partir de la cual se toma la siguiente instantánea intermedia durante el prefill.
    proxima_marca_gdn: usize,
    /// Buffers del prefill por bloques (T82); se crean la primera vez que hacen falta.
    block: Option<BlockScratch>,
    /// Prefill por bloques activado (por defecto sí). Apagarlo fuerza el camino token
    /// a token: sirve para comparar ambos y como salida de emergencia.
    pub prefill_por_bloques: bool,
    /// Cuántos tokens se han procesado por el camino de bloques (diagnóstico/pruebas).
    pub tokens_en_bloque: u64,
    /// Por qué se descartó el camino por bloques la última vez que se descartó ("" si no).
    pub motivo_sin_bloque: &'static str,
    /// Decode greedy con borrador PLD (por defecto sí). Apagarlo hace un forward por token generado.
    pub borrador_activo: bool,
    /// Verifica el borrador con **un** forward por bloque (por defecto sí); apagado, token a token.
    pub borrador_por_bloques: bool,
    /// Forwards de decode lanzados (uno por token, o uno por bloque de verificación).
    pub forwards_decode: u64,
    /// Tokens de borrador enviados a verificar y cuántos se aceptaron (diagnóstico/pruebas).
    pub borrador_ofrecidos: u64,
    pub borrador_aceptados: u64,
    /// Reloj en milisegundos para medir lo que cuesta cada ronda y elegir cuánto borrador verificar. Sin él,
    /// se verifica el borrador entero. Lo fija quien lanza la generación (`generate_stream_planned_observed`
    /// usa el suyo).
    pub reloj: Option<fn() -> u64>,
    /// Coste por longitud de ronda y acierto del borrador por posición (ver `especulativo`).
    pub politica: crate::especulativo::PoliticaBorrador,
    /// Estados ocultos de los tokens del último bloque de verificación (`n × hidden`).
    hs_bloque: Vec<f32>,
    /// Logits de varias posiciones (verificación por bloque), reutilizado entre rondas.
    logits_lote: Vec<f32>,
    pub backend: Backend,
    scratch: LayerScratch,
    logits_buf: Vec<f32>,
    pub decode_gpu: decode_gpu::DecGpu,
    has_gate: bool,
    has_lm_head: bool,
    has_output_norm: bool,
    pub planner: Option<ResourcePlanner>,
    /// Tras cada capa (`layer` 0-based, `n_layers`). `askd` lo usa para no
    /// parecer colgado: Mixtral en CPU tarda minutos *por token*.
    pub layer_hook: Option<fn(u32, u32)>,
    /// Antes de cada capa; `askd` emite un punto al entrar (p. ej. capa 0).
    pub layer_enter_hook: Option<fn(u32, u32)>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    Cpu,
    Gpu,
    Auto,
}

impl Runtime {
    pub fn new(manifest: Manifest, index: TensorIndex, ram_budget: usize, vram_budget: usize) -> Self {
        let h = manifest.hidden_dim as usize;
        let vocab = manifest.vocab_size as usize;
        let n = manifest.num_layers as usize;
        let scratch = LayerScratch::new(&manifest);
        let has_gate = index.find("L00.ffn_gate").is_some();
        let has_lm_head = index.find("lm_head").is_some();
        let has_output_norm = index.find("output_norm").is_some();
        let kv_cap = (manifest.max_seq as usize).min(256).max(32);
        let manifest_for_kv = manifest.clone();
        Self {
            manifest,
            index,
            tiers: TierManager::new(ram_budget, vram_budget),
            hidden: vec![0.0f32; h],
            kv: (0..n)
                .map(|l| {
                    make_layer_kv(
                        &manifest_for_kv,
                        l as u32,
                        kv_cap,
                        crate::kv::KvDtype::F16,
                    )
                })
                .collect(),
            pos: 0,
            cached_tokens: Vec::new(),
            gdn_instantaneas: Vec::new(),
            proxima_marca_gdn: Self::INTERVALO_INSTANTANEAS_GDN,
            block: None,
            prefill_por_bloques: true,
            tokens_en_bloque: 0,
            motivo_sin_bloque: "",
            borrador_activo: true,
            borrador_por_bloques: true,
            forwards_decode: 0,
            borrador_ofrecidos: 0,
            borrador_aceptados: 0,
            reloj: None,
            politica: crate::especulativo::PoliticaBorrador::new(),
            hs_bloque: Vec::new(),
            logits_lote: Vec::new(),
            backend: Backend::Auto,
            scratch,
            logits_buf: vec![0.0f32; vocab],
            has_gate,
            has_lm_head,
            has_output_norm,
            planner: None,
            layer_hook: None,
            layer_enter_hook: None,
            decode_gpu: Default::default(),
        }
    }

    pub fn set_planner(&mut self, planner: ResourcePlanner) {
        // Antes del primer token: alinear dtype KV (KIVI-lite) con el planner.
        if self.pos == 0 {
            let dtype = planner.kv_dtype();
            let kv_cap = planner.kv_window_tokens().min(256).max(32);
            let manifest = self.manifest.clone();
            self.kv = (0..self.manifest.num_layers as usize)
                .map(|l| make_layer_kv(&manifest, l as u32, kv_cap, dtype))
                .collect();
        }
        self.planner = Some(planner);
    }

    /// Comprueba que las shapes del index casan con lo que espera el ejecutor
    /// (convención row-major `[filas, columnas]` = `[out_dim, in_dim]`).
    pub fn validate_shapes(&self) -> Result<(), ()> {
        self.manifest.supported_by_runtime().map_err(|_| ())?;
        self.validate_shapes_for_role(PipelineRole::Full, 0, self.manifest.num_layers)
            .map_err(|_| ())
    }

    /// Valida solo los tensores necesarios para un rol/rango de capas.
    pub fn validate_shapes_for_role(
        &self,
        role: PipelineRole,
        layer_start: u32,
        layer_end: u32,
    ) -> Result<(), String> {
        let h = self.manifest.hidden_dim;
        let vocab = self.manifest.vocab_size;

        let check = |name: &str, want: &[u32], required: bool| -> Result<(), String> {
            match self.index.find(name) {
                Some(e) => {
                    if e.shape == want {
                        Ok(())
                    } else {
                        Err(alloc::format!("{name}: shape {:?} vs {want:?}", e.shape))
                    }
                }
                None if required => Err(alloc::format!("{name}: falta")),
                None => Ok(()),
            }
        };

        let needs_embed = matches!(role, PipelineRole::Head | PipelineRole::Full);
        let needs_logits = matches!(role, PipelineRole::Tail | PipelineRole::Full);
        check("embed", &[vocab, h], needs_embed)?;
        // Con `tied_lm_head` no hay `lm_head`: los logits salen de `embed`.
        check("lm_head", &[vocab, h], false)?;
        if self.manifest.ext.tied_lm_head && self.index.find("lm_head").is_some() {
            return Err(String::from("lm_head: sobra (el manifest declara embeddings atados)"));
        }
        check("output_norm", &[h], needs_logits && self.has_output_norm)?;
        for layer in layer_start..layer_end {
            let p = alloc::format!("L{layer:02}");
            let spec = self.manifest.layer(layer).cloned().unwrap_or_default();
            let layer_heads = self.manifest.effective_num_heads(layer);
            let layer_kv_heads = self.manifest.effective_num_kv_heads(layer);
            let head_dim = self.manifest.effective_head_dim(layer);
            let kv_dim = layer_kv_heads * head_dim;
            let ffn = self.manifest.effective_ffn_dim(layer);
            let moe_ffn = self.manifest.effective_moe_ffn_dim(layer);
            let is_moe = self.manifest.layer_is_moe(layer)
                || spec.ffn_kind == sosomodel::FfnKind::LatentMoe;
            let n_exp = self.manifest.effective_num_experts(layer);
            let n_shared = spec.num_shared_experts;
            check(&alloc::format!("{p}.attn_norm"), &[h], true)?;
            if spec.attn_kind == sosomodel::AttnKind::Mla {
                let q_rank = spec.q_lora_rank;
                let kv_rank = spec.kv_lora_rank;
                let qk_per = if spec.qk_nope_head_dim > 0 || spec.qk_rope_head_dim > 0 {
                    spec.qk_nope_head_dim + spec.qk_rope_head_dim
                } else {
                    head_dim as u32
                };
                let v_dim = if spec.v_head_dim > 0 {
                    spec.v_head_dim
                } else {
                    qk_per
                };
                let kv_qk = layer_kv_heads * qk_per;
                let kv_v = layer_kv_heads * v_dim;
                check(&alloc::format!("{p}.attn_q_down"), &[q_rank, h], true)?;
                check(&alloc::format!("{p}.attn_q_up"), &[h, q_rank], true)?;
                check(&alloc::format!("{p}.attn_kv_down"), &[kv_rank, h], true)?;
                check(&alloc::format!("{p}.attn_k_up"), &[kv_qk, kv_rank], true)?;
                check(&alloc::format!("{p}.attn_v_up"), &[kv_v, kv_rank], true)?;
                check(&alloc::format!("{p}.attn_output"), &[h, h], true)?;
            } else if spec.attn_kind == sosomodel::AttnKind::Gated {
                let q_rows = layer_heads * head_dim * 2;
                let attn_in = layer_heads * head_dim;
                check(&alloc::format!("{p}.attn_q"), &[q_rows, h], true)?;
                check(&alloc::format!("{p}.attn_k"), &[kv_dim, h], true)?;
                check(&alloc::format!("{p}.attn_v"), &[kv_dim, h], true)?;
                check(&alloc::format!("{p}.attn_output"), &[h, attn_in], true)?;
                check(&alloc::format!("{p}.attn_q_norm"), &[head_dim], true)?;
                check(&alloc::format!("{p}.attn_k_norm"), &[head_dim], true)?;
            } else if spec.attn_kind == sosomodel::AttnKind::Gdn {
                let n_k = layer_heads;
                let n_v = layer_kv_heads;
                let d = head_dim;
                let qkv = n_k * d * 2 + n_v * d;
                let kernel = spec.qk_rope_head_dim.max(1);
                check(&alloc::format!("{p}.attn_qkv"), &[qkv, h], true)?;
                check(&alloc::format!("{p}.attn_gate"), &[n_v * d, h], true)?;
                check(&alloc::format!("{p}.ssm_conv1d"), &[qkv, kernel], true)?;
                check(&alloc::format!("{p}.ssm_dt"), &[n_v], true)?;
                check(&alloc::format!("{p}.ssm_a"), &[n_v], true)?;
                check(&alloc::format!("{p}.ssm_beta"), &[n_v, h], true)?;
                check(&alloc::format!("{p}.ssm_alpha"), &[n_v, h], true)?;
                check(&alloc::format!("{p}.ssm_norm"), &[d], true)?;
                check(&alloc::format!("{p}.ssm_out"), &[h, n_v * d], true)?;
            } else {
                let q_dim = layer_heads * head_dim;
                check(&alloc::format!("{p}.attn_q"), &[q_dim, h], true)?;
                check(&alloc::format!("{p}.attn_k"), &[kv_dim, h], true)?;
                // Gemma, capas globales: sin `attn_v` (el valor sale de la proyección K).
                let v_de_k = spec.flags & sosomodel::FLAG_V_FROM_K != 0;
                check(&alloc::format!("{p}.attn_v"), &[kv_dim, h], !v_de_k)?;
                if v_de_k && self.index.find(&alloc::format!("{p}.attn_v")).is_some() {
                    return Err(alloc::format!("{p}.attn_v: sobra (la capa declara V = K)"));
                }
                check(&alloc::format!("{p}.attn_output"), &[h, q_dim], true)?;
                if spec.flags & sosomodel::FLAG_QK_NORM != 0 {
                    check(&alloc::format!("{p}.attn_q_norm"), &[head_dim], true)?;
                    check(&alloc::format!("{p}.attn_k_norm"), &[head_dim], true)?;
                }
                if spec.flags & sosomodel::FLAG_SANDWICH_NORM != 0 {
                    check(&alloc::format!("{p}.post_attn_norm"), &[h], true)?;
                    check(&alloc::format!("{p}.post_ffn_norm"), &[h], true)?;
                }
                if spec.flags & sosomodel::FLAG_LAYER_SCALE != 0 {
                    check(&alloc::format!("{p}.layer_scale"), &[1], true)?;
                }
                if spec.flags & sosomodel::FLAG_ROPE_FACTORS != 0 {
                    check("rope_freqs", &[head_dim / 2], true)?;
                }
            }
            check(&alloc::format!("{p}.ffn_norm"), &[h], true)?;
            if spec.ffn_kind == sosomodel::FfnKind::LatentMoe {
                let latent = moe_ffn;
                check(&alloc::format!("{p}.ffn_latent_in"), &[latent, h], true)?;
                check(&alloc::format!("{p}.ffn_latent_out"), &[h, latent], true)?;
                check(&alloc::format!("{p}.ffn_gate_inp"), &[n_exp, latent], true)?;
                for e in 0..n_exp {
                    let ep = alloc::format!("{p}.E{e:02}");
                    check(&alloc::format!("{ep}.ffn_gate"), &[latent, latent], true)?;
                    check(&alloc::format!("{ep}.ffn_up"), &[latent, latent], true)?;
                    check(&alloc::format!("{ep}.ffn_down"), &[latent, latent], true)?;
                }
            } else if is_moe {
                check(
                    &alloc::format!("{p}.ffn_gate_inp"),
                    &[n_exp, h],
                    true,
                )?;
                for e in 0..n_exp {
                    let ep = alloc::format!("{p}.E{e:02}");
                    check(&alloc::format!("{ep}.ffn_gate"), &[moe_ffn, h], true)?;
                    check(&alloc::format!("{ep}.ffn_up"), &[moe_ffn, h], true)?;
                    check(&alloc::format!("{ep}.ffn_down"), &[h, moe_ffn], true)?;
                }
                for s in 0..n_shared {
                    let sp = alloc::format!("{p}.S{s:02}");
                    check(&alloc::format!("{sp}.ffn_gate"), &[moe_ffn, h], true)?;
                    check(&alloc::format!("{sp}.ffn_up"), &[moe_ffn, h], true)?;
                    check(&alloc::format!("{sp}.ffn_down"), &[h, moe_ffn], true)?;
                }
            } else {
                check(&alloc::format!("{p}.ffn_up"), &[ffn, h], true)?;
                check(&alloc::format!("{p}.ffn_down"), &[h, ffn], true)?;
                check(&alloc::format!("{p}.ffn_gate"), &[ffn, h], self.has_gate)?;
            }
        }
        Ok(())
    }

    pub fn set_backend(&mut self, backend: Backend) {
        self.backend = backend;
    }

    pub fn backend(&self) -> Backend {
        self.backend
    }

    /// Recorta el KV local a `pos` tokens (rollback tras borrador rechazado o failover).
    pub fn truncar_kv(&mut self, pos: usize) {
        for l in 0..self.kv.len() {
            let d = self.kv_dim_recortable(l).unwrap_or(0);
            self.kv[l].truncate_tokens(pos, d);
        }
        self.pos = pos;
    }

    pub fn reset_sequence(&mut self) {
        self.decode_gpu_descartar();
        self.pos = 0;
        self.cached_tokens.clear();
        self.gdn_instantaneas.clear();
        self.proxima_marca_gdn = Self::INTERVALO_INSTANTANEAS_GDN;
        for layer in &mut self.kv {
            layer.reset();
        }
        if let Some(pl) = self.planner.as_mut() {
            pl.reset_moe_hints();
        }
    }

    /// Copia posición, oculto y caché KV/GDN tras codificar el estado Kev.
    pub fn snapshot_kev_prefix(&self) -> KevPrefixSnapshot {
        KevPrefixSnapshot {
            pos: self.pos,
            hidden: self.hidden.clone(),
            kv: self.kv.clone(),
        }
    }

    /// Restaura un prefijo Kev antes de procesar otra fila de pregunta.
    pub fn restore_kev_prefix(&mut self, snap: &KevPrefixSnapshot) {
        self.pos = snap.pos;
        self.hidden.copy_from_slice(&snap.hidden);
        self.kv.clone_from(&snap.kv);
        self.cached_tokens.clear();
    }

    /// Ancho de K/V por token de una capa de atención clásica; `None` si la
    /// capa guarda otra cosa (MLA, GDN) y no admite recorte.
    fn kv_dim_recortable(&self, layer: usize) -> Option<usize> {
        let spec = self.manifest.layer(layer as u32).cloned().unwrap_or_default();
        if spec.attn_kind == AttnKind::Gdn
            || (spec.attn_kind == AttnKind::Mla && spec.kv_lora_rank > 0)
        {
            return None;
        }
        let l = layer as u32;
        Some(
            self.manifest.effective_num_kv_heads(l) as usize
                * self.manifest.effective_head_dim(l) as usize,
        )
    }

    /// Cuántos tokens del principio de `prompt` ya tienen su K/V en la caché.
    ///
    /// Por causalidad, el K/V de los primeros `p` tokens depende sólo de ellos,
    /// así que recortar la caché a `p` deja exactamente el estado de un prefill
    /// limpio de `prompt[..p]`. Siempre queda al menos el último token por
    /// procesar: hace falta para obtener los logits.
    fn prefijo_reutilizable(&self, prompt: &[u32]) -> usize {
        // Con planificador sólo vale un modelo denso: los hints de expertos de un
        // MoE son estado de la secuencia anterior y no se pueden recortar. La
        // expulsión de K/V (ventana deslizante) la delata `tokens != pos` abajo.
        if (self.planner.is_some() && self.manifest.num_experts > 0)
            || self.cached_tokens.is_empty()
            || self.cached_tokens.len() != self.pos
            || prompt.len() < 2
        {
            return 0;
        }
        // Con ventana deslizante real (Gemma), una capa que ya expulsó filas sólo puede volver a una posición
        // `p` si conserva las `ventana` filas que preceden a `p`: de ahí el suelo `minimo`.
        let mut minimo = 0usize;
        let hay_gdn = self.hay_gdn();
        for (l, kv) in self.kv.iter().enumerate() {
            match self.kv_dim_recortable(l) {
                Some(d) if kv.posiciones(d) == self.pos => {}
                None if hay_gdn && self.manifest.layer(l as u32).is_some_and(|sp| sp.attn_kind == AttnKind::Gdn) => continue,
                _ => return 0,
            }
            if kv.dropped > 0 {
                minimo = minimo.max(kv.dropped + self.manifest.layer_window(l as u32) as usize);
            }
        }
        let comun = self
            .cached_tokens
            .iter()
            .zip(prompt)
            .take_while(|(a, b)| a == b)
            .count();
        let mut p = comun.min(prompt.len() - 1);
        if hay_gdn {
            // Sólo se puede volver a un punto con instantánea: la mayor posición guardada que sea prefijo del prompt nuevo
            // y cuyo K/V siga en la caché de las capas de atención.
            p = match self.instantanea_para(prompt, p) {
                Some(i) => self.gdn_instantaneas[i].tokens.len(),
                None => 0,
            };
        }
        if p < minimo {
            return 0;
        }
        p
    }

    /// Cada cuántos tokens de prefill se guarda una instantánea intermedia del estado GDN, y cuántas se conservan como mucho. Si el
    /// arnés cambia el principio de la conversación (pliega mensajes antiguos), sólo se puede volver a una instantánea anterior al
    /// primer token distinto: con sólo la del final del prompt había que reprocesar todo (8 min con 20 000 tokens).
    pub const INTERVALO_INSTANTANEAS_GDN: usize = 2048;
    const MAX_INSTANTANEAS_GDN: usize = 16;

    fn acotar_instantaneas(&mut self) {
        while self.gdn_instantaneas.len() > Self::MAX_INSTANTANEAS_GDN {
            self.gdn_instantaneas.sort_by_key(|i| i.tokens.len());
            // Se quita una de cada dos, salvo la primera y la última.
            let mut k = 0usize;
            let n = self.gdn_instantaneas.len();
            self.gdn_instantaneas.retain(|_| {
                k += 1;
                k == 1 || k == n || k % 2 == 1
            });
        }
    }

    /// Instantánea intermedia durante el prefill (`ctx` son todos los tokens del prompt).
    fn instantanea_intermedia_gdn(&mut self, ctx: Option<&[u32]>) {
        if self.pos < self.proxima_marca_gdn || !self.hay_gdn() {
            return;
        }
        self.proxima_marca_gdn = (self.pos / Self::INTERVALO_INSTANTANEAS_GDN + 1) * Self::INTERVALO_INSTANTANEAS_GDN;
        if let Some(c) = ctx.filter(|c| c.len() >= self.pos) {
            let tokens = c[..self.pos].to_vec();
            self.guardar_instantanea_gdn(&tokens);
        }
    }

    fn hay_gdn(&self) -> bool {
        (0..self.manifest.num_layers).any(|l| self.manifest.layer(l).is_some_and(|sp| sp.attn_kind == AttnKind::Gdn))
    }

    /// Índice de la instantánea de mayor posición `<= limite` cuyos tokens son prefijo de `prompt`.
    fn instantanea_para(&self, prompt: &[u32], limite: usize) -> Option<usize> {
        self.gdn_instantaneas
            .iter()
            .enumerate()
            .filter(|(_, i)| i.tokens.len() <= limite && i.tokens.len() < prompt.len() && prompt.starts_with(&i.tokens))
            .max_by_key(|(_, i)| i.tokens.len())
            .map(|(k, _)| k)
    }

    /// Guarda el estado GDN tras el prefill de `prompt` (llamar con `self.pos == prompt.len()`).
    fn guardar_instantanea_gdn(&mut self, prompt: &[u32]) {
        if !self.hay_gdn() || self.pos != prompt.len() {
            return;
        }
        let capas: Vec<(usize, Vec<f32>, Vec<f32>)> = self
            .kv
            .iter()
            .enumerate()
            .filter(|(_, kv)| !kv.gdn_s.is_empty())
            .map(|(l, kv)| (l, kv.gdn_s.clone(), kv.gdn_conv.clone()))
            .collect();
        self.gdn_instantaneas.retain(|i| i.tokens != prompt);
        self.gdn_instantaneas.insert(0, InstantaneaGdn { tokens: prompt.to_vec(), capas });
        self.acotar_instantaneas();
    }

    /// Empieza una secuencia reaprovechando el prefijo de la anterior. Devuelve
    /// cuántos tokens del prompt no hay que volver a procesar.
    pub fn begin_sequence_reusing(&mut self, prompt: &[u32]) -> usize {
        self.decode_gpu_descartar();
        let p = self.prefijo_reutilizable(prompt);
        if p == 0 {
            self.reset_sequence();
            return 0;
        }
        if self.hay_gdn() {
            let Some(i) = self.instantanea_para(prompt, p) else {
                self.reset_sequence();
                return 0;
            };
            let snap = self.gdn_instantaneas[i].clone();
            // Las instantáneas más allá de `p` son de la continuación que se abandona.
            self.gdn_instantaneas.retain(|x| x.tokens.len() <= p);
            self.proxima_marca_gdn = (p / Self::INTERVALO_INSTANTANEAS_GDN + 1) * Self::INTERVALO_INSTANTANEAS_GDN;
            for (l, s, c) in &snap.capas {
                self.kv[*l].gdn_s.clone_from(s);
                self.kv[*l].gdn_conv.clone_from(c);
            }
        }
        for l in 0..self.kv.len() {
            if let Some(d) = self.kv_dim_recortable(l) {
                self.kv[l].truncate_tokens(p, d);
            }
        }
        self.pos = p;
        // Durante la generación la caché no es fiable hasta que termina bien.
        self.cached_tokens.clear();
        p
    }

    /// Anota qué tokens quedan en la caché al terminar una generación completa.
    fn recordar_cache(&mut self, tokens: &[u32]) {
        self.cached_tokens = if self.pos <= tokens.len() {
            tokens[..self.pos].to_vec()
        } else {
            Vec::new()
        };
    }

    pub fn embed_token(&mut self, token: u32, source: &mut impl TensorSource) -> Result<(), ()> {
        let h = self.manifest.hidden_dim as usize;
        if token >= self.manifest.vocab_size {
            note_infer_op(None, "embed");
            return Err(());
        }
        note_infer_op(None, "embed");
        source.load_f32_range("embed", token as usize * h, &mut self.hidden)?;
        // Gemma multiplica los embeddings de entrada por sqrt(hidden).
        let k = self.manifest.embed_scale();
        if k != 1.0 {
            for v in self.hidden.iter_mut() {
                *v *= k;
            }
        }
        Ok(())
    }

    /// Límite blando de los logits (`cap·tanh(x/cap)`), si el modelo lo declara.
    fn softcap_logits(&mut self) {
        let cap = self.manifest.ext.logit_softcap;
        if cap > 0.0 {
            for v in self.logits_buf.iter_mut() {
                *v = cap * libm::tanhf(*v / cap);
            }
        }
    }

    /// Prefetch de la fila de embed (solapa I/O con el forward del token actual).
    pub fn prefetch_embed(&mut self, token: u32, source: &mut impl TensorSource) {
        let h = self.manifest.hidden_dim as usize;
        if token < self.manifest.vocab_size {
            source.prefetch_embed_row(token, h);
        }
    }

    /// Tokens por bloque en el prefill (T82). 8 son los acumuladores que caben en los
    /// registros YMM de los kernels por lotes; con más baja el rendimiento por caché.
    pub const BLOQUE_PREFILL: usize = 8;
    /// Bloque del prefill de un modelo (MoE o Gemma) que la GPU hace entero: más tokens por bloque = cada experto lee sus pesos
    /// una vez para varios tokens.
    pub const BLOQUE_PREFILL_MOE_GPU: usize = 64;
    /// Máximo del bloque con capas MoE «en flujo».
    pub const BLOQUE_PREFILL_FLUJO: usize = 256;
    /// Bloque cuando los expertos no residentes pasan por el bus (cada bloque los sube una vez: cuanto mayor, menos viajes).
    pub const BLOQUE_PREFILL_STREAM: usize = 1024;

    /// Tokens por bloque de prefill según el modelo y el despacho.
    /// Tokens por bloque del prefill de Qwen3.5/3.6 (`SOSO_BLOQUE_PREFILL` lo cambia): con expertos que pasan por el bus, cuanto mayor, menos viajes.
    pub fn bloque_flujo() -> usize {
        Self::bloque_forzado().unwrap_or(Self::BLOQUE_PREFILL_FLUJO)
    }

    fn bloque_forzado() -> Option<usize> {
        #[cfg(feature = "std")]
        if let Some(n) = std::env::var("SOSO_BLOQUE_PREFILL").ok().and_then(|v| v.parse::<usize>().ok()) {
            return Some(n.max(8));
        }
        None
    }

    fn bloque_prefill(&self, gpu: &Option<&mut dyn crate::gpu::GpuDispatch>) -> usize {
        let base_gpu = matches!(self.backend, Backend::Gpu | Backend::Auto)
            && gpu.as_ref().is_some_and(|g| g.available());
        let exec = LayerExecutor { manifest: &self.manifest, has_gate: self.has_gate, parallel: None };
        let moe_gpu = base_gpu
            && gpu.as_ref().is_some_and(|g| g.moe_puede(0))
            && (0..self.manifest.num_layers).all(|l| exec.es_capa_moe_gpu(l));
        let gemma_gpu = base_gpu
            && gpu.as_ref().is_some_and(|g| g.gemma_puede(0))
            && self.manifest.layer(0).is_some_and(|l| crate::gemma::es_capa_gemma(l.flags));
        if moe_gpu && gpu.as_ref().is_some_and(|g| g.moe_flujo_activo()) {
            // Con capas «en flujo» cada bloque lee de RAM los expertos de esas capas una vez: cuanto mayor el bloque, menos
            // lecturas por token. Lo limita el búfer de parciales de la atención, que crece con n × (contexto / 256).
            let por_token = (self.pos / 256 + 1) * 16384;
            let max = (160usize << 20) / por_token.max(1);
            max.clamp(Self::BLOQUE_PREFILL_MOE_GPU, Self::BLOQUE_PREFILL_FLUJO).next_power_of_two().min(Self::BLOQUE_PREFILL_FLUJO)
        } else if moe_gpu {
            Self::BLOQUE_PREFILL_MOE_GPU
        } else if gemma_gpu {
            32
        } else if (0..self.manifest.num_layers).all(|l| crate::bloque35::soporta(&exec, l, &self.kv[l as usize])) {
            // Qwen3.5: cada experto/matriz lee sus pesos una vez por bloque, así que cuanto mayor, mejor.
            Self::bloque_forzado().or_else(|| gpu.as_ref().and_then(|g| g.bloque_prefill_moe())).unwrap_or(Self::BLOQUE_PREFILL_FLUJO)
        } else {
            Self::BLOQUE_PREFILL
        }
    }

    /// ¿Puede ejecutarse el siguiente bloque de `n` tokens con `forward_layer_block`?
    ///
    /// Sólo si el resultado va a ser idéntico al de `n` pasos de un token: atención
    /// clásica con KV f16 y FFN denso en todas las capas, sin GPU ni capas remotas, con
    /// los pesos residentes (nada que descargar entre capas) y sin que la ventana de KV
    /// deslice a mitad de bloque.
    fn bloque_posible(
        &mut self,
        n: usize,
        gpu: &Option<&mut dyn crate::gpu::GpuDispatch>,
    ) -> bool {
        match self.motivo_para_no_usar_bloque(n, gpu) {
            None => true,
            Some(m) => {
                self.motivo_sin_bloque = m;
                false
            }
        }
    }

    fn motivo_para_no_usar_bloque(
        &self,
        n: usize,
        gpu: &Option<&mut dyn crate::gpu::GpuDispatch>,
    ) -> Option<&'static str> {
        if n < 2 {
            return Some("quedan menos de 2 tokens");
        }
        if !self.prefill_por_bloques {
            return Some("desactivado");
        }
        let base_gpu = matches!(self.backend, Backend::Gpu | Backend::Auto)
            && gpu.as_ref().is_some_and(|g| g.available());
        // Con GPU, Gemma sigue por bloques (su capa por bloques usa `matmul` de la GPU); el resto de modelos
        // van token a token por el camino de GPU de siempre.
        let es_gemma = self.manifest.layer(0).is_some_and(|l| crate::gemma::es_capa_gemma(l.flags));
        // Y las MoE de GQA clásica: la GPU las hace enteras por bloques (con fallback token a token si no puede).
        let es_moe_gpu = base_gpu
            && gpu.as_ref().is_some_and(|g| g.moe_puede(0))
            && (0..self.manifest.num_layers).all(|l| {
                LayerExecutor { manifest: &self.manifest, has_gate: self.has_gate, parallel: None }.es_capa_moe_gpu(l)
            });
        // Qwen3.5/3.6 (GDN + atención con puerta): por bloques en CPU aunque haya GPU.
        let exec0 = LayerExecutor { manifest: &self.manifest, has_gate: self.has_gate, parallel: None };
        let todas_q35 = (0..self.manifest.num_layers).all(|l| crate::bloque35::soporta(&exec0, l, &self.kv[l as usize]));
        if base_gpu && !es_gemma && !es_moe_gpu && !todas_q35 {
            return Some("hay GPU");
        }
        if let Some(pl) = self.planner.as_ref() {
            if !pl.keep_weights_mapped() {
                return Some("el planificador no mantiene los pesos residentes");
            }
            if self.pos + n > pl.kv_window_tokens() {
                return Some("la ventana de KV es más corta que el prompt");
            }
            if (0..self.manifest.num_layers).any(|l| pl.layer_dest(l) != ExecDest::Cpu) {
                return Some("alguna capa no va en CPU");
            }
        } else if self.pos + n > self.manifest.max_seq as usize {
            return Some("supera max_seq");
        }
        let exec = LayerExecutor {
            manifest: &self.manifest,
            has_gate: self.has_gate,
            parallel: None,
        };
        for l in 0..self.manifest.num_layers {
            let kv = &self.kv[l as usize];
            let kv_dim = self.manifest.effective_num_kv_heads(l) as usize
                * self.manifest.effective_head_dim(l) as usize;
            // Con GPU, las capas de Gemma (con ventana) y las MoE de GQA clásica van por bloques por su propio camino
            // (`forward_layer_gemma_block` / `moe_gpu`), que cae a token a token por CPU si la GPU las rechaza.
            let es_gemma_l = self.manifest.layer(l).is_some_and(|sp| crate::gemma::es_capa_gemma(sp.flags));
            let q35 = crate::bloque35::soporta(&exec, l, kv);
            if !q35 && !exec.soporta_bloque(l, kv) && !(base_gpu && (exec.es_capa_moe_gpu(l) || es_gemma_l)) {
                return Some("capa no soportada (MLA/KDA/gated/GDN/MoE)");
            }
            // Las capas GDN no guardan K/V: su posición no es la del bloque.
            let sin_kv = q35 && self.manifest.layer(l).is_some_and(|sp| sp.attn_kind == AttnKind::Gdn);
            if !sin_kv && kv.posiciones(kv_dim) != self.pos {
                return Some("el KV ya no coincide con la posición");
            }
        }
        None
    }

    /// Procesa `tokens` de una vez (prefill por bloques). Devuelve `false` si el
    /// observador canceló a mitad (la caché queda a medias y no se anota).
    fn forward_bloque(
        &mut self,
        source: &mut impl TensorSource,
        tokens: &[u32],
        parallel: Option<&dyn RowParallel>,
        ledger: &mut GenerationLedger,
        observer: &mut dyn GenerationObserver,
        cancel_slot: &mut Option<&mut GenCancel>,
        gpu: &mut Option<&mut dyn crate::gpu::GpuDispatch>,
        conservar: bool,
        cluster: &mut Option<&mut crate::remoto::ClusterGen<'_>>,
        hash_context: Option<&[u32]>,
    ) -> Result<bool, ()> {
        let n = tokens.len();
        let use_gpu = matches!(self.backend, Backend::Gpu | Backend::Auto)
            && gpu.as_ref().is_some_and(|g| g.available());
        let h = self.manifest.hidden_dim as usize;
        let mut bs = self
            .block
            .take()
            .unwrap_or_else(|| BlockScratch::new(&self.manifest, Self::bloque_flujo().max(Self::BLOQUE_PREFILL_STREAM)));
        let mut hs = vec![0.0f32; n * h];
        for (i, &tok) in tokens.iter().enumerate() {
            self.embed_token(tok, source)?;
            hs[i * h..(i + 1) * h].copy_from_slice(&self.hidden);
        }
        let exec = LayerExecutor {
            manifest: &self.manifest,
            has_gate: self.has_gate,
            parallel,
        };
        let local_end = cluster
            .as_ref()
            .map(|c| c.local_end)
            .unwrap_or(self.manifest.num_layers);
        let mut seguir = true;
        for layer in 0..local_end {
            note_infer_op(Some(layer), "");
            if let Some(hook) = self.layer_enter_hook {
                hook(layer, local_end);
            }
            if let Some(g) = cancel_slot.as_mut() {
                if g.checkpoint(observer, ledger, GenCheckpoint::Layer { layer, of: local_end }) {
                    seguir = false;
                    break;
                }
            }
            let r = exec.forward_layer_block(
                layer,
                self.pos,
                n,
                &mut hs,
                &mut bs,
                &mut self.scratch,
                &mut self.kv[layer as usize],
                source,
                self.planner.as_ref(),
                gpu,
                use_gpu,
            );
            if r.is_err() {
                self.block = Some(bs);
                if let Some(g) = gpu.as_deref_mut() {
                    let _ = g.res_volcar(&mut hs);
                }
                return Err(());
            }
            if let Some(hook) = self.layer_hook {
                hook(layer, local_end);
            }
        }
        self.block = Some(bs);
        if let Some(g) = gpu.as_deref_mut() {
            g.res_volcar(&mut hs)?; // el residual de la última capa puede estar sólo en el dispositivo
        }
        if !seguir {
            return Ok(false);
        }
        if let Some(crate::remoto::ClusterGen {
            remoto,
            journal,
            prefix_hash,
            ..
        }) = cluster.as_deref_mut()
        {
            let pos0 = self.pos;
            let end = pos0 + n;
            *prefix_hash = if let Some(ctx) = hash_context {
                if end <= ctx.len() {
                    crate::pipeline::prefix_hash_tokens(&ctx[..end])
                } else {
                    let extra = end - ctx.len();
                    let mut ids = ctx.to_vec();
                    ids.extend_from_slice(&tokens[..extra.min(n)]);
                    crate::pipeline::prefix_hash_tokens(&ids)
                }
            } else {
                crate::pipeline::prefix_hash_tokens(&tokens[..n.min(end)])
            };
            if let Some(j) = journal {
                j.registrar(pos0 as u32, n as u32, &hs);
            }
            remoto.forward(pos0, &mut hs, n, h, *prefix_hash)?;
        }
        if conservar {
            // Verificación de un borrador: el K/V ya está añadido, pero `pos`, `hidden` y el planificador los
            // fija quien decida cuántos de estos tokens valen (`cerrar_bloque_especulativo`).
            self.hs_bloque = hs;
            return Ok(true);
        }
        // El último token deja su estado final en `hidden`, como el paso de un token.
        self.hidden.copy_from_slice(&hs[(n - 1) * h..n * h]);
        self.pos += n;
        self.tokens_en_bloque += n as u64;
        for _ in 0..n {
            if let Some(pl) = self.planner.as_mut() {
                let _ = pl.on_token_complete(&self.manifest, &self.index);
            }
        }
        self.slide_kv_if_needed();
        Ok(true)
    }

    fn ahora_ms(&self) -> u64 {
        self.reloj.map_or(0, |r| r())
    }

    /// Recorta el borrador a lo que la política cree que compensa (todo si aún no hay datos o no hay reloj).
    fn recortar_borrador(&mut self, drafts: &mut Vec<u32>) {
        if self.reloj.is_some() && !drafts.is_empty() {
            let k = self.politica.elegir_k(drafts.len());
            drafts.truncate(k);
        }
    }

    /// Anota una ronda terminada: su coste (si el reloj está y la longitud es comparable) y el acierto del borrador.
    fn anotar_ronda(&mut self, t0: u64, posiciones: usize, ofrecidos: usize, aceptados: usize) {
        if self.reloj.is_some() {
            let dt = self.ahora_ms().saturating_sub(t0);
            self.politica.registrar_coste(posiciones, dt);
        }
        if ofrecidos > 0 {
            self.politica.registrar_ronda(ofrecidos, aceptados);
        }
    }

    /// Tokens de borrador que caben en un bloque de verificación (el bloque lleva además el token ya muestreado).
    pub fn max_borrador_bloque(&self, gpu: &Option<&mut dyn crate::gpu::GpuDispatch>) -> usize {
        self.bloque_prefill(gpu).saturating_sub(1)
    }

    /// ¿Se puede verificar un borrador de `k` tokens con un bloque y deshacer luego lo rechazado?
    ///
    /// Además de lo que exige `bloque_posible` (resultado idéntico al de `n` pasos de un token), el deshacer
    /// pide que el estado sea recortable: K/V de GQA clásica sin ventana ya expulsada por delante del punto de
    /// vuelta, sin masa H2O acumulada (las consultas rechazadas ya habrían sumado) y sin pistas de expertos
    /// de un MoE con planificador.
    fn borrador_en_bloque_posible(&mut self, k: usize, gpu: &Option<&mut dyn crate::gpu::GpuDispatch>) -> bool {
        if !self.borrador_por_bloques || k == 0 || k > self.max_borrador_bloque(gpu) {
            return false;
        }
        if let Some(pl) = self.planner.as_ref() {
            if pl.use_h2o() || self.manifest.num_experts > 0 {
                return false;
            }
        }
        // Tras el rechazo total el K/V vuelve a `pos + 1`: tiene que seguir teniendo la ventana de cada capa.
        let mut minimo = 0usize;
        for (l, kv) in self.kv.iter().enumerate() {
            if self.kv_dim_recortable(l).is_none() {
                return false;
            }
            if kv.dropped > 0 {
                minimo = minimo.max(kv.dropped + self.manifest.layer_window(l as u32) as usize);
            }
        }
        if self.pos + 1 < minimo {
            return false;
        }
        self.bloque_posible(k + 1, gpu)
    }

    /// Posición más probable (argmax) tras cada uno de los `n` primeros estados de `hs_bloque`.
    ///
    /// En CPU, un solo `matvec` por lotes sobre la cabeza de logits (lee sus pesos una vez). Con GPU se hace un
    /// matvec por vector, el mismo kernel que `logits_gpu`, para que el token elegido no dependa de si se
    /// especuló.
    fn argmax_posiciones(
        &mut self,
        n: usize,
        source: &mut impl TensorSource,
        parallel: Option<&dyn RowParallel>,
        gpu: &mut Option<&mut dyn crate::gpu::GpuDispatch>,
    ) -> Result<Vec<u32>, ()> {
        let h = self.manifest.hidden_dim as usize;
        let vocab = self.manifest.vocab_size as usize;
        let name = if self.has_lm_head { "lm_head" } else { "embed" };
        let eps = self.manifest.rms_eps;
        let cap = self.manifest.ext.logit_softcap;
        note_infer_op(None, name);
        if self.has_output_norm {
            source.load_f32("output_norm", &mut self.scratch.norm_w)?;
        }
        let view = source.tensor_view(name)?;
        let mut salida = Vec::with_capacity(n);
        let usar_gpu = matches!(self.backend, Backend::Gpu | Backend::Auto) && gpu.is_some();
        if usar_gpu {
            let seq = Sequential;
            let par: &dyn RowParallel = parallel.unwrap_or(&seq);
            for j in 0..n {
                self.scratch.residual.copy_from_slice(&self.hs_bloque[j * h..(j + 1) * h]);
                if self.has_output_norm {
                    rmsnorm(&mut self.scratch.residual, &self.scratch.norm_w, eps);
                }
                let en_gpu = match gpu.as_deref_mut() {
                    Some(g) => crate::gpu::try_gpu_matvec(g, name, &view, vocab, h, &self.scratch.residual, &mut self.logits_buf)
                        .unwrap_or(false),
                    None => false,
                };
                if !en_gpu {
                    matvec_view_par(&view, vocab, h, &self.scratch.residual, &mut self.logits_buf, par)?;
                }
                self.softcap_logits();
                salida.push(Self::argmax(&self.logits_buf));
            }
            return Ok(salida);
        }
        let mut xs = self.hs_bloque[..n * h].to_vec();
        if self.has_output_norm {
            for j in 0..n {
                rmsnorm(&mut xs[j * h..(j + 1) * h], &self.scratch.norm_w, eps);
            }
        }
        self.logits_lote.resize(n * vocab, 0.0);
        let mut q8k: Vec<u8> = Vec::new();
        if view.dtype == sosomodel::layout::DTYPE_Q4_K && h % 256 == 0 {
            q8k.resize(crate::q8k::q8k_buffer_len(n, h), 0);
        }
        let seq = Sequential;
        let par: &dyn RowParallel = parallel.unwrap_or(&seq);
        crate::layer::matvec_view_batch_par(
            &view,
            vocab,
            h,
            &xs,
            n,
            &mut self.logits_lote,
            par,
            if q8k.is_empty() { None } else { Some(&mut q8k[..]) },
        )?;
        for j in 0..n {
            let fila = &mut self.logits_lote[j * vocab..(j + 1) * vocab];
            if cap > 0.0 {
                for v in fila.iter_mut() {
                    *v = cap * libm::tanhf(*v / cap);
                }
            }
            salida.push(Self::argmax(fila));
        }
        Ok(salida)
    }

    /// Deja el estado como si sólo hubieran pasado `1 + aceptados` tokens del último bloque de verificación:
    /// recorta el K/V rechazado, fija `pos` y `hidden` y avisa al planificador. Devuelve `true` si el
    /// planificador pide replanificar.
    fn cerrar_bloque_especulativo(
        &mut self,
        pos0: usize,
        aceptados: usize,
        cluster: &mut Option<&mut crate::remoto::ClusterGen<'_>>,
    ) -> Result<bool, ()> {
        let h = self.manifest.hidden_dim as usize;
        let p = pos0 + 1 + aceptados;
        for l in 0..self.kv.len() {
            let d = self.kv_dim_recortable(l).unwrap_or(0);
            self.kv[l].truncate_tokens(p, d);
        }
        if let Some(crate::remoto::ClusterGen {
            remoto,
            journal,
            ..
        }) = cluster.as_deref_mut()
        {
            remoto.truncar(p)?;
            if let Some(j) = journal {
                j.truncar(p as u32);
            }
        }
        self.pos = p;
        self.hidden.copy_from_slice(&self.hs_bloque[aceptados * h..(aceptados + 1) * h]);
        let mut replanificar = false;
        for _ in 0..1 + aceptados {
            if let Some(pl) = self.planner.as_mut() {
                replanificar |= pl.on_token_complete(&self.manifest, &self.index);
            }
        }
        self.slide_kv_if_needed();
        Ok(replanificar)
    }

    /// Verifica `drafts` (continuación propuesta tras `next`) con **un** forward de `[next, d1..dk]`.
    ///
    /// El estado tras `next` predice `d1`; el de `d1`, `d2`; etc. Se acepta el prefijo en el que el argmax
    /// coincide con el borrador. Los K/V de lo rechazado se recortan y el estado oculto queda en el último
    /// aceptado, así que el siguiente muestreo del bucle sale del mismo punto que con el camino token a token:
    /// con las mismas operaciones por token (bit a bit, T82) la secuencia generada es idéntica, con un
    /// forward por ronda en vez de uno por token aceptado.
    #[allow(clippy::too_many_arguments)]
    fn especular_en_bloque(
        &mut self,
        source: &mut impl TensorSource,
        next: u32,
        drafts: &[u32],
        remaining: &mut usize,
        tokens: &mut Vec<u32>,
        stops: &[u32],
        ledger: &mut GenerationLedger,
        observer: &mut dyn GenerationObserver,
        cancel_slot: &mut Option<&mut GenCancel>,
        parallel: Option<&dyn RowParallel>,
        gpu: &mut Option<&mut dyn crate::gpu::GpuDispatch>,
        restr: &mut Option<Box<dyn Restriccion>>,
        cluster: &mut Option<&mut crate::remoto::ClusterGen<'_>>,
    ) -> Result<Especulativo, ()> {
        let k = drafts.len();
        if !self.borrador_en_bloque_posible(k, gpu) {
            return Ok(Especulativo::NoAplica);
        }
        if cancel_slot.as_mut().is_some_and(|g| g.checkpoint(observer, ledger, GenCheckpoint::BeforeSample)) {
            return Ok(Especulativo::Parar);
        }
        let pos0 = self.pos;
        let mut bloque: Vec<u32> = Vec::with_capacity(k + 1);
        bloque.push(next);
        bloque.extend_from_slice(drafts);
        if !self.forward_bloque(
            source,
            &bloque,
            parallel,
            ledger,
            observer,
            cancel_slot,
            gpu,
            true,
            cluster,
            Some(tokens),
        )? {
            return Ok(Especulativo::Parar);
        }
        self.forwards_decode += 1;
        let previstos = self.argmax_posiciones(k, source, parallel, gpu)?;
        let mut aceptados = previstos.iter().zip(drafts).take_while(|(p, d)| p == d).count();
        let mut parar = false;
        for j in 0..aceptados {
            if !ledger.accept_sample(drafts[j], stops, tokens, observer) {
                aceptados = j;
                parar = true;
                break;
            }
            avanzar_restriccion(restr, drafts[j]);
            *remaining -= 1;
        }
        self.borrador_ofrecidos += k as u64;
        self.borrador_aceptados += aceptados as u64;
        let replanificado = self.cerrar_bloque_especulativo(pos0, aceptados, cluster)?;
        if parar {
            return Ok(Especulativo::Parar);
        }
        Ok(Especulativo::Hecho { ofrecidos: k, aceptados, replanificado })
    }

    /// Prefill del prompt: prefetch del siguiente embed mientras se calcula
    /// el forward del token actual.
    pub fn prefill_prompt(
        &mut self,
        source: &mut impl TensorSource,
        prompt: &[u32],
        parallel: Option<&dyn RowParallel>,
        gpu: &mut Option<&mut dyn crate::gpu::GpuDispatch>,
        clock_ms: Option<fn() -> u64>,
    ) -> Result<(), ()> {
        let mut noop = NoCancelObserver;
        let mut scratch_ledger = GenerationLedger::new(0);
        let mut cancel_slot = None;
        self.prefill_prompt_with(
            source,
            prompt,
            parallel,
            gpu,
            clock_ms,
            &mut scratch_ledger,
            &mut noop,
            &mut cancel_slot,
            &mut None,
            None,
        )
    }

    /// Como `prefill_prompt`, con checkpoints cancelables (T09).
    pub fn prefill_prompt_with(
        &mut self,
        source: &mut impl TensorSource,
        prompt: &[u32],
        parallel: Option<&dyn RowParallel>,
        gpu: &mut Option<&mut dyn crate::gpu::GpuDispatch>,
        clock_ms: Option<fn() -> u64>,
        ledger: &mut GenerationLedger,
        observer: &mut dyn GenerationObserver,
        cancel_slot: &mut Option<&mut GenCancel>,
        cluster: &mut Option<&mut crate::remoto::ClusterGen<'_>>,
        hash_context: Option<&[u32]>,
    ) -> Result<(), ()> {
        let total = prompt.len();
        if let Some(g) = cancel_slot.as_mut() {
            if g.checkpoint(observer, ledger, GenCheckpoint::BeforePrefill) {
                return Ok(());
            }
        } else {
            let _ = observer.cancel_at(GenCheckpoint::BeforePrefill);
        }
        let mut i = 0usize;
        while i < total {
            let bloque = self.bloque_prefill(gpu);
            // Prefill por bloques (T82): ~4× más rápido que token a token y con el
            // mismo resultado bit a bit. Cae al camino de siempre si no es seguro.
            let n = (total - i).min(bloque);
            if n >= 2 && self.bloque_posible(n, gpu) {
                // El observador ve cada `done` como en el camino token a token (hay
                // quien cancela en uno concreto); la cancelación surte efecto al
                // empezar el bloque.
                for d in 1..=n {
                    let at = GenCheckpoint::PrefillToken {
                        done: i + d,
                        total,
                    };
                    if let Some(g) = cancel_slot.as_mut() {
                        if g.checkpoint(observer, ledger, at) {
                            return Ok(());
                        }
                    } else {
                        let _ = observer.cancel_at(at);
                    }
                }
                let sigue = self.forward_bloque(
                    source,
                    &prompt[i..i + n],
                    parallel,
                    ledger,
                    observer,
                    cancel_slot,
                    gpu,
                    false,
                    cluster,
                    hash_context,
                )?;
                if !sigue {
                    return Ok(());
                }
                i += n;
                self.instantanea_intermedia_gdn(hash_context);
                continue;
            }
            let tok = prompt[i];
            let at = GenCheckpoint::PrefillToken {
                done: i + 1,
                total,
            };
            if let Some(g) = cancel_slot.as_mut() {
                if g.checkpoint(observer, ledger, at) {
                    return Ok(());
                }
            } else {
                let _ = observer.cancel_at(at);
            }
            if let Some(&next) = prompt.get(i + 1) {
                self.prefetch_embed(next, source);
            }
            self.embed_token(tok, source)?;
            if let Some(c) = clock_ms {
                let _ = self.forward_step_timed(
                    source,
                    parallel,
                    gpu,
                    c,
                    Some(observer),
                    cancel_slot,
                    Some(ledger),
                    cluster,
                )?;
            } else {
                self.forward_step_par(
                    source,
                    parallel,
                    gpu,
                    Some(observer),
                    cancel_slot,
                    Some(ledger),
                    cluster,
                )?;
            }
            if cancel_slot.as_ref().is_some_and(|g| g.stopped) {
                return Ok(());
            }
            i += 1;
            self.instantanea_intermedia_gdn(hash_context);
        }
        Ok(())
    }

    pub fn forward_step(&mut self, source: &mut impl TensorSource) -> Result<(), ()> {
        let mut cancel_slot = None;
        self.forward_step_par(source, None, &mut None, None, &mut cancel_slot, None, &mut None)
    }

    pub fn forward_step_par(
        &mut self,
        source: &mut impl TensorSource,
        parallel: Option<&dyn RowParallel>,
        gpu: &mut Option<&mut dyn crate::gpu::GpuDispatch>,
        mut observer: Option<&mut dyn GenerationObserver>,
        cancel_slot: &mut Option<&mut GenCancel>,
        ledger: Option<&mut GenerationLedger>,
        cluster: &mut Option<&mut crate::remoto::ClusterGen<'_>>,
    ) -> Result<(), ()> {
        let local_end = cluster
            .as_ref()
            .map(|c| c.local_end)
            .unwrap_or(self.manifest.num_layers);
        self.forward_layers_range_clock(
            0,
            local_end,
            source,
            parallel,
            gpu,
            None,
            &mut observer,
            cancel_slot,
            ledger,
        )?;
        if cancel_slot.as_ref().is_some_and(|g| g.stopped) {
            return Ok(());
        }
        if let Some(crate::remoto::ClusterGen {
            remoto,
            journal,
            prefix_hash,
            ..
        }) = cluster.as_deref_mut()
        {
            let h = self.manifest.hidden_dim as usize;
            let pos0 = self.pos;
            let mut hs = self.hidden.clone();
            if let Some(j) = journal {
                j.registrar(pos0 as u32, 1, &hs);
            }
            remoto.forward(pos0, &mut hs, 1, h, *prefix_hash)?;
            self.hidden.copy_from_slice(&hs);
        }
        self.advance_pos();
        self.slide_kv_if_needed();
        Ok(())
    }

    /// Como `forward_step_par` con reloj y replanificación al final del token.
    pub fn forward_step_timed(
        &mut self,
        source: &mut impl TensorSource,
        parallel: Option<&dyn RowParallel>,
        gpu: &mut Option<&mut dyn crate::gpu::GpuDispatch>,
        clock_ms: fn() -> u64,
        mut observer: Option<&mut dyn GenerationObserver>,
        cancel_slot: &mut Option<&mut GenCancel>,
        ledger: Option<&mut GenerationLedger>,
        cluster: &mut Option<&mut crate::remoto::ClusterGen<'_>>,
    ) -> Result<bool, ()> {
        let local_end = cluster
            .as_ref()
            .map(|c| c.local_end)
            .unwrap_or(self.manifest.num_layers);
        self.forward_layers_range_clock(
            0,
            local_end,
            source,
            parallel,
            gpu,
            Some(clock_ms),
            &mut observer,
            cancel_slot,
            ledger,
        )?;
        if let Some(crate::remoto::ClusterGen {
            remoto,
            journal,
            prefix_hash,
            ..
        }) = cluster.as_deref_mut()
        {
            let h = self.manifest.hidden_dim as usize;
            let pos0 = self.pos;
            let mut hs = self.hidden.clone();
            if let Some(j) = journal {
                j.registrar(pos0 as u32, 1, &hs);
            }
            remoto.forward(pos0, &mut hs, 1, h, *prefix_hash)?;
            self.hidden.copy_from_slice(&hs);
        }
        if cancel_slot.as_ref().is_some_and(|g| g.stopped) {
            return Ok(false);
        }
        self.advance_pos();
        self.slide_kv_if_needed();
        let replanned = self
            .planner
            .as_mut()
            .map(|p| p.on_token_complete(&self.manifest, &self.index))
            .unwrap_or(false);
        Ok(replanned)
    }

    /// Aplica `layer_start..layer_end` sobre `n` vectores contiguos en `hs` (posición actual).
    /// Si el camino por bloques no aplica, cae a un paso por token idéntico bit a bit.
    pub fn forward_bloque_rango(
        &mut self,
        layer_start: u32,
        layer_end: u32,
        source: &mut impl TensorSource,
        parallel: Option<&dyn RowParallel>,
        gpu: &mut Option<&mut dyn crate::gpu::GpuDispatch>,
        hs: &mut [f32],
        n: usize,
    ) -> Result<(), ()> {
        if layer_start > layer_end || layer_end > self.manifest.num_layers {
            return Err(());
        }
        let h = self.manifest.hidden_dim as usize;
        if hs.len() != n * h {
            return Err(());
        }
        if n == 0 {
            return Ok(());
        }
        if n == 1 {
            self.set_hidden(&hs[..h])?;
            self.forward_layers_range(layer_start, layer_end, source, parallel, gpu)?;
            hs[..h].copy_from_slice(self.hidden_slice());
            return Ok(());
        }
        if !self.bloque_posible_en_rango(layer_start, layer_end, n, gpu) {
            for t in 0..n {
                self.set_hidden(&hs[t * h..(t + 1) * h])?;
                self.forward_layers_range(layer_start, layer_end, source, parallel, gpu)?;
                hs[t * h..(t + 1) * h].copy_from_slice(self.hidden_slice());
                self.advance_pos();
            }
            return Ok(());
        }
        let use_gpu = matches!(self.backend, Backend::Gpu | Backend::Auto)
            && gpu.as_ref().is_some_and(|g| g.available());
        let mut bs = self
            .block
            .take()
            .unwrap_or_else(|| BlockScratch::new(&self.manifest, Self::bloque_flujo().max(Self::BLOQUE_PREFILL_STREAM)));
        let exec = LayerExecutor {
            manifest: &self.manifest,
            has_gate: self.has_gate,
            parallel,
        };
        let pos0 = self.pos;
        for layer in layer_start..layer_end {
            let r = exec.forward_layer_block(
                layer,
                pos0,
                n,
                hs,
                &mut bs,
                &mut self.scratch,
                &mut self.kv[layer as usize],
                source,
                self.planner.as_ref(),
                gpu,
                use_gpu,
            );
            if r.is_err() {
                self.block = Some(bs);
                if let Some(g) = gpu.as_deref_mut() {
                    let _ = g.res_volcar(hs);
                }
                return Err(());
            }
        }
        self.block = Some(bs);
        if let Some(g) = gpu.as_deref_mut() {
            g.res_volcar(hs)?;
        }
        Ok(())
    }

    fn bloque_posible_en_rango(
        &self,
        layer_start: u32,
        layer_end: u32,
        n: usize,
        gpu: &Option<&mut dyn crate::gpu::GpuDispatch>,
    ) -> bool {
        if n < 2 || !self.prefill_por_bloques {
            return false;
        }
        let base_gpu = matches!(self.backend, Backend::Gpu | Backend::Auto)
            && gpu.as_ref().is_some_and(|g| g.available());
        let exec0 = LayerExecutor { manifest: &self.manifest, has_gate: self.has_gate, parallel: None };
        // Las capas de Qwen3.5 van por bloques (en CPU) también con GPU disponible.
        let todas_q35 = (layer_start..layer_end).all(|l| crate::bloque35::soporta(&exec0, l, &self.kv[l as usize]));
        if base_gpu && !todas_q35 {
            return false;
        }
        if self.planner.is_some() {
            return false;
        }
        if self.pos + n > self.manifest.max_seq as usize {
            return false;
        }
        let exec = LayerExecutor {
            manifest: &self.manifest,
            has_gate: self.has_gate,
            parallel: None,
        };
        for l in layer_start..layer_end {
            let kv = &self.kv[l as usize];
            let kv_dim = self.manifest.effective_num_kv_heads(l) as usize
                * self.manifest.effective_head_dim(l) as usize;
            let q35 = crate::bloque35::soporta(&exec, l, kv);
            if !q35 && !exec.soporta_bloque(l, kv) {
                return false;
            }
            // Las capas GDN no guardan K/V: su posición no es la del bloque.
            if !q35 && kv.posiciones(kv_dim) != self.pos {
                return false;
            }
            if q35 && self.manifest.layer(l).is_some_and(|sp| sp.attn_kind == AttnKind::Gated) && kv.posiciones(kv_dim) != self.pos {
                return false;
            }
        }
        true
    }

    /// Ejecuta un rango de capas en la posición actual sin avanzar `pos`.
    pub fn forward_layers_range(
        &mut self,
        layer_start: u32,
        layer_end: u32,
        source: &mut impl TensorSource,
        parallel: Option<&dyn RowParallel>,
        gpu: &mut Option<&mut dyn crate::gpu::GpuDispatch>,
    ) -> Result<(), ()> {
        let mut cancel_slot = None;
        let mut observer = None;
        self.forward_layers_range_clock(
            layer_start,
            layer_end,
            source,
            parallel,
            gpu,
            None,
            &mut observer,
            &mut cancel_slot,
            None,
        )
    }

    /// Como `forward_layers_range` pero cronometra capas si se pasa `clock_ms`.
    pub fn forward_layers_range_clock(
        &mut self,
        layer_start: u32,
        layer_end: u32,
        source: &mut impl TensorSource,
        parallel: Option<&dyn RowParallel>,
        gpu: &mut Option<&mut dyn crate::gpu::GpuDispatch>,
        clock_ms: Option<fn() -> u64>,
        observer: &mut Option<&mut dyn GenerationObserver>,
        cancel_slot: &mut Option<&mut GenCancel>,
        mut ledger: Option<&mut GenerationLedger>,
    ) -> Result<(), ()> {
        // Con planner: la ventana KV deslizante permite superar max_seq.
        if self.planner.is_none() && self.pos >= self.manifest.max_seq as usize {
            return Err(());
        }
        if layer_start > layer_end || layer_end > self.manifest.num_layers {
            return Err(());
        }
        let base_gpu = matches!(self.backend, Backend::Gpu | Backend::Auto)
            && gpu.as_ref().is_some_and(|g| g.available());
        let exec = LayerExecutor {
            manifest: &self.manifest,
            has_gate: self.has_gate,
            parallel,
        };
        let keep_mapped = self
            .planner
            .as_ref()
            .is_some_and(|p| p.keep_weights_mapped());
        // Kick capa inicial (prefetch adelantado antes del bucle).
        if !keep_mapped {
            if let Some(pf) = self.manifest.prefetch.get(layer_start as usize) {
                source.kick_prefetch_shards(&pf.shards);
                if let Some(pl) = self.planner.as_mut() {
                    pl.note_prefetch();
                }
            }
        }
        for layer in layer_start..layer_end {
            // Esperar capa N (prefetchada mientras se calculó N-1).
            if !keep_mapped {
                let wait0 = clock_ms.map(|c| c());
                source.wait_prefetch();
                if let (Some(c), Some(t0)) = (clock_ms, wait0) {
                    let ms = c().saturating_sub(t0);
                    if let Some(pl) = self.planner.as_mut() {
                        pl.note_stage_wait_ms(ms);
                    }
                }
                // Kick capa N+1 antes del cómputo (solape I/O ∥ matvec/attn).
                if let Some(next) = self.manifest.prefetch.get((layer + 1) as usize) {
                    self.tiers.schedule_prefetch(&next.shards);
                    self.tiers.kick_pending(source);
                    if let Some(pl) = self.planner.as_mut() {
                        pl.note_prefetch();
                    }
                }
            }
            let use_gpu = base_gpu
                && self
                    .planner
                    .as_ref()
                    .map(|p| p.use_gpu_for_layer(layer))
                    .unwrap_or(true);
            let dest = if use_gpu {
                ExecDest::Gpu
            } else if self
                .planner
                .as_ref()
                .is_some_and(|p| p.layer_dest(layer) == ExecDest::Remote)
            {
                ExecDest::Remote
            } else {
                ExecDest::Cpu
            };
            if let Some(pl) = self.planner.as_mut() {
                pl.note_trunk_layer(layer, &self.index);
            }
            note_infer_op(Some(layer), "");
            if let Some(hook) = self.layer_enter_hook {
                hook(layer, layer_end);
            }
            if let (Some(g), Some(led)) = (cancel_slot.as_mut(), ledger.as_mut()) {
                if let Some(obs) = observer.as_deref_mut() {
                    if g.checkpoint(
                        obs,
                        led,
                        GenCheckpoint::Layer {
                            layer,
                            of: layer_end,
                        },
                    ) {
                        return Ok(());
                    }
                }
            }
            let t0 = clock_ms.map(|c| c());
            let timing = exec.forward_layer(
                layer,
                self.pos,
                &mut self.hidden,
                &mut self.scratch,
                &mut self.kv[layer as usize],
                source,
                gpu,
                use_gpu,
                self.planner.as_mut(),
                Some(&self.index),
                clock_ms,
            )?;
            if let (Some(c), Some(pl)) = (clock_ms, self.planner.as_mut()) {
                let ms = c().saturating_sub(t0.unwrap_or(0));
                pl.observe_layer(layer, dest, ms);
                pl.observe_hotpath(timing.matvec_ms, timing.attn_ms);
            }
            if let Some(hook) = self.layer_hook {
                hook(layer, layer_end);
            }
            // Liberar shards fuera del working set (streaming FlexGen).
            if !keep_mapped {
                let keep = self
                    .planner
                    .as_ref()
                    .map(|pl| pl.keep_all_shards_after(layer, layer_end, &self.manifest));
                if let Some(keep) = keep {
                    if !keep.is_empty() {
                        let tr0 = clock_ms.map(|c| c());
                        source.release_shards_except(&keep);
                        let ms = match (clock_ms, tr0) {
                            (Some(c), Some(t)) => c().saturating_sub(t),
                            _ => 0,
                        };
                        if let Some(pl) = self.planner.as_mut() {
                            pl.note_shard_release();
                            pl.note_release_ms(ms);
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// StreamingLLM / H2O: recorta KV a la ventana presupuestada.
    pub fn slide_kv_if_needed(&mut self) {
        let (keep, sink, use_h2o) = match self.planner.as_ref() {
            Some(pl) => (pl.kv_window_tokens(), pl.sink_tokens(), pl.use_h2o()),
            None => return,
        };
        let recent = keep.saturating_sub(sink) / 2;
        let mut slid = false;
        for (layer_idx, kv) in self.kv.iter_mut().enumerate() {
            let storage = kv_storage_dim(&self.manifest, layer_idx as u32);
            if kv.tokens(storage) > keep {
                kv.slide_window_h2o(keep, sink, recent, storage, use_h2o);
                slid = true;
            }
        }
        if slid {
            if let Some(pl) = self.planner.as_mut() {
                pl.note_kv_slide();
            }
        }
    }

    pub fn advance_pos(&mut self) {
        self.pos += 1;
    }

    pub fn set_hidden(&mut self, hidden: &[f32]) -> Result<(), ()> {
        if hidden.len() != self.hidden.len() {
            return Err(());
        }
        self.hidden.copy_from_slice(hidden);
        Ok(())
    }

    pub fn hidden_slice(&self) -> &[f32] {
        &self.hidden
    }

    /// Calcula los logits directamente sobre la vista zero-copy de lm_head
    /// (o embed con weight-tying): un solo matvec sin copiar la matriz.
    pub fn logits(&mut self, source: &mut impl TensorSource) -> Result<&[f32], ()> {
        let h = self.manifest.hidden_dim as usize;
        let vocab = self.manifest.vocab_size as usize;
        let name = if self.has_lm_head { "lm_head" } else { "embed" };
        note_infer_op(None, name);

        // `attn_out` puede ser q_dim > hidden (GQA Qwen3). `residual` sí es `h`.
        self.scratch.residual.copy_from_slice(&self.hidden);
        if self.has_output_norm {
            source.load_f32("output_norm", &mut self.scratch.norm_w)?;
            rmsnorm(&mut self.scratch.residual, &self.scratch.norm_w, self.manifest.rms_eps);
        }

        let view = source.tensor_view(name)?;
        matvec_view_par(
            &view,
            vocab,
            h,
            &self.scratch.residual,
            &mut self.logits_buf,
            &Sequential,
        )?;
        self.softcap_logits();
        Ok(&self.logits_buf)
    }

    pub fn logits_par(
        &mut self,
        source: &mut impl TensorSource,
        parallel: Option<&dyn RowParallel>,
    ) -> Result<&[f32], ()> {
        self.logits_gpu(source, parallel, &mut None)
    }

    /// Como [`Self::logits_par`], pero la cabeza de logits (la matriz más grande del modelo: `vocab × hidden`)
    /// puede ir a la GPU si el despacho la acepta; si no, CPU.
    pub fn logits_gpu(
        &mut self,
        source: &mut impl TensorSource,
        parallel: Option<&dyn RowParallel>,
        gpu: &mut Option<&mut dyn crate::gpu::GpuDispatch>,
    ) -> Result<&[f32], ()> {
        if self.decode_gpu.logits_listos {
            self.decode_gpu.logits_listos = false;
            return Ok(&self.logits_buf);
        }
        let h = self.manifest.hidden_dim as usize;
        let vocab = self.manifest.vocab_size as usize;
        let name = if self.has_lm_head { "lm_head" } else { "embed" };
        note_infer_op(None, name);
        self.scratch.residual.copy_from_slice(&self.hidden);
        if self.has_output_norm {
            source.load_f32("output_norm", &mut self.scratch.norm_w)?;
            rmsnorm(&mut self.scratch.residual, &self.scratch.norm_w, self.manifest.rms_eps);
        }
        let view = source.tensor_view(name)?;
        let usar_gpu = matches!(self.backend, Backend::Gpu | Backend::Auto);
        if usar_gpu {
            if let Some(g) = gpu.as_deref_mut() {
                if crate::gpu::try_gpu_matvec(g, name, &view, vocab, h, &self.scratch.residual, &mut self.logits_buf)
                    .unwrap_or(false)
                {
                    self.softcap_logits();
                    return Ok(&self.logits_buf);
                }
            }
        }
        let seq = Sequential;
        let par: &dyn RowParallel = parallel.unwrap_or(&seq);
        matvec_view_par(
            &view,
            vocab,
            h,
            &self.scratch.residual,
            &mut self.logits_buf,
            par,
        )?;
        self.softcap_logits();
        Ok(&self.logits_buf)
    }

    pub fn argmax(logits: &[f32]) -> u32 {
        logits
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(core::cmp::Ordering::Equal))
            .map(|(i, _)| i as u32)
            .unwrap_or(0)
    }

    /// Genera hasta `max_new` tokens muestreando con `sampler`, llamando a
    /// `on_token` según se emite cada uno (streaming). Devuelve la secuencia
    /// completa (prompt + generados).
    pub fn generate_stream(
        &mut self,
        source: &mut impl TensorSource,
        prompt: &[u32],
        max_new: usize,
        eos: Option<u32>,
        sampler: &mut crate::sample::Sampler,
        on_token: impl FnMut(u32),
    ) -> Result<Vec<u32>, ()> {
        self.generate_stream_par(
            source,
            prompt,
            max_new,
            eos,
            sampler,
            on_token,
            None,
            &mut None,
            &mut |_, _| {},
        )
    }

    /// Como `generate_stream` con matvec paralelo opcional.
    pub fn generate_stream_par(
        &mut self,
        source: &mut impl TensorSource,
        prompt: &[u32],
        max_new: usize,
        eos: Option<u32>,
        sampler: &mut crate::sample::Sampler,
        on_token: impl FnMut(u32),
        parallel: Option<&dyn RowParallel>,
        gpu: &mut Option<&mut dyn crate::gpu::GpuDispatch>,
        on_prefill: &mut dyn FnMut(usize, usize),
    ) -> Result<Vec<u32>, ()> {
        let stops = stops_from_eos(eos);
        self.generate_stream_par_with_report(
            source,
            prompt,
            GenerationOptions::new(max_new, stops),
            sampler,
            on_token,
            parallel,
            gpu,
            on_prefill,
        )
        .map(|(tokens, _)| tokens)
    }

    /// Como `generate_stream_par` con [`GenerationReport`](crate::generation::GenerationReport).
    pub fn generate_stream_par_with_report(
        &mut self,
        source: &mut impl TensorSource,
        prompt: &[u32],
        options: GenerationOptions,
        sampler: &mut crate::sample::Sampler,
        on_token: impl FnMut(u32),
        parallel: Option<&dyn RowParallel>,
        gpu: &mut Option<&mut dyn crate::gpu::GpuDispatch>,
        on_prefill: &mut dyn FnMut(usize, usize),
    ) -> Result<(Vec<u32>, GenerationReport), ()> {
        let mut legacy = LegacyStreamObserver {
            on_token,
            on_prefill: |done, total| on_prefill(done, total),
        };
        self.generate_stream_par_observed(
            source,
            prompt,
            options,
            sampler,
            &mut legacy,
            parallel,
            gpu,
            &mut None,
        )
    }

    /// Generación con observador explícito (cancelación cooperativa, T09).
    pub fn generate_stream_par_observed(
        &mut self,
        source: &mut impl TensorSource,
        prompt: &[u32],
        options: GenerationOptions,
        sampler: &mut crate::sample::Sampler,
        observer: &mut dyn GenerationObserver,
        parallel: Option<&dyn RowParallel>,
        gpu: &mut Option<&mut dyn crate::gpu::GpuDispatch>,
        cluster: &mut Option<&mut crate::remoto::ClusterGen<'_>>,
    ) -> Result<(Vec<u32>, GenerationReport), ()> {
        clear_infer_op();
        if prompt.is_empty() {
            note_infer_op(None, "prompt vacío");
            return Err(());
        }
        if cluster.is_none() && self.decode_gpu_posible(gpu) {
            self.decode_gpu_preparar(source, gpu);
        }
        let ya = self.begin_sequence_reusing(prompt);
        sync_cluster_prefijo(cluster, ya)?;
        let mut ledger = GenerationLedger::new(prompt.len());
        let mut cancel = GenCancel::default();
        let mut cancel_slot: Option<&mut GenCancel> = Some(&mut cancel);
        let stop_token_ids = options.stop_token_ids;
        let mut restr: Option<Box<dyn Restriccion>> = options.restriccion;
        let mut tokens: Vec<u32> = prompt.to_vec();
        self.prefill_prompt_with(
            source,
            &prompt[ya..],
            parallel,
            gpu,
            None,
            &mut ledger,
            observer,
            &mut cancel_slot,
            cluster,
            Some(&tokens),
        )?;
        if cancel_slot.as_ref().is_some_and(|g| g.stopped) {
            return Ok((tokens, ledger.into_report()));
        }
        self.guardar_instantanea_gdn(prompt);
        let greedy = sampler.temp <= 0.0;
        let mut remaining = options.max_new;
        if cancel_slot.as_mut().is_some_and(|g| {
            g.checkpoint(observer, &mut ledger, GenCheckpoint::BeforeDecode)
        }) {
            return Ok((tokens, ledger.into_report()));
        }
        let dec_gpu = cluster.is_none() && self.decode_gpu_posible(gpu);
        while remaining > 0 {
            if self.planner.is_none() && self.pos >= self.manifest.max_seq as usize {
                ledger.note_context_limit();
                break;
            }
            if cancel_slot.as_mut().is_some_and(|g| {
                g.checkpoint(observer, &mut ledger, GenCheckpoint::BeforeSample)
            }) {
                break;
            }
            let t0 = self.ahora_ms();
            let logits = self.logits_gpu(source, parallel, gpu)?;
            let next = match restr.as_deref_mut() {
                Some(r) => sampler.sample_restringido(logits, r),
                None => sampler.sample(logits),
            };
            if !ledger.accept_sample(next, &stop_token_ids, &mut tokens, observer) {
                break;
            }
            avanzar_restriccion(&mut restr, next);
            remaining -= 1;
            let mut drafts = if greedy && self.borrador_activo && !dec_gpu {
                crate::attn::prompt_lookup_draft(&tokens, remaining.min(8))
            } else {
                alloc::vec::Vec::new()
            };
            self.recortar_borrador(&mut drafts);
            filtrar_borrador(&restr, &mut drafts);
            if !drafts.is_empty() {
                let k = drafts.len().min(self.max_borrador_bloque(gpu));
                match self.especular_en_bloque(
                    source,
                    next,
                    &drafts[..k],
                    &mut remaining,
                    &mut tokens,
                    &stop_token_ids,
                    &mut ledger,
                    observer,
                    &mut cancel_slot,
                    parallel,
                    gpu,
                    &mut restr,
                    cluster,
                )? {
                    Especulativo::Hecho { ofrecidos, aceptados, .. } => {
                        self.anotar_ronda(t0, ofrecidos + 1, ofrecidos, aceptados);
                        continue;
                    }
                    Especulativo::Parar => break,
                    Especulativo::NoAplica => {}
                }
            }
            if let Some(&d0) = drafts.first() {
                self.prefetch_embed(d0, source);
            }
            self.embed_token(next, source)?;
            let en_gpu = dec_gpu && self.forward_token_gpu(source, parallel, gpu)?;
            if !en_gpu {
                self.forward_step_par(
                    source,
                    parallel,
                    gpu,
                    Some(observer),
                    &mut cancel_slot,
                    Some(&mut ledger),
                    cluster,
                )?;
            }
            self.forwards_decode += 1;
            if cancel_slot.as_ref().is_some_and(|g| g.stopped) {
                break;
            }
            if drafts.is_empty() {
                self.anotar_ronda(t0, 1, 0, 0);
                continue;
            }
            self.borrador_ofrecidos += drafts.len() as u64;
            let aceptados_antes = self.borrador_aceptados;
            for (di, &draft) in drafts.iter().enumerate() {
                if remaining == 0 {
                    break;
                }
                if cancel_slot.as_mut().is_some_and(|g| {
                    g.checkpoint(observer, &mut ledger, GenCheckpoint::BeforeSample)
                }) {
                    break;
                }
                let logits = self.logits_gpu(source, parallel, gpu)?;
                let next = Self::argmax(logits);
                if next != draft {
                    break;
                }
                if !ledger.accept_sample(next, &stop_token_ids, &mut tokens, observer) {
                    break;
                }
                remaining -= 1;
                avanzar_restriccion(&mut restr, next);
                self.borrador_aceptados += 1;
                if let Some(&nxt) = drafts.get(di + 1) {
                    self.prefetch_embed(nxt, source);
                }
                self.embed_token(next, source)?;
                self.forward_step_par(
                    source,
                    parallel,
                    gpu,
                    Some(observer),
                    &mut cancel_slot,
                    Some(&mut ledger),
                    cluster,
                )?;
                self.forwards_decode += 1;
                if cancel_slot.as_ref().is_some_and(|g| g.stopped) {
                    break;
                }
            }
            let aceptados = (self.borrador_aceptados - aceptados_antes) as usize;
            self.anotar_ronda(t0, 0, drafts.len(), aceptados);
        }
        self.decode_gpu_cerrar(gpu);
        if !cancel_slot.as_ref().is_some_and(|g| g.stopped) {
            self.recordar_cache(&tokens);
        }
        Ok((tokens, ledger.into_report()))
    }

    /// Como `generate_stream_par` con cronometraje de capas y replanificación.
    pub fn generate_stream_planned(
        &mut self,
        source: &mut impl TensorSource,
        prompt: &[u32],
        max_new: usize,
        eos: Option<u32>,
        sampler: &mut crate::sample::Sampler,
        on_token: impl FnMut(u32),
        parallel: Option<&dyn RowParallel>,
        gpu: &mut Option<&mut dyn crate::gpu::GpuDispatch>,
        clock_ms: fn() -> u64,
        refresh_mem: impl FnMut() -> crate::plan::MemSnapshot,
    ) -> Result<Vec<u32>, ()> {
        let stops = stops_from_eos(eos);
        self.generate_stream_planned_with_report(
            source,
            prompt,
            GenerationOptions::new(max_new, stops),
            sampler,
            on_token,
            parallel,
            gpu,
            clock_ms,
            refresh_mem,
        )
        .map(|(tokens, _)| tokens)
    }

    /// Como `generate_stream_planned` con informe de generación.
    pub fn generate_stream_planned_with_report(
        &mut self,
        source: &mut impl TensorSource,
        prompt: &[u32],
        options: GenerationOptions,
        sampler: &mut crate::sample::Sampler,
        on_token: impl FnMut(u32),
        parallel: Option<&dyn RowParallel>,
        gpu: &mut Option<&mut dyn crate::gpu::GpuDispatch>,
        clock_ms: fn() -> u64,
        mut refresh_mem: impl FnMut() -> crate::plan::MemSnapshot,
    ) -> Result<(Vec<u32>, GenerationReport), ()> {
        let mut legacy = LegacyStreamObserver {
            on_token,
            on_prefill: |_, _| {},
        };
        self.generate_stream_planned_observed(
            source,
            prompt,
            options,
            sampler,
            &mut legacy,
            parallel,
            gpu,
            clock_ms,
            &mut refresh_mem,
            &mut None,
        )
    }

    /// Como `generate_stream_planned_with_report` con observador explícito.
    pub fn generate_stream_planned_observed(
        &mut self,
        source: &mut impl TensorSource,
        prompt: &[u32],
        options: GenerationOptions,
        sampler: &mut crate::sample::Sampler,
        observer: &mut dyn GenerationObserver,
        parallel: Option<&dyn RowParallel>,
        gpu: &mut Option<&mut dyn crate::gpu::GpuDispatch>,
        clock_ms: fn() -> u64,
        refresh_mem: &mut dyn FnMut() -> crate::plan::MemSnapshot,
        cluster: &mut Option<&mut crate::remoto::ClusterGen<'_>>,
    ) -> Result<(Vec<u32>, GenerationReport), ()> {
        clear_infer_op();
        if prompt.is_empty() {
            note_infer_op(None, "prompt vacío");
            return Err(());
        }
        let ya = self.begin_sequence_reusing(prompt);
        sync_cluster_prefijo(cluster, ya)?;
        self.reloj = Some(clock_ms);
        if let Some(pl) = self.planner.as_mut() {
            pl.refresh_mem(refresh_mem());
        }
        let mut ledger = GenerationLedger::new(prompt.len());
        let mut cancel = GenCancel::default();
        let mut cancel_slot: Option<&mut GenCancel> = Some(&mut cancel);
        let stop_token_ids = options.stop_token_ids;
        let mut restr: Option<Box<dyn Restriccion>> = options.restriccion;
        let mut tokens: Vec<u32> = prompt.to_vec();
        self.prefill_prompt_with(
            source,
            &prompt[ya..],
            parallel,
            gpu,
            Some(clock_ms),
            &mut ledger,
            observer,
            &mut cancel_slot,
            cluster,
            Some(&tokens),
        )?;
        if cancel_slot.as_ref().is_some_and(|g| g.stopped) {
            return Ok((tokens, ledger.into_report()));
        }
        self.guardar_instantanea_gdn(prompt);
        let greedy = sampler.temp <= 0.0;
        let mut remaining = options.max_new;
        if cancel_slot.as_mut().is_some_and(|g| {
            g.checkpoint(observer, &mut ledger, GenCheckpoint::BeforeDecode)
        }) {
            return Ok((tokens, ledger.into_report()));
        }
        while remaining > 0 {
            if self.planner.is_none() && self.pos >= self.manifest.max_seq as usize {
                ledger.note_context_limit();
                break;
            }
            if cancel_slot.as_mut().is_some_and(|g| {
                g.checkpoint(observer, &mut ledger, GenCheckpoint::BeforeSample)
            }) {
                break;
            }
            if let Some(pl) = self.planner.as_mut() {
                pl.begin_token();
            }
            let t0 = self.ahora_ms();
            let logits = self.logits_gpu(source, parallel, gpu)?;
            let next = match restr.as_deref_mut() {
                Some(r) => sampler.sample_restringido(logits, r),
                None => sampler.sample(logits),
            };
            if !ledger.accept_sample(next, &stop_token_ids, &mut tokens, observer) {
                break;
            }
            avanzar_restriccion(&mut restr, next);
            remaining -= 1;
            let mut drafts = if greedy && self.borrador_activo {
                let (max_d, min_n, max_n, hint) = self
                    .planner
                    .as_ref()
                    .map(|p| p.pld_params())
                    .unwrap_or((8, 2, 7, 4));
                crate::attn::prompt_lookup_draft_hinted(
                    &tokens,
                    remaining.min(max_d),
                    min_n,
                    max_n,
                    hint,
                )
            } else {
                alloc::vec::Vec::new()
            };
            self.recortar_borrador(&mut drafts);
            filtrar_borrador(&restr, &mut drafts);
            if !drafts.is_empty() {
                let k = drafts.len().min(self.max_borrador_bloque(gpu));
                match self.especular_en_bloque(
                    source,
                    next,
                    &drafts[..k],
                    &mut remaining,
                    &mut tokens,
                    &stop_token_ids,
                    &mut ledger,
                    observer,
                    &mut cancel_slot,
                    parallel,
                    gpu,
                    &mut restr,
                    cluster,
                )? {
                    Especulativo::Hecho { ofrecidos, aceptados, replanificado } => {
                        self.anotar_ronda(t0, ofrecidos + 1, ofrecidos, aceptados);
                        if replanificado {
                            if let Some(pl) = self.planner.as_mut() {
                                pl.refresh_mem(refresh_mem());
                                pl.recompute_streaming_budgets(&self.manifest, &self.index);
                            }
                        }
                        if let Some(pl) = self.planner.as_mut() {
                            pl.note_pld_attempt();
                            if aceptados > 0 {
                                pl.note_pld_accepted(aceptados as u32);
                            }
                            pl.tune_pld(ofrecidos, aceptados as u32);
                        }
                        continue;
                    }
                    Especulativo::Parar => break,
                    Especulativo::NoAplica => {}
                }
            }
            if let Some(&d0) = drafts.first() {
                self.prefetch_embed(d0, source);
            }
            self.embed_token(next, source)?;
            if let Some(crate::remoto::ClusterGen { prefix_hash, .. }) = cluster.as_deref_mut() {
                let end = (self.pos + 1).min(tokens.len());
                *prefix_hash = crate::pipeline::prefix_hash_tokens(&tokens[..end]);
            }
            let replanificar = self.forward_step_timed(
                source,
                parallel,
                gpu,
                clock_ms,
                Some(observer),
                &mut cancel_slot,
                Some(&mut ledger),
                cluster,
            )?;
            self.forwards_decode += 1;
            if replanificar {
                if let Some(pl) = self.planner.as_mut() {
                    pl.refresh_mem(refresh_mem());
                    pl.recompute_streaming_budgets(&self.manifest, &self.index);
                }
            }
            if cancel_slot.as_ref().is_some_and(|g| g.stopped) {
                break;
            }
            if drafts.is_empty() {
                self.anotar_ronda(t0, 1, 0, 0);
                continue;
            }
            let offered = drafts.len();
            self.borrador_ofrecidos += offered as u64;
            if let Some(pl) = self.planner.as_mut() {
                pl.note_pld_attempt();
            }
            let mut accepted = 0u32;
            for (di, &draft) in drafts.iter().enumerate() {
                if remaining == 0 {
                    break;
                }
                if cancel_slot.as_mut().is_some_and(|g| {
                    g.checkpoint(observer, &mut ledger, GenCheckpoint::BeforeSample)
                }) {
                    break;
                }
                let logits = self.logits_gpu(source, parallel, gpu)?;
                let next = Self::argmax(logits);
                if next != draft {
                    break;
                }
                if !ledger.accept_sample(next, &stop_token_ids, &mut tokens, observer) {
                    break;
                }
                remaining -= 1;
                avanzar_restriccion(&mut restr, next);
                accepted += 1;
                self.borrador_aceptados += 1;
                if let Some(&nxt) = drafts.get(di + 1) {
                    self.prefetch_embed(nxt, source);
                }
                self.embed_token(next, source)?;
                let _ = self.forward_step_timed(
                    source,
                    parallel,
                    gpu,
                    clock_ms,
                    Some(observer),
                    &mut cancel_slot,
                    Some(&mut ledger),
                    cluster,
                )?;
                self.forwards_decode += 1;
                if cancel_slot.as_ref().is_some_and(|g| g.stopped) {
                    break;
                }
            }
            self.anotar_ronda(t0, 0, offered, accepted as usize);
            if let Some(pl) = self.planner.as_mut() {
                if accepted > 0 {
                    pl.note_pld_accepted(accepted);
                }
                pl.tune_pld(offered, accepted);
            }
        }
        if !cancel_slot.as_ref().is_some_and(|g| g.stopped) {
            self.recordar_cache(&tokens);
        }
        Ok((tokens, ledger.into_report()))
    }

    /// Decode greedy sin streaming (tests y usos simples).
    pub fn generate(
        &mut self,
        source: &mut impl TensorSource,
        prompt: &[u32],
        max_new: usize,
        eos: Option<u32>,
    ) -> Result<Vec<u32>, ()> {
        let mut sampler = crate::sample::Sampler::greedy();
        self.generate_stream(source, prompt, max_new, eos, &mut sampler, |_| {})
    }

    /// Como `generate` con informe de parada y contadores.
    pub fn generate_with_report(
        &mut self,
        source: &mut impl TensorSource,
        prompt: &[u32],
        options: GenerationOptions,
    ) -> Result<(Vec<u32>, GenerationReport), ()> {
        let mut sampler = crate::sample::Sampler::greedy();
        let mut noop = crate::generation::NoCancelObserver;
        self.generate_stream_par_observed(
            source,
            prompt,
            options,
            &mut sampler,
            &mut noop,
            None,
            &mut None,
            &mut None,
        )
    }

    /// Como `generate_with_report` con observador (cancelación).
    pub fn generate_with_report_observed(
        &mut self,
        source: &mut impl TensorSource,
        prompt: &[u32],
        options: GenerationOptions,
        observer: &mut dyn GenerationObserver,
    ) -> Result<(Vec<u32>, GenerationReport), ()> {
        let mut sampler = crate::sample::Sampler::greedy();
        self.generate_stream_par_observed(
            source,
            prompt,
            options,
            &mut sampler,
            observer,
            None,
            &mut None,
            &mut None,
        )
    }

    pub fn load_shard_to_ram(&mut self, name: &str, bytes: Vec<u8>) -> Result<(), ()> {
        use crate::tier::Tier;
        self.tiers.promote(name, bytes, Tier::Ram)
    }
}
