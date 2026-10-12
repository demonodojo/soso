//! `hexdump <fichero>`: volcado hexadecimal + ASCII. Con `-` lee **stdin**, que es
//! lo que hace útil `algo | hexdump -` (ver la nota de `cat` sobre por qué `-` y no
//! "sin argumentos": en la tty de soso no hay EOF).

#![no_std]
#![no_main]

use libsoso::{abi, errno_str, print, println, sys};

libsoso::entry!(main);

fn main(args: &str) -> u8 {
    let path = args.trim();
    if path.is_empty() {
        println!("uso: hexdump <fichero>  |  hexdump - (lee stdin)");
        return 2;
    }
    let desde_stdin = path == "-";
    let fd = if desde_stdin {
        0
    } else {
        let fd = sys::open(path, abi::O_RDONLY);
        if fd < 0 {
            println!("hexdump: {path}: {}", errno_str(fd));
            return 1;
        }
        fd as u64
    };
    let mut off = 0usize;
    let mut buf = [0u8; 1024];
    loop {
        let n = sys::read(fd, &mut buf);
        if n <= 0 {
            break;
        }
        for fila in buf[..n as usize].chunks(16) {
            print!("{off:08x} ");
            for b in fila {
                print!(" {b:02x}");
            }
            for _ in fila.len()..16 {
                print!("   ");
            }
            print!("  |");
            for &b in fila {
                let c = if b.is_ascii_graphic() || b == b' ' { b as char } else { '.' };
                print!("{c}");
            }
            println!("|");
            off += fila.len();
        }
    }
    if !desde_stdin {
        sys::close(fd);
    }
    0
}
