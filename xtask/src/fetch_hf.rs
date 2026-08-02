//! Descarga modelos GGUF desde Hugging Face Hub, convierte a `.som` y deja
//! listo `SOSO_MODELS_DIR` para `cargo xtask run`.
//!
//! Uso: `cargo xtask fetch-hf <org/repo> [--file NAME.gguf] [--name NOMBRE] [--out DIR] [--run]`

use std::path::{Path, PathBuf};
use std::process::{Command, exit};

/// Entrada del árbol de ficheros de la API Hub (`/api/models/{repo}/tree/main`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HfTreeEntry {
    pub path: String,
    pub size: u64,
}

fn gguf_basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// Selecciona el fichero GGUF a descargar.
///
/// Prioridad: `--file` explícito → `*Q4_K_M*.gguf` → `*Q4_K*.gguf` → primer `.gguf`.
pub fn pick_gguf_file(entries: &[HfTreeEntry], explicit: Option<&str>) -> Result<String, String> {
    let ggufs: Vec<&HfTreeEntry> = entries
        .iter()
        .filter(|e| e.path.ends_with(".gguf"))
        .collect();
    if ggufs.is_empty() {
        return Err(
            "el repositorio no contiene ficheros .gguf (safetensors no soportados; convierte a GGUF en el host)".into(),
        );
    }
    if let Some(name) = explicit {
        if ggufs.iter().any(|e| e.path == name || e.path.ends_with(&format!("/{name}"))) {
            return Ok(normalize_hf_path(name));
        }
        let list: Vec<_> = ggufs.iter().map(|e| e.path.as_str()).collect();
        return Err(format!(
            "no encuentro {name} en el repo; GGUF disponibles: {}",
            list.join(", ")
        ));
    }
    if let Some(e) = ggufs.iter().find(|e| {
        gguf_basename(&e.path).to_ascii_uppercase().contains("Q4_K_M")
    }) {
        return Ok(e.path.clone());
    }
    if let Some(e) = ggufs.iter().find(|e| {
        gguf_basename(&e.path).to_ascii_uppercase().contains("Q4_K")
    }) {
        return Ok(e.path.clone());
    }
    Ok(ggufs[0].path.clone())
}

fn normalize_hf_path(p: &str) -> String {
    p.trim_start_matches("./").to_string()
}

/// Parsea la respuesta JSON del árbol Hub (array de objetos con `type`, `path`, `size`).
pub fn parse_hf_tree(json: &str) -> Result<Vec<HfTreeEntry>, String> {
    let v: serde_json::Value =
        serde_json::from_str(json).map_err(|e| format!("JSON del árbol Hub inválido: {e}"))?;
    let arr = v
        .as_array()
        .ok_or_else(|| "se esperaba un array en la respuesta del árbol Hub".to_string())?;
    let mut out = Vec::new();
    for item in arr {
        let ty = item.get("type").and_then(|t| t.as_str()).unwrap_or("");
        if ty != "file" {
            continue;
        }
        let path = item
            .get("path")
            .and_then(|p| p.as_str())
            .ok_or_else(|| "entrada sin path".to_string())?
            .to_string();
        let size = item.get("size").and_then(|s| s.as_u64()).unwrap_or(0);
        out.push(HfTreeEntry { path, size });
    }
    Ok(out)
}

fn hf_token() -> Option<String> {
    std::env::var("HF_TOKEN")
        .ok()
        .or_else(|| std::env::var("HUGGING_FACE_HUB_TOKEN").ok())
        .filter(|s| !s.is_empty())
}

fn repo_slug(repo: &str) -> String {
    repo.trim().trim_end_matches('/').to_string()
}

fn default_model_name(_repo: &str, file: &str) -> String {
    file.rsplit('/')
        .next()
        .unwrap_or(file)
        .strip_suffix(".gguf")
        .unwrap_or(file)
        .to_string()
}

fn dir_size_bytes(dir: &Path) -> u64 {
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

fn suggest_models_size(bytes: u64) -> String {
    const G: u64 = 1024 * 1024 * 1024;
    let with_margin = bytes.saturating_mul(12) / 10 + 256 * 1024 * 1024;
    let gb = (with_margin + G - 1) / G;
    format!("{}G", gb.max(1))
}

pub fn run(args: &[String]) {
    let root = super::project_root();
    let mut pos = 0usize;
    let repo = args.get(pos).cloned().unwrap_or_else(|| {
        eprintln!(
            "uso: cargo xtask fetch-hf <org/repo> [--file NAME.gguf] [--name NOMBRE] [--out DIR] [--run]"
        );
        exit(2);
    });
    pos += 1;

    let mut file: Option<String> = None;
    let mut name: Option<String> = None;
    let mut out: Option<PathBuf> = None;
    let mut do_run = false;

    while pos < args.len() {
        match args[pos].as_str() {
            "--file" => {
                pos += 1;
                file = Some(args.get(pos).cloned().unwrap_or_else(|| {
                    eprintln!("fetch-hf: --file requiere un argumento");
                    exit(2);
                }));
            }
            "--name" => {
                pos += 1;
                name = Some(args.get(pos).cloned().unwrap_or_else(|| {
                    eprintln!("fetch-hf: --name requiere un argumento");
                    exit(2);
                }));
            }
            "--out" => {
                pos += 1;
                out = Some(PathBuf::from(args.get(pos).cloned().unwrap_or_else(|| {
                    eprintln!("fetch-hf: --out requiere un argumento");
                    exit(2);
                })));
            }
            "--run" => do_run = true,
            other => {
                eprintln!("fetch-hf: opción desconocida {other}");
                exit(2);
            }
        }
        pos += 1;
    }

    let slug = repo_slug(&repo);
    if slug.is_empty() || !slug.contains('/') {
        eprintln!("fetch-hf: el repo debe ser org/nombre (ej. TinyLlama/TinyLlama-1.1B-Chat-v1.0)");
        exit(2);
    }

    let entries = fetch_tree(&slug);
    let chosen = match pick_gguf_file(&entries, file.as_deref()) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("fetch-hf: {e}");
            exit(1);
        }
    };

    let model_name = name.clone().unwrap_or_else(|| default_model_name(&slug, &chosen));
    let out_dir = out.unwrap_or_else(|| root.join("target/hf-models").join(&model_name));
    let cache_dir = root.join("target/hf-cache").join(&slug);
    std::fs::create_dir_all(&cache_dir).expect("crear cache hf");
    let gguf_cache = cache_dir.join(chosen.rsplit('/').next().unwrap_or(&chosen));

    if !gguf_cache.exists() {
        download_gguf(&slug, &chosen, &gguf_cache);
    } else {
        println!("fetch-hf: cache hit → {}", gguf_cache.display());
    }

    convert_gguf(&root, &gguf_cache, &out_dir, &model_name);

    let size_hint = suggest_models_size(dir_size_bytes(&out_dir));
    println!("fetch-hf: modelo listo en {}", out_dir.display());
    println!(
        "  SOSO_MODELS_DIR={} SOSO_MODELS_SIZE={} cargo xtask run",
        out_dir.display(),
        size_hint
    );
    println!("  # guest: soso-llm run {model_name} --prompt hola");

    if do_run {
        unsafe {
            std::env::set_var("SOSO_MODELS_DIR", &out_dir);
            std::env::set_var("SOSO_MODELS_SIZE", &size_hint);
        }
        let img = super::build_image();
        super::run_qemu(&img, false);
    }
}

fn fetch_tree(slug: &str) -> Vec<HfTreeEntry> {
    let url = format!("https://huggingface.co/api/models/{slug}/tree/main");
    println!("fetch-hf: listando {url}…");
    let mut cmd = Command::new("curl");
    cmd.args(["-fsSL", &url]);
    if let Some(tok) = hf_token() {
        cmd.args(["-H", &format!("Authorization: Bearer {tok}")]);
    }
    let out = cmd.output().unwrap_or_else(|e| {
        eprintln!("fetch-hf: no se pudo ejecutar curl: {e}");
        exit(1);
    });
    if !out.status.success() {
        eprintln!(
            "fetch-hf: listado falló (HTTP {}): {}",
            out.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&out.stderr)
        );
        exit(1);
    }
    let body = String::from_utf8_lossy(&out.stdout);
    match parse_hf_tree(&body) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("fetch-hf: {e}");
            exit(1);
        }
    }
}

fn download_gguf(slug: &str, file: &str, dest: &Path) {
    let url = format!(
        "https://huggingface.co/{slug}/resolve/main/{}",
        file.trim_start_matches('/')
    );
    println!("fetch-hf: descargando {url}…");
    if let Some(parent) = dest.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let mut cmd = Command::new("curl");
    cmd.args(["-fL", &url, "-o"]).arg(dest);
    if let Some(tok) = hf_token() {
        cmd.args(["-H", &format!("Authorization: Bearer {tok}")]);
    }
    let status = cmd.status().expect("curl");
    if !status.success() {
        eprintln!("fetch-hf: descarga falló ({url})");
        let _ = std::fs::remove_file(dest);
        exit(1);
    }
    println!("fetch-hf: guardado en {}", dest.display());
}

fn convert_gguf(root: &Path, gguf: &Path, out_dir: &Path, name: &str) {
    if out_dir.join("manifest.som").exists() {
        println!(
            "fetch-hf: {} ya existe; omitiendo conversión (borra el dir para reconvertir)",
            out_dir.display()
        );
        return;
    }
    let _ = std::fs::create_dir_all(out_dir);
    let mut cmd = Command::new("cargo");
    cmd.current_dir(root)
        .args(["run", "-q", "-p", "convert-gguf", "--"])
        .arg(gguf)
        .arg(out_dir)
        .args(["--name", name]);
    let status = cmd.status().expect("convert-gguf");
    if !status.success() {
        exit(status.code().unwrap_or(1));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_TREE: &str = r#"[
        {"type":"file","path":"README.md","size":100},
        {"type":"file","path":"model-q4_k_m.gguf","size":700000000},
        {"type":"file","path":"model-f16.gguf","size":2000000000},
        {"type":"directory","path":"other","size":0}
    ]"#;

    #[test]
    fn parse_tree_extrae_solo_ficheros() {
        let e = parse_hf_tree(SAMPLE_TREE).unwrap();
        assert_eq!(e.len(), 3);
        assert!(e.iter().any(|x| x.path == "model-q4_k_m.gguf"));
    }

    #[test]
    fn pick_prefiere_q4_k_m() {
        let e = parse_hf_tree(SAMPLE_TREE).unwrap();
        assert_eq!(pick_gguf_file(&e, None).unwrap(), "model-q4_k_m.gguf");
    }

    #[test]
    fn pick_file_explicito() {
        let e = parse_hf_tree(SAMPLE_TREE).unwrap();
        assert_eq!(
            pick_gguf_file(&e, Some("model-f16.gguf")).unwrap(),
            "model-f16.gguf"
        );
    }

    #[test]
    fn pick_sin_gguf_falla() {
        let e = parse_hf_tree(r#"[{"type":"file","path":"model.safetensors","size":1}]"#)
            .unwrap();
        assert!(pick_gguf_file(&e, None).is_err());
    }
}
