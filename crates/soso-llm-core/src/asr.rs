//! Runtime ASR encoder-decoder (Whisper-style).

use core::cell::Cell;

use crate::gemm::{dot_f32, gelu_inplace, layernorm, matmul_f32, matmul_xwt_f32, softmax_inplace};
use crate::gpu::{try_gpu_matmul, try_gpu_matvec, GpuDispatch};
use crate::sched::{Dest, OpDesc, OpSched};
use crate::layer::{matvec_view, TensorView, TensorSource};
use crate::tokenizer::Tokenizer;
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use sosomodel::index::TensorIndex;
use sosomodel::layout::DTYPE_F32;
use sosomodel::manifest::{Manifest, ModelKind};

// ── Telemetría (TSC / fase por token) ───────────────────────────────────────

#[inline]
fn ticks() -> u64 {
    #[cfg(target_arch = "x86_64")]
    unsafe {
        let hi: u32;
        let lo: u32;
        core::arch::asm!("rdtsc", out("eax") lo, out("edx") hi, options(nomem, nostack));
        ((hi as u64) << 32) | lo as u64
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AsrPhase {
    Encode,
    CrossKv,
    SelfAttn,
    CrossAttn,
    Mlp,
    Logits,
    CpuAttn,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct AsrTokenProfile {
    pub seq: usize,
    pub self_attn: u64,
    pub cross_attn: u64,
    pub mlp: u64,
    pub logits: u64,
    pub cpu_attn: u64,
    pub matvec_calls: u32,
    pub gpu_matvec_calls: u32,
}

impl AsrTokenProfile {
    pub fn total_cycles(&self) -> u64 {
        self.self_attn + self.cross_attn + self.mlp + self.logits + self.cpu_attn
    }
}

pub struct AsrProfile {
    pub enabled: bool,
    pub encode_cycles: Cell<u64>,
    pub encode_gpu_ns: Cell<u64>,
    pub encode_cpu_ns: Cell<u64>,
    pub cross_kv_gpu_ns: Cell<u64>,
    pub cross_kv_cpu_ns: Cell<u64>,
    pub tokens: Vec<AsrTokenProfile>,
    phase: Cell<AsrPhase>,
    phase_start: Cell<u64>,
    current: Cell<AsrTokenProfile>,
}

impl Default for AsrProfile {
    fn default() -> Self {
        Self {
            enabled: false,
            encode_cycles: Cell::new(0),
            encode_gpu_ns: Cell::new(0),
            encode_cpu_ns: Cell::new(0),
            cross_kv_gpu_ns: Cell::new(0),
            cross_kv_cpu_ns: Cell::new(0),
            tokens: Vec::new(),
            phase: Cell::new(AsrPhase::Encode),
            phase_start: Cell::new(0),
            current: Cell::new(AsrTokenProfile::default()),
        }
    }
}

impl AsrProfile {
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            ..Default::default()
        }
    }

    fn begin_token(&self, seq: usize) {
        if !self.enabled {
            return;
        }
        self.current.set(AsrTokenProfile { seq, ..Default::default() });
        self.phase.set(AsrPhase::SelfAttn);
        self.phase_start.set(ticks());
    }

    fn set_phase(&self, phase: AsrPhase) {
        if !self.enabled {
            return;
        }
        self.flush_phase();
        self.phase.set(phase);
        self.phase_start.set(ticks());
    }

    fn flush_phase(&self) {
        if !self.enabled {
            return;
        }
        let elapsed = ticks().wrapping_sub(self.phase_start.get());
        let mut cur = self.current.get();
        match self.phase.get() {
            AsrPhase::Encode => self.encode_cycles.set(self.encode_cycles.get().wrapping_add(elapsed)),
            AsrPhase::CrossKv => {}
            AsrPhase::SelfAttn => cur.self_attn = cur.self_attn.wrapping_add(elapsed),
            AsrPhase::CrossAttn => cur.cross_attn = cur.cross_attn.wrapping_add(elapsed),
            AsrPhase::Mlp => cur.mlp = cur.mlp.wrapping_add(elapsed),
            AsrPhase::Logits => cur.logits = cur.logits.wrapping_add(elapsed),
            AsrPhase::CpuAttn => cur.cpu_attn = cur.cpu_attn.wrapping_add(elapsed),
        }
        self.current.set(cur);
    }

    fn record_matvec(&self, gpu: bool) {
        if !self.enabled {
            return;
        }
        let mut cur = self.current.get();
        cur.matvec_calls += 1;
        if gpu {
            cur.gpu_matvec_calls += 1;
        }
        self.current.set(cur);
    }

    pub fn record_op(&self, phase: AsrPhase, gpu: bool, ns: u64) {
        if !self.enabled {
            return;
        }
        match phase {
            AsrPhase::Encode => {
                if gpu {
                    self.encode_gpu_ns.set(self.encode_gpu_ns.get().wrapping_add(ns));
                } else {
                    self.encode_cpu_ns.set(self.encode_cpu_ns.get().wrapping_add(ns));
                }
            }
            AsrPhase::CrossKv => {
                if gpu {
                    self.cross_kv_gpu_ns.set(self.cross_kv_gpu_ns.get().wrapping_add(ns));
                } else {
                    self.cross_kv_cpu_ns.set(self.cross_kv_cpu_ns.get().wrapping_add(ns));
                }
            }
            _ => {}
        }
    }

    pub fn format_phase_summary(&self) -> String {
        let mut matvec_calls = 0u32;
        let mut gpu_matvec_calls = 0u32;
        for t in &self.tokens {
            matvec_calls += t.matvec_calls;
            gpu_matvec_calls += t.gpu_matvec_calls;
        }
        format!(
            "asr encode gpu_ns={} cpu_ns={} cross_kv gpu_ns={} cpu_ns={} matvec={}/{}\n",
            self.encode_gpu_ns.get(),
            self.encode_cpu_ns.get(),
            self.cross_kv_gpu_ns.get(),
            self.cross_kv_cpu_ns.get(),
            gpu_matvec_calls,
            matvec_calls,
        )
    }

    /// Resumen compacto por token (una línea cada uno).
    pub fn format_token_summaries(&self) -> String {
        let mut out = String::new();
        for t in &self.tokens {
            out.push_str(&format!(
                "asr tok seq={} matvec={}/{} ciclos self={} cross={} mlp={} logits={} cpu_attn={} total={}\n",
                t.seq,
                t.gpu_matvec_calls,
                t.matvec_calls,
                t.self_attn,
                t.cross_attn,
                t.mlp,
                t.logits,
                t.cpu_attn,
                t.total_cycles(),
            ));
        }
        out
    }
}

/// Puntero al perfil activo; válido mientras no se mueva/reemplace `profile`.
fn prof_ptr<S: TensorSource>(rt: &AsrRuntime<S>) -> Option<*const AsrProfile> {
    rt.profile.as_ref().filter(|p| p.enabled).map(|p| p as *const AsrProfile)
}

fn prof_record(p: Option<*const AsrProfile>, gpu: bool) {
    if let Some(p) = p {
        // SAFETY: ver `prof_ptr`; solo mutación interior vía `Cell`.
        unsafe {
            (*p).record_matvec(gpu);
        }
    }
}

fn prof_record_op(p: Option<*const AsrProfile>, phase: AsrPhase, gpu: bool, ns: u64) {
    if let Some(p) = p {
        unsafe {
            (*p).record_op(phase, gpu, ns);
        }
    }
}

struct ExecCtx<'s> {
    gpu_allowed: bool,
    sched: &'s mut OpSched,
    profile: Option<*const AsrProfile>,
    phase: AsrPhase,
}

fn exec_ctx<'s>(
    gpu_allowed: bool,
    sched: &'s mut OpSched,
    profile: Option<*const AsrProfile>,
    phase: AsrPhase,
) -> ExecCtx<'s> {
    ExecCtx {
        gpu_allowed,
        sched,
        profile,
        phase,
    }
}

fn dispatch_matvec_view(
    ctx: &mut ExecCtx<'_>,
    gpu: &mut Option<&mut dyn GpuDispatch>,
    key: &str,
    v: &TensorView<'_>,
    rows: usize,
    cols: usize,
    x: &[f32],
    out: &mut [f32],
) -> Result<(), ()> {
    let op = OpDesc::matvec(rows, cols, true, DTYPE_F32);
    let gpu_avail = ctx.gpu_allowed && gpu.is_some();
    let dest = ctx.sched.elegir(&op, gpu_avail);
    let t0 = ticks();
    let gpu_used = match dest {
        Dest::Gpu => {
            if let Some(g) = gpu.as_deref_mut() {
                if try_gpu_matvec(g, key, v, rows, cols, x, out)? {
                    true
                } else {
                    matvec_view(v, rows, cols, x, out)?;
                    false
                }
            } else {
                matvec_view(v, rows, cols, x, out)?;
                false
            }
        }
        Dest::Cpu => {
            matvec_view(v, rows, cols, x, out)?;
            false
        }
    };
    let elapsed = ticks().wrapping_sub(t0);
    let actual_dest = if gpu_used { Dest::Gpu } else { Dest::Cpu };
    ctx.sched.registrar(&op, actual_dest, elapsed);
    prof_record(ctx.profile, gpu_used);
    prof_record_op(ctx.profile, ctx.phase, gpu_used, elapsed);
    Ok(())
}

fn dispatch_matvec<S: TensorSource>(
    ctx: &mut ExecCtx<'_>,
    gpu: &mut Option<&mut dyn GpuDispatch>,
    source: &mut S,
    key: &str,
    rows: usize,
    cols: usize,
    x: &[f32],
    out: &mut [f32],
) -> Result<(), ()> {
    let v = source.tensor_view(key)?;
    dispatch_matvec_view(ctx, gpu, key, &v, rows, cols, x, out)
}

fn dispatch_matmul_view(
    ctx: &mut ExecCtx<'_>,
    gpu: &mut Option<&mut dyn GpuDispatch>,
    key: &str,
    v: &TensorView<'_>,
    rows: usize,
    cols: usize,
    n: usize,
    x: &[f32],
    out: &mut [f32],
) -> Result<(), ()> {
    let op = OpDesc::matmul(rows, cols, n, true, DTYPE_F32);
    let gpu_avail = ctx.gpu_allowed && gpu.is_some();
    let dest = ctx.sched.elegir(&op, gpu_avail);
    let t0 = ticks();
    let gpu_used = match dest {
        Dest::Gpu => {
            if let Some(g) = gpu.as_deref_mut() {
                if try_gpu_matmul(g, key, v, rows, cols, n, x, out)? {
                    true
                } else {
                    let w = v.f32().ok_or(())?;
                    matmul_xwt_f32(w, x, rows, cols, n, out);
                    false
                }
            } else {
                let w = v.f32().ok_or(())?;
                matmul_xwt_f32(w, x, rows, cols, n, out);
                false
            }
        }
        Dest::Cpu => {
            let w = v.f32().ok_or(())?;
            matmul_xwt_f32(w, x, rows, cols, n, out);
            false
        }
    };
    let elapsed = ticks().wrapping_sub(t0);
    let actual_dest = if gpu_used { Dest::Gpu } else { Dest::Cpu };
    ctx.sched.registrar(&op, actual_dest, elapsed);
    prof_record(ctx.profile, gpu_used);
    prof_record_op(ctx.profile, ctx.phase, gpu_used, elapsed);
    Ok(())
}

fn dispatch_matmul<S: TensorSource>(
    ctx: &mut ExecCtx<'_>,
    gpu: &mut Option<&mut dyn GpuDispatch>,
    source: &mut S,
    key: &str,
    rows: usize,
    cols: usize,
    n: usize,
    x: &[f32],
    out: &mut [f32],
) -> Result<(), ()> {
    let v = source.tensor_view(key)?;
    dispatch_matmul_view(ctx, gpu, key, &v, rows, cols, n, x, out)
}

fn prof_set_phase(p: Option<*const AsrProfile>, phase: AsrPhase) {
    if let Some(p) = p {
        unsafe {
            (*p).set_phase(phase);
        }
    }
}

fn prof_begin_token(p: Option<*const AsrProfile>, seq: usize) {
    if let Some(p) = p {
        unsafe {
            (*p).begin_token(seq);
        }
    }
}

struct LayerCrossKv {
    k: Vec<f32>,
    v: Vec<f32>,
}

struct LayerSelfKv {
    k: Vec<f32>,
    v: Vec<f32>,
    len: usize,
}

impl LayerSelfKv {
    fn new(max_ctx: usize, d: usize) -> Self {
        Self {
            k: vec![0.0; max_ctx * d],
            v: vec![0.0; max_ctx * d],
            len: 0,
        }
    }

    fn append(&mut self, k_row: &[f32], v_row: &[f32], d: usize) {
        let pos = self.len;
        self.k[pos * d..(pos + 1) * d].copy_from_slice(k_row);
        self.v[pos * d..(pos + 1) * d].copy_from_slice(v_row);
        self.len += 1;
    }
}

pub struct DecoderCache {
    cross: Vec<LayerCrossKv>,
    self_kv: Vec<LayerSelfKv>,
    enc_seq: usize,
    d: usize,
}

impl DecoderCache {
    fn new(n_layers: usize, max_ctx: usize, d: usize) -> Self {
        Self {
            cross: (0..n_layers)
                .map(|_| LayerCrossKv { k: Vec::new(), v: Vec::new() })
                .collect(),
            self_kv: (0..n_layers)
                .map(|_| LayerSelfKv::new(max_ctx, d))
                .collect(),
            enc_seq: 0,
            d,
        }
    }
}

// ── Runtime ─────────────────────────────────────────────────────────────────

pub struct AsrRuntime<S: TensorSource> {
    pub manifest: Manifest,
    pub index: TensorIndex,
    pub source: S,
    pub tokenizer: Tokenizer,
    pub profile: Option<AsrProfile>,
    pub sched: OpSched,
    scratch_q: Vec<f32>,
    scratch_k: Vec<f32>,
    scratch_v: Vec<f32>,
    scratch_scores: Vec<f32>,
    scratch_tmp: Vec<f32>,
    scratch_mlp: Vec<f32>,
    scratch_logits: Vec<f32>,
    scratch_warmup_x: Vec<f32>,
    scratch_warmup_y: Vec<f32>,
    scratch_qkv: Vec<f32>,
    scratch_head: Vec<f32>,
    scratch_kt: Vec<f32>,
    scratch_k_head: Vec<f32>,
    fused_staging: Vec<f32>,
    fused_weights: Vec<(String, Vec<f32>)>,
    token_hidden: Vec<f32>,
    mel_padded: Vec<f32>,
}

const WHISPER_EOT: u32 = 50257;
const WHISPER_SOT: u32 = 50258;
const WHISPER_LANG_LO: u32 = 50259;
const WHISPER_LANG_HI: u32 = 50357;
const WHISPER_TRANSLATE: u32 = 50358;
const WHISPER_TRANSCRIBE: u32 = 50359;
const WHISPER_SOLM: u32 = 50360;
const WHISPER_PREV: u32 = 50361;
const WHISPER_NOSPEECH: u32 = 50362;
const WHISPER_NOTIMESTAMPS: u32 = 50363;
const WHISPER_TS_BEG: u32 = 50364;
/// Token « » en vocabulario Whisper multilingüe.
const WHISPER_BLANK: u32 = 220;

fn suppress_logit(logits: &mut [f32], id: u32) {
    if (id as usize) < logits.len() {
        logits[id as usize] = f32::NEG_INFINITY;
    }
}

/// Filtros de logits de whisper.cpp (greedy, sin timestamps).
fn apply_whisper_logit_filters(logits: &mut [f32], n_generated: usize) {
    let is_initial = n_generated == 0;
    if is_initial {
        suppress_logit(logits, WHISPER_EOT);
        suppress_logit(logits, WHISPER_BLANK);
    }
    suppress_logit(logits, WHISPER_NOTIMESTAMPS);
    for id in WHISPER_TS_BEG..logits.len() as u32 {
        suppress_logit(logits, id);
    }
    suppress_logit(logits, WHISPER_SOT);
    suppress_logit(logits, WHISPER_NOSPEECH);
    suppress_logit(logits, WHISPER_SOLM);
    suppress_logit(logits, WHISPER_TRANSLATE);
    suppress_logit(logits, WHISPER_TRANSCRIBE);
    suppress_logit(logits, WHISPER_PREV);
    for id in WHISPER_LANG_LO..=WHISPER_LANG_HI {
        suppress_logit(logits, id);
    }
}

fn argmax_logit(logits: &[f32]) -> Option<u32> {
    logits
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(core::cmp::Ordering::Equal))
        .map(|(i, _)| i as u32)
}

fn add_bias<S: TensorSource>(
    source: &mut S,
    weight_key: &str,
    rows: usize,
    out: &mut [f32],
) -> Result<(), ()> {
    let bias_name = weight_key.replace(".weight", ".bias");
    if let Ok(bv) = source.tensor_view(&bias_name) {
        if let Some(b) = bv.f32() {
            for (o, &bi) in out.iter_mut().zip(b.iter().take(rows)) {
                *o += bi;
            }
        }
    }
    Ok(())
}

fn matvec_bias<S: TensorSource>(
    ctx: &mut ExecCtx<'_>,
    gpu: &mut Option<&mut dyn GpuDispatch>,
    source: &mut S,
    weight_key: &str,
    rows: usize,
    cols: usize,
    x: &[f32],
    out: &mut [f32],
) -> Result<(), ()> {
    dispatch_matvec(ctx, gpu, source, weight_key, rows, cols, x, out)?;
    add_bias(source, weight_key, rows, out)
}

fn f32_tensor_view(data: &[f32]) -> TensorView<'_> {
    TensorView {
        bytes: unsafe {
            core::slice::from_raw_parts(data.as_ptr().cast::<u8>(), data.len() * 4)
        },
        dtype: DTYPE_F32,
        elems: data.len(),
    }
}

fn add_fused_bias3<S: TensorSource>(
    source: &mut S,
    q_key: &str,
    k_key: &str,
    v_key: &str,
    d: usize,
    out: &mut [f32],
) -> Result<(), ()> {
    add_bias(source, q_key, d, &mut out[..d])?;
    add_bias(source, k_key, d, &mut out[d..2 * d])?;
    add_bias(source, v_key, d, &mut out[2 * d..3 * d])
}

fn add_fused_bias2<S: TensorSource>(
    source: &mut S,
    k_key: &str,
    v_key: &str,
    d: usize,
    out: &mut [f32],
) -> Result<(), ()> {
    add_bias(source, k_key, d, &mut out[..d])?;
    add_bias(source, v_key, d, &mut out[d..2 * d])
}

fn matvec_fused_qkv3<S: TensorSource>(
    ctx: &mut ExecCtx<'_>,
    gpu: &mut Option<&mut dyn GpuDispatch>,
    source: &mut S,
    gpu_key: &str,
    weight: &[f32],
    d: usize,
    q_key: &str,
    k_key: &str,
    v_key: &str,
    x: &[f32],
    scratch_qkv: &mut [f32],
    q_out: &mut [f32],
    k_out: &mut [f32],
    v_out: &mut [f32],
) -> Result<(), ()> {
    let rows = 3 * d;
    let v = f32_tensor_view(weight);
    dispatch_matvec_view(ctx, gpu, gpu_key, &v, rows, d, x, &mut scratch_qkv[..rows])?;
    add_fused_bias3(source, q_key, k_key, v_key, d, &mut scratch_qkv[..rows])?;
    q_out.copy_from_slice(&scratch_qkv[..d]);
    k_out.copy_from_slice(&scratch_qkv[d..2 * d]);
    v_out.copy_from_slice(&scratch_qkv[2 * d..3 * d]);
    Ok(())
}

fn matmul_fused_qkv3<S: TensorSource>(
    ctx: &mut ExecCtx<'_>,
    gpu: &mut Option<&mut dyn GpuDispatch>,
    source: &mut S,
    gpu_key: &str,
    weight: &[f32],
    d: usize,
    q_key: &str,
    k_key: &str,
    v_key: &str,
    x: &[f32],
    seq: usize,
    q_out: &mut [f32],
    k_out: &mut [f32],
    v_out: &mut [f32],
    scratch_qkv: &mut [f32],
) -> Result<(), ()> {
    let rows = 3 * d;
    let v = f32_tensor_view(weight);
    dispatch_matmul_view(ctx, gpu, gpu_key, &v, rows, d, seq, x, scratch_qkv)?;
    for t in 0..seq {
        let row = &mut scratch_qkv[t * rows..(t + 1) * rows];
        add_fused_bias3(source, q_key, k_key, v_key, d, row)?;
        q_out[t * d..(t + 1) * d].copy_from_slice(&row[..d]);
        k_out[t * d..(t + 1) * d].copy_from_slice(&row[d..2 * d]);
        v_out[t * d..(t + 1) * d].copy_from_slice(&row[2 * d..3 * d]);
    }
    Ok(())
}

/// Multi-cabeza batched: scores = softmax(QK^T/√d) V.
/// `q_buf` y `k_buf` deben ser independientes (Q no se sobrescribe con K).
fn attention_batched(
    q: &[f32],
    k: &[f32],
    v: &[f32],
    seq_q: usize,
    seq_kv: usize,
    d: usize,
    heads: usize,
    hd: usize,
    causal: bool,
    scores: &mut [f32],
    q_buf: &mut [f32],
    k_buf: &mut [f32],
    kt_buf: &mut [f32],
    head_out: &mut [f32],
    merged: &mut [f32],
    profile: Option<*const AsrProfile>,
) {
    let t0 = ticks();
    let scale = 1.0 / libm::sqrtf(hd as f32);
    merged.fill(0.0);
    for h in 0..heads {
        let off = h * hd;
        for t in 0..seq_q {
            q_buf[t * hd..(t + 1) * hd].copy_from_slice(&q[t * d + off..t * d + off + hd]);
        }
        for t in 0..seq_kv {
            k_buf[t * hd..(t + 1) * hd].copy_from_slice(&k[t * d + off..t * d + off + hd]);
        }
        for j in 0..seq_kv {
            for ki in 0..hd {
                kt_buf[ki * seq_kv + j] = k_buf[j * hd + ki];
            }
        }
        matmul_f32(q_buf, seq_q, hd, kt_buf, seq_kv, scores);
        for i in 0..seq_q {
            let row = &mut scores[i * seq_kv..(i + 1) * seq_kv];
            if causal {
                for j in 0..seq_kv {
                    if j > i {
                        row[j] = f32::NEG_INFINITY;
                    }
                }
            }
            for x in row.iter_mut() {
                *x *= scale;
            }
            softmax_inplace(row);
        }
        for t in 0..seq_kv {
            k_buf[t * hd..(t + 1) * hd].copy_from_slice(&v[t * d + off..t * d + off + hd]);
        }
        matmul_f32(scores, seq_q, seq_kv, k_buf, hd, head_out);
        for t in 0..seq_q {
            merged[t * d + off..t * d + off + hd]
                .copy_from_slice(&head_out[t * hd..(t + 1) * hd]);
        }
    }
    if let Some(p) = profile {
        unsafe {
            (*p).flush_phase();
            let elapsed = ticks().wrapping_sub(t0);
            let mut cur = (*p).current.get();
            cur.cpu_attn = cur.cpu_attn.wrapping_add(elapsed);
            (*p).current.set(cur);
            (*p).phase_start.set(ticks());
        }
    }
}

/// Atención causal de un solo query sobre K/V cacheados (decode incremental).
fn attention_single(
    q: &[f32],
    k: &[f32],
    v: &[f32],
    seq: usize,
    d: usize,
    heads: usize,
    hd: usize,
    scores: &mut [f32],
    out: &mut [f32],
    profile: Option<*const AsrProfile>,
) {
    let t0 = ticks();
    let scale = 1.0 / libm::sqrtf(hd as f32);
    out.fill(0.0);
    for h in 0..heads {
        let off = h * hd;
        let q_h = &q[off..off + hd];
        for t in 0..seq {
            let k_h = &k[t * d + off..t * d + off + hd];
            scores[t] = dot_f32(q_h, k_h) * scale;
        }
        softmax_inplace(&mut scores[..seq]);
        for ki in 0..hd {
            let mut acc = 0.0f32;
            for t in 0..seq {
                acc += scores[t] * v[t * d + off + ki];
            }
            out[off + ki] = acc;
        }
    }
    if let Some(p) = profile {
        unsafe {
            (*p).flush_phase();
            let elapsed = ticks().wrapping_sub(t0);
            let mut cur = (*p).current.get();
            cur.cpu_attn = cur.cpu_attn.wrapping_add(elapsed);
            (*p).current.set(cur);
            (*p).phase_start.set(ticks());
        }
    }
}

fn build_conv_patch(
    input: &[f32],
    in_ch: usize,
    in_len: usize,
    ol: usize,
    stride: usize,
    padding: usize,
    k: usize,
    patch: &mut [f32],
) {
    let in_start = ol * stride;
    for ic in 0..in_ch {
        for ki in 0..k {
            let il = in_start as isize + ki as isize - padding as isize;
            patch[ic * k + ki] = if il >= 0 && (il as usize) < in_len {
                input[ic * in_len + il as usize]
            } else {
                0.0
            };
        }
    }
}

fn conv1d_matvec<S: TensorSource>(
    ctx: &mut ExecCtx<'_>,
    gpu: &mut Option<&mut dyn GpuDispatch>,
    source: &mut S,
    weight_key: &str,
    input: &[f32],
    in_ch: usize,
    in_len: usize,
    out_ch: usize,
    k: usize,
    stride: usize,
    padding: usize,
    scratch_patch: &mut [f32],
    scratch_im2col: &mut [f32],
    scratch_out: &mut [f32],
    out: &mut [f32],
) -> Result<(), ()> {
    let patch_len = in_ch * k;
    let out_len = (in_len + 2 * padding - k) / stride + 1;
    if scratch_patch.len() < patch_len
        || scratch_im2col.len() < patch_len * out_len
        || scratch_out.len() < out_ch * out_len
        || out.len() != out_ch * out_len
    {
        return Err(());
    }
    for ol in 0..out_len {
        build_conv_patch(
            input,
            in_ch,
            in_len,
            ol,
            stride,
            padding,
            k,
            &mut scratch_patch[..patch_len],
        );
        scratch_im2col[ol * patch_len..(ol + 1) * patch_len]
            .copy_from_slice(&scratch_patch[..patch_len]);
    }
    dispatch_matmul(
        ctx,
        gpu,
        source,
        weight_key,
        out_ch,
        patch_len,
        out_len,
        &scratch_im2col[..patch_len * out_len],
        &mut scratch_out[..out_ch * out_len],
    )?;
    for ol in 0..out_len {
        for oc in 0..out_ch {
            out[oc * out_len + ol] = scratch_out[ol * out_ch + oc];
        }
    }
    Ok(())
}

fn add_conv_bias<S: TensorSource>(
    source: &mut S,
    bias_key: &str,
    out_ch: usize,
    out_len: usize,
    out: &mut [f32],
) -> Result<(), ()> {
    let v = source.tensor_view(bias_key)?;
    let b = v.f32().ok_or(())?;
    for oc in 0..out_ch {
        let bi = b.get(oc).copied().unwrap_or(0.0);
        for t in 0..out_len {
            out[oc * out_len + t] += bi;
        }
    }
    Ok(())
}

fn logits_matvec<S: TensorSource>(
    ctx: &mut ExecCtx<'_>,
    gpu: &mut Option<&mut dyn GpuDispatch>,
    source: &mut S,
    last: &[f32],
    logits: &mut [f32],
) -> Result<(), ()> {
    let vocab = logits.len();
    let cols = last.len();
    let v = source.tensor_view("token_embed")?;
    if v.elems != vocab * cols {
        return Err(());
    }
    dispatch_matvec(ctx, gpu, source, "token_embed", vocab, cols, last, logits)
}

fn layer_norm_row<S: TensorSource>(
    source: &mut S,
    name: &str,
    x: &mut [f32],
) -> Result<(), ()> {
    let gw = tensor_f32(source, &format!("{name}.weight"))?;
    let gb = tensor_f32(source, &format!("{name}.bias"))?;
    layernorm(x, &gw, &gb, 1e-5);
    Ok(())
}

fn layer_norm_rows<S: TensorSource>(
    source: &mut S,
    name: &str,
    x: &mut [f32],
    d: usize,
) -> Result<(), ()> {
    let gw = tensor_f32(source, &format!("{name}.weight"))?;
    let gb = tensor_f32(source, &format!("{name}.bias"))?;
    for row in x.chunks_mut(d) {
        layernorm(row, &gw, &gb, 1e-5);
    }
    Ok(())
}

fn tensor_f32<S: TensorSource>(source: &mut S, name: &str) -> Result<Vec<f32>, ()> {
    let v = source.tensor_view(name)?;
    Ok(v.f32().ok_or(())?.iter().copied().collect())
}

impl<S: TensorSource> AsrRuntime<S> {
    pub fn new(manifest: Manifest, index: TensorIndex, source: S, tokenizer: Tokenizer) -> Result<Self, ()> {
        if manifest.model_kind != ModelKind::AsrEncoderDecoder {
            return Err(());
        }
        let d = manifest.audio.n_text_state as usize;
        Ok(Self {
            manifest,
            index,
            source,
            tokenizer,
            profile: None,
            sched: OpSched::default(),
            scratch_q: vec![0.0; d * 64],
            scratch_k: vec![0.0; d * 64],
            scratch_v: vec![0.0; d * 64],
            scratch_scores: vec![0.0; 64 * 64],
            scratch_tmp: vec![0.0; d * 64],
            scratch_mlp: Vec::new(),
            scratch_logits: Vec::new(),
            scratch_warmup_x: Vec::new(),
            scratch_warmup_y: Vec::new(),
            scratch_qkv: Vec::new(),
            scratch_head: Vec::new(),
            scratch_kt: Vec::new(),
            scratch_k_head: Vec::new(),
            fused_staging: Vec::new(),
            fused_weights: Vec::new(),
            token_hidden: vec![0.0; d],
            mel_padded: Vec::new(),
        })
    }

    /// Rellena mel hasta `2 * n_audio_ctx` con el piso normalizado (silencio).
    fn mel_for_encode(&mut self, mel: &[f32], n_frames: usize) -> (Vec<f32>, usize) {
        let n_mels = self.manifest.audio.n_mels as usize;
        let target = (self.manifest.audio.n_audio_ctx as usize) * 2;
        let src_len = n_mels * n_frames;
        if n_frames >= target {
            return (mel[..src_len.min(mel.len())].to_vec(), n_frames);
        }
        self.mel_padded.resize(n_mels * target, -1.0);
        let copy = src_len.min(mel.len()).min(self.mel_padded.len());
        self.mel_padded[..copy].copy_from_slice(&mel[..copy]);
        let max_norm = self.mel_padded[..copy]
            .iter()
            .copied()
            .fold(f32::NEG_INFINITY, f32::max);
        // Piso normalizado whisper: (max_log - 4) / 4 ≈ max_norm - 2.
        let pad_val = if max_norm.is_finite() {
            max_norm - 2.0
        } else {
            -1.0
        };
        for v in &mut self.mel_padded[copy..] {
            *v = pad_val;
        }
        (self.mel_padded.clone(), target)
    }

    fn prof(&self) -> Option<*const AsrProfile> {
        prof_ptr(self)
    }

    fn ensure_fused(&mut self, cache_key: &str, keys: &[&str]) -> Result<(), ()> {
        if self.fused_weights.iter().any(|(k, _)| k == cache_key) {
            return Ok(());
        }
        let mut fused = Vec::new();
        for key in keys {
            let v = self.source.tensor_view(key)?.f32().ok_or(())?;
            fused.extend_from_slice(v);
        }
        self.fused_weights
            .push((String::from(cache_key), fused));
        Ok(())
    }

    fn stage_fused(&mut self, cache_key: &str, keys: &[&str]) -> Result<(), ()> {
        self.ensure_fused(cache_key, keys)?;
        let w = self
            .fused_weights
            .iter()
            .find(|(k, _)| k == cache_key)
            .map(|(_, v)| v.as_slice())
            .ok_or(())?;
        self.fused_staging.clear();
        self.fused_staging.extend_from_slice(w);
        Ok(())
    }

    pub fn warmup_gpu(&mut self, gpu: &mut Option<&mut dyn GpuDispatch>) -> usize {
        let Some(g) = gpu.as_deref_mut() else {
            return 0;
        };
        if !g.available() {
            return 0;
        }
        let mut warmed = 0usize;
        for e in &self.index.entries {
            if e.shape.len() != 2 {
                continue;
            }
            let rows = e.shape[0] as usize;
            let cols = e.shape[1] as usize;
            if rows == 0 || cols == 0 {
                continue;
            }
            let Ok(v) = self.source.tensor_view(&e.name) else {
                continue;
            };
            if v.elems != rows * cols {
                continue;
            }
            if self.scratch_warmup_x.len() < cols {
                self.scratch_warmup_x.resize(cols, 0.0);
            }
            if self.scratch_warmup_y.len() < rows {
                self.scratch_warmup_y.resize(rows, 0.0);
            }
            if try_gpu_matvec(
                g,
                &e.name,
                &v,
                rows,
                cols,
                &self.scratch_warmup_x[..cols],
                &mut self.scratch_warmup_y[..rows],
            )
            .unwrap_or(false)
            {
                warmed += 1;
            }
        }
        let d = self.manifest.audio.n_audio_state as usize;
        let n_enc = self.manifest.audio.n_audio_layer as usize;
        let n_dec = self.manifest.audio.n_text_layer as usize;
        for layer in 0..n_enc {
            let prefix = format!("E{layer:02}");
            let q = format!("{prefix}.attn_q.weight");
            let k = format!("{prefix}.attn_k.weight");
            let v = format!("{prefix}.attn_v.weight");
            let fused = format!("{prefix}.fused_qkv");
            if self.ensure_fused(&fused, &[&q, &k, &v]).is_ok() {
                self.fused_staging.clear();
                if let Some(w) = self.fused_weights.iter().find(|(n, _)| n == &fused) {
                    self.fused_staging.extend_from_slice(&w.1);
                    let rows = 3 * d;
                    if self.scratch_warmup_x.len() < d {
                        self.scratch_warmup_x.resize(d, 0.0);
                    }
                    if self.scratch_warmup_y.len() < rows {
                        self.scratch_warmup_y.resize(rows, 0.0);
                    }
                    let tv = f32_tensor_view(&self.fused_staging);
                    if try_gpu_matvec(
                        g,
                        &fused,
                        &tv,
                        rows,
                        d,
                        &self.scratch_warmup_x[..d],
                        &mut self.scratch_warmup_y[..rows],
                    )
                    .unwrap_or(false)
                    {
                        warmed += 1;
                    }
                }
            }
        }
        for layer in 0..n_dec {
            let prefix = format!("D{layer:02}");
            let k = format!("{prefix}.cross_k.weight");
            let v = format!("{prefix}.cross_v.weight");
            let fused = format!("{prefix}.fused_kv");
            if self.ensure_fused(&fused, &[&k, &v]).is_ok() {
                self.fused_staging.clear();
                if let Some(w) = self.fused_weights.iter().find(|(n, _)| n == &fused) {
                    self.fused_staging.extend_from_slice(&w.1);
                    let rows = 2 * d;
                    if self.scratch_warmup_x.len() < d {
                        self.scratch_warmup_x.resize(d, 0.0);
                    }
                    if self.scratch_warmup_y.len() < rows {
                        self.scratch_warmup_y.resize(rows, 0.0);
                    }
                    let tv = f32_tensor_view(&self.fused_staging);
                    if try_gpu_matvec(
                        g,
                        &fused,
                        &tv,
                        rows,
                        d,
                        &self.scratch_warmup_x[..d],
                        &mut self.scratch_warmup_y[..rows],
                    )
                    .unwrap_or(false)
                    {
                        warmed += 1;
                    }
                }
            }
        }
        warmed
    }

    fn head_dim(&self) -> usize {
        let heads = self.manifest.audio.n_audio_head.max(1) as usize;
        self.manifest.audio.n_audio_state as usize / heads
    }

    fn mlp_gelu_rows(
        &mut self,
        use_gpu: bool,
        gpu: &mut Option<&mut dyn GpuDispatch>,
        prefix: &str,
        hidden: &mut [f32],
        d: usize,
        ffn: usize,
        seq: usize,
        phase: AsrPhase,
    ) -> Result<(), ()> {
        self.scratch_mlp.resize(ffn * seq, 0.0);
        let pp = self.prof();
        let fc1 = format!("{prefix}.mlp_fc1.weight");
        {
            let sched = &mut self.sched;
            let mut ctx = exec_ctx(use_gpu, sched, pp, phase);
            dispatch_matmul(
                &mut ctx,
                gpu,
                &mut self.source,
                &fc1,
                ffn,
                d,
                seq,
                hidden,
                &mut self.scratch_mlp,
            )?;
        }
        for t in 0..seq {
            add_bias(
                &mut self.source,
                &fc1,
                ffn,
                &mut self.scratch_mlp[t * ffn..(t + 1) * ffn],
            )?;
            gelu_inplace(&mut self.scratch_mlp[t * ffn..(t + 1) * ffn]);
        }
        let fc2 = format!("{prefix}.mlp_fc2.weight");
        {
            let sched = &mut self.sched;
            let mut ctx = exec_ctx(use_gpu, sched, pp, phase);
            dispatch_matmul(
                &mut ctx,
                gpu,
                &mut self.source,
                &fc2,
                d,
                ffn,
                seq,
                &self.scratch_mlp,
                hidden,
            )?;
        }
        for t in 0..seq {
            add_bias(
                &mut self.source,
                &fc2,
                d,
                &mut hidden[t * d..(t + 1) * d],
            )?;
        }
        Ok(())
    }

    fn self_attn(
        &mut self,
        use_gpu: bool,
        gpu: &mut Option<&mut dyn GpuDispatch>,
        prefix: &str,
        input: &[f32],
        output: &mut [f32],
        seq: usize,
        d: usize,
        heads: usize,
        hd: usize,
        causal: bool,
    ) -> Result<(), ()> {
        let pp = self.prof();
        let q_key = format!("{prefix}.attn_q.weight");
        let k_key = format!("{prefix}.attn_k.weight");
        let v_key = format!("{prefix}.attn_v.weight");
        let out_key = format!("{prefix}.attn_out.weight");
        let fused_key = format!("{prefix}.fused_qkv");
        self.stage_fused(&fused_key, &[&q_key, &k_key, &v_key])?;
        self.scratch_qkv.resize(3 * d * seq, 0.0);
        let fused = self.fused_staging.as_slice();

        self.scratch_q.resize(seq * d, 0.0);
        self.scratch_k.resize(seq * d, 0.0);
        self.scratch_v.resize(seq * d, 0.0);
        {
            let sched = &mut self.sched;
            let mut ctx = exec_ctx(use_gpu, sched, pp, AsrPhase::Encode);
            matmul_fused_qkv3(
                &mut ctx,
                gpu,
                &mut self.source,
                &fused_key,
                fused,
                d,
                &q_key,
                &k_key,
                &v_key,
                input,
                seq,
                &mut self.scratch_q[..seq * d],
                &mut self.scratch_k[..seq * d],
                &mut self.scratch_v[..seq * d],
                &mut self.scratch_qkv,
            )?;
        }

        let score_len = seq * seq;
        self.scratch_scores.resize(score_len, 0.0);
        self.scratch_head.resize(seq * hd, 0.0);
        self.scratch_k_head.resize(seq * hd, 0.0);
        self.scratch_kt.resize(hd * seq, 0.0);
        self.scratch_mlp.resize(seq * hd, 0.0);
        self.scratch_tmp.resize(seq * d, 0.0);
        prof_set_phase(pp, AsrPhase::CpuAttn);
        attention_batched(
            &self.scratch_q[..seq * d],
            &self.scratch_k[..seq * d],
            &self.scratch_v[..seq * d],
            seq,
            seq,
            d,
            heads,
            hd,
            causal,
            &mut self.scratch_scores[..score_len],
            &mut self.scratch_head[..seq * hd],
            &mut self.scratch_k_head[..seq * hd],
            &mut self.scratch_kt[..hd * seq],
            &mut self.scratch_mlp[..seq * hd],
            &mut self.scratch_tmp[..seq * d],
            pp,
        );
        prof_set_phase(pp, AsrPhase::SelfAttn);

        {
            let sched = &mut self.sched;
            let mut ctx = exec_ctx(use_gpu, sched, pp, AsrPhase::Encode);
            dispatch_matmul(
                &mut ctx,
                gpu,
                &mut self.source,
                &out_key,
                d,
                d,
                seq,
                &self.scratch_tmp[..seq * d],
                output,
            )?;
        }
        for t in 0..seq {
            add_bias(
                &mut self.source,
                &out_key,
                d,
                &mut output[t * d..(t + 1) * d],
            )?;
        }
        Ok(())
    }

    fn init_cross_kv(
        &mut self,
        cache: &mut DecoderCache,
        enc: &[f32],
        enc_seq: usize,
        use_gpu: bool,
        gpu: &mut Option<&mut dyn GpuDispatch>,
    ) -> Result<(), ()> {
        let pp = self.prof();
        prof_set_phase(pp, AsrPhase::CrossKv);
        let d = cache.d;
        let n_dec = self.manifest.audio.n_text_layer as usize;
        cache.enc_seq = enc_seq;
        for layer in 0..n_dec {
            let prefix = format!("D{layer:02}");
            let k_key = format!("{prefix}.cross_k.weight");
            let v_key = format!("{prefix}.cross_v.weight");
            let fused_kv_key = format!("{prefix}.fused_kv");
            self.stage_fused(&fused_kv_key, &[&k_key, &v_key])?;
            let fused_kv = self.fused_staging.as_slice();
            cache.cross[layer].k.resize(enc_seq * d, 0.0);
            cache.cross[layer].v.resize(enc_seq * d, 0.0);
            self.scratch_qkv.resize(2 * d * enc_seq, 0.0);
            {
                let sched = &mut self.sched;
                let mut ctx = exec_ctx(use_gpu, sched, pp, AsrPhase::CrossKv);
                dispatch_matmul_view(
                    &mut ctx,
                    gpu,
                    &fused_kv_key,
                    &f32_tensor_view(fused_kv),
                    2 * d,
                    d,
                    enc_seq,
                    enc,
                    &mut self.scratch_qkv[..2 * d * enc_seq],
                )?;
            }
            for t in 0..enc_seq {
                let row = &mut self.scratch_qkv[t * 2 * d..(t + 1) * 2 * d];
                add_fused_bias2(&mut self.source, &k_key, &v_key, d, row)?;
                cache.cross[layer].k[t * d..(t + 1) * d].copy_from_slice(&row[..d]);
                cache.cross[layer].v[t * d..(t + 1) * d].copy_from_slice(&row[d..2 * d]);
            }
        }
        Ok(())
    }

    fn decoder_self_attn(
        &mut self,
        cache: &mut DecoderCache,
        layer: usize,
        hidden: &mut [f32],
        d: usize,
        heads: usize,
        hd: usize,
        use_gpu: bool,
        gpu: &mut Option<&mut dyn GpuDispatch>,
    ) -> Result<(), ()> {
        let pp = self.prof();
        let prefix = format!("D{layer:02}");
        let q_key = format!("{prefix}.attn_q.weight");
        let k_key = format!("{prefix}.attn_k.weight");
        let v_key = format!("{prefix}.attn_v.weight");
        let out_key = format!("{prefix}.attn_out.weight");
        let fused_key = format!("{prefix}.fused_qkv");
        let ln = format!("{prefix}.attn_ln");

        self.scratch_head.resize(d, 0.0);
        self.scratch_mlp.resize(d, 0.0);
        self.scratch_head.copy_from_slice(hidden);
        layer_norm_row(&mut self.source, &ln, hidden)?;

        self.stage_fused(&fused_key, &[&q_key, &k_key, &v_key])?;
        self.scratch_qkv.resize(3 * d, 0.0);
        let fused = self.fused_staging.as_slice();
        self.scratch_q.resize(d, 0.0);
        self.scratch_k.resize(d, 0.0);
        self.scratch_v.resize(d, 0.0);
        {
            let sched = &mut self.sched;
            let mut ctx = exec_ctx(use_gpu, sched, pp, AsrPhase::SelfAttn);
            matvec_fused_qkv3(
                &mut ctx,
                gpu,
                &mut self.source,
                &fused_key,
                fused,
                d,
                &q_key,
                &k_key,
                &v_key,
                hidden,
                &mut self.scratch_qkv,
                &mut self.scratch_q[..d],
                &mut self.scratch_k[..d],
                &mut self.scratch_v[..d],
            )?;
        }
        cache.self_kv[layer].append(&self.scratch_k[..d], &self.scratch_v[..d], d);
        let seq = cache.self_kv[layer].len;
        self.scratch_k.resize(seq * d, 0.0);
        self.scratch_v.resize(seq * d, 0.0);
        self.scratch_k[..seq * d].copy_from_slice(&cache.self_kv[layer].k[..seq * d]);
        self.scratch_v[..seq * d].copy_from_slice(&cache.self_kv[layer].v[..seq * d]);
        self.scratch_scores.resize(seq, 0.0);
        self.scratch_tmp.resize(d, 0.0);
        prof_set_phase(pp, AsrPhase::CpuAttn);
        attention_single(
            &self.scratch_q[..d],
            &self.scratch_k[..seq * d],
            &self.scratch_v[..seq * d],
            seq,
            d,
            heads,
            hd,
            &mut self.scratch_scores[..seq],
            &mut self.scratch_tmp[..d],
            pp,
        );
        prof_set_phase(pp, AsrPhase::SelfAttn);
        {
            let sched = &mut self.sched;
            let mut ctx = exec_ctx(use_gpu, sched, pp, AsrPhase::SelfAttn);
            matvec_bias(
                &mut ctx,
                gpu,
                &mut self.source,
                &out_key,
                d,
                d,
                &self.scratch_tmp[..d],
                &mut self.scratch_mlp[..d],
            )?;
        }
        for i in 0..d {
            hidden[i] = self.scratch_head[i] + self.scratch_mlp[i];
        }
        Ok(())
    }

    fn decoder_cross_attn(
        &mut self,
        cache: &mut DecoderCache,
        layer: usize,
        hidden: &mut [f32],
        d: usize,
        heads: usize,
        hd: usize,
        use_gpu: bool,
        gpu: &mut Option<&mut dyn GpuDispatch>,
    ) -> Result<(), ()> {
        let pp = self.prof();
        let prefix = format!("D{layer:02}");
        let q_key = format!("{prefix}.cross_q.weight");
        let out_key = format!("{prefix}.cross_out.weight");
        let ln = format!("{prefix}.cross_ln");
        let enc_seq = cache.enc_seq;

        self.scratch_head.resize(d, 0.0);
        self.scratch_mlp.resize(d, 0.0);
        self.scratch_head.copy_from_slice(hidden);
        layer_norm_row(&mut self.source, &ln, hidden)?;

        self.scratch_q.resize(d, 0.0);
        {
            let sched = &mut self.sched;
            let mut ctx = exec_ctx(use_gpu, sched, pp, AsrPhase::CrossAttn);
            matvec_bias(
                &mut ctx,
                gpu,
                &mut self.source,
                &q_key,
                d,
                d,
                hidden,
                &mut self.scratch_q[..d],
            )?;
        }
        self.scratch_scores.resize(enc_seq, 0.0);
        self.scratch_tmp.resize(d, 0.0);
        self.scratch_k.resize(enc_seq * d, 0.0);
        self.scratch_v.resize(enc_seq * d, 0.0);
        self.scratch_k[..enc_seq * d].copy_from_slice(&cache.cross[layer].k[..enc_seq * d]);
        self.scratch_v[..enc_seq * d].copy_from_slice(&cache.cross[layer].v[..enc_seq * d]);
        prof_set_phase(pp, AsrPhase::CpuAttn);
        attention_single(
            &self.scratch_q[..d],
            &self.scratch_k[..enc_seq * d],
            &self.scratch_v[..enc_seq * d],
            enc_seq,
            d,
            heads,
            hd,
            &mut self.scratch_scores[..enc_seq],
            &mut self.scratch_tmp[..d],
            pp,
        );
        prof_set_phase(pp, AsrPhase::CrossAttn);
        {
            let sched = &mut self.sched;
            let mut ctx = exec_ctx(use_gpu, sched, pp, AsrPhase::CrossAttn);
            matvec_bias(
                &mut ctx,
                gpu,
                &mut self.source,
                &out_key,
                d,
                d,
                &self.scratch_tmp[..d],
                &mut self.scratch_mlp[..d],
            )?;
        }
        for i in 0..d {
            hidden[i] = self.scratch_head[i] + self.scratch_mlp[i];
        }
        Ok(())
    }

    fn decoder_mlp(
        &mut self,
        layer: usize,
        hidden: &mut [f32],
        d: usize,
        ffn: usize,
        use_gpu: bool,
        gpu: &mut Option<&mut dyn GpuDispatch>,
    ) -> Result<(), ()> {
        let prefix = format!("D{layer:02}");
        let ln = format!("{prefix}.mlp_ln");
        self.scratch_head.resize(d, 0.0);
        self.scratch_v.resize(d, 0.0);
        self.scratch_head.copy_from_slice(hidden);
        layer_norm_row(&mut self.source, &ln, hidden)?;
        let pp = self.prof();
        self.scratch_mlp.resize(ffn, 0.0);
        let fc1 = format!("{prefix}.mlp_fc1.weight");
        {
            let sched = &mut self.sched;
            let mut ctx = exec_ctx(use_gpu, sched, pp, AsrPhase::Mlp);
            matvec_bias(
                &mut ctx,
                gpu,
                &mut self.source,
                &fc1,
                ffn,
                d,
                hidden,
                &mut self.scratch_mlp,
            )?;
        }
        gelu_inplace(&mut self.scratch_mlp);
        let fc2 = format!("{prefix}.mlp_fc2.weight");
        {
            let sched = &mut self.sched;
            let mut ctx = exec_ctx(use_gpu, sched, pp, AsrPhase::Mlp);
            matvec_bias(
                &mut ctx,
                gpu,
                &mut self.source,
                &fc2,
                d,
                ffn,
                &self.scratch_mlp,
                &mut self.scratch_v[..d],
            )?;
        }
        for i in 0..d {
            hidden[i] = self.scratch_head[i] + self.scratch_v[i];
        }
        Ok(())
    }

    fn decoder_step(
        &mut self,
        cache: &mut DecoderCache,
        token: u32,
        pos: usize,
        use_gpu: bool,
        gpu: &mut Option<&mut dyn GpuDispatch>,
        need_logits: bool,
    ) -> Result<(), ()> {
        let pp = self.prof();
        let d = cache.d;
        let heads = self.manifest.audio.n_text_head as usize;
        let hd = d / heads.max(1);
        let ffn = self.manifest.ffn_dim as usize;
        let vocab = self.manifest.vocab_size as usize;
        let n_dec = self.manifest.audio.n_text_layer as usize;

        prof_begin_token(pp, pos + 1);

        let ti = token as usize;
        if ti >= vocab {
            return Err(());
        }
        self.token_hidden.fill(0.0);
        self.source
            .load_f32_range("token_embed", ti * d, &mut self.token_hidden)?;
        if let Ok(pe) = tensor_f32(&mut self.source, "dec_pos_embed") {
            if pos * d + d <= pe.len() {
                for c in 0..d {
                    self.token_hidden[c] += pe[pos * d + c];
                }
            }
        }

        let mut hidden = self.token_hidden.clone();

        for layer in 0..n_dec {
            prof_set_phase(pp, AsrPhase::SelfAttn);
            self.decoder_self_attn(&mut *cache, layer, &mut hidden, d, heads, hd, use_gpu, gpu)?;
            prof_set_phase(pp, AsrPhase::CrossAttn);
            self.decoder_cross_attn(&mut *cache, layer, &mut hidden, d, heads, hd, use_gpu, gpu)?;
            prof_set_phase(pp, AsrPhase::Mlp);
            self.decoder_mlp(layer, &mut hidden, d, ffn, use_gpu, gpu)?;
        }

        layer_norm_row(&mut self.source, "dec_ln", &mut hidden)?;
        self.token_hidden.copy_from_slice(&hidden);

        if need_logits {
            prof_set_phase(pp, AsrPhase::Logits);
            self.scratch_logits.resize(vocab, 0.0);
            {
                let sched = &mut self.sched;
                let mut ctx = exec_ctx(use_gpu, sched, pp, AsrPhase::Logits);
                logits_matvec(
                    &mut ctx,
                    gpu,
                    &mut self.source,
                    &hidden,
                    &mut self.scratch_logits[..vocab],
                )?;
            }
        }

        if let Some(prof) = self.profile.as_mut() {
            if prof.enabled {
                prof.flush_phase();
                prof.tokens.push(prof.current.get());
            }
        }
        Ok(())
    }

    pub fn encode(&mut self, mel: &[f32], n_frames: usize) -> Result<Vec<f32>, ()> {
        self.encode_with_gpu(mel, n_frames, false, &mut None)
    }

    pub fn encode_with_gpu(
        &mut self,
        mel: &[f32],
        n_frames: usize,
        use_gpu: bool,
        gpu: &mut Option<&mut dyn GpuDispatch>,
    ) -> Result<Vec<f32>, ()> {
        let (mel, n_frames) = self.mel_for_encode(mel, n_frames);
        let pp = self.prof();
        prof_set_phase(pp, AsrPhase::Encode);
        let n_mels = self.manifest.audio.n_mels as usize;
        let d = self.manifest.audio.n_audio_state as usize;
        let heads = self.manifest.audio.n_audio_head as usize;
        let hd = self.head_dim();
        let ffn = self.manifest.ffn_dim as usize;
        let conv1_out_len = n_frames;
        let mut h1 = vec![0.0f32; d * conv1_out_len];
        {
            let patch_len = n_mels * 3;
            if self.scratch_warmup_x.len() < patch_len {
                self.scratch_warmup_x.resize(patch_len, 0.0);
            }
            let im2col_len = patch_len * conv1_out_len;
            if self.scratch_tmp.len() < im2col_len {
                self.scratch_tmp.resize(im2col_len, 0.0);
            }
            if self.scratch_warmup_y.len() < d * conv1_out_len {
                self.scratch_warmup_y.resize(d * conv1_out_len, 0.0);
            }
            {
                let sched = &mut self.sched;
            let mut ctx = exec_ctx(use_gpu, sched, pp, AsrPhase::Encode);
                conv1d_matvec(
                    &mut ctx,
                    gpu,
                    &mut self.source,
                    "conv1.weight",
                    &mel,
                    n_mels,
                    n_frames,
                    d,
                    3,
                    1,
                    1,
                    &mut self.scratch_warmup_x[..patch_len],
                    &mut self.scratch_tmp[..im2col_len],
                    &mut self.scratch_warmup_y[..d * conv1_out_len],
                    &mut h1,
                )?;
            }
            add_conv_bias(&mut self.source, "conv1.bias", d, conv1_out_len, &mut h1)?;
        }
        gelu_inplace(&mut h1);
        let conv2_out_len = (n_frames + 2 - 3) / 2 + 1;
        let mut enc = vec![0.0f32; d * conv2_out_len];
        {
            let patch_len = d * 3;
            if self.scratch_warmup_x.len() < patch_len {
                self.scratch_warmup_x.resize(patch_len, 0.0);
            }
            let im2col_len = patch_len * conv2_out_len;
            if self.scratch_tmp.len() < im2col_len {
                self.scratch_tmp.resize(im2col_len, 0.0);
            }
            if self.scratch_warmup_y.len() < d * conv2_out_len {
                self.scratch_warmup_y.resize(d * conv2_out_len, 0.0);
            }
            {
                let sched = &mut self.sched;
            let mut ctx = exec_ctx(use_gpu, sched, pp, AsrPhase::Encode);
                conv1d_matvec(
                    &mut ctx,
                    gpu,
                    &mut self.source,
                    "conv2.weight",
                    &h1,
                    d,
                    conv1_out_len,
                    d,
                    3,
                    2,
                    1,
                    &mut self.scratch_warmup_x[..patch_len],
                    &mut self.scratch_tmp[..im2col_len],
                    &mut self.scratch_warmup_y[..d * conv2_out_len],
                    &mut enc,
                )?;
            }
            add_conv_bias(&mut self.source, "conv2.bias", d, conv2_out_len, &mut enc)?;
        }
        gelu_inplace(&mut enc);
        let mut hidden = vec![0.0f32; conv2_out_len * d];
        for t in 0..conv2_out_len {
            for c in 0..d {
                hidden[t * d + c] = enc[c * conv2_out_len + t];
            }
        }
        if let Ok(pos) = tensor_f32(&mut self.source, "pos_embed") {
            for t in 0..conv2_out_len {
                for c in 0..d {
                    hidden[t * d + c] += pos[t * d + c];
                }
            }
        }
        let n_enc = self.manifest.audio.n_audio_layer as usize;
        let hidden_len = conv2_out_len * d;
        self.scratch_mlp.resize(hidden_len, 0.0);
        self.scratch_tmp.resize(hidden_len, 0.0);
        for layer in 0..n_enc {
            let prefix = format!("E{layer:02}");
            self.scratch_warmup_y.resize(hidden_len, 0.0);
            self.scratch_warmup_y.copy_from_slice(&hidden);
            layer_norm_rows(&mut self.source, &format!("{prefix}.attn_ln"), &mut hidden, d)?;
            let mut attn_out = vec![0.0f32; hidden_len];
            self.self_attn(
                use_gpu,
                gpu,
                &prefix,
                &hidden,
                &mut attn_out,
                conv2_out_len,
                d,
                heads,
                hd,
                false,
            )?;
            for i in 0..hidden_len {
                hidden[i] = self.scratch_warmup_y[i] + attn_out[i];
            }
            self.scratch_warmup_y.copy_from_slice(&hidden);
            layer_norm_rows(&mut self.source, &format!("{prefix}.mlp_ln"), &mut hidden, d)?;
            self.mlp_gelu_rows(
                use_gpu,
                gpu,
                &prefix,
                &mut hidden,
                d,
                ffn,
                conv2_out_len,
                AsrPhase::Encode,
            )?;
            for i in 0..hidden_len {
                hidden[i] = self.scratch_warmup_y[i] + hidden[i];
            }
        }
        layer_norm_rows(&mut self.source, "enc_ln_post", &mut hidden, d)?;
        if let Some(p) = pp {
            unsafe {
                (*p).flush_phase();
            }
        }
        Ok(hidden)
    }

    pub fn decode_greedy(&mut self, enc: &[f32], enc_seq: usize, prompt: &[u32]) -> Result<String, ()> {
        self.decode_greedy_with_gpu(enc, enc_seq, prompt, false, &mut None, None)
    }

    pub fn decode_greedy_with_gpu(
        &mut self,
        enc: &[f32],
        enc_seq: usize,
        prompt: &[u32],
        use_gpu: bool,
        gpu: &mut Option<&mut dyn GpuDispatch>,
        max_new_tokens: Option<usize>,
    ) -> Result<String, ()> {
        let d = self.manifest.audio.n_text_state as usize;
        let max_ctx = self.manifest.audio.n_text_ctx as usize;
        let token_limit = max_new_tokens
            .map(|n| prompt.len().saturating_add(n).min(max_ctx))
            .unwrap_or(max_ctx);
        let vocab = self.manifest.vocab_size as usize;
        let n_dec = self.manifest.audio.n_text_layer as usize;
        self.scratch_logits.resize(vocab, 0.0);

        let mut cache = DecoderCache::new(n_dec, max_ctx, d);
        self.init_cross_kv(&mut cache, enc, enc_seq, use_gpu, gpu)?;

        let mut tokens = prompt.to_vec();

        // Prefill del prompt (logits solo en el último token del prompt).
        for (pos, &tok) in prompt.iter().enumerate() {
            let need_logits = pos + 1 == prompt.len();
            self.decoder_step(&mut cache, tok, pos, use_gpu, gpu, need_logits)?;
        }

        loop {
            if tokens.len() >= token_limit {
                break;
            }
            let logits = &mut self.scratch_logits[..vocab];
            apply_whisper_logit_filters(logits, tokens.len() - prompt.len());
            let next = argmax_logit(logits).ok_or(())?;
            if next == WHISPER_EOT || self.tokenizer.eos() == Some(next) {
                break;
            }
            tokens.push(next);
            let pos = tokens.len() - 1;
            self.decoder_step(&mut cache, next, pos, use_gpu, gpu, true)?;
        }
        Ok(self.tokenizer.decode(&tokens[prompt.len()..]))
    }

    pub fn transcribe_tokens(
        &mut self,
        mel: &[f32],
        n_frames: usize,
        lang_token: u32,
        max_new_tokens: Option<usize>,
    ) -> Result<(String, Vec<u32>, Vec<u32>), ()> {
        self.transcribe_tokens_with_gpu(mel, n_frames, lang_token, false, &mut None, max_new_tokens)
    }

    pub fn transcribe_tokens_with_gpu(
        &mut self,
        mel: &[f32],
        n_frames: usize,
        lang_token: u32,
        use_gpu: bool,
        gpu: &mut Option<&mut dyn GpuDispatch>,
        max_new_tokens: Option<usize>,
    ) -> Result<(String, Vec<u32>, Vec<u32>), ()> {
        let enc = self.encode_with_gpu(mel, n_frames, use_gpu, gpu)?;
        let enc_seq = enc.len() / self.manifest.audio.n_audio_state as usize;
        let lang = whisper_lang_token(lang_token);
        let prompt = vec![50258, lang, 50359, 50363];
        let d = self.manifest.audio.n_text_state as usize;
        let max_ctx = self.manifest.audio.n_text_ctx as usize;
        let token_limit = max_new_tokens
            .map(|n| prompt.len().saturating_add(n).min(max_ctx))
            .unwrap_or(max_ctx);
        let vocab = self.manifest.vocab_size as usize;
        let n_dec = self.manifest.audio.n_text_layer as usize;
        self.scratch_logits.resize(vocab, 0.0);
        let mut cache = DecoderCache::new(n_dec, max_ctx, d);
        self.init_cross_kv(&mut cache, &enc, enc_seq, use_gpu, gpu)?;
        let mut tokens = prompt.clone();
        for (pos, &tok) in prompt.iter().enumerate() {
            let need_logits = pos + 1 == prompt.len();
            self.decoder_step(&mut cache, tok, pos, use_gpu, gpu, need_logits)?;
        }
        loop {
            if tokens.len() >= token_limit {
                break;
            }
            let logits = &mut self.scratch_logits[..vocab];
            apply_whisper_logit_filters(logits, tokens.len() - prompt.len());
            let next = argmax_logit(logits).ok_or(())?;
            if next == WHISPER_EOT || self.tokenizer.eos() == Some(next) {
                break;
            }
            tokens.push(next);
            let pos = tokens.len() - 1;
            self.decoder_step(&mut cache, next, pos, use_gpu, gpu, true)?;
        }
        let new_tokens = tokens[prompt.len()..].to_vec();
        let text = self.tokenizer.decode(&new_tokens);
        Ok((text, tokens, new_tokens))
    }

    pub fn transcribe(&mut self, mel: &[f32], n_frames: usize, lang_token: u32) -> Result<String, ()> {
        self.transcribe_with_gpu(mel, n_frames, lang_token, false, &mut None, None)
    }

    pub fn transcribe_with_gpu(
        &mut self,
        mel: &[f32],
        n_frames: usize,
        lang_token: u32,
        use_gpu: bool,
        gpu: &mut Option<&mut dyn GpuDispatch>,
        max_new_tokens: Option<usize>,
    ) -> Result<String, ()> {
        // Encode + cross-K/V precalc: O(enc_seq) matvec por capa decoder (~9k–25k
        // lanzamientos GSP con mel padded). En VFIO eso supera 300 s; en CPU ~10 s.
        let enc = self.encode_with_gpu(mel, n_frames, use_gpu, gpu)?;
        let enc_seq = enc.len() / self.manifest.audio.n_audio_state as usize;
        let lang = whisper_lang_token(lang_token);
        let prompt = vec![50258, lang, 50359, 50363];
        self.decode_greedy_with_gpu(&enc, enc_seq, &prompt, use_gpu, gpu, max_new_tokens)
    }
}

fn whisper_lang_token(idioma: u32) -> u32 {
    if (50259..=50357).contains(&idioma) {
        return idioma;
    }
    if idioma < 99 {
        return 50259 + idioma;
    }
    idioma
}

// Tests unitarios de atención/residual en `tests/asr_math.rs`.
