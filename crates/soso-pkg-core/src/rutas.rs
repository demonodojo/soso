//! Validación de nombres de paquete y rutas internas.

use alloc::string::String;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RutaError {
    NombreVacio,
    NombreInvalido,
    RutaVacia,
    RutaAbsoluta,
    ComponenteInvalido,
    ParentDir,
    PrefijoNoPermitido,
}

impl core::fmt::Display for RutaError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NombreVacio => write!(f, "nombre de paquete vacío"),
            Self::NombreInvalido => {
                write!(f, "nombre inválido (use [a-z0-9][a-z0-9_-]{{0,31}})")
            }
            Self::RutaVacia => write!(f, "ruta vacía"),
            Self::RutaAbsoluta => write!(f, "ruta absoluta no permitida"),
            Self::ComponenteInvalido => write!(f, "componente de ruta inválido"),
            Self::ParentDir => write!(f, "«..» no permitido"),
            Self::PrefijoNoPermitido => write!(f, "ruta debe empezar por bin/ o share/"),
        }
    }
}

fn componente_ok(c: &str) -> bool {
    !c.is_empty()
        && c.as_bytes()
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
}

/// `[a-z0-9][a-z0-9_-]{0,31}`
pub fn nombre_valido(n: &str) -> Result<(), RutaError> {
    if n.is_empty() {
        return Err(RutaError::NombreVacio);
    }
    if n.len() > 32 {
        return Err(RutaError::NombreInvalido);
    }
    let mut it = n.bytes();
    let first = it.next().unwrap_or(b' ');
    if !first.is_ascii_lowercase() && !first.is_ascii_digit() {
        return Err(RutaError::NombreInvalido);
    }
    for b in it {
        if !(b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_') {
            return Err(RutaError::NombreInvalido);
        }
    }
    Ok(())
}

/// Ruta relativa dentro del paquete: solo `bin/` o `share/`.
pub fn ruta_valida(rel: &str) -> Result<(), RutaError> {
    if rel.is_empty() {
        return Err(RutaError::RutaVacia);
    }
    if rel.starts_with('/') || rel.contains('\\') {
        return Err(RutaError::RutaAbsoluta);
    }
    if rel.split('/').any(|p| p == "..") {
        return Err(RutaError::ParentDir);
    }
    if !(rel.starts_with("bin/") || rel.starts_with("share/")) {
        return Err(RutaError::PrefijoNoPermitido);
    }
    for c in rel.split('/') {
        if c.is_empty() {
            continue;
        }
        if !componente_ok(c) {
            return Err(RutaError::ComponenteInvalido);
        }
    }
    Ok(())
}

/// Nombre del ejecutable en `bin/foo` → `foo`.
pub fn nombre_bin(rel: &str) -> Option<String> {
    ruta_valida(rel).ok()?;
    let rest = rel.strip_prefix("bin/")?;
    if rest.contains('/') {
        return None;
    }
    Some(rest.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nombres() {
        assert!(nombre_valido("hola").is_ok());
        assert!(nombre_valido("hola-demo_1").is_ok());
        assert!(nombre_valido("Hola").is_err());
        assert!(nombre_valido("-x").is_err());
    }

    #[test]
    fn rutas() {
        assert!(ruta_valida("bin/demo").is_ok());
        assert!(ruta_valida("share/doc/readme.txt").is_ok());
        assert!(ruta_valida("etc/x").is_err());
        assert!(ruta_valida("bin/../etc").is_err());
    }
}
