//! Marcadores locales en `/etc/web-marcadores`.

use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use core::fmt::Write;
use libsoso::sys;

pub const PATH: &str = "/etc/web-marcadores";
pub const SCHEME: &str = "soso:bookmarks";

pub fn read_all() -> Vec<(String, String)> {
    let fd = sys::open(PATH, soso_abi::O_RDONLY);
    if fd < 0 {
        return Vec::new();
    }
    let fd = fd as u64;
    let mut st = soso_abi::Stat::default();
    if sys::stat(PATH, &mut st) < 0 || st.size <= 0 || st.size > 32 * 1024 {
        let _ = sys::close(fd);
        return Vec::new();
    }
    let mut buf = vec![0u8; st.size as usize];
    let n = sys::read(fd, &mut buf);
    let _ = sys::close(fd);
    if n <= 0 {
        return Vec::new();
    }
    buf.truncate(n as usize);
    let text = core::str::from_utf8(&buf).unwrap_or("");
    parse_lines(text)
}

fn parse_lines(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((t, u)) = line.split_once('\t') {
            if !u.is_empty() {
                out.push((t.to_string(), u.to_string()));
            }
        }
    }
    out
}

pub fn contains_url(url: &str) -> bool {
    read_all().iter().any(|(_, u)| u == url)
}

pub fn toggle(title: &str, url: &str) -> bool {
    let mut items = read_all();
    if let Some(i) = items.iter().position(|(_, u)| u == url) {
        items.remove(i);
        write_all(&items);
        false
    } else {
        items.push((title.to_string(), url.to_string()));
        write_all(&items);
        true
    }
}

fn write_all(items: &[(String, String)]) {
    let mut body = String::new();
    for (t, u) in items {
        body.push_str(t);
        body.push('\t');
        body.push_str(u);
        body.push('\n');
    }
    let bytes = body.as_bytes();
    let fd = sys::open(PATH, soso_abi::O_WRONLY | soso_abi::O_CREAT | soso_abi::O_TRUNC);
    if fd < 0 {
        return;
    }
    let fd = fd as u64;
    let _ = sys::write(fd, bytes);
    let _ = sys::close(fd);
}

pub fn list_html() -> String {
    let mut html = String::from(
        "<!DOCTYPE html><html><head><meta charset=\"utf-8\"><title>Marcadores</title></head><body><h1>Marcadores</h1><ul>",
    );
    for (t, u) in read_all() {
        let _ = write!(html, "<li><a href=\"{u}\">{t}</a></li>");
    }
    html.push_str("</ul></body></html>");
    html
}
