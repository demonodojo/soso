//! Catálogo y selección del modelo LLM para el live USB según capacidad del pendrive.

use std::path::{Path, PathBuf};
use std::process::{Command, exit};

use crate::fetch_hf;

/// Margen extra sobre el árbol `.som` al dimensionar la imagen sosomfs.
pub const MODELS_IMAGE_MARGIN: u64 = 256 * 1024 * 1024;

/// Reserva fija para la partición SOSOINSTALL al flashear.
pub const P4_INSTALL_BYTES: u64 = 32 * 1024 * 1024;

/// Holgura GPT + alineaciones al calcular el presupuesto de modelos.
pub const GPT_OVERHEAD_BYTES: u64 = 1024 * 1024;

/// Tamaño estimado del modelo sintético `tiny` empaquetado junto al demo.
pub const TINY_SOM_ESTIMATE: u64 = 64 * 1024 * 1024;

/// Entrada del catálogo live (GGUF llama, tokenizer SentencePiece).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LiveModelSpec {
    /// Nombre en `/models` y en `/etc/llm.conf`.
    pub name: &'static str,
    /// Repositorio Hugging Face (`org/repo`).
    pub repo: &'static str,
    /// Fichero GGUF concreto; `None` → `pick_gguf_file` elige Q4_K_M.
    pub gguf_file: Option<&'static str>,
    /// Tamaño GGUF estimado (bytes) para presupuesto antes de descargar.
    pub gguf_bytes_estimate: u64,
    /// Tamaño mínimo del pendrive (bytes) para considerar este modelo.
    pub min_usb_bytes: u64,
}

/// Catálogo ordenado de peor a mejor calidad (el picker recorre al revés).
pub const CATALOG: &[LiveModelSpec] = &[
    LiveModelSpec {
        name: "tinyllama",
        repo: "TinyLlama/TinyLlama-1.1B-Chat-v1.0",
        gguf_file: None,
        gguf_bytes_estimate: 700 * 1024 * 1024,
        min_usb_bytes: 8 * 1024 * 1024 * 1024,
    },
    LiveModelSpec {
        name: "mistral-7b",
        repo: "TheBloke/Mistral-7B-Instruct-v0.2-GGUF",
        gguf_file: None,
        gguf_bytes_estimate: 4_400 * 1024 * 1024,
        min_usb_bytes: 16 * 1024 * 1024 * 1024,
    },
    LiveModelSpec {
        name: "mixtral",
        repo: "TheBloke/Mixtral-8x7B-Instruct-v0.1-GGUF",
        gguf_file: None,
        gguf_bytes_estimate: 26 * 1024 * 1024 * 1024,
        min_usb_bytes: 32 * 1024 * 1024 * 1024,
    },
    LiveModelSpec {
        name: "llama2-70b",
        repo: "TheBloke/Llama-2-70B-Chat-GGUF",
        gguf_file: None,
        gguf_bytes_estimate: 39 * 1024 * 1024 * 1024,
        min_usb_bytes: 64 * 1024 * 1024 * 1024,
    },
];

impl LiveModelSpec {
    pub fn target_dir(&self, root: &Path) -> PathBuf {
        root.join(format!("target/{}-model", self.name))
    }

    /// Bytes necesarios en p3: modelo principal + `tiny` + margen de imagen.
    pub fn need_bytes(&self, root: &Path) -> u64 {
        let dir = self.target_dir(root);
        let model_bytes = if dir.join("manifest.som").exists() {
            dir_size_bytes(&dir)
        } else {
            (self.gguf_bytes_estimate * 115) / 100
        };
        model_bytes
            .saturating_add(TINY_SOM_ESTIMATE)
            .saturating_add(MODELS_IMAGE_MARGIN)
    }

    pub fn format_need(&self, root: &Path) -> String {
        format_bytes(self.need_bytes(root))
    }
}

/// Presupuesto de la partición 3 (modelos) dentro de un USB de `usb_bytes`.
pub fn models_budget_from_layout(usb_bytes: u64, esp_aligned: u64, rootfs_aligned: u64) -> u64 {
    usb_bytes.saturating_sub(
        P4_INSTALL_BYTES
            .saturating_add(esp_aligned)
            .saturating_add(rootfs_aligned)
            .saturating_add(GPT_OVERHEAD_BYTES),
    )
}

/// Elige el mejor modelo del catálogo que cabe en el USB indicado.
pub fn pick_for_usb(
    usb_bytes: u64,
    esp_aligned: u64,
    rootfs_aligned: u64,
    root: &Path,
) -> LiveModelSpec {
    let budget = models_budget_from_layout(usb_bytes, esp_aligned, rootfs_aligned);
    for spec in CATALOG.iter().rev() {
        if usb_bytes >= spec.min_usb_bytes && spec.need_bytes(root) <= budget {
            return *spec;
        }
    }
    CATALOG[0]
}

/// `SOSO_LIVE_OFFLINE=1|true|yes`: no descargar desde Hugging Face al empaquetar/flashear.
pub fn offline_mode() -> bool {
    matches!(
        std::env::var("SOSO_LIVE_OFFLINE").as_deref(),
        Ok("1") | Ok("true") | Ok("yes")
    )
}

pub fn is_materialized(root: &Path, spec: &LiveModelSpec) -> bool {
    spec.target_dir(root).join("manifest.som").exists()
}

/// Mejor modelo ya materializado que cabe en el USB (`SOSO_LIVE_OFFLINE`).
pub fn pick_materialized_for_usb(
    usb_bytes: u64,
    esp_aligned: u64,
    rootfs_aligned: u64,
    root: &Path,
) -> Option<LiveModelSpec> {
    let budget = models_budget_from_layout(usb_bytes, esp_aligned, rootfs_aligned);
    for spec in CATALOG.iter().rev() {
        if !is_materialized(root, spec) {
            continue;
        }
        if usb_bytes >= spec.min_usb_bytes && spec.need_bytes(root) <= budget {
            return Some(*spec);
        }
    }
    None
}

/// Mayor modelo del catálogo con `manifest.som` (sin comprobar tamaño de USB).
pub fn pick_largest_materialized(root: &Path) -> Option<LiveModelSpec> {
    CATALOG
        .iter()
        .rev()
        .find(|spec| is_materialized(root, spec))
        .copied()
}

fn materialized_catalog_names(root: &Path) -> Vec<&'static str> {
    CATALOG
        .iter()
        .filter(|spec| is_materialized(root, spec))
        .map(|spec| spec.name)
        .collect()
}

/// Nombres de modelos ya materializados (para mensajes de error).
pub fn list_materialized(root: &Path) -> Vec<&'static str> {
    materialized_catalog_names(root)
}

/// Parsea `SOSO_LIVE_CAPACITY` (`64G`, `128G`, …) o bytes enteros.
pub fn parse_capacity_env(raw: &str) -> Result<u64, String> {
    let s = raw.trim();
    if s.is_empty() {
        return Err("capacidad vacía".into());
    }
    if let Ok(n) = s.parse::<u64>() {
        return Ok(n);
    }
    let upper = s.to_ascii_uppercase();
    const G: u64 = 1024 * 1024 * 1024;
    const M: u64 = 1024 * 1024;
    if let Some(num) = upper.strip_suffix('G') {
        let v: f64 = num
            .trim()
            .parse()
            .map_err(|_| format!("capacidad inválida: {raw}"))?;
        return Ok((v * G as f64) as u64);
    }
    if let Some(num) = upper.strip_suffix('M') {
        let v: f64 = num
            .trim()
            .parse()
            .map_err(|_| format!("capacidad inválida: {raw}"))?;
        return Ok((v * M as f64) as u64);
    }
    Err(format!("capacidad inválida: {raw}"))
}

pub fn format_bytes(n: u64) -> String {
    const G: u64 = 1024 * 1024 * 1024;
    const M: u64 = 1024 * 1024;
    if n >= G {
        format!("{:.1} GiB", n as f64 / G as f64)
    } else if n >= M {
        format!("{:.0} MiB", n / M)
    } else if n > 0 {
        format!("{n} B")
    } else {
        String::from("0 B")
    }
}

pub fn dir_size_bytes(dir: &Path) -> u64 {
    let mut total = 0u64;
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                total += dir_size_bytes(&p);
            } else if let Ok(m) = e.metadata() {
                total += m.len();
            }
        }
    }
    total
}

/// Tamaño de imagen sosomfs recomendado para un árbol `.som` ya materializado.
pub fn suggest_models_image_size(model_dir: &Path, tiny_dir: &Path) -> String {
    let bytes = dir_size_bytes(model_dir)
        .saturating_add(dir_size_bytes(tiny_dir))
        .saturating_add(MODELS_IMAGE_MARGIN);
    const G: u64 = 1024 * 1024 * 1024;
    let gb = (bytes + G - 1) / G;
    format!("{}G", gb.max(1))
}

/// Falla si falta `manifest.som` (modo offline; no descarga).
pub fn require_materialized(root: &Path, spec: &LiveModelSpec) {
    let out = spec.target_dir(root);
    if out.join("manifest.som").exists() {
        println!(
            "live-models: {} ya en {}",
            spec.name,
            out.display()
        );
        return;
    }
    eprintln!(
        "live-models: SOSO_LIVE_OFFLINE=1 pero falta {} en {}\n\
         Descarga antes con: cargo xtask fetch-hf {} --name {} --out {}",
        spec.name,
        out.display(),
        spec.repo,
        spec.name,
        out.display()
    );
    exit(1);
}

/// Descarga/convierte el modelo si falta `manifest.som`.
pub fn ensure_materialized(root: &Path, spec: &LiveModelSpec) {
    let out = spec.target_dir(root);
    if out.join("manifest.som").exists() {
        println!(
            "live-models: {} ya en {}",
            spec.name,
            out.display()
        );
        return;
    }
    println!(
        "live-models: materializando {} desde {}…",
        spec.name, spec.repo
    );
    fetch_hf::pull_to(root, spec.repo, spec.gguf_file, spec.name, &out);
    if !out.join("manifest.som").exists() {
        eprintln!(
            "live-models: falta manifest.som en {} tras fetch-hf",
            out.display()
        );
        exit(1);
    }
}

/// Genera el modelo sintético `tiny` si hace falta.
pub fn ensure_tiny(root: &Path) -> PathBuf {
    let tiny = root.join("target/tiny-model");
    if tiny.join("manifest.som").exists() {
        return tiny;
    }
    let status = Command::new("cargo")
        .current_dir(root)
        .args(["run", "-q", "--release", "-p", "mkmodel-soso", "--"])
        .arg(&tiny)
        .status()
        .expect("mkmodel-soso tiny (live)");
    if !status.success() {
        exit(status.code().unwrap_or(1));
    }
    tiny
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> PathBuf {
        PathBuf::from("/nonexistent/soso-root")
    }

    #[test]
    fn pick_8g_tinyllama() {
        let usb = 8 * 1024 * 1024 * 1024;
        let spec = pick_for_usb(usb, 64 * 1024 * 1024, 512 * 1024 * 1024, &root());
        assert_eq!(spec.name, "tinyllama");
    }

    #[test]
    fn pick_12g_mistral() {
        let usb = 16 * 1024 * 1024 * 1024;
        let spec = pick_for_usb(usb, 64 * 1024 * 1024, 512 * 1024 * 1024, &root());
        assert_eq!(spec.name, "mistral-7b");
    }

    #[test]
    fn pick_30g_mixtral() {
        let usb = 32 * 1024 * 1024 * 1024;
        let spec = pick_for_usb(usb, 64 * 1024 * 1024, 512 * 1024 * 1024, &root());
        assert_eq!(spec.name, "mixtral");
    }

    #[test]
    fn pick_50g_llama2_70b() {
        let usb = 64 * 1024 * 1024 * 1024;
        let spec = pick_for_usb(usb, 64 * 1024 * 1024, 512 * 1024 * 1024, &root());
        assert_eq!(spec.name, "llama2-70b");
    }

    #[test]
    fn pick_muy_pequeno_cae_a_tinyllama() {
        let usb = 512 * 1024 * 1024;
        let spec = pick_for_usb(usb, 0, 0, &root());
        assert_eq!(spec.name, "tinyllama");
    }

    #[test]
    fn parse_capacity_gigabytes() {
        assert_eq!(parse_capacity_env("64G").unwrap(), 64 * 1024 * 1024 * 1024);
        assert_eq!(parse_capacity_env("2G").unwrap(), 2 * 1024 * 1024 * 1024);
    }

    #[test]
    fn models_budget_resta_layout() {
        let usb = 32 * 1024 * 1024 * 1024;
        let esp = 64 * 1024 * 1024;
        let rootfs = 512 * 1024 * 1024;
        let budget = models_budget_from_layout(usb, esp, rootfs);
        assert!(budget < usb);
        assert!(budget > 30 * 1024 * 1024 * 1024);
    }

    fn offline_fixture(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "soso-live-offline-{}-{tag}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        for name in ["tinyllama", "mistral-7b"] {
            let model = dir.join(format!("target/{name}-model"));
            std::fs::create_dir_all(&model).unwrap();
            std::fs::write(model.join("manifest.som"), b"x").unwrap();
        }
        dir
    }

    #[test]
    fn pick_offline_largest_materialized() {
        let root = offline_fixture("largest");
        let spec = pick_largest_materialized(&root).unwrap();
        assert_eq!(spec.name, "mistral-7b");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn pick_offline_for_usb_skips_missing() {
        let root = offline_fixture("usb64");
        let usb = 64 * 1024 * 1024 * 1024;
        let spec =
            pick_materialized_for_usb(usb, 64 * 1024 * 1024, 512 * 1024 * 1024, &root).unwrap();
        assert_eq!(spec.name, "mistral-7b");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn pick_offline_for_usb_falls_back_to_smaller() {
        let root = offline_fixture("usb8");
        let usb = 8 * 1024 * 1024 * 1024;
        let spec =
            pick_materialized_for_usb(usb, 64 * 1024 * 1024, 512 * 1024 * 1024, &root).unwrap();
        assert_eq!(spec.name, "tinyllama");
        let _ = std::fs::remove_dir_all(&root);
    }
}
