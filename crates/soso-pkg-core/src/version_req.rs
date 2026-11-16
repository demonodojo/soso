//! Restricciones de versiones para dependencias de paquetes.

use core::cmp::Ordering;
use soso_update_core::{SemVer, parse_semver, semver_cmp};

/// Comprueba una restricción exacta, comparador, caret, tilde o conjunción.
/// Ejemplos: `1.2.3`, `>=1.2.0`, `^0.5.1`, `~1.4.0`, `>=1.2.0,<2.0.0`.
pub fn satisface(version: &SemVer, req: &str) -> bool {
    let mut clauses = req.split(',');
    let mut any = false;
    for clause in &mut clauses {
        any = true;
        if !satisface_clausula(version, clause.trim()) {
            return false;
        }
    }
    any
}

pub fn valida(req: &str) -> bool {
    if req == "*" {
        return true;
    }
    let mut any = false;
    for clause in req.split(',') {
        any = true;
        let c = clause.trim();
        let parsed = c
            .strip_prefix('^')
            .or_else(|| c.strip_prefix('~'))
            .map(parse_semver)
            .unwrap_or_else(|| version_clausula(c).map(|(_, v)| v));
        if c.is_empty() || c == "*" || parsed.is_none() {
            return false;
        }
    }
    any
}

fn satisface_clausula(version: &SemVer, clause: &str) -> bool {
    if clause == "*" {
        return true;
    }
    if let Some(raw) = clause.strip_prefix('^') {
        let Some(lower) = parse_semver(raw) else {
            return false;
        };
        let same_compat = if lower.major != 0 {
            version.major == lower.major
        } else if lower.minor != 0 {
            version.major == 0 && version.minor == lower.minor
        } else {
            version.major == 0 && version.minor == 0 && version.patch == lower.patch
        };
        return semver_cmp(version, &lower) != Ordering::Less && same_compat;
    }
    if let Some(raw) = clause.strip_prefix('~') {
        let Some(lower) = parse_semver(raw) else {
            return false;
        };
        return semver_cmp(version, &lower) != Ordering::Less
            && version.major == lower.major
            && version.minor == lower.minor;
    }
    let Some((op, bound)) = version_clausula(clause) else {
        return false;
    };
    match (op, semver_cmp(version, &bound)) {
        (Op::Eq, Ordering::Equal)
        | (Op::Ge, Ordering::Equal | Ordering::Greater)
        | (Op::Gt, Ordering::Greater)
        | (Op::Le, Ordering::Equal | Ordering::Less)
        | (Op::Lt, Ordering::Less) => true,
        _ => false,
    }
}

#[derive(Clone, Copy)]
enum Op {
    Eq,
    Ge,
    Gt,
    Le,
    Lt,
}

fn version_clausula(c: &str) -> Option<(Op, SemVer)> {
    let (op, raw) = if let Some(v) = c.strip_prefix(">=") {
        (Op::Ge, v)
    } else if let Some(v) = c.strip_prefix("<=") {
        (Op::Le, v)
    } else if let Some(v) = c.strip_prefix('>') {
        (Op::Gt, v)
    } else if let Some(v) = c.strip_prefix('<') {
        (Op::Lt, v)
    } else if let Some(v) = c.strip_prefix('=') {
        (Op::Eq, v)
    } else {
        (Op::Eq, c)
    };
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    Some((op, parse_semver(raw)?))
}
