//! Paridad opcional con un servidor Kev de referencia (`KEV_REF_URL`).

#![cfg(feature = "std")]

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

#[test]
fn systemone_paridad_opcional_kev_ref() {
    let Some(base) = std::env::var("KEV_REF_URL").ok() else {
        eprintln!("KEV_REF_URL no definida — omitiendo paridad Kev");
        return;
    };
    let body = r#"{
      "state": "Shoes arrived two weeks late and in the wrong size.",
      "questions": {
        "department": {
          "type": "choice",
          "instructions": "Which team should handle this?",
          "criteria": {"returns": "Exchanges", "shipping": "Delays"}
        }
      }
    }"#;
    let ref_json = post_json(&format!("{base}/v1/systemone"), body).expect("referencia");
    let soso_json = post_json("http://127.0.0.1:8009/v1/systemone", body);
    let soso_json = match soso_json {
        Ok(j) => j,
        Err(e) => {
            eprintln!("servidor soso local no disponible ({e}) — sólo comprobada referencia");
            assert!(ref_json.contains("answers"));
            return;
        }
    };
    let ref_v: serde_json::Value = serde_json::from_str(&ref_json).unwrap();
    let sos_v: serde_json::Value = serde_json::from_str(&soso_json).unwrap();
    let ref_probs = ref_v["answers"]["department"]["probabilities"].as_object().unwrap();
    let sos_probs = sos_v["answers"]["department"]["probabilities"].as_object().unwrap();
    for (k, rv) in ref_probs {
        let sv = sos_probs.get(k).and_then(|v| v.as_f64()).unwrap_or(0.0);
        let r = rv.as_f64().unwrap_or(0.0);
        assert!(
            (sv - r).abs() < 0.05,
            "prob {k}: soso {sv} vs ref {r}"
        );
    }
}

fn post_json(url: &str, body: &str) -> Result<String, String> {
    let (host, path) = parse_url(url)?;
    let mut stream = TcpStream::connect((host.as_str(), 80))
        .or_else(|_| TcpStream::connect((host.as_str(), 8009)))
        .or_else(|_| TcpStream::connect((host.as_str(), 8080)))
        .map_err(|e| e.to_string())?;
    stream.set_read_timeout(Some(Duration::from_secs(120))).ok();
    let req = format!(
        "POST {path} HTTP/1.1\r\nHost: {host}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(req.as_bytes()).map_err(|e| e.to_string())?;
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).map_err(|e| e.to_string())?;
    let text = String::from_utf8_lossy(&buf);
    let body_start = text.find("\r\n\r\n").map(|i| i + 4).unwrap_or(0);
    Ok(text[body_start..].to_string())
}

fn parse_url(url: &str) -> Result<(String, String), String> {
    let url = url.strip_prefix("http://").ok_or("solo http")?;
    let (host_port, path) = url.split_once('/').map(|(h, p)| (h, format!("/{p}"))).unwrap_or((url, "/v1/systemone".into()));
    let host = host_port.split(':').next().unwrap_or(host_port).to_string();
    Ok((host, path))
}
