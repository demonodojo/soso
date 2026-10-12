//! Capa de Gemma 4 (decode, un token).
//!
//! Lo que la distingue de la capa GQA clásica, fijado contra la implementación de referencia (llama.cpp,
//! `src/models/gemma4.cpp`) y no supuesto:
//!
//! ```text
//! x      = rmsnorm(h, attn_norm)
//! q      = rope(rmsnorm_cabeza(Wq·x, q_norm))
//! k      = rope(rmsnorm_cabeza(Wk·x, k_norm))        // capas globales: V = Wk·x (antes de k_norm)
//! v      = rmsnorm_sin_pesos_cabeza(Wv·x)            // o la proyección K, sin norma de K
//! a      = atención(q, k, v)  con escala 1.0 y, en capas locales, ventana deslizante
//! h      = h + rmsnorm(Wo·a, post_attn_norm)
//! y      = rmsnorm(h, ffn_norm)
//! h      = h + rmsnorm(Wdown·(gelu(Wgate·y) ⊙ Wup·y), post_ffn_norm)
//! h      = h · layer_scale
//! ```
//!
//! RoPE tipo NeoX (parejas `(i, i + d/2)`), con la base propia de cada capa y, en las globales, los
//! factores por frecuencia de `rope_freqs` (`theta_i / factor_i`).

use alloc::format;

use sosomodel::manifest::{
    FLAG_GEGLU, FLAG_LAYER_SCALE, FLAG_QK_NORM, FLAG_ROPE_FACTORS, FLAG_V_FROM_K, FLAG_V_NORM,
};

use crate::attn_gemma::attention_gqa_f16;
use crate::gemm::{gelu, rmsnorm};
use crate::kv::{KvDtype, LayerKv};
use crate::layer::{matvec_step, LayerExecutor, LayerScratch, LayerTiming, TensorSource};
use crate::parallel::{RowParallel, Sequential};

/// RMSNorm sin pesos (Gemma lo aplica a V).
pub fn rmsnorm_sin_pesos(x: &mut [f32], eps: f32) {
    let n = x.len() as f32;
    let var = x.iter().map(|v| v * v).sum::<f32>() / n;
    let scale = 1.0 / libm::sqrtf(var + eps);
    for v in x.iter_mut() {
        *v *= scale;
    }
}

/// RoPE NeoX sobre `heads` cabezas de `head_dim`, con base `theta` y factores opcionales por frecuencia
/// (`factores[i]` divide la frecuencia del par `i`). Todas las cabezas comparten cos/sin.
pub fn rope_neox_factores(x: &mut [f32], heads: usize, head_dim: usize, pos: usize, theta: f32, factores: Option<&[f32]>) {
    let mitad = head_dim / 2;
    let mut cos = alloc::vec![0.0f32; mitad];
    let mut sin = alloc::vec![0.0f32; mitad];
    let ln_theta = libm::logf(theta);
    for i in 0..mitad {
        // theta^(-2i/d) = exp(-ln(theta)·2i/d)
        let mut freq = libm::expf(-ln_theta * (2 * i) as f32 / head_dim as f32);
        if let Some(f) = factores {
            freq /= f[i];
        }
        let ang = pos as f32 * freq;
        cos[i] = libm::cosf(ang);
        sin[i] = libm::sinf(ang);
    }
    for h in 0..heads {
        let v = &mut x[h * head_dim..(h + 1) * head_dim];
        for i in 0..mitad {
            let (a, b) = (v[i], v[i + mitad]);
            v[i] = a * cos[i] - b * sin[i];
            v[i + mitad] = a * sin[i] + b * cos[i];
        }
    }
}

/// ¿Esta capa se ejecuta con el camino de Gemma?
pub fn es_capa_gemma(flags: u32) -> bool {
    flags & sosomodel::manifest::FLAG_SANDWICH_NORM != 0
}

#[allow(clippy::too_many_arguments)]
pub fn forward_layer_gemma<S: TensorSource>(
    exec: &LayerExecutor<'_>,
    layer: u32,
    pos: usize,
    hidden: &mut [f32],
    s: &mut LayerScratch,
    kv: &mut LayerKv,
    source: &mut S,
    gpu: &mut Option<&mut dyn crate::gpu::GpuDispatch>,
    use_gpu: bool,
    planner: Option<&crate::plan::ResourcePlanner>,
) -> Result<LayerTiming, ()> {
    if kv.dtype != KvDtype::F16 {
        return Err(()); // el camino de Gemma sólo lee un KV f16
    }
    let m = exec.manifest;
    let spec = m.layer(layer).cloned().unwrap_or_default();
    let h = m.hidden_dim as usize;
    let heads = m.effective_num_heads(layer) as usize;
    let kv_heads = m.effective_num_kv_heads(layer) as usize;
    let hd = m.effective_head_dim(layer) as usize;
    let q_dim = heads * hd;
    let kv_dim = kv_heads * hd;
    let g = heads / kv_heads;
    let ffn = m.effective_ffn_dim(layer) as usize;
    let eps = m.rms_eps;
    let theta = m.layer_rope_theta(layer);
    let ventana = m.layer_window(layer) as usize;
    let escala = if m.ext.attn_scale > 0.0 { m.ext.attn_scale } else { 1.0 / libm::sqrtf(hd as f32) };
    let p = format!("L{layer:02}");
    let seq = Sequential;
    let par: &dyn RowParallel = exec.parallel.unwrap_or(&seq);

    // ---------------------------------------------------------------- atención
    s.residual.copy_from_slice(hidden);
    source.load_f32(&format!("{p}.attn_norm"), &mut s.norm_w[..h])?;
    rmsnorm(hidden, &s.norm_w[..h], eps);

    let (nq, nk, nv, no) = (format!("{p}.attn_q"), format!("{p}.attn_k"), format!("{p}.attn_v"), format!("{p}.attn_output"));
    matvec_step(use_gpu, gpu, &nq, source.tensor_view(&nq)?, q_dim, h, hidden, &mut s.q[..q_dim], par, planner, layer)?;
    matvec_step(use_gpu, gpu, &nk, source.tensor_view(&nk)?, kv_dim, h, hidden, &mut s.k[..kv_dim], par, planner, layer)?;
    if spec.flags & FLAG_V_FROM_K != 0 {
        // El valor es la proyección K **antes** de su norma.
        let (k, v) = (&s.k[..kv_dim], &mut s.v[..kv_dim]);
        v.copy_from_slice(k);
    } else {
        matvec_step(use_gpu, gpu, &nv, source.tensor_view(&nv)?, kv_dim, h, hidden, &mut s.v[..kv_dim], par, planner, layer)?;
    }
    if spec.flags & FLAG_QK_NORM != 0 {
        source.load_f32(&format!("{p}.attn_q_norm"), &mut s.head_out[..hd])?;
        for c in 0..heads {
            rmsnorm(&mut s.q[c * hd..(c + 1) * hd], &s.head_out[..hd], eps);
        }
        source.load_f32(&format!("{p}.attn_k_norm"), &mut s.head_out[..hd])?;
        for c in 0..kv_heads {
            rmsnorm(&mut s.k[c * hd..(c + 1) * hd], &s.head_out[..hd], eps);
        }
    }
    if spec.flags & FLAG_V_NORM != 0 {
        for c in 0..kv_heads {
            rmsnorm_sin_pesos(&mut s.v[c * hd..(c + 1) * hd], eps);
        }
    }
    if spec.flags & FLAG_ROPE_FACTORS != 0 {
        source.load_f32("rope_freqs", &mut s.head_out[..hd / 2])?;
        let f = s.head_out[..hd / 2].to_vec();
        rope_neox_factores(&mut s.q[..q_dim], heads, hd, pos, theta, Some(&f));
        rope_neox_factores(&mut s.k[..kv_dim], kv_heads, hd, pos, theta, Some(&f));
    } else {
        rope_neox_factores(&mut s.q[..q_dim], heads, hd, pos, theta, None);
        rope_neox_factores(&mut s.k[..kv_dim], kv_heads, hd, pos, theta, None);
    }
    kv.append_f16(&s.k[..kv_dim], &s.v[..kv_dim]);
    let total = kv.tokens(kv_dim);
    let t_ini = if ventana > 0 { total.saturating_sub(ventana) } else { 0 };

    // Una tarea por (cabeza KV, trozo de cabezas Q): las globales tienen una sola cabeza KV y 16 de Q,
    // así que se reparten por trozos de Q (cada trozo reconvierte K/V; es el precio de usar los hilos).
    let trozos = if g >= 8 { 4 } else { 1 };
    let tareas = kv_heads * trozos;
    let (k_f16, v_f16) = (kv.k_f16_slice(), kv.v_f16_slice());
    let q_ptr = s.q.as_ptr() as usize;
    let o_ptr = s.attn_out.as_mut_ptr() as usize;
    let trabajo = move |t0: usize, t1: usize| {
        for t in t0..t1 {
            let (kvh, c) = (t / trozos, t % trozos);
            let (j0, j1) = (c * g / trozos, (c + 1) * g / trozos);
            let ng = j1 - j0;
            let base = (kvh * g + j0) * hd;
            let (q, o) = unsafe {
                (
                    core::slice::from_raw_parts((q_ptr as *const f32).add(base), ng * hd),
                    core::slice::from_raw_parts_mut((o_ptr as *mut f32).add(base), ng * hd),
                )
            };
            attention_gqa_f16(q, ng, k_f16, v_f16, hd, kv_dim, kvh, t_ini, total, escala, o);
        }
    };
    if tareas * (total - t_ini) < 4096 {
        trabajo(0, tareas);
    } else {
        par.for_rows(tareas, &trabajo);
    }

    matvec_step(use_gpu, gpu, &no, source.tensor_view(&no)?, h, q_dim, &s.attn_out[..q_dim], &mut s.q[..h], par, planner, layer)?;
    source.load_f32(&format!("{p}.post_attn_norm"), &mut s.norm_w[..h])?;
    rmsnorm(&mut s.q[..h], &s.norm_w[..h], eps);
    for i in 0..h {
        hidden[i] = s.residual[i] + s.q[i];
    }

    // ---------------------------------------------------------------- MLP (GeGLU)
    s.residual.copy_from_slice(hidden);
    source.load_f32(&format!("{p}.ffn_norm"), &mut s.norm_w[..h])?;
    rmsnorm(hidden, &s.norm_w[..h], eps);
    let (nu, ng_, nd) = (format!("{p}.ffn_up"), format!("{p}.ffn_gate"), format!("{p}.ffn_down"));
    matvec_step(use_gpu, gpu, &nu, source.tensor_view(&nu)?, ffn, h, hidden, &mut s.up[..ffn], par, planner, layer)?;
    matvec_step(use_gpu, gpu, &ng_, source.tensor_view(&ng_)?, ffn, h, hidden, &mut s.gate[..ffn], par, planner, layer)?;
    if spec.flags & FLAG_GEGLU != 0 {
        for i in 0..ffn {
            s.up[i] *= gelu(s.gate[i]);
        }
    } else {
        crate::gemm::swiglu_inplace(&mut s.up[..ffn], &s.gate[..ffn]);
    }
    matvec_step(use_gpu, gpu, &nd, source.tensor_view(&nd)?, h, ffn, &s.up[..ffn], hidden, par, planner, layer)?;
    source.load_f32(&format!("{p}.post_ffn_norm"), &mut s.norm_w[..h])?;
    rmsnorm(hidden, &s.norm_w[..h], eps);
    for i in 0..h {
        hidden[i] += s.residual[i];
    }
    if spec.flags & FLAG_LAYER_SCALE != 0 {
        let mut e = [0.0f32; 1];
        source.load_f32(&format!("{p}.layer_scale"), &mut e)?;
        for v in hidden.iter_mut() {
            *v *= e[0];
        }
    }
    Ok(LayerTiming::default())
}
