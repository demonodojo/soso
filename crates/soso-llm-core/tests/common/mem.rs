//! Conector en memoria: enlaza `EstrellaHead` con `NodoPipeline` sin TCP.

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;

use soso_llm_core::cluster::{Conector, NodoConfig, NodoPipeline};
use soso_llm_core::pipeline::{
    AckPayload, ErrorPayload, FramedTransport, HelloPayload, Message, NodeCaps, PipelineRole,
    StepPayload, Transport, ERR_NODE_DOWN, ROLE_HEAD,
};
use soso_llm_core::runtime::Runtime;
use soso_llm_core::source::MmapTensorSource;
use soso_llm_core::source::host::MemFileMapper;
use sosomodel::index::TensorIndex;
use sosomodel::manifest::Manifest;

use super::{tiny_model, TINY_BASE};

#[derive(Default)]
struct MockTransport {
    rx: VecDeque<u8>,
    tx: Vec<u8>,
}

impl Transport for MockTransport {
    fn send_all(&mut self, data: &[u8]) -> Result<(), ()> {
        self.tx.extend_from_slice(data);
        Ok(())
    }

    fn recv_some(&mut self, buf: &mut [u8]) -> Result<usize, ()> {
        if self.rx.is_empty() {
            return Ok(0);
        }
        let n = buf.len().min(self.rx.len());
        for (d, s) in buf[..n].iter_mut().zip(self.rx.drain(..n)) {
            *d = s;
        }
        Ok(n)
    }
}

enum NodePhase {
    Hello,
    Assign,
    Begin,
    Run,
}

struct NodeWorker {
    nodo: NodoPipeline,
    rt: Runtime,
    source: MmapTensorSource<MemFileMapper>,
    tx: FramedTransport<MockTransport>,
    phase: NodePhase,
    hello_kv: u32,
    steps_atendidos: u32,
    /// En el Step número N (1-based), responde `Error` una vez (reintento/reconexión del head).
    fail_step_at: Option<u32>,
}

impl NodeWorker {
    fn new(
        manifest: Manifest,
        index: TensorIndex,
        mapper: MemFileMapper,
        layer_start: u32,
        layer_end: u32,
        model_name: &str,
        manifest_crc: u32,
        index_crc: u32,
        fail_step_at: Option<u32>,
    ) -> Self {
        let mut rt = Runtime::new(manifest.clone(), index.clone(), 0, 0);
        let role = PipelineRole::from_layer_range(layer_start, layer_end, manifest.num_layers);
        rt.validate_shapes_for_role(role, layer_start, layer_end)
            .expect("shapes nodo mem");
        let source = MmapTensorSource::new(String::from(TINY_BASE), index, mapper);
        let nodo = NodoPipeline::new(NodoConfig {
            layer_start,
            layer_end,
            fixed_layers: true,
            model_name: String::from(model_name),
            manifest_crc,
            index_crc,
        });
        Self {
            nodo,
            rt,
            source,
            tx: FramedTransport::new(MockTransport::default()),
            phase: NodePhase::Hello,
            hello_kv: 0,
            steps_atendidos: 0,
            fail_step_at,
        }
    }

    fn responder_hello(&mut self, hello: &HelloPayload) -> Result<(), ()> {
        if hello.role != ROLE_HEAD
            || !NodoPipeline::hello_head_valido(
                hello,
                &self.nodo.cfg,
                self.rt.manifest.num_layers,
                self.rt.manifest.hidden_dim,
            )
        {
            let _ = self.tx.send(&NodoPipeline::error_handshake("mem handshake"));
            return Err(());
        }
                    self.hello_kv = hello.kv_pos;
                    if hello.kv_pos > 0 {
                        if hello.prefix_hash != 0
                            && self.nodo.prefix_hash != 0
                            && hello.prefix_hash != self.nodo.prefix_hash
                        {
                            return Err(());
                        }
                        if self.rt.pos != hello.kv_pos as usize {
                            self.rt.truncar_kv(hello.kv_pos as usize);
                        }
                        self.nodo.kv_pos = hello.kv_pos;
                        self.nodo.prefix_hash = hello.prefix_hash;
                    }
        self.tx.send(&Message::Hello(self.nodo.hello_nodo(
            &self.rt,
            self.rt.manifest.num_layers,
            self.rt.manifest.hidden_dim,
            self.rt.manifest.vocab_size,
        )))?;
        self.tx.send(&Message::Caps(NodeCaps {
            ram_free_mb: 512,
            workers: 1,
            bw_score: 1000,
        }))?;
        self.phase = NodePhase::Assign;
        Ok(())
    }

    fn poll(&mut self, head: &mut MockTransport) -> Result<(), ()> {
        self.tx.inner.rx.extend(head.tx.drain(..));
        head.rx.extend(self.tx.inner.tx.drain(..));

        loop {
            let msg = match self.tx.try_recv() {
                Ok(Some(m)) => m,
                Ok(None) => break,
                Err(()) => return Err(()),
            };
            match self.phase {
                NodePhase::Hello => {
                    let hello = match msg {
                        Message::Hello(h) => h,
                        Message::Ping => {
                            self.tx.send(&Message::Pong)?;
                            continue;
                        }
                        _ => return Err(()),
                    };
                    self.responder_hello(&hello)?;
                }
                NodePhase::Assign => {
                    let assign = match msg {
                        Message::Assign(a) => a,
                        Message::Ping => {
                            self.tx.send(&Message::Pong)?;
                            continue;
                        }
                        _ => return Err(()),
                    };
                    self.nodo.apply_assign(assign).map_err(|_| ())?;
                    self.tx.send(&Message::Ack(AckPayload { pos: 0 }))?;
                    self.phase = NodePhase::Begin;
                }
                NodePhase::Begin => {
                    let begin = match msg {
                        Message::Begin(b) => b,
                        Message::Ping => {
                            self.tx.send(&Message::Pong)?;
                            continue;
                        }
                        _ => return Err(()),
                    };
                    let reset_kv = self.hello_kv == 0 && self.rt.pos == 0;
                    self.nodo.procesar_begin(begin, &mut self.rt, reset_kv);
                    self.tx.send(&Message::Ack(AckPayload { pos: 0 }))?;
                    self.phase = NodePhase::Run;
                }
                NodePhase::Run => match msg {
                    Message::Ping => {
                        self.tx.send(&Message::Pong)?;
                    }
                    Message::Hello(h) => {
                        self.responder_hello(&h)?;
                    }
                    Message::Step(step) => {
                        if self.should_fail_step(&step) {
                            self.tx.send(&Message::Error(ErrorPayload {
                                code: ERR_NODE_DOWN,
                                message: String::from("fallo simulado en mem"),
                            }))?;
                        } else if let Some(reply) = self
                            .nodo
                            .atender(&mut self.rt, &mut self.source, None, Message::Step(step))?
                        {
                            self.steps_atendidos += 1;
                            self.tx.send(&reply)?;
                        }
                    }
                    other => {
                        if let Some(reply) = self
                            .nodo
                            .atender(&mut self.rt, &mut self.source, None, other)?
                        {
                            self.tx.send(&reply)?;
                        }
                    }
                },
            }
            head.rx.extend(self.tx.inner.tx.drain(..));
        }
        Ok(())
    }

    fn should_fail_step(&mut self, step: &StepPayload) -> bool {
        let Some(at) = self.fail_step_at else {
            return false;
        };
        if self.steps_atendidos + 1 == at && step.n_tokens > 0 {
            self.fail_step_at = None;
            return true;
        }
        false
    }
}

struct MemWire {
    head: RefCell<MockTransport>,
    worker: RefCell<NodeWorker>,
}

impl MemWire {
    fn pump(&self) {
        let mut head = self.head.borrow_mut();
        let mut worker = self.worker.borrow_mut();
        let _ = worker.poll(&mut head);
    }
}

pub struct HeadSide {
    wire: Rc<MemWire>,
}

impl Transport for HeadSide {
    fn send_all(&mut self, data: &[u8]) -> Result<(), ()> {
        self.wire.head.borrow_mut().tx.extend_from_slice(data);
        self.wire.pump();
        Ok(())
    }

    fn recv_some(&mut self, buf: &mut [u8]) -> Result<usize, ()> {
        self.wire.pump();
        self.wire.head.borrow_mut().recv_some(buf)
    }
}

/// Registro de enlaces `addr` → nodo simulado.
pub struct MemConector {
    wires: HashMap<String, Rc<MemWire>>,
}

impl MemConector {
    pub fn new() -> Self {
        Self {
            wires: HashMap::new(),
        }
    }

    /// Añade un nodo tail (o intermedio) en `addr` con capas `[layer_start, layer_end)`.
    pub fn add_node(
        &mut self,
        addr: &str,
        layer_start: u32,
        layer_end: u32,
        model_name: &str,
        manifest_crc: u32,
        index_crc: u32,
    ) {
        self.add_node_opts(
            addr,
            layer_start,
            layer_end,
            model_name,
            manifest_crc,
            index_crc,
            None,
        );
    }

    /// Como [`add_node`], pero el Step `fail_step_at` (1-based) devuelve error una vez.
    pub fn add_node_fail_step(
        &mut self,
        addr: &str,
        layer_start: u32,
        layer_end: u32,
        model_name: &str,
        manifest_crc: u32,
        index_crc: u32,
        fail_step_at: u32,
    ) {
        self.add_node_opts(
            addr,
            layer_start,
            layer_end,
            model_name,
            manifest_crc,
            index_crc,
            Some(fail_step_at),
        );
    }

    fn add_node_opts(
        &mut self,
        addr: &str,
        layer_start: u32,
        layer_end: u32,
        model_name: &str,
        manifest_crc: u32,
        index_crc: u32,
        fail_step_at: Option<u32>,
    ) {
        let (manifest, index, mapper) = tiny_model();
        let worker = NodeWorker::new(
            manifest,
            index,
            mapper,
            layer_start,
            layer_end,
            model_name,
            manifest_crc,
            index_crc,
            fail_step_at,
        );
        let wire = Rc::new(MemWire {
            head: RefCell::new(MockTransport::default()),
            worker: RefCell::new(worker),
        });
        self.wires.insert(String::from(addr), wire);
    }
}

impl Conector for MemConector {
    type T = HeadSide;

    fn conectar(&mut self, addr: &str, _timeout_ms: u64) -> Result<FramedTransport<HeadSide>, ()> {
        let wire = self.wires.get(addr).ok_or(())?.clone();
        Ok(FramedTransport::new(HeadSide { wire }))
    }
}

/// Reloj de pared (ms): evita busy-loop infinito en `recv_timeout` si falta un frame.
pub fn now_wall_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
