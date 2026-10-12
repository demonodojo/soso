//! Sampling de tokens: greedy (temp = 0) o nucleus top-p con temperatura.
//! RNG xorshift64* determinista (no hay fuente de tiempo en userspace).

use alloc::vec::Vec;

pub struct Sampler {
    pub temp: f32,
    pub top_p: f32,
    rng: u64,
    // buffers reutilizados entre tokens
    probs: Vec<f32>,
    idx: Vec<u32>,
}

impl Sampler {
    pub fn new(temp: f32, top_p: f32, seed: u64) -> Self {
        Self {
            temp,
            top_p: top_p.clamp(0.01, 1.0),
            rng: seed | 1,
            probs: Vec::new(),
            idx: Vec::new(),
        }
    }

    pub fn greedy() -> Self {
        Self::new(0.0, 1.0, 1)
    }

    fn next_f32(&mut self) -> f32 {
        // xorshift64*
        let mut x = self.rng;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.rng = x;
        let r = x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 40;
        r as f32 / (1u64 << 24) as f32
    }

    fn argmax(logits: &[f32]) -> u32 {
        logits
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(core::cmp::Ordering::Equal))
            .map(|(i, _)| i as u32)
            .unwrap_or(0)
    }

    pub fn sample(&mut self, logits: &[f32]) -> u32 {
        if self.temp <= 0.0 || logits.len() < 2 {
            return Self::argmax(logits);
        }
        let n = logits.len();
        self.probs.clear();
        self.probs.reserve(n);
        let max = logits.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        let inv_t = 1.0 / self.temp;
        let mut sum = 0.0f32;
        for &l in logits {
            let p = libm::expf((l - max) * inv_t);
            self.probs.push(p);
            sum += p;
        }
        // índices ordenados por probabilidad descendente
        self.idx.clear();
        self.idx.extend(0..n as u32);
        let probs = &self.probs;
        self.idx.sort_unstable_by(|&a, &b| {
            probs[b as usize]
                .partial_cmp(&probs[a as usize])
                .unwrap_or(core::cmp::Ordering::Equal)
        });
        // núcleo: el prefijo más corto que acumula top_p de la masa
        let cutoff = self.top_p * sum;
        let mut mass = 0.0f32;
        let mut k = 0;
        for &i in &self.idx {
            mass += self.probs[i as usize];
            k += 1;
            if mass >= cutoff {
                break;
            }
        }
        // muestrear dentro del núcleo
        let mut r = self.next_f32() * mass;
        for &i in &self.idx[..k] {
            r -= self.probs[i as usize];
            if r <= 0.0 {
                return i;
            }
        }
        self.idx[k - 1]
    }
}

#[cfg(test)]
#[cfg(feature = "std")]
mod tests {
    use super::*;

    #[test]
    fn temp_cero_es_greedy() {
        let mut s = Sampler::greedy();
        assert_eq!(s.sample(&[0.1, 5.0, 0.3]), 1);
    }

    #[test]
    fn top_p_estrecho_elige_el_maximo() {
        // con top_p diminuto el núcleo es solo el token más probable
        let mut s = Sampler::new(1.0, 0.01, 42);
        for _ in 0..20 {
            assert_eq!(s.sample(&[0.0, 10.0, 0.0, 1.0]), 1);
        }
    }

    #[test]
    fn muestrea_solo_tokens_plausibles() {
        // dos tokens dominantes: nunca debe salir un token de logit ínfimo
        let mut s = Sampler::new(0.8, 0.9, 7);
        let mut vistos = std::collections::BTreeSet::new();
        let logits = [8.0, 8.0, -20.0, -20.0];
        for _ in 0..200 {
            let t = s.sample(&logits);
            assert!(t == 0 || t == 1, "token improbable {t}");
            vistos.insert(t);
        }
        // y con temperatura alta debe variar entre los dos
        assert_eq!(vistos.len(), 2);
    }

    #[test]
    fn determinista_con_misma_semilla() {
        let logits: Vec<f32> = (0..100).map(|i| (i % 13) as f32 * 0.3).collect();
        let mut a = Sampler::new(1.0, 0.95, 1234);
        let mut b = Sampler::new(1.0, 0.95, 1234);
        for _ in 0..50 {
            assert_eq!(a.sample(&logits), b.sample(&logits));
        }
    }
}
