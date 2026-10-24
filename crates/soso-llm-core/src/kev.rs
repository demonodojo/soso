//! Inferencia Kev / System One: tronco causal + cabeza de puntero (sin generación).

use crate::gemm::{matvec_f32_scalar, softmax_inplace};
use crate::layer::TensorSource;
use crate::runtime::Runtime;
use crate::tokenizer::Tokenizer;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use sosomodel::manifest::KevSpec;

const SPECIAL: [&str; 5] = [
    "<|fim_prefix|>",
    "<|fim_middle|>",
    "<|box_start|>",
    "<|box_end|>",
    "<|fim_suffix|>",
];

fn zeros_f32(n: usize) -> Vec<f32> {
    let mut v = Vec::new();
    v.resize(n, 0.0);
    v
}

pub struct KevEncodedQuestion {
    pub probs: Vec<f32>,
    pub option_labels: Vec<String>,
    pub kind: KevQuestionKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KevQuestionKind {
    Noul,
    Choice,
    Score,
}

struct KevQuestionEnc {
    q_start: usize,
    decide: usize,
    opt_ends: Vec<usize>,
    kind: KevQuestionKind,
    option_labels: Vec<String>,
}

struct KevRecord {
    state_end: usize,
    questions: Vec<KevQuestionEnc>,
    ids: Vec<u32>,
}

fn piece_id(tok: &Tokenizer, piece: &str) -> Result<u32, String> {
    tok.piece_id(piece).ok_or_else(|| format!("token especial ausente: {piece}"))
}

fn user_tokens(tok: &Tokenizer, text: &str) -> Vec<u32> {
    tok.encode_trozo(&sanitize_user_text(text), false, false)
}

fn sanitize_user_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'<' && i + 1 < bytes.len() && bytes[i + 1] == b'|' {
            if let Some(end) = text[i..].find("|>") {
                let inner = &text[i + 2..i + end];
                out.push('<');
                out.push('¦');
                out.push_str(inner);
                out.push('¦');
                out.push('>');
                i += end + 2;
                continue;
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

fn encode_record(tok: &Tokenizer, state: &str, questions: &[(String, KevQuestionWire)]) -> Result<KevRecord, String> {
    let (s0, q_id, o_id, c_id, d_id) = (
        piece_id(tok, SPECIAL[0])?,
        piece_id(tok, SPECIAL[1])?,
        piece_id(tok, SPECIAL[2])?,
        piece_id(tok, SPECIAL[3])?,
        piece_id(tok, SPECIAL[4])?,
    );
    let mut ids = Vec::new();
    ids.push(s0);
    ids.extend(user_tokens(tok, state));
    let state_end = ids.len();
    let mut enc_q = Vec::new();
    for (_key, q) in questions {
        let q_start = ids.len();
        ids.push(q_id);
        ids.extend(user_tokens(tok, &q.instructions));
        let (labels, opt_spans): (Vec<String>, Vec<Vec<u32>>) = match q.kind {
            KevQuestionKind::Noul => {
                let a = user_tokens(tok, "true");
                let b = user_tokens(tok, "false");
                {
                    let mut s0 = Vec::from([o_id]);
                    s0.extend(a);
                    s0.push(c_id);
                    let mut s1 = Vec::from([o_id]);
                    s1.extend(b);
                    s1.push(c_id);
                    (
                        Vec::from([String::from("true"), String::from("false")]),
                        Vec::from([s0, s1]),
                    )
                }
            }
            KevQuestionKind::Choice | KevQuestionKind::Score => {
                let mut labels = Vec::new();
                let mut spans = Vec::new();
                for (name, desc) in &q.options {
                    labels.push(name.clone());
                    let mut span = Vec::from([o_id]);
                    if let Some(d) = desc {
                        span.extend(user_tokens(tok, d));
                    } else {
                        span.extend(user_tokens(tok, name));
                    }
                    span.push(c_id);
                    spans.push(span);
                }
                (labels, spans)
            }
        };
        let mut opt_ends = Vec::new();
        for sp in &opt_spans {
            let end = ids.len() + sp.len() - 1;
            ids.extend_from_slice(sp);
            opt_ends.push(end);
        }
        ids.push(d_id);
        let decide = ids.len() - 1;
        enc_q.push(KevQuestionEnc {
            q_start,
            decide,
            opt_ends,
            kind: q.kind,
            option_labels: labels,
        });
    }
    Ok(KevRecord {
        state_end,
        questions: enc_q,
        ids,
    })
}

/// Petición de una pregunta (wire simplificado).
pub struct KevQuestionWire {
    pub kind: KevQuestionKind,
    pub instructions: String,
    /// `(nombre, descripción opcional)` por opción.
    pub options: Vec<(String, Option<String>)>,
}

pub struct KevEngine<'a> {
    pub rt: &'a mut Runtime,
    kev: KevSpec,
}

impl<'a> KevEngine<'a> {
    pub fn new(rt: &'a mut Runtime) -> Result<Self, String> {
        let kev = rt
            .manifest
            .kev
            .ok_or_else(|| String::from("el modelo no trae cabeza Kev"))?;
        Ok(Self { rt, kev })
    }

    pub fn decide(
        &mut self,
        tok: &Tokenizer,
        source: &mut impl TensorSource,
        state: &str,
        questions: &[(String, KevQuestionWire)],
    ) -> Result<Vec<(String, KevEncodedQuestion)>, String> {
        let enc = encode_record(tok, state, questions)?;
        self.rt.reset_sequence();
        self.forward_ids(source, &enc.ids[..enc.state_end])?;
        let prefix = self.rt.snapshot_kev_prefix();

        let mut out = Vec::new();
        for (qi, (key, _)) in questions.iter().enumerate() {
            let qe = &enc.questions[qi];
            let branch = &enc.ids[qe.q_start..=qe.decide];
            self.rt.restore_kev_prefix(&prefix);

            let hidden = self.rt.manifest.hidden_dim as usize;
            let mut h_decide = zeros_f32(hidden);
            let mut h_opts: Vec<Vec<f32>> = qe
                .opt_ends
                .iter()
                .map(|_| zeros_f32(hidden))
                .collect();

            for (j, &tid) in branch.iter().enumerate() {
                let abs = qe.q_start + j;
                self.rt
                    .embed_token(tid, source)
                    .map_err(|_| String::from("embed"))?;
                self.rt
                    .forward_step(source)
                    .map_err(|_| String::from("forward"))?;
                if abs == qe.decide {
                    h_decide.copy_from_slice(self.rt.hidden_slice());
                }
                for (oi, &end_abs) in qe.opt_ends.iter().enumerate() {
                    if abs == end_abs {
                        h_opts[oi].copy_from_slice(self.rt.hidden_slice());
                    }
                }
            }

            let probs = pointer_probs(source, self.kev, &h_decide, &h_opts)?;
            out.push((
                key.clone(),
                KevEncodedQuestion {
                    probs,
                    option_labels: qe.option_labels.clone(),
                    kind: qe.kind,
                },
            ));
        }
        Ok(out)
    }

    fn forward_ids(&mut self, source: &mut impl TensorSource, ids: &[u32]) -> Result<(), String> {
        for &tid in ids {
            self.rt
                .embed_token(tid, source)
                .map_err(|_| String::from("embed"))?;
            self.rt
                .forward_step(source)
                .map_err(|_| String::from("forward"))?;
        }
        Ok(())
    }
}

fn pointer_probs(
    source: &mut impl TensorSource,
    kev: KevSpec,
    h_decide: &[f32],
    h_opts: &[Vec<f32>],
) -> Result<Vec<f32>, String> {
    let hidden = h_decide.len();
    let dp = kev.pointer_dim as usize;
    let mut w_q = zeros_f32(dp * hidden);
    let mut w_k = zeros_f32(dp * hidden);
    source
        .load_f32("kev.head_q", &mut w_q)
        .map_err(|_| String::from("falta kev.head_q"))?;
    source
        .load_f32("kev.head_k", &mut w_k)
        .map_err(|_| String::from("falta kev.head_k"))?;

    let mut q_vec = zeros_f32(dp);
    let mut k_vec = zeros_f32(dp);
    matvec_f32_scalar(&w_q, dp, hidden, h_decide, &mut q_vec);
    let scale = 1.0 / libm::sqrtf(dp as f32);
    let mut logits = Vec::with_capacity(h_opts.len());
    for h_opt in h_opts {
        matvec_f32_scalar(&w_k, dp, hidden, h_opt, &mut k_vec);
        let mut z = 0.0f32;
        for i in 0..dp {
            z += k_vec[i] * q_vec[i];
        }
        z *= scale;
        if kev.temperature > 0.0 && kev.temperature != 1.0 {
            z /= kev.temperature;
        }
        logits.push(z);
    }
    softmax_inplace(&mut logits);
    Ok(logits)
}
