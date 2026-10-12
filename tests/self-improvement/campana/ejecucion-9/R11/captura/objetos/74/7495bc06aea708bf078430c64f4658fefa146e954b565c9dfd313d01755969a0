//! Protocolo soso ↔ llama-server para inferencia CUDA en host (L6-H).

use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::Instant;

const BPS: u32 = 10_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InferRequest {
    pub model: String,
    pub max_new: u32,
    pub temp_bps: u32,
    pub top_p_bps: u32,
    pub seed: u64,
    pub prompt: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InferResponse {
    pub text: String,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InferError {
    BadHeader,
    BadPrompt,
    Upstream(String),
}

impl InferRequest {
    pub fn temp_f32(&self) -> f32 {
        self.temp_bps as f32 / BPS as f32
    }

    pub fn top_p_f32(&self) -> f32 {
        self.top_p_bps as f32 / BPS as f32
    }
}

pub fn f32_to_bps(v: f32) -> u32 {
    (v.clamp(0.0, 1.0) * BPS as f32).round() as u32
}

pub fn read_request<R: Read>(mut r: R) -> Result<InferRequest, InferError> {
    let mut line = Vec::new();
    read_line(&mut r, &mut line).map_err(|_| InferError::BadHeader)?;
    let line = String::from_utf8(line).map_err(|_| InferError::BadHeader)?;
    let parts: Vec<&str> = line.trim_end().split_whitespace().collect();
    if parts.len() != 7 || parts[0] != "INFER" {
        return Err(InferError::BadHeader);
    }
    let prompt_len: usize = parts[6].parse().map_err(|_| InferError::BadHeader)?;
    let mut prompt_buf = vec![0u8; prompt_len];
    r.read_exact(&mut prompt_buf)
        .map_err(|_| InferError::BadPrompt)?;
    Ok(InferRequest {
        model: parts[1].into(),
        max_new: parts[2].parse().map_err(|_| InferError::BadHeader)?,
        temp_bps: parts[3].parse().map_err(|_| InferError::BadHeader)?,
        top_p_bps: parts[4].parse().map_err(|_| InferError::BadHeader)?,
        seed: parts[5].parse().map_err(|_| InferError::BadHeader)?,
        prompt: String::from_utf8(prompt_buf).map_err(|_| InferError::BadPrompt)?,
    })
}

pub fn write_request<W: Write>(w: &mut W, req: &InferRequest) -> io::Result<()> {
    let header = format!(
        "INFER {} {} {} {} {} {}\n",
        req.model,
        req.max_new,
        req.temp_bps,
        req.top_p_bps,
        req.seed,
        req.prompt.len()
    );
    w.write_all(header.as_bytes())?;
    w.write_all(req.prompt.as_bytes())?;
    w.flush()
}

pub fn write_ok<W: Write>(w: &mut W, resp: &InferResponse) -> io::Result<()> {
    let header = format!("OK {} {}\n", resp.text.len(), resp.elapsed_ms);
    w.write_all(header.as_bytes())?;
    w.write_all(resp.text.as_bytes())?;
    w.flush()
}

pub fn write_err<W: Write>(w: &mut W, code: u32, msg: &str) -> io::Result<()> {
    let header = format!("ERR {} {}\n", code, msg.len());
    w.write_all(header.as_bytes())?;
    w.write_all(msg.as_bytes())?;
    w.flush()
}

pub fn read_response<R: Read>(mut r: R) -> Result<InferResponse, String> {
    let mut line = Vec::new();
    read_line(&mut r, &mut line).map_err(|e| e.to_string())?;
    let line = String::from_utf8(line).map_err(|e| e.to_string())?;
    if line.starts_with("ERR ") {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 3 {
            let msg_len: usize = parts[2].parse().unwrap_or(0);
            let mut msg = vec![0u8; msg_len];
            r.read_exact(&mut msg).ok();
            return Err(String::from_utf8_lossy(&msg).into_owned());
        }
        return Err("upstream error".into());
    }
    let parts: Vec<&str> = line.trim_end().split_whitespace().collect();
    if parts.len() != 3 || parts[0] != "OK" {
        return Err("bad response header".into());
    }
    let text_len: usize = parts[1].parse().map_err(|_| "bad text_len")?;
    let elapsed_ms: u64 = parts[2].parse().map_err(|_| "bad elapsed")?;
    let mut text_buf = vec![0u8; text_len];
    r.read_exact(&mut text_buf)
        .map_err(|e| e.to_string())?;
    Ok(InferResponse {
        text: String::from_utf8(text_buf).map_err(|e| e.to_string())?,
        elapsed_ms,
    })
}

fn read_line<R: Read>(mut r: R, buf: &mut Vec<u8>) -> io::Result<()> {
    buf.clear();
    loop {
        let mut byte = [0u8; 1];
        r.read_exact(&mut byte)?;
        buf.push(byte[0]);
        if byte[0] == b'\n' {
            return Ok(());
        }
        if buf.len() > 8192 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "line too long"));
        }
    }
}

/// POST /v1/completions hacia llama-server (OpenAI-compatible).
pub fn llama_completions(base_url: &str, req: &InferRequest) -> Result<String, String> {
    let url = format!(
        "{}/v1/completions",
        base_url.trim_end_matches('/')
    );
    let body = build_completions_json(req);
    let response = http_post_json(&url, &body)?;
    parse_completions_text(&response)
}

fn build_completions_json(req: &InferRequest) -> String {
    let prompt = json_escape(&req.prompt);
    let model = json_escape(&req.model);
    format!(
        r#"{{"model":"{model}","prompt":"{prompt}","max_tokens":{max},"temperature":{temp},"top_p":{top_p},"seed":{seed},"stream":false}}"#,
        model = model,
        prompt = prompt,
        max = req.max_new,
        temp = req.temp_f32(),
        top_p = req.top_p_f32(),
        seed = req.seed,
    )
}

fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

fn http_post_json(url: &str, body: &str) -> Result<String, String> {
    let (host, port, path) = parse_http_url(url)?;
    let mut stream = TcpStream::connect((host.as_str(), port))
        .map_err(|e| format!("connect: {e}"))?;
    let request = format!(
        "POST {path} HTTP/1.1\r\nHost: {host}:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len(),
        path = path,
        host = host,
        port = port,
        body = body,
    );
    stream
        .write_all(request.as_bytes())
        .map_err(|e| format!("write: {e}"))?;
    let _ = stream.shutdown(std::net::Shutdown::Write);
    let mut raw = Vec::new();
    stream
        .read_to_end(&mut raw)
        .map_err(|e| format!("read: {e}"))?;
    let raw = String::from_utf8_lossy(&raw);
    http_body(&raw).ok_or_else(|| "empty http body".into())
}

fn parse_http_url(url: &str) -> Result<(String, u16, String), String> {
    let rest = url
        .strip_prefix("http://")
        .ok_or_else(|| "only http:// supported".to_string())?;
    let (authority, path) = match rest.split_once('/') {
        Some((a, p)) => (a, format!("/{p}")),
        None => (rest, "/".into()),
    };
    let (host, port) = match authority.split_once(':') {
        Some((h, p)) => (h.to_string(), p.parse().map_err(|_| "bad port")?),
        None => (authority.to_string(), 80),
    };
    Ok((host, port, path))
}

fn http_body(raw: &str) -> Option<String> {
    let (_, body) = raw.split_once("\r\n\r\n")?;
    Some(body.to_string())
}

fn parse_completions_text(json: &str) -> Result<String, String> {
    let key = "\"text\":\"";
    let start = json.find(key).ok_or("missing text field")? + key.len();
    let tail = &json[start..];
    let mut out = String::new();
    let mut chars = tail.chars();
    while let Some(ch) = chars.next() {
        if ch == '"' {
            break;
        }
        if ch == '\\' {
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('r') => out.push('\r'),
                Some('t') => out.push('\t'),
                Some('"') => out.push('"'),
                Some('\\') => out.push('\\'),
                Some(other) => out.push(other),
                None => break,
            }
        } else {
            out.push(ch);
        }
    }
    Ok(out)
}

pub fn handle_connection(
    mut stream: TcpStream,
    llama_base: &str,
) -> io::Result<()> {
    let req = match read_request(&mut stream) {
        Ok(r) => r,
        Err(InferError::BadHeader) => {
            let _ = write_err(&mut stream, 1, "bad INFER header");
            return Ok(());
        }
        Err(InferError::BadPrompt) => {
            let _ = write_err(&mut stream, 2, "bad prompt");
            return Ok(());
        }
        Err(InferError::Upstream(msg)) => {
            let _ = write_err(&mut stream, 3, &msg);
            return Ok(());
        }
    };
    let t0 = Instant::now();
    match llama_completions(llama_base, &req) {
        Ok(text) => {
            let resp = InferResponse {
                text,
                elapsed_ms: t0.elapsed().as_millis() as u64,
            };
            write_ok(&mut stream, &resp)?;
        }
        Err(e) => {
            write_err(&mut stream, 3, &e)?;
        }
    }
    Ok(())
}

pub fn serve(listen: &str, llama_base: &str) -> io::Result<()> {
    let listener = TcpListener::bind(listen)?;
    eprintln!("cuda-proxy: escuchando en {listen} → llama {llama_base}");
    for conn in listener.incoming() {
        match conn {
            Ok(stream) => {
                if let Err(e) = handle_connection(stream, llama_base) {
                    eprintln!("cuda-proxy: conexión: {e}");
                }
            }
            Err(e) => eprintln!("cuda-proxy: accept: {e}"),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    #[test]
    fn roundtrip_request_response() {
        let req = InferRequest {
            model: "tiny".into(),
            max_new: 8,
            temp_bps: 7000,
            top_p_bps: 9000,
            seed: 42,
            prompt: "hola".into(),
        };
        let mut buf = Vec::new();
        write_request(&mut buf, &req).unwrap();
        let parsed = read_request(buf.as_slice()).unwrap();
        assert_eq!(parsed, req);

        let resp = InferResponse {
            text: " mundo".into(),
            elapsed_ms: 123,
        };
        let mut out = Vec::new();
        write_ok(&mut out, &resp).unwrap();
        let got = read_response(out.as_slice()).unwrap();
        assert_eq!(got, resp);
    }

    #[test]
    fn llama_completions_uses_mock_http() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut req = Vec::new();
            stream.read_to_end(&mut req).unwrap();
            let req = String::from_utf8_lossy(&req);
            assert!(req.contains("POST /v1/completions"));
            let body = r#"{"choices":[{"text":" cuda"}]}"#;
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(resp.as_bytes()).unwrap();
        });

        let req = InferRequest {
            model: "m".into(),
            max_new: 4,
            temp_bps: 0,
            top_p_bps: 9000,
            seed: 1,
            prompt: "hi".into(),
        };
        let text = llama_completions(&format!("http://127.0.0.1:{port}"), &req).unwrap();
        assert_eq!(text, " cuda");
        handle.join().unwrap();
    }

    #[test]
    fn json_escape_controls() {
        let req = InferRequest {
            model: "x".into(),
            max_new: 1,
            temp_bps: 0,
            top_p_bps: 9000,
            seed: 0,
            prompt: "a\"b\n".into(),
        };
        let j = build_completions_json(&req);
        assert!(j.contains(r#"prompt":"a\"b\n""#));
    }
}
