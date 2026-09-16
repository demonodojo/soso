//! Catálogo y selección del modelo LLM para el live USB según capacidad del pendrive.

use std::path::{Path, PathBuf};
use std::process::{Command, exit};

use crate::fetch_hf;
use sosomodel::parse_som;

/// Margen extra sobre el árbol `.som` al dimensionar la imagen sosomfs.
pub const MODELS_IMAGE_MARGIN: u64 = 256 * 1024 * 1024;

/// Reserva fija para la partición SOSOINSTALL al flashear (FAT32 exige ≥64 MiB).
pub const P4_INSTALL_BYTES: u64 = 64 * 1024 * 1024;

/// Holgura GPT + alineaciones al calcular el presupuesto de modelos.
pub const GPT_OVERHEAD_BYTES: u64 = 1024 * 1024;

/// Tamaño estimado del modelo sintético `tiny` empaquetado junto al demo.
pub const TINY_SOM_ESTIMATE: u64 = 64 * 1024 * 1024;

/// Qué tokenizer tiene que traer el `.som` convertido de un modelo del catálogo.
///
/// Se **declara** aquí en vez de deducirlo del vocabulario: contar marcas de
/// espacio acierta con el separador y no con el algoritmo (ficha T52). Y
/// equivocarse en esto no da un error, da un modelo que carga, responde y
/// segmenta mal en silencio, que es lo que se coló en el pendrive con el
/// Qwen2.5 del 15 de septiembre de 2026.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenizerEsperado {
    /// SentencePiece: el `.som` v1, sin tabla de fusiones, es lo correcto.
    Piezas,
    /// BPE byte-level (GPT-2, Qwen2, Qwen3): sin la tabla de fusiones no se
    /// puede reproducir su segmentación, y esa tabla es un `.som` v2.
    Fusiones,
}

/// Entrada del catálogo live (GGUF llama/qwen2, tokenizer SentencePiece o GPT-2).
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
    /// Tokenizer que tiene que traer su `.som` para segmentar como el original.
    pub tokenizer: TokenizerEsperado,
}

/// Catálogo ordenado de peor a mejor calidad (el picker recorre al revés).
pub const CATALOG: &[LiveModelSpec] = &[
    LiveModelSpec {
        name: "tinyllama",
        repo: "TinyLlama/TinyLlama-1.1B-Chat-v1.0",
        gguf_file: None,
        gguf_bytes_estimate: 700 * 1024 * 1024,
        min_usb_bytes: 8 * 1024 * 1024 * 1024,
        tokenizer: TokenizerEsperado::Piezas,
    },
    LiveModelSpec {
        name: "qwen2.5-coder-3b",
        repo: "Qwen/Qwen2.5-Coder-3B-Instruct-GGUF",
        gguf_file: Some("qwen2.5-coder-3b-instruct-q4_k_m.gguf"),
        gguf_bytes_estimate: 2100 * 1024 * 1024,
        min_usb_bytes: 8 * 1024 * 1024 * 1024,
        tokenizer: TokenizerEsperado::Fusiones,
    },
    LiveModelSpec {
        name: "mistral-7b",
        repo: "TheBloke/Mistral-7B-Instruct-v0.2-GGUF",
        gguf_file: None,
        gguf_bytes_estimate: 4_400 * 1024 * 1024,
        min_usb_bytes: 16 * 1024 * 1024 * 1024,
        tokenizer: TokenizerEsperado::Piezas,
    },
    LiveModelSpec {
        name: "mixtral",
        repo: "TheBloke/Mixtral-8x7B-Instruct-v0.1-GGUF",
        gguf_file: None,
        gguf_bytes_estimate: 26 * 1024 * 1024 * 1024,
        min_usb_bytes: 32 * 1024 * 1024 * 1024,
        tokenizer: TokenizerEsperado::Piezas,
    },
    LiveModelSpec {
        name: "llama2-70b",
        repo: "TheBloke/Llama-2-70B-Chat-GGUF",
        gguf_file: None,
        gguf_bytes_estimate: 39 * 1024 * 1024 * 1024,
        min_usb_bytes: 64 * 1024 * 1024 * 1024,
        tokenizer: TokenizerEsperado::Piezas,
    },
    LiveModelSpec {
        name: "qwen3.8-27b",
        repo: "ggml-org/Qwen3.8-27B-GGUF",
        gguf_file: Some("Qwen3.8-27B-Q4_K_M.gguf"),
        gguf_bytes_estimate: 19 * 1024 * 1024 * 1024,
        min_usb_bytes: 32 * 1024 * 1024 * 1024,
        tokenizer: TokenizerEsperado::Fusiones,
    },
];

/// Entrada del catálogo con ese nombre, si la hay.
pub fn spec_by_name(name: &str) -> Option<LiveModelSpec> {
    CATALOG.iter().find(|s| s.name == name).copied()
}

/// Versión del `.som` de `path`, validando magic y CRC de paso.
fn som_version(path: &Path) -> Result<u32, String> {
    let data = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    parse_som(&data)
        .map(|(v, _)| v)
        .map_err(|_| format!("{}: cabecera o CRC de .som inválidos", path.display()))
}

/// Modelo demo de `package-usb-live` cuando no hay pendrive ni `SOSO_LIVE_CAPACITY`.
pub fn default_live_spec() -> LiveModelSpec {
    *CATALOG
        .iter()
        .find(|s| s.name == "qwen2.5-coder-3b")
        .expect("qwen2.5-coder-3b en CATALOG")
}

impl LiveModelSpec {
    /// ¿Trae este directorio el tokenizer que el modelo necesita?
    ///
    /// Un `.som` v1 en un modelo BPE no rompe nada de forma visible: carga,
    /// responde y elige otros tokens. Por eso se comprueba antes de meterlo en
    /// una imagen, que es donde deja de poder arreglarse a tiempo.
    pub fn check_tokenizer(&self, dir: &Path) -> Result<(), String> {
        let path = dir.join("tokenizer.som");
        if !path.exists() {
            return Err(format!("falta {}", path.display()));
        }
        let version = som_version(&path)?;
        match self.tokenizer {
            // Un v2 en un SentencePiece querría decir que el GGUF traía
            // fusiones, y entonces usarlas es lo correcto: no es un fallo.
            TokenizerEsperado::Piezas => Ok(()),
            TokenizerEsperado::Fusiones if version >= 2 => Ok(()),
            TokenizerEsperado::Fusiones => Err(format!(
                "{} es un .som v{version}, sin tabla de fusiones BPE: {} segmentaría \
                 por pieza más larga y no como el tokenizer original",
                path.display(),
                self.name
            )),
        }
    }

    /// Aborta si el tokenizer no es el que toca. Mejor no empaquetar que
    /// empaquetar un modelo que segmenta mal.
    pub fn require_tokenizer(&self, dir: &Path) {
        if let Err(why) = self.check_tokenizer(dir) {
            eprintln!(
                "live-models: {why}\n\
                 Reconviértelo con: cargo xtask fetch-hf {} --name {} --out {}",
                self.repo,
                self.name,
                dir.display()
            );
            exit(1);
        }
    }

    pub fn target_dir(&self, root: &Path) -> PathBuf {
        root.join(format!("target/{}-model", self.name))
    }

    /// Directorio materializado válido (`--check` OK), con fallback a `{name}-model-new`.
    pub fn resolved_dir(&self, root: &Path) -> PathBuf {
        let primary = self.target_dir(root);
        if primary.join("manifest.som").exists() {
            if crate::fetch_hf::check_som_model(root, &primary).is_ok()
                && self.check_tokenizer(&primary).is_ok()
            {
                return primary;
            }
        }
        let alt = root.join(format!("target/{}-model-new", self.name));
        if alt.join("manifest.som").exists() {
            if crate::fetch_hf::check_som_model(root, &alt).is_ok()
                && self.check_tokenizer(&alt).is_ok()
            {
                eprintln!(
                    "live-models: {} obsoleto en {} — uso {}",
                    self.name,
                    primary.display(),
                    alt.display()
                );
                return alt;
            }
        }
        primary
    }

    /// Bytes necesarios en p3: modelo principal + `tiny` + margen de imagen.
    pub fn need_bytes(&self, root: &Path) -> u64 {
        let dir = self.target_dir(root);
        let model_bytes = if dir.join("manifest.som").exists() {
            packed_dir_size_bytes(&dir)
        } else {
            // GGUF → .som; 115 % cubre cuantización; el empaquetado real usa packed_*.
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

/// `SOSO_LIVE_AUTO_MODEL=1|true|yes`: al flashear, escalar modelo según tamaño real del USB.
pub fn auto_model_from_usb() -> bool {
    matches!(
        std::env::var("SOSO_LIVE_AUTO_MODEL").as_deref(),
        Ok("1") | Ok("true") | Ok("yes")
    )
}

/// Capacidad para `pick_for_usb` al flashear: `SOSO_LIVE_CAPACITY`, auto con
/// `SOSO_LIVE_AUTO_MODEL`, o `None` → `default_live_spec` (qwen2.5-coder-3b).
pub fn model_pick_bytes_for_flash(disk_bytes: u64) -> Option<u64> {
    if let Ok(raw) = std::env::var("SOSO_LIVE_CAPACITY") {
        return Some(parse_capacity_env(&raw).unwrap_or_else(|e| {
            eprintln!("flash-usb-live: {e}");
            exit(1);
        }));
    }
    if auto_model_from_usb() {
        return Some(disk_bytes);
    }
    None
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

/// Tamaño en disco que ocupa un árbol `.som` al empaquetarse en sosomfs
/// (alineación 2 MiB / 64 KiB — ver `sosomfs/src/builder.rs`).
fn packed_dir_size_bytes(dir: &Path) -> u64 {
    let mut total = 0u64;
    walk_dir_packed(dir, dir, &mut total);
    total
}

fn walk_dir_packed(root: &Path, dir: &Path, total: &mut u64) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk_dir_packed(root, &p, total);
            continue;
        }
        let Ok(m) = e.metadata() else {
            continue;
        };
        let len = m.len() as usize;
        let rel = p
            .strip_prefix(root)
            .ok()
            .and_then(|r| r.to_str())
            .unwrap_or("");
        let align = if rel.starts_with("shards/") && rel.ends_with(".tensor") {
            if len >= 2 * 1024 * 1024 {
                2 * 1024 * 1024
            } else {
                64 * 1024
            }
        } else {
            4096
        };
        *total += align_up(len, align) as u64;
    }
}

fn align_up(len: usize, align: usize) -> usize {
    (len + align - 1) & !(align - 1)
}

/// Holgura para catálogo sosomfs + superbloques (no van en el árbol `.som`).
const SOSOMFS_CATALOG_HEADROOM: u64 = 64 * 1024 * 1024;

/// Tamaño de imagen sosomfs recomendado para un árbol `.som` ya materializado.
pub fn suggest_models_image_size(model_dir: &Path, tiny_dir: &Path) -> String {
    let bytes = packed_dir_size_bytes(model_dir)
        .saturating_add(packed_dir_size_bytes(tiny_dir))
        .saturating_add(MODELS_IMAGE_MARGIN)
        .saturating_add(SOSOMFS_CATALOG_HEADROOM);
    const G: u64 = 1024 * 1024 * 1024;
    let gb = (bytes + G - 1) / G;
    format!("{}G", gb.max(1))
}

/// Falla si falta `manifest.som` (modo offline; no descarga).
pub fn require_materialized(root: &Path, spec: &LiveModelSpec) {
    let out = spec.target_dir(root);
    if out.join("manifest.som").exists() {
        crate::fetch_hf::require_valid_model(root, &out);
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
        if crate::fetch_hf::check_som_model(root, &out).is_ok() {
            println!(
                "live-models: {} ya en {}",
                spec.name,
                out.display()
            );
            return;
        }
        eprintln!(
            "live-models: {} obsoleto en {} — reconvirtiendo",
            spec.name,
            out.display()
        );
        let _ = std::fs::remove_dir_all(&out);
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
    fn default_live_es_qwen25_coder_3b() {
        assert_eq!(default_live_spec().name, "qwen2.5-coder-3b");
        assert_eq!(
            default_live_spec().gguf_file,
            Some("qwen2.5-coder-3b-instruct-q4_k_m.gguf")
        );
    }

    #[test]
    fn pick_8g_qwen25_coder() {
        let usb = 8 * 1024 * 1024 * 1024;
        let spec = pick_for_usb(usb, 64 * 1024 * 1024, 512 * 1024 * 1024, &root());
        assert_eq!(spec.name, "qwen2.5-coder-3b");
    }

    #[test]
    fn pick_12g_mistral() {
        let usb = 16 * 1024 * 1024 * 1024;
        let spec = pick_for_usb(usb, 64 * 1024 * 1024, 512 * 1024 * 1024, &root());
        assert_eq!(spec.name, "mistral-7b");
    }

    #[test]
    fn pick_30g_qwen38() {
        let usb = 32 * 1024 * 1024 * 1024;
        let spec = pick_for_usb(usb, 64 * 1024 * 1024, 512 * 1024 * 1024, &root());
        assert_eq!(spec.name, "qwen3.8-27b");
    }

    #[test]
    fn pick_64g_sigue_qwen38() {
        let usb = 64 * 1024 * 1024 * 1024;
        let spec = pick_for_usb(usb, 64 * 1024 * 1024, 512 * 1024 * 1024, &root());
        assert_eq!(spec.name, "qwen3.8-27b");
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
    fn packed_size_no_menor_que_raw() {
        let dir = std::env::temp_dir().join(format!(
            "soso-packed-size-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("shards")).unwrap();
        // 4,7 MiB → alineado a 6 MiB (2 MiB).
        std::fs::write(dir.join("shards/L00.attn_k.tensor"), vec![0u8; 4_900_000]).unwrap();
        let packed = packed_dir_size_bytes(&dir);
        assert_eq!(packed, 6 * 1024 * 1024);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn suggest_mixtral_image_at_least_26g() {
        let mixtral = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../target/mixtral-model");
        if !mixtral.join("manifest.som").exists() {
            return;
        }
        let tiny = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../target/tiny-model");
        let size = suggest_models_image_size(&mixtral, &tiny);
        let gb: u64 = size.trim_end_matches('G').parse().unwrap();
        assert!(
            gb >= 26,
            "mixtral empaquetado necesita ≥26G, sugerido {size}"
        );
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

    fn dir_con_tokenizer(tag: &str, version: Option<u32>) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "soso-live-tok-{}-{tag}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        if let Some(v) = version {
            let som = sosomodel::pack_som(b"cuerpo de mentira", v, sosomodel::CACHE_ALIGN);
            std::fs::write(dir.join("tokenizer.som"), som).unwrap();
        }
        dir
    }

    #[test]
    fn un_som_v1_no_vale_para_un_modelo_bpe() {
        let dir = dir_con_tokenizer("bpe-v1", Some(1));
        let err = spec_by_name("qwen2.5-coder-3b")
            .unwrap()
            .check_tokenizer(&dir)
            .expect_err("un v1 no trae fusiones");
        assert!(err.contains("fusiones"), "{err}");
        assert!(err.contains("pieza más larga"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn un_som_v2_vale_para_un_modelo_bpe() {
        let dir = dir_con_tokenizer("bpe-v2", Some(2));
        spec_by_name("qwen2.5-coder-3b")
            .unwrap()
            .check_tokenizer(&dir)
            .expect("v2 trae la tabla de fusiones");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn un_sentencepiece_sigue_valiendo_en_v1() {
        // Es lo correcto para TinyLlama: no cambia de versión por no tener
        // fusiones, y los modelos ya convertidos siguen siendo los mismos.
        let dir = dir_con_tokenizer("sp-v1", Some(1));
        spec_by_name("tinyllama")
            .unwrap()
            .check_tokenizer(&dir)
            .expect("v1 es lo que le toca a SentencePiece");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sin_tokenizer_no_se_empaqueta() {
        let dir = dir_con_tokenizer("sin", None);
        let err = spec_by_name("tinyllama")
            .unwrap()
            .check_tokenizer(&dir)
            .expect_err("sin tokenizer.som no hay modelo que valga");
        assert!(err.contains("falta"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn el_catalogo_declara_fusiones_en_las_familias_bpe() {
        // Si mañana entra otro Qwen/GPT al catálogo y nadie declara su
        // tokenizer, esto lo dice antes de que se meta en un pendrive.
        for spec in CATALOG {
            let bpe = spec.name.starts_with("qwen");
            assert_eq!(
                spec.tokenizer == TokenizerEsperado::Fusiones,
                bpe,
                "{}: familia BPE y declaración no concuerdan",
                spec.name
            );
        }
    }

    #[test]
    fn el_modelo_por_defecto_materializado_trae_sus_fusiones() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
        let spec = default_live_spec();
        let dir = spec.target_dir(&root);
        if !dir.join("tokenizer.som").exists() {
            return;
        }
        spec.check_tokenizer(&dir)
            .expect("el modelo que empaqueta el live tiene que segmentar como el original");
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

