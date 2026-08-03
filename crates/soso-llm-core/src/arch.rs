//! Ramas de arquitectura extendidas (MLA, KDA, LatentMoE).

use crate::attn;
use crate::gemm::{add_f32, rmsnorm, rope_inplace, swiglu_inplace, topk_softmax};
use crate::gpu::GpuDispatch;
use crate::kv::{KvDtype, LayerKv};
use crate::layer::{matvec_step, LayerScratch, TensorSource};
use crate::plan::ResourcePlanner;
use crate::parallel::RowParallel;
use alloc::format;
use alloc::string::String;
use sosomodel::index::TensorIndex;
use sosomodel::manifest::{LayerSpec, Manifest};

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
    let kv_dim = kv_heads * head_dim;
    let group = heads / kv_heads;
    let q_rank = spec.q_lora_rank as usize;
    let kv_rank = spec.kv_lora_rank as usize;
    let eps = manifest.rms_eps;
    let theta = manifest.rope_theta;

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
    matvec_step(
        use_gpu,
        gpu,
        &name_k_up,
        source.tensor_view(&name_k_up)?,
        kv_dim,
        kv_rank,
        &s.up[..kv_rank],
        &mut s.k[..kv_dim],
        par,
        planner,
        layer,
    )?;
    matvec_step(
        use_gpu,
        gpu,
        &name_v_up,
        source.tensor_view(&name_v_up)?,
        kv_dim,
        kv_rank,
        &s.up[..kv_rank],
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
        if matches!(kv.dtype, KvDtype::F16) {
            attn::attention_decode_f16_tiled(
                q_h,
                kv.k_f16_slice(),
                kv.v_f16_slice(),
                head_dim,
                kv_dim,
                kv_head,
                seq,
                &mut s.head_out,
            );
        } else {
            attn::attention_decode_kv(
                q_h,
                kv,
                head_dim,
                kv_dim,
                kv_head,
                seq,
                &mut s.head_out,
                None,
                false,
            );
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

pub fn run_shared_expert<S: TensorSource>(
    prefix: &str,
    shared: u32,
    h: usize,
    ffn: usize,
    hidden: &[f32],
    s: &mut LayerScratch,
    source: &mut S,
    gpu: &mut Option<&mut dyn GpuDispatch>,
    use_gpu: bool,
    par: &dyn RowParallel,
    planner: Option<&ResourcePlanner>,
    layer: u32,
    acc: &mut [f32],
) -> Result<(), ()> {
    let ep = format!("{prefix}.S{shared:02}");
    let name_gate = format!("{ep}.ffn_gate");
    let name_up = format!("{ep}.ffn_up");
    let name_down = format!("{ep}.ffn_down");
    matvec_step(
        use_gpu,
        gpu,
        &name_gate,
        source.tensor_view(&name_gate)?,
        ffn,
        h,
        hidden,
        &mut s.gate,
        par,
        planner,
        layer,
    )?;
    matvec_step(
        use_gpu,
        gpu,
        &name_up,
        source.tensor_view(&name_up)?,
        ffn,
        h,
        hidden,
        &mut s.up,
        par,
        planner,
        layer,
    )?;
    swiglu_inplace(&mut s.up, &s.gate);
    matvec_step(
        use_gpu,
        gpu,
        &name_down,
        source.tensor_view(&name_down)?,
        h,
        ffn,
        &s.up,
        &mut s.q,
        par,
        planner,
        layer,
    )?;
    for i in 0..h {
        acc[i] += s.q[i];
    }
    Ok(())
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
