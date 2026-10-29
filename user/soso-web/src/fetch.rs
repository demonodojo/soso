//! Descarga HTTPS y lectura local de HTML.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use libsoso::sys;
use soso_http::{header_value, https_get_full, https_post_full_typed, HttpError};
use soso_web_core::html::{parse_gfx_document, resolve_url, GfxForm, GfxFormField};

use soso_web_core::decode::{bytes_to_text, maybe_decompress};
use crate::bookmarks;
use crate::net::Net;
use alloc::vec;
use soso_web::png::{self, DecodedImage, MAX_IMAGES};

pub enum FetchError {
    Http(HttpError),
    Io(i64),
    Status(u16),
}

impl FetchError {
    pub fn mensaje(&self) -> &'static str {
        match self {
            Self::Http(HttpError::Parse) | Self::Http(HttpError::ParseDetail(_)) => {
                "error HTTP (parse)"
            }
            Self::Http(HttpError::Tls(_)) => "error TLS",
            Self::Http(HttpError::Io(_)) => "error HTTP (E/S)",
            Self::Http(HttpError::Dns) => "error DNS",
            Self::Http(HttpError::Clock) => "reloj del sistema no utilizable",
            Self::Http(HttpError::Interrupted) => "interrumpido",
            Self::Io(_) => "error de E/S",
            Self::Status(st) => {
                let _ = st;
                "respuesta HTTP no válida"
            }
        }
    }
}

pub struct PageBody {
    pub html: String,
    pub base: String,
    pub status: u16,
}

/// Descarga una URL HTTPS y devuelve HTML como texto.
pub fn fetch_https(url: &str) -> Result<(String, String), FetchError> {
    let p = fetch_https_ex(url)?;
    if p.status != 200 {
        return Err(FetchError::Status(p.status));
    }
    Ok((p.html, p.base))
}

pub fn fetch_https_ex(url: &str) -> Result<PageBody, FetchError> {
    crate::net::ensure_wall_clock();
    let resp = https_get_full(&Net, url, None).map_err(FetchError::Http)?;
    let enc = header_value(&resp.headers, "content-encoding");
    let body = maybe_decompress(&resp.body, enc).map_err(|_| FetchError::Status(resp.status))?;
    let charset = header_value(&resp.headers, "content-type")
        .and_then(|ct| ct.split("charset=").nth(1).map(|s| s.trim()));
    let html = bytes_to_text(&body, charset);
    Ok(PageBody {
        html,
        base: url.to_string(),
        status: resp.status,
    })
}

/// Lee HTML desde un fichero local (modo prueba / sin red).
pub fn fetch_local(path: &str) -> Result<String, FetchError> {
    let fd = sys::open(path, soso_abi::O_RDONLY);
    if fd < 0 {
        return Err(FetchError::Io(fd));
    }
    let fd = fd as u64;
    let mut st = soso_abi::Stat::default();
    if sys::stat(path, &mut st) < 0 {
        let _ = sys::close(fd);
        return Err(FetchError::Io(-soso_abi::EIO));
    }
    let mut buf = Vec::with_capacity(st.size.min(512 * 1024) as usize);
    buf.resize(st.size.min(512 * 1024) as usize, 0);
    let n = sys::read(fd, &mut buf);
    let _ = sys::close(fd);
    if n < 0 {
        return Err(FetchError::Io(n));
    }
    buf.truncate(n as usize);
    Ok(bytes_to_text(&buf, Some("utf-8")))
}

pub fn fetch_bytes(spec: &str) -> Result<Vec<u8>, FetchError> {
    crate::net::ensure_wall_clock();
    if spec.starts_with("https://") || spec.starts_with("http://") {
        let resp = https_get_full(&Net, spec, None).map_err(FetchError::Http)?;
        if resp.status != 200 {
            return Err(FetchError::Status(resp.status));
        }
        let enc = header_value(&resp.headers, "content-encoding");
        return maybe_decompress(&resp.body, enc).map_err(|_| FetchError::Status(resp.status));
    }
    let path = spec
        .strip_prefix("file://")
        .unwrap_or(spec);
    let fd = sys::open(path, soso_abi::O_RDONLY);
    if fd < 0 {
        return Err(FetchError::Io(fd));
    }
    let fd = fd as u64;
    let mut st = soso_abi::Stat::default();
    if sys::stat(path, &mut st) < 0 {
        let _ = sys::close(fd);
        return Err(FetchError::Io(-soso_abi::EIO));
    }
    let cap = st
        .size
        .min(png::MAX_FILE_BYTES as u64)
        .min(512 * 1024) as usize;
    let mut buf = vec![0u8; cap];
    let n = sys::read(fd, &mut buf);
    let _ = sys::close(fd);
    if n < 0 {
        return Err(FetchError::Io(n));
    }
    buf.truncate(n as usize);
    Ok(buf)
}

pub fn fetch_page_ex(spec: &str) -> Result<PageBody, FetchError> {
    if spec == bookmarks::SCHEME {
        return Ok(PageBody {
            html: bookmarks::list_html(),
            base: bookmarks::SCHEME.to_string(),
            status: 200,
        });
    }
    crate::net::ensure_wall_clock();
    if spec.starts_with("https://") || spec.starts_with("http://") {
        return fetch_https_ex(spec);
    }
    if let Some(path) = spec.strip_prefix("file://") {
        let html = fetch_local(path)?;
        return Ok(PageBody {
            html,
            base: spec.to_string(),
            status: 200,
        });
    }
    if spec.starts_with('/') {
        let html = fetch_local(spec)?;
        return Ok(PageBody {
            html,
            base: format!("file://{spec}"),
            status: 200,
        });
    }
    fetch_https_ex(spec)
}

pub fn load_decoded_images(html: &str, base_url: &str) -> Vec<Option<DecodedImage>> {
    let gfx = parse_gfx_document(html, base_url);
    let mut out: Vec<Option<DecodedImage>> = Vec::new();
    let mut loaded = 0usize;
    for img in &gfx.images {
        while out.len() <= img.idx {
            out.push(None);
        }
        if loaded >= MAX_IMAGES {
            continue;
        }
        let abs = resolve_url(base_url, &img.src);
        let decoded = fetch_bytes(&abs)
            .ok()
            .and_then(|b| {
                if b.len() > png::MAX_FILE_BYTES {
                    None
                } else {
                    png::decode_png(&b).ok()
                }
            });
        if decoded.is_some() {
            loaded += 1;
        }
        out[img.idx] = decoded;
    }
    out
}

pub fn urlencode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            b' ' => out.push('+'),
            _ => {
                let _ = write!(out, "%{b:02X}");
            }
        }
    }
    out
}

use core::fmt::Write;

pub fn build_form_get_url(
    action: &str,
    base: &str,
    fields: &[GfxFormField],
    values: &[String],
    form_idx: usize,
) -> String {
    let mut url = resolve_url(base, action);
    let mut first = !url.contains('?');
    for f in fields.iter().filter(|f| f.form_idx == form_idx) {
        let val = values
            .get(f.field_idx)
            .cloned()
            .unwrap_or_else(|| f.value.clone());
        if f.name.is_empty() {
            continue;
        }
        url.push(if first { '?' } else { '&' });
        first = false;
        url.push_str(&urlencode(&f.name));
        url.push('=');
        url.push_str(&urlencode(&val));
    }
    url
}

pub fn build_form_post_body(
    fields: &[GfxFormField],
    values: &[String],
    form_idx: usize,
) -> String {
    let mut body = String::new();
    let mut first = true;
    for f in fields.iter().filter(|f| f.form_idx == form_idx) {
        let val = values
            .get(f.field_idx)
            .cloned()
            .unwrap_or_else(|| f.value.clone());
        if f.name.is_empty() {
            continue;
        }
        if !first {
            body.push('&');
        }
        first = false;
        body.push_str(&urlencode(&f.name));
        body.push('=');
        body.push_str(&urlencode(&val));
    }
    body
}

pub fn submit_form_post(url: &str, body: &str) -> Result<PageBody, FetchError> {
    crate::net::ensure_wall_clock();
    let resp = https_post_full_typed(&Net, url, None, body.as_bytes(), "application/x-www-form-urlencoded")
        .map_err(FetchError::Http)?;
    let enc = header_value(&resp.headers, "content-encoding");
    let raw = maybe_decompress(&resp.body, enc).map_err(|_| FetchError::Status(resp.status))?;
    let charset = header_value(&resp.headers, "content-type")
        .and_then(|ct| ct.split("charset=").nth(1).map(|s| s.trim()));
    let html = bytes_to_text(&raw, charset);
    Ok(PageBody {
        html,
        base: url.to_string(),
        status: resp.status,
    })
}

pub fn form_for_submit<'a>(forms: &'a [GfxForm], form_idx: usize) -> Option<&'a GfxForm> {
    forms.iter().find(|f| f.idx == form_idx)
}
