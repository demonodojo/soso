//! KV cache: f16 (default) o int8 per-token (KIVI-lite) + eviction H2O.
//!
//! - **KIVI** (KV cache quantization): escala por token, int8 simétrico → ~2×
//!   menos RAM que f16 con pérdida acotada en decode.
//! - **H2O**: al deslizar la ventana, conserva sink + tokens de mayor masa de
//!   atención acumulada + recientes (en vez de solo FIFO).

use crate::f16::{f16_to_f32, f32_to_f16};
use alloc::vec::Vec;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KvDtype {
    F16,
    /// int8 + escala f32 por token (K/V por separado).
    I8,
}

/// Cache K/V de una capa.
pub struct LayerKv {
    pub dtype: KvDtype,
    /// Si true, `k_*` guarda el vector latente MLA (`c_kv`) por token.
    pub mla_latent: bool,
    /// f16 packed, o vacío si I8.
    pub k_f16: Vec<u16>,
    pub v_f16: Vec<u16>,
    pub k_i8: Vec<i8>,
    pub v_i8: Vec<i8>,
    /// Escala por token: valor_real ≈ i8 * scale.
    pub k_scale: Vec<f32>,
    pub v_scale: Vec<f32>,
    /// Masa de atención acumulada (H2O), un f32 por token.
    pub mass: Vec<f32>,
    /// Estado recurrente GDN: `[n_v_heads][d][d]`.
    pub gdn_s: Vec<f32>,
    /// Historial causal conv1d: `[(kernel-1) * conv_dim]`.
    pub gdn_conv: Vec<f32>,
}

impl LayerKv {
    pub fn new() -> Self {
        Self::with_capacity_dtype(0, 0, KvDtype::F16)
    }

    pub fn with_capacity(tokens: usize, kv_dim: usize) -> Self {
        Self::with_capacity_dtype(tokens, kv_dim, KvDtype::F16)
    }

    pub fn with_capacity_dtype(tokens: usize, kv_dim: usize, dtype: KvDtype) -> Self {
        let n = tokens.saturating_mul(kv_dim);
        match dtype {
            KvDtype::F16 => Self {
                dtype,
                mla_latent: false,
                k_f16: Vec::with_capacity(n),
                v_f16: Vec::with_capacity(n),
                k_i8: Vec::new(),
                v_i8: Vec::new(),
                k_scale: Vec::new(),
                v_scale: Vec::new(),
                mass: Vec::with_capacity(tokens),
                gdn_s: Vec::new(),
                gdn_conv: Vec::new(),
            },
            KvDtype::I8 => Self {
                dtype,
                mla_latent: false,
                k_f16: Vec::new(),
                v_f16: Vec::new(),
                k_i8: Vec::with_capacity(n),
                v_i8: Vec::with_capacity(n),
                k_scale: Vec::with_capacity(tokens),
                v_scale: Vec::with_capacity(tokens),
                mass: Vec::with_capacity(tokens),
                gdn_s: Vec::new(),
                gdn_conv: Vec::new(),
            },
        }
    }

    /// Cache MLA: un vector latente `c_kv` por token (sin materializar K/V).
    pub fn with_capacity_mla(tokens: usize, kv_rank: usize) -> Self {
        let mut kv = Self::with_capacity_dtype(tokens, kv_rank, KvDtype::F16);
        kv.mla_latent = true;
        kv
    }

    pub fn with_capacity_mla_dtype(tokens: usize, kv_rank: usize, dtype: KvDtype) -> Self {
        let mut kv = Self::with_capacity_dtype(tokens, kv_rank, dtype);
        kv.mla_latent = true;
        kv
    }

    pub fn reset(&mut self) {
        self.k_f16.clear();
        self.v_f16.clear();
        self.k_i8.clear();
        self.v_i8.clear();
        self.k_scale.clear();
        self.v_scale.clear();
        self.mass.clear();
        self.gdn_s.fill(0.0);
        self.gdn_conv.fill(0.0);
    }

    /// Reserva (y pone a cero) el estado GDN si el tamaño no coincide.
    pub fn ensure_gdn(&mut self, n_v_heads: usize, head_dim: usize, conv_len: usize) {
        let s_len = n_v_heads.saturating_mul(head_dim).saturating_mul(head_dim);
        if self.gdn_s.len() != s_len {
            self.gdn_s.clear();
            self.gdn_s.resize(s_len, 0.0);
        }
        if self.gdn_conv.len() != conv_len {
            self.gdn_conv.clear();
            self.gdn_conv.resize(conv_len, 0.0);
        }
    }

    pub fn tokens(&self, kv_dim: usize) -> usize {
        if kv_dim == 0 {
            return 0;
        }
        match self.dtype {
            KvDtype::F16 => self.k_f16.len() / kv_dim,
            KvDtype::I8 => self.k_scale.len(),
        }
    }

    fn quantize_token(x: &[f32]) -> (Vec<i8>, f32) {
        let mut max_abs = 0.0f32;
        for &v in x {
            let a = libm::fabsf(v);
            if a > max_abs {
                max_abs = a;
            }
        }
        let scale = if max_abs > 0.0 { max_abs / 127.0 } else { 1.0 };
        let inv = 1.0 / scale;
        let mut out = Vec::with_capacity(x.len());
        for &v in x {
            let q = libm::roundf(v * inv).clamp(-127.0, 127.0) as i8;
            out.push(q);
        }
        (out, scale)
    }

    pub fn append(&mut self, k: &[f32], v: &[f32]) {
        if self.mla_latent {
            self.append_mla_latent(k);
            return;
        }
        match self.dtype {
            KvDtype::F16 => {
                self.k_f16.extend(k.iter().map(|&x| f32_to_f16(x)));
                self.v_f16.extend(v.iter().map(|&x| f32_to_f16(x)));
            }
            KvDtype::I8 => {
                let (kq, ks) = Self::quantize_token(k);
                let (vq, vs) = Self::quantize_token(v);
                self.k_i8.extend_from_slice(&kq);
                self.v_i8.extend_from_slice(&vq);
                self.k_scale.push(ks);
                self.v_scale.push(vs);
            }
        }
        self.mass.push(0.0);
    }

    /// Append del vector latente MLA (`c_kv`); ignora `v`.
    pub fn append_mla_latent(&mut self, c_kv: &[f32]) {
        debug_assert!(self.mla_latent);
        match self.dtype {
            KvDtype::F16 => {
                self.k_f16.extend(c_kv.iter().map(|&x| f32_to_f16(x)));
            }
            KvDtype::I8 => {
                let (kq, ks) = Self::quantize_token(c_kv);
                self.k_i8.extend_from_slice(&kq);
                self.k_scale.push(ks);
            }
        }
        self.mass.push(0.0);
    }

    /// Lee `c_kv` del token `t` (modo MLA).
    pub fn load_latent_token(&self, t: usize, kv_rank: usize, out: &mut [f32]) {
        if !self.mla_latent || out.len() < kv_rank {
            return;
        }
        let base = t * kv_rank;
        match self.dtype {
            KvDtype::F16 => {
                for d in 0..kv_rank {
                    out[d] = f16_to_f32(self.k_f16[base + d]);
                }
            }
            KvDtype::I8 => {
                let s = self.k_scale[t];
                for d in 0..kv_rank {
                    out[d] = self.k_i8[base + d] as f32 * s;
                }
            }
        }
    }

    /// Compat: append desde f32 (antes `append_f16`).
    pub fn append_f16(&mut self, k: &[f32], v: &[f32]) {
        self.append(k, v);
    }

    /// Acumula masa de atención (softmax) sobre las posiciones atendidas.
    pub fn accumulate_mass(&mut self, scores: &[f32]) {
        let n = self.mass.len().min(scores.len());
        for i in 0..n {
            self.mass[i] += scores[i];
        }
    }

    /// StreamingLLM FIFO: sink + recientes.
    pub fn slide_window(&mut self, keep: usize, sink: usize, kv_dim: usize) {
        self.slide_window_h2o(keep, sink, keep.saturating_sub(sink) / 2, kv_dim, false);
    }

    /// H2O + StreamingLLM: sink + top-(keep-sink-recent) por masa + recientes.
    /// Si `use_h2o` es false, equivale a FIFO StreamingLLM.
    pub fn slide_window_h2o(
        &mut self,
        keep: usize,
        sink: usize,
        recent: usize,
        kv_dim: usize,
        use_h2o: bool,
    ) {
        let n = self.tokens(kv_dim);
        if n <= keep || keep == 0 || kv_dim == 0 {
            return;
        }
        let sink = sink.min(keep);
        let recent = recent.min(keep.saturating_sub(sink));
        let mid_budget = keep.saturating_sub(sink).saturating_sub(recent);

        let mut keep_idx: Vec<usize> = Vec::with_capacity(keep);
        for i in 0..sink.min(n) {
            keep_idx.push(i);
        }
        let recent_start = n.saturating_sub(recent);
        if use_h2o && mid_budget > 0 && recent_start > sink {
            // Candidatos en (sink .. recent_start): los de mayor masa.
            let mut cands: Vec<(usize, f32)> = (sink..recent_start)
                .map(|i| (i, self.mass.get(i).copied().unwrap_or(0.0)))
                .collect();
            cands.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(core::cmp::Ordering::Equal));
            for (i, _) in cands.into_iter().take(mid_budget) {
                keep_idx.push(i);
            }
        } else {
            // FIFO: el "medio" son los tokens justo antes de recent.
            let drop = n - keep;
            let start_recent = sink + drop;
            for i in sink..start_recent.min(n) {
                // no añadir medio en FIFO puro
                let _ = i;
            }
            // En FIFO, keep = sink + [start_recent..n]
            keep_idx.clear();
            for i in 0..sink.min(n) {
                keep_idx.push(i);
            }
            let start_recent = sink + (n - keep);
            for i in start_recent..n {
                keep_idx.push(i);
            }
            self.repack_indices(&keep_idx, kv_dim);
            return;
        }
        for i in recent_start..n {
            if !keep_idx.contains(&i) {
                keep_idx.push(i);
            }
        }
        keep_idx.sort_unstable();
        keep_idx.dedup();
        if keep_idx.len() > keep {
            keep_idx.truncate(keep);
        }
        self.repack_indices(&keep_idx, kv_dim);
    }

    fn repack_indices(&mut self, idx: &[usize], kv_dim: usize) {
        match self.dtype {
            KvDtype::F16 => {
                let mut nk = Vec::with_capacity(idx.len() * kv_dim);
                let mut nv = Vec::with_capacity(idx.len() * kv_dim);
                let mut nm = Vec::with_capacity(idx.len());
                for &t in idx {
                    let off = t * kv_dim;
                    nk.extend_from_slice(&self.k_f16[off..off + kv_dim]);
                    if !self.mla_latent {
                        nv.extend_from_slice(&self.v_f16[off..off + kv_dim]);
                    }
                    nm.push(self.mass.get(t).copied().unwrap_or(0.0));
                }
                self.k_f16 = nk;
                if !self.mla_latent {
                    self.v_f16 = nv;
                }
                self.mass = nm;
            }
            KvDtype::I8 => {
                let mut nk = Vec::with_capacity(idx.len() * kv_dim);
                let mut nv = Vec::with_capacity(idx.len() * kv_dim);
                let mut nks = Vec::with_capacity(idx.len());
                let mut nvs = Vec::with_capacity(idx.len());
                let mut nm = Vec::with_capacity(idx.len());
                for &t in idx {
                    let off = t * kv_dim;
                    nk.extend_from_slice(&self.k_i8[off..off + kv_dim]);
                    if !self.mla_latent {
                        nv.extend_from_slice(&self.v_i8[off..off + kv_dim]);
                        nvs.push(self.v_scale[t]);
                    }
                    nks.push(self.k_scale[t]);
                    nm.push(self.mass.get(t).copied().unwrap_or(0.0));
                }
                self.k_i8 = nk;
                if !self.mla_latent {
                    self.v_i8 = nv;
                    self.v_scale = nvs;
                }
                self.k_scale = nks;
                self.mass = nm;
            }
        }
    }

    /// Dequantiza token `t` cabeza `kv_head` a `out` (head_dim).
    pub fn load_k_head(&self, t: usize, kv_head: usize, head_dim: usize, kv_dim: usize, out: &mut [f32]) {
        let base = t * kv_dim + kv_head * head_dim;
        match self.dtype {
            KvDtype::F16 => {
                for d in 0..head_dim {
                    out[d] = f16_to_f32(self.k_f16[base + d]);
                }
            }
            KvDtype::I8 => {
                let s = self.k_scale[t];
                for d in 0..head_dim {
                    out[d] = self.k_i8[base + d] as f32 * s;
                }
            }
        }
    }

    pub fn load_v_head(&self, t: usize, kv_head: usize, head_dim: usize, kv_dim: usize, out: &mut [f32]) {
        let base = t * kv_dim + kv_head * head_dim;
        match self.dtype {
            KvDtype::F16 => {
                for d in 0..head_dim {
                    out[d] = f16_to_f32(self.v_f16[base + d]);
                }
            }
            KvDtype::I8 => {
                let s = self.v_scale[t];
                for d in 0..head_dim {
                    out[d] = self.v_i8[base + d] as f32 * s;
                }
            }
        }
    }
}

/// Compat: tests antiguos usan `.k` / `.v` f16.
impl LayerKv {
    pub fn k_f16_slice(&self) -> &[u16] {
        &self.k_f16
    }
    pub fn v_f16_slice(&self) -> &[u16] {
        &self.v_f16
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn int8_roundtrip_reasonable() {
        let mut kv = LayerKv::with_capacity_dtype(4, 4, KvDtype::I8);
        let k = [1.0f32, -0.5, 0.25, 0.0];
        let v = [0.1f32, 0.2, 0.3, 0.4];
        kv.append(&k, &v);
        let mut out = [0.0f32; 4];
        kv.load_k_head(0, 0, 4, 4, &mut out);
        for i in 0..4 {
            assert!((out[i] - k[i]).abs() < 0.02, "{} vs {}", out[i], k[i]);
        }
    }

    #[test]
    fn h2o_keeps_heavy_hitters() {
        let mut kv = LayerKv::with_capacity_dtype(10, 2, KvDtype::F16);
        for t in 0..10 {
            let k = [t as f32, 0.0];
            let v = [0.0f32, t as f32];
            kv.append(&k, &v);
        }
        // Masa alta en token 3
        kv.mass = vec![0.0, 0.0, 0.0, 100.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
        kv.slide_window_h2o(5, 1, 2, 2, true);
        assert_eq!(kv.tokens(2), 5);
        // Debe incluir sink(0), heavy(3), y recientes (8,9) + otro
        let mut heads = Vec::new();
        let mut tmp = [0.0f32; 2];
        for t in 0..kv.tokens(2) {
            kv.load_k_head(t, 0, 2, 2, &mut tmp);
            heads.push(tmp[0] as u32);
        }
        assert!(heads.contains(&0));
        assert!(heads.contains(&3));
        assert!(heads.contains(&9));
    }
}
