//! Reenvío slirp del API LLM guest (T18, C3–C4): 127.0.0.1:host → guest:7422.

pub const GUEST_LLM_HTTP_PORT: u16 = 7422;

const MIN_HOST_PORT: u16 = 1024;

/// Puertos slirp de una instancia QEMU (user netdev).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SlirpForwards {
    pub echo_port: u16,
    pub ssh_port: u16,
    /// Reenvío API en loopback del host; `None` = sin forward LLM.
    pub llm_host_port: Option<u16>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LlmPortError {
    InvalidPort(u16),
    Collision { label_a: &'static str, label_b: &'static str, port: u16 },
}

impl std::fmt::Display for LlmPortError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LlmPortError::InvalidPort(p) => write!(f, "puerto inválido {p}"),
            LlmPortError::Collision {
                label_a,
                label_b,
                port,
            } => write!(
                f,
                "colisión: {label_a} y {label_b} comparten el puerto {port}"
            ),
        }
    }
}

impl std::error::Error for LlmPortError {}

/// Host distinto por shard de test (no usar 7422 global en paralelo).
pub fn test_shard_llm_host_port(shard_index: u8) -> u16 {
    17_222 + 10 * u16::from(shard_index)
}

pub fn validate_port(port: u16) -> Result<(), LlmPortError> {
    if port < MIN_HOST_PORT {
        return Err(LlmPortError::InvalidPort(port));
    }
    Ok(())
}

pub fn validate_forwards(f: &SlirpForwards) -> Result<(), LlmPortError> {
    validate_port(f.echo_port)?;
    validate_port(f.ssh_port)?;
    if f.echo_port == f.ssh_port {
        return Err(LlmPortError::Collision {
            label_a: "echo",
            label_b: "ssh",
            port: f.echo_port,
        });
    }
    if let Some(llm) = f.llm_host_port {
        validate_port(llm)?;
        if llm == f.echo_port {
            return Err(LlmPortError::Collision {
                label_a: "llm",
                label_b: "echo",
                port: llm,
            });
        }
        if llm == f.ssh_port {
            return Err(LlmPortError::Collision {
                label_a: "llm",
                label_b: "ssh",
                port: llm,
            });
        }
    }
    Ok(())
}

/// Argumento completo `-netdev user,...` (sin VFIO).
pub fn build_user_netdev(f: &SlirpForwards) -> Result<String, LlmPortError> {
    validate_forwards(f)?;
    let mut fwd = vec![
        format!("hostfwd=tcp::{echo}-:7", echo = f.echo_port),
        format!("hostfwd=tcp::{ssh}-:22", ssh = f.ssh_port),
    ];
    if let Some(h) = f.llm_host_port {
        fwd.push(format!(
            "hostfwd=tcp:127.0.0.1:{h}-:{guest}",
            guest = GUEST_LLM_HTTP_PORT
        ));
    }
    Ok(format!("user,id=net0,{}", fwd.join(",")))
}

/// `SOSO_LLM_HOST_PORT` para `cargo xtask run` (opcional).
pub fn llm_host_port_from_env() -> Option<u16> {
    let raw = std::env::var("SOSO_LLM_HOST_PORT").ok()?;
    let port: u16 = raw.parse().ok()?;
    validate_port(port).ok()?;
    Some(port)
}

pub fn describe_forwards(f: &SlirpForwards) -> String {
    let llm = f
        .llm_host_port
        .map(|h| format!("127.0.0.1:{h}→guest:{GUEST_LLM_HTTP_PORT}"))
        .unwrap_or_else(|| "sin API".into());
    format!(
        "echo:{} ssh:{} llm:{llm}",
        f.echo_port, f.ssh_port
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> SlirpForwards {
        SlirpForwards {
            echo_port: 7777,
            ssh_port: 2222,
            llm_host_port: None,
        }
    }

    #[test]
    fn sin_api_conserva_echo_ssh() {
        let s = build_user_netdev(&base()).unwrap();
        assert!(s.contains("hostfwd=tcp::7777-:7"));
        assert!(s.contains("hostfwd=tcp::2222-:22"));
        assert!(!s.contains("127.0.0.1"));
        assert!(!s.contains("7422"));
    }

    #[test]
    fn api_explicita_loopback() {
        let mut f = base();
        f.llm_host_port = Some(17422);
        let s = build_user_netdev(&f).unwrap();
        assert!(s.contains("hostfwd=tcp:127.0.0.1:17422-:7422"));
    }

    #[test]
    fn dos_instancias_distintas() {
        let a = SlirpForwards {
            echo_port: 7700,
            ssh_port: 2200,
            llm_host_port: Some(test_shard_llm_host_port(0)),
        };
        let b = SlirpForwards {
            echo_port: 7710,
            ssh_port: 2210,
            llm_host_port: Some(test_shard_llm_host_port(1)),
        };
        let sa = build_user_netdev(&a).unwrap();
        let sb = build_user_netdev(&b).unwrap();
        assert!(sa.contains(":17222-:7422"));
        assert!(sb.contains(":17232-:7422"));
        assert_ne!(sa, sb);
    }

    #[test]
    fn puerto_invalido() {
        let mut f = base();
        f.echo_port = 22;
        assert!(matches!(
            build_user_netdev(&f),
            Err(LlmPortError::InvalidPort(22))
        ));
    }

    #[test]
    fn colision_ssh_llm() {
        let f = SlirpForwards {
            echo_port: 7777,
            ssh_port: 9000,
            llm_host_port: Some(9000),
        };
        assert!(matches!(
            validate_forwards(&f),
            Err(LlmPortError::Collision { .. })
        ));
    }

    #[test]
    fn vfio_no_netdev() {
        // Documentación: VFIO no usa slirp; el test de integración solo comprueba
        // que build_user_netdev sigue siendo la ruta slirp (virtio/e1000e).
        assert!(build_user_netdev(&base()).unwrap().starts_with("user,id=net0"));
    }
}
