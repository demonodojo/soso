//! Tests del protocolo y paridad numérica del forward por rangos.

#![cfg(feature = "std")]

use soso_llm_core::pipeline::{
    self, crc32c, decode_message, encode_message, FramedTransport, Message, PipelinePlan,
    PipelineRole, RecvError, StepPayload, StepReplyPayload, Transport, HelloPayload, ROLE_HEAD,
    ROLE_NODE,
};
use soso_llm_core::runtime::Runtime;
use soso_llm_core::source::host::MemFileMapper;
use soso_llm_core::source::MmapTensorSource;
use sosomodel::index::{make_f32_entry, pack_shard, TensorIndex};
use sosomodel::manifest::Manifest;

const BASE: &str = "/models/tiny/shards";

struct MockTransport {
    rx: std::collections::VecDeque<u8>,
    tx: Vec<u8>,
}

impl Default for MockTransport {
    fn default() -> Self {
        Self {
            rx: std::collections::VecDeque::new(),
            tx: Vec::new(),
        }
    }
}

impl MockTransport {
    fn pair() -> (Self, Self) {
        let a = Self {
            rx: std::collections::VecDeque::new(),
            tx: Vec::new(),
        };
        let b = Self {
            rx: std::collections::VecDeque::new(),
            tx: Vec::new(),
        };
        (a, b)
    }
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

fn f32_shard(elems: usize, seed: u32) -> Vec<u8> {
    let raw: Vec<u8> = (0..elems)
        .flat_map(|i| {
            let v = (((i as u32).wrapping_mul(0x9e37_79b9) ^ seed) % 1000) as f32 * 1e-4;
            v.to_le_bytes()
        })
        .collect();
    pack_shard(&raw)
}

fn tiny_model() -> (Manifest, TensorIndex, MemFileMapper) {
    let manifest = Manifest::tiny("tiny");
    let h = manifest.hidden_dim;
    let ffn = manifest.ffn_dim;
    let vocab = manifest.vocab_size;

    let mut mapper = MemFileMapper::new();
    let mut index = TensorIndex::default();
    let mut id = 0u32;
    let add = |index: &mut TensorIndex,
               mapper: &mut MemFileMapper,
               id: &mut u32,
               name: &str,
               shape: &[u32]| {
        let elems: usize = shape.iter().map(|&d| d as usize).product();
        let shard = format!("{name}.tensor");
        mapper
            .files
            .insert(format!("{BASE}/{shard}"), f32_shard(elems, *id));
        index
            .entries
            .push(make_f32_entry(*id, name, &shard, 0, shape));
        *id += 1;
    };

    for layer in 0..manifest.num_layers {
        let p = format!("L{layer:02}");
        add(&mut index, &mut mapper, &mut id, &format!("{p}.attn_norm"), &[h]);
        add(&mut index, &mut mapper, &mut id, &format!("{p}.attn_q"), &[h, h]);
        add(&mut index, &mut mapper, &mut id, &format!("{p}.attn_k"), &[h, h]);
        add(&mut index, &mut mapper, &mut id, &format!("{p}.attn_v"), &[h, h]);
        add(&mut index, &mut mapper, &mut id, &format!("{p}.attn_output"), &[h, h]);
        add(&mut index, &mut mapper, &mut id, &format!("{p}.ffn_norm"), &[h]);
        add(&mut index, &mut mapper, &mut id, &format!("{p}.ffn_up"), &[ffn, h]);
        add(&mut index, &mut mapper, &mut id, &format!("{p}.ffn_down"), &[h, ffn]);
    }
    add(&mut index, &mut mapper, &mut id, "embed", &[vocab, h]);
    (manifest, index, mapper)
}

#[test]
fn codec_roundtrip_fragmentado() {
    let msg = Message::Step(StepPayload {
        pos: 3,
        want_token: 1,
        hidden: vec![1.0, 2.0, 3.0],
    });
    let frame = encode_message(7, &msg);
    let (seq, decoded) = decode_message(&frame).unwrap();
    assert_eq!(seq, 7);
    assert_eq!(decoded, msg);

    let reply = Message::StepReply(StepReplyPayload {
        pos: 3,
        hidden: vec![4.0, 5.0],
    });
    let frame2 = encode_message(8, &reply);
    assert_eq!(decode_message(&frame2).unwrap().1, reply);
}

#[test]
fn pipeline_plan_from_splits() {
    let plan = PipelinePlan::from_splits(&[2, 3], 4).unwrap();
    assert_eq!(plan.segments.len(), 3);
    assert_eq!(plan.head_segment().layer_start, 0);
    assert_eq!(plan.head_segment().layer_end, 2);
    assert_eq!(plan.remote_segment(0).unwrap().layer_start, 2);
    assert_eq!(plan.remote_segment(1).unwrap().layer_end, 4);
}

#[test]
fn framed_transport_mock() {
    let (a, b) = MockTransport::pair();
    let mut ta = FramedTransport::new(a);
    let mut tb = FramedTransport::new(b);
    ta.send(&Message::Hello(HelloPayload {
        role: ROLE_HEAD,
        layer_start: 0,
        layer_end: 2,
        num_layers: 4,
        hidden_dim: 128,
        vocab_size: 256,
        model_name: String::from("tiny"),
        manifest_crc: 1,
        index_crc: 2,
    }))
    .unwrap();
    tb.inner.rx.extend(ta.inner.tx.drain(..));
    let msg = tb.recv().unwrap();
    assert!(matches!(msg, Message::Hello(_)));
}

#[test]
fn forward_por_rangos_paridad() {
    let split = 2u32;
    let prompt = [10u32, 20];

    let (m1, i1, map1) = tiny_model();
    let mut full = Runtime::new(m1, i1, 0, 0);
    full.validate_shapes().unwrap();
    let mut src_full = MmapTensorSource::new(String::from(BASE), full.index.clone(), map1);

    for &tok in &prompt {
        full.embed_token(tok, &mut src_full).unwrap();
        full.forward_step_par(&mut src_full, None).unwrap();
    }
    let logits_full = full.logits(&mut src_full).unwrap().to_vec();

    let (m2, i2, map2) = tiny_model();
    let mut head = Runtime::new(m2, i2, 0, 0);
    head.validate_shapes_for_role(PipelineRole::Head, 0, split)
        .unwrap();
    let mut src_head = MmapTensorSource::new(String::from(BASE), head.index.clone(), map2);

    let (m3, i3, map3) = tiny_model();
    let mut tail = Runtime::new(m3, i3, 0, 0);
    tail.validate_shapes_for_role(PipelineRole::Tail, split, 4)
        .unwrap();
    let mut src_tail = MmapTensorSource::new(String::from(BASE), tail.index.clone(), map3);

    for &tok in &prompt {
        head.embed_token(tok, &mut src_head).unwrap();
        head.forward_layers_range(0, split, &mut src_head, None).unwrap();
        tail.set_hidden(head.hidden_slice()).unwrap();
        tail.forward_layers_range(split, 4, &mut src_tail, None).unwrap();
        head.advance_pos();
        tail.advance_pos();
    }
    let logits_split = tail.logits(&mut src_tail).unwrap();

    assert_eq!(logits_split.len(), logits_full.len());
    for (a, b) in logits_split.iter().zip(logits_full.iter()) {
        assert!((a - b).abs() < 1e-5, "logits difieren: {a} vs {b}");
    }
}

#[test]
fn forward_estrella_tres_segmentos() {
    let prompt = [10u32];
    let plan = PipelinePlan::from_splits(&[1, 2], 4).unwrap();

    let (m1, i1, map1) = tiny_model();
    let mut full = Runtime::new(m1, i1, 0, 0);
    let mut src_full = MmapTensorSource::new(String::from(BASE), full.index.clone(), map1);
    full.embed_token(prompt[0], &mut src_full).unwrap();
    full.forward_step_par(&mut src_full, None).unwrap();
    let logits_full = full.logits(&mut src_full).unwrap().to_vec();

    let run_seg =
        |start: u32, end: u32, role: PipelineRole, hidden: &[f32]| -> Vec<f32> {
            let (m, i, map) = tiny_model();
            let mut rt = Runtime::new(m, i, 0, 0);
            rt.validate_shapes_for_role(role, start, end).unwrap();
            let mut src = MmapTensorSource::new(String::from(BASE), rt.index.clone(), map);
            rt.set_hidden(hidden).unwrap();
            rt.forward_layers_range(start, end, &mut src, None).unwrap();
            rt.hidden_slice().to_vec()
        };

    let (m2, i2, map2) = tiny_model();
    let mut head = Runtime::new(m2, i2, 0, 0);
    head.validate_shapes_for_role(PipelineRole::Head, 0, 1).unwrap();
    let mut src_head = MmapTensorSource::new(String::from(BASE), head.index.clone(), map2);
    head.embed_token(prompt[0], &mut src_head).unwrap();
    head.forward_layers_range(0, 1, &mut src_head, None).unwrap();
    let mut h = head.hidden_slice().to_vec();

    let seg1 = plan.remote_segment(0).unwrap();
    h = run_seg(
        seg1.layer_start,
        seg1.layer_end,
        PipelineRole::Node,
        &h,
    );
    let seg2 = plan.remote_segment(1).unwrap();
    h = run_seg(
        seg2.layer_start,
        seg2.layer_end,
        PipelineRole::Tail,
        &h,
    );

    let (m3, i3, map3) = tiny_model();
    let mut tail = Runtime::new(m3, i3, 0, 0);
    tail.validate_shapes_for_role(PipelineRole::Tail, 2, 4).unwrap();
    let mut src_tail = MmapTensorSource::new(String::from(BASE), tail.index.clone(), map3);
    tail.set_hidden(&h).unwrap();
    let logits_star = tail.logits(&mut src_tail).unwrap();

    assert_eq!(logits_star.len(), logits_full.len());
    for (a, b) in logits_star.iter().zip(logits_full.iter()) {
        assert!((a - b).abs() < 1e-5, "logits difieren: {a} vs {b}");
    }
}

#[test]
fn crc32c_estable() {
    let a = crc32c(b"manifest");
    let b = crc32c(b"manifest");
    assert_eq!(a, b);
    assert_ne!(a, crc32c(b"index"));
}

#[test]
fn ping_pong_auto_reply() {
    fn tick_now() -> u64 {
        static C: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        C.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    }

    let (a, b) = MockTransport::pair();
    let mut head = FramedTransport::new(a);
    let mut node = FramedTransport::new(b);
    head.send(&Message::Ping).unwrap();
    node.inner.rx.extend(head.inner.tx.drain(..));
    assert_eq!(
        node.recv_timeout(5, tick_now),
        Err(RecvError::Timeout),
        "ping se consume sin devolver mensaje de aplicación"
    );
    head.inner.rx.extend(node.inner.tx.drain(..));
    assert!(matches!(head.recv().unwrap(), Message::Pong));
}

fn advancing_now() -> u64 {
    static C: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    C.fetch_add(20, std::sync::atomic::Ordering::Relaxed)
}

#[test]
fn recv_timeout_sin_datos() {
    let mut t = FramedTransport::new(MockTransport::default());
    assert_eq!(t.recv_timeout(10, advancing_now), Err(RecvError::Timeout));
}

#[test]
fn begin_payload_session_id() {
    let msg = Message::Begin(soso_llm_core::pipeline::BeginPayload {
        temp: 0.0,
        top_p: 0.9,
        seed: 42,
        max_new: 8,
        session_id: 0xdead_beef,
    });
    let frame = encode_message(1, &msg);
    let (_, decoded) = decode_message(&frame).unwrap();
    assert_eq!(decoded, msg);
}

#[test]
fn keepalive_detecta_peer_caido() {
    fn tick() -> u64 {
        static C: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        C.fetch_add(5, std::sync::atomic::Ordering::Relaxed)
    }
    let mut t = FramedTransport::new(MockTransport::default());
    assert_eq!(
        t.recv_timeout_keepalive(u64::MAX, 10, 20, tick),
        Err(RecvError::Timeout)
    );
}

#[test]
fn mensaje_invalido_falla() {
    assert!(decode_message(&[0u8; 4]).is_err());
}
