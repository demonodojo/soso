//! Router GOP vs NVDisplay (N0/N7): misma lógica que `kernel/src/drivers/scanout.rs`.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScanoutPolicy {
    Auto,
    Gop,
    Nvdisp,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScanoutBackend {
    Gop,
    Nvdisp,
}

/// Backend efectivo tras política + capacidades (sin errno: `None` = ENOSYS/ENOTSUP).
pub fn effective_backend(
    policy: ScanoutPolicy,
    nvdisp_capable: bool,
    gop_capable: bool,
) -> Option<ScanoutBackend> {
    match policy {
        ScanoutPolicy::Gop => {
            if gop_capable {
                Some(ScanoutBackend::Gop)
            } else {
                None
            }
        }
        ScanoutPolicy::Nvdisp => {
            if nvdisp_capable {
                Some(ScanoutBackend::Nvdisp)
            } else {
                None
            }
        }
        ScanoutPolicy::Auto => {
            if nvdisp_capable {
                Some(ScanoutBackend::Nvdisp)
            } else if gop_capable {
                Some(ScanoutBackend::Gop)
            } else {
                None
            }
        }
    }
}

/// Codificación en `GpuInfo._pad[3]` (`GPU_SCANOUT_EFFECTIVE_*`).
pub fn effective_tag(
    policy: ScanoutPolicy,
    nvdisp_capable: bool,
    gop_capable: bool,
) -> u8 {
    match effective_backend(policy, nvdisp_capable, gop_capable) {
        Some(ScanoutBackend::Nvdisp) => 1,
        Some(ScanoutBackend::Gop) => 2,
        None => 0,
    }
}

/// `GNVD` en el kernel exige backend NVDisplay (con `SOSO_SCANOUT=gop` → ENOTSUP).
pub fn gnvd_allowed(
    policy: ScanoutPolicy,
    nvdisp_capable: bool,
    gop_capable: bool,
) -> bool {
    matches!(
        effective_backend(policy, nvdisp_capable, gop_capable),
        Some(ScanoutBackend::Nvdisp)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_prefers_nvdisp_when_both() {
        assert_eq!(
            effective_backend(ScanoutPolicy::Auto, true, true),
            Some(ScanoutBackend::Nvdisp)
        );
    }

    #[test]
    fn gop_regression_forces_shadow_when_both_capable() {
        assert_eq!(
            effective_backend(ScanoutPolicy::Gop, true, true),
            Some(ScanoutBackend::Gop)
        );
        assert!(!gnvd_allowed(ScanoutPolicy::Gop, true, true));
        assert_eq!(effective_tag(ScanoutPolicy::Gop, true, true), 2);
    }

    #[test]
    fn nvdisp_policy_fails_without_head() {
        assert!(effective_backend(ScanoutPolicy::Nvdisp, false, true).is_none());
    }

    #[test]
    fn auto_falls_back_to_gop_in_qemu() {
        assert_eq!(
            effective_backend(ScanoutPolicy::Auto, false, true),
            Some(ScanoutBackend::Gop)
        );
    }
}
