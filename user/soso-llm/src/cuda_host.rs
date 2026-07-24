//! Cliente L6-H: inferencia CUDA en host Linux vía cuda-proxy.

use crate::net::{parse_sock_addr, TcpFd};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use libsoso::{println, sys};
use soso_abi;

const CONNECT_TIMEOUT_MS: u64 = 30_000;
const READ_CHUNK_MS: u64 = 5_000;

pub struct CudaHostConfig {
    pub host: String,
    pub model: String,
    pub prompt: String,
    pub max_new: u32,
    pub temp_bps: u32,
    pub top_p_bps: u32,
    pub seed: u64,
}

fn f32_to_bps(v: f32) -> u32 {
    let clamped = if v < 0.0 {
        0.0
    } else if v > 1.0 {
        1.0
    } else {
        v
    };
    ((clamped * 10_000.0) + 0.5) as u32
}

pub fn run(cfg: &CudaHostConfig) -> u8 {
    let addr = match parse_sock_addr(&cfg.host) {
        Some(a) => a,
        None => {
            println!("soso-llm: --cuda-host inválido ({})", cfg.host);
            return 1;
        }
    };
    println!(
        "soso-llm: CUDA host {} (modelo {})",
        cfg.host, cfg.model
    );
    let mut stream = match TcpFd::connect(&addr, CONNECT_TIMEOUT_MS) {
        Ok(s) => s,
        Err(e) => {
            println!("soso-llm: tcp_connect a {} falló ({e})", cfg.host);
            return 1;
        }
    };
    let header = format!(
        "INFER {} {} {} {} {} {}\n",
        cfg.model,
        cfg.max_new,
        cfg.temp_bps,
        cfg.top_p_bps,
        cfg.seed,
        cfg.prompt.len()
    );
    if sys::write_all(stream.fd, header.as_bytes()).is_err() {
        println!("soso-llm: envío cabecera CUDA falló");
        return 1;
    }
    if sys::write_all(stream.fd, cfg.prompt.as_bytes()).is_err() {
        println!("soso-llm: envío prompt CUDA falló");
        return 1;
    }

    let mut line = Vec::new();
    if read_line(&mut stream, &mut line).is_err() {
        println!("soso-llm: respuesta CUDA incompleta");
        return 1;
    }
    let line = match core::str::from_utf8(&line) {
        Ok(s) => s.trim_end(),
        Err(_) => {
            println!("soso-llm: respuesta CUDA no UTF-8");
            return 1;
        }
    };
    if let Some(rest) = line.strip_prefix("ERR ") {
        let parts: alloc::vec::Vec<&str> = rest.split_whitespace().collect();
        if parts.len() >= 2 {
            if let Ok(msg_len) = parts[1].parse::<usize>() {
                let mut msg = alloc::vec![0u8; msg_len];
                if read_exact(&mut stream, &mut msg).is_ok() {
                    if let Ok(text) = core::str::from_utf8(&msg) {
                        println!("soso-llm: CUDA host error: {text}");
                        return 1;
                    }
                }
            }
        }
        println!("soso-llm: CUDA host error");
        return 1;
    }
    let parts: alloc::vec::Vec<&str> = line.split_whitespace().collect();
    if parts.len() != 3 || parts[0] != "OK" {
        println!("soso-llm: cabecera CUDA inválida");
        return 1;
    }
    let text_len: usize = match parts[1].parse() {
        Ok(n) => n,
        Err(_) => {
            println!("soso-llm: text_len CUDA inválido");
            return 1;
        }
    };
    let elapsed_ms: u64 = match parts[2].parse() {
        Ok(n) => n,
        Err(_) => {
            println!("soso-llm: elapsed CUDA inválido");
            return 1;
        }
    };
    let mut text = alloc::vec![0u8; text_len];
    if read_exact(&mut stream, &mut text).is_err() {
        println!("soso-llm: cuerpo CUDA incompleto");
        return 1;
    }
    let out = match core::str::from_utf8(&text) {
        Ok(s) => s,
        Err(_) => {
            println!("soso-llm: texto CUDA no UTF-8");
            return 1;
        }
    };
    libsoso::print!("{out}");
    println!();
    let tok_est = out.split_whitespace().count().max(1);
    let tok_s = tok_est as f64 * 1000.0 / elapsed_ms.max(1) as f64;
    println!(
        "soso-llm: CUDA host (~{} tokens, {} ms host, {:.2} tok/s)",
        tok_est, elapsed_ms, tok_s
    );
    0
}

pub fn config_from_run(
    cuda_host: &str,
    model: &str,
    prompt: &str,
    max_new: usize,
    temp: f32,
    top_p: f32,
    seed: u64,
) -> CudaHostConfig {
    CudaHostConfig {
        host: String::from(cuda_host),
        model: String::from(model),
        prompt: String::from(prompt),
        max_new: max_new as u32,
        temp_bps: f32_to_bps(temp),
        top_p_bps: f32_to_bps(top_p),
        seed,
    }
}

fn read_line(stream: &mut TcpFd, buf: &mut Vec<u8>) -> Result<(), ()> {
    buf.clear();
    loop {
        let mut byte = [0u8; 1];
        let n = sys::read_timeout(stream.fd, &mut byte, READ_CHUNK_MS);
        if n == -(soso_abi::EAGAIN as i64) {
            continue;
        }
        if n <= 0 {
            return Err(());
        }
        buf.push(byte[0]);
        if byte[0] == b'\n' {
            return Ok(());
        }
        if buf.len() > 8192 {
            return Err(());
        }
    }
}

fn read_exact(stream: &mut TcpFd, buf: &mut [u8]) -> Result<(), ()> {
    let mut off = 0usize;
    while off < buf.len() {
        let n = sys::read_timeout(stream.fd, &mut buf[off..], READ_CHUNK_MS);
        if n == -(soso_abi::EAGAIN as i64) {
            continue;
        }
        if n <= 0 {
            return Err(());
        }
        off += n as usize;
    }
    Ok(())
}
