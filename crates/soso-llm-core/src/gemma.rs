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

/// Filas que una capa local guarda **de más** sobre su ventana antes de expulsar, y a cuántas se recorta.
/// La holgura es lo que permite reutilizar el prefijo de una petición anterior cuando diverge poco antes
/// del final (el patrón de un agente que reenvía su historial).
pub const EXPULSION_DISPARO: usize = 2048;
pub const EXPULSION_HOLGURA: usize = 1024;

/// Ventana deslizante real de una capa local: antes de añadir filas nuevas, tira las más antiguas si hay
/// demasiadas. Las capas globales (`ventana == 0`) conservan todo.
fn expulsar_si_hace_falta(kv: &mut LayerKv, ventana: usize, kv_dim: usize) {
    if ventana == 0 {
        return;
    }
    let guardadas = kv.tokens(kv_dim);
    if guardadas >= ventana + EXPULSION_DISPARO {
        kv.expulsar_delante(guardadas - (ventana + EXPULSION_HOLGURA), kv_dim);
    }
}

/// `up[i] *= gelu(gate[i])`, repartido entre hilos: la GELU (con `tanh`) en un solo hilo costaba ~12 ms por
/// token en el MLP de 15 360 columnas × 48 capas, casi lo que cuesta toda la multiplicación en una GPU.
pub fn geglu_par(par: &dyn RowParallel, up: &mut [f32], gate: &[f32]) {
    const TROZO: usize = 2048;
    let n = up.len().min(gate.len());
    if n <= TROZO {
        for i in 0..n {
            up[i] *= gelu(gate[i]);
        }
        return;
    }
    let up_ptr = up.as_mut_ptr() as usize;
    let gate_ptr = gate.as_ptr() as usize;
    let tareas = n.div_ceil(TROZO);
    par.for_rows(tareas, &move |t0: usize, t1: usize| {
        for t in t0..t1 {
            let (a, b) = (t * TROZO, ((t + 1) * TROZO).min(n));
            let (u, g) = unsafe {
                (
                    core::slice::from_raw_parts_mut((up_ptr as *mut f32).add(a), b - a),
                    core::slice::from_raw_parts((gate_ptr as *const f32).add(a), b - a),
                )
            };
            for i in 0..u.len() {
                u[i] *= gelu(g[i]);
            }
        }
    });
}

/// Intenta ejecutar la capa **entera** en la GPU. `Ok(true)` si el `hidden` ya es el de salida de la capa y el
/// KV de la capa está en el dispositivo; `Ok(false)` si hay que hacerla en CPU (sin tocar nada).
#[allow(clippy::too_many_arguments)]
fn capa_en_gpu<S: TensorSource>(
    exec: &LayerExecutor<'_>,
    layer: u32,
    pos0: usize,
    n: usize,
    hs: &mut [f32],
    kv: &mut LayerKv,
    source: &mut S,
    gpu: &mut Option<&mut dyn crate::gpu::GpuDispatch>,
    use_gpu: bool,
) -> Result<bool, ()> {
    if !use_gpu || kv.dtype != KvDtype::F16 {
        return Ok(false);
    }
    let Some(g) = gpu.as_deref_mut() else { return Ok(false) };
    if !g.available() || !g.gemma_puede(layer) {
        return Ok(false);
    }
    let m = exec.manifest;
    let spec = m.layer(layer).cloned().unwrap_or_default();
    let h = m.hidden_dim as usize;
    let heads = m.effective_num_heads(layer) as usize;
    let kv_heads = m.effective_num_kv_heads(layer) as usize;
    let hd = m.effective_head_dim(layer) as usize;
    let ffn = m.effective_ffn_dim(layer) as usize;
    let p = format!("L{layer:02}");
    let (nq, nk, nv, no) = (format!("{p}.attn_q"), format!("{p}.attn_k"), format!("{p}.attn_v"), format!("{p}.attn_output"));
    let (nu, ng, nd) = (format!("{p}.ffn_up"), format!("{p}.ffn_gate"), format!("{p}.ffn_down"));
    let v_de_k = spec.flags & FLAG_V_FROM_K != 0;
    let mut tensores: alloc::vec::Vec<(&str, usize, usize)> = alloc::vec![
        (nq.as_str(), heads * hd, h),
        (nk.as_str(), kv_heads * hd, h),
        (no.as_str(), h, heads * hd),
        (nu.as_str(), ffn, h),
        (ng.as_str(), ffn, h),
        (nd.as_str(), h, ffn),
    ];
    if !v_de_k {
        tensores.push((nv.as_str(), kv_heads * hd, h));
    }
    for (clave, rows, cols) in tensores {
        if !g.gemma_residente(clave) {
            let vista = source.tensor_view(clave)?;
            if !g.gemma_subir(clave, &vista, rows, cols).unwrap_or(false) {
                return Ok(false);
            }
        }
    }
    let cargar = |source: &mut S, nombre: String, n: usize| -> Result<alloc::vec::Vec<f32>, ()> {
        let mut v = alloc::vec![0.0f32; n];
        source.load_f32(&nombre, &mut v)?;
        Ok(v)
    };
    let attn_norm = cargar(source, format!("{p}.attn_norm"), h)?;
    let post_attn = cargar(source, format!("{p}.post_attn_norm"), h)?;
    let ffn_norm = cargar(source, format!("{p}.ffn_norm"), h)?;
    let post_ffn = cargar(source, format!("{p}.post_ffn_norm"), h)?;
    let qk = spec.flags & FLAG_QK_NORM != 0;
    let q_norm = if qk { cargar(source, format!("{p}.attn_q_norm"), hd)? } else { alloc::vec::Vec::new() };
    let k_norm = if qk { cargar(source, format!("{p}.attn_k_norm"), hd)? } else { alloc::vec::Vec::new() };
    let rope = if spec.flags & FLAG_ROPE_FACTORS != 0 { Some(cargar(source, "rope_freqs".into(), hd / 2)?) } else { None };
    let layer_scale = if spec.flags & FLAG_LAYER_SCALE != 0 {
        let mut e = [0.0f32; 1];
        source.load_f32(&format!("{p}.layer_scale"), &mut e)?;
        e[0]
    } else {
        1.0
    };
    let mut capa = crate::gpu::GemmaCapa {
        layer,
        pos0,
        n,
        hidden: hs,
        h,
        heads,
        kv_heads,
        head_dim: hd,
        ffn,
        ventana: m.layer_window(layer) as usize,
        ctx_max: (m.max_seq as usize).min(32768),
        eps: m.rms_eps,
        theta: m.layer_rope_theta(layer),
        escala: if m.ext.attn_scale > 0.0 { m.ext.attn_scale } else { 1.0 / libm::sqrtf(hd as f32) },
        v_de_k,
        v_norm: spec.flags & FLAG_V_NORM != 0,
        qk_norm: qk,
        geglu: spec.flags & FLAG_GEGLU != 0,
        layer_scale,
        clave_q: &nq,
        clave_k: &nk,
        clave_v: &nv,
        clave_o: &no,
        clave_up: &nu,
        clave_gate: &ng,
        clave_down: &nd,
        attn_norm: &attn_norm,
        q_norm: &q_norm,
        k_norm: &k_norm,
        post_attn_norm: &post_attn,
        ffn_norm: &ffn_norm,
        post_ffn_norm: &post_ffn,
        rope_freqs: rope.as_deref(),
        expulsadas: 0,
    };
    match g.gemma_capa(&mut capa) {
        Ok(true) => {}
        Ok(false) => return Ok(false),
        // Con el KV de esta capa ya en el dispositivo, seguir en CPU leería un KV vacío: se aborta.
        Err(()) if kv.en_gpu => return Err(()),
        Err(()) => return Ok(false),
    }
    kv.en_gpu = true;
    kv.gpu_pos = pos0 + n;
    kv.dropped = capa.expulsadas;
    Ok(true)
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

    if capa_en_gpu(exec, layer, pos, 1, hidden, kv, source, gpu, use_gpu)? {
        return Ok(LayerTiming::default());
    }
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
    expulsar_si_hace_falta(kv, ventana, kv_dim);
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
        geglu_par(par, &mut s.up[..ffn], &s.gate[..ffn]);
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

/// Una capa de Gemma para `n` tokens consecutivos (`pos0..pos0+n`) a la vez: el prefill por bloques.
///
/// Proyecciones y MLP —casi todo el coste— leen cada matriz una sola vez para los `n` vectores; la
/// atención sigue siendo por token (causal: el token `i` ve `seq_antes + i + 1` posiciones). El resultado
/// es el de `forward_layer_gemma` repetido `n` veces, salvo el orden de las sumas dentro de los núcleos
/// por lotes.
#[allow(clippy::too_many_arguments)]
pub fn forward_layer_gemma_block<S: TensorSource>(
    exec: &LayerExecutor<'_>,
    layer: u32,
    pos0: usize,
    n: usize,
    hs: &mut [f32],
    bs: &mut crate::layer::BlockScratch,
    s: &mut LayerScratch,
    kv: &mut LayerKv,
    source: &mut S,
    gpu: &mut Option<&mut dyn crate::gpu::GpuDispatch>,
    use_gpu: bool,
) -> Result<(), ()> {
    use crate::layer::matvec_view_batch_par;
    if capa_en_gpu(exec, layer, pos0, n, hs, kv, source, gpu, use_gpu)? {
        return Ok(());
    }
    // `matmul` de la GPU si el despacho lo acepta; si no, el núcleo por lotes de CPU.
    let mut mm = |source: &mut S,
                  bs_q8k: &mut alloc::vec::Vec<u8>,
                  name: &str,
                  rows: usize,
                  cols: usize,
                  xs: &[f32],
                  n: usize,
                  out: &mut [f32],
                  par: &dyn RowParallel|
     -> Result<(), ()> {
        let vista = source.tensor_view(name)?;
        if use_gpu {
            if let Some(g) = gpu.as_deref_mut() {
                if crate::gpu::try_gpu_matmul(g, name, &vista, rows, cols, n, xs, out).unwrap_or(false) {
                    return Ok(());
                }
            }
        }
        matvec_view_batch_par(&vista, rows, cols, xs, n, out, par, Some(bs_q8k))
    };
    if kv.dtype != KvDtype::F16 {
        return Err(());
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
    if hs.len() != n * h || n == 0 || n > bs.cap {
        return Err(());
    }
    bs.ensure_q8k(n, h.max(q_dim).max(ffn));

    let mut t_perfil = crate::layer::perfil_ahora();
    use crate::layer::{perfil_marca, FASE_ATENCION, FASE_FFN_DOWN, FASE_FFN_UP_GATE, FASE_O, FASE_OTROS, FASE_QKV, FASE_ROPE_KV, FASE_SWIGLU};
    // ---------------------------------------------------------------- atención
    bs.residual[..n * h].copy_from_slice(hs);
    source.load_f32(&format!("{p}.attn_norm"), &mut s.norm_w[..h])?;
    for i in 0..n {
        rmsnorm(&mut hs[i * h..(i + 1) * h], &s.norm_w[..h], eps);
    }
    let (nq, nk, nv, no) = (format!("{p}.attn_q"), format!("{p}.attn_k"), format!("{p}.attn_v"), format!("{p}.attn_output"));
    mm(source, &mut bs.q8k_x, &nq, q_dim, h, hs, n, &mut bs.q[..n * q_dim], par)?;
    mm(source, &mut bs.q8k_x, &nk, kv_dim, h, hs, n, &mut bs.k[..n * kv_dim], par)?;
    if spec.flags & FLAG_V_FROM_K != 0 {
        let (k, v) = (&bs.k[..n * kv_dim], &mut bs.v[..n * kv_dim]);
        v.copy_from_slice(k);
    } else {
        mm(source, &mut bs.q8k_x, &nv, kv_dim, h, hs, n, &mut bs.v[..n * kv_dim], par)?;
    }
    perfil_marca(&mut t_perfil, FASE_QKV);
    if spec.flags & FLAG_QK_NORM != 0 {
        source.load_f32(&format!("{p}.attn_q_norm"), &mut s.head_out[..hd])?;
        for i in 0..n {
            for c in 0..heads {
                let o = i * q_dim + c * hd;
                rmsnorm(&mut bs.q[o..o + hd], &s.head_out[..hd], eps);
            }
        }
        source.load_f32(&format!("{p}.attn_k_norm"), &mut s.head_out[..hd])?;
        for i in 0..n {
            for c in 0..kv_heads {
                let o = i * kv_dim + c * hd;
                rmsnorm(&mut bs.k[o..o + hd], &s.head_out[..hd], eps);
            }
        }
    }
    if spec.flags & FLAG_V_NORM != 0 {
        for i in 0..n {
            for c in 0..kv_heads {
                let o = i * kv_dim + c * hd;
                rmsnorm_sin_pesos(&mut bs.v[o..o + hd], eps);
            }
        }
    }
    let factores = if spec.flags & FLAG_ROPE_FACTORS != 0 {
        source.load_f32("rope_freqs", &mut s.head_out[..hd / 2])?;
        Some(s.head_out[..hd / 2].to_vec())
    } else {
        None
    };
    expulsar_si_hace_falta(kv, ventana, kv_dim);
    let seq_antes = kv.tokens(kv_dim);
    for i in 0..n {
        let pos = pos0 + i; // posición absoluta: RoPE no depende de las filas expulsadas
        rope_neox_factores(&mut bs.q[i * q_dim..(i + 1) * q_dim], heads, hd, pos, theta, factores.as_deref());
        rope_neox_factores(&mut bs.k[i * kv_dim..(i + 1) * kv_dim], kv_heads, hd, pos, theta, factores.as_deref());
        kv.append_f16(&bs.k[i * kv_dim..(i + 1) * kv_dim], &bs.v[i * kv_dim..(i + 1) * kv_dim]);
    }

    perfil_marca(&mut t_perfil, FASE_ROPE_KV);
    // Tareas: (token, cabeza KV, trozo de cabezas Q). Cada token ve su propio prefijo.
    let trozos = if g >= 8 { 4 } else { 1 };
    let por_token = kv_heads * trozos;
    let tareas = n * por_token;
    let (k_f16, v_f16) = (kv.k_f16_slice(), kv.v_f16_slice());
    let q_ptr = bs.q.as_ptr() as usize;
    let o_ptr = bs.attn.as_mut_ptr() as usize;
    let trabajo = move |t0: usize, t1: usize| {
        for t in t0..t1 {
            let (i, r) = (t / por_token, t % por_token);
            let (kvh, c) = (r / trozos, r % trozos);
            let (j0, j1) = (c * g / trozos, (c + 1) * g / trozos);
            let ng = j1 - j0;
            let base = i * q_dim + (kvh * g + j0) * hd;
            let total = seq_antes + i + 1;
            let t_ini = if ventana > 0 { total.saturating_sub(ventana) } else { 0 };
            let (q, o) = unsafe {
                (
                    core::slice::from_raw_parts((q_ptr as *const f32).add(base), ng * hd),
                    core::slice::from_raw_parts_mut((o_ptr as *mut f32).add(base), ng * hd),
                )
            };
            attention_gqa_f16(q, ng, k_f16, v_f16, hd, kv_dim, kvh, t_ini, total, escala, o);
        }
    };
    par.for_rows(tareas, &trabajo);
    perfil_marca(&mut t_perfil, FASE_ATENCION);

    mm(source, &mut bs.q8k_x, &no, h, q_dim, &bs.attn[..n * q_dim], n, &mut bs.o[..n * h], par)?;
    perfil_marca(&mut t_perfil, FASE_O);
    source.load_f32(&format!("{p}.post_attn_norm"), &mut s.norm_w[..h])?;
    for i in 0..n {
        rmsnorm(&mut bs.o[i * h..(i + 1) * h], &s.norm_w[..h], eps);
        for j in 0..h {
            hs[i * h + j] = bs.residual[i * h + j] + bs.o[i * h + j];
        }
    }

    // ---------------------------------------------------------------- MLP (GeGLU)
    bs.residual[..n * h].copy_from_slice(hs);
    source.load_f32(&format!("{p}.ffn_norm"), &mut s.norm_w[..h])?;
    for i in 0..n {
        rmsnorm(&mut hs[i * h..(i + 1) * h], &s.norm_w[..h], eps);
    }
    let (nu, ng_, nd) = (format!("{p}.ffn_up"), format!("{p}.ffn_gate"), format!("{p}.ffn_down"));
    perfil_marca(&mut t_perfil, FASE_OTROS);
    mm(source, &mut bs.q8k_x, &nu, ffn, h, hs, n, &mut bs.up[..n * ffn], par)?;
    mm(source, &mut bs.q8k_x, &ng_, ffn, h, hs, n, &mut bs.gate[..n * ffn], par)?;
    perfil_marca(&mut t_perfil, FASE_FFN_UP_GATE);
    if spec.flags & FLAG_GEGLU != 0 {
        geglu_par(par, &mut bs.up[..n * ffn], &bs.gate[..n * ffn]);
    } else {
        crate::gemm::swiglu_inplace(&mut bs.up[..n * ffn], &bs.gate[..n * ffn]);
    }
    perfil_marca(&mut t_perfil, FASE_SWIGLU);
    mm(source, &mut bs.q8k_x, &nd, h, ffn, &bs.up[..n * ffn], n, &mut bs.o[..n * h], par)?;
    perfil_marca(&mut t_perfil, FASE_FFN_DOWN);
    source.load_f32(&format!("{p}.post_ffn_norm"), &mut s.norm_w[..h])?;
    let escala_capa = if spec.flags & FLAG_LAYER_SCALE != 0 {
        let mut e = [0.0f32; 1];
        source.load_f32(&format!("{p}.layer_scale"), &mut e)?;
        e[0]
    } else {
        1.0
    };
    for i in 0..n {
        rmsnorm(&mut bs.o[i * h..(i + 1) * h], &s.norm_w[..h], eps);
        for j in 0..h {
            hs[i * h + j] = (bs.o[i * h + j] + bs.residual[i * h + j]) * escala_capa;
        }
    }
    Ok(())
}
