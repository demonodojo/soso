//! Kernel-shell por el puerto serie. Es la consola de emergencia y el
//! banco de pruebas hasta que exista la shell de usuario (fase 7).

use crate::{drivers::serial, print, println, qemu};
use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, Ordering};

/// Evita spamear el banner si `schedule` reentra al kshell (p. ej. tras `spawn`).
static BANNER_SHOWN: AtomicBool = AtomicBool::new(false);

pub fn run() -> ! {
    if !BANNER_SHOWN.swap(true, Ordering::Relaxed) {
        println!("kernel-shell lista; escribe 'help' (dmesg = log de arranque)");
    }
    let mut line = String::new();
    print!("soso> ");
    loop {
        // Polling activo: PS/2 y USB HID no dependen solo de IRQ (placa real).
        let Some(byte) = serial::read_byte() else {
            crate::net::poll();
            #[cfg(feature = "drv-live-disk")]
            crate::drivers::fatlog::poll();
            let _ = crate::drivers::kbd::has_input();
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
            println!("comandos: help dmesg [patrón|save] hwscan kbd spawn ps ls cat stat write mkdir rm df uptime mem io wifi blk blkread blkwrite pf panic halt");
        }
        "dmesg" => match args.first() {
            Some(&"save") => {
                #[cfg(feature = "drv-live-disk")]
                match crate::drivers::fatlog::flush() {
                    Ok(()) => println!("dmesg: volcado a SOSOLOG.TXT"),
                    Err(()) => println!("dmesg: fatlog no activo o fallo de escritura"),
                }
                #[cfg(not(feature = "drv-live-disk"))]
                println!("dmesg: fatlog no disponible en esta imagen");
            }
            Some(pat) => dmesg_grep(pat),
            None => dmesg_paged(),
        },
        "hwscan" => {
            crate::drivers::registry::print_hwscan();
            #[cfg(feature = "drv-live-disk")]
            match crate::drivers::drvlog::flush() {
                Ok(()) => println!("hwscan: informe en SOSODRV.TXT"),
                Err(()) => println!("hwscan: sin ESP live o fallo de escritura"),
            }
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
                        let mtime = crate::time::wall_secs();
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
        "kbd" => {
            if let Some(name) = args.first() {
                if crate::drivers::kbd::set_layout(name) {
                    println!("kbd: layout {}", crate::drivers::kbd::layout_name());
                } else {
                    println!("kbd: layout desconocido (es|us)");
                }
                return;
            }
            println!("kbd: layout {}", crate::drivers::kbd::layout_name());
            let (tot, ultimo, enc, ent) = crate::drivers::kbd::latido();
            println!(
                "kbd: {tot} scancodes (último {ultimo:#04x}), {enc} encolados, {ent} entregados"
            );
            let mut sc = [0u8; 8];
            let n = crate::drivers::kbd::scancodes_iniciales(&mut sc);
            if n == 0 {
                println!("kbd: ningún scancode todavía");
            } else {
                print!("kbd: primeros scancodes:");
                for b in &sc[..n] {
                    print!(" {b:#04x}");
                }
                println!();
            }
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
        "wifi" => {
            #[cfg(feature = "lxdde")]
            {
                match args.first() {
                    Some(&"scan") => {
                        if !crate::lxdde::wifi_present() {
                            println!("wifi: no hay adaptador");
                        } else if !crate::lxdde::wifi_alive() {
                            println!(
                                "wifi: firmware no arrancó (phase={})",
                                crate::lxdde::wifi_phase()
                            );
                        } else {
                            crate::lxdde::wifi_scan();
                            let mut n = 0u32;
                            for (ssid, rssi, ch, open) in crate::lxdde::wifi_scan_results() {
                                let sec = if open { "abierta" } else { "WPA" };
                                println!("  {ssid}: {rssi} dBm, canal {ch}, {sec}");
                                n += 1;
                            }
                            if n == 0 {
                                println!("wifi: ninguna red");
                            }
                        }
                    }
                    Some(&"status") => {
                        println!(
                            "wifi: alive={} connected={} phase={}",
                            crate::lxdde::wifi_alive(),
                            crate::lxdde::wifi_connected(),
                            crate::lxdde::wifi_phase()
                        );
                        if let Some(mac) = crate::lxdde::wifi_mac() {
                            println!(
                                "  mac {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
                                mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
                            );
                        }
                    }
                    Some(&"connect") => match args.get(1) {
                        Some(ssid) => {
                            let rc = if args.len() > 2 {
                                let pass = args[2..].join(" ");
                                crate::net::wifi_wpa::connect_wpa2(ssid, &pass)
                            } else {
                                crate::lxdde::wifi::connect_open(ssid)
                            };
                            if rc == 0 {
                                crate::net::on_wifi_connected();
                            }
                            println!("wifi connect: rc={rc}");
                        }
                        None => println!("uso: wifi connect <ssid> [psk]"),
                    }
                    _ => println!("uso: wifi scan | status | connect <ssid> [psk]"),
                }
            }
            #[cfg(not(feature = "lxdde"))]
            println!("wifi: requiere feature lxdde");
        }
        #[cfg(feature = "drv-virtio-blk")]
        "blk" => match crate::drivers::virtio_blk::capacity_sectors() {
            Some(cap) => println!("{} sectores ({} MiB)", cap, cap * 512 / (1024 * 1024)),
            None => println!("no hay disco"),
        },
        #[cfg(not(feature = "drv-virtio-blk"))]
        "blk" => println!("blk: driver virtio-blk no compilado"),
        #[cfg(feature = "drv-virtio-blk")]
        "blkread" => match args.first().and_then(|s| s.parse().ok()) {
            Some(sector) => {
                let mut buf = [0u8; 512];
                match crate::drivers::virtio_blk::read_sector(sector, &mut buf) {
                    Ok(()) => hexdump(sector * 512, &buf[..64]),
                    Err(e) => println!("blkread: {e}"),
                }
            }
            None => println!("uso: blkread <sector>"),
        },
        #[cfg(feature = "drv-virtio-blk")]
        "blkwrite" => match args.first().and_then(|s| s.parse::<u64>().ok()) {
            Some(sector) => {
                let texto = args[1..].join(" ");
                let mut buf = [0u8; 512];
                let n = texto.len().min(512);
                buf[..n].copy_from_slice(&texto.as_bytes()[..n]);
                match crate::drivers::virtio_blk::write_sector(sector, &buf) {
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

/// Vuelca el ring buffer de consola por páginas (~30 líneas) para que no se
/// pierda otra vez por el scroll del framebuffer.
fn dmesg_paged() {
    let total = crate::drivers::logbuf::len();
    println!("--- dmesg ({total} bytes; espacio/enter = más, q = salir) ---");
    let mut offset = 0usize;
    let mut page_lines = 0usize;
    let mut tmp = [0u8; 256];
    const PAGE_LINES: usize = 30;

    while offset < total {
        let n = crate::drivers::logbuf::copy_from(offset, &mut tmp);
        if n == 0 {
            break;
        }
        let mut take = n;
        let mut lines = page_lines;
        for (i, &b) in tmp[..n].iter().enumerate() {
            if b == b'\n' {
                lines += 1;
                if lines >= PAGE_LINES {
                    take = i + 1;
                    break;
                }
            }
        }
        serial::write_bytes_raw(&tmp[..take]);
        page_lines = lines;
        offset += take;

        if offset >= total {
            break;
        }
        if page_lines >= PAGE_LINES {
            page_lines = 0;
            serial::write_bytes_raw(b"--more--");
            loop {
                let Some(byte) = serial::read_byte() else {
                    crate::net::poll();
                    x86_64::instructions::hlt();
                    continue;
                };
                match byte {
                    b'q' | b'Q' => {
                        serial::write_bytes_raw(b"\n");
                        println!("--- dmesg abortado ---");
                        return;
                    }
                    b' ' | b'\r' | b'\n' => {
                        serial::write_bytes_raw(b"\r        \r");
                        break;
                    }
                    _ => {}
                }
            }
        }
    }
    println!("--- fin dmesg ---");
}

/// Espera en `--more--`. Devuelve false si el usuario aborta con 'q'.
fn esperar_pagina() -> bool {
    serial::write_bytes_raw(b"--more--");
    loop {
        let Some(byte) = serial::read_byte() else {
            crate::net::poll();
            x86_64::instructions::hlt();
            continue;
        };
        match byte {
            b'q' | b'Q' => {
                serial::write_bytes_raw(b"\n");
                return false;
            }
            b' ' | b'\r' | b'\n' => {
                serial::write_bytes_raw(b"\r        \r");
                return true;
            }
            _ => {}
        }
    }
}

/// `dmesg <patrón>`: sólo las líneas que contienen la subcadena. En placa real
/// la pantalla son ~40 filas y el log de arranque pasa de 100: sin filtro no
/// hay forma de leer las líneas de un subsistema concreto.
fn dmesg_grep(pat: &str) {
    let total = crate::drivers::logbuf::len();
    println!("--- dmesg «{pat}» (espacio/enter = más, q = salir) ---");

    // Una línea de log cabe de sobra aquí; las más largas se parten.
    let mut linea = [0u8; 320];
    let mut largo = 0usize;
    let mut tmp = [0u8; 256];
    let mut offset = 0usize;
    let mut page_lines = 0usize;
    let mut encontradas = 0usize;
    const PAGE_LINES: usize = 30;

    while offset < total {
        let n = crate::drivers::logbuf::copy_from(offset, &mut tmp);
        if n == 0 {
            break;
        }
        offset += n;

        for &b in &tmp[..n] {
            if b != b'\n' && largo < linea.len() {
                linea[largo] = b;
                largo += 1;
                continue;
            }
            if b != b'\n' {
                continue; // línea desbordada: se descarta el resto
            }
            if let Ok(s) = core::str::from_utf8(&linea[..largo]) {
                if s.contains(pat) {
                    encontradas += 1;
                    println!("{s}");
                    page_lines += 1;
                    if page_lines >= PAGE_LINES {
                        page_lines = 0;
                        if !esperar_pagina() {
                            println!("--- dmesg abortado ---");
                            return;
                        }
                    }
                }
            }
            largo = 0;
        }
    }

    println!("--- fin dmesg: {encontradas} líneas con «{pat}» ---");
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
