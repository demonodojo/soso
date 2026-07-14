//! cat <fichero>...: vuelca ficheros a la salida.

#![no_std]
#![no_main]

use libsoso::{abi, errno_str, println, sys};

libsoso::entry!(main);

fn main(args: &str) -> u8 {
    let mut alguno = false;
    let mut fallo = 0u8;
    for path in args.split_whitespace() {
        alguno = true;
        let fd = sys::open(path, abi::O_RDONLY);
        if fd < 0 {
            println!("cat: {path}: {}", errno_str(fd));
            fallo = 1;
            continue;
        }
        let mut buf = [0u8; 1024];
        loop {
            let n = sys::read(fd as u64, &mut buf);
            if n <= 0 {
                if n < 0 {
                    println!("cat: {path}: {}", errno_str(n));
                    fallo = 1;
                }
                break;
            }
            sys::write(1, &buf[..n as usize]);
        }
        sys::close(fd as u64);
    }
    if !alguno {
        println!("uso: cat <fichero>...");
        return 2;
    }
    fallo
}
