//! Servidor de forja (host): recibe sync/build y ejecuta `cargo xtask`.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct Estado {
    manifest: String,
}

#[derive(Debug, PartialEq, Eq)]
struct HttpRequest<'a> {
    method: &'a str,
    path: &'a str,
    body: &'a [u8],
}

fn parse_http_request(raw: &[u8]) -> Option<HttpRequest<'_>> {
    let req_str = std::str::from_utf8(raw).ok()?;
    let primera = req_str.lines().next()?;
    let mut partes = primera.split_whitespace();
    let method = partes.next()?;
    let path = partes.next()?;
    let body_off = req_str.find("\r\n\r\n").map(|i| i + 4).unwrap_or(raw.len());
    let body = &raw[body_off.min(raw.len())..];
    Some(HttpRequest { method, path, body })
}

fn project_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn leer_cuerpo(stream: &mut TcpStream) -> std::io::Result<Vec<u8>> {
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf)?;
    Ok(buf)
}

fn responder(stream: &mut TcpStream, status: &str, body: &[u8]) -> std::io::Result<()> {
    let hdr = format!(
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(hdr.as_bytes())?;
    stream.write_all(body)?;
    Ok(())
}

fn manejar(mut stream: TcpStream, estado: Arc<Mutex<Estado>>) {
    let mut req = [0u8; 4096];
    let n = stream.read(&mut req).unwrap_or(0);
    if n == 0 {
        return;
    }
    let Some(parsed) = parse_http_request(&req[..n]) else {
        let _ = responder(&mut stream, "400 Bad Request", b"?\n");
        return;
    };
    let mut body = parsed.body.to_vec();
    match (parsed.method, parsed.path) {
        ("POST", "/sync") => {
            let mut extra = leer_cuerpo(&mut stream).unwrap_or_default();
            body.append(&mut extra);
            estado.lock().unwrap().manifest = String::from_utf8_lossy(&body).into_owned();
            let _ = responder(&mut stream, "200 OK", b"OK\n");
        }
        ("POST", "/build") => {
            let root = project_root();
            let out = Command::new("cargo")
                .current_dir(&root)
                .args(["xtask", "release"])
                .output();
            match out {
                Ok(o) if o.status.success() => {
                    let ver = std::fs::read_to_string(root.join("VERSION")).unwrap_or_default();
                    let ver = ver.trim();
                    let pack = root.join(format!("target/release-soso/v{ver}/rootfs.pack"));
                    match std::fs::read(&pack) {
                        Ok(data) => {
                            let _ = responder(&mut stream, "200 OK", &data);
                        }
                        Err(_) => {
                            let _ = responder(&mut stream, "500 Error", b"sin rootfs.pack\n");
                        }
                    }
                }
                Ok(o) => {
                    eprintln!("forja-server: build falló: {}", String::from_utf8_lossy(&o.stderr));
                    let _ = responder(&mut stream, "500 Error", b"build failed\n");
                }
                Err(e) => {
                    eprintln!("forja-server: {e}");
                    let _ = responder(&mut stream, "500 Error", b"spawn failed\n");
                }
            }
        }
        _ => {
            let _ = responder(&mut stream, "404 Not Found", b"?\n");
        }
    }
}

fn main() {
    let port: u16 = std::env::var("SOSO_FORJA_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(8740);
    let estado = Arc::new(Mutex::new(Estado::default()));
    let listener = TcpListener::bind(format!("0.0.0.0:{port}")).expect("bind");
    eprintln!("soso-forja-server en :{port}");
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
    fn parse_post_build_sin_cuerpo() {
        let req = b"POST /build HTTP/1.1\r\nHost: x\r\n\r\n";
        let p = parse_http_request(req).expect("parse");
        assert_eq!(p.path, "/build");
        assert!(p.body.is_empty());
    }

    #[test]
    fn parse_get_404_path() {
        let req = b"GET /nope HTTP/1.1\r\n\r\n";
        let p = parse_http_request(req).expect("parse");
        assert_eq!(p.method, "GET");
        assert_eq!(p.path, "/nope");
    }
}
