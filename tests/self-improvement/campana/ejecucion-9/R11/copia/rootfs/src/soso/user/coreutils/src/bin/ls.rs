//! ls [ruta]: lista un directorio (o muestra un fichero suelto).

#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use libsoso::{abi, errno_str, println, sys};

libsoso::entry!(main);

fn main(args: &str) -> u8 {
    let path = match args.trim() {
        "" => "/",
        p => p,
    };
    let mut st = abi::Stat::default();
    let r = sys::stat(path, &mut st);
    if r < 0 {
        println!("ls: {path}: {}", errno_str(r));
        return 1;
    }
    if st.file_type != abi::FT_DIR {
        println!("- {:>8}  {path}", st.size);
        return 0;
    }

    let fd = sys::open(path, abi::O_RDONLY);
    if fd < 0 {
        println!("ls: {path}: {}", errno_str(fd));
        return 1;
    }
    let mut ents = [abi::Dirent::default(); 16];
    loop {
        let n = sys::getdents(fd as u64, &mut ents);
        if n < 0 {
            println!("ls: {path}: {}", errno_str(n));
            return 1;
        }
        if n == 0 {
            break;
        }
        for d in &ents[..n as usize / abi::DIRENT_SIZE] {
            let name = core::str::from_utf8(d.name_bytes()).unwrap_or("?");
            let ruta = if path == "/" { format!("/{name}") } else { format!("{path}/{name}") };
            let mut st = abi::Stat::default();
            let (tipo, size) = if sys::stat(&ruta, &mut st) == 0 {
                (if st.file_type == abi::FT_DIR { 'd' } else { '-' }, st.size)
            } else {
                ('?', 0)
            };
            println!("{tipo} {size:>8}  {name}");
        }
    }
    sys::close(fd as u64);
    0
}
