//! `POST /v1/systemone` (subset TypeSafe / Kev).

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use soso_llm_core::kev::{KevEngine, KevQuestionKind, KevQuestionWire};
use soso_llm_core::layer::TensorSource;
use soso_llm_core::runtime::Runtime;
use soso_llm_core::tokenizer::Tokenizer;

#[derive(Debug)]
pub enum SystemOneError {
    Json(String),
    Modelo(String),
    Inferencia(String),
}

pub fn run_systemone(
    body: &str,
    rt: &mut Runtime,
    tok: &Tokenizer,
    source: &mut impl TensorSource,
) -> Result<String, SystemOneError> {
    let v: serde_json::Value =
        serde_json::from_str(body).map_err(|e| SystemOneError::Json(e.to_string()))?;
    let state = v
        .get("state")
        .map(state_to_string)
        .ok_or_else(|| SystemOneError::Json("falta state".into()))?;
    let questions_val = v
        .get("questions")
        .and_then(|q| q.as_object())
        .ok_or_else(|| SystemOneError::Json("falta questions".into()))?;

    let mut questions = Vec::new();
    for (key, qv) in questions_val {
        let ty = qv
            .get("type")
            .and_then(|t| t.as_str())
            .ok_or_else(|| SystemOneError::Json(format!("questions.{key}: falta type")))?;
        let instructions = qv
            .get("instructions")
            .map(value_to_string)
            .unwrap_or_default();
        let (kind, options) = match ty {
            "noul" => (KevQuestionKind::Noul, Vec::new()),
            "choice" => {
                let crit = qv.get("criteria").ok_or_else(|| {
                    SystemOneError::Json(format!("questions.{key}: falta criteria"))
                })?;
                (KevQuestionKind::Choice, parse_criteria(crit)?)
            }
            "score" => {
                let crit = qv.get("criteria").ok_or_else(|| {
                    SystemOneError::Json(format!("questions.{key}: falta criteria"))
                })?;
                (KevQuestionKind::Score, parse_criteria(crit)?)
            }
            other => {
                return Err(SystemOneError::Json(format!(
                    "questions.{key}: type {other} no soportado"
                )));
            }
        };
        questions.push((
            key.clone(),
            KevQuestionWire {
                kind,
                instructions,
                options,
            },
        ));
    }

    if rt.manifest.kev.is_none() {
        return Err(SystemOneError::Modelo(
            "este checkpoint no es un modelo Kev".into(),
        ));
    }

    let mut engine = KevEngine::new(rt).map_err(SystemOneError::Modelo)?;
    let answers = engine
        .decide(tok, source, &state, &questions)
        .map_err(SystemOneError::Inferencia)?;

    let mut out = serde_json::Map::new();
    out.insert("model".into(), serde_json::Value::String("kev".into()));
    let mut ans = serde_json::Map::new();
    for (key, q) in answers {
        let mut entry = serde_json::Map::new();
        match q.kind {
            KevQuestionKind::Noul => {
                entry.insert("type".into(), "noul".into());
                let p_true = q.probs.first().copied().unwrap_or(0.0);
                entry.insert("noul".into(), serde_json::json!(p_true));
            }
            KevQuestionKind::Choice => {
                entry.insert("type".into(), "choice".into());
                let (best_i, best_p) = argmax(&q.probs);
                let choice = q.option_labels.get(best_i).cloned().unwrap_or_default();
                entry.insert("choice".into(), choice.clone().into());
                entry.insert("confidence".into(), best_p.into());
                entry.insert(
                    "probabilities".into(),
                    prob_map(&q.option_labels, &q.probs),
                );
            }
            KevQuestionKind::Score => {
                entry.insert("type".into(), "score".into());
                let (best_i, best_p) = argmax(&q.probs);
                entry.insert("score".into(), (best_i as f64).into());
                entry.insert("confidence".into(), best_p.into());
                entry.insert(
                    "probabilities".into(),
                    prob_map(&q.option_labels, &q.probs),
                );
            }
        }
        ans.insert(key, entry.into());
    }
    out.insert("answers".into(), ans.into());
    serde_json::to_string(&out).map_err(|e| SystemOneError::Json(e.to_string()))
}

fn argmax(p: &[f32]) -> (usize, f64) {
    let mut best = 0usize;
    let mut v = f32::NEG_INFINITY;
    for (i, &x) in p.iter().enumerate() {
        if x > v {
            v = x;
            best = i;
        }
    }
    (best, v as f64)
}

fn prob_map(labels: &[String], probs: &[f32]) -> serde_json::Value {
    let mut m = serde_json::Map::new();
    for (i, lab) in labels.iter().enumerate() {
        if let Some(&p) = probs.get(i) {
            m.insert(lab.clone(), (p as f64).into());
        }
    }
    m.into()
}

fn parse_criteria(crit: &serde_json::Value) -> Result<Vec<(String, Option<String>)>, SystemOneError> {
    if let Some(arr) = crit.as_array() {
        return Ok(arr
            .iter()
            .filter_map(|v| v.as_str().map(|s| (s.to_string(), None)))
            .collect());
    }
    if let Some(obj) = crit.as_object() {
        return Ok(obj
            .iter()
            .map(|(k, v)| {
                let desc = match v {
                    serde_json::Value::Null => None,
                    other => Some(value_to_string(other)),
                };
                (k.clone(), desc)
            })
            .collect());
    }
    Err(SystemOneError::Json("criteria inválido".into()))
}

fn state_to_string(v: &serde_json::Value) -> String {
    value_to_string(v)
}

fn value_to_string(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}
