//! Head en estrella: enlaces remotos, journal y telemetría.

use alloc::string::String;
use alloc::vec::Vec;

use crate::pipeline::{
    AckPayload, AssignPayload, BeginPayload, FramedTransport, HelloPayload, Message, NodeCaps,
    PipelinePlan, PipelineSegment, RecvError, StepPayload, StepReplyPayload, TruncatePayload,
    Transport, NowMs, ROLE_HEAD, ROLE_NODE, WIRE_F16,
};
use crate::remoto::{JournalActivaciones, TramoRemoto};

#[derive(Clone, Debug)]
pub struct EstrellaConfig {
    pub wire: u8,
    pub step_timeout_ms: u64,
    pub handshake_timeout_ms: u64,
    pub ping_interval_ms: u64,
    pub ping_idle_ms: u64,
    pub standby_retry_ms: u64,
    pub retry_max: u32,
    /// Si es false, el `Hello` del nodo no tiene que traer el rango final (modo `--auto`).
    pub strict_node_layers: bool,
    pub model_name: String,
    pub manifest_crc: u32,
    pub index_crc: u32,
}

impl Default for EstrellaConfig {
    fn default() -> Self {
        Self {
            wire: 0,
            step_timeout_ms: crate::pipeline::DEFAULT_STEP_TIMEOUT_MS,
            handshake_timeout_ms: crate::pipeline::DEFAULT_HANDSHAKE_TIMEOUT_MS,
            ping_interval_ms: crate::pipeline::DEFAULT_PING_INTERVAL_MS,
            ping_idle_ms: crate::pipeline::DEFAULT_PING_IDLE_MS,
            standby_retry_ms: crate::pipeline::DEFAULT_STANDBY_RETRY_MS,
            retry_max: 3,
            strict_node_layers: true,
            model_name: String::new(),
            manifest_crc: 0,
            index_crc: 0,
        }
    }
}

pub struct HandshakeCaps {
    pub caps: NodeCaps,
}

pub struct EnlaceRemoto<T: Transport> {
    pub label: String,
    pub tx: FramedTransport<T>,
    pub caps: NodeCaps,
}

/// Conexión tras Hello+Caps, antes de `Assign` (reparto `--splits auto`).
pub struct PendingEnlace<T: Transport> {
    pub addr: String,
    pub tx: FramedTransport<T>,
    pub caps: NodeCaps,
}

struct InflightStep {
    pos: usize,
    n: usize,
    prefix_hash: u32,
    hidden: Vec<f32>,
}

pub trait Conector {
    type T: Transport;
    fn conectar(&mut self, addr: &str, timeout_ms: u64) -> Result<FramedTransport<Self::T>, ()>;
}

pub struct EstrellaHead<C: Conector> {
    pub cfg: EstrellaConfig,
    pub plan: PipelinePlan,
    pub links: Vec<EnlaceRemoto<C::T>>,
    conector: C,
    hidden_dim: u32,
    num_layers: u32,
    journals: Vec<JournalActivaciones>,
    ultimo_bytes: u64,
    ultimo_ms: u64,
    now_ms: NowMs,
    rtt_hook: Option<fn(u64)>,
    sleep_ms: Option<fn(u64)>,
    last_begin: Option<BeginPayload>,
    inflight: Vec<Option<InflightStep>>,
}

impl<C: Conector> EstrellaHead<C> {
    pub fn new(
        cfg: EstrellaConfig,
        plan: PipelinePlan,
        conector: C,
        hidden_dim: u32,
        num_layers: u32,
        now_ms: NowMs,
    ) -> Self {
        Self {
            cfg,
            plan,
            links: Vec::new(),
            conector,
            hidden_dim,
            num_layers,
            journals: Vec::new(),
            ultimo_bytes: 0,
            ultimo_ms: 0,
            now_ms,
            rtt_hook: None,
            sleep_ms: None,
            last_begin: None,
            inflight: Vec::new(),
        }
    }

    pub fn set_rtt_hook(&mut self, hook: fn(u64)) {
        self.rtt_hook = Some(hook);
    }

    pub fn set_sleep_ms(&mut self, sleep: fn(u64)) {
        self.sleep_ms = Some(sleep);
    }

    pub fn set_plan(&mut self, plan: PipelinePlan) {
        self.plan = plan;
    }

    pub fn ultimo_ms(&self) -> u64 {
        self.ultimo_ms
    }

    pub fn ultimo_bytes(&self) -> u64 {
        self.ultimo_bytes
    }

    pub fn journal_mut(&mut self, idx: usize) -> Option<&mut JournalActivaciones> {
        self.journals.get_mut(idx)
    }

    fn sync_journals(&mut self) {
        while self.journals.len() < self.links.len() {
            self.journals
                .push(JournalActivaciones::new(self.cfg.wire));
        }
        while self.inflight.len() < self.links.len() {
            self.inflight.push(None);
        }
    }

    fn hello_probe(&self, num_layers: u32) -> HelloPayload {
        HelloPayload {
            role: ROLE_HEAD,
            layer_start: 0,
            layer_end: num_layers,
            num_layers: self.num_layers,
            hidden_dim: self.hidden_dim,
            vocab_size: 0,
            model_name: self.cfg.model_name.clone(),
            manifest_crc: self.cfg.manifest_crc,
            index_crc: self.cfg.index_crc,
            kv_pos: 0,
            prefix_hash: 0,
        }
    }

    /// Hello → Hello+Caps sin `Assign` (misma conexión para el reparto auto).
    pub fn recolectar_caps(
        &mut self,
        addrs: &[String],
    ) -> Result<Vec<PendingEnlace<C::T>>, ()> {
        let hs_to = self.cfg.handshake_timeout_ms;
        let mut out = Vec::with_capacity(addrs.len());
        for addr in addrs {
            let mut tx = self.conector.conectar(addr, hs_to)?;
            tx.send(&Message::Hello(self.hello_probe(self.num_layers)))?;
            let caps = Self::recv_node_hello_caps(
                &mut tx,
                &self.cfg,
                None,
                self.num_layers,
                hs_to,
                self.now_ms,
            )?;
            out.push(PendingEnlace {
                addr: addr.clone(),
                tx,
                caps,
            });
        }
        Ok(out)
    }

    /// Completa `Assign` sobre enlaces ya abiertos (tras fijar `plan`).
    pub fn assignar_pending(
        &mut self,
        pending: Vec<PendingEnlace<C::T>>,
        kv_pos: u32,
        prefix_hash: u32,
    ) -> Result<(), ()> {
        if pending.len() != self.plan.remote_count() {
            return Err(());
        }
        self.links.clear();
        let hs_to = self.cfg.handshake_timeout_ms;
        for (i, mut p) in pending.into_iter().enumerate() {
            let seg = self.plan.remote_segment(i).ok_or(())?;
            let _ = (kv_pos, prefix_hash);
            p.tx.send(&Message::Assign(AssignPayload {
                layer_start: seg.layer_start,
                layer_end: seg.layer_end,
            }))?;
            match Self::recv(&mut p.tx, &self.cfg, hs_to, self.now_ms, "ack assign")? {
                Message::Ack { .. } | Message::Reset => {}
                _ => return Err(()),
            }
            self.links.push(EnlaceRemoto {
                label: p.addr,
                tx: p.tx,
                caps: p.caps,
            });
        }
        self.sync_journals();
        Ok(())
    }

    fn recv(
        tx: &mut FramedTransport<C::T>,
        cfg: &EstrellaConfig,
        timeout_ms: u64,
        now_ms: NowMs,
        _context: &str,
    ) -> Result<Message, ()> {
        let result = if cfg.ping_interval_ms != 0 {
            tx.recv_timeout_keepalive(
                timeout_ms,
                cfg.ping_interval_ms,
                cfg.ping_idle_ms,
                now_ms,
            )
        } else {
            tx.recv_timeout(timeout_ms, now_ms)
        };
        result.map_err(|_| ())
    }

    /// Handshake v4: Hello → Hello+Caps → Assign → Ack (por enlace).
    pub fn enlazar(
        &mut self,
        addrs: &[String],
        kv_pos: u32,
        prefix_hash: u32,
    ) -> Result<(), ()> {
        if addrs.len() != self.plan.remote_count() {
            return Err(());
        }
        self.links.clear();
        let hs_to = self.cfg.handshake_timeout_ms;
        for (i, addr) in addrs.iter().enumerate() {
            let seg = self.plan.remote_segment(i).ok_or(())?;
            let mut tx = self.conector.conectar(addr, hs_to)?;
            let hello = HelloPayload {
                role: ROLE_HEAD,
                layer_start: seg.layer_start,
                layer_end: seg.layer_end,
                num_layers: self.num_layers,
                hidden_dim: self.hidden_dim,
                vocab_size: 0,
                model_name: self.cfg.model_name.clone(),
                manifest_crc: self.cfg.manifest_crc,
                index_crc: self.cfg.index_crc,
                kv_pos,
                prefix_hash,
            };
            tx.send(&Message::Hello(hello))?;
            let caps = Self::recv_node_hello_caps(
                &mut tx,
                &self.cfg,
                Some(seg),
                self.num_layers,
                hs_to,
                self.now_ms,
            )?;
            tx.send(&Message::Assign(AssignPayload {
                layer_start: seg.layer_start,
                layer_end: seg.layer_end,
            }))?;
            match Self::recv(&mut tx, &self.cfg, hs_to, self.now_ms, "ack assign")? {
                Message::Ack { .. } | Message::Reset => {}
                _ => return Err(()),
            }
            self.links.push(EnlaceRemoto {
                label: addr.clone(),
                tx,
                caps,
            });
        }
        self.sync_journals();
        Ok(())
    }

    fn recv_node_hello_caps(
        tx: &mut FramedTransport<C::T>,
        cfg: &EstrellaConfig,
        seg: Option<PipelineSegment>,
        num_layers: u32,
        hs_to: u64,
        now_ms: NowMs,
    ) -> Result<NodeCaps, ()> {
        let mut have_hello = false;
        let mut caps = NodeCaps {
            ram_free_mb: 512,
            workers: 1,
            bw_score: 1000,
        };
        for _ in 0..4 {
            match Self::recv(tx, cfg, hs_to, now_ms, "handshake")? {
                Message::Hello(h) => {
                    if h.role != ROLE_NODE || h.num_layers != num_layers {
                        return Err(());
                    }
                    if cfg.strict_node_layers {
                        if let Some(seg) = seg {
                            if h.layer_start != seg.layer_start || h.layer_end != seg.layer_end {
                                return Err(());
                            }
                        }
                    }
                    have_hello = true;
                    if let Ok(Some(Message::Caps(c))) = tx.try_recv() {
                        caps = c;
                    }
                    break;
                }
                Message::Caps(c) => caps = c,
                _ => return Err(()),
            }
        }
        if !have_hello {
            return Err(());
        }
        Ok(caps)
    }

    fn replay_journal_link(&mut self, idx: usize, hidden_dim: usize) -> Result<(), ()> {
        let Some(journal) = self.journals.get(idx) else {
            return Ok(());
        };
        let entries: Vec<(u32, u32, Vec<f32>)> = journal
            .entradas_snapshot()
            .into_iter()
            .map(|(p, n, hs)| (p, n, hs))
            .collect();
        for (pos, n, mut hs) in entries {
            self.forward_link(idx, pos as usize, &mut hs, n as usize, hidden_dim, 0)?;
        }
        Ok(())
    }

    fn reconectar_y_replay(
        &mut self,
        idx: usize,
        kv_pos: u32,
        prefix_hash: u32,
    ) -> Result<(), ()> {
        let addr = self.links.get(idx).map(|l| l.label.clone()).ok_or(())?;
        let begin = self.last_begin.ok_or(())?;
        self.reconectar_enlace(idx, &addr, kv_pos, prefix_hash, begin)?;
        self.replay_journal_link(idx, self.hidden_dim as usize)
    }

    /// Tras caída del enlace `idx`: reconecta y reproduce el journal de ese salto.
    pub fn reconectar_enlace(
        &mut self,
        idx: usize,
        addr: &str,
        kv_pos: u32,
        prefix_hash: u32,
        begin: BeginPayload,
    ) -> Result<(), ()> {
        let seg = self.plan.remote_segment(idx).ok_or(())?;
        let hs_to = self.cfg.handshake_timeout_ms;
        let mut tx = self.conector.conectar(addr, hs_to)?;
        let hello = HelloPayload {
            role: ROLE_HEAD,
            layer_start: seg.layer_start,
            layer_end: seg.layer_end,
            num_layers: self.num_layers,
            hidden_dim: self.hidden_dim,
            vocab_size: 0,
            model_name: self.cfg.model_name.clone(),
            manifest_crc: self.cfg.manifest_crc,
            index_crc: self.cfg.index_crc,
            kv_pos,
            prefix_hash,
        };
        tx.send(&Message::Hello(hello))?;
        let caps = Self::recv_node_hello_caps(
            &mut tx,
            &self.cfg,
            Some(seg),
            self.num_layers,
            hs_to,
            self.now_ms,
        )?;
        tx.send(&Message::Assign(AssignPayload {
            layer_start: seg.layer_start,
            layer_end: seg.layer_end,
        }))?;
        match Self::recv(&mut tx, &self.cfg, hs_to, self.now_ms, "ack assign")? {
            Message::Ack { .. } | Message::Reset => {}
            _ => return Err(()),
        }
        tx.send(&Message::Begin(begin))?;
        match Self::recv(&mut tx, &self.cfg, hs_to, self.now_ms, "ack begin")? {
            Message::Ack { .. } | Message::Reset => {}
            _ => return Err(()),
        }
        if let Some(link) = self.links.get_mut(idx) {
            link.tx = tx;
            link.caps = caps;
            link.label = String::from(addr);
        }
        Ok(())
    }

    pub fn begin_session(&mut self, begin: BeginPayload) -> Result<(), ()> {
        self.last_begin = Some(begin);
        let hs_to = self.cfg.handshake_timeout_ms;
        for link in self.links.iter_mut() {
            link.tx.send(&Message::Begin(begin))?;
        }
        for link in self.links.iter_mut() {
            match Self::recv(&mut link.tx, &self.cfg, hs_to, self.now_ms, "ack begin")? {
                Message::Ack { .. } | Message::Reset => {}
                _ => return Err(()),
            }
        }
        Ok(())
    }

    fn forward_link(
        &mut self,
        idx: usize,
        pos: usize,
        hs: &mut [f32],
        n: usize,
        h: usize,
        prefix_hash: u32,
    ) -> Result<(), ()> {
        let t0 = (self.now_ms)();
        let wire = self.cfg.wire;
        let step_to = self.cfg.step_timeout_ms;
        let cfg = self.cfg.clone();
        let link = self.links.get_mut(idx).ok_or(())?;
        link.tx.send(&Message::Step(StepPayload {
            pos: pos as u32,
            prefix_hash,
            n_tokens: n as u32,
            wire,
            hidden: hs.to_vec(),
        }))?;
        match Self::recv(&mut link.tx, &cfg, step_to, self.now_ms, "step")? {
            Message::StepReply(r) => {
                if r.pos != pos as u32 || r.n_tokens != n as u32 {
                    return Err(());
                }
                hs.copy_from_slice(&r.hidden[..n * h]);
                let bpe = if wire == WIRE_F16 { 2 } else { 4 };
                self.ultimo_bytes = hs.len() as u64 * bpe;
                let ms = (self.now_ms)().saturating_sub(t0);
                self.ultimo_ms = ms;
                if let Some(hook) = self.rtt_hook {
                    hook(ms);
                }
                Ok(())
            }
            _ => Err(()),
        }
    }
}

impl<C: Conector> TramoRemoto for EstrellaHead<C> {
    fn forward(
        &mut self,
        pos: usize,
        hs: &mut [f32],
        n: usize,
        hidden_dim: usize,
        prefix_hash: u32,
    ) -> Result<(), ()> {
        let t0 = (self.now_ms)();
        let mut hidden = hs[..n * hidden_dim].to_vec();
        let remote_count = self.links.len();
        for i in 0..remote_count {
            if let Some(j) = self.journals.get_mut(i) {
                j.registrar(pos as u32, n as u32, &hidden);
            }
            let mut attempt = 0u32;
            loop {
                match self.forward_link(i, pos, &mut hidden, n, hidden_dim, prefix_hash) {
                    Ok(()) => break,
                    Err(()) if attempt < self.cfg.retry_max => {
                        attempt += 1;
                        if let Some(sleep) = self.sleep_ms {
                            sleep(self.cfg.standby_retry_ms);
                        }
                        let _ = self.reconectar_y_replay(
                            i,
                            pos as u32,
                            prefix_hash,
                        );
                        continue;
                    }
                    Err(()) => return Err(()),
                }
            }
            if i + 1 == remote_count {
                break;
            }
        }
        hs[..n * hidden_dim].copy_from_slice(&hidden);
        self.ultimo_ms = (self.now_ms)().saturating_sub(t0);
        Ok(())
    }

    fn truncar(&mut self, pos: usize) -> Result<(), ()> {
        for j in self.journals.iter_mut() {
            j.truncar(pos as u32);
        }
        let cfg = self.cfg.clone();
        let step_to = self.cfg.step_timeout_ms;
        for link in self.links.iter_mut() {
            link.tx
                .send(&Message::Truncate(TruncatePayload { pos: pos as u32 }))?;
            match Self::recv(&mut link.tx, &cfg, step_to, self.now_ms, "truncate")? {
                Message::Ack { .. } | Message::Reset => {}
                _ => return Err(()),
            }
        }
        Ok(())
    }

    fn encolar(
        &mut self,
        pos: usize,
        hs: &mut [f32],
        n: usize,
        hidden_dim: usize,
        prefix_hash: u32,
    ) -> Result<(), ()> {
        let remote_count = self.links.len();
        if remote_count == 0 {
            return Ok(());
        }
        let mut hidden = hs[..n * hidden_dim].to_vec();
        for i in 0..remote_count {
            if let Some(j) = self.journals.get_mut(i) {
                j.registrar(pos as u32, n as u32, &hidden);
            }
            let wire = self.cfg.wire;
            let link = self.links.get_mut(i).ok_or(())?;
            link.tx.send(&Message::Step(StepPayload {
                pos: pos as u32,
                prefix_hash,
                n_tokens: n as u32,
                wire,
                hidden: hidden.clone(),
            }))?;
            if i + 1 == remote_count {
                self.inflight[i] = Some(InflightStep {
                    pos,
                    n,
                    prefix_hash,
                    hidden: hidden.clone(),
                });
                break;
            }
            let cfg = self.cfg.clone();
            let step_to = self.cfg.step_timeout_ms;
            match Self::recv(&mut link.tx, &cfg, step_to, self.now_ms, "step encolar")? {
                Message::StepReply(r) => {
                    if r.pos != pos as u32 || r.n_tokens != n as u32 {
                        return Err(());
                    }
                    hidden.copy_from_slice(&r.hidden[..n * hidden_dim]);
                }
                _ => return Err(()),
            }
        }
        hs[..n * hidden_dim].copy_from_slice(&hidden);
        Ok(())
    }

    fn vaciar(&mut self, hidden_dim: usize) -> Result<(), ()> {
        let cfg = self.cfg.clone();
        let step_to = self.cfg.step_timeout_ms;
        for (i, slot) in self.inflight.iter_mut().enumerate() {
            if slot.is_none() {
                continue;
            }
            let link = self.links.get_mut(i).ok_or(())?;
            match Self::recv(&mut link.tx, &cfg, step_to, self.now_ms, "vaciar")? {
                Message::StepReply(r) => {
                    let pending = slot.take().ok_or(())?;
                    if r.pos != pending.pos as u32 || r.n_tokens != pending.n as u32 {
                        return Err(());
                    }
                    let _ = (hidden_dim, pending);
                }
                _ => return Err(()),
            }
        }
        Ok(())
    }

    fn reiniciar(&mut self) -> Result<(), ()> {
        for j in self.journals.iter_mut() {
            *j = JournalActivaciones::new(self.cfg.wire);
        }
        for s in self.inflight.iter_mut() {
            *s = None;
        }
        let cfg = self.cfg.clone();
        let step_to = self.cfg.step_timeout_ms;
        for link in self.links.iter_mut() {
            link.tx.send(&Message::Reset)?;
            match Self::recv(&mut link.tx, &cfg, step_to, self.now_ms, "ack reset")? {
                Message::Ack { .. } | Message::Reset => {}
                _ => return Err(()),
            }
        }
        Ok(())
    }

    fn wire_format(&self) -> u8 {
        self.cfg.wire
    }

    fn ultimo_bytes(&self) -> u64 {
        self.ultimo_bytes
    }

    fn ultimo_ms(&self) -> u64 {
        self.ultimo_ms
    }
}
