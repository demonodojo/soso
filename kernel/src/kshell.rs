//! Kernel-shell por el puerto serie. Es la consola de emergencia y el
//! banco de pruebas hasta que exista la shell de usuario (fase 7).

use crate::drivers::virtio_blk;
use crate::{drivers::serial, print, println, qemu};
use alloc::string::String;
use alloc::vec::Vec;

pub fn run() -> ! {
    println!("kernel-shell lista; escribe 'help'");
    let mut line = String::new();
    print!("soso> ");
    loop {
        let Some(byte) = serial::read_byte() else {
            crate::net::poll();
            x86_64::instructions::hlt();
            continue;
        };
        match byte {
            b'\r' | b'\n' => {
                println!();
                exec(line.trim());
                line.clear();
                print!("soso> ");
            }
            0x08 | 0x7f => {
                if line.pop().is_some() {
                    print!("\x08 \x08");
                }
            }
            0x20..=0x7e => {
                line.push(byte as char);
                print!("{}", byte as char);
            }
            _ => {}
        }
    }
}

fn exec(line: &str) {
    let mut partes = line.split_whitespace();
    let Some(cmd) = partes.next() else { return };
    let args: Vec<&str> = partes.collect();

    match cmd {
        "help" => {
            println!("comandos: help spawn <elf> [args] ps ls cat stat write <ruta> <texto> mkdir <ruta> rm <ruta> df uptime mem io blk blkread blkwrite pf panic halt");
        }
        "spawn" => match args.first() {
            Some(ruta) => {
                let argumentos = args[1..].join(" ");
                match crate::task::spawn(ruta, &argumentos, 0) {
                    Ok(pid) => {
                        println!("pid {pid}");
                        crate::task::schedule(); // no vuelve hasta vaciarse la tabla
                    }
                    Err(e) => println!("spawn: errno {e}"),
                }
            }
            None => println!("uso: spawn <elf> [args]"),
        },
        "ps" => {
            for p in crate::task::PROCS.lock().iter() {
                println!("{:>3}  padre {:>3}  {:?}  {}", p.pid, p.parent, p.state, p.name);
            }
        }
        "write" => match args.first() {
            Some(ruta) => {
                let texto = args[1..].join(" ");
                match split_path(ruta) {
                    Some((padre, nombre)) => with_vfs(|| {
                        let dir = crate::vfs::resolve(padre)?;
                        let mtime = crate::arch::pit::uptime_ms() / 1000;
                        crate::vfs::create_file(dir, nombre, texto.as_bytes(), mtime)?;
                        println!("{} bytes -> {ruta}", texto.len());
                        Ok(())
                    }),
                    None => println!("ruta inválida"),
                }
            }
            None => println!("uso: write <ruta> <texto>"),
        },
        "mkdir" => match args.first().and_then(|r| split_path(r)) {
            Some((padre, nombre)) => with_vfs(|| {
                let dir = crate::vfs::resolve(padre)?;
                crate::vfs::mkdir(dir, nombre, crate::arch::pit::uptime_ms() / 1000)?;
                Ok(())
            }),
            None => println!("uso: mkdir <ruta>"),
        },
        "rm" => match args.first().and_then(|r| split_path(r)) {
            Some((padre, nombre)) => with_vfs(|| {
                let dir = crate::vfs::resolve(padre)?;
                crate::vfs::unlink(dir, nombre)
            }),
            None => println!("uso: rm <ruta>"),
        },
        "df" => {
            if let Some(fs) = crate::fs::FS.get() {
                let fs = fs.lock();
                let libres = fs.free_blocks();
                println!(
                    "{}/{} bloques libres ({} MiB), generación {}",
                    libres,
                    fs.block_count(),
                    libres * 4096 / (1024 * 1024),
                    fs.generation()
                );
            } else {
                println!("fs: no montado");
            }
        }
        "ls" => {
            let ruta = args.first().copied().unwrap_or("/");
            with_vfs(|| {
                let ino = crate::vfs::resolve(ruta)?;
                for (nombre, child) in crate::vfs::read_dir(ino)? {
                    let st = crate::vfs::stat_inode(child)?;
                    let tipo = if st.file_type == sosofs::layout::FT_DIR { "d" } else { "-" };
                    println!("{tipo} {:>8}  {nombre}", st.size.get());
                }
                Ok(())
            });
        }
        "cat" => match args.first() {
            Some(ruta) => with_vfs(|| {
                let ino = crate::vfs::resolve(ruta)?;
                let data = crate::vfs::read_file(ino)?;
                print!("{}", alloc::string::String::from_utf8_lossy(&data));
                Ok(())
            }),
            None => println!("uso: cat <ruta>"),
        },
        "stat" => match args.first() {
            Some(ruta) => with_vfs(|| {
                let ino = crate::vfs::resolve(ruta)?;
                let st = crate::vfs::stat_inode(ino)?;
                let tipo = if st.file_type == sosofs::layout::FT_DIR { "directorio" } else { "fichero" };
                println!("inode {ino}: {tipo}, {} bytes, mtime {}", st.size.get(), st.mtime.get());
                Ok(())
            }),
            None => println!("uso: stat <ruta>"),
        },
        "uptime" => {
            let ms = crate::arch::pit::uptime_ms();
            println!("{}.{:02} s ({} ticks)", ms / 1000, (ms % 1000) / 10, crate::arch::pit::ticks());
        }
        "mem" => {
            let libres = crate::mm::FRAME_ALLOC.get().unwrap().lock().free_frames();
            println!("{} frames libres ({} MiB)", libres, libres * 4096 / (1024 * 1024));
        }
        "io" => {
            if args.first() == Some(&"reset") {
                crate::drivers::blkstat::reiniciar();
                println!("contadores de disco a cero");
            } else {
                let (peticiones, bloques, nanos, escrituras) = crate::drivers::blkstat::leer();
                let us_por_peticion = if peticiones > 0 { nanos / peticiones / 1000 } else { 0 };
                println!(
                    "disco: {peticiones} lecturas, {bloques} bloques, {} ms ({us_por_peticion} us/lectura), {escrituras} escrituras",
                    nanos / 1_000_000
                );
                if let Some(m) = crate::fs::MODELS.get().and_then(|m| m.try_lock()) {
                    let (aciertos, fallos) = m.cache.estadisticas();
                    let total = aciertos + fallos;
                    let pct = if total > 0 { aciertos * 100 / total } else { 0 };
                    println!("caché sosomfs: {aciertos} aciertos, {fallos} fallos ({pct} %)");
                }
            }
        }
        "blk" => match virtio_blk::capacity_sectors() {
            Some(cap) => println!("{} sectores ({} MiB)", cap, cap * 512 / (1024 * 1024)),
            None => println!("no hay disco"),
        },
        "blkread" => match args.first().and_then(|s| s.parse().ok()) {
            Some(sector) => {
                let mut buf = [0u8; 512];
                match virtio_blk::read_sector(sector, &mut buf) {
                    Ok(()) => hexdump(sector * 512, &buf[..64]),
                    Err(e) => println!("blkread: {e}"),
                }
            }
            None => println!("uso: blkread <sector>"),
        },
        "blkwrite" => match args.first().and_then(|s| s.parse::<u64>().ok()) {
            Some(sector) => {
                let texto = args[1..].join(" ");
                let mut buf = [0u8; 512];
                let n = texto.len().min(512);
                buf[..n].copy_from_slice(&texto.as_bytes()[..n]);
                match virtio_blk::write_sector(sector, &buf) {
                    Ok(()) => println!("{n} bytes escritos en el sector {sector}"),
                    Err(e) => println!("blkwrite: {e}"),
                }
            }
            None => println!("uso: blkwrite <sector> <texto>"),
        },
        "pf" => {
            // Demostración del diagnóstico de page fault.
            unsafe { core::ptr::read_volatile(0xdead_beef as *const u8) };
        }
        "panic" => {
            panic!("panic solicitado desde la shell");
        }
        "halt" => {
            println!("apagando");
            qemu::exit(qemu::ExitCode::Success);
        }
        otro => {
            println!("¿{otro}? escribe 'help'");
        }
    }
}

/// Separa una ruta en (directorio padre, nombre): "/a/b/c" -> ("/a/b", "c").
fn split_path(ruta: &str) -> Option<(&str, &str)> {
    let ruta = ruta.trim_end_matches('/');
    let i = ruta.rfind('/')?;
    let nombre = &ruta[i + 1..];
    if nombre.is_empty() {
        return None;
    }
    Some((if i == 0 { "/" } else { &ruta[..i] }, nombre))
}

/// Ejecuta una operación VFS, imprimiendo el error si lo hay.
fn with_vfs(f: impl FnOnce() -> Result<(), sosofs::FsError>) {
    if let Err(e) = f() {
        println!("fs: {e:?}");
    }
}

fn hexdump(base: u64, datos: &[u8]) {
    for fila in datos.chunks(16) {
        print!("{:08x} ", base + (fila.as_ptr() as usize - datos.as_ptr() as usize) as u64);
        for b in fila {
            print!(" {b:02x}");
        }
        print!("  |");
        for b in fila {
            let c = if b.is_ascii_graphic() || *b == b' ' { *b as char } else { '.' };
            print!("{c}");
        }
        println!("|");
    }
}
