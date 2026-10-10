//! JSON y respuestas de `POST /repos/{owner}/{repo}/issues`.

use alloc::format;
use alloc::string::{String, ToString};

pub const TITULO_MAX: usize = 240;
pub const CUERPO_GITHUB_MAX: usize = 65536;

pub fn issues_url(repo: &str) -> Result<String, &'static str> {
    let repo = repo.trim();
    if repo.is_empty() {
        return Err("repo vacío");
    }
    let Some((owner, name)) = repo.split_once('/') else {
        return Err("repo debe ser owner/nombre");
    };
    if owner.is_empty() || name.is_empty() || repo.contains("//") {
        return Err("repo inválido");
    }
    Ok(format!("https://api.github.com/repos/{repo}/issues"))
}

pub fn titulo_issue(mensaje: &str, release: Option<&str>) -> String {
    let msg = mensaje.trim();
    let base = if msg.is_empty() {
        release
            .map(|r| {
                let r = r.lines().next().unwrap_or(r).trim();
                if r.is_empty() {
                    String::from("notifybug")
                } else {
                    format!("notifybug {r}")
                }
            })
            .unwrap_or_else(|| String::from("notifybug"))
    } else {
        format!("notifybug: {msg}")
    };
    if base.len() <= TITULO_MAX {
        base
    } else {
        let mut t = base;
        t.truncate(TITULO_MAX);
        t
    }
}

pub fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

pub fn build_issue_json(title: &str, body: &str) -> String {
    format!(
        "{{\"title\":\"{}\",\"body\":\"{}\"}}",
        json_escape(title),
        json_escape(body)
    )
}

pub fn parse_issue_response(status: u16, body: &[u8]) -> Result<String, &'static str> {
    match status {
        201 => extract_html_url(body).ok_or("respuesta sin html_url"),
        401 => Err(mensaje_error_http(401)),
        403 => Err(mensaje_error_http(403)),
        422 => Err(mensaje_error_http(422)),
        _ => Err("GitHub respondió con un código distinto de 201"),
    }
}

pub fn mensaje_error_http(status: u16) -> &'static str {
    match status {
        401 => "token inválido o caducado (HTTP 401)",
        403 => "sin permiso para crear issues (HTTP 403)",
        422 => "GitHub rechazó el informe (HTTP 422)",
        _ => "error HTTP",
    }
}

fn extract_html_url(body: &[u8]) -> Option<String> {
    let s = core::str::from_utf8(body).ok()?;
    let key = "\"html_url\":";
    let i = s.find(key)?;
    let rest = s[i + key.len()..].trim_start();
    let rest = rest.strip_prefix('"')?;
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}
