//! Descubrimiento de nodos del cluster por sondeo TCP del /24.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use libsoso::{sys, thread};
use soso_abi as abi;
use soso_red_core::parsear_red_conf;

use crate::net::{parse_sock_addr, TcpFd};

const SONDEO_MS: u64 = 40;
const HILOS: usize = 16;

struct ScanJob {
    base: [u8; 4],
    puerto: u16,
    host_lo: u8,
    host_hi: u8,
    done: core::sync::atomic::AtomicBool,
    found: Vec<String>,
}

extern "C" fn scan_worker(arg: u64) -> ! {
    let job = unsafe { &mut *(arg as *mut ScanJob) };
    for host in job.host_lo..=job.host_hi {
        if host == 0 || host == job.base[3] {
            continue;
        }
        let addr_s = format!(
            "{}.{}.{}.{}:{}",
            job.base[0], job.base[1], job.base[2], host, job.puerto
        );
        let Some(sock) = parse_sock_addr(&addr_s) else {
            continue;
        };
        if TcpFd::connect(&sock, SONDEO_MS).is_ok() {
            job.found.push(addr_s);
        }
    }
    job.done.store(true, core::sync::atomic::Ordering::Release);
    loop {}
}

/// Devuelve `ip:puerto` de hosts que aceptan TCP en `puerto` dentro del /24 local.
pub fn descubrir_nodos(puerto: u16) -> Vec<String> {
    let base = leer_red_local().unwrap_or([10, 0, 2, 15]);
    let mut jobs = Vec::with_capacity(HILOS);
    for t in 0..HILOS {
        let lo = (t * 254 / HILOS) as u8 + 1;
        let hi = ((t + 1) * 254 / HILOS) as u8;
        jobs.push(ScanJob {
            base,
            puerto,
            host_lo: lo,
            host_hi: hi,
            done: core::sync::atomic::AtomicBool::new(false),
            found: Vec::new(),
        });
    }
    let mut handles = Vec::new();
    for job in jobs.iter_mut() {
        let ptr = job as *mut ScanJob as u64;
        if let Ok(h) = thread::spawn(scan_worker, ptr) {
            handles.push(h);
        }
    }
    for job in jobs.iter() {
        while !job.done.load(core::sync::atomic::Ordering::Acquire) {
            sys::sleep_ms(1);
        }
    }
    let _ = handles;
    let mut out = Vec::new();
    for job in jobs {
        out.extend(job.found);
    }
    out.sort();
    out.dedup();
    out
}

fn leer_red_local() -> Option<[u8; 4]> {
    let mut ni = abi::NetInfo::default();
    if sys::netinfo(&mut ni) >= 0 && ni.flags & abi::NET_FLAG_CONFIGURED != 0 {
        return Some(ni.addr);
    }
    let data = crate::ask::leer_fichero("/etc/red.conf")?;
    match parsear_red_conf(&data).ok()? {
        Some(red) => Some(red.addr),
        None => None,
    }
}
