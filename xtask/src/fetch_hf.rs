//! Descarga modelos GGUF desde Hugging Face Hub, convierte a `.som` y deja
//! listo `SOSO_MODELS_DIR` para `cargo xtask run`.
//!
//! Uso:
//!   cargo xtask fetch-hf search <consulta> [--limit N] [--all]
//!   cargo xtask fetch-hf list <org/repo>
//!   cargo xtask fetch-hf <org/repo> [--file …] [--name …] [--out …] [--run]

use std::path::{Path, PathBuf};
use std::process::{Command, exit};

/// Entrada del árbol de ficheros de la API Hub (`/api/models/{repo}/tree/main`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HfTreeEntry {
    pub path: String,
    pub size: u64,
}

/// Resultado de búsqueda en el Hub (`/api/models?search=…`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HfModelHit {
    pub id: String,
    pub downloads: u64,
    pub tags: Vec<String>,
}

impl HfModelHit {
    pub fn has_gguf_tag(&self) -> bool {
        self.tags.iter().any(|t| t.eq_ignore_ascii_case("gguf"))
    }
}

/// Parsea la respuesta JSON de búsqueda de modelos del Hub.
pub fn parse_hf_search(json: &str) -> Result<Vec<HfModelHit>, String> {
    let v: serde_json::Value =
        serde_json::from_str(json).map_err(|e| format!("JSON de búsqueda Hub inválido: {e}"))?;
    let arr = v
        .as_array()
        .ok_or_else(|| "se esperaba un array en la búsqueda Hub".to_string())?;
    let mut out = Vec::new();
    for item in arr {
        let id = item
            .get("id")
            .or_else(|| item.get("modelId"))
            .and_then(|x| x.as_str())
            .ok_or_else(|| "entrada sin id".to_string())?
            .to_string();
        let downloads = item.get("downloads").and_then(|d| d.as_u64()).unwrap_or(0);
        let tags: Vec<String> = item
            .get("tags")
            .and_then(|t| t.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        out.push(HfModelHit { id, downloads, tags });
    }
    Ok(out)
}

fn format_bytes(n: u64) -> String {
    const G: u64 = 1024 * 1024 * 1024;
    const M: u64 = 1024 * 1024;
    if n >= G {
        format!("{:.1} GiB", n as f64 / G as f64)
    } else if n >= M {
        format!("{:.0} MiB", n / M)
    } else if n > 0 {
        format!("{n} B")
    } else {
        String::from("?")
    }
}

fn print_model_hits(hits: &[HfModelHit]) {
    if hits.is_empty() {
        println!("(sin resultados)");
        return;
    }
    let w = hits.iter().map(|h| h.id.len()).max().unwrap_or(8).max(8);
    for h in hits {
        println!("  {:w$}  {:>10} descargas", h.id, h.downloads, w = w);
    }
}

fn print_gguf_list(slug: &str, entries: &[HfTreeEntry]) {
    let ggufs: Vec<_> = entries.iter().filter(|e| e.path.ends_with(".gguf")).collect();
    if ggufs.is_empty() {
        println!("fetch-hf: {slug} no contiene ficheros .gguf");
        return;
    }
    println!("GGUF en {slug}:");
    for e in ggufs {
        println!("  {}  ({})", e.path, format_bytes(e.size));
    }
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
    match args.first().map(String::as_str) {
        Some("search") => run_search(&args[1..]),
        Some("list") => run_list(&args[1..]),
        Some(s) if s.contains('/') => run_pull(args),
        _ => {
            eprintln!(
                "uso:\n  \
                 cargo xtask fetch-hf search <consulta> [--limit N] [--all]\n  \
                 cargo xtask fetch-hf list <org/repo>\n  \
                 cargo xtask fetch-hf <org/repo> [--file NAME.gguf] [--name N] [--out DIR] [--run]"
            );
            exit(2);
        }
    }
}

fn run_search(args: &[String]) {
    let query = args.first().cloned().unwrap_or_else(|| {
        eprintln!("uso: cargo xtask fetch-hf search <consulta> [--limit N] [--all]");
        exit(2);
    });
    let mut limit = 20u32;
    let mut gguf_only = true;
    let mut i = 1usize;
    while i < args.len() {
        match args[i].as_str() {
            "--limit" => {
                i += 1;
                limit = args.get(i).and_then(|s| s.parse().ok()).unwrap_or_else(|| {
                    eprintln!("fetch-hf: --limit requiere un número");
                    exit(2);
                });
            }
            "--all" => gguf_only = false,
            other => {
                eprintln!("fetch-hf: opción desconocida {other}");
                exit(2);
            }
        }
        i += 1;
    }
    let hits = search_models(&query, limit);
    let hits: Vec<_> = if gguf_only {
        hits.into_iter().filter(|h| h.has_gguf_tag()).collect()
    } else {
        hits
    };
    if gguf_only {
        println!("Modelos GGUF en el Hub (consulta «{query}», límite {limit}):");
    } else {
        println!("Modelos en el Hub (consulta «{query}», límite {limit}):");
    }
    print_model_hits(&hits);
    if gguf_only && hits.is_empty() {
        eprintln!("fetch-hf: prueba con --all o una consulta distinta");
    }
}

fn run_list(args: &[String]) {
    let slug = args.first().cloned().unwrap_or_else(|| {
        eprintln!("uso: cargo xtask fetch-hf list <org/repo>");
        exit(2);
    });
    let slug = repo_slug(&slug);
    if !slug.contains('/') {
        eprintln!("fetch-hf: el repo debe ser org/nombre");
        exit(2);
    }
    let entries = fetch_tree(&slug);
    print_gguf_list(&slug, &entries);
}

fn run_pull(args: &[String]) {
    let root = super::project_root();
    let mut pos = 0usize;
    let repo = args.get(pos).cloned().unwrap_or_else(|| {
        eprintln!("fetch-hf: falta org/repo");
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

    pull_to(&root, &slug, Some(&chosen), &model_name, &out_dir);

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

/// Descarga GGUF (cache en `target/hf-cache/`) y convierte a `.som` en `out_dir`.
///
/// `repo` es `org/nombre`. `file` opcional fija el GGUF; si es `None` se lista
/// el árbol del Hub y se elige Q4_K_M.
pub fn pull_to(
    root: &Path,
    repo: &str,
    file: Option<&str>,
    model_name: &str,
    out_dir: &Path,
) {
    let slug = repo_slug(repo);
    if slug.is_empty() || !slug.contains('/') {
        eprintln!("fetch-hf: el repo debe ser org/nombre (ej. TinyLlama/TinyLlama-1.1B-Chat-v1.0)");
        exit(2);
    }

    let chosen = if let Some(f) = file {
        f.to_string()
    } else {
        let entries = fetch_tree(&slug);
        match pick_gguf_file(&entries, None) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("fetch-hf: {e}");
                exit(1);
            }
        }
    };

    let cache_dir = root.join("target/hf-cache").join(&slug);
    std::fs::create_dir_all(&cache_dir).expect("crear cache hf");
    let gguf_cache = cache_dir.join(chosen.rsplit('/').next().unwrap_or(&chosen));

    if !gguf_cache.exists() {
        download_gguf(&slug, &chosen, &gguf_cache);
    } else {
        println!("fetch-hf: cache hit → {}", gguf_cache.display());
    }

    convert_gguf(root, &gguf_cache, out_dir, model_name);
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

fn search_models(query: &str, limit: u32) -> Vec<HfModelHit> {
    let q = urlencoding_query(query);
    let url = format!(
        "https://huggingface.co/api/models?search={q}&limit={limit}&sort=downloads&direction=-1"
    );
    println!("fetch-hf: buscando {url}…");
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
            "fetch-hf: búsqueda falló (HTTP {}): {}",
            out.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&out.stderr)
        );
        exit(1);
    }
    let body = String::from_utf8_lossy(&out.stdout);
    match parse_hf_search(&body) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("fetch-hf: {e}");
            exit(1);
        }
    }
}

fn urlencoding_query(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            b' ' => out.push_str("%20"),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
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

    #[test]
    fn parse_search_id_y_tags() {
        let json = r#"[{"id":"org/m","downloads":42,"tags":["gguf","en"]}]"#;
        let h = parse_hf_search(json).unwrap();
        assert_eq!(h.len(), 1);
        assert_eq!(h[0].id, "org/m");
        assert_eq!(h[0].downloads, 42);
        assert!(h[0].has_gguf_tag());
    }

    #[test]
    fn urlencoding_espacios() {
        assert_eq!(urlencoding_query("tiny llama"), "tiny%20llama");
    }
}
