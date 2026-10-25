//! Lógica de un nodo de pipeline (responde mensajes del head).

use alloc::string::String;

use crate::layer::TensorSource;
use crate::parallel::RowParallel;
use crate::pipeline::{
    AckPayload, AssignPayload, BeginPayload, ErrorPayload, HelloPayload, Message, NodeCaps,
    StepPayload, StepReplyPayload, ERR_HANDSHAKE, ERR_POS, ROLE_HEAD, ROLE_NODE,
};
use crate::runtime::Runtime;

#[derive(Clone, Debug)]
pub struct NodoConfig {
    pub layer_start: u32,
    pub layer_end: u32,
    pub fixed_layers: bool,
    pub model_name: String,
    pub manifest_crc: u32,
    pub index_crc: u32,
}

pub struct NodoPipeline {
    pub cfg: NodoConfig,
    pub session_id: u64,
    pub kv_pos: u32,
    pub prefix_hash: u32,
}

impl NodoPipeline {
    pub fn new(cfg: NodoConfig) -> Self {
        Self {
            cfg,
            session_id: 0,
            kv_pos: 0,
            prefix_hash: 0,
        }
    }

    /// Valida el `Hello` del head (sin exigir rango de capas: eso va en `Assign`).
    pub fn hello_head_valido(
        h: &HelloPayload,
        cfg: &NodoConfig,
        num_layers: u32,
        hidden_dim: u32,
    ) -> bool {
        h.role == ROLE_HEAD
            && h.num_layers == num_layers
            && h.hidden_dim == hidden_dim
            && h.model_name == cfg.model_name
            && h.manifest_crc == cfg.manifest_crc
            && h.index_crc == cfg.index_crc
    }

    pub fn hello_nodo(
        &self,
        rt: &Runtime,
        num_layers: u32,
        hidden_dim: u32,
        vocab_size: u32,
    ) -> HelloPayload {
        HelloPayload {
            role: ROLE_NODE,
            layer_start: self.cfg.layer_start,
            layer_end: self.cfg.layer_end,
            num_layers,
            hidden_dim,
            vocab_size,
            model_name: self.cfg.model_name.clone(),
            manifest_crc: self.cfg.manifest_crc,
            index_crc: self.cfg.index_crc,
            kv_pos: self.kv_pos.max(rt.pos as u32),
            prefix_hash: self.prefix_hash,
        }
    }

    pub fn error_handshake(msg: &str) -> Message {
        Message::Error(ErrorPayload {
            code: ERR_HANDSHAKE,
            message: String::from(msg),
        })
    }

    pub fn procesar_begin(&mut self, begin: BeginPayload, rt: &mut Runtime, reset_kv: bool) {
        self.session_id = begin.session_id;
        if reset_kv {
            rt.reset_sequence();
            self.kv_pos = 0;
            self.prefix_hash = 0;
        }
    }

    pub fn caps_default() -> NodeCaps {
        NodeCaps {
            ram_free_mb: 512,
            workers: 1,
            bw_score: 1000,
        }
    }

    pub fn apply_assign(&mut self, a: AssignPayload) -> Result<(), Message> {
        if self.cfg.fixed_layers {
            if a.layer_start != self.cfg.layer_start || a.layer_end != self.cfg.layer_end {
                return Err(Message::Error(ErrorPayload {
                    code: ERR_HANDSHAKE,
                    message: String::from("assign incompatible con capas fijas"),
                }));
            }
            return Ok(());
        }
        self.cfg.layer_start = a.layer_start;
        self.cfg.layer_end = a.layer_end;
        Ok(())
    }

    /// Procesa un mensaje de aplicación y devuelve la respuesta (si aplica).
    pub fn atender(
        &mut self,
        rt: &mut Runtime,
        source: &mut impl TensorSource,
        par: Option<&dyn RowParallel>,
        msg: Message,
    ) -> Result<Option<Message>, ()> {
        let h = rt.manifest.hidden_dim as usize;
        match msg {
            Message::Step(step) => {
                let reply = self.step(rt, source, par, step, h)?;
                Ok(Some(reply))
            }
            Message::Truncate(t) => {
                rt.truncar_kv(t.pos as usize);
                Ok(Some(Message::Ack(AckPayload { pos: t.pos })))
            }
            Message::Assign(a) => match self.apply_assign(a) {
                Ok(()) => Ok(Some(Message::Ack(AckPayload { pos: 0 }))),
                Err(m) => Ok(Some(m)),
            },
            Message::Reset => {
                rt.reset_sequence();
                Ok(Some(Message::Ack(AckPayload { pos: 0 })))
            }
            Message::Error(_) => Err(()),
            _ => Ok(None),
        }
    }

    fn step(
        &mut self,
        rt: &mut Runtime,
        source: &mut impl TensorSource,
        par: Option<&dyn RowParallel>,
        step: StepPayload,
        h: usize,
    ) -> Result<Message, ()> {
        let n = step.n_tokens as usize;
        if n == 0 || step.hidden.len() != n * h {
            return Err(());
        }
        if step.pos as usize != rt.pos {
            return Ok(Message::Error(ErrorPayload {
                code: ERR_POS,
                message: alloc::format!("pos desincronizada {} vs {}", step.pos, rt.pos),
            }));
        }
        self.prefix_hash = step.prefix_hash;
        let mut hs = step.hidden;
        let layer_start = self.cfg.layer_start;
        let layer_end = self.cfg.layer_end;
        let pos0 = rt.pos;
        rt.forward_bloque_rango(
            layer_start,
            layer_end,
            source,
            par,
            &mut None,
            &mut hs,
            n,
        )?;
        if rt.pos == pos0 {
            for _ in 0..n {
                rt.advance_pos();
            }
        }
        self.kv_pos = rt.pos as u32;
        Ok(Message::StepReply(StepReplyPayload {
            pos: step.pos,
            n_tokens: step.n_tokens,
            wire: step.wire,
            hidden: hs,
        }))
    }
}
