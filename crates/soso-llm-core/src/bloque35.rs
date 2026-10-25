//! Prefill por bloques de las capas de Qwen3.5 / 3.6 (atención con puerta y Gated DeltaNet, FFN densa o MoE con experto
//! compartido).
//!
//! Hasta aquí estas capas iban token a token: cada token releía todos los pesos (9 tok/s de prefill en el 35B-A3B). Aquí un bloque de
//! `n` tokens lee cada matriz **una vez**:
//! - las proyecciones (qkv, z, β, α, salida, FFN densa) son matvec por lotes (`matvec_view_batch_par`);
//! - la recurrencia de GDN y la atención van token a token (dependen del estado) con el mismo código de siempre;
//! - en el MoE se agrupan los tokens por experto: cada experto lee sus pesos una vez para todos los tokens que lo eligen. La suma de
//!   los expertos de cada token se hace en el orden de su ranking, como en la decodificación, así que el resultado coincide.
//!
//! El resultado es el de `forward_layer` repetido `n` veces (salvo el orden de sumas interno de los kernels por lotes).

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use sosomodel::layout::DTYPE_F32;
use sosomodel::manifest::{AttnKind, FfnKind};

use crate::arch::{gdn_step, l2_normalize, sigmoid, softplus};
use crate::gemm::{add_f32, rmsnorm, rope_inplace_n, silu, swiglu_inplace, topk_softmax};
use crate::kv::{KvDtype, LayerKv};
use crate::layer::{atencion_f16_grupos, perfil_ahora, perfil_marca, FASE_ATENCION, FASE_FFN_UP_GATE, FASE_O, FASE_QKV, FASE_ROPE_KV, matvec_view_batch_par, BlockScratch, LayerExecutor, LayerScratch, TensorSource};
use crate::parallel::{RowParallel, Sequential};

/// ¿Esta capa es de las que sabe hacer este módulo?
pub fn soporta(exec: &LayerExecutor<'_>, layer: u32, kv: &LayerKv) -> bool {
    let Some(spec) = exec.manifest.layer(layer) else { return false };
    match spec.attn_kind {
        AttnKind::Gdn => {}
        AttnKind::Gated => {
            if kv.dtype != KvDtype::F16 {
                return false;
            }
        }
        _ => return false,
    }
    spec.ffn_kind != FfnKind::LatentMoe && !exec.manifest.ext.yarn.activa() && exec.manifest.layer_window(layer) == 0
}

/// Una capa de `n` tokens (`hs`, `n` vectores de `hidden_dim`, se actualiza en su sitio).
#[allow(clippy::too_many_arguments)]
pub fn forward_layer_block_q35<S: TensorSource>(
    exec: &LayerExecutor<'_>,
    layer: u32,
    pos0: usize,
    n: usize,
    hs: &mut [f32],
    bs: &mut BlockScratch,
    s: &mut LayerScratch,
    kv: &mut LayerKv,
    source: &mut S,
    gpu: &mut Option<&mut dyn crate::gpu::GpuDispatch>,
    use_gpu: bool,
) -> Result<(), ()> {
    let m = exec.manifest;
    let spec = m.layer(layer).cloned().ok_or(())?;
    let h = m.hidden_dim as usize;
    let eps = m.rms_eps;
    let seq_par = Sequential;
    let par: &dyn RowParallel = exec.parallel.unwrap_or(&seq_par);
    if hs.len() != n * h || n == 0 || n > bs.cap {
        return Err(());
    }
    let prefix = format!("L{layer:02}");
    let residual: Vec<f32> = hs.to_vec();
    source.load_f32(&format!("{prefix}.attn_norm"), &mut s.norm_w[..h])?;
    for i in 0..n {
        rmsnorm(&mut hs[i * h..(i + 1) * h], &s.norm_w[..h], eps);
    }

    let mut salida_attn = vec![0.0f32; n * h];
    match spec.attn_kind {
        AttnKind::Gdn => gdn_bloque(exec, layer, n, hs, &mut salida_attn, bs, s, kv, source, par, &prefix, gpu, use_gpu)?,
        _ => atencion_bloque(exec, layer, pos0, n, hs, &mut salida_attn, bs, s, kv, source, par, &prefix, gpu, use_gpu)?,
    }
    // hs = residual + salida de atención; luego FFN con su residual.
    for i in 0..n {
        add_f32(&residual[i * h..(i + 1) * h], &salida_attn[i * h..(i + 1) * h], &mut hs[i * h..(i + 1) * h]);
    }
    drop(salida_attn);

    let residual2: Vec<f32> = hs.to_vec();
    source.load_f32(&format!("{prefix}.ffn_norm"), &mut s.norm_w[..h])?;
    for i in 0..n {
        rmsnorm(&mut hs[i * h..(i + 1) * h], &s.norm_w[..h], eps);
    }
    let mut salida_ffn = vec![0.0f32; n * h];
    if m.layer_is_moe(layer) {
        moe_bloque(exec, layer, n, hs, &mut salida_ffn, bs, par, source, &prefix, gpu, use_gpu)?;
    } else {
        ffn_denso_bloque(exec, layer, n, hs, &mut salida_ffn, bs, par, source, &prefix, gpu, use_gpu)?;
    }
    for i in 0..n {
        add_f32(&residual2[i * h..(i + 1) * h], &salida_ffn[i * h..(i + 1) * h], &mut hs[i * h..(i + 1) * h]);
    }
    Ok(())
}

fn mv<S: TensorSource>(
    source: &mut S,
    name: &str,
    layer: u32,
    rows: usize,
    cols: usize,
    xs: &[f32],
    n: usize,
    out: &mut [f32],
    par: &dyn RowParallel,
    bs: &mut BlockScratch,
    gpu: &mut Option<&mut dyn crate::gpu::GpuDispatch>,
    use_gpu: bool,
) -> Result<(), ()> {
    crate::layer::note_infer_op(Some(layer), name);
    let v = source.tensor_view(name)?;
    // Con GPU, las matrices que ella sabe multiplicar por varios tokens a la vez (pesos residentes, una lectura para los `n` vectores).
    if use_gpu {
        if let Some(g) = gpu.as_deref_mut() {
            if matches!(crate::gpu::try_gpu_matmul(g, name, &v, rows, cols, n, xs, out), Ok(true)) {
                return Ok(());
            }
        }
    }
    bs.ensure_q8k(n, cols);
    matvec_view_batch_par(&v, rows, cols, xs, n, out, par, Some(&mut bs.q8k_x))
}

#[allow(clippy::too_many_arguments)]
fn gdn_bloque<S: TensorSource>(
    exec: &LayerExecutor<'_>,
    layer: u32,
    n: usize,
    xs: &[f32],
    salida: &mut [f32],
    bs: &mut BlockScratch,
    s: &mut LayerScratch,
    kv: &mut LayerKv,
    source: &mut S,
    par: &dyn RowParallel,
    prefix: &str,
    gpu: &mut Option<&mut dyn crate::gpu::GpuDispatch>,
    use_gpu: bool,
) -> Result<(), ()> {
    let m = exec.manifest;
    let spec = m.layer(layer).cloned().ok_or(())?;
    let h = m.hidden_dim as usize;
    let n_k = m.effective_num_heads(layer) as usize;
    let n_v = m.effective_num_kv_heads(layer) as usize;
    let d = m.effective_head_dim(layer) as usize;
    let kernel = spec.qk_rope_head_dim.max(1) as usize;
    let (qk_dim, v_dim) = (n_k * d, n_v * d);
    let qkv_dim = qk_dim * 2 + v_dim;
    let eps = m.rms_eps;
    kv.ensure_gdn(n_v, d, kernel.saturating_sub(1).saturating_mul(qkv_dim));

    let mut tp = perfil_ahora();
    // Con GPU, la capa entera en el dispositivo (proyecciones, conv, recurrencia y salida) sin llevar las activaciones intermedias por el bus.
    if use_gpu && n >= kernel && d == 128 {
        let nombres = [format!("{prefix}.attn_qkv"), format!("{prefix}.attn_gate"), format!("{prefix}.ssm_out")];
        let mut dt = vec![0.0f32; n_v];
        let mut a = vec![0.0f32; n_v];
        let mut norm_w = vec![0.0f32; d];
        source.load_f32(&format!("{prefix}.ssm_dt"), &mut dt)?;
        source.load_f32(&format!("{prefix}.ssm_a"), &mut a)?;
        source.load_f32(&format!("{prefix}.ssm_norm"), &mut norm_w)?;
        let mut crudo = |source: &mut S, name: &str| -> Result<(usize, usize, u8, usize), ()> {
            let v = source.tensor_view(name)?;
            Ok((v.bytes.as_ptr() as usize, v.bytes.len(), v.dtype, v.elems))
        };
        let (rq, rz, ro) = (crudo(source, &nombres[0])?, crudo(source, &nombres[1])?, crudo(source, &nombres[2])?);
        let (rb, ra, rc) = (
            crudo(source, &format!("{prefix}.ssm_beta"))?,
            crudo(source, &format!("{prefix}.ssm_alpha"))?,
            crudo(source, &format!("{prefix}.ssm_conv1d"))?,
        );
        let mk = |r: (usize, usize, u8, usize)| crate::layer::TensorView { bytes: unsafe { core::slice::from_raw_parts(r.0 as *const u8, r.1) }, dtype: r.2, elems: r.3 };
        let (vb, va, vc) = (mk(rb), mk(ra), mk(rc));
        if let (Some(bw), Some(aw), Some(cw)) = (vb.f32(), va.f32(), vc.f32()) {
            let (tq, tz, to) = (mk(rq), mk(rz), mk(ro));
            let capa = crate::gpu::GdnCapa {
                claves: [nombres[0].as_str(), nombres[1].as_str(), nombres[2].as_str()],
                vistas: [&tq, &tz, &to],
                beta_w: bw,
                alpha_w: aw,
                conv_w: cw,
                dt: &dt,
                a: &a,
                norma: &norm_w,
                n_k,
                n_v,
                d,
                kernel,
                eps,
            };
            let prev = kernel - 1;
            if kv.gdn_conv.len() >= prev * qkv_dim {
                let ok = gpu.as_deref_mut().is_some_and(|g| {
                    matches!(g.gdn_capa(&capa, xs, n, h, &mut kv.gdn_conv[..prev * qkv_dim], &mut kv.gdn_s, salida), Ok(true))
                });
                if ok {
                    perfil_marca(&mut tp, FASE_ROPE_KV);
                    return Ok(());
                }
            }
        }
    }
    let mut qkv = vec![0.0f32; n * qkv_dim];
    let mut z = vec![0.0f32; n * v_dim];
    let mut beta = vec![0.0f32; n * n_v];
    let mut alpha = vec![0.0f32; n * n_v];
    mv(source, &format!("{prefix}.attn_qkv"), layer, qkv_dim, h, xs, n, &mut qkv, par, bs, gpu, use_gpu)?;
    mv(source, &format!("{prefix}.attn_gate"), layer, v_dim, h, xs, n, &mut z, par, bs, gpu, use_gpu)?;
    mv(source, &format!("{prefix}.ssm_beta"), layer, n_v, h, xs, n, &mut beta, par, bs, gpu, use_gpu)?;
    mv(source, &format!("{prefix}.ssm_alpha"), layer, n_v, h, xs, n, &mut alpha, par, bs, gpu, use_gpu)?;

    perfil_marca(&mut tp, FASE_QKV);
    let mut dt = vec![0.0f32; n_v];
    let mut a = vec![0.0f32; n_v];
    source.load_f32(&format!("{prefix}.ssm_dt"), &mut dt)?;
    source.load_f32(&format!("{prefix}.ssm_a"), &mut a)?;
    let mut norm_w = vec![0.0f32; d];
    source.load_f32(&format!("{prefix}.ssm_norm"), &mut norm_w)?;
    let conv_name = format!("{prefix}.ssm_conv1d");
    let conv_view = source.tensor_view(&conv_name)?;
    if conv_view.dtype != DTYPE_F32 {
        return Err(());
    }
    let conv = conv_view.f32().ok_or(())?;

    // 1) conv causal + SiLU + normas de q/k de **todos** los tokens (la conv arrastra estado entre tokens, pero es barata) y las
    //    constantes de decaimiento y β de cada token y cabeza.
    let mut ups = vec![0.0f32; n * qkv_dim];
    let mut decay = vec![0.0f32; n * n_v];
    let mut beta_t = vec![0.0f32; n * n_v];
    let escala = 1.0 / libm::sqrtf(d as f32);
    // La conv causal es función de las `kernel - 1` entradas anteriores (estado) y las del bloque: los tokens son independientes entre sí,
    // así que se reparten entre hilos. `hist` = estado (más antiguo primero) seguido de las entradas del bloque.
    let prev = kernel.saturating_sub(1);
    if kv.gdn_conv.len() < prev * qkv_dim {
        return Err(());
    }
    let mut hist = vec![0.0f32; (prev + n) * qkv_dim];
    hist[..prev * qkv_dim].copy_from_slice(&kv.gdn_conv[..prev * qkv_dim]);
    hist[prev * qkv_dim..].copy_from_slice(&qkv[..n * qkv_dim]);
    // Con GPU (y d = 128), conv + recurrencia + puerta del bloque entero en el dispositivo; el estado de la conv lo actualiza igualmente el host.
    let mut attn_out = vec![0.0f32; n * v_dim];
    let en_gpu = use_gpu && {
        let mut dec = vec![0.0f32; n * n_v];
        let mut bet = vec![0.0f32; n * n_v];
        for t in 0..n {
            for i in 0..n_v {
                dec[t * n_v + i] = libm::expf(softplus(alpha[t * n_v + i] + dt[i]) * a[i]);
                bet[t * n_v + i] = sigmoid(beta[t * n_v + i]);
            }
        }
        gpu.as_deref_mut().is_some_and(|g| {
            matches!(g.gdn_bloque(&hist, conv, &z, &dec, &bet, &norm_w, &mut kv.gdn_s, &mut attn_out, n, n_k, n_v, d, kernel, eps), Ok(true))
        })
    };
    if en_gpu {
        if prev > 0 {
            kv.gdn_conv[..prev * qkv_dim].copy_from_slice(&hist[n * qkv_dim..(n + prev) * qkv_dim]);
        }
        perfil_marca(&mut tp, FASE_ROPE_KV);
        let r = mv(source, &format!("{prefix}.ssm_out"), layer, h, v_dim, &attn_out, n, salida, par, bs, gpu, use_gpu);
        perfil_marca(&mut tp, FASE_O);
        return r;
    }
    {
        let (hist_p, ups_p, conv_p) = (hist.as_ptr() as usize, ups.as_mut_ptr() as usize, conv.as_ptr() as usize);
        let (dec_p, bet_p, alpha_p, beta_raw_p, dt_p, a_p) = (
            decay.as_mut_ptr() as usize,
            beta_t.as_mut_ptr() as usize,
            alpha.as_ptr() as usize,
            beta.as_ptr() as usize,
            dt.as_ptr() as usize,
            a.as_ptr() as usize,
        );
        par.for_rows(n, &move |t0, t1| {
            for t in t0..t1 {
                // SAFETY: cada token escribe sólo su fila de `ups`/`decay`/`beta_t`; lo demás es de solo lectura.
                unsafe {
                    for i in 0..n_v {
                        *(dec_p as *mut f32).add(t * n_v + i) = libm::expf(
                            softplus(*(alpha_p as *const f32).add(t * n_v + i) + *(dt_p as *const f32).add(i)) * *(a_p as *const f32).add(i),
                        );
                        *(bet_p as *mut f32).add(t * n_v + i) = sigmoid(*(beta_raw_p as *const f32).add(t * n_v + i));
                    }
                    let up = core::slice::from_raw_parts_mut((ups_p as *mut f32).add(t * qkv_dim), qkv_dim);
                    for ch in 0..qkv_dim {
                        let mut acc = 0.0f32;
                        for k in 0..prev {
                            acc += *(conv_p as *const f32).add(ch * kernel + k) * *(hist_p as *const f32).add((t + k) * qkv_dim + ch);
                        }
                        acc += *(conv_p as *const f32).add(ch * kernel + prev) * *(hist_p as *const f32).add((t + prev) * qkv_dim + ch);
                        up[ch] = silu(acc);
                    }
                    for head in 0..n_k {
                        l2_normalize(&mut up[head * d..(head + 1) * d], eps);
                        l2_normalize(&mut up[qk_dim + head * d..qk_dim + (head + 1) * d], eps);
                    }
                    for v in up[..qk_dim].iter_mut() {
                        *v *= escala;
                    }
                }
            }
        });
    }
    // Estado de la conv tras el bloque: las `prev` últimas entradas.
    if prev > 0 {
        kv.gdn_conv[..prev * qkv_dim].copy_from_slice(&hist[n * qkv_dim..(n + prev) * qkv_dim]);
    }
    // 2) recurrencia: cada cabeza recorre sus `n` tokens seguidos (su estado es independiente), así que el bloque entero cuesta una
    //    sola barrera en vez de una por token. Mismo cálculo, mismo orden por cabeza: el resultado no cambia.
    attn_out.fill(0.0);
    {
        let (up_p, decay_p, beta_p, normw_p, out_p, st_p, z_p) = (
            ups.as_ptr() as usize,
            decay.as_ptr() as usize,
            beta_t.as_ptr() as usize,
            norm_w.as_ptr() as usize,
            attn_out.as_mut_ptr() as usize,
            kv.gdn_s.as_mut_ptr() as usize,
            z.as_ptr() as usize,
        );
        par.for_rows(n_v, &move |h0, h1| {
            for vh in h0..h1 {
                let kh = vh % n_k;
                // SAFETY: cada cabeza toca su porción de `attn_out` y de `state`; el resto es de solo lectura y `for_rows` espera.
                unsafe {
                    let st = core::slice::from_raw_parts_mut((st_p as *mut f32).add(vh * d * d), d * d);
                    let nw = core::slice::from_raw_parts(normw_p as *const f32, d);
                    for t in 0..n {
                        let base = (up_p as *const f32).add(t * qkv_dim);
                        let q = core::slice::from_raw_parts(base.add(kh * d), d);
                        let k = core::slice::from_raw_parts(base.add(qk_dim + kh * d), d);
                        let v = core::slice::from_raw_parts(base.add(2 * qk_dim + vh * d), d);
                        let o = core::slice::from_raw_parts_mut((out_p as *mut f32).add(t * v_dim + vh * d), d);
                        let g = core::slice::from_raw_parts((z_p as *const f32).add(t * v_dim + vh * d), d);
                        gdn_step(st, q, k, v, *(decay_p as *const f32).add(t * n_v + vh), *(beta_p as *const f32).add(t * n_v + vh), o);
                        rmsnorm(o, nw, eps);
                        for i in 0..d {
                            o[i] *= silu(g[i]);
                        }
                    }
                }
            }
        });
    }
    let _ = s;
    perfil_marca(&mut tp, FASE_ROPE_KV);
    let r = mv(source, &format!("{prefix}.ssm_out"), layer, h, v_dim, &attn_out, n, salida, par, bs, gpu, use_gpu);
    perfil_marca(&mut tp, FASE_O);
    r
}

#[allow(clippy::too_many_arguments)]
fn atencion_bloque<S: TensorSource>(
    exec: &LayerExecutor<'_>,
    layer: u32,
    pos0: usize,
    n: usize,
    xs: &[f32],
    salida: &mut [f32],
    bs: &mut BlockScratch,
    s: &mut LayerScratch,
    kv: &mut LayerKv,
    source: &mut S,
    par: &dyn RowParallel,
    prefix: &str,
    gpu: &mut Option<&mut dyn crate::gpu::GpuDispatch>,
    use_gpu: bool,
) -> Result<(), ()> {
    let m = exec.manifest;
    let spec = m.layer(layer).cloned().ok_or(())?;
    let h = m.hidden_dim as usize;
    let heads = m.effective_num_heads(layer) as usize;
    let kv_heads = m.effective_num_kv_heads(layer) as usize;
    let hd = m.effective_head_dim(layer) as usize;
    let (kv_dim, q_dim) = (kv_heads * hd, heads * hd);
    let q_full = 2 * q_dim;
    let group = heads / kv_heads;
    let eps = m.rms_eps;
    let theta = m.rope_theta;
    let rope_n = if spec.qk_rope_head_dim > 0 { spec.qk_rope_head_dim as usize } else { hd };

    let mut tp = perfil_ahora();
    let mut qf = vec![0.0f32; n * q_full];
    let mut k = vec![0.0f32; n * kv_dim];
    let mut v = vec![0.0f32; n * kv_dim];
    mv(source, &format!("{prefix}.attn_q"), layer, q_full, h, xs, n, &mut qf, par, bs, gpu, use_gpu)?;
    mv(source, &format!("{prefix}.attn_k"), layer, kv_dim, h, xs, n, &mut k, par, bs, gpu, use_gpu)?;
    mv(source, &format!("{prefix}.attn_v"), layer, kv_dim, h, xs, n, &mut v, par, bs, gpu, use_gpu)?;

    perfil_marca(&mut tp, FASE_QKV);
    // q y puerta están entrelazados por cabeza: [q_h | gate_h].
    let mut q = vec![0.0f32; n * q_dim];
    let mut puerta = vec![0.0f32; n * q_dim];
    for i in 0..n {
        for head in 0..heads {
            let src = i * q_full + head * 2 * hd;
            q[i * q_dim + head * hd..i * q_dim + (head + 1) * hd].copy_from_slice(&qf[src..src + hd]);
            puerta[i * q_dim + head * hd..i * q_dim + (head + 1) * hd].copy_from_slice(&qf[src + hd..src + 2 * hd]);
        }
    }
    drop(qf);
    source.load_f32(&format!("{prefix}.attn_q_norm"), &mut s.norm_w[..hd])?;
    for i in 0..n {
        for head in 0..heads {
            let o = i * q_dim + head * hd;
            rmsnorm(&mut q[o..o + hd], &s.norm_w[..hd], eps);
            rope_inplace_n(&mut q[o..o + hd], pos0 + i, theta, rope_n);
        }
    }
    source.load_f32(&format!("{prefix}.attn_k_norm"), &mut s.norm_w[..hd])?;
    let seq_antes = kv.tokens(kv_dim);
    for i in 0..n {
        for head in 0..kv_heads {
            let o = i * kv_dim + head * hd;
            rmsnorm(&mut k[o..o + hd], &s.norm_w[..hd], eps);
            rope_inplace_n(&mut k[o..o + hd], pos0 + i, theta, rope_n);
        }
        kv.append_f16(&k[i * kv_dim..(i + 1) * kv_dim], &v[i * kv_dim..(i + 1) * kv_dim]);
    }
    perfil_marca(&mut tp, FASE_ROPE_KV);
    let mut attn = vec![0.0f32; n * q_dim];
    // Con GPU, la atención del bloque va al dispositivo (el coste es cuadrático con el contexto y en CPU dominaba el prefill largo); el KV
    // del host sigue siendo la fuente de verdad y el dispositivo sube lo que le falte.
    let en_gpu = use_gpu
        && kv.dropped == 0
        && gpu.as_deref_mut().is_some_and(|g| {
            let mut filas = kv.gpu_kv_filas;
            let ok = matches!(g.atencion_prefill(layer, &q, &mut attn, n, seq_antes, heads, kv_heads, hd, kv.k_f16_slice(), kv.v_f16_slice(), &mut filas), Ok(true));
            kv.gpu_kv_filas = filas;
            ok
        });
    if !en_gpu {
        atencion_f16_grupos(par, &q, &mut attn, n, q_dim, kv, kv_heads, group, hd, kv_dim, seq_antes + 1);
    }
    for i in 0..n * q_dim {
        attn[i] *= sigmoid(puerta[i]);
    }
    perfil_marca(&mut tp, FASE_ATENCION);
    let r = mv(source, &format!("{prefix}.attn_output"), layer, h, q_dim, &attn, n, salida, par, bs, gpu, use_gpu);
    perfil_marca(&mut tp, FASE_O);
    r
}

#[allow(clippy::too_many_arguments)]
fn ffn_denso_bloque<S: TensorSource>(
    exec: &LayerExecutor<'_>,
    layer: u32,
    n: usize,
    xs: &[f32],
    salida: &mut [f32],
    bs: &mut BlockScratch,
    par: &dyn RowParallel,
    source: &mut S,
    prefix: &str,
    gpu: &mut Option<&mut dyn crate::gpu::GpuDispatch>,
    use_gpu: bool,
) -> Result<(), ()> {
    let m = exec.manifest;
    let h = m.hidden_dim as usize;
    let ffn = m.layer_expert_ffn_dim(layer) as usize;
    let mut up = vec![0.0f32; n * ffn];
    let mut gate = vec![0.0f32; n * ffn];
    mv(source, &format!("{prefix}.ffn_up"), layer, ffn, h, xs, n, &mut up, par, bs, gpu, use_gpu)?;
    mv(source, &format!("{prefix}.ffn_gate"), layer, ffn, h, xs, n, &mut gate, par, bs, gpu, use_gpu)?;
    for i in 0..n {
        swiglu_inplace(&mut up[i * ffn..(i + 1) * ffn], &gate[i * ffn..(i + 1) * ffn]);
    }
    mv(source, &format!("{prefix}.ffn_down"), layer, h, ffn, &up, n, salida, par, bs, gpu, use_gpu)
}

#[allow(clippy::too_many_arguments)]
fn moe_bloque<S: TensorSource>(
    exec: &LayerExecutor<'_>,
    layer: u32,
    n: usize,
    xs: &[f32],
    salida: &mut [f32],
    bs: &mut BlockScratch,
    par: &dyn RowParallel,
    source: &mut S,
    prefix: &str,
    gpu: &mut Option<&mut dyn crate::gpu::GpuDispatch>,
    use_gpu: bool,
) -> Result<(), ()> {
    let m = exec.manifest;
    let h = m.hidden_dim as usize;
    let ffn = m.layer_expert_ffn_dim(layer) as usize;
    let n_exp = m.effective_num_experts(layer) as usize;
    let top_k = m.effective_num_experts_per_tok(layer) as usize;
    let n_shared = m.layer(layer).map_or(0, |sp| sp.num_shared_experts) as usize;
    if n_exp == 0 || top_k == 0 {
        return Err(());
    }
    let mut tp = perfil_ahora();
    // Router por lotes y top-k por token.
    let mut logits = vec![0.0f32; n * n_exp];
    mv(source, &format!("{prefix}.ffn_gate_inp"), layer, n_exp, h, xs, n, &mut logits, par, bs, gpu, use_gpu)?;
    let mut ranking: Vec<Vec<(usize, f32)>> = Vec::with_capacity(n);
    let mut por_experto: Vec<Vec<(usize, usize)>> = vec![Vec::new(); n_exp]; // (token, posición en su ranking)
    for t in 0..n {
        let r = topk_softmax(&mut logits[t * n_exp..(t + 1) * n_exp], top_k);
        for (j, (e, _)) in r.iter().enumerate() {
            por_experto[*e].push((t, j));
        }
        ranking.push(r);
    }
    perfil_marca(&mut tp, FASE_QKV);
    // Salida de cada (token, posición del ranking).
    let mut y = vec![0.0f32; n * top_k * h];
    // Los expertos activos se reparten entre hilos **de uno en uno**: cada experto lee sus pesos una vez para todos sus tokens y no hay
    // barrera por matvec (con matrices de 512×2048 la barrera costaba más que el cálculo). Las vistas se piden antes, en serie.
    struct Activo {
        lista: *const Vec<(usize, usize)>,
        v: [(usize, usize, u8, usize); 3],
        claves: [String; 3],
    }
    unsafe impl Sync for Activo {}
    let mut activos: Vec<Activo> = Vec::new();
    for (e, lista) in por_experto.iter().enumerate() {
        if lista.is_empty() {
            continue;
        }
        let mut v = [(0usize, 0usize, 0u8, 0usize); 3];
        let mut claves: [String; 3] = Default::default();
        for (i, suf) in ["ffn_gate", "ffn_up", "ffn_down"].iter().enumerate() {
            let name = format!("{prefix}.E{e:02}.{suf}");
            claves[i] = name.clone();
            crate::layer::note_infer_op(Some(layer), &name);
            let vista = source.tensor_view(&name)?;
            v[i] = (vista.bytes.as_ptr() as usize, vista.bytes.len(), vista.dtype, vista.elems);
        }
        activos.push(Activo { lista: lista as *const _, v, claves });
    }
    perfil_marca(&mut tp, crate::layer::FASE_SWIGLU);
    // Con GPU: los expertos con pesos en la tarjeta se resuelven allí de una vez (una subida de los vectores, un lanzamiento por experto y
    // una sola espera); los demás siguen por la CPU a la vez que se reparten abajo.
    let mut hecho_gpu = vec![false; activos.len()];
    let mut suma_gpu = vec![0.0f32; if use_gpu { n * h } else { 0 }];
    let mut hecha = vec![false; n * top_k]; // asignación (token, posición) ya sumada por la GPU
    if use_gpu {
        if let Some(g) = gpu.as_deref_mut().filter(|g| g.available()) {
            let vistas: Vec<[crate::layer::TensorView<'_>; 3]> = activos
                .iter()
                .map(|a| {
                    core::array::from_fn(|i| crate::layer::TensorView {
                        bytes: unsafe { core::slice::from_raw_parts(a.v[i].0 as *const u8, a.v[i].1) },
                        dtype: a.v[i].2,
                        elems: a.v[i].3,
                    })
                })
                .collect();
            let tokens: Vec<Vec<u32>> = activos.iter().map(|a| unsafe { &*a.lista }.iter().map(|(t, _)| *t as u32).collect()).collect();
            let destinos: Vec<Vec<u32>> = activos.iter().map(|a| unsafe { &*a.lista }.iter().map(|(t, j)| (t * top_k + j) as u32).collect()).collect();
            let mut pesos = vec![0.0f32; n * top_k];
            for (t, r) in ranking.iter().enumerate() {
                for (j, (_, w)) in r.iter().enumerate() {
                    pesos[t * top_k + j] = *w;
                }
            }
            {
                let mut exps: Vec<crate::gpu::ExpertoBloque<'_>> = activos
                    .iter()
                    .zip(vistas.iter())
                    .zip(tokens.iter())
                    .zip(destinos.iter())
                    .map(|(((a, v), t), d)| crate::gpu::ExpertoBloque {
                        claves: [a.claves[0].as_str(), a.claves[1].as_str(), a.claves[2].as_str()],
                        vistas: [&v[0], &v[1], &v[2]],
                        tokens: t.as_slice(),
                        destinos: d.as_slice(),
                    })
                    .collect();
                let _ = g.expertos_bloque(xs, n, h, ffn, top_k, &pesos, &mut exps, &mut suma_gpu, &mut hecho_gpu);
            }
            for (k, d) in destinos.iter().enumerate() {
                if hecho_gpu[k] {
                    for &x in d {
                        hecha[x as usize] = true;
                    }
                }
            }
        }
    }
    perfil_marca(&mut tp, crate::layer::FASE_FFN_DOWN);
    let (y_p, xs_p) = (y.as_mut_ptr() as usize, xs.as_ptr() as usize);
    let fallo = core::sync::atomic::AtomicBool::new(false);
    let seq = Sequential;
    par.for_rows(activos.len(), &|k0, k1| {
        for k in k0..k1 {
            if hecho_gpu[k] {
                continue;
            }
            let act = &activos[k];
            // SAFETY: las listas viven hasta el final de la función y cada (token, posición) pertenece a un solo experto.
            let lista = unsafe { &*act.lista };
            let cnt = lista.len();
            let xs_all = unsafe { core::slice::from_raw_parts(xs_p as *const f32, n * h) };
            let mut xg = Vec::with_capacity(cnt * h);
            for (t, _) in lista {
                xg.extend_from_slice(&xs_all[t * h..(t + 1) * h]);
            }
            let vista = |i: usize| crate::layer::TensorView {
                bytes: unsafe { core::slice::from_raw_parts(act.v[i].0 as *const u8, act.v[i].1) },
                dtype: act.v[i].2,
                elems: act.v[i].3,
            };
            let mut q8 = vec![0u8; crate::q8k::q8k_buffer_len(cnt, h.max(ffn))];
            let mut gate = vec![0.0f32; cnt * ffn];
            let mut up = vec![0.0f32; cnt * ffn];
            let mut down = vec![0.0f32; cnt * h];
            let ok = matvec_view_batch_par(&vista(0), ffn, h, &xg, cnt, &mut gate, &seq, Some(&mut q8)).is_ok()
                && matvec_view_batch_par(&vista(1), ffn, h, &xg, cnt, &mut up, &seq, Some(&mut q8)).is_ok()
                && {
                    for i in 0..cnt {
                        swiglu_inplace(&mut up[i * ffn..(i + 1) * ffn], &gate[i * ffn..(i + 1) * ffn]);
                    }
                    matvec_view_batch_par(&vista(2), h, ffn, &up, cnt, &mut down, &seq, Some(&mut q8)).is_ok()
                };
            if !ok {
                fallo.store(true, core::sync::atomic::Ordering::Relaxed);
                continue;
            }
            for (i, (t, j)) in lista.iter().enumerate() {
                let dst = unsafe { core::slice::from_raw_parts_mut((y_p as *mut f32).add((t * top_k + j) * h), h) };
                dst.copy_from_slice(&down[i * h..(i + 1) * h]);
            }
        }
    });
    if fallo.load(core::sync::atomic::Ordering::Relaxed) {
        return Err(());
    }
    perfil_marca(&mut tp, FASE_FFN_UP_GATE);
    // Experto compartido (puerta sigmoide por token si existe `ffn_gate_inp_shexp`).
    let mut compartido = vec![0.0f32; n_shared * n * h];
    let mut puerta = vec![1.0f32; n];
    if n_shared > 0 {
        let name_pg = format!("{prefix}.ffn_gate_inp_shexp");
        if source.tensor_view(&name_pg).is_ok() {
            let mut pg = vec![0.0f32; n];
            mv(source, &name_pg, layer, 1, h, xs, n, &mut pg, par, bs, gpu, use_gpu)?;
            for t in 0..n {
                puerta[t] = 1.0 / (1.0 + libm::expf(-pg[t]));
            }
        }
        for sh in 0..n_shared {
            let mut gate = vec![0.0f32; n * ffn];
            let mut up = vec![0.0f32; n * ffn];
            mv(source, &format!("{prefix}.S{sh:02}.ffn_gate"), layer, ffn, h, xs, n, &mut gate, par, bs, gpu, use_gpu)?;
            mv(source, &format!("{prefix}.S{sh:02}.ffn_up"), layer, ffn, h, xs, n, &mut up, par, bs, gpu, use_gpu)?;
            for i in 0..n {
                swiglu_inplace(&mut up[i * ffn..(i + 1) * ffn], &gate[i * ffn..(i + 1) * ffn]);
            }
            mv(source, &format!("{prefix}.S{sh:02}.ffn_down"), layer, h, ffn, &up, n, &mut compartido[sh * n * h..(sh + 1) * n * h], par, bs, gpu, use_gpu)?;
        }
    }
    perfil_marca(&mut tp, FASE_O);
    // Suma por token: lo que ya sumó la GPU, luego los expertos de la CPU en el orden del ranking y por último los compartidos.
    for t in 0..n {
        let o = &mut salida[t * h..(t + 1) * h];
        if use_gpu && !suma_gpu.is_empty() {
            o.copy_from_slice(&suma_gpu[t * h..(t + 1) * h]);
        } else {
            o.fill(0.0);
        }
        for (j, (_, w)) in ranking[t].iter().enumerate() {
            if hecha[t * top_k + j] {
                continue;
            }
            let yj = &y[(t * top_k + j) * h..(t * top_k + j + 1) * h];
            for i in 0..h {
                o[i] += yj[i] * w;
            }
        }
        for sh in 0..n_shared {
            let c = &compartido[(sh * n + t) * h..(sh * n + t + 1) * h];
            for i in 0..h {
                o[i] += c[i] * puerta[t];
            }
        }
    }
    Ok(())
}
