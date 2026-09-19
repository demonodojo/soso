//! Informe de generación, observador y cancelación cooperativa (T08–T09, C2).

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
    /// Cancelación solicitada por el observador.
    Cancelled,
}

/// Punto del bucle donde el observador puede pedir parar (T09).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GenCheckpoint {
    BeforePrefill,
    PrefillToken { done: usize, total: usize },
    Layer { layer: u32, of: u32 },
    BeforeDecode,
    BeforeSample,
}

/// Recibe tokens emitidos y puede cancelar en checkpoints cooperativos.
pub trait GenerationObserver {
    fn on_token(&mut self, _token: u32) {}
    /// `true` = cancelar lo antes posible; no se emitirán más tokens.
    fn cancel_at(&mut self, at: GenCheckpoint) -> bool;
}

/// Observador por defecto: nunca cancela (adaptadores antiguos).
pub struct NoCancelObserver;

impl GenerationObserver for NoCancelObserver {
    fn cancel_at(&mut self, _: GenCheckpoint) -> bool {
        false
    }
}

/// Enlaza los callbacks `on_token` / progreso de prefill de la API previa.
pub struct LegacyStreamObserver<F, G> {
    pub on_token: F,
    pub on_prefill: G,
}

impl<F, G> GenerationObserver for LegacyStreamObserver<F, G>
where
    F: FnMut(u32),
    G: FnMut(usize, usize),
{
    fn on_token(&mut self, token: u32) {
        (self.on_token)(token);
    }

    fn cancel_at(&mut self, at: GenCheckpoint) -> bool {
        match at {
            GenCheckpoint::PrefillToken { done, total } => {
                (self.on_prefill)(done, total);
            }
            _ => {}
        }
        false
    }
}

/// Bandera de parada cooperativa (no presta el ledger para evitar conflictos de borrow).
#[derive(Debug, Clone, Copy, Default)]
pub struct GenCancel {
    pub stopped: bool,
}

impl GenCancel {
    pub fn checkpoint(
        &mut self,
        observer: &mut dyn GenerationObserver,
        ledger: &mut GenerationLedger,
        at: GenCheckpoint,
    ) -> bool {
        if self.stopped {
            return true;
        }
        if observer.cancel_at(at) {
            ledger.note_cancelled();
            self.stopped = true;
            true
        } else {
            false
        }
    }
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
        observer: &mut dyn GenerationObserver,
    ) -> bool {
        self.sampled_tokens += 1;
        if stops.contains(&token) {
            self.stop = Some(StopReason::StopToken(token));
            return false;
        }
        tokens.push(token);
        observer.on_token(token);
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
        struct Obs<'b>(&'b mut alloc::vec::Vec<u32>);
        impl GenerationObserver for Obs<'_> {
            fn on_token(&mut self, t: u32) {
                self.0.push(t);
            }
            fn cancel_at(&mut self, _: GenCheckpoint) -> bool {
                false
            }
        }
        let mut obs = Obs(&mut emitidos);
        assert!(ledger.accept_sample(10, &[], &mut tokens, &mut obs));
        assert!(!ledger.accept_sample(99, &[99], &mut tokens, &mut obs));
        let report = ledger.into_report();
        assert_eq!(emitidos, alloc::vec![10]);
        assert_eq!(tokens, alloc::vec![7, 10]);
        assert_eq!(report.generated, 1);
        assert_eq!(report.sampled_tokens, 2);
        assert_eq!(report.stop, StopReason::StopToken(99));
    }
}
