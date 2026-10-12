//! Comparación semver numérica `major.minor.patch`.

use core::cmp::Ordering;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SemVer {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

/// Parsea `major.minor.patch` (ignora sufijos tras `-` o `+`).
pub fn parse(s: &str) -> Option<SemVer> {
    let base = s.split(['-', '+']).next()?.trim();
    let mut it = base.split('.');
    let major = it.next()?.parse().ok()?;
    let minor = it.next()?.parse().ok()?;
    let patch = it.next()?.parse().ok()?;
    Some(SemVer {
        major,
        minor,
        patch,
    })
}

pub fn cmp(a: &SemVer, b: &SemVer) -> Ordering {
    a.major
        .cmp(&b.major)
        .then_with(|| a.minor.cmp(&b.minor))
        .then_with(|| a.patch.cmp(&b.patch))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordena() {
        let a = parse("0.2.0").unwrap();
        let b = parse("0.10.0").unwrap();
        assert_eq!(cmp(&a, &b), Ordering::Less);
        assert_eq!(cmp(&b, &a), Ordering::Greater);
        assert_eq!(cmp(&a, &parse("0.2.0").unwrap()), Ordering::Equal);
    }
}
