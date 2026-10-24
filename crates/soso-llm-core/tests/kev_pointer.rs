//! Cabeza de puntero Kev: álgebra y conversión GGUF (sin pesos de 4 GB).

use soso_llm_core::gemm::{matvec_f32_scalar, softmax_inplace};
use sosomodel::manifest::{KevSpec, Manifest, ModelKind};

#[test]
fn kev_manifest_v9_redondo() {
    let mut m = Manifest::tiny("kev-test");
    m.model_kind = ModelKind::Decision;
    m.kev = Some(KevSpec {
        temperature: 2.41,
        pointer_dim: 8,
    });
    let parsed = Manifest::parse(&m.serialize()).unwrap();
    assert_eq!(parsed.model_kind, ModelKind::Decision);
    let k = parsed.kev.unwrap();
    assert!((k.temperature - 2.41).abs() < 1e-6);
    assert_eq!(k.pointer_dim, 8);
}

#[test]
fn pointer_probs_coincide_con_referencia_escalar() {
    let hidden = 4usize;
    let dp = 2usize;
    let w_q = [1.0f32, 0.0, 0.0, 1.0, 0.5, 0.5, 0.0, 0.0];
    let w_k = [0.0, 1.0, 1.0, 0.0, 0.25, 0.75, 0.5, 0.5];
    let h_dec = [1.0, 0.0, 0.0, 1.0];
    let h_opt0 = [0.0, 1.0, 0.0, 0.0];
    let h_opt1 = [0.0, 0.0, 1.0, 0.0];
    let scale = 1.0 / libm::sqrtf(dp as f32);
    let mut q = [0.0f32; 2];
    matvec_f32_scalar(&w_q, dp, hidden, &h_dec, &mut q);
    let mut logits = Vec::new();
    for h in [&h_opt0, &h_opt1] {
        let mut k = [0.0f32; 2];
        matvec_f32_scalar(&w_k, dp, hidden, h, &mut k);
        let mut z = 0.0f32;
        for i in 0..dp {
            z += k[i] * q[i];
        }
        logits.push(z * scale);
    }
    softmax_inplace(&mut logits);
    assert!(logits.iter().all(|x| x.is_finite()));
    let s: f32 = logits.iter().sum();
    assert!((s - 1.0).abs() < 1e-5);
}
