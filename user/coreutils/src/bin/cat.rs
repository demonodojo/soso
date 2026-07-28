//! `cat <fichero>...` vuelca ficheros a la salida; `cat -` vuelca **stdin**, que es
//! lo que hace falta para que los pipelines de sosh sirvan de algo (los soportaba
//! desde siempre, pero ningún programa leía stdin).
//!
//! Por qué `-` y no "sin argumentos, como en Unix": en la tty de soso no hay EOF
//! —nadie interpreta Ctrl-D—, así que un `cat` sin argumentos leyendo la consola se
//! quedaría colgado para siempre en vez de decir cómo se usa. En un pipeline sí hay
//! EOF de verdad: la lectura del pipe devuelve 0 cuando el escritor cierra.

#![no_std]
#![no_main]

use libsoso::{abi, errno_str, println, sys};

libsoso::entry!(main);

/// Vuelca `fd` a la salida. Devuelve 1 si algo falló.
fn volcar(fd: u64, quien: &str) -> u8 {
    let mut buf = [0u8; 1024];
    loop {
        let n = sys::read(fd, &mut buf);
        if n <= 0 {
            if n < 0 {
                println!("cat: {quien}: {}", errno_str(n));
                return 1;
            }
            return 0;
        }
        // `write_all` y no `write`: una escritura puede ser CORTA —el pipe de un
        // pipeline acepta lo que le quepa y lo dice en el retorno— y aquí se
        // escriben trozos de 1 KiB. Con `write` a secas se perdía la cola de cada
        // trozo sin ningún error.
        if sys::write_all(1, &buf[..n as usize]).is_err() {
            return 1;
        }
    }
}

fn main(args: &str) -> u8 {
    let mut alguno = false;
    let mut fallo = 0u8;
    for path in args.split_whitespace() {
        alguno = true;
        if path == "-" {
            fallo |= volcar(0, "-");
            continue;
        }
        let fd = sys::open(path, abi::O_RDONLY);
        if fd < 0 {
            println!("cat: {path}: {}", errno_str(fd));
            fallo = 1;
            continue;
        }
        fallo |= volcar(fd as u64, path);
        sys::close(fd as u64);
    }
    if !alguno {
        println!("uso: cat <fichero>...  |  cat - (lee stdin, para pipelines)");
        return 2;
    }
    fallo
}
