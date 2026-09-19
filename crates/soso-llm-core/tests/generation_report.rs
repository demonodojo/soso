//! T08 — informe de generación y compatibilidad con la API anterior.

#![cfg(feature = "std")]

use soso_llm_core::generation::{GenerationOptions, StopReason};
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
    let manifest = Manifest::tiny("tiny-report");
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
    let source = MmapTensorSource::new(String::from(BASE), index, mapper);
    (rt, source)
}

fn primer_token_nuevo(
    rt: &mut Runtime,
    source: &mut MmapTensorSource<MemFileMapper>,
    prompt: &[u32],
) -> u32 {
    let tokens = rt
        .generate(source, prompt, 1, None)
        .expect("un token");
    *tokens.last().expect("hay salida")
}

#[test]
fn legacy_y_report_devuelven_los_mismos_tokens() {
    let (mut rt, mut source) = setup();
    let prompt = [10u32, 20, 30];
    let legacy = rt
        .generate(&mut source, &prompt, 4, None)
        .expect("legacy");
    let (con_reporte, report) = rt
        .generate_with_report(
            &mut source,
            &prompt,
            GenerationOptions::new(4, vec![]),
        )
        .expect("report");
    assert_eq!(legacy, con_reporte);
    assert_eq!(report.prompt_tokens, prompt.len() as u32);
    assert_eq!(report.generated, (legacy.len() - prompt.len()) as u32);
    assert_eq!(report.sampled_tokens, report.generated);
}

#[test]
fn stop_en_el_primer_muestreo() {
    let (mut rt, mut source) = setup();
    let prompt = [10u32, 20, 30];
    let stop = primer_token_nuevo(&mut rt, &mut source, &prompt);
    let (tokens, report) = rt
        .generate_with_report(
            &mut source,
            &prompt,
            GenerationOptions::new(8, vec![stop]),
        )
        .expect("stop inmediato");
    assert_eq!(tokens, prompt);
    assert_eq!(report.generated, 0);
    assert_eq!(report.sampled_tokens, 1);
    assert_eq!(report.stop, StopReason::StopToken(stop));
}

#[test]
fn max_uno_es_limit() {
    let (mut rt, mut source) = setup();
    let prompt = [5u32, 10];
    let (tokens, report) = rt
        .generate_with_report(
            &mut source,
            &prompt,
            GenerationOptions::new(1, vec![]),
        )
        .expect("max 1");
    assert_eq!(tokens.len(), prompt.len() + 1);
    assert_eq!(report.generated, 1);
    assert_eq!(report.sampled_tokens, 1);
    assert_eq!(report.stop, StopReason::Limit);
}

#[test]
fn tope_de_contexto() {
    let (mut manifest, index, mapper) = tiny_model();
    manifest.max_seq = 6;
    let mut rt = Runtime::new(manifest, index.clone(), 0, 0);
    rt.validate_shapes().expect("shapes");
    let mut source = MmapTensorSource::new(String::from(BASE), index, mapper);
    let prompt = [1u32, 2, 3, 4, 5];
    let (tokens, report) = rt
        .generate_with_report(
            &mut source,
            &prompt,
            GenerationOptions::new(50, vec![]),
        )
        .expect("ctx");
    assert!(tokens.len() > prompt.len());
    assert!(tokens.len() - prompt.len() <= 2);
    assert_eq!(report.stop, StopReason::ContextLimit);
    assert_eq!(
        report.generated,
        (tokens.len() - prompt.len()) as u32
    );
}

#[test]
fn varios_stop_ids() {
    let (mut rt, mut source) = setup();
    let prompt = [10u32, 20];
    let a = primer_token_nuevo(&mut rt, &mut source, &prompt);
    let (tokens, report) = rt
        .generate_with_report(
            &mut source,
            &prompt,
            GenerationOptions::new(4, vec![a, 999]),
        )
        .expect("multi stop");
    assert_eq!(tokens.len(), prompt.len());
    assert!(matches!(report.stop, StopReason::StopToken(t) if t == a));
}

#[test]
fn draft_greedy_cuenta_cada_token() {
    let (mut rt, mut source) = setup();
    let prompt: Vec<u32> = (0..12).collect();
    let legacy = rt
        .generate(&mut source, &prompt, 6, None)
        .expect("legacy");
    let (con_reporte, report) = rt
        .generate_with_report(
            &mut source,
            &prompt,
            GenerationOptions::new(6, vec![]),
        )
        .expect("report");
    assert_eq!(legacy, con_reporte);
    assert_eq!(report.sampled_tokens, report.generated);
    assert!(report.generated <= 6);
}
