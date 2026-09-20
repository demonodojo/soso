//! `ps` — lista procesos del sistema.

#![no_std]
#![no_main]

use libsoso::{abi, errno_str, println, sys};

libsoso::entry!(main);

fn estado_texto(s: u8) -> &'static str {
    match s {
        abi::PROC_STATE_RUNNABLE => "listo",
        abi::PROC_STATE_RUNNING => "corriendo",
        abi::PROC_STATE_SLEEPING => "dormido",
        abi::PROC_STATE_WAIT_CHILD => "hijo",
        abi::PROC_STATE_WAIT_TTY => "tty",
        abi::PROC_STATE_WAIT_PIPE => "pipe",
        abi::PROC_STATE_WAIT_FUTEX => "futex",
        abi::PROC_STATE_WAIT_SOCKET => "socket",
        abi::PROC_STATE_ZOMBIE => "zombi",
        _ => "?",
    }
}

fn nombre_cstr(name: &[u8; 48]) -> &str {
    let end = name.iter().position(|&b| b == 0).unwrap_or(name.len());
    core::str::from_utf8(&name[..end]).unwrap_or("?")
}

fn main(_args: &str) -> u8 {
    let mut buf = [abi::ProcInfo::default(); 128];
    let n = sys::pslist(&mut buf);
    if n < 0 {
        println!("ps: {}", errno_str(n));
        return 1;
    }
    let n = n as usize;
    println!("  PID  PADRE  ESTADO     COMANDO");
    for p in &buf[..n] {
        let cmd = nombre_cstr(&p.name);
        if p.flags & abi::PROC_FLAG_THREAD != 0 {
            println!(
                " {:>4} {:>6} {:>10}  {cmd} (hilo)",
                p.pid,
                p.ppid,
                estado_texto(p.state),
            );
        } else {
            println!(
                " {:>4} {:>6} {:>10}  {cmd}",
                p.pid,
                p.ppid,
                estado_texto(p.state),
            );
        }
    }
    0
}
