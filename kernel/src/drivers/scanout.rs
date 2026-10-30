//! Router de presentación (F8 / N0): GOP vs NVDisplay.
//!
//! `GPRES` sigue siendo explícitamente GOP. `GSCEN`/`GFRAM` y `GNVD` consultan
//! la política (`SOSO_SCANOUT` en build) y las capacidades en `GpuInfo::_pad`.

use soso_abi as abi;

/// Política fijada en compilación (`SOSO_SCANOUT=auto|gop|nvdisp`).
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

fn compile_policy() -> ScanoutPolicy {
    match option_env!("SOSO_SCANOUT") {
        Some("gop") => ScanoutPolicy::Gop,
        Some("nvdisp") | Some("nv") => ScanoutPolicy::Nvdisp,
        Some("auto") | None => ScanoutPolicy::Auto,
        Some(_) => ScanoutPolicy::Auto,
    }
}

fn nvdisp_capable() -> bool {
    #[cfg(feature = "lxdde")]
    {
        crate::lxdde::nvdisp_ready()
    }
    #[cfg(not(feature = "lxdde"))]
    {
        false
    }
}

fn gop_capable() -> bool {
    super::fb::available()
}

/// Backend que debe usarse tras composición (`GSCEN` / `GFRAM`) o `GNVD`.
pub fn effective_backend() -> Result<ScanoutBackend, i64> {
    match compile_policy() {
        ScanoutPolicy::Gop => {
            if gop_capable() {
                Ok(ScanoutBackend::Gop)
            } else {
                Err(abi::ENOSYS)
            }
        }
        ScanoutPolicy::Nvdisp => {
            if nvdisp_capable() {
                Ok(ScanoutBackend::Nvdisp)
            } else {
                Err(abi::ENOTSUP)
            }
        }
        ScanoutPolicy::Auto => {
            if nvdisp_capable() {
                Ok(ScanoutBackend::Nvdisp)
            } else if gop_capable() {
                Ok(ScanoutBackend::Gop)
            } else {
                Err(abi::ENOSYS)
            }
        }
    }
}

/// Rellena `GpuInfo::_pad` (ver `soso_abi::GPU_PAD_*`).
pub fn info_pad(gfx3d: u8) -> [u8; 4] {
    let effective = match effective_backend() {
        Ok(ScanoutBackend::Nvdisp) => abi::GPU_SCANOUT_EFFECTIVE_NVDISP,
        Ok(ScanoutBackend::Gop) => abi::GPU_SCANOUT_EFFECTIVE_GOP,
        Err(_) => abi::GPU_SCANOUT_EFFECTIVE_NONE,
    };
    [
        gfx3d,
        nvdisp_capable() as u8,
        gop_capable() as u8,
        effective,
    ]
}

pub fn boot_log() {
    let pol = compile_policy();
    let pol_s = match pol {
        ScanoutPolicy::Auto => "auto",
        ScanoutPolicy::Gop => "gop",
        ScanoutPolicy::Nvdisp => "nvdisp",
    };
    let backend = match effective_backend() {
        Ok(ScanoutBackend::Gop) => "gop",
        Ok(ScanoutBackend::Nvdisp) => "nvdisp",
        Err(_) => "none",
    };
    let mode_s = if nvdisp_capable() {
        "nvdisp_head=640x480@60"
    } else if gop_capable() {
        "gop_console"
    } else {
        "-"
    };
    crate::klogln!(
        "scanout: policy={pol_s} gop={} nvdisp={} effective={backend} mode={mode_s}",
        gop_capable() as u8,
        nvdisp_capable() as u8,
    );
}

