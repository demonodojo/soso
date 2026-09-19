//! Informe de generación y contadores (T08, contrato C2).

use alloc::vec::Vec;

/// Por qué terminó el bucle de decode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    /// Se muestreó un id de parada del perfil; no se decodifica ni se emite.
    StopToken(u32),
    /// Se alcanzó `max_new` sin stop.
    Limit,
    /// La secuencia llegó a `max_seq` del manifiesto.
    ContextLimit,
    /// Cancelación externa (p. ej. observer); reservado para integraciones futuras.
    Cancelled,
}

/// Estadísticas al cerrar una generación.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GenerationReport {
    /// Tokens nuevos emitidos al cliente (sin stop).
    pub generated: u32,
    pub prompt_tokens: u32,
    /// Cada muestreo cuenta, incluido un stop consumido.
    pub sampled_tokens: u32,
    pub stop: StopReason,
}

/// Parámetros de la variante con informe.
#[derive(Debug, Clone)]
pub struct GenerationOptions {
    pub max_new: usize,
    pub stop_token_ids: Vec<u32>,
}

impl GenerationOptions {
    pub fn new(max_new: usize, stop_token_ids: Vec<u32>) -> Self {
        Self {
            max_new,
            stop_token_ids,
        }
    }

    pub fn from_eos(max_new: usize, eos: Option<u32>) -> Self {
        Self {
            max_new,
            stop_token_ids: stops_from_eos(eos),
        }
    }
}

pub fn stops_from_eos(eos: Option<u32>) -> Vec<u32> {
    eos.map(|t| alloc::vec![t]).unwrap_or_default()
}

/// Contadores compartidos por los caminos normal, paralelo, planned y drafts.
#[derive(Debug, Clone, Copy)]
pub struct GenerationLedger {
    prompt_tokens: u32,
    generated: u32,
    sampled_tokens: u32,
    stop: Option<StopReason>,
}

impl GenerationLedger {
    pub fn new(prompt_len: usize) -> Self {
        Self {
            prompt_tokens: prompt_len as u32,
            generated: 0,
            sampled_tokens: 0,
            stop: None,
        }
    }

    pub fn note_context_limit(&mut self) {
        if self.stop.is_none() {
            self.stop = Some(StopReason::ContextLimit);
        }
    }

    pub fn note_cancelled(&mut self) {
        self.stop = Some(StopReason::Cancelled);
    }

    /// Registra un token muestreado. Devuelve `false` si es stop (no emitir).
    pub fn accept_sample(
        &mut self,
        token: u32,
        stops: &[u32],
        tokens: &mut Vec<u32>,
        on_token: &mut impl FnMut(u32),
    ) -> bool {
        self.sampled_tokens += 1;
        if stops.contains(&token) {
            self.stop = Some(StopReason::StopToken(token));
            return false;
        }
        tokens.push(token);
        on_token(token);
        self.generated += 1;
        true
    }

    pub fn into_report(self) -> GenerationReport {
        GenerationReport {
            generated: self.generated,
            prompt_tokens: self.prompt_tokens,
            sampled_tokens: self.sampled_tokens,
            stop: self.stop.unwrap_or(StopReason::Limit),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stop_tras_un_token_emitido() {
        let mut ledger = GenerationLedger::new(1);
        let mut tokens = alloc::vec![7u32];
        let mut emitidos = alloc::vec::Vec::new();
        let mut on = |t: u32| emitidos.push(t);
        assert!(ledger.accept_sample(10, &[], &mut tokens, &mut on));
        assert!(!ledger.accept_sample(99, &[99], &mut tokens, &mut on));
        let report = ledger.into_report();
        assert_eq!(emitidos, alloc::vec![10]);
        assert_eq!(tokens, alloc::vec![7, 10]);
        assert_eq!(report.generated, 1);
        assert_eq!(report.sampled_tokens, 2);
        assert_eq!(report.stop, StopReason::StopToken(99));
    }
}
