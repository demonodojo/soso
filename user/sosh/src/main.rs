//! sosh: la shell de soso. Sin pipes ni redirecciones ni variables (v1):
//! una línea = un comando de /bin con sus argumentos, más los builtins
//! `exit` y `help`.
//!
//! La tty del kernel es cruda: el eco y el backspace los hace la shell.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use libsoso::{errno_str, print, println, sys};

libsoso::entry!(main);

const PROMPT: &str = "$ ";

fn main(_args: &str) -> u8 {
    println!("sosh — escribe 'help' para la ayuda");
    let mut line = [0u8; 256];
    let mut len = 0usize;
    print!("{PROMPT}");
    loop {
        let mut buf = [0u8; 64];
        let n = sys::read(0, &mut buf);
        if n <= 0 {
            continue;
        }
        for &c in &buf[..n as usize] {
            match c {
                b'\r' | b'\n' => {
                    println!();
                    let cmd = core::str::from_utf8(&line[..len]).unwrap_or("");
                    if let Some(code) = ejecutar(cmd) {
                        return code;
                    }
                    len = 0;
                    print!("{PROMPT}");
                }
                0x08 | 0x7f => {
                    if len > 0 {
                        len -= 1;
                        print!("\x08 \x08");
                    }
                }
                0x20..=0x7e if len < line.len() => {
                    line[len] = c;
                    len += 1;
                    sys::write(1, &[c]);
                }
                _ => {}
            }
        }
    }
}

/// Ejecuta una línea. Some(código) = salir de la shell.
fn ejecutar(line: &str) -> Option<u8> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    let (cmd, args) = match line.split_once(char::is_whitespace) {
        Some((c, a)) => (c, a.trim()),
        None => (line, ""),
    };
    match cmd {
        "exit" => return Some(args.parse().unwrap_or(0)),
        "help" => {
            println!("builtins: exit [código], help");
            println!("comandos: cualquier ELF de /bin (ls cat echo mkdir rm hexdump...)");
            println!("          o una ruta absoluta (/bin/init test)");
        }
        _ => {
            let path =
                if cmd.starts_with('/') { format!("{cmd}") } else { format!("/bin/{cmd}") };
            let pid = sys::spawn(&path, args);
            if pid < 0 {
                println!("sosh: {cmd}: {}", errno_str(pid));
                return None;
            }
            match sys::wait() {
                Ok((_, 0)) => {}
                Ok((_, code)) => println!("sosh: [{cmd} salió con código {code}]"),
                Err(e) => println!("sosh: wait: {}", errno_str(e)),
            }
        }
    }
    None
}
