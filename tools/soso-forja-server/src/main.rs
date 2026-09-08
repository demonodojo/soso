//! Servidor de forja (host): sync de fuentes → build identificado en árbol aislado.

use std::fs;
use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;

const DATA_SEP: &[u8] = b"\n---DATA---\n";
const MAX_BODY: usize = 64 * 1024 * 1024;

#[derive(Default)]
struct Estado {
    manifest: String,
    build_id: String,
    build_lock: bool,
}

#[derive(Debug, PartialEq, Eq)]
struct HttpRequest<'a> {
    method: &'a str,
    path: &'a str,
    headers: &'a str,
    body: &'a [u8],
}

fn parse_http_request(raw: &[u8]) -> Option<HttpRequest<'_>> {
    let req_str = std::str::from_utf8(raw).ok()?;
    let header_end = req_str.find("\r\n\r\n").map(|i| i + 4)?;
    let headers = &req_str[..header_end];
    let primera = headers.lines().next()?;
    let mut partes = primera.split_whitespace();
    let method = partes.next()?;
    let path = partes.next()?;
    let body = &raw[header_end.min(raw.len())..];
    Some(HttpRequest {
        method,
        path,
        headers,
        body,
    })
}

fn header_value<'a>(headers: &'a str, name: &str) -> Option<&'a str> {
    let needle = name.to_ascii_lowercase();
    headers.lines().skip(1).find_map(|line| {
        let (k, v) = line.split_once(':')?;
        if k.trim().eq_ignore_ascii_case(&needle) {
            Some(v.trim())
        } else {
            None
        }
    })
}

fn read_http_request(stream: &mut TcpStream) -> io::Result<Vec<u8>> {
    stream.set_read_timeout(Some(Duration::from_secs(120)))?;
    let mut buf = Vec::new();
    let mut tmp = [0u8; 4096];
    loop {
        let n = stream.read(&mut tmp)?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&tmp[..n]);
        if buf.len() > MAX_BODY + 65536 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "petición demasiado grande",
            ));
        }
        if buf.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
    }
    let Some(parsed) = parse_http_request(&buf) else {
        return Ok(buf);
    };
    let cl = header_value(parsed.headers, "Content-Length")
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(parsed.body.len());
    let header_end = buf
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|i| i + 4)
        .unwrap_or(buf.len());
    let need = header_end + cl;
    while buf.len() < need {
        let n = stream.read(&mut tmp)?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&tmp[..n]);
        if buf.len() > MAX_BODY + 65536 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "cuerpo demasiado grande",
            ));
        }
    }
    Ok(buf)
}

fn project_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn work_root(root: &Path) -> PathBuf {
    root.join("target/forja-work")
}

fn ensure_worktree(root: &Path, work: &Path) -> io::Result<()> {
    if work.join("VERSION").is_file() {
        return Ok(());
    }
    eprintln!("forja-server: inicializando árbol de trabajo en {}", work.display());
    copy_tree(root, work, root)?;
    Ok(())
}

fn copy_tree(src: &Path, dst: &Path, root: &Path) -> io::Result<()> {
    if src.is_file() {
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(src, dst)?;
        return Ok(());
    }
    if !src.is_dir() {
        return Ok(());
    }
    fs::create_dir_all(dst)?;
    for e in fs::read_dir(src)? {
        let e = e?;
        let name = e.file_name();
        let s = name.to_string_lossy();
        if s == "target" || s == ".git" || s.starts_with('.') {
            continue;
        }
        let path = e.path();
        let rel = path.strip_prefix(root).unwrap_or(&path);
        if rel.components().any(|c| matches!(c, Component::ParentDir)) {
            continue;
        }
        copy_tree(&path, &dst.join(name), root)?;
    }
    Ok(())
}

fn normalize_rel(path: &str) -> Option<PathBuf> {
    let p = Path::new(path);
    if p.is_absolute() {
        return None;
    }
    for c in p.components() {
        match c {
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
            _ => {}
        }
    }
    Some(p.to_path_buf())
}

fn parse_sync_body(body: &[u8]) -> Result<(String, Vec<(String, Vec<u8>)>), &'static str> {
    let sep = body
        .windows(DATA_SEP.len())
        .position(|w| w == DATA_SEP)
        .ok_or("sin separador ---DATA---")?;
    let manifest = std::str::from_utf8(&body[..sep])
        .map_err(|_| "manifest no UTF-8")?
        .to_string();
    let mut files = Vec::new();
    let mut rest = &body[sep + DATA_SEP.len()..];
    while !rest.is_empty() {
        let nl = rest.iter().position(|&b| b == b'\n').ok_or("ruta truncada")?;
        let path = std::str::from_utf8(&rest[..nl]).map_err(|_| "ruta no UTF-8")?;
        rest = &rest[nl + 1..];
        let nl2 = rest
            .iter()
            .position(|&b| b == b'\n')
            .ok_or("tamaño truncado")?;
        let size: usize = std::str::from_utf8(&rest[..nl2])
            .map_err(|_| "tamaño no UTF-8")?
            .parse()
            .map_err(|_| "tamaño inválido")?;
        rest = &rest[nl2 + 1..];
        if rest.len() < size {
            return Err("contenido truncado");
        }
        files.push((path.to_string(), rest[..size].to_vec()));
        rest = &rest[size..];
    }
    Ok((manifest, files))
}

fn apply_sync(work: &Path, files: &[(String, Vec<u8>)]) -> io::Result<usize> {
    let mut n = 0usize;
    for (rel, data) in files {
        let Some(rel) = normalize_rel(rel) else {
            continue;
        };
        let dst = work.join(rel);
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(dst, data)?;
        n += 1;
    }
    Ok(n)
}

fn manifest_id(manifest: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    manifest.hash(&mut h);
    format!("{:016x}", h.finish())
}

fn release_dir(work: &Path) -> Option<PathBuf> {
    let ver = fs::read_to_string(work.join("VERSION")).ok()?;
    let ver = ver.trim();
    let dir = work.join(format!("target/release-soso/v{ver}"));
    if dir.join("manifest.txt").is_file() {
        Some(dir)
    } else {
        None
    }
}

fn run_release(work: &Path) -> bool {
    let out = Command::new("cargo")
        .current_dir(work)
        .args(["xtask", "release"])
        .output();
    match out {
        Ok(o) if o.status.success() => {
            let _ = Command::new("cargo")
                .current_dir(work)
                .args(["xtask", "forja-out"])
                .status();
            true
        }
        Ok(o) => {
            eprintln!(
                "forja-server: build fallo: {}",
                String::from_utf8_lossy(&o.stderr)
            );
            false
        }
        Err(e) => {
            eprintln!("forja-server: {e}");
            false
        }
    }
}

fn responder(stream: &mut TcpStream, status: &str, body: &[u8], extra_hdr: &str) -> io::Result<()> {
    let hdr = format!(
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n{extra_hdr}\r\n",
        body.len()
    );
    stream.write_all(hdr.as_bytes())?;
    stream.write_all(body)?;
    stream.flush()?;
    Ok(())
}

fn auth_ok(headers: &str) -> bool {
    let Some(token) = std::env::var("SOSO_FORJA_TOKEN").ok() else {
        return true;
    };
    header_value(headers, "Authorization")
        .is_some_and(|v| v == format!("Bearer {token}"))
}

fn servir_artifact(stream: &mut TcpStream, work: &Path, name: &str, build_id: &str) {
    let Some(rel) = release_dir(work) else {
        let _ = responder(stream, "404 Not Found", b"sin release\n", "");
        return;
    };
    match fs::read(rel.join(name)) {
        Ok(data) => {
            let hdr = format!("X-Forja-Build-Id: {build_id}\r\n");
            let _ = responder(stream, "200 OK", &data, &hdr);
        }
        Err(_) => {
            let _ = responder(stream, "404 Not Found", b"missing\n", "");
        }
    }
}

fn manejar(mut stream: TcpStream, estado: Arc<Mutex<Estado>>) {
    let raw = match read_http_request(&mut stream) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("forja-server: lectura: {e}");
            let _ = responder(&mut stream, "400 Bad Request", b"read error\n", "");
            return;
        }
    };
    let Some(parsed) = parse_http_request(&raw) else {
        let _ = responder(&mut stream, "400 Bad Request", b"parse\n", "");
        return;
    };
    if parsed.method == "POST" && !auth_ok(parsed.headers) {
        let _ = responder(&mut stream, "401 Unauthorized", b"auth\n", "");
        return;
    }
    let root = project_root();
    let work = work_root(&root);
    match (parsed.method, parsed.path) {
        ("POST", "/sync") => {
            let (manifest, files) = match parse_sync_body(parsed.body) {
                Ok(v) => v,
                Err(why) => {
                    let msg = format!("sync parse: {why}\n");
                    let _ = responder(&mut stream, "400 Bad Request", msg.as_bytes(), "");
                    return;
                }
            };
            if ensure_worktree(&root, &work).is_err() {
                let _ = responder(&mut stream, "500 Error", b"worktree\n", "");
                return;
            }
            let n = match apply_sync(&work, &files) {
                Ok(n) => n,
                Err(e) => {
                    eprintln!("forja-server: sync write: {e}");
                    let _ = responder(&mut stream, "500 Error", b"write\n", "");
                    return;
                }
            };
            let id = manifest_id(&manifest);
            {
                let mut st = estado.lock().unwrap();
                st.manifest = manifest;
                st.build_id = id.clone();
            }
            let body = format!("OK {n} ficheros\nbuild-id={id}\n");
            let _ = responder(&mut stream, "200 OK", body.as_bytes(), "");
        }
        ("POST", "/build") => {
            let mut st = estado.lock().unwrap();
            if st.build_lock {
                let _ = responder(
                    &mut stream,
                    "503 Busy",
                    b"build en curso\n",
                    "",
                );
                return;
            }
            st.build_lock = true;
            let build_id = st.build_id.clone();
            drop(st);
            if ensure_worktree(&root, &work).is_err() || !run_release(&work) {
                estado.lock().unwrap().build_lock = false;
                let _ = responder(&mut stream, "500 Error", b"build failed\n", "");
                return;
            }
            estado.lock().unwrap().build_lock = false;
            servir_artifact(&mut stream, &work, "rootfs.pack", &build_id);
        }
        ("GET", "/manifest.txt") => {
            let id = estado.lock().unwrap().build_id.clone();
            servir_artifact(&mut stream, &work, "manifest.txt", &id);
        }
        ("GET", "/kernel-x86_64") => {
            let id = estado.lock().unwrap().build_id.clone();
            servir_artifact(&mut stream, &work, "kernel-x86_64", &id);
        }
        ("GET", "/rootfs.pack") => {
            let id = estado.lock().unwrap().build_id.clone();
            servir_artifact(&mut stream, &work, "rootfs.pack", &id);
        }
        _ => {
            let _ = responder(&mut stream, "404 Not Found", b"?\n", "");
        }
    }
}

fn main() {
    let bind = std::env::var("SOSO_FORJA_BIND").unwrap_or_else(|_| "0.0.0.0:8740".into());
    let estado = Arc::new(Mutex::new(Estado::default()));
    let listener = TcpListener::bind(&bind).unwrap_or_else(|e| panic!("bind {bind}: {e}"));
    eprintln!("forja-server en {bind}");
    for conn in listener.incoming() {
        if let Ok(stream) = conn {
            let st = estado.clone();
            std::thread::spawn(move || manejar(stream, st));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_post_sync() {
        let req = b"POST /sync HTTP/1.1\r\nContent-Length: 3\r\n\r\nabc";
        let p = parse_http_request(req).expect("parse");
        assert_eq!(p.method, "POST");
        assert_eq!(p.path, "/sync");
        assert_eq!(p.body, b"abc");
    }

    #[test]
    fn parse_get_kernel() {
        let req = b"GET /kernel-x86_64 HTTP/1.1\r\nHost: x\r\n\r\n";
        let p = parse_http_request(req).expect("parse");
        assert_eq!(p.method, "GET");
        assert_eq!(p.path, "/kernel-x86_64");
    }

    #[test]
    fn parse_sync_with_data() {
        let body = b"a.rs\tdeadbeef\n---DATA---\na.rs\n3\nabc";
        let (m, files) = parse_sync_body(body).expect("sync");
        assert!(m.contains("a.rs"));
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].0, "a.rs");
        assert_eq!(files[0].1, b"abc");
    }

    #[test]
    fn normalize_rejects_traversal() {
        assert!(normalize_rel("../etc/passwd").is_none());
        assert!(normalize_rel("/abs").is_none());
        assert!(normalize_rel("user/hola.rs").is_some());
    }
}
