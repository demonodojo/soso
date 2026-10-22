//! Tramo remoto del pipeline: abstracción compartida entre guest TCP y tests loopback.

use alloc::vec::Vec;

use crate::f16;
use crate::pipeline::{WIRE_F16, WIRE_F32};

pub use crate::pipeline::prefix_hash_tokens;

/// Puente de capas remotas invocado desde el head durante la generación.
pub trait TramoRemoto {
    /// Aplica el tramo remoto sobre `n` vectores contiguos (`n * hidden_dim` en `hs`).
    fn forward(
        &mut self,
        pos: usize,
        hs: &mut [f32],
        n: usize,
        hidden_dim: usize,
        prefix_hash: u32,
    ) -> Result<(), ()>;
    /// Recorta el KV remoto a `pos` tokens.
    fn truncar(&mut self, pos: usize) -> Result<(), ()>;
    /// Reinicia estado remoto (nueva sesión).
    fn reiniciar(&mut self) -> Result<(), ()>;
    fn wire_format(&self) -> u8 {
        WIRE_F32
    }
    /// Telemetría opcional: bytes enviados en el último salto.
    fn ultimo_bytes(&self) -> u64 {
        0
    }
    fn ultimo_ms(&self) -> u64 {
        0
    }
    /// Prefill: encola un bloque remoto (pipelining); por defecto no-op.
    fn encolar(
        &mut self,
        pos: usize,
        hs: &mut [f32],
        n: usize,
        hidden_dim: usize,
        prefix_hash: u32,
    ) -> Result<(), ()> {
        let _ = (pos, hs, n, hidden_dim, prefix_hash);
        Ok(())
    }
    /// Drena respuestas encoladas; por defecto no-op.
    fn vaciar(&mut self, _hidden_dim: usize) -> Result<(), ()> {
        Ok(())
    }
}

/// Contexto de cluster pasado a la generación planificada.
pub struct ClusterGen<'a> {
    pub local_end: u32,
    pub remoto: &'a mut dyn TramoRemoto,
    pub journal: Option<&'a mut JournalActivaciones>,
    /// Hash FNV de los tokens de contexto hasta el bloque remoto actual.
    pub prefix_hash: u32,
}

/// Journal de activaciones en el head (reconstrucción de KV tras caída de un nodo).
pub struct JournalActivaciones {
    wire: u8,
    entradas: Vec<EntradaJournal>,
}

struct EntradaJournal {
    pos: u32,
    n: u32,
    hidden: Vec<u8>,
}

impl JournalActivaciones {
    pub fn new(wire: u8) -> Self {
        Self {
            wire,
            entradas: Vec::new(),
        }
    }

    pub fn registrar(&mut self, pos: u32, n: u32, hidden: &[f32]) {
        let bytes = if self.wire == WIRE_F16 {
            hidden
                .iter()
                .flat_map(|v| f16::f32_to_f16(*v).to_le_bytes())
                .collect()
        } else {
            hidden.iter().flat_map(|v| v.to_le_bytes()).collect()
        };
        self.entradas.push(EntradaJournal { pos, n, hidden: bytes });
    }

    pub fn truncar(&mut self, pos: u32) {
        self.entradas.retain(|e| e.pos.saturating_add(e.n) <= pos);
    }

    /// Copia de entradas decodificadas a f32 (replay por enlace).
    pub fn entradas_snapshot(&self) -> Vec<(u32, u32, Vec<f32>)> {
        self.entradas
            .iter()
            .map(|e| {
                let hs = if self.wire == WIRE_F16 {
                    e.hidden
                        .chunks_exact(2)
                        .map(|c| f16::f16_to_f32(u16::from_le_bytes(c.try_into().unwrap())))
                        .collect()
                } else {
                    e.hidden
                        .chunks_exact(4)
                        .map(|c| f32::from_le_bytes(c.try_into().unwrap()))
                        .collect()
                };
                (e.pos, e.n, hs)
            })
            .collect()
    }

    pub fn replay(
        &self,
        remoto: &mut dyn TramoRemoto,
        hidden_dim: usize,
    ) -> Result<(), ()> {
        for e in &self.entradas {
            let mut hs = if self.wire == WIRE_F16 {
                e.hidden
                    .chunks_exact(2)
                    .map(|c| f16::f16_to_f32(u16::from_le_bytes(c.try_into().unwrap())))
                    .collect::<Vec<_>>()
            } else {
                e.hidden
                    .chunks_exact(4)
                    .map(|c| f32::from_le_bytes(c.try_into().unwrap()))
                    .collect()
            };
            remoto.forward(e.pos as usize, &mut hs, e.n as usize, hidden_dim, 0)?;
        }
        Ok(())
    }
}
