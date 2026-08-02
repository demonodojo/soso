//! API Hugging Face Hub (parseo; sin red en tests).

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

pub struct TreeEntry {
    pub path: String,
    pub size: u64,
}

pub struct ModelHit {
    pub id: String,
    pub downloads: u64,
    pub tags: Vec<String>,
}

impl ModelHit {
    pub fn has_gguf_tag(&self) -> bool {
        self.tags.iter().any(|t| t.eq_ignore_ascii_case("gguf"))
    }
}

pub fn default_name(file: &str) -> String {
    file.strip_suffix(".gguf")
        .unwrap_or(file)
        .to_string()
}

pub fn pick_gguf(entries: &[TreeEntry], explicit: Option<&str>) -> Result<String, String> {
    let ggufs: Vec<_> = entries
        .iter()
        .filter(|e| e.path.ends_with(".gguf"))
        .collect();
    if ggufs.is_empty() {
        return Err(
            "el repositorio no contiene .gguf (safetensors no soportados)".into(),
        );
    }
    if let Some(name) = explicit {
        if ggufs.iter().any(|e| e.path == name) {
            return Ok(String::from(name));
        }
        return Err(format!("no encuentro {name} en el repo"));
    }
    for e in &ggufs {
        let b = e.path.to_ascii_uppercase();
        if b.contains("Q4_K_M") {
            return Ok(e.path.clone());
        }
    }
    for e in &ggufs {
        if e.path.to_ascii_uppercase().contains("Q4_K") {
            return Ok(e.path.clone());
        }
    }
    Ok(ggufs[0].path.clone())
}

pub fn parse_tree(json: &str) -> Result<Vec<TreeEntry>, String> {
    let v: serde_json::Value =
        serde_json::from_str(json).map_err(|e| format!("JSON inválido: {e}"))?;
    let arr = v.as_array().ok_or("se esperaba array")?;
    let mut out = Vec::new();
    for item in arr {
        if item.get("type").and_then(|t| t.as_str()) != Some("file") {
            continue;
        }
        let path = item
            .get("path")
            .and_then(|p| p.as_str())
            .ok_or("entrada sin path")?
            .to_string();
        let size = item.get("size").and_then(|s| s.as_u64()).unwrap_or(0);
        out.push(TreeEntry { path, size });
    }
    Ok(out)
}

pub fn parse_search(json: &str) -> Result<Vec<ModelHit>, String> {
    let v: serde_json::Value =
        serde_json::from_str(json).map_err(|e| format!("JSON inválido: {e}"))?;
    let arr = v.as_array().ok_or("se esperaba array")?;
    let mut out = Vec::new();
    for item in arr {
        let id = item
            .get("id")
            .or_else(|| item.get("modelId"))
            .and_then(|x| x.as_str())
            .ok_or("entrada sin id")?
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
        out.push(ModelHit { id, downloads, tags });
    }
    Ok(out)
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

pub fn fetch_tree(slug: &str, token: Option<&str>) -> Result<Vec<TreeEntry>, String> {
    let url = format!("https://huggingface.co/api/models/{slug}/tree/main");
    let body = crate::net::https_get_body(&url, token).map_err(|e| format!("HTTP: {e:?}"))?;
    parse_tree(core::str::from_utf8(&body).map_err(|_| String::from("utf8 inválido"))?)
}

pub fn fetch_search(
    query: &str,
    limit: u32,
    token: Option<&str>,
) -> Result<Vec<ModelHit>, String> {
    let q = urlencoding_query(query);
    let url = format!(
        "https://huggingface.co/api/models?search={q}&limit={limit}&sort=downloads&direction=-1"
    );
    let body = crate::net::https_get_body(&url, token).map_err(|e| format!("HTTP: {e:?}"))?;
    parse_search(core::str::from_utf8(&body).map_err(|_| String::from("utf8 inválido"))?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pick_q4km() {
        let e = vec![
            TreeEntry { path: String::from("a-f16.gguf"), size: 1 },
            TreeEntry { path: String::from("b-q4_k_m.gguf"), size: 2 },
        ];
        assert_eq!(pick_gguf(&e, None).unwrap(), "b-q4_k_m.gguf");
    }

    #[test]
    fn parse_search_gguf_tag() {
        let json = r#"[{"id":"org/m","downloads":10,"tags":["gguf"]}]"#;
        let h = parse_search(json).unwrap();
        assert!(h[0].has_gguf_tag());
    }
}
