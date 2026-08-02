//! API Hugging Face Hub (parseo; sin red en tests).

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

pub struct TreeEntry {
    pub path: String,
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
        out.push(TreeEntry { path });
    }
    Ok(out)
}

pub fn fetch_tree(slug: &str, token: Option<&str>) -> Result<Vec<TreeEntry>, String> {
    let url = format!("https://huggingface.co/api/models/{slug}/tree/main");
    let body = crate::net::https_get_body(&url, token).map_err(|e| format!("HTTP: {e:?}"))?;
    parse_tree(core::str::from_utf8(&body).map_err(|_| String::from("utf8 inválido"))?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pick_q4km() {
        let e = vec![
            TreeEntry { path: String::from("a-f16.gguf") },
            TreeEntry { path: String::from("b-q4_k_m.gguf") },
        ];
        assert_eq!(pick_gguf(&e, None).unwrap(), "b-q4_k_m.gguf");
    }
}
