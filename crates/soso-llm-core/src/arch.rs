//! Ramas de arquitectura extendidas (MLA, KDA, Gated/GDN, LatentMoE).

use crate::attn;
use crate::gemm::{add_f32, rmsnorm, rope_inplace, rope_inplace_n, silu, swiglu_inplace, topk_softmax};
use crate::gpu::GpuDispatch;
use crate::kv::LayerKv;
use crate::layer::{matvec_step, LayerScratch, TensorSource};
use crate::plan::ResourcePlanner;
use crate::parallel::RowParallel;
use alloc::format;
use alloc::string::String;
use sosomodel::index::TensorIndex;
use sosomodel::manifest::{LayerSpec, Manifest};

/// Dimensiones efectivas MLA por cabeza (RoPE solo en el slice `rope`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MlaHeadDims {
    pub qk_nope: usize,
    pub qk_rope: usize,
    pub qk_per_head: usize,
    pub v_dim: usize,
    pub kv_qk_dim: usize,
    pub kv_v_dim: usize,
}

pub fn mla_head_dims(spec: &LayerSpec, head_dim: usize, kv_heads: usize) -> MlaHeadDims {
    let (qk_nope, qk_rope) = if spec.qk_rope_head_dim == 0 && spec.qk_nope_head_dim == 0 {
        (0, head_dim)
    } else {
        (
            spec.qk_nope_head_dim as usize,
            spec.qk_rope_head_dim as usize,
        )
    };
    let qk_per_head = qk_nope + qk_rope;
    let v_dim = if spec.v_head_dim > 0 {
        spec.v_head_dim as usize
    } else {
        qk_per_head
    };
    MlaHeadDims {
        qk_nope,
        qk_rope,
        qk_per_head,
        v_dim,
        kv_qk_dim: kv_heads * qk_per_head,
        kv_v_dim: kv_heads * v_dim,
    }
}

pub fn forward_mla_attn<S: TensorSource>(
    spec: &LayerSpec,
    manifest: &Manifest,
    layer: u32,
    pos: usize,
    prefix: &str,
    hidden: &mut [f32],
    s: &mut LayerScratch,
    kv: &mut LayerKv,
    source: &mut S,
    gpu: &mut Option<&mut dyn GpuDispatch>,
    use_gpu: bool,
    par: &dyn RowParallel,
    planner: Option<&ResourcePlanner>,
    clock_ms: Option<fn() -> u64>,
) -> Result<(u64, u64), ()> {
    let tick = |c: Option<fn() -> u64>| c.map(|f| f()).unwrap_or(0);
    let h = manifest.hidden_dim as usize;
    let heads = manifest.effective_num_heads(layer) as usize;
    let head_dim = h / heads;
    let kv_heads = manifest.effective_num_kv_heads(layer) as usize;
    let dims = mla_head_dims(spec, head_dim, kv_heads);
    let group = heads / kv_heads;
    let q_rank = spec.q_lora_rank as usize;
    let kv_rank = spec.kv_lora_rank as usize;
    let eps = manifest.rms_eps;
    let theta = manifest.rope_theta;
    let kv_buf = dims.kv_qk_dim.max(dims.kv_v_dim);

    s.residual.copy_from_slice(hidden);
    source.load_f32(&format!("{prefix}.attn_norm"), &mut s.norm_w)?;
    rmsnorm(hidden, &s.norm_w, eps);

    let t_mv0 = tick(clock_ms);
    let name_q_down = format!("{prefix}.attn_q_down");
    let name_q_up = format!("{prefix}.attn_q_up");
    let name_kv_down = format!("{prefix}.attn_kv_down");
    let name_k_up = format!("{prefix}.attn_k_up");
    let name_v_up = format!("{prefix}.attn_v_up");
    let name_out = format!("{prefix}.attn_output");

    if s.q.len() < h.max(q_rank).max(kv_rank) {
        return Err(());
    }

    matvec_step(
        use_gpu,
        gpu,
        &name_q_down,
        source.tensor_view(&name_q_down)?,
        q_rank,
        h,
        hidden,
        &mut s.gate[..q_rank],
        par,
        planner,
        layer,
    )?;
    matvec_step(
        use_gpu,
        gpu,
        &name_q_up,
        source.tensor_view(&name_q_up)?,
        h,
        q_rank,
        &s.gate[..q_rank],
        &mut s.q[..h],
        par,
        planner,
        layer,
    )?;
    matvec_step(
        use_gpu,
        gpu,
        &name_kv_down,
        source.tensor_view(&name_kv_down)?,
        kv_rank,
        h,
        hidden,
        &mut s.up[..kv_rank],
        par,
        planner,
        layer,
    )?;
    let t_attn0 = tick(clock_ms);

    for head in 0..heads {
        let base = head * head_dim;
        if dims.qk_rope > 0 {
            rope_inplace(
                &mut s.q[base + dims.qk_nope..base + dims.qk_nope + dims.qk_rope],
                pos,
                theta,
            );
        }
    }

    kv.append_mla_latent(&s.up[..kv_rank]);
    let seq = kv.tokens(kv_rank);
    s.attn_out.fill(0.0);
    for head in 0..heads {
        let q_h = &s.q[head * head_dim..(head + 1) * head_dim];
        let kv_head = head / group;
        if s.k.len() < kv_buf || s.v.len() < kv_buf {
            return Err(());
        }
        attn::attention_decode_mla_latent(
            q_h,
            kv,
            kv_rank,
            dims.qk_nope,
            dims.qk_rope,
            dims.v_dim,
            head_dim,
            kv_head,
            seq,
            theta,
            source,
            &name_k_up,
            &name_v_up,
            dims.kv_qk_dim,
            dims.kv_v_dim,
            &mut s.k[..kv_buf],
            &mut s.v[..kv_buf],
            &mut s.gate[..kv_rank],
            &mut s.head_out,
        )?;
        s.attn_out[head * head_dim..(head + 1) * head_dim].copy_from_slice(&s.head_out);
    }
    let t_attn1 = tick(clock_ms);

    matvec_step(
        use_gpu,
        gpu,
        &name_out,
        source.tensor_view(&name_out)?,
        h,
        h,
        &s.attn_out,
        &mut s.q[..h],
        par,
        planner,
        layer,
    )?;
    add_f32(&s.residual, &s.q[..h], hidden);
    let t_mv1 = tick(clock_ms);
    Ok((
        t_attn0.saturating_sub(t_mv0).saturating_add(t_mv1.saturating_sub(t_attn1)),
        t_attn1.saturating_sub(t_attn0),
    ))
}

/// KDA sintético: atención GQA con decaimiento exponencial por distancia temporal.
pub fn forward_kda_attn<S: TensorSource>(
    manifest: &Manifest,
    layer: u32,
    pos: usize,
    prefix: &str,
    hidden: &mut [f32],
    s: &mut LayerScratch,
    kv: &mut LayerKv,
    source: &mut S,
    gpu: &mut Option<&mut dyn GpuDispatch>,
    use_gpu: bool,
    par: &dyn RowParallel,
    planner: Option<&ResourcePlanner>,
    clock_ms: Option<fn() -> u64>,
) -> Result<(u64, u64), ()> {
    let tick = |c: Option<fn() -> u64>| c.map(|f| f()).unwrap_or(0);
    let h = manifest.hidden_dim as usize;
    let heads = manifest.effective_num_heads(layer) as usize;
    let head_dim = h / heads;
    let kv_heads = manifest.effective_num_kv_heads(layer) as usize;
    let kv_dim = kv_heads * head_dim;
    let group = heads / kv_heads;
    let eps = manifest.rms_eps;
    let theta = manifest.rope_theta;
    let decay = 0.95f32;

    s.residual.copy_from_slice(hidden);
    source.load_f32(&format!("{prefix}.attn_norm"), &mut s.norm_w)?;
    rmsnorm(hidden, &s.norm_w, eps);

    let t_mv0 = tick(clock_ms);
    let name_q = format!("{prefix}.attn_q");
    let name_k = format!("{prefix}.attn_k");
    let name_v = format!("{prefix}.attn_v");
    let name_out = format!("{prefix}.attn_output");

    matvec_step(
        use_gpu,
        gpu,
        &name_q,
        source.tensor_view(&name_q)?,
        h,
        h,
        hidden,
        &mut s.q[..h],
        par,
        planner,
        layer,
    )?;
    matvec_step(
        use_gpu,
        gpu,
        &name_k,
        source.tensor_view(&name_k)?,
        kv_dim,
        h,
        hidden,
        &mut s.k[..kv_dim],
        par,
        planner,
        layer,
    )?;
    matvec_step(
        use_gpu,
        gpu,
        &name_v,
        source.tensor_view(&name_v)?,
        kv_dim,
        h,
        hidden,
        &mut s.v[..kv_dim],
        par,
        planner,
        layer,
    )?;
    let t_attn0 = tick(clock_ms);

    for head in 0..heads {
        rope_inplace(&mut s.q[head * head_dim..(head + 1) * head_dim], pos, theta);
    }
    for head in 0..kv_heads {
        rope_inplace(&mut s.k[head * head_dim..(head + 1) * head_dim], pos, theta);
    }
    kv.append_f16(&s.k[..kv_dim], &s.v[..kv_dim]);
    let seq = kv.tokens(kv_dim);
    s.attn_out.fill(0.0);
    for head in 0..heads {
        let q_h = &s.q[head * head_dim..(head + 1) * head_dim];
        let kv_head = head / group;
        let mut logits = [0.0f32; 512];
        let cap = logits.len().min(seq);
        for t in 0..cap {
            let mut k_h = [0.0f32; 128];
            let mut v_h = [0.0f32; 128];
            kv.load_k_head(t, kv_head, head_dim, kv_dim, &mut k_h[..head_dim]);
            kv.load_v_head(t, kv_head, head_dim, kv_dim, &mut v_h[..head_dim]);
            let mut dot = 0.0f32;
            for d in 0..head_dim {
                dot += q_h[d] * k_h[d];
            }
            let age = pos.saturating_sub(t) as f32;
            logits[t] = dot * libm::powf(decay, age);
        }
        let mut max_l = logits[0];
        for t in 1..cap {
            if logits[t] > max_l {
                max_l = logits[t];
            }
        }
        let mut sum = 0.0f32;
        for t in 0..cap {
            logits[t] = libm::expf(logits[t] - max_l);
            sum += logits[t];
        }
        if sum > 0.0 {
            for t in 0..cap {
                logits[t] /= sum;
            }
        }
        s.head_out.fill(0.0);
        for t in 0..cap {
            let mut v_h = [0.0f32; 128];
            kv.load_v_head(t, kv_head, head_dim, kv_dim, &mut v_h[..head_dim]);
            for d in 0..head_dim {
                s.head_out[d] += logits[t] * v_h[d];
            }
        }
        s.attn_out[head * head_dim..(head + 1) * head_dim].copy_from_slice(&s.head_out);
    }
    let t_attn1 = tick(clock_ms);

    matvec_step(
        use_gpu,
        gpu,
        &name_out,
        source.tensor_view(&name_out)?,
        h,
        h,
        &s.attn_out,
        &mut s.q[..h],
        par,
        planner,
        layer,
    )?;
    add_f32(&s.residual, &s.q[..h], hidden);
    let t_mv1 = tick(clock_ms);
    Ok((
        t_attn0.saturating_sub(t_mv0).saturating_add(t_mv1.saturating_sub(t_attn1)),
        t_attn1.saturating_sub(t_attn0),
    ))
}

pub(crate) fn sigmoid(x: f32) -> f32 {
    1.0 / (1.0 + libm::expf(-x))
}

pub(crate) fn softplus(x: f32) -> f32 {
    if x > 20.0 {
        x
    } else {
        libm::log1pf(libm::expf(x))
    }
}

pub(crate) fn l2_normalize(x: &mut [f32], eps: f32) {
    let mut ss = 0.0f32;
    for &v in x.iter() {
        ss += v * v;
    }
    let inv = 1.0 / libm::sqrtf(ss + eps);
    for v in x.iter_mut() {
        *v *= inv;
    }
}

/// Un paso Gated DeltaNet (llama.cpp / GDA): `S ← gS + k Δᵀ`, `o = Sᵀ q`.
pub(crate) fn gdn_step(state: &mut [f32], q: &[f32], k: &[f32], v: &[f32], g: f32, beta: f32, out: &mut [f32]) {
    let d = q.len();
    if state.len() < d * d || k.len() < d || v.len() < d || out.len() < d {
        return;
    }
    let mut delta = [0.0f32; 256];
    if d > delta.len() {
        return;
    }
    // Recorridos por filas del estado (contiguos): cada `delta[j]` y `out[j]` se acumula en el mismo orden de `i`
    // que con el bucle por columnas, así que el resultado es idéntico bit a bit y el compilador puede vectorizar.
    for j in 0..d {
        delta[j] = 0.0;
        out[j] = 0.0;
    }
    for i in 0..d {
        let (fila, ki) = (&state[i * d..(i + 1) * d], k[i]);
        for j in 0..d {
            delta[j] += fila[j] * ki;
        }
    }
    for j in 0..d {
        delta[j] = (v[j] - g * delta[j]) * beta;
    }
    for i in 0..d {
        let (ki, qi) = (k[i], q[i]);
        let fila = &mut state[i * d..(i + 1) * d];
        for j in 0..d {
            let ns = g * fila[j] + ki * delta[j];
            fila[j] = ns;
            out[j] += ns * qi;
        }
    }
}

pub(crate) fn conv1d_causal_step(x: &[f32], weight: &[f32], kernel: usize, state: &mut [f32], out: &mut [f32]) {
    let c = x.len();
    if kernel == 0 || weight.len() < c * kernel || out.len() < c {
        return;
    }
    let prev = kernel.saturating_sub(1);
    for ch in 0..c {
        let mut acc = 0.0f32;
        for k in 0..prev {
            let sidx = k * c + ch;
            if sidx < state.len() {
                acc += weight[ch * kernel + k] * state[sidx];
            }
        }
        acc += weight[ch * kernel + prev] * x[ch];
        out[ch] = acc;
    }
    if prev == 0 {
        return;
    }
    if state.len() < prev * c {
        return;
    }
    if prev > 1 {
        state.copy_within(c.., 0);
    }
    let dest = (prev - 1) * c;
    state[dest..dest + c].copy_from_slice(x);
}

/// Atención gated Qwen3.5/3.8 (Q+gate fusionados, QK-norm, RoPE parcial).
pub fn forward_gated_attn<S: TensorSource>(
    spec: &LayerSpec,
    manifest: &Manifest,
    layer: u32,
    pos: usize,
    prefix: &str,
    hidden: &mut [f32],
    s: &mut LayerScratch,
    kv: &mut LayerKv,
    source: &mut S,
    gpu: &mut Option<&mut dyn GpuDispatch>,
    use_gpu: bool,
    par: &dyn RowParallel,
    planner: Option<&ResourcePlanner>,
    clock_ms: Option<fn() -> u64>,
) -> Result<(u64, u64), ()> {
    let tick = |c: Option<fn() -> u64>| c.map(|f| f()).unwrap_or(0);
    let h = manifest.hidden_dim as usize;
    let heads = manifest.effective_num_heads(layer) as usize;
    let kv_heads = manifest.effective_num_kv_heads(layer) as usize;
    let head_dim = manifest.effective_head_dim(layer) as usize;
    let kv_dim = kv_heads * head_dim;
    let q_dim = heads * head_dim;
    let q_full = q_dim * 2;
    let group = heads / kv_heads;
    let eps = manifest.rms_eps;
    let theta = manifest.rope_theta;
    let rope_n = if spec.qk_rope_head_dim > 0 {
        spec.qk_rope_head_dim as usize
    } else {
        head_dim
    };

    if s.q.len() < q_full || s.gate.len() < q_dim || s.k.len() < kv_dim || s.v.len() < kv_dim {
        return Err(());
    }
    if s.attn_out.len() < q_dim || s.up.len() < q_dim || s.head_out.len() < head_dim {
        return Err(());
    }

    s.residual.copy_from_slice(hidden);
    source.load_f32(&format!("{prefix}.attn_norm"), &mut s.norm_w[..h])?;
    rmsnorm(hidden, &s.norm_w[..h], eps);

    let t_mv0 = tick(clock_ms);
    let name_q = format!("{prefix}.attn_q");
    let name_k = format!("{prefix}.attn_k");
    let name_v = format!("{prefix}.attn_v");
    let name_out = format!("{prefix}.attn_output");
    let mut lote = crate::layer::Lote::nuevo();
    lote.anadir(layer, &name_q, source.tensor_view(&name_q)?, q_full, h, hidden, &mut s.q[..q_full])?;
    lote.anadir(layer, &name_k, source.tensor_view(&name_k)?, kv_dim, h, hidden, &mut s.k[..kv_dim])?;
    lote.anadir(layer, &name_v, source.tensor_view(&name_v)?, kv_dim, h, hidden, &mut s.v[..kv_dim])?;
    lote.ejecutar(par, gpu, use_gpu)?;

    for head in 0..heads {
        let src = head * 2 * head_dim;
        s.up[head * head_dim..(head + 1) * head_dim]
            .copy_from_slice(&s.q[src..src + head_dim]);
        s.gate[head * head_dim..(head + 1) * head_dim]
            .copy_from_slice(&s.q[src + head_dim..src + 2 * head_dim]);
    }

    source.load_f32(&format!("{prefix}.attn_q_norm"), &mut s.norm_w[..head_dim])?;
    for head in 0..heads {
        rmsnorm(
            &mut s.up[head * head_dim..(head + 1) * head_dim],
            &s.norm_w[..head_dim],
            eps,
        );
        rope_inplace_n(
            &mut s.up[head * head_dim..(head + 1) * head_dim],
            pos,
            theta,
            rope_n,
        );
    }
    source.load_f32(&format!("{prefix}.attn_k_norm"), &mut s.norm_w[..head_dim])?;
    for head in 0..kv_heads {
        rmsnorm(
            &mut s.k[head * head_dim..(head + 1) * head_dim],
            &s.norm_w[..head_dim],
            eps,
        );
        rope_inplace_n(
            &mut s.k[head * head_dim..(head + 1) * head_dim],
            pos,
            theta,
            rope_n,
        );
    }

    let t_attn0 = tick(clock_ms);
    kv.append_f16(&s.k[..kv_dim], &s.v[..kv_dim]);
    let seq = kv.tokens(kv_dim);
    if kv.dtype == crate::kv::KvDtype::F16 {
        // Kernel por grupos GQA (AVX2, repartido entre hilos): el camino de siempre para el resto de modelos.
        crate::layer::atencion_f16_grupos(par, &s.up[..q_dim], &mut s.attn_out[..q_dim], 1, q_dim, kv, kv_heads, group, head_dim, kv_dim, seq);
        for i in 0..q_dim {
            s.attn_out[i] *= sigmoid(s.gate[i]);
        }
    } else {
        s.attn_out[..q_dim].fill(0.0);
        for head in 0..heads {
            let q_h = &s.up[head * head_dim..(head + 1) * head_dim];
            let kv_head = head / group;
            crate::attn::attention_decode_kv(q_h, kv, head_dim, kv_dim, kv_head, seq, &mut s.head_out[..head_dim], None, false, 0);
            let gbase = head * head_dim;
            for d in 0..head_dim {
                s.attn_out[gbase + d] = s.head_out[d] * sigmoid(s.gate[gbase + d]);
            }
        }
    }
    let t_attn1 = tick(clock_ms);

    matvec_step(
        use_gpu,
        gpu,
        &name_out,
        source.tensor_view(&name_out)?,
        h,
        q_dim,
        &s.attn_out[..q_dim],
        &mut s.q[..h],
        par,
        planner,
        layer,
    )?;
    add_f32(&s.residual, &s.q[..h], hidden);
    let t_mv1 = tick(clock_ms);
    Ok((
        t_attn0.saturating_sub(t_mv0).saturating_add(t_mv1.saturating_sub(t_attn1)),
        t_attn1.saturating_sub(t_attn0),
    ))
}

/// Gated DeltaNet (Qwen3.5/3.8): conv1d causal + recuerrencia GDA + norma gated.
pub fn forward_gdn_attn<S: TensorSource>(
    spec: &LayerSpec,
    manifest: &Manifest,
    layer: u32,
    _pos: usize,
    prefix: &str,
    hidden: &mut [f32],
    s: &mut LayerScratch,
    kv: &mut LayerKv,
    source: &mut S,
    gpu: &mut Option<&mut dyn GpuDispatch>,
    use_gpu: bool,
    par: &dyn RowParallel,
    planner: Option<&ResourcePlanner>,
    clock_ms: Option<fn() -> u64>,
) -> Result<(u64, u64), ()> {
    let tick = |c: Option<fn() -> u64>| c.map(|f| f()).unwrap_or(0);
    let h = manifest.hidden_dim as usize;
    let n_k = manifest.effective_num_heads(layer) as usize;
    let n_v = manifest.effective_num_kv_heads(layer) as usize;
    let d = manifest.effective_head_dim(layer) as usize;
    let kernel = spec.qk_rope_head_dim.max(1) as usize;
    let qk_dim = n_k * d;
    let v_dim = n_v * d;
    let qkv_dim = qk_dim * 2 + v_dim;
    let eps = manifest.rms_eps;
    let conv_len = kernel.saturating_sub(1).saturating_mul(qkv_dim);
    kv.ensure_gdn(n_v, d, conv_len);

    if s.q.len() < qkv_dim
        || s.up.len() < qkv_dim
        || s.gate.len() < v_dim
        || s.k.len() < v_dim
        || s.v.len() < v_dim
        || s.attn_out.len() < v_dim
        || s.router.len() < n_v
        || s.moe_acc.len() < n_v
        || s.head_out.len() < d
    {
        return Err(());
    }

    s.residual.copy_from_slice(hidden);
    source.load_f32(&format!("{prefix}.attn_norm"), &mut s.norm_w[..h])?;
    rmsnorm(hidden, &s.norm_w[..h], eps);

    let t_mv0 = tick(clock_ms);
    let name_qkv = format!("{prefix}.attn_qkv");
    let name_z = format!("{prefix}.attn_gate");
    let name_beta = format!("{prefix}.ssm_beta");
    let name_alpha = format!("{prefix}.ssm_alpha");
    let name_out = format!("{prefix}.ssm_out");
    let mut lote = crate::layer::Lote::nuevo();
    lote.anadir(layer, &name_qkv, source.tensor_view(&name_qkv)?, qkv_dim, h, hidden, &mut s.q[..qkv_dim])?;
    lote.anadir(layer, &name_z, source.tensor_view(&name_z)?, v_dim, h, hidden, &mut s.gate[..v_dim])?;
    lote.anadir(layer, &name_beta, source.tensor_view(&name_beta)?, n_v, h, hidden, &mut s.router[..n_v])?;
    lote.anadir(layer, &name_alpha, source.tensor_view(&name_alpha)?, n_v, h, hidden, &mut s.moe_acc[..n_v])?;
    lote.ejecutar(par, gpu, use_gpu)?;

    let mut dt = [0.0f32; 256];
    let mut a = [0.0f32; 256];
    if n_v > dt.len() {
        return Err(());
    }
    source.load_f32(&format!("{prefix}.ssm_dt"), &mut dt[..n_v])?;
    source.load_f32(&format!("{prefix}.ssm_a"), &mut a[..n_v])?;
    for i in 0..n_v {
        let g = softplus(s.moe_acc[i] + dt[i]) * a[i];
        s.moe_acc[i] = libm::expf(g);
        s.router[i] = sigmoid(s.router[i]);
    }

    let conv_name = format!("{prefix}.ssm_conv1d");
    let view = source.tensor_view(&conv_name)?;
    if let Some(w) = view.f32() {
        conv1d_causal_step(
            &s.q[..qkv_dim],
            w,
            kernel,
            &mut kv.gdn_conv,
            &mut s.up[..qkv_dim],
        );
    } else {
        let mut row = [0.0f32; 16];
        if kernel > row.len() {
            return Err(());
        }
        let c = qkv_dim;
        let prev = kernel.saturating_sub(1);
        for ch in 0..c {
            source.load_f32_range(&conv_name, ch * kernel, &mut row[..kernel])?;
            let mut acc = 0.0f32;
            for k in 0..prev {
                let sidx = k * c + ch;
                if sidx < kv.gdn_conv.len() {
                    acc += row[k] * kv.gdn_conv[sidx];
                }
            }
            acc += row[prev] * s.q[ch];
            s.up[ch] = acc;
        }
        if prev > 0 && kv.gdn_conv.len() >= prev * c {
            if prev > 1 {
                kv.gdn_conv.copy_within(c.., 0);
            }
            let dest = (prev - 1) * c;
            kv.gdn_conv[dest..dest + c].copy_from_slice(&s.q[..c]);
        }
    }
    for v in s.up[..qkv_dim].iter_mut() {
        *v = silu(*v);
    }

    // q, k, v tras conv+silu
    let q_off = 0;
    let k_off = qk_dim;
    let v_off = qk_dim * 2;
    for head in 0..n_k {
        l2_normalize(&mut s.up[q_off + head * d..q_off + (head + 1) * d], eps);
        l2_normalize(&mut s.up[k_off + head * d..k_off + (head + 1) * d], eps);
    }
    let scale = 1.0 / libm::sqrtf(d as f32);
    for i in q_off..q_off + qk_dim {
        s.up[i] *= scale;
    }

    let t_attn0 = tick(clock_ms);
    source.load_f32(&format!("{prefix}.ssm_norm"), &mut s.norm_w[..d])?;
    // Las cabezas son independientes (estado, salida y puerta propios; q/k/v de solo lectura): se reparten entre hilos.
    {
        let up = s.up.as_ptr() as usize;
        let gate = s.gate.as_ptr() as usize;
        let decay = s.moe_acc.as_ptr() as usize;
        let beta = s.router.as_ptr() as usize;
        let normw = s.norm_w.as_ptr() as usize;
        let out = s.attn_out.as_mut_ptr() as usize;
        let state = kv.gdn_s.as_mut_ptr() as usize;
        par.for_rows(n_v, &move |h0, h1| {
            for vh in h0..h1 {
                let kh = vh % n_k; // ggml_repeat: las cabezas q/k se repiten en mosaico, no entrelazadas
                // SAFETY: cada `vh` toca sólo su porción de `out`, `state` y lee el resto; `for_rows` espera a todos.
                let (qh, khs, vv, st, o, nw, gt) = unsafe {
                    (
                        core::slice::from_raw_parts((up as *const f32).add(q_off + kh * d), d),
                        core::slice::from_raw_parts((up as *const f32).add(k_off + kh * d), d),
                        core::slice::from_raw_parts((up as *const f32).add(v_off + vh * d), d),
                        core::slice::from_raw_parts_mut((state as *mut f32).add(vh * d * d), d * d),
                        core::slice::from_raw_parts_mut((out as *mut f32).add(vh * d), d),
                        core::slice::from_raw_parts(normw as *const f32, d),
                        core::slice::from_raw_parts((gate as *const f32).add(vh * d), d),
                    )
                };
                let (g, b) = unsafe { (*(decay as *const f32).add(vh), *(beta as *const f32).add(vh)) };
                gdn_step(st, qh, khs, vv, g, b, o);
                rmsnorm(o, nw, eps);
                for i in 0..d {
                    o[i] *= silu(gt[i]);
                }
            }
        });
    }
    let t_attn1 = tick(clock_ms);

    matvec_step(
        use_gpu,
        gpu,
        &name_out,
        source.tensor_view(&name_out)?,
        h,
        v_dim,
        &s.attn_out[..v_dim],
        &mut s.q[..h],
        par,
        planner,
        layer,
    )?;
    add_f32(&s.residual, &s.q[..h], hidden);
    let t_mv1 = tick(clock_ms);
    Ok((
        t_attn0.saturating_sub(t_mv0).saturating_add(t_mv1.saturating_sub(t_attn1)),
        t_attn1.saturating_sub(t_attn0),
    ))
}

pub fn forward_latent_moe_ffn<S: TensorSource>(
    manifest: &Manifest,
    layer: u32,
    prefix: String,
    h: usize,
    latent: usize,
    hidden: &mut [f32],
    s: &mut LayerScratch,
    source: &mut S,
    gpu: &mut Option<&mut dyn GpuDispatch>,
    use_gpu: bool,
    par: &dyn RowParallel,
    planner: Option<&mut ResourcePlanner>,
    index: Option<&TensorIndex>,
) -> Result<(), ()> {
    let n_exp = manifest.effective_num_experts(layer) as usize;
    let top_k = manifest.effective_num_experts_per_tok(layer) as usize;
    if n_exp == 0 || top_k == 0 || s.router.len() < n_exp {
        return Err(());
    }
    let name_in = format!("{prefix}.ffn_latent_in");
    let name_out = format!("{prefix}.ffn_latent_out");
    let name_router = format!("{prefix}.ffn_gate_inp");

    if s.up.len() < latent.max(h) {
        return Err(());
    }

    matvec_step(
        use_gpu,
        gpu,
        &name_in,
        source.tensor_view(&name_in)?,
        latent,
        h,
        hidden,
        &mut s.up[..latent],
        par,
        planner.as_deref(),
        layer,
    )?;

    matvec_step(
        use_gpu,
        gpu,
        &name_router,
        source.tensor_view(&name_router)?,
        n_exp,
        latent,
        &s.up[..latent],
        &mut s.router[..n_exp],
        par,
        planner.as_deref(),
        layer,
    )?;
    let ranked = topk_softmax(&mut s.router[..n_exp], top_k);

    s.moe_acc.fill(0.0);
    for (expert, weight) in &ranked {
        let ep = format!("{prefix}.E{expert:02}");
        let name_gate = format!("{ep}.ffn_gate");
        let name_up = format!("{ep}.ffn_up");
        let name_down = format!("{ep}.ffn_down");
        matvec_step(
            use_gpu,
            gpu,
            &name_gate,
            source.tensor_view(&name_gate)?,
            latent,
            latent,
            &s.up[..latent],
            &mut s.gate[..latent],
            par,
            planner.as_deref(),
            layer,
        )?;
        matvec_step(
            use_gpu,
            gpu,
            &name_up,
            source.tensor_view(&name_up)?,
            latent,
            latent,
            &s.up[..latent],
            &mut s.q[..latent],
            par,
            planner.as_deref(),
            layer,
        )?;
        swiglu_inplace(&mut s.q[..latent], &s.gate[..latent]);
        matvec_step(
            use_gpu,
            gpu,
            &name_down,
            source.tensor_view(&name_down)?,
            latent,
            latent,
            &s.q[..latent],
            &mut s.gate[..latent],
            par,
            planner.as_deref(),
            layer,
        )?;
        for i in 0..latent {
            s.moe_acc[i] += s.gate[i] * weight;
        }
        let _ = index;
    }

    matvec_step(
        use_gpu,
        gpu,
        &name_out,
        source.tensor_view(&name_out)?,
        h,
        latent,
        &s.moe_acc[..latent],
        hidden,
        par,
        planner.as_deref(),
        layer,
    )?;
    Ok(())
}
