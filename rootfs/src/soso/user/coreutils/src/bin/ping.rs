//! `ping [-c N] [-W seg] destino` — ICMP Echo Request.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::string::String;
use libsoso::{errno_str, println, sys};

libsoso::entry!(main);

fn parse_u32(s: &str) -> Option<u32> {
    if s.is_empty() {
        return None;
    }
    let mut v = 0u32;
    for b in s.bytes() {
        if !b.is_ascii_digit() {
            return None;
        }
        v = v.saturating_mul(10).saturating_add((b - b'0') as u32);
    }
    Some(v)
}

fn parse_ipv4(s: &str) -> Option<[u8; 4]> {
    let mut out = [0u8; 4];
    let mut n = 0usize;
    for (i, part) in s.split('.').enumerate() {
        if i >= 4 || part.is_empty() {
            return None;
        }
        let v = parse_u32(part)?;
        if v > 255 {
            return None;
        }
        out[i] = v as u8;
        n = i + 1;
    }
    if n == 4 {
        Some(out)
    } else {
        None
    }
}

fn usage() {
    println!("uso: ping [-c N] [-W seg] destino");
}

fn main(args: &[String]) -> u8 {
    let mut count = 4u32;
    let mut timeout_s = 1u64;
    let mut dest: Option<&str> = None;
    let mut it = args.iter().map(|s| s.as_str());
    while let Some(tok) = it.next() {
        match tok {
            "-c" => match it.next().and_then(parse_u32) {
                Some(n) if n > 0 => count = n.min(64),
                _ => {
                    usage();
                    return 1;
                }
            },
            "-W" => match it.next().and_then(parse_u32) {
                Some(n) if n > 0 => timeout_s = n as u64,
                _ => {
                    usage();
                    return 1;
                }
            },
            "-h" | "--help" => {
                usage();
                return 0;
            }
            t if t.starts_with('-') => {
                println!("ping: opción desconocida {t}");
                usage();
                return 1;
            }
            t => dest = Some(t),
        }
    }
    let Some(host) = dest else {
        usage();
        return 1;
    };

    let addr = if let Some(ip) = parse_ipv4(host) {
        ip
    } else {
        let mut ip = [0u8; 4];
        if let Err(e) = sys::dns_resolve(host, &mut ip) {
            println!("ping: {host}: {}", errno_str(e));
            return 1;
        }
        ip
    };

    println!(
        "PING {host} ({}.{}.{}.{})",
        addr[0], addr[1], addr[2], addr[3]
    );

    let timeout_ms = timeout_s.saturating_mul(1000);
    let mut ok = 0u32;
    for seq in 1..=count {
        let r = sys::ping(addr, timeout_ms);
        if r >= 0 {
            println!(
                "ping: {}.{}.{}.{} seq={seq} {r} ms",
                addr[0], addr[1], addr[2], addr[3]
            );
            ok += 1;
        } else if r == -libsoso::abi::ETIMEDOUT {
            println!(
                "ping: {}.{}.{}.{} seq={seq} timeout",
                addr[0], addr[1], addr[2], addr[3]
            );
        } else {
            println!("ping: {}", errno_str(r));
            return 1;
        }
        if seq < count {
            sys::sleep_ms(1000);
        }
    }
    println!("--- {host} ---");
    println!("{count} enviados, {ok} recibidos");
    if ok == count {
        0
    } else {
        1
    }
}
