//! Decode de un token entero en la GPU (grafo residente) para Qwen3.5/3.6 (GDN + atención con puerta + MoE): construcción del plan a
//! partir del modelo y gestión de la sesión (estado GDN y KV en el dispositivo mientras dura la generación).

use super::*;
use alloc::format;
use crate::gpu::{CapaDec, CompartidoDec, DecodePlan, EstadoCapaDec, GatedDec, GdnDec, MatDec, MoeDec, VistaCruda};
use sosomodel::layout::DTYPE_F32;

/// Estado del decode en GPU dentro del `Runtime`.
#[derive(Default)]
pub struct DecGpu {
    /// `Some(true)` = preparado; `Some(false)` = rechazado (no se reintenta).
    pub preparado: Option<bool>,
    /// El estado GDN y el KV están en el dispositivo y el host está desfasado.
    pub sesion: bool,
    /// `logits_buf` ya tiene los logits del último token (lo consume `logits_gpu`).
    pub logits_listos: bool,
    pub ffn: Vec<usize>,
    pub kv_dims: Vec<usize>,
    pub kv_rows: Vec<u16>,
}

fn crudo(source: &mut impl TensorSource, name: &str) -> Result<VistaCruda, ()> {
    let v = source.tensor_view(name)?;
    Ok(VistaCruda::de(&v))
}

fn crudo_f32(source: &mut impl TensorSource, name: &str) -> Result<VistaCruda, ()> {
    let c = crudo(source, name)?;
    if c.dtype != DTYPE_F32 {
        return Err(());
    }
    Ok(c)
}

fn mat(source: &mut impl TensorSource, name: String, rows: usize, cols: usize) -> Result<MatDec, ()> {
    let v = crudo(source, &name)?;
    if v.elems != rows * cols {
        return Err(());
    }
    Ok(MatDec { clave: name, v, rows, cols })
}

fn norma(source: &mut impl TensorSource, name: &str, n: usize) -> Result<Vec<f32>, ()> {
    let mut v = vec![0.0f32; n];
    source.load_f32(name, &mut v)?;
    Ok(v)
}

impl Runtime {
    /// ¿Se puede (y se quiere) hacer el decode en la GPU? `SOSO_DECODE_GPU=1` lo activa.
    pub fn decode_gpu_posible(&self, gpu: &Option<&mut dyn crate::gpu::GpuDispatch>) -> bool {
        #[cfg(not(feature = "std"))]
        {
            let _ = gpu;
            return false;
        }
        #[cfg(feature = "std")]
        {
            let pedido = std::env::var("SOSO_DECODE_GPU").as_deref() == Ok("1");
            let backend = matches!(self.backend, Backend::Gpu | Backend::Auto);
            let disp = gpu.as_ref().is_some_and(|g| g.available());
            let kv_ok = self.kv.iter().all(|kv| kv.dtype == crate::kv::KvDtype::F16 && !kv.en_gpu);
            let ok = self.decode_gpu.preparado != Some(false) && pedido && backend && disp && self.hay_gdn() && kv_ok;
            if pedido && !ok {
                std::eprintln!("[decode GPU] no aplicable: preparado={:?} backend={backend} gpu={disp} gdn={} kv={kv_ok}", self.decode_gpu.preparado, self.hay_gdn());
            }
            ok
        }
    }

    fn construir_plan_decode(&self, source: &mut impl TensorSource) -> Result<DecodePlan, ()> {
        let m = &self.manifest;
        let h = m.hidden_dim as usize;
        let vocab = m.vocab_size as usize;
        if !self.has_output_norm {
            return Err(());
        }
        let mut capas = Vec::new();
        for layer in 0..m.num_layers {
            let prefix = format!("L{layer:02}");
            let spec = m.layer(layer).cloned().ok_or(())?;
            let n_exp = m.effective_num_experts(layer) as usize;
            let top_k = m.effective_num_experts_per_tok(layer) as usize;
            let ffn = m.effective_moe_ffn_dim(layer) as usize;
            if n_exp == 0 || top_k == 0 || n_exp > 1024 || top_k > 16 || ffn == 0 {
                return Err(());
            }
            let heads = m.effective_num_heads(layer) as usize;
            let kv_heads = m.effective_num_kv_heads(layer) as usize;
            let d = m.effective_head_dim(layer) as usize;
            let (mut gdn, mut gated, mut mats) = (None, None, Vec::new());
            match spec.attn_kind {
                AttnKind::Gdn => {
                    let (n_k, n_v) = (heads, kv_heads);
                    let kernel = spec.qk_rope_head_dim.max(1) as usize;
                    let qkv = 2 * n_k * d + n_v * d;
                    if d != 128 || n_v > 256 || kernel > 16 {
                        return Err(());
                    }
                    mats.push(mat(source, format!("{prefix}.attn_qkv"), qkv, h)?);
                    mats.push(mat(source, format!("{prefix}.attn_gate"), n_v * d, h)?);
                    mats.push(mat(source, format!("{prefix}.ssm_out"), h, n_v * d)?);
                    gdn = Some(GdnDec {
                        n_k,
                        n_v,
                        d,
                        kernel,
                        beta_w: crudo_f32(source, &format!("{prefix}.ssm_beta"))?,
                        alpha_w: crudo_f32(source, &format!("{prefix}.ssm_alpha"))?,
                        conv_w: crudo_f32(source, &format!("{prefix}.ssm_conv1d"))?,
                        dt: norma(source, &format!("{prefix}.ssm_dt"), n_v)?,
                        a: norma(source, &format!("{prefix}.ssm_a"), n_v)?,
                        norma: norma(source, &format!("{prefix}.ssm_norm"), d)?,
                    });
                }
                AttnKind::Gated => {
                    let rope_n = if spec.qk_rope_head_dim > 0 { spec.qk_rope_head_dim as usize } else { d };
                    if d != 256 || heads % kv_heads != 0 {
                        return Err(());
                    }
                    mats.push(mat(source, format!("{prefix}.attn_q"), 2 * heads * d, h)?);
                    mats.push(mat(source, format!("{prefix}.attn_k"), kv_heads * d, h)?);
                    mats.push(mat(source, format!("{prefix}.attn_v"), kv_heads * d, h)?);
                    mats.push(mat(source, format!("{prefix}.attn_output"), h, heads * d)?);
                    gated = Some(GatedDec {
                        heads,
                        kv_heads,
                        hd: d,
                        rope_n,
                        theta: m.rope_theta,
                        q_norm: norma(source, &format!("{prefix}.attn_q_norm"), d)?,
                        k_norm: norma(source, &format!("{prefix}.attn_k_norm"), d)?,
                    });
                }
                _ => return Err(()),
            }
            let mut expertos = Vec::with_capacity(n_exp);
            for e in 0..n_exp {
                let ep = format!("{prefix}.E{e:02}");
                expertos.push([
                    mat(source, format!("{ep}.ffn_gate"), ffn, h)?,
                    mat(source, format!("{ep}.ffn_up"), ffn, h)?,
                    mat(source, format!("{ep}.ffn_down"), h, ffn)?,
                ]);
            }
            let compartido = if spec.num_shared_experts > 0 {
                let ep = format!("{prefix}.S00");
                Some(CompartidoDec {
                    mats: [
                        mat(source, format!("{ep}.ffn_gate"), ffn, h)?,
                        mat(source, format!("{ep}.ffn_up"), ffn, h)?,
                        mat(source, format!("{ep}.ffn_down"), h, ffn)?,
                    ],
                    pg: crudo_f32(source, &format!("{prefix}.ffn_gate_inp_shexp"))?,
                })
            } else {
                None
            };
            capas.push(CapaDec {
                layer,
                gdn,
                gated,
                norma_attn: norma(source, &format!("{prefix}.attn_norm"), h)?,
                norma_ffn: norma(source, &format!("{prefix}.ffn_norm"), h)?,
                mats,
                moe: MoeDec { router: crudo_f32(source, &format!("{prefix}.ffn_gate_inp"))?, n_exp, top_k, ffn, expertos, compartido },
            });
        }
        let name = if self.has_lm_head { "lm_head" } else { "embed" };
        Ok(DecodePlan {
            h,
            eps: m.rms_eps,
            norma_final: norma(source, "output_norm", h)?,
            lm_head: mat(source, name.into(), vocab, h)?,
            vocab,
            capas,
        })
    }

    /// Termina la sesión de decode en GPU: baja el estado GDN al host (el KV ya se fue añadiendo al host token a token).
    pub fn decode_gpu_cerrar(&mut self, gpu: &mut Option<&mut dyn crate::gpu::GpuDispatch>) {
        if !self.decode_gpu.sesion {
            return;
        }
        self.decode_gpu.sesion = false;
        self.decode_gpu.logits_listos = false;
        if let Some(g) = gpu.as_deref_mut() {
            let mut estados = self.estados_dec(0);
            let _ = g.decode_bajar_estado(&mut estados);
        }
    }

    /// Descarta la sesión sin bajar nada (el host va a restaurar su propio estado).
    pub fn decode_gpu_descartar(&mut self) {
        self.decode_gpu.sesion = false;
        self.decode_gpu.logits_listos = false;
    }

    fn estados_dec(&mut self, _pos: usize) -> Vec<EstadoCapaDec<'_>> {
        self.kv
            .iter_mut()
            .map(|kv| {
                let LayerKv { gdn_s, gdn_conv, k_f16, v_f16, gpu_kv_filas, .. } = kv;
                EstadoCapaDec { gdn_s, gdn_conv, k: k_f16, v: v_f16, filas_validas: gpu_kv_filas }
            })
            .collect()
    }

    /// Prepara el decode en GPU (una vez): conviene hacerlo **antes** del primer prefill, que si no llena la VRAM de expertos sueltos.
    pub fn decode_gpu_preparar(&mut self, source: &mut impl TensorSource, gpu: &mut Option<&mut dyn crate::gpu::GpuDispatch>) {
        if self.decode_gpu.preparado.is_none() {
            let ok = match self.construir_plan_decode(source) {
                Ok(plan) => {
                    self.decode_gpu.ffn = plan.capas.iter().map(|c| c.moe.ffn).collect();
                    self.decode_gpu.kv_dims = plan.capas.iter().map(|c| c.gated.as_ref().map_or(0, |g| g.kv_heads * g.hd)).collect();
                    gpu.as_deref_mut().is_some_and(|g| g.decode_preparar(&plan).unwrap_or(false))
                }
                Err(()) => false,
            };
            self.decode_gpu.preparado = Some(ok);
            if !ok {
                #[cfg(feature = "std")]
                std::eprintln!("[decode GPU] no disponible: se sigue en CPU");
            }
        }
    }

    /// Un token de decode en la GPU: embedding en `self.hidden` → logits en `logits_buf`. `Ok(false)` = no se pudo (la CPU lo hace; la
    /// sesión queda cerrada).
    pub fn forward_token_gpu(
        &mut self,
        source: &mut impl TensorSource,
        parallel: Option<&dyn RowParallel>,
        gpu: &mut Option<&mut dyn crate::gpu::GpuDispatch>,
    ) -> Result<bool, ()> {
        self.decode_gpu_preparar(source, gpu);
        if self.decode_gpu.preparado != Some(true) {
            return Ok(false);
        }
        let h = self.manifest.hidden_dim as usize;
        let pos = self.pos;
        if self.planner.is_some() || pos + 1 >= self.manifest.max_seq as usize || self.hidden.len() != h {
            self.decode_gpu_cerrar(gpu);
            return Ok(false);
        }
        let g = match gpu.as_deref_mut() {
            Some(g) => g,
            None => return Ok(false),
        };
        if !self.decode_gpu.sesion {
            let mut estados = self.estados_dec(pos);
            if g.decode_subir_estado(&mut estados, pos).is_err() {
                return Ok(false);
            }
            self.decode_gpu.sesion = true;
        }
        let n_attn = self.decode_gpu.kv_dims.iter().filter(|&&d| d > 0).count();
        let kv_total: usize = self.decode_gpu.kv_dims.iter().sum();
        let mut kv_rows = core::mem::take(&mut self.decode_gpu.kv_rows);
        kv_rows.resize(2 * kv_total, 0);
        let _ = n_attn;
        let ffn = core::mem::take(&mut self.decode_gpu.ffn);
        let hidden = self.hidden.clone();
        let seq = Sequential;
        let par: &dyn RowParallel = parallel.unwrap_or(&seq);
        let mut cpu_moe = |capa: usize, xn: &[f32], idx: &[u32], wt: &[f32], out: &mut [f32]| -> Result<(), ()> {
            let ffn = ffn[capa];
            let prefix = format!("L{capa:02}");
            // Vistas en serie (la fuente no es compartible entre hilos) y luego un experto por hilo: sin barreras por matvec.
            let mut vistas: Vec<[(usize, usize, u8, usize); 3]> = Vec::with_capacity(idx.len());
            for &e in idx {
                let mut v = [(0usize, 0usize, 0u8, 0usize); 3];
                for (i, suf) in ["ffn_gate", "ffn_up", "ffn_down"].iter().enumerate() {
                    let vista = source.tensor_view(&format!("{prefix}.E{e:02}.{suf}"))?;
                    v[i] = (vista.bytes.as_ptr() as usize, vista.bytes.len(), vista.dtype, vista.elems);
                }
                vistas.push(v);
            }
            let mut ys = vec![0.0f32; idx.len() * h];
            let ys_p = ys.as_mut_ptr() as usize;
            let fallo = core::sync::atomic::AtomicBool::new(false);
            let seq = Sequential;
            par.for_rows(idx.len(), &|k0, k1| {
                let mut gate = vec![0.0f32; ffn];
                let mut up = vec![0.0f32; ffn];
                for k in k0..k1 {
                    let vista = |i: usize| TensorView {
                        bytes: unsafe { core::slice::from_raw_parts(vistas[k][i].0 as *const u8, vistas[k][i].1) },
                        dtype: vistas[k][i].2,
                        elems: vistas[k][i].3,
                    };
                    // SAFETY: cada hilo escribe sólo la fila `k` de `ys`.
                    let y = unsafe { core::slice::from_raw_parts_mut((ys_p as *mut f32).add(k * h), h) };
                    let ok = matvec_view_par(&vista(0), ffn, h, xn, &mut gate, &seq).is_ok()
                        && matvec_view_par(&vista(1), ffn, h, xn, &mut up, &seq).is_ok()
                        && {
                            crate::gemm::swiglu_inplace(&mut up, &gate);
                            matvec_view_par(&vista(2), h, ffn, &up, y, &seq).is_ok()
                        };
                    if !ok {
                        fallo.store(true, core::sync::atomic::Ordering::Relaxed);
                    }
                }
            });
            if fallo.load(core::sync::atomic::Ordering::Relaxed) {
                return Err(());
            }
            out.fill(0.0);
            for (k, &w) in wt.iter().enumerate() {
                for i in 0..h {
                    out[i] += ys[k * h + i] * w;
                }
            }
            Ok(())
        };
        let r = g.decode_token(&hidden, pos, &mut cpu_moe, &mut self.logits_buf, &mut kv_rows);
        self.decode_gpu.ffn = ffn;
        match r {
            Ok(true) => {}
            _ => {
                self.decode_gpu.kv_rows = kv_rows;
                self.decode_gpu_cerrar(gpu);
                return Ok(false);
            }
        }
        // KV nuevo al host (f16 tal cual salió del dispositivo) y el espejo del dispositivo ya lo tiene
        let mut off = 0usize;
        for (kv, &kd) in self.kv.iter_mut().zip(self.decode_gpu.kv_dims.iter()) {
            if kd == 0 {
                continue;
            }
            kv.k_f16.extend_from_slice(&kv_rows[off..off + kd]);
            kv.v_f16.extend_from_slice(&kv_rows[off + kd..off + 2 * kd]);
            kv.mass.push(0.0);
            kv.gpu_kv_filas = pos + 1;
            off += 2 * kd;
        }
        self.decode_gpu.kv_rows = kv_rows;
        self.decode_gpu.logits_listos = true;
        self.advance_pos();
        Ok(true)
    }
}
