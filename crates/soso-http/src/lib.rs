//! Cliente HTTP/1.1 + TLS mínimo para userspace soso (HTTPS GET).

#![no_std]

extern crate alloc;

use alloc::format;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::fmt::Write as _;
use core::time::Duration;

use rustls::client::UnbufferedClientConnection;
use rustls::pki_types::ServerName;
use rustls::time_provider::TimeProvider;
use rustls::unbuffered::ConnectionState;
use rustls::{ClientConfig, RootCertStore};
use rustls::pki_types::UnixTime;
use soso_abi::SockAddr;

/// Respuesta HTTP simplificada.
pub struct Response {
    pub status: u16,
    pub body: Vec<u8>,
}

/// Transporte TCP (DNS + connect + read/write).
pub trait TcpTransport {
    fn dns_resolve(&self, host: &str, out: &mut [u8; 4]) -> Result<(), i64>;
    fn tcp_connect(&self, addr: SockAddr, timeout_ms: u64) -> Result<u64, i64>;
    fn read_timeout(&self, fd: u64, buf: &mut [u8], timeout_ms: u64) -> i64;
    fn write_all(&self, fd: u64, data: &[u8]) -> Result<(), i64>;
    fn close(&self, fd: u64);
}

#[derive(Debug, Clone, Copy)]
struct FixedTimeProvider;

impl TimeProvider for FixedTimeProvider {
    fn current_time(&self) -> Option<UnixTime> {
        // Guest sin RTC: fecha fija reciente para validar certs HF (2025-01-01 UTC).
        Some(UnixTime::since_unix_epoch(Duration::from_secs(1_735_689_600)))
    }
}

fn client_config() -> Arc<ClientConfig> {
    let provider = rustls::crypto::ring::default_provider();
    let mut roots = RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    Arc::new(
        ClientConfig::builder_with_details(Arc::new(provider), Arc::new(FixedTimeProvider))
            .with_safe_default_protocol_versions()
            .expect("protocol versions")
            .with_root_certificates(roots)
            .with_no_client_auth(),
    )
}

struct TlsSession<'a, T: TcpTransport> {
    conn: UnbufferedClientConnection,
    fd: u64,
    transport: &'a T,
    incoming: Vec<u8>,
    outgoing_pending: Vec<u8>,
}

impl<'a, T: TcpTransport> TlsSession<'a, T> {
    fn new(transport: &'a T, fd: u64, host: &str) -> Result<Self, HttpError> {
        let name = ServerName::try_from(host.to_string()).map_err(|_| HttpError::Tls)?;
        let conn = UnbufferedClientConnection::new(client_config(), name).map_err(|_| HttpError::Tls)?;
        Ok(Self {
            conn,
            fd,
            transport,
            incoming: Vec::new(),
            outgoing_pending: Vec::new(),
        })
    }

    fn flush_pending(&mut self) -> Result<(), HttpError> {
        if !self.outgoing_pending.is_empty() {
            self.transport
                .write_all(self.fd, &self.outgoing_pending)
                .map_err(|_| HttpError::Io)?;
            self.outgoing_pending.clear();
        }
        Ok(())
    }

    fn read_more(&mut self, timeout_ms: u64) -> Result<bool, HttpError> {
        let mut buf = [0u8; 4096];
        let n = self.transport.read_timeout(self.fd, &mut buf, timeout_ms);
        if n <= 0 {
            return Ok(false);
        }
        self.incoming.extend_from_slice(&buf[..n as usize]);
        Ok(true)
    }

    fn handshake(&mut self) -> Result<(), HttpError> {
        let mut out_buf = [0u8; 8192];
        loop {
            let discard = {
                let mut scratch = self.incoming.as_mut_slice();
                let status = self.conn.process_tls_records(&mut scratch);
                let discard = status.discard;
                match status.state {
                    Err(_) => {
                        if !self.read_more(30_000)? {
                            return Err(HttpError::Io);
                        }
                    }
                    Ok(state) => match state {
                        ConnectionState::EncodeTlsData(mut enc) => {
                            let n = enc.encode(&mut out_buf).map_err(|_| HttpError::Tls)?;
                            drop(enc);
                            self.outgoing_pending.extend_from_slice(&out_buf[..n]);
                            self.flush_pending()?;
                        }
                        ConnectionState::TransmitTlsData(tx) => {
                            tx.done();
                        }
                        ConnectionState::BlockedHandshake => {
                            if !self.read_more(30_000)? {
                                return Err(HttpError::Io);
                            }
                        }
                        ConnectionState::WriteTraffic(_) => return Ok(()),
                        ConnectionState::ReadTraffic(_)
                        | ConnectionState::PeerClosed
                        | ConnectionState::Closed => return Ok(()),
                        _ => {}
                    },
                }
                discard
            };
            if discard > 0 {
                self.incoming.drain(..discard);
            }
        }
    }

    fn write(&mut self, data: &[u8]) -> Result<(), HttpError> {
        let mut out_buf = [0u8; 8192];
        let mut off = 0;
        while off < data.len() {
            let discard = {
                let mut scratch = self.incoming.as_mut_slice();
                let status = self.conn.process_tls_records(&mut scratch);
                let discard = status.discard;
                match status.state {
                    Err(_) => {
                        self.read_more(30_000)?;
                    }
                    Ok(state) => match state {
                        ConnectionState::WriteTraffic(mut wt) => {
                            let n = wt
                                .encrypt(&data[off..], &mut out_buf)
                                .map_err(|_| HttpError::Tls)?;
                            drop(wt);
                            self.outgoing_pending.extend_from_slice(&out_buf[..n]);
                            self.flush_pending()?;
                            off = data.len();
                        }
                        ConnectionState::EncodeTlsData(mut enc) => {
                            let n = enc.encode(&mut out_buf).map_err(|_| HttpError::Tls)?;
                            drop(enc);
                            self.outgoing_pending.extend_from_slice(&out_buf[..n]);
                            self.flush_pending()?;
                        }
                        ConnectionState::TransmitTlsData(tx) => {
                            tx.done();
                        }
                        ConnectionState::BlockedHandshake => {
                            self.read_more(30_000)?;
                        }
                        _ => {}
                    },
                }
                discard
            };
            if discard > 0 {
                self.incoming.drain(..discard);
            }
        }
        Ok(())
    }

    fn read_plain_to<S: BodySink>(
        &mut self,
        stream: &mut HttpStreamState,
        sink: &mut S,
        timeout_ms: u64,
    ) -> Result<(), HttpError> {
        let mut out_buf = [0u8; 8192];
        loop {
            let discard = {
                let mut scratch = self.incoming.as_mut_slice();
                let status = self.conn.process_tls_records(&mut scratch);
                let discard = status.discard;
                match status.state {
                    Err(_) => {
                        if !self.read_more(timeout_ms)? {
                            return Ok(());
                        }
                    }
                    Ok(state) => match state {
                        ConnectionState::ReadTraffic(mut rt) => {
                            while let Some(rec) = rt.next_record() {
                                let rec = rec.map_err(|_| HttpError::Tls)?;
                                stream.feed(rec.payload, sink)?;
                            }
                            drop(rt);
                        }
                        ConnectionState::PeerClosed | ConnectionState::Closed => return Ok(()),
                        ConnectionState::EncodeTlsData(mut enc) => {
                            let n = enc.encode(&mut out_buf).map_err(|_| HttpError::Tls)?;
                            drop(enc);
                            self.outgoing_pending.extend_from_slice(&out_buf[..n]);
                            self.flush_pending()?;
                        }
                        ConnectionState::TransmitTlsData(tx) => {
                            tx.done();
                        }
                        ConnectionState::BlockedHandshake => {
                            if !self.read_more(timeout_ms)? {
                                return Ok(());
                            }
                        }
                        ConnectionState::WriteTraffic(_) => {}
                        _ => {}
                    },
                }
                discard
            };
            if discard > 0 {
                self.incoming.drain(..discard);
            }
        }
    }
}

/// Destino del cuerpo HTTP: en RAM (respuestas pequeñas) o disco (descargas grandes).
pub trait BodySink {
    fn write_body(&mut self, chunk: &[u8]) -> Result<(), HttpError>;
}

struct VecSink<'a>(&'a mut Vec<u8>);

impl BodySink for VecSink<'_> {
    fn write_body(&mut self, chunk: &[u8]) -> Result<(), HttpError> {
        self.0.extend_from_slice(chunk);
        Ok(())
    }
}

const MAX_HTTP_HEADER: usize = 16 * 1024;

/// Acumula cabeceras HTTP y vuelca el cuerpo al sink en cuanto llega.
struct HttpStreamState {
    header_buf: Vec<u8>,
    headers_done: bool,
    discard_body: bool,
    status: Option<u16>,
    headers: Vec<(String, String)>,
}

impl HttpStreamState {
    fn new() -> Self {
        Self {
            header_buf: Vec::new(),
            headers_done: false,
            discard_body: false,
            status: None,
            headers: Vec::new(),
        }
    }

    fn feed<S: BodySink>(&mut self, chunk: &[u8], sink: &mut S) -> Result<(), HttpError> {
        if !self.headers_done {
            self.header_buf.extend_from_slice(chunk);
            if self.header_buf.len() > MAX_HTTP_HEADER {
                return Err(HttpError::Parse);
            }
            let Some(sep) = self
                .header_buf
                .windows(4)
                .position(|w| w == b"\r\n\r\n")
            else {
                return Ok(());
            };
            let (status, headers) = parse_response_head(&self.header_buf[..sep])?;
            self.status = Some(status);
            self.headers = headers;
            self.discard_body = (300..400).contains(&status);
            self.headers_done = true;
            let body = self.header_buf[sep + 4..].to_vec();
            self.header_buf.clear();
            if !self.discard_body && !body.is_empty() {
                sink.write_body(&body)?;
            }
            return Ok(());
        }
        if !self.discard_body {
            sink.write_body(chunk)?;
        }
        Ok(())
    }

    fn finish(self) -> Result<(u16, Vec<(String, String)>), HttpError> {
        if let Some(status) = self.status {
            return Ok((status, self.headers));
        }
        if !self.header_buf.is_empty() {
            let sep = self
                .header_buf
                .windows(4)
                .position(|w| w == b"\r\n\r\n")
                .ok_or(HttpError::Parse)?;
            let (status, headers) = parse_response_head(&self.header_buf[..sep])?;
            return Ok((status, headers));
        }
        Err(HttpError::Parse)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpError {
    Parse,
    Tls,
    Io,
    Dns,
}

/// GET HTTPS volcando el cuerpo a `sink` (p. ej. fichero). Sigue redirects (hasta 8).
pub fn https_download<T: TcpTransport, S: BodySink>(
    transport: &T,
    url: &str,
    auth: Option<&str>,
    sink: &mut S,
) -> Result<(u16, Vec<(String, String)>), HttpError> {
    let mut current = url.to_string();
    for _ in 0..8 {
        let (scheme, host, port, path) = parse_url(&current)?;
        if scheme != "https" {
            return Err(HttpError::Parse);
        }
        let mut ip = [0u8; 4];
        transport.dns_resolve(host, &mut ip).map_err(|_| HttpError::Dns)?;
        let fd = transport
            .tcp_connect(
                SockAddr {
                    addr: ip,
                    port,
                    _pad: 0,
                },
                30_000,
            )
            .map_err(|_| HttpError::Io)?;
        let mut tls = TlsSession::new(transport, fd, host)?;
        tls.handshake()?;
        let req = build_get(host, &path, auth);
        tls.write(req.as_bytes())?;
        let mut stream = HttpStreamState::new();
        tls.read_plain_to(&mut stream, sink, 60_000)?;
        transport.close(fd);
        let (status, headers) = stream.finish()?;
        if (300..400).contains(&status) {
            if let Some(loc) = header_value(&headers, "location") {
                current = resolve_redirect(&current, loc);
                continue;
            }
        }
        return Ok((status, headers));
    }
    Err(HttpError::Parse)
}

/// GET HTTPS con redirects (hasta 8). `auth` opcional: token Bearer HF.
pub fn https_get<T: TcpTransport>(
    transport: &T,
    url: &str,
    auth: Option<&str>,
) -> Result<Response, HttpError> {
    let mut body = Vec::new();
    let (status, _) = https_download(transport, url, auth, &mut VecSink(&mut body))?;
    Ok(Response { status, body })
}

fn build_get(host: &str, path: &str, auth: Option<&str>) -> String {
    let mut req = format!("GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n");
    if let Some(tok) = auth {
        let _ = write!(req, "Authorization: Bearer {tok}\r\n");
    }
    req.push_str("\r\n");
    req
}

fn parse_url(url: &str) -> Result<(&str, &str, u16, String), HttpError> {
    let rest = url.strip_prefix("https://").ok_or(HttpError::Parse)?;
    let (host_port, path) = match rest.split_once('/') {
        Some((h, p)) => (h, format!("/{p}")),
        None => (rest, String::from("/")),
    };
    if let Some((host, port)) = host_port.rsplit_once(':') {
        let port: u16 = port.parse().map_err(|_| HttpError::Parse)?;
        Ok(("https", host, port, path))
    } else {
        Ok(("https", host_port, 443, path))
    }
}

fn parse_response_head(head: &[u8]) -> Result<(u16, Vec<(String, String)>), HttpError> {
    let head = core::str::from_utf8(head).map_err(|_| HttpError::Parse)?;
    let mut lines = head.split("\r\n");
    let status_line = lines.next().ok_or(HttpError::Parse)?;
    let status = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .ok_or(HttpError::Parse)?;
    let mut headers = Vec::new();
    for line in lines {
        if let Some((k, v)) = line.split_once(':') {
            headers.push((k.trim().to_ascii_lowercase(), v.trim().to_string()));
        }
    }
    Ok((status, headers))
}

#[cfg(test)]
fn parse_response(raw: &[u8]) -> Result<(u16, Vec<(String, String)>, Vec<u8>), HttpError> {
    let sep = raw.windows(4).position(|w| w == b"\r\n\r\n").ok_or(HttpError::Parse)?;
    let (status, headers) = parse_response_head(&raw[..sep])?;
    let body = raw[sep + 4..].to_vec();
    Ok((status, headers, body))
}

fn header_value<'a>(headers: &'a [(String, String)], key: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
}

fn resolve_redirect(base: &str, loc: &str) -> String {
    if loc.starts_with("https://") {
        loc.to_string()
    } else if loc.starts_with('/') {
        let host = base
            .strip_prefix("https://")
            .and_then(|r| r.split('/').next())
            .unwrap_or("huggingface.co");
        format!("https://{host}{loc}")
    } else {
        loc.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockTransport;

    impl TcpTransport for MockTransport {
        fn dns_resolve(&self, _host: &str, out: &mut [u8; 4]) -> Result<(), i64> {
            out.copy_from_slice(&[127, 0, 0, 1]);
            Ok(())
        }

        fn tcp_connect(&self, _addr: SockAddr, _timeout_ms: u64) -> Result<u64, i64> {
            Ok(1)
        }

        fn read_timeout(&self, _fd: u64, _buf: &mut [u8], _timeout_ms: u64) -> i64 {
            0
        }

        fn write_all(&self, _fd: u64, _data: &[u8]) -> Result<(), i64> {
            Ok(())
        }

        fn close(&self, _fd: u64) {}
    }

    #[test]
    fn parse_url_simple() {
        let (_, host, port, path) = parse_url("https://huggingface.co/api/models/x/tree/main").unwrap();
        assert_eq!(host, "huggingface.co");
        assert_eq!(port, 443);
        assert!(path.starts_with("/api/"));
    }

    #[test]
    fn parse_response_ok() {
        let raw = b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello";
        let (st, _, body) = parse_response(raw).unwrap();
        assert_eq!(st, 200);
        assert_eq!(body, b"hello");
    }

    #[test]
    fn http_stream_state_chunks() {
        let mut stream = HttpStreamState::new();
        let mut out = Vec::new();
        stream.feed(b"HTTP/1.1 200 OK\r\n", &mut VecSink(&mut out)).unwrap();
        assert!(out.is_empty());
        stream
            .feed(b"Content-Length: 5\r\n\r\nhel", &mut VecSink(&mut out))
            .unwrap();
        assert_eq!(out, b"hel");
        stream.feed(b"lo", &mut VecSink(&mut out)).unwrap();
        assert_eq!(out, b"hello");
        let (st, _) = stream.finish().unwrap();
        assert_eq!(st, 200);
    }

    #[test]
    fn http_stream_discards_redirect_body() {
        let mut stream = HttpStreamState::new();
        let mut out = Vec::new();
        stream
            .feed(
                b"HTTP/1.1 302 Found\r\nLocation: https://x/y\r\n\r\nignored",
                &mut VecSink(&mut out),
            )
            .unwrap();
        stream.feed(b"more", &mut VecSink(&mut out)).unwrap();
        assert!(out.is_empty());
        let (st, headers) = stream.finish().unwrap();
        assert_eq!(st, 302);
        assert_eq!(header_value(&headers, "location"), Some("https://x/y"));
    }

    #[test]
    fn mock_transport_dns_and_connect() {
        let t = MockTransport;
        let mut ip = [0u8; 4];
        t.dns_resolve("example.com", &mut ip).unwrap();
        assert_eq!(ip, [127, 0, 0, 1]);
        assert_eq!(t.tcp_connect(SockAddr { addr: ip, port: 443, _pad: 0 }, 1000).unwrap(), 1);
    }
}
