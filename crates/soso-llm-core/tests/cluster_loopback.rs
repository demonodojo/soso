//! Paridad numérica vía protocolo de cluster (`EstrellaHead` + `MemConector`).

#![cfg(feature = "std")]

mod common;

use common::mem::{now_wall_ms, MemConector};
use common::{tiny_model, TINY_BASE};
use soso_llm_core::cluster::{EstrellaConfig, EstrellaHead};
use soso_llm_core::generation::{GenerationOptions, NoCancelObserver};
use soso_llm_core::pipeline::{prefix_hash_tokens, BeginPayload, PipelinePlan, PipelineRole, WIRE_F16};
use soso_llm_core::remoto::{ClusterGen, JournalActivaciones, TramoRemoto};
use soso_llm_core::runtime::Runtime;
use soso_llm_core::sample::Sampler;
use soso_llm_core::source::host::MemFileMapper;
use soso_llm_core::source::MmapTensorSource;

const MODEL: &str = "tiny";
const CRC_M: u32 = 1;
const CRC_I: u32 = 2;

fn estrella_cfg(model_name: &str, wire: u8) -> EstrellaConfig {
    EstrellaConfig {
        wire,
        ping_interval_ms: 0,
        model_name: String::from(model_name),
        manifest_crc: CRC_M,
        index_crc: CRC_I,
        ..EstrellaConfig::default()
    }
}

fn forward_full(prompt: &[u32]) -> Vec<f32> {
    let (m, i, map) = tiny_model();
    let mut rt = Runtime::new(m, i, 0, 0);
    rt.validate_shapes().unwrap();
    let mut src = MmapTensorSource::new(String::from(TINY_BASE), rt.index.clone(), map);
    for &tok in prompt {
        rt.embed_token(tok, &mut src).unwrap();
        let mut cancel = None;
        rt.forward_step_par(
            &mut src,
            None,
            &mut None,
            None,
            &mut cancel,
            None,
            &mut None,
        )
        .unwrap();
    }
    rt.logits(&mut src).unwrap().to_vec()
}

fn assert_logits_cerca(a: &[f32], b: &[f32], ctx: &str, tol: f32) {
    assert_eq!(a.len(), b.len(), "{ctx}: vocab distinto");
    for (i, (x, y)) in a.iter().zip(b.iter()).enumerate() {
        assert!((x - y).abs() < tol, "{ctx} idx {i}: {x} vs {y}");
    }
}

fn forward_full_then_truncar(tokens: &[u32], pos: usize) -> Vec<f32> {
    let (m, i, map) = tiny_model();
    let mut rt = Runtime::new(m, i, 0, 0);
    rt.validate_shapes().unwrap();
    let mut src = MmapTensorSource::new(String::from(TINY_BASE), rt.index.clone(), map);
    for &tok in tokens {
        rt.embed_token(tok, &mut src).unwrap();
        let mut cancel = None;
        rt.forward_step_par(
            &mut src,
            None,
            &mut None,
            None,
            &mut cancel,
            None,
            &mut None,
        )
        .unwrap();
    }
    rt.truncar_kv(pos);
    rt.logits(&mut src).unwrap().to_vec()
}

struct DosSegmentos {
    split: u32,
    head_rt: Runtime,
    src: MmapTensorSource<MemFileMapper>,
    est: EstrellaHead<MemConector>,
}

impl DosSegmentos {
    fn new(wire: u8, conector: MemConector) -> Self {
        let split = 2u32;
        let plan = PipelinePlan::from_splits(&[split], 4).unwrap();
        let (m, i, map) = tiny_model();
        let mut head_rt = Runtime::new(m, i, 0, 0);
        head_rt
            .validate_shapes_for_role(PipelineRole::Head, 0, split)
            .unwrap();
        let src = MmapTensorSource::new(String::from(TINY_BASE), head_rt.index.clone(), map);
        let est = EstrellaHead::new(
            estrella_cfg(MODEL, wire),
            plan,
            conector,
            head_rt.manifest.hidden_dim,
            head_rt.manifest.num_layers,
            now_wall_ms,
        );
        Self {
            split,
            head_rt,
            src,
            est,
        }
    }

    fn enlazar_begin(&mut self, session_id: u64) {
        self.est.enlazar(&[String::from("n0")], 0, 0).unwrap();
        self.est
            .begin_session(BeginPayload {
                temp: 0.0,
                top_p: 1.0,
                seed: 1,
                max_new: 0,
                session_id,
            })
            .unwrap();
    }

    fn forward(&mut self, tokens: &[u32], journal: Option<&mut JournalActivaciones>) {
        let mut cg = ClusterGen {
            local_end: self.split,
            remoto: &mut self.est,
            journal,
            prefix_hash: 0,
        };
        let mut slot = Some(&mut cg);
        for &tok in tokens {
            self.head_rt.embed_token(tok, &mut self.src).unwrap();
            let mut cancel = None;
            self.head_rt
                .forward_step_par(
                    &mut self.src,
                    None,
                    &mut None,
                    None,
                    &mut cancel,
                    None,
                    &mut slot,
                )
                .unwrap();
        }
    }

    fn logits(&mut self) -> Vec<f32> {
        self.head_rt.logits(&mut self.src).unwrap().to_vec()
    }

    fn truncar(&mut self, pos: usize) {
        self.head_rt.truncar_kv(pos);
        self.est.truncar(pos).unwrap();
    }
}

#[test]
fn estrella_dos_segmentos_greedy() {
    let prompt = [10u32, 20];
    let ref_logits = forward_full(&prompt);

    let mut conector = MemConector::new();
    conector.add_node("n0", 2, 4, MODEL, CRC_M, CRC_I);
    let mut bench = DosSegmentos::new(0, conector);
    bench.enlazar_begin(42);
    bench.forward(&prompt, None);
    assert_logits_cerca(&bench.logits(), &ref_logits, "greedy 2 seg", 1e-5);
}

#[test]
fn estrella_tres_segmentos_un_token() {
    let prompt = [10u32];
    let ref_logits = forward_full(&prompt);
    let plan = PipelinePlan::from_splits(&[1, 2], 4).unwrap();

    let mut conector = MemConector::new();
    conector.add_node("n0", 1, 2, MODEL, CRC_M, CRC_I);
    conector.add_node("n1", 2, 4, MODEL, CRC_M, CRC_I);

    let (m, i, map) = tiny_model();
    let mut head_rt = Runtime::new(m, i, 0, 0);
    head_rt
        .validate_shapes_for_role(PipelineRole::Head, 0, 1)
        .unwrap();
    let mut src = MmapTensorSource::new(String::from(TINY_BASE), head_rt.index.clone(), map);

    let mut est = EstrellaHead::new(
        estrella_cfg(MODEL, 0),
        plan,
        conector,
        head_rt.manifest.hidden_dim,
        head_rt.manifest.num_layers,
        now_wall_ms,
    );
    est.enlazar(
        &[String::from("n0"), String::from("n1")],
        0,
        0,
    )
    .unwrap();
    est.begin_session(BeginPayload {
        temp: 0.0,
        top_p: 1.0,
        seed: 1,
        max_new: 0,
        session_id: 43,
    })
    .unwrap();

    let mut cg = ClusterGen {
        local_end: 1,
        remoto: &mut est,
        journal: None,
        prefix_hash: 0,
    };
    let mut slot = Some(&mut cg);
    head_rt.embed_token(prompt[0], &mut src).unwrap();
    let mut cancel = None;
    head_rt
        .forward_step_par(
            &mut src,
            None,
            &mut None,
            None,
            &mut cancel,
            None,
            &mut slot,
        )
        .unwrap();
    assert_logits_cerca(
        head_rt.logits(&mut src).unwrap(),
        &ref_logits,
        "3 seg 1 tok",
        1e-5,
    );
}

#[test]
fn estrella_wire_f16() {
    let prompt = [10u32, 20, 30];
    let ref_logits = forward_full(&prompt);

    let mut conector = MemConector::new();
    conector.add_node("n0", 2, 4, MODEL, CRC_M, CRC_I);
    let mut bench = DosSegmentos::new(WIRE_F16, conector);
    bench.enlazar_begin(44);
    bench.forward(&prompt, None);
    assert_logits_cerca(&bench.logits(), &ref_logits, "wire f16", 1e-2);
}

#[test]
fn estrella_truncar_kv_remoto() {
    let all = [10u32, 20, 30];
    let ref_logits = forward_full_then_truncar(&all, 2);

    let mut conector = MemConector::new();
    conector.add_node("n0", 2, 4, MODEL, CRC_M, CRC_I);
    let mut bench = DosSegmentos::new(0, conector);
    bench.enlazar_begin(45);
    bench.forward(&all, None);
    bench.truncar(2);
    assert_logits_cerca(&bench.logits(), &ref_logits, "tras truncar@2", 1e-5);
}

#[test]
fn estrella_prefix_hash_explicito() {
    let prompt = [10u32, 20];
    let ref_logits = forward_full(&prompt);
    let hash = prefix_hash_tokens(&prompt);

    let mut conector = MemConector::new();
    conector.add_node("n0", 2, 4, MODEL, CRC_M, CRC_I);
    let mut bench = DosSegmentos::new(0, conector);
    bench.enlazar_begin(46);

    let mut cg = ClusterGen {
        local_end: bench.split,
        remoto: &mut bench.est,
        journal: None,
        prefix_hash: hash,
    };
    let mut slot = Some(&mut cg);
    for &tok in &prompt {
        bench.head_rt.embed_token(tok, &mut bench.src).unwrap();
        let mut cancel = None;
        bench.head_rt
            .forward_step_par(
                &mut bench.src,
                None,
                &mut None,
                None,
                &mut cancel,
                None,
                &mut slot,
            )
            .unwrap();
    }
    assert_logits_cerca(&bench.logits(), &ref_logits, "prefix_hash", 1e-5);
}

#[test]
fn estrella_dos_sesiones_enlace_fresco() {
    let prompt = [10u32, 20];
    let ref_logits = forward_full(&prompt);

    let mut conector = MemConector::new();
    conector.add_node("n0", 2, 4, MODEL, CRC_M, CRC_I);
    let mut bench = DosSegmentos::new(0, conector);
    bench.enlazar_begin(47);
    bench.forward(&prompt, None);
    assert_logits_cerca(&bench.logits(), &ref_logits, "sesión 1", 1e-5);

    let mut conector2 = MemConector::new();
    conector2.add_node("n0", 2, 4, MODEL, CRC_M, CRC_I);
    let mut bench2 = DosSegmentos::new(0, conector2);
    bench2.enlazar_begin(48);
    bench2.forward(&prompt, None);
    assert_logits_cerca(&bench2.logits(), &ref_logits, "sesión 2", 1e-5);
}

#[test]
fn estrella_generacion_greedy_dos_tokens() {
    let prompt = [10u32, 20];
    let max_new = 2usize;

    let (m1, i1, map1) = tiny_model();
    let mut full = Runtime::new(m1, i1, 0, 0);
    let mut src_full = MmapTensorSource::new(String::from(TINY_BASE), full.index.clone(), map1);
    let mut sampler = Sampler::new(0.0, 1.0, 99);
    let mut obs = NoCancelObserver;
    let (tok_full, _) = full
        .generate_stream_planned_observed(
            &mut src_full,
            &prompt,
            GenerationOptions::new(max_new, vec![]),
            &mut sampler,
            &mut obs,
            None,
            &mut None,
            now_wall_ms,
            &mut || soso_llm_core::plan::MemSnapshot::default(),
            &mut None,
        )
        .unwrap();

    let mut conector = MemConector::new();
    conector.add_node("n0", 2, 4, MODEL, CRC_M, CRC_I);
    let mut bench = DosSegmentos::new(0, conector);
    bench.enlazar_begin(49);

    let mut sampler2 = Sampler::new(0.0, 1.0, 99);
    let mut obs2 = NoCancelObserver;
    let mut cg = ClusterGen {
        local_end: bench.split,
        remoto: &mut bench.est,
        journal: None,
        prefix_hash: 0,
    };
    let mut slot = Some(&mut cg);
    let (tok_star, _) = bench
        .head_rt
        .generate_stream_planned_observed(
            &mut bench.src,
            &prompt,
            GenerationOptions::new(max_new, vec![]),
            &mut sampler2,
            &mut obs2,
            None,
            &mut None,
            now_wall_ms,
            &mut || soso_llm_core::plan::MemSnapshot::default(),
            &mut slot,
        )
        .unwrap();

    assert_eq!(tok_star, tok_full, "tokens generados");
}

#[test]
fn estrella_failover_reconectar_enlace() {
    let all = [10u32, 20, 30];
    let ref_logits = forward_full(&all);
    let pref = [10u32, 20];
    let hash = prefix_hash_tokens(&pref);

    let mut conector = MemConector::new();
    conector.add_node("n0", 2, 4, MODEL, CRC_M, CRC_I);
    let mut bench = DosSegmentos::new(0, conector);
    bench.enlazar_begin(70);
    bench.forward(&pref, None);
    assert_eq!(bench.head_rt.pos, 2);

    bench
        .est
        .reconectar_enlace(
            0,
            "n0",
            2,
            hash,
            BeginPayload {
                temp: 0.0,
                top_p: 1.0,
                seed: 1,
                max_new: 0,
                session_id: 71,
            },
        )
        .unwrap();
    bench.forward(&[30], None);
    assert_logits_cerca(
        &bench.logits(),
        &ref_logits,
        "failover reconectar",
        1e-5,
    );
}

#[test]
fn estrella_reintento_tras_fallo_de_nodo() {
    let prompt = [10u32, 20, 30];
    let ref_logits = forward_full(&prompt);

    let mut conector = MemConector::new();
    conector.add_node_fail_step("n0", 2, 4, MODEL, CRC_M, CRC_I, 2);
    let mut bench = DosSegmentos::new(0, conector);
    bench.enlazar_begin(50);
    let mut journal = JournalActivaciones::new(0);
    bench.forward(&prompt, Some(&mut journal));
    assert_logits_cerca(&bench.logits(), &ref_logits, "reintento nodo", 1e-5);
    assert!(
        !journal.entradas_snapshot().is_empty(),
        "journal debe registrar saltos remotos"
    );
}
