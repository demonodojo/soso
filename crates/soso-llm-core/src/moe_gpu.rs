//! Capa MoE (Mellum / Qwen3-MoE) entera en la GPU: el lado del núcleo.
//!
//! Reúne lo que el despacho necesita (claves de tensores, normas, router, parámetros del RoPE), sube los
//! expertos la primera vez y lanza [`GpuDispatch::moe_capa`]. Si algo no encaja (dtype, VRAM, bias, forma),
//! devuelve `Ok(false)` **sin haber tocado nada** y la capa sigue por CPU.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use sosomodel::layout::{DTYPE_Q4_K, DTYPE_Q8_0};
use sosomodel::manifest::{AttnKind, FfnKind, FLAG_QK_NORM, FLAG_ROPE_NEOX};

use crate::gpu::{MoeCapa, MoeNormas};
use crate::kv::{KvDtype, LayerKv};
use crate::layer::{LayerExecutor, TensorSource};

/// Intenta ejecutar la capa MoE `layer` entera en la GPU para `n` tokens (`hidden` son `n` vectores). `Ok(true)` si `hidden` ya es el de salida y el KV de
/// la capa está en el dispositivo; `Ok(false)` si hay que hacerla en CPU.
#[allow(clippy::too_many_arguments)]
pub fn capa_moe_en_gpu<S: TensorSource>(
    exec: &LayerExecutor<'_>,
    layer: u32,
    pos0: usize,
    n: usize,
    hidden: &mut [f32],
    kv: &mut LayerKv,
    source: &mut S,
    gpu: &mut Option<&mut dyn crate::gpu::GpuDispatch>,
    use_gpu: bool,
) -> Result<bool, ()> {
    if !use_gpu || kv.dtype != KvDtype::F16 {
        return Ok(false);
    }
    let Some(g) = gpu.as_deref_mut() else { return Ok(false) };
    if !g.available() || !g.moe_puede(layer) {
        return Ok(false);
    }
    let m = exec.manifest;
    let spec = m.layer(layer).cloned().unwrap_or_default();
    if spec.attn_kind != AttnKind::Gqa
        || spec.ffn_kind == FfnKind::LatentMoe
        || spec.num_shared_experts != 0
        || spec.flags & FLAG_ROPE_NEOX == 0
        || crate::gemma::es_capa_gemma(spec.flags)
        || !m.layer_is_moe(layer)
    {
        return Ok(false);
    }
    let h = m.hidden_dim as usize;
    let heads = m.effective_num_heads(layer) as usize;
    let kv_heads = m.effective_num_kv_heads(layer) as usize;
    let hd = m.effective_head_dim(layer) as usize;
    let ffn = m.layer_expert_ffn_dim(layer) as usize;
    let n_exp = m.effective_num_experts(layer) as usize;
    let top_k = m.effective_num_experts_per_tok(layer) as usize;
    let ventana = m.layer_window(layer) as usize;
    let p = format!("L{layer:02}");
    let (nq, nk, nv, no) = (format!("{p}.attn_q"), format!("{p}.attn_k"), format!("{p}.attn_v"), format!("{p}.attn_output"));
    let lista = g.moe_lista(layer);
    if !lista {
        // Primera vez: se comprueban las formas y se sube todo (si algo no cabe, la capa se queda en CPU).
        if n_exp == 0 || top_k == 0 || top_k > n_exp || n_exp > 256 || h > 4096 || h % 256 != 0 || ffn % 32 != 0 {
            return Ok(false);
        }
        for suf in [".bias"] {
            for b in [&nq, &nk, &nv] {
                if source.tensor_view(&format!("{b}{suf}")).is_ok() {
                    return Ok(false);
                }
            }
        }
        let attn: [(&str, usize, usize); 4] = [(&nq, heads * hd, h), (&nk, kv_heads * hd, h), (&nv, kv_heads * hd, h), (&no, h, heads * hd)];
        for (clave, rows, cols) in attn {
            if !g.gemma_residente(clave) {
                let vista = source.tensor_view(clave)?;
                if !g.gemma_subir(clave, &vista, rows, cols).unwrap_or(false) {
                    return Ok(false);
                }
            }
        }
        for e in 0..n_exp {
            for (suf, rows, cols, dtype) in [("ffn_gate", ffn, h, DTYPE_Q4_K), ("ffn_up", ffn, h, DTYPE_Q4_K), ("ffn_down", h, ffn, DTYPE_Q8_0)] {
                let clave = format!("{p}.E{e:02}.{suf}");
                if !g.gemma_residente(&clave) {
                    let vista = source.tensor_view(&clave)?;
                    if vista.dtype != dtype || !g.gemma_subir(&clave, &vista, rows, cols).unwrap_or(false) {
                        return Ok(false);
                    }
                }
            }
        }
    }
    let cargar = |source: &mut S, nombre: String, n: usize| -> Result<Vec<f32>, ()> {
        let mut v = alloc::vec![0.0f32; n];
        source.load_f32(&nombre, &mut v)?;
        Ok(v)
    };
    let theta = m.layer_rope_theta(layer);
    // YaRN sólo en las capas de atención completa (las de ventana llevan RoPE plano), como en la CPU.
    let (factores, mscale) = if m.ext.yarn.activa() && ventana == 0 {
        let (f, ms) = crate::gemm::yarn_factores(hd, theta, m.ext.yarn);
        (Some(f), ms)
    } else {
        (None, 1.0)
    };
    let qk = spec.flags & FLAG_QK_NORM != 0;
    let datos = if lista {
        None
    } else {
        let attn_norm = cargar(source, format!("{p}.attn_norm"), h)?;
        let ffn_norm = cargar(source, format!("{p}.ffn_norm"), h)?;
        let q_norm = if qk { cargar(source, format!("{p}.attn_q_norm"), hd)? } else { Vec::new() };
        let k_norm = if qk { cargar(source, format!("{p}.attn_k_norm"), hd)? } else { Vec::new() };
        let router = cargar(source, format!("{p}.ffn_gate_inp"), n_exp * h)?;
        Some((attn_norm, ffn_norm, q_norm, k_norm, router))
    };
    // Encadenado: si la capa anterior dejó su salida en el dispositivo se continúa desde ella, y si la siguiente
    // también va en GPU no se descarga ni se sincroniza (el `hidden` del host queda desfasado hasta la última capa).
    let continuar = layer > 0 && g.moe_pendiente(layer - 1, pos0, n);
    let encadenar = lista
        && layer + 1 < m.num_layers
        && exec.es_capa_moe_gpu(layer + 1)
        && g.moe_puede(layer + 1)
        && g.moe_lista(layer + 1);
    let normas = datos.as_ref().map(|(a, f, q, k, r)| MoeNormas {
        attn_norm: a,
        q_norm: q,
        k_norm: k,
        ffn_norm: f,
        router: r,
        rope_factores: factores.as_deref(),
    });
    let mut capa = MoeCapa {
        layer,
        pos0,
        n,
        hidden,
        h,
        heads,
        kv_heads,
        head_dim: hd,
        ffn,
        n_exp,
        top_k,
        ventana,
        ctx_max: (m.max_seq as usize).min(32768),
        eps: m.rms_eps,
        theta,
        escala: 1.0 / libm::sqrtf(hd as f32),
        qk_norm: qk,
        rope_mscale: mscale,
        continuar,
        encadenar,
        clave_q: &nq,
        clave_k: &nk,
        clave_v: &nv,
        clave_o: &no,
        prefijo: &p,
        normas,
        expulsadas: 0,
    };
    let r = g.moe_capa(&mut capa);
    let expulsadas = capa.expulsadas;
    match r {
        Ok(true) => {}
        other => {
            // Si la GPU no pudo, el `hidden` de la capa anterior sigue pendiente en el dispositivo: se baja antes de
            // que la CPU la haga (o de abortar).
            if continuar {
                g.moe_volcar(hidden).map_err(|_| ())?;
            }
            return match other {
                Ok(_) => Ok(false),
                Err(()) if kv.en_gpu => Err(()),
                Err(()) => Ok(false),
            };
        }
    }
    kv.en_gpu = true;
    kv.gpu_pos = pos0 + n;
    kv.dropped = expulsadas;
    Ok(true)
}
