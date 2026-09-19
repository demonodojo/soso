//! T09 — cancelación cooperativa en prefill, capas y decode.

#![cfg(feature = "std")]

use soso_llm_core::generation::{
    GenCheckpoint, GenerationObserver, GenerationOptions, StopReason,
};
use soso_llm_core::runtime::Runtime;
use soso_llm_core::source::host::MemFileMapper;
use soso_llm_core::source::MmapTensorSource;
use sosomodel::index::{make_f32_entry, pack_shard, TensorIndex};
use sosomodel::manifest::Manifest;

const BASE: &str = "/models/tiny/shards";

fn f32_shard(elems: usize, seed: u32) -> Vec<u8> {
    let raw: Vec<u8> = (0..elems)
        .flat_map(|i| {
            let v = (((i as u32).wrapping_mul(0x9e37_79b9) ^ seed) as f32) * 1e-4;
            v.to_le_bytes()
        })
        .collect();
    pack_shard(&raw)
}

fn tiny_model() -> (Manifest, TensorIndex, MemFileMapper) {
    let manifest = Manifest::tiny("tiny-cancel");
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

fn setup() -> (Runtime, MmapTensorSource<MemFileMapper>) {
    let (manifest, index, mapper) = tiny_model();
    let rt = Runtime::new(manifest, index.clone(), 0, 0);
    rt.validate_shapes().expect("shapes");
    (
        rt,
        MmapTensorSource::new(String::from(BASE), index, mapper),
    )
}

/// Cancela en el checkpoint n-ésimo (1-based) o falla si `fail_at` coincide.
struct PhaseObserver {
    seen: u32,
    cancel_at: u32,
    fail_at: Option<u32>,
    emitted: Vec<u32>,
    last: Option<GenCheckpoint>,
}

impl PhaseObserver {
    fn new(cancel_at: u32) -> Self {
        Self {
            seen: 0,
            cancel_at,
            fail_at: None,
            emitted: Vec::new(),
            last: None,
        }
    }

    fn with_fail(cancel_at: u32, fail_at: u32) -> Self {
        let mut s = Self::new(cancel_at);
        s.fail_at = Some(fail_at);
        s
    }
}

impl GenerationObserver for PhaseObserver {
    fn on_token(&mut self, token: u32) {
        self.emitted.push(token);
    }

    fn cancel_at(&mut self, at: GenCheckpoint) -> bool {
        self.seen += 1;
        self.last = Some(at);
        if self.fail_at == Some(self.seen) {
            panic!("observador inyectó error en checkpoint {}", self.seen);
        }
        self.seen >= self.cancel_at
    }
}

fn checkpoint_id(at: GenCheckpoint) -> u8 {
    match at {
        GenCheckpoint::BeforePrefill => 1,
        GenCheckpoint::PrefillToken { .. } => 2,
        GenCheckpoint::Layer { .. } => 3,
        GenCheckpoint::BeforeDecode => 4,
        GenCheckpoint::BeforeSample => 5,
    }
}

struct AtCheckpointObserver {
    target: GenCheckpoint,
    seen: bool,
}

impl GenerationObserver for AtCheckpointObserver {
    fn cancel_at(&mut self, at: GenCheckpoint) -> bool {
        if !self.seen && checkpoint_id(at) == checkpoint_id(self.target) {
            if std::mem::discriminant(&at) == std::mem::discriminant(&self.target) {
                match (&self.target, &at) {
                    (
                        GenCheckpoint::PrefillToken { done: a, .. },
                        GenCheckpoint::PrefillToken { done: b, .. },
                    ) if a == b => {
                        self.seen = true;
                        return true;
                    }
                    (
                        GenCheckpoint::Layer { layer: a, .. },
                        GenCheckpoint::Layer { layer: b, .. },
                    ) if a == b => {
                        self.seen = true;
                        return true;
                    }
                    (GenCheckpoint::BeforePrefill, GenCheckpoint::BeforePrefill)
                    | (GenCheckpoint::BeforeDecode, GenCheckpoint::BeforeDecode)
                    | (GenCheckpoint::BeforeSample, GenCheckpoint::BeforeSample) => {
                        self.seen = true;
                        return true;
                    }
                    _ => {}
                }
            }
        }
        false
    }
}

fn referencia(
    rt: &mut Runtime,
    source: &mut MmapTensorSource<MemFileMapper>,
    prompt: &[u32],
    max_new: usize,
) -> Vec<u32> {
    rt.generate(source, prompt, max_new, None).expect("referencia")
}

#[test]
fn cancela_antes_de_prefill() {
    let (mut rt, mut source) = setup();
    let prompt = [1u32, 2, 3, 4, 5, 6, 7, 8];
    let mut obs = AtCheckpointObserver {
        target: GenCheckpoint::BeforePrefill,
        seen: false,
    };
    let (_, report) = rt
        .generate_with_report_observed(
            &mut source,
            &prompt,
            GenerationOptions::new(4, vec![]),
            &mut obs,
        )
        .expect("cancel");
    assert_eq!(report.stop, StopReason::Cancelled);
    assert_eq!(report.generated, 0);
    assert_eq!(report.sampled_tokens, 0);
}

#[test]
fn cancela_en_prefill_largo() {
    let (mut rt, mut source) = setup();
    let prompt: Vec<u32> = (0..24).collect();
    let mut obs = AtCheckpointObserver {
        target: GenCheckpoint::PrefillToken { done: 12, total: 24 },
        seen: false,
    };
    let (tokens, report) = rt
        .generate_with_report_observed(
            &mut source,
            &prompt,
            GenerationOptions::new(8, vec![]),
            &mut obs,
        )
        .expect("cancel prefill");
    assert_eq!(report.stop, StopReason::Cancelled);
    assert_eq!(tokens, prompt);
    assert_eq!(report.generated, 0);
}

#[test]
fn cancela_en_capa() {
    let (mut rt, mut source) = setup();
    let prompt = [10u32, 20, 30];
    let mut obs = AtCheckpointObserver {
        target: GenCheckpoint::Layer {
            layer: 0,
            of: rt.manifest.num_layers,
        },
        seen: false,
    };
    let (_, report) = rt
        .generate_with_report_observed(
            &mut source,
            &prompt,
            GenerationOptions::new(4, vec![]),
            &mut obs,
        )
        .expect("cancel layer");
    assert_eq!(report.stop, StopReason::Cancelled);
}

#[test]
fn cancela_antes_de_muestrear() {
    let (mut rt, mut source) = setup();
    let prompt = [10u32, 20, 30];
    let mut obs = AtCheckpointObserver {
        target: GenCheckpoint::BeforeSample,
        seen: false,
    };
    let (tokens, report) = rt
        .generate_with_report_observed(
            &mut source,
            &prompt,
            GenerationOptions::new(6, vec![]),
            &mut obs,
        )
        .expect("cancel decode");
    assert_eq!(report.stop, StopReason::Cancelled);
    assert_eq!(tokens, prompt);
    assert_eq!(report.generated, 0);
}

#[test]
fn no_emite_tras_cancelar_en_decode() {
    let (mut rt, mut source) = setup();
    let prompt = [5u32, 10, 15];
    let mut obs = PhaseObserver::new(999);
    obs.cancel_at = 50;
    for n in 1..80u32 {
        obs.seen = 0;
        obs.emitted.clear();
        obs.cancel_at = n;
        let (tokens, report) = rt
            .generate_with_report_observed(
                &mut source,
                &prompt,
                GenerationOptions::new(3, vec![]),
                &mut obs,
            )
            .expect("gen");
        if report.stop == StopReason::Cancelled {
            let prefix = &tokens[..prompt.len()];
            assert_eq!(prefix, prompt);
            assert_eq!(obs.emitted.len(), report.generated as usize);
            return;
        }
    }
    panic!("no se alcanzó cancelación en decode");
}

#[test]
fn runtime_reutilizable_tras_cancelar() {
    let (mut rt, mut source) = setup();
    let prompt = [10u32, 20, 30];
    let mut obs = AtCheckpointObserver {
        target: GenCheckpoint::BeforeSample,
        seen: false,
    };
    rt.generate_with_report_observed(
        &mut source,
        &prompt,
        GenerationOptions::new(5, vec![]),
        &mut obs,
    )
    .expect("cancel");

    let despues = referencia(&mut rt, &mut source, &prompt, 3);
    let (manifest, index, mapper) = tiny_model();
    let mut rt2 = Runtime::new(manifest, index.clone(), 0, 0);
    rt2.validate_shapes().expect("shapes");
    let mut source2 = MmapTensorSource::new(String::from(BASE), index, mapper);
    let esperado = referencia(&mut rt2, &mut source2, &prompt, 3);
    assert_eq!(despues, esperado);
}

#[test]
fn adaptador_legacy_sigue_funcionando() {
    let (mut rt, mut source) = setup();
    let prompt = [10u32, 20];
    let tokens = rt.generate(&mut source, &prompt, 2, None).expect("legacy");
    assert!(tokens.len() >= prompt.len());
}

#[test]
#[should_panic(expected = "observador inyectó error")]
fn error_en_observador_aborta() {
    let (mut rt, mut source) = setup();
    let prompt = [1u32, 2, 3];
    let mut obs = PhaseObserver::with_fail(100, 2);
    let _ = rt.generate_with_report_observed(
        &mut source,
        &prompt,
        GenerationOptions::new(2, vec![]),
        &mut obs,
    );
}
