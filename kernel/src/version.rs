//! Versión del kernel inyectada en build por xtask (`SOSO_VERSION`, `SOSO_BUILD`).

const DEFAULT_VERSION: &str = "0.0.0-dev";
const DEFAULT_BUILD: &str = "unknown";

pub fn version() -> &'static str {
    option_env!("SOSO_VERSION").unwrap_or(DEFAULT_VERSION)
}

pub fn build() -> &'static str {
    option_env!("SOSO_BUILD").unwrap_or(DEFAULT_BUILD)
}

/// `"version build"` para banner y syscall.
pub fn full() -> alloc::string::String {
    alloc::format!("{} {}", version(), build())
}
