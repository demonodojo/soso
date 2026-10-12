//! Planificador por operación GPU/CPU basado en costes medidos.

use sosomodel::layout::DTYPE_F32;

pub const N_TRAMOS: usize = 3;

/// Tramo por MACs totales de la operación (`rows * cols * batch`).
#[inline]
pub fn tramo_idx(macs: u64) -> usize {
    if macs <= 256_000 {
        0
    } else if macs <= 4_000_000 {
        1
    } else {
        2
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Dest {
    #[default]
    Cpu,
    Gpu,
}

#[derive(Clone, Copy, Debug)]
pub struct OpDesc {
    pub rows: usize,
    pub cols: usize,
    pub batch: usize,
    pub resident: bool,
    pub dtype: u8,
}

impl OpDesc {
    pub fn matvec(rows: usize, cols: usize, resident: bool, dtype: u8) -> Self {
        Self {
            rows,
            cols,
            batch: 1,
            resident,
            dtype,
        }
    }

    pub fn matmul(rows: usize, cols: usize, batch: usize, resident: bool, dtype: u8) -> Self {
        Self {
            rows,
            cols,
            batch,
            resident,
            dtype,
        }
    }

    pub fn elementwise(rows: usize, cols: usize) -> Self {
        Self {
            rows,
            cols,
            batch: 1,
            resident: true,
            dtype: DTYPE_F32,
        }
    }

    pub fn macs(&self) -> u64 {
        (self.rows as u64)
            .saturating_mul(self.cols as u64)
            .saturating_mul(self.batch.max(1) as u64)
    }

    pub fn xfer_bytes(&self) -> u64 {
        let x = (self.cols as u64)
            .saturating_mul(self.batch.max(1) as u64)
            .saturating_mul(4);
        let y = (self.rows as u64)
            .saturating_mul(self.batch.max(1) as u64)
            .saturating_mul(4);
        x.saturating_add(y)
    }
}

/// Modelo de coste calibrado (ns).
#[derive(Clone, Copy, Debug)]
pub struct CostModel {
    pub gpu_fixed_ns: f64,
    pub gpu_ns_per_mac: f64,
    pub gpu_ns_per_byte: f64,
    pub cpu_ns_per_mac: f64,
    /// Subida amortizada por tensor no residente (estimación inicial).
    pub gpu_upload_ns: f64,
}

impl Default for CostModel {
    fn default() -> Self {
        Self {
            // Valores conservadores hasta calibrar: favorecen CPU en ops pequeñas.
            gpu_fixed_ns: 500_000.0,
            gpu_ns_per_mac: 0.05,
            gpu_ns_per_byte: 2.0,
            cpu_ns_per_mac: 0.8,
            gpu_upload_ns: 2_000_000.0,
        }
    }
}

pub struct OpSched {
    pub model: CostModel,
    ewma_gpu: [f64; N_TRAMOS],
    ewma_cpu: [f64; N_TRAMOS],
    n_gpu: [u64; N_TRAMOS],
    n_cpu: [u64; N_TRAMOS],
    n: u64,
}

impl Default for OpSched {
    fn default() -> Self {
        Self::new(CostModel::default())
    }
}

impl OpSched {
    pub fn new(model: CostModel) -> Self {
        Self {
            model,
            ewma_gpu: [0.0; N_TRAMOS],
            ewma_cpu: [0.0; N_TRAMOS],
            n_gpu: [0; N_TRAMOS],
            n_cpu: [0; N_TRAMOS],
            n: 0,
        }
    }

    pub fn estimate_gpu_ns(&self, op: &OpDesc) -> f64 {
        let macs = op.macs() as f64;
        let mut t = self.model.gpu_fixed_ns
            + macs * self.model.gpu_ns_per_mac
            + op.xfer_bytes() as f64 * self.model.gpu_ns_per_byte;
        if !op.resident {
            t += self.model.gpu_upload_ns;
        }
        t
    }

    pub fn estimate_cpu_ns(&self, op: &OpDesc) -> f64 {
        op.macs() as f64 * self.model.cpu_ns_per_mac
    }

    fn ewma_sample(_tramo: usize, slot: &mut f64, sample_ns: f64) -> f64 {
        if *slot <= 0.0 {
            *slot = sample_ns;
        } else {
            *slot = 0.125 * sample_ns + 0.875 * *slot;
        }
        *slot
    }

    pub fn elegir(&mut self, op: &OpDesc, gpu_disponible: bool) -> Dest {
        if !gpu_disponible {
            return Dest::Cpu;
        }
        self.n = self.n.wrapping_add(1);
        let tr = tramo_idx(op.macs());
        let t_gpu = if self.ewma_gpu[tr] > 0.0 {
            self.ewma_gpu[tr]
        } else {
            self.estimate_gpu_ns(op)
        };
        let t_cpu = if self.ewma_cpu[tr] > 0.0 {
            self.ewma_cpu[tr]
        } else {
            self.estimate_cpu_ns(op)
        };
        let prefer_gpu = t_gpu <= t_cpu * 0.8;
        let prefer_cpu = t_cpu <= t_gpu * 0.8;
        let dest = if prefer_gpu {
            Dest::Gpu
        } else if prefer_cpu {
            Dest::Cpu
        } else if t_gpu <= t_cpu {
            Dest::Gpu
        } else {
            Dest::Cpu
        };
        // Exploración: 1/64 al destino no elegido para no quedarse ciego.
        if self.n % 64 == 0 {
            return match dest {
                Dest::Gpu => Dest::Cpu,
                Dest::Cpu => Dest::Gpu,
            };
        }
        dest
    }

    pub fn registrar(&mut self, op: &OpDesc, dest: Dest, ns: u64) {
        let tr = tramo_idx(op.macs());
        let nsf = ns.max(1) as f64;
        match dest {
            Dest::Gpu => {
                Self::ewma_sample(tr, &mut self.ewma_gpu[tr], nsf);
                self.n_gpu[tr] = self.n_gpu[tr].wrapping_add(1);
            }
            Dest::Cpu => {
                Self::ewma_sample(tr, &mut self.ewma_cpu[tr], nsf);
                self.n_cpu[tr] = self.n_cpu[tr].wrapping_add(1);
            }
        }
    }

    pub fn ewma_gpu(&self, tramo: usize) -> f64 {
        self.ewma_gpu.get(tramo).copied().unwrap_or(0.0)
    }

    pub fn ewma_cpu(&self, tramo: usize) -> f64 {
        self.ewma_cpu.get(tramo).copied().unwrap_or(0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pequeno_va_a_cpu() {
        let mut sched = OpSched::default();
        let op = OpDesc::matvec(384, 384, true, DTYPE_F32);
        assert_eq!(sched.elegir(&op, true), Dest::Cpu);
    }

    #[test]
    fn grande_va_a_gpu() {
        let mut sched = OpSched::new(CostModel {
            gpu_fixed_ns: 100_000.0,
            gpu_ns_per_mac: 0.01,
            gpu_ns_per_byte: 0.5,
            cpu_ns_per_mac: 1.0,
            gpu_upload_ns: 0.0,
        });
        let op = OpDesc::matmul(4096, 4096, 1, true, DTYPE_F32);
        assert_eq!(sched.elegir(&op, true), Dest::Gpu);
    }

    #[test]
    fn batch_grande_va_a_gpu() {
        let mut sched = OpSched::new(CostModel {
            gpu_fixed_ns: 200_000.0,
            gpu_ns_per_mac: 0.02,
            gpu_ns_per_byte: 0.5,
            cpu_ns_per_mac: 0.5,
            gpu_upload_ns: 0.0,
        });
        let op = OpDesc::matmul(384, 384, 1500, true, DTYPE_F32);
        assert_eq!(sched.elegir(&op, true), Dest::Gpu);
    }

    #[test]
    fn registrar_mueve_decision() {
        let mut sched = OpSched::default();
        let op = OpDesc::matvec(384, 384, true, DTYPE_F32);
        for _ in 0..20 {
            sched.registrar(&op, Dest::Gpu, 50_000);
        }
        assert!(sched.ewma_gpu(0) < sched.estimate_cpu_ns(&op));
    }
}
