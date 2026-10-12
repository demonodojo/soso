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
const READ_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Clone, Copy, Default)]
enum ReleaseKind {
    #[default]
    Cargo,
    FakeOk,
    FakeFail,
    /// Solo `hola-std` (demo B3); el pack es el ELF ligado al build-id.
    HolaStd,
}

#[derive(Default)]
struct Estado {
    manifest: String,
    build_id: String,
    build_lock: bool,
    synced: Vec<String>,
    release: ReleaseKind,
}

#[derive(Debug, PartialEq, Eq)]
struct HttpRequest<'a> {
    method: &'a str,
    path: &'a str,
    headers: &'a str,
    body: &'a [u8],
}

fn parse_http_request(raw: &[u8]) -> Option<HttpRequest<'_>> {
    let header_end = raw.windows(4).position(|w| w == b"\r\n\r\n")? + 4;
    let headers = std::str::from_utf8(&raw[..header_end]).ok()?;
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

fn sha256_hex(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn verify_hashes(manifest: &str, files: &[(String, Vec<u8>)]) -> Result<(), &'static str> {
    let mut expected = std::collections::HashMap::new();
    for line in manifest.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let (path, hash) = line.split_once('\t').ok_or("manifest malformado")?;
        expected.insert(path.to_string(), hash.to_string());
    }
    if expected.len() != files.len() {
        return Err("manifiesto y datos no coinciden");
    }
    for (path, data) in files {
        let want = expected.get(path).ok_or("fichero no listado")?;
        if sha256_hex(data) != *want {
            return Err("hash no coincide");
        }
    }
    Ok(())
}

fn read_http_request(stream: &mut TcpStream) -> io::Result<Vec<u8>> {
    read_http_request_timeout(stream, READ_TIMEOUT)
}

fn read_http_request_timeout(stream: &mut TcpStream, timeout: Duration) -> io::Result<Vec<u8>> {
    stream.set_read_timeout(Some(timeout))?;
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
    std::env::var("SOSO_FORJA_WORK")
        .map(PathBuf::from)
        .unwrap_or_else(|_| root.join("target/forja-work"))
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

fn apply_sync(work: &Path, files: &[(String, Vec<u8>)], prev: &[String]) -> io::Result<Vec<String>> {
    let mut new_rels = Vec::new();
    for (rel, data) in files {
        let Some(rel) = normalize_rel(rel) else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "ruta inválida",
            ));
        };
        let key = rel.to_string_lossy().replace('\\', "/");
        let dst = work.join(&rel);
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(dst, data)?;
        new_rels.push(key);
    }
    for old in prev {
        if !new_rels.iter().any(|n| n == old) {
            let _ = fs::remove_file(work.join(old));
        }
    }
    Ok(new_rels)
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

fn release_dir_for(work: &Path) -> Option<PathBuf> {
    let ver = fs::read_to_string(work.join("VERSION")).ok()?;
    Some(work.join(format!("target/release-soso/v{}", ver.trim())))
}

fn write_identified_artifacts(work: &Path, build_id: &str, pack: &[u8], kernel: &[u8]) -> io::Result<()> {
    let dir = release_dir_for(work).ok_or_else(|| {
        io::Error::new(io::ErrorKind::NotFound, "VERSION")
    })?;
    fs::create_dir_all(&dir)?;
    let manifest = format!(
        "forja-id={build_id}\nsources={}\npack-size={}\n",
        sha256_hex(pack),
        pack.len()
    );
    fs::write(dir.join("manifest.txt"), manifest)?;
    fs::write(dir.join("rootfs.pack"), pack)?;
    fs::write(dir.join("kernel-x86_64"), kernel)?;
    Ok(())
}

fn pack_from_synced(work: &Path, build_id: &str, synced: &[String]) -> Vec<u8> {
    let mut pack = format!("forja-build-id={build_id}\n").into_bytes();
    for rel in synced {
        pack.extend_from_slice(b"-- ");
        pack.extend_from_slice(rel.as_bytes());
        pack.extend_from_slice(b" --\n");
        if let Ok(data) = fs::read(work.join(rel)) {
            pack.extend_from_slice(&data);
            if !data.ends_with(b"\n") {
                pack.push(b'\n');
            }
        }
    }
    pack
}

fn hola_target_dir(work: &Path) -> PathBuf {
    if let Some(p) = std::env::var_os("SOSO_FORJA_HOLA_TARGET") {
        return PathBuf::from(p);
    }
    let checkout = project_root().join("user/target");
    if checkout.is_dir() {
        return checkout;
    }
    work.join("target/hola-std")
}

fn run_hola_std(work: &Path, build_id: &str) -> bool {
    let user = work.join("user");
    if !user.join("hola-std/src/main.rs").is_file() {
        eprintln!("forja-server: falta user/hola-std en el árbol de trabajo");
        return false;
    }
    let target_dir = hola_target_dir(work);
    let out = Command::new("cargo")
        .current_dir(&user)
        .env("CARGO_TARGET_DIR", &target_dir)
        .args(["build", "--release", "-p", "hola-std"])
        .output();
    match out {
        Ok(o) if o.status.success() => {
            let elf = target_dir.join("x86_64-soso-user/release/hola-std");
            let bytes = match fs::read(&elf) {
                Ok(b) => b,
                Err(e) => {
                    eprintln!("forja-server: no leí {}: {e}", elf.display());
                    return false;
                }
            };
            if !bytes.windows(4).any(|w| w == b"\x7fELF") {
                eprintln!("forja-server: hola-std no es ELF");
                return false;
            }
            write_identified_artifacts(work, build_id, &bytes, build_id.as_bytes()).is_ok()
        }
        Ok(o) => {
            eprintln!(
                "forja-server: hola-std fallo: {}",
                String::from_utf8_lossy(&o.stderr)
            );
            false
        }
        Err(e) => {
            eprintln!("forja-server: hola-std: {e}");
            false
        }
    }
}

fn run_release(work: &Path, kind: ReleaseKind, build_id: &str, synced: &[String]) -> bool {
    match kind {
        ReleaseKind::FakeFail => return false,
        ReleaseKind::FakeOk => {
            let pack = pack_from_synced(work, build_id, synced);
            return write_identified_artifacts(work, build_id, &pack, build_id.as_bytes()).is_ok();
        }
        ReleaseKind::HolaStd => return run_hola_std(work, build_id),
        ReleaseKind::Cargo => {}
    }
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
            if let Some(dir) = release_dir(work) {
                let p = dir.join("manifest.txt");
                if let Ok(mut t) = fs::read_to_string(&p) {
                    if !t.contains("forja-id=") {
                        t.push_str(&format!("forja-id={build_id}\n"));
                        let _ = fs::write(p, t);
                    }
                }
            }
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

fn release_kind_from_env() -> ReleaseKind {
    match std::env::var("SOSO_FORJA_RELEASE").ok().as_deref() {
        Some("hola-std") => ReleaseKind::HolaStd,
        Some("fake-ok") => ReleaseKind::FakeOk,
        Some("fake-fail") => ReleaseKind::FakeFail,
        _ => ReleaseKind::Cargo,
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

fn auth_ok(headers: &str, token: Option<&str>) -> bool {
    let Some(token) = token else {
        return true;
    };
    header_value(headers, "Authorization")
        .is_some_and(|v| v == format!("Bearer {token}"))
}

fn bind_host(bind: &str) -> &str {
    if let Some(rest) = bind.strip_prefix('[') {
        return rest.split(']').next().unwrap_or(bind);
    }
    match bind.rsplit_once(':') {
        Some((h, port)) if !port.is_empty() && port.chars().all(|c| c.is_ascii_digit()) => h,
        _ => bind,
    }
}

fn bind_is_loopback(bind: &str) -> bool {
    matches!(bind_host(bind), "127.0.0.1" | "::1" | "localhost")
}

/// En 0.0.0.0 / LAN el token no es opcional: cualquiera podría sync/build.
fn resolve_listen_token(
    bind: &str,
    token: Option<String>,
) -> Result<Option<String>, &'static str> {
    let token = token.filter(|t| !t.is_empty());
    if bind_is_loopback(bind) {
        Ok(token)
    } else {
        Ok(Some(token.ok_or("SOSO_FORJA_TOKEN obligatorio si el bind no es loopback")?))
    }
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

fn manejar(
    mut stream: TcpStream,
    estado: Arc<Mutex<Estado>>,
    root: PathBuf,
    work: PathBuf,
    token: Option<String>,
) {
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
    if !auth_ok(parsed.headers, token.as_deref()) {
        let _ = responder(&mut stream, "401 Unauthorized", b"auth\n", "");
        return;
    }
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
            if let Err(why) = verify_hashes(&manifest, &files) {
                let msg = format!("sync hash: {why}\n");
                let _ = responder(&mut stream, "400 Bad Request", msg.as_bytes(), "");
                return;
            }
            if ensure_worktree(&root, &work).is_err() {
                let _ = responder(&mut stream, "500 Error", b"worktree\n", "");
                return;
            }
            let prev = estado.lock().unwrap().synced.clone();
            let synced = match apply_sync(&work, &files, &prev) {
                Ok(s) => s,
                Err(e) if e.kind() == io::ErrorKind::InvalidInput => {
                    let _ = responder(&mut stream, "400 Bad Request", b"ruta invalida\n", "");
                    return;
                }
                Err(e) => {
                    eprintln!("forja-server: sync write: {e}");
                    let _ = responder(&mut stream, "500 Error", b"write\n", "");
                    return;
                }
            };
            let n = synced.len();
            let id = manifest_id(&manifest);
            {
                let mut st = estado.lock().unwrap();
                st.manifest = manifest;
                st.build_id = id.clone();
                st.synced = synced;
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
            let kind = st.release;
            let synced = st.synced.clone();
            drop(st);
            if ensure_worktree(&root, &work).is_err()
                || !run_release(&work, kind, &build_id, &synced)
            {
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
        ("GET", "/build-id") => {
            let id = estado.lock().unwrap().build_id.clone();
            if id.is_empty() {
                let _ = responder(&mut stream, "404 Not Found", b"sin sync\n", "");
            } else {
                let hdr = format!("X-Forja-Build-Id: {id}\r\n");
                let _ = responder(&mut stream, "200 OK", format!("{id}\n").as_bytes(), &hdr);
            }
        }
        _ => {
            let _ = responder(&mut stream, "404 Not Found", b"?\n", "");
        }
    }
}

fn main() {
    let bind = std::env::var("SOSO_FORJA_BIND").unwrap_or_else(|_| "0.0.0.0:8740".into());
    let token = match resolve_listen_token(&bind, std::env::var("SOSO_FORJA_TOKEN").ok()) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("forja-server: {e} (SOSO_FORJA_BIND={bind})");
            std::process::exit(1);
        }
    };
    let root = project_root();
    let work = work_root(&root);
    let estado = Arc::new(Mutex::new(Estado {
        release: release_kind_from_env(),
        ..Estado::default()
    }));
    let listener = TcpListener::bind(&bind).unwrap_or_else(|e| panic!("bind {bind}: {e}"));
    eprintln!("forja-server en {bind}");
    for conn in listener.incoming() {
        if let Ok(stream) = conn {
            let st = estado.clone();
            let root = root.clone();
            let work = work.clone();
            let token = token.clone();
            std::thread::spawn(move || manejar(stream, st, root, work, token));
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

    #[test]
    fn parse_binary_body() {
        let mut req = b"POST /sync HTTP/1.1\r\nContent-Length: 3\r\n\r\n".to_vec();
        req.extend_from_slice(&[0x00, 0xff, 0xfe]);
        let p = parse_http_request(&req).expect("parse binario");
        assert_eq!(p.body, &[0x00, 0xff, 0xfe]);
    }

    #[test]
    fn parse_sync_binary_file() {
        let mut body = b"bin.dat\tdead\n---DATA---\nbin.dat\n3\n".to_vec();
        body.extend_from_slice(&[0x00, 0xff, 0xfe]);
        let (_, files) = parse_sync_body(&body).expect("sync binario");
        assert_eq!(files[0].1, &[0x00, 0xff, 0xfe]);
    }

    #[test]
    fn verify_hashes_ok_and_mismatch() {
        let data = b"abc";
        let h = sha256_hex(data);
        let manifest = format!("a.rs\t{h}\n");
        verify_hashes(&manifest, &[("a.rs".into(), data.to_vec())]).unwrap();
        assert!(verify_hashes(&manifest, &[("a.rs".into(), b"xyz".to_vec())]).is_err());
        assert!(verify_hashes("a.rs\n", &[("a.rs".into(), data.to_vec())]).is_err());
    }

    fn start_test_server(
        work: PathBuf,
        release: ReleaseKind,
        token: Option<&str>,
    ) -> (std::net::SocketAddr, Arc<Mutex<Estado>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let estado = Arc::new(Mutex::new(Estado {
            release,
            ..Estado::default()
        }));
        let st = estado.clone();
        let root = project_root();
        let token = token.map(String::from);
        std::thread::spawn(move || {
            for conn in listener.incoming() {
                if let Ok(stream) = conn {
                    manejar(stream, st.clone(), root.clone(), work.clone(), token.clone());
                }
            }
        });
        (addr, estado)
    }

    fn http_raw(addr: std::net::SocketAddr, bytes: &[u8]) -> Vec<u8> {
        let mut s = TcpStream::connect(addr).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        s.write_all(bytes).unwrap();
        let mut out = Vec::new();
        let _ = s.read_to_end(&mut out);
        out
    }

    fn status_of(resp: &[u8]) -> u16 {
        let headers = match resp.windows(4).position(|w| w == b"\r\n\r\n") {
            Some(i) => &resp[..i],
            None => resp,
        };
        let text = std::str::from_utf8(headers).unwrap_or("");
        text.split_whitespace()
            .nth(1)
            .and_then(|s| s.parse().ok())
            .unwrap_or(0)
    }

    fn temp_work() -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "forja-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&d).unwrap();
        fs::write(d.join("VERSION"), "0.2.2\n").unwrap();
        d
    }

    fn sync_body(rel: &str, data: &[u8]) -> Vec<u8> {
        let h = sha256_hex(data);
        let mut body = format!("{rel}\t{h}\n").into_bytes();
        body.extend_from_slice(DATA_SEP);
        body.extend_from_slice(rel.as_bytes());
        body.push(b'\n');
        body.extend_from_slice(format!("{}\n", data.len()).as_bytes());
        body.extend_from_slice(data);
        body
    }

    #[test]
    fn http_fragmented_and_large_body() {
        let work = temp_work();
        let (addr, _) = start_test_server(work.clone(), ReleaseKind::FakeOk, None);
        let data = vec![b'x'; 5000];
        let body = sync_body("big.txt", &data);
        let mut req = format!(
            "POST /sync HTTP/1.1\r\nContent-Length: {}\r\n\r\n",
            body.len()
        )
        .into_bytes();
        req.extend_from_slice(&body);
        let mut s = TcpStream::connect(addr).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        for chunk in req.chunks(64) {
            s.write_all(chunk).unwrap();
        }
        let mut out = Vec::new();
        let _ = s.read_to_end(&mut out);
        assert_eq!(status_of(&out), 200);
        assert_eq!(fs::read(work.join("big.txt")).unwrap(), data);
        let _ = fs::remove_dir_all(work);
    }

    #[test]
    fn http_rejects_invalid_path() {
        let work = temp_work();
        let (addr, _) = start_test_server(work.clone(), ReleaseKind::FakeOk, None);
        let data = b"x";
        let h = sha256_hex(data);
        let mut body = format!("../etc/passwd\t{h}\n").into_bytes();
        body.extend_from_slice(DATA_SEP);
        body.extend_from_slice(b"../etc/passwd\n1\nx");
        let mut req = format!(
            "POST /sync HTTP/1.1\r\nContent-Length: {}\r\n\r\n",
            body.len()
        )
        .into_bytes();
        req.extend_from_slice(&body);
        let resp = http_raw(addr, &req);
        assert_eq!(status_of(&resp), 400);
        let _ = fs::remove_dir_all(work);
    }

    #[test]
    fn http_build_500_and_503() {
        let work = temp_work();
        let (addr, estado) = start_test_server(work.clone(), ReleaseKind::FakeFail, None);
        let resp = http_raw(addr, b"POST /build HTTP/1.1\r\nContent-Length: 0\r\n\r\n");
        assert_eq!(status_of(&resp), 500);
        estado.lock().unwrap().build_lock = true;
        estado.lock().unwrap().release = ReleaseKind::FakeOk;
        let resp = http_raw(addr, b"POST /build HTTP/1.1\r\nContent-Length: 0\r\n\r\n");
        assert_eq!(status_of(&resp), 503);
        let _ = fs::remove_dir_all(work);
    }

    #[test]
    fn http_timeout_incomplete() {
        let pair = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = pair.local_addr().unwrap();
        let h = std::thread::spawn(move || {
            let (mut stream, _) = pair.accept().unwrap();
            read_http_request_timeout(&mut stream, Duration::from_millis(200))
        });
        let _client = TcpStream::connect(addr).unwrap();
        let err = h.join().unwrap().unwrap_err();
        assert!(
            err.kind() == io::ErrorKind::TimedOut || err.kind() == io::ErrorKind::WouldBlock,
            "kind={err:?}"
        );
    }

    #[test]
    fn http_auth_required() {
        let work = temp_work();
        let (addr, _) = start_test_server(work.clone(), ReleaseKind::FakeOk, Some("secret"));
        let body = sync_body("a.rs", b"hi");
        let mut req = format!(
            "POST /sync HTTP/1.1\r\nContent-Length: {}\r\n\r\n",
            body.len()
        )
        .into_bytes();
        req.extend_from_slice(&body);
        let resp = http_raw(addr, &req);
        assert_eq!(status_of(&resp), 401);
        let mut ok = format!(
            "POST /sync HTTP/1.1\r\nAuthorization: Bearer secret\r\nContent-Length: {}\r\n\r\n",
            body.len()
        )
        .into_bytes();
        ok.extend_from_slice(&body);
        assert_eq!(status_of(&http_raw(addr, &ok)), 200);
        assert_eq!(
            status_of(&http_raw(addr, b"GET /build-id HTTP/1.1\r\nHost: x\r\n\r\n")),
            401
        );
        assert_eq!(
            status_of(&http_raw(
                addr,
                b"GET /build-id HTTP/1.1\r\nAuthorization: Bearer secret\r\nHost: x\r\n\r\n"
            )),
            200
        );
        let _ = fs::remove_dir_all(work);
    }

    #[test]
    fn public_bind_requires_token() {
        assert!(bind_is_loopback("127.0.0.1:8740"));
        assert!(bind_is_loopback("[::1]:8740"));
        assert!(bind_is_loopback("localhost:8740"));
        assert!(!bind_is_loopback("0.0.0.0:8740"));
        assert!(!bind_is_loopback("[::]:8740"));
        assert!(resolve_listen_token("127.0.0.1:8740", None).unwrap().is_none());
        assert!(resolve_listen_token("0.0.0.0:8740", None).is_err());
        assert!(resolve_listen_token("0.0.0.0:8740", Some(String::new())).is_err());
        assert_eq!(
            resolve_listen_token("0.0.0.0:8740", Some("s".into())).unwrap(),
            Some("s".into())
        );
    }

    fn header_of(resp: &[u8], name: &str) -> Option<String> {
        let sep = resp.windows(4).position(|w| w == b"\r\n\r\n")?;
        let headers = std::str::from_utf8(&resp[..sep]).ok()?;
        header_value(headers, name).map(str::to_string)
    }

    fn body_of(resp: &[u8]) -> &[u8] {
        match resp.windows(4).position(|w| w == b"\r\n\r\n") {
            Some(i) => &resp[i + 4..],
            None => resp,
        }
    }

    fn post_sync(addr: std::net::SocketAddr, rel: &str, data: &[u8]) -> Vec<u8> {
        let body = sync_body(rel, data);
        let mut req = format!(
            "POST /sync HTTP/1.1\r\nContent-Length: {}\r\n\r\n",
            body.len()
        )
        .into_bytes();
        req.extend_from_slice(&body);
        http_raw(addr, &req)
    }

    /// Demo B3 (host): un mensaje nuevo de hola-std viaja en el sync, el
    /// build-id identifica esos fuentes y el pack los contiene.
    #[test]
    fn hola_std_demo_identified_artifact() {
        let work = temp_work();
        let (addr, _) = start_test_server(work.clone(), ReleaseKind::FakeOk, None);
        let msg = "hola-astra-B3-demo";
        let src = format!(
            "//! demo B3\nfn main() {{ println!(\"{msg}\"); }}\n"
        );
        let rel = "user/hola-std/src/main.rs";
        let sync = post_sync(addr, rel, src.as_bytes());
        assert_eq!(status_of(&sync), 200);
        let sync_txt = std::str::from_utf8(body_of(&sync)).unwrap();
        let id = sync_txt
            .lines()
            .find_map(|l| l.strip_prefix("build-id="))
            .expect("build-id en sync");
        assert_eq!(fs::read_to_string(work.join(rel)).unwrap(), src);

        let build = http_raw(addr, b"POST /build HTTP/1.1\r\nContent-Length: 0\r\n\r\n");
        assert_eq!(status_of(&build), 200);
        assert_eq!(header_of(&build, "X-Forja-Build-Id").as_deref(), Some(id));
        let pack = body_of(&build);
        assert!(pack.windows(msg.len()).any(|w| w == msg.as_bytes()));
        assert!(
            pack.windows(id.len()).any(|w| w == id.as_bytes()),
            "el pack debe llevar el build-id"
        );

        let mid = http_raw(addr, b"GET /build-id HTTP/1.1\r\nHost: x\r\n\r\n");
        assert_eq!(status_of(&mid), 200);
        assert_eq!(std::str::from_utf8(body_of(&mid)).unwrap().trim(), id);

        let man = http_raw(addr, b"GET /manifest.txt HTTP/1.1\r\nHost: x\r\n\r\n");
        assert_eq!(status_of(&man), 200);
        let man_txt = std::str::from_utf8(body_of(&man)).unwrap();
        assert!(man_txt.contains(&format!("forja-id={id}")));
        assert!(man_txt.contains(&format!("sources={}", sha256_hex(pack))));

        let src2 = src.replace(msg, "hola-astra-B3-otro");
        let sync2 = post_sync(addr, rel, src2.as_bytes());
        let id2 = std::str::from_utf8(body_of(&sync2))
            .unwrap()
            .lines()
            .find_map(|l| l.strip_prefix("build-id="))
            .unwrap();
        assert_ne!(id, id2, "otro mensaje → otra identidad");
        let _ = fs::remove_dir_all(work);
    }

    fn seed_hola_std_workspace(work: &Path) {
        let root = project_root();
        fs::create_dir_all(work.join("user")).unwrap();
        fs::create_dir_all(work.join("crates")).unwrap();
        let _ = fs::copy(root.join("rust-toolchain.toml"), work.join("rust-toolchain.toml"));
        fs::write(
            work.join("user/Cargo.toml"),
            r#"[workspace]
resolver = "2"
members = ["libsoso", "hola-std"]

[workspace.dependencies]
soso-abi = { path = "../crates/soso-abi" }
libsoso = { path = "libsoso" }
getrandom = { version = "0.2", features = ["rdrand"] }

[profile.release]
panic = "abort"
opt-level = "s"
strip = "debuginfo"
"#,
        )
        .unwrap();
        copy_tree(&root.join("user/libsoso"), &work.join("user/libsoso"), &root.join("user/libsoso"))
            .unwrap();
        copy_tree(
            &root.join("user/hola-std"),
            &work.join("user/hola-std"),
            &root.join("user/hola-std"),
        )
        .unwrap();
        copy_tree(
            &root.join("crates/soso-abi"),
            &work.join("crates/soso-abi"),
            &root.join("crates/soso-abi"),
        )
        .unwrap();
        copy_tree(
            &root.join("crates/soso-std"),
            &work.join("crates/soso-std"),
            &root.join("crates/soso-std"),
        )
        .unwrap();
        let _ = fs::copy(root.join("user/Cargo.lock"), work.join("user/Cargo.lock"));
        let _ = fs::copy(root.join("user/link.ld"), work.join("user/link.ld"));
        let _ = fs::copy(
            root.join("user/x86_64-soso-user.json"),
            work.join("user/x86_64-soso-user.json"),
        );
        fs::create_dir_all(work.join("user/.cargo")).unwrap();
        let _ = fs::copy(
            root.join("user/.cargo/config.toml"),
            work.join("user/.cargo/config.toml"),
        );
    }

    /// Compila el ELF de hola-std con el mensaje sincronizado (reusa
    /// `user/target` del checkout si existe).
    #[test]
    fn hola_std_compiles_synced_message() {
        let work = temp_work();
        seed_hola_std_workspace(&work);
        let (addr, _) = start_test_server(work.clone(), ReleaseKind::HolaStd, None);
        let msg = "hola-astra-B3-compiled";
        let src = format!(
            r#"//! Hola mundo con `soso-std` (demo B3).

#![no_std]
#![no_main]

extern crate alloc;

use soso_std::println;

libsoso::entry!(main);

fn main(args: &str) -> u8 {{
    libsoso::heap_init();
    soso_std::init_from_args(args);
    soso_std::init();
    println!("{msg}");
    0
}}
"#
        );
        let rel = "user/hola-std/src/main.rs";
        let sync = post_sync(addr, rel, src.as_bytes());
        assert_eq!(status_of(&sync), 200, "{}", String::from_utf8_lossy(&sync));

        let mut s = TcpStream::connect(addr).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(180))).unwrap();
        s.write_all(b"POST /build HTTP/1.1\r\nContent-Length: 0\r\n\r\n")
            .unwrap();
        let mut build = Vec::new();
        let _ = s.read_to_end(&mut build);
        assert_eq!(
            status_of(&build),
            200,
            "{}",
            String::from_utf8_lossy(&build)
        );
        let pack = body_of(&build);
        assert!(pack.starts_with(b"\x7fELF"), "el pack debe ser el ELF");
        assert!(
            pack.windows(msg.len()).any(|w| w == msg.as_bytes()),
            "el ELF debe incrustar el mensaje sincronizado"
        );
        let _ = fs::remove_dir_all(work);
    }
}
