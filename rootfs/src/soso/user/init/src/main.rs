//! PID 1 de soso: lanza la shell y la relanza si muere mal. Con args
//! "test" ejecuta la suite de la fase 6 (las 14 syscalls, OK/FALLO por
//! bloque); "hijo"/"cpu N"/"crash" son los modos auxiliares de esa suite.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;
use libsoso::abi;
use libsoso::{print, println, sys};
use soso_abi::UPD_WHICH_MAILBOX;
use soso_update_core::mailbox::{Mailbox, MailboxCmd};

libsoso::entry!(main);

macro_rules! check {
    ($cond:expr, $($msg:tt)*) => {
        if $cond {
            println!("init: OK  {}", format_args!($($msg)*));
        } else {
            println!("init: FALLO  {}", format_args!($($msg)*));
            return 1;
        }
    };
}

fn main(args: &str) -> u8 {
    if args.is_empty() {
        return lanzar_shell();
    }
    if args == "test" {
        return suite();
    }
    // Modos de hijo (el propio init se re-spawnea para las pruebas).
    if args == "hijo" {
        println!("hijo: hola, me voy con código 7");
        return 7;
    }
    if let Some(rest) = args.strip_prefix("sleep ") {
        let ms = rest
            .trim()
            .bytes()
            .fold(0u64, |acc, b| acc.saturating_mul(10).saturating_add((b - b'0') as u64));
        let _ = sys::sleep_ms(ms);
        return 0;
    }
    if args == "crash" {
        // Para probar que una falta de usuario mata al proceso, no al kernel.
        unsafe { core::ptr::read_volatile(core::ptr::null::<u8>()) };
        return 0;
    }
    if let Some(n) = args.strip_prefix("cpu ") {
        // Trabajo de CPU puro, sin syscalls entre iteraciones: si esto se
        // intercala con el otro hijo, la preempción por timer funciona.
        for i in 1..=3 {
            busy();
            println!("cpu {n}: iteración {i}");
        }
        return 0;
    }
    if let Some(s) = args.strip_prefix("fpu ") {
        // Estrés de preservación FPU/SSE: mantiene un patrón en YMM durante
        // muchos desalojos de timer y verifica que no se corrompe.
        let seed = s.bytes().next().unwrap_or(b'1');
        return fpu_stress(seed);
    }
    println!("init: args desconocidos {args:?} (usa: test)");
    2
}

/// Bucle cpu-bound que mantiene un patrón en ymm3 y lo compara contra
/// memoria en cada iteración, todo en UN bloque asm (el compilador no puede
/// tocar esos registros por medio). El timer desaloja este bucle decenas de
/// veces; si el kernel no preserva XMM/YMM, la comparación falla.
/// MXCSR de este proceso (`stmxcsr`).
fn leer_mxcsr() -> u32 {
    let mut v: u32 = 0;
    unsafe {
        core::arch::asm!("stmxcsr [{p}]", p = in(reg) &mut v, options(nostack));
    }
    v
}

fn fpu_stress(seed: u8) -> u8 {
    let pat = [seed as f32 * 1.5 + 0.25; 8];
    let mut fallos: u64;
    unsafe {
        core::arch::asm!(
            "vmovups ymm3, [{pat}]",
            "xor {fallos}, {fallos}",
            "2:",
            "vpcmpeqd ymm4, ymm3, [{pat}]",
            "vpmovmskb {t:e}, ymm4",
            "cmp {t:e}, -1",
            "je 3f",
            "inc {fallos}",
            "3:",
            "dec {i}",
            "jnz 2b",
            pat = in(reg) pat.as_ptr(),
            fallos = out(reg) fallos,
            i = inout(reg) 4_000_000u64 => _,
            t = out(reg) _,
            out("ymm3") _,
            out("ymm4") _,
        );
    }
    if fallos == 0 {
        0
    } else {
        println!("fpu {}: {fallos} corrupciones de YMM", seed as char);
        1
    }
}

/// Si el shim dejó el buzón en PROBANDO, confirma que el kernel arrancó.
fn confirmar_actualizacion() -> bool {
    let mut buf = [0u8; 4096];
    let n = sys::upd_read(UPD_WHICH_MAILBOX, 0, &mut buf);
    if n <= 0 {
        return true;
    }
    let text = core::str::from_utf8(&buf[..n as usize]).unwrap_or("");
    let mb = Mailbox::parse(text);
    if let MailboxCmd::Probando { version } = mb.cmd {
        let payload = Mailbox::format_ok(&version);
        let r = sys::upd_write(UPD_WHICH_MAILBOX, 0, &payload);
        return r >= 0;
    }
    true
}

fn rootfs_accesible() -> bool {
    let mut st = abi::Stat::default();
    sys::stat("/etc/soso-release", &mut st) >= 0
}

/// Bucle de PID 1: sosh en marcha siempre. Si la shell sale limpia
/// (exit 0), init termina y el kernel vuelve a su shell de emergencia.
fn lanzar_shell() -> u8 {
    if !rootfs_accesible() {
        println!("init: rootfs no accesible (/etc/soso-release)");
        return 1;
    }
    let pid = sys::spawn("/bin/sosh", "");
    if pid < 0 {
        println!("init: no puedo lanzar /bin/sosh (errno {pid})");
        return 1;
    }
    if !confirmar_actualizacion() {
        println!("init: no pude confirmar actualización en buzón");
        return 1;
    }
    loop {
        match sys::wait() {
            Ok((_, 0)) => {
                println!("init: shell cerrada; adiós");
                return 0;
            }
            Ok((_, code)) => {
                println!("init: sosh murió con código {code}; relanzando");
                let pid = sys::spawn("/bin/sosh", "");
                if pid < 0 {
                    println!("init: no puedo relanzar /bin/sosh (errno {pid})");
                    return 1;
                }
            }
            Err(e) => {
                println!("init: wait falló (errno {e})");
                return 1;
            }
        }
    }
}

fn suite() -> u8 {
    println!("init: hola desde ring 3");

    // stat + open + read + seek + close sobre un fichero de la imagen.
    let mut st = abi::Stat::default();
    check!(sys::stat("/etc/motd", &mut st) == 0 && st.file_type == abi::FT_FILE,
        "stat /etc/motd ({} bytes)", st.size);

    let fd = sys::open("/etc/motd", abi::O_RDONLY);
    check!(fd >= 0, "open /etc/motd (fd {fd})");
    let fd = fd as u64;
    let mut buf = [0u8; 512];
    let n = sys::read(fd, &mut buf);
    check!(n as u64 == st.size, "read devuelve el fichero entero ({n} bytes)");
    print!("--- motd ---\n{}------------\n", core::str::from_utf8(&buf[..n as usize]).unwrap_or("?"));
    check!(sys::seek(fd, 0, abi::SEEK_SET) == 0, "seek al principio");
    let n2 = sys::read(fd, &mut buf[..4]);
    check!(n2 == 4, "releer tras seek");
    check!(sys::close(fd) == 0, "close");

    // getdents de /.
    let fd = sys::open("/", abi::O_RDONLY);
    check!(fd >= 0, "open /");
    let mut entradas = [abi::Dirent::default(); 16];
    let n = sys::getdents(fd as u64, &mut entradas);
    check!(n > 0, "getdents / ({} entradas)", n as usize / abi::DIRENT_SIZE);
    print!("init: / contiene:");
    for d in entradas.iter().take(n as usize / abi::DIRENT_SIZE) {
        print!(" {}", core::str::from_utf8(d.name_bytes()).unwrap_or("?"));
    }
    println!();
    sys::close(fd as u64);

    // mkdir + escritura (open O_WRONLY + write + close) + relectura + unlink.
    let r = sys::mkdir("/tmp");
    check!(r == 0 || r == -abi::EEXIST, "mkdir /tmp");
    let fd = sys::open("/tmp/init.txt", abi::O_WRONLY);
    check!(fd >= 0, "open /tmp/init.txt para escribir");
    let texto = "escrito desde ring 3\n";
    check!(sys::write(fd as u64, texto.as_bytes()) == texto.len() as i64, "write");
    check!(sys::close(fd as u64) == 0, "close publica el fichero");
    let fd = sys::open("/tmp/init.txt", abi::O_RDONLY);
    let n = sys::read(fd as u64, &mut buf);
    sys::close(fd as u64);
    check!(n == texto.len() as i64 && &buf[..n as usize] == texto.as_bytes(),
        "relectura idéntica");
    check!(sys::unlink("/tmp/init.txt") == 0, "unlink");
    let mut st2 = abi::Stat::default();
    check!(sys::stat("/tmp/init.txt", &mut st2) == -abi::ENOENT, "el fichero ya no está");

    // sbrk: pedir 2 páginas, escribirlas enteras y comprobar el puntero.
    let base = sys::sbrk(8192);
    check!(base > 0, "sbrk(+8192) (base {base:#x})");
    unsafe {
        core::ptr::write_bytes(base as *mut u8, 0xAB, 8192);
        check!(*((base + 8191) as *const u8) == 0xAB, "la memoria de sbrk es usable");
    }
    check!(sys::sbrk(0) == base + 8192, "sbrk(0) refleja el brk nuevo");

    // mmap de fichero grande (lazy open + demand paging).
    let mut st_big = abi::Stat::default();
    if sys::stat("/etc/motd", &mut st_big) == 0 {
        let fd = sys::open("/etc/motd", abi::O_RDONLY);
        if fd >= 0 {
            let map = sys::mmap(0, st_big.size, fd as u64, 0);
            check!(map > 0, "mmap /etc/motd -> {map:#x}");
            let b = unsafe { core::ptr::read_volatile(map as *const u8) };
            check!(b == b'#' || b > 0, "primer byte mmap legible ({b})");
            check!(sys::munmap(map as u64, st_big.size.next_multiple_of(4096) as u64) == 0, "munmap");
            sys::close(fd as u64);
        }
    }

    // El heap de alloc (sbrk) también funciona.
    let v: Vec<u64> = (0..10_000).collect();
    let s = String::from("heap ok: ") + itoa(v.iter().sum::<u64>());
    check!(v.len() == 10_000, "{s}");

    // spawn + wait con código de salida.
    let pid = sys::spawn("/bin/init", "hijo");
    check!(pid > 0, "spawn hijo (pid {pid})");
    let r = sys::wait();
    check!(r == Ok((pid as u64, 7)), "wait devuelve (pid {pid}, código 7)");

    // Señales: getpid, kill(SIGINT) y grupos de procesos.
    let me = sys::getpid();
    check!(me > 0, "getpid ({me})");
    let sid = sys::setsid();
    check!(sid == me as i64, "setsid devuelve el pid ({sid})");

    let sleeper = sys::spawn("/bin/init", "sleep 5000");
    check!(sleeper > 0, "spawn sleeper (pid {sleeper})");
    check!(sys::kill(sleeper, abi::SIGINT) > 0, "kill SIGINT al sleeper");
    check!(
        sys::wait() == Ok((sleeper as u64, abi::exit_by_signal(abi::SIGINT as u8))),
        "wait sleeper interrumpido (130)"
    );

    check!(sys::kill(1, abi::SIGINT) > 0, "SIGINT a PID 1 se ignora sin error");

    let g1 = sys::spawn("/bin/init", "sleep 30000");
    let g2 = sys::spawn("/bin/init", "sleep 30000");
    check!(g1 > 0 && g2 > 0, "spawn par para kill de grupo");
    check!(sys::setpgid(g1 as u64, g1 as u64) == 0, "setpgid líder");
    check!(sys::setpgid(g2 as u64, g1 as u64) == 0, "setpgid segundo al grupo");
    check!(sys::kill(-(g1 as i64), abi::SIGINT) >= 2, "kill(-pgid) al par");
    let mut interrumpidos = 0u8;
    for _ in 0..2 {
        if let Ok((_, code)) = sys::wait() {
            if code == abi::exit_by_signal(abi::SIGINT as u8) {
                interrumpidos += 1;
            }
        }
    }
    check!(interrumpidos == 2, "wait de ambos del grupo (130)");

    // Dos hijos de CPU pura: su salida intercalada demuestra la preempción.
    let p1 = sys::spawn("/bin/init", "cpu A");
    let p2 = sys::spawn("/bin/init", "cpu B");
    check!(p1 > 0 && p2 > 0, "spawn 2 hijos de cpu");
    check!(sys::wait().is_ok() && sys::wait().is_ok(), "wait de ambos");
    check!(sys::wait() == Err(-abi::ECHILD), "wait sin hijos da ECHILD");

    // Preservación FPU/SSE: dos hijos cpu-bound con patrones YMM distintos
    // que se desalojan mutuamente; si el kernel no guarda/restaura, se ven
    // el patrón del otro (o los clobbers de la cripto de net::poll).
    let f1 = sys::spawn("/bin/init", "fpu 3");
    let f2 = sys::spawn("/bin/init", "fpu 5");
    check!(f1 > 0 && f2 > 0, "spawn 2 hijos fpu");
    let ra = sys::wait();
    let rb = sys::wait();
    let fpu_ok = matches!(ra, Ok((_, 0))) && matches!(rb, Ok((_, 0)));
    check!(fpu_ok, "los YMM sobreviven a los desalojos");

    // MXCSR del proceso recién nacido: las seis excepciones SIMD ENMASCARADAS.
    //
    // Con MXCSR=0 cualquier operación inexacta levanta #XM, y en soso eso era
    // un double fault sin diagnóstico. Pasó en placa real: `"0.7".parse::<f32>()`
    // compila a un `vdivss` en `dec2flt` y mataba a `ask` al leer /etc/llm.conf.
    // QEMU no entrega #XM, así que el único modo de cazarlo en el banco es
    // mirar el registro. `xrstor` carga MXCSR de la imagen SIEMPRE, así que un
    // área xsave a ceros lo deja a 0 — de ahí venía.
    let mxcsr = leer_mxcsr();
    check!(
        mxcsr & 0x1F80 == 0x1F80,
        "MXCSR con las excepciones SIMD enmascaradas"
    );
    if mxcsr & 0x1F80 != 0x1F80 {
        println!("init:   MXCSR={mxcsr:#06x} (esperaba los bits 7..12 a 1)");
    }
    // Y que una división inexacta de verdad no reviente.
    let inexacta: f32 = "0.7".parse().unwrap_or(0.0);
    check!(inexacta > 0.69 && inexacta < 0.71, "parse::<f32> de 0.7 (dec2flt/vdivss)");

    // sleep_ms.
    check!(sys::sleep_ms(200) == 0, "sleep_ms(200)");

    // pipe: escribir en un extremo y leer en el otro.
    let (r, w) = match sys::pipe() {
        Ok(p) => p,
        Err(e) => {
            println!("init: FALLO  pipe() errno {e}");
            return 1;
        }
    };
    check!(sys::write(w, b"abc") == 3, "pipe write");
    let mut pbuf = [0u8; 8];
    let pn = sys::read(r, &mut pbuf);
    check!(pn == 3 && &pbuf[..3] == b"abc", "pipe read ({pn} bytes)");
    sys::close(w);
    sys::close(r);

    // O_APPEND: segunda escritura concatena.
    let fd = sys::open("/tmp/append.txt", abi::O_WRONLY);
    check!(fd >= 0, "open append 1");
    check!(sys::write(fd as u64, b"uno") == 3, "append write 1");
    check!(sys::close(fd as u64) == 0, "close append 1");
    let fd = sys::open("/tmp/append.txt", abi::O_WRONLY | abi::O_APPEND);
    check!(fd >= 0, "open append 2");
    check!(sys::write(fd as u64, b"dos") == 3, "append write 2");
    check!(sys::close(fd as u64) == 0, "close append 2");
    let fd = sys::open("/tmp/append.txt", abi::O_RDONLY);
    let n = sys::read(fd as u64, &mut buf);
    sys::close(fd as u64);
    check!(
        n == 6 && &buf[..6] == b"unodos",
        "O_APPEND concatena (leído {:?})",
        core::str::from_utf8(&buf[..n as usize]).unwrap_or("?")
    );
    sys::unlink("/tmp/append.txt");

    // spawn_io: redirige stdout de echo a un fichero.
    let fd = sys::open("/tmp/spawn_io.txt", abi::O_WRONLY);
    check!(fd >= 0, "open para spawn_io");
    let pid = sys::spawn_io(
        "/bin/echo",
        "spawn_io_ok",
        abi::FD_INHERIT_TTY,
        fd as u64,
        abi::FD_INHERIT_TTY,
    );
    check!(pid > 0, "spawn_io echo (pid {pid})");
    check!(sys::wait().is_ok(), "wait spawn_io");
    let fd = sys::open("/tmp/spawn_io.txt", abi::O_RDONLY);
    let n = sys::read(fd as u64, &mut buf);
    sys::close(fd as u64);
    check!(
        n > 0 && core::str::from_utf8(&buf[..n as usize]).unwrap_or("").contains("spawn_io_ok"),
        "spawn_io escribió al fichero"
    );
    sys::unlink("/tmp/spawn_io.txt");

    // spawn_io a consola serie: `FD_SERIAL_TTY` no es un fd del padre. Si el
    // kernel lo trata como índice, el spawn devuelve EBADF y este check falla.
    let pid = sys::spawn_io(
        "/bin/echo",
        "serial_ok",
        abi::FD_SERIAL_TTY,
        abi::FD_SERIAL_TTY,
        abi::FD_SERIAL_TTY,
    );
    check!(pid > 0, "spawn_io FD_SERIAL_TTY (pid {pid})");
    check!(sys::wait().is_ok(), "wait spawn_io serial");

    // Hilos + futex: N workers incrementan un contador compartido.
    {
        use core::sync::atomic::{AtomicU32, Ordering};
        use libsoso::thread;

        static COUNTER: AtomicU32 = AtomicU32::new(0);
        static DONE: AtomicU32 = AtomicU32::new(0);
        const N: u32 = 4;
        const PER: u32 = 1000;

        static ALLOC_MAL: AtomicU32 = AtomicU32::new(0);

        extern "C" fn thr_entry(arg: u64) -> ! {
            for _ in 0..PER {
                COUNTER.fetch_add(1, Ordering::Relaxed);
            }
            // Asignar y liberar DESDE VARIOS HILOS a la vez. El allocator sirve las
            // reservas pequeñas de un arena propio en userspace, y esa memoria la
            // comparten los hilos: si su candado estuviera mal, dos hilos se
            // repartirían el mismo trozo y cada uno vería los bytes del otro. Sin
            // esto, el arena no tenía ninguna prueba concurrente — antes la
            // exclusión la daba el kernel de rebote, porque cada reserva era una
            // syscall `sbrk`.
            let marca = (arg as u8).wrapping_add(1);
            for i in 0..200usize {
                let n = 8 + (i % 96);
                let mut v: Vec<u8> = Vec::new();
                for _ in 0..n {
                    v.push(marca);
                }
                if v.len() != n || v.iter().any(|&b| b != marca) {
                    ALLOC_MAL.fetch_add(1, Ordering::Relaxed);
                }
                // Y una cadena, que crece con realloc (el camino de "agrandar en el
                // sitio" del arena).
                let mut s = String::new();
                for _ in 0..n {
                    s.push('x');
                }
                if s.len() != n {
                    ALLOC_MAL.fetch_add(1, Ordering::Relaxed);
                }
            }
            DONE.fetch_add(1, Ordering::Release);
            let _ = sys::futex_wake(&DONE as *const AtomicU32 as *const u32, u64::MAX);
            sys::exit(0);
        }

        COUNTER.store(0, Ordering::Relaxed);
        DONE.store(0, Ordering::Relaxed);
        let mut handles = alloc::vec::Vec::new();
        let mut spawn_err: i64 = 0;
        for i in 0..N as usize {
            match thread::spawn(thr_entry, i as u64) {
                Ok(h) => handles.push(h),
                Err(e) => {
                    spawn_err = e;
                    break;
                }
            }
        }
        check!(spawn_err == 0, "thread_spawn ×{N} (errno {spawn_err})");
        while DONE.load(Ordering::Acquire) < N {
            let d = DONE.load(Ordering::Acquire);
            let _ = sys::futex_wait(&DONE as *const AtomicU32 as *const u32, d);
        }
        for h in handles {
            let _ = h.join();
        }
        check!(
            COUNTER.load(Ordering::Relaxed) == N * PER,
            "hilos: contador={} (esperado {})",
            COUNTER.load(Ordering::Relaxed),
            N * PER
        );
        check!(sys::ncpu() >= 1, "ncpu={}", sys::ncpu());
        {
            let mut mi = abi::MemInfo::default();
            check!(sys::meminfo(&mut mi) == 0, "meminfo errno");
            check!(mi.total_frames > 0, "total_frames={}", mi.total_frames);
            check!(
                mi.free_frames <= mi.total_frames,
                "free={} total={}",
                mi.free_frames,
                mi.total_frames
            );
            println!(
                "init: OK  meminfo total={} libre={} reclaimable={}",
                mi.total_frames, mi.free_frames, mi.reclaimable_frames
            );
        }
        {
            let mut st = abi::WifiStatus::default();
            let r = sys::wifi_status(&mut st);
            check!(
                r == 0 || r == -abi::ENOSYS,
                "wifi_status errno {r}"
            );
        }
        check!(
            ALLOC_MAL.load(Ordering::Relaxed) == 0,
            "arena concurrente: {} bloques corruptos en {} hilos × 200 reservas",
            ALLOC_MAL.load(Ordering::Relaxed),
            N
        );
    }

    // Pipes: una escritura grande NO se recorta a un temporal del kernel.
    //
    // Aquí había una pérdida de datos silenciosa: el pipe transfería 256 bytes por
    // llamada porque ése era el tamaño del búfer de tránsito del kernel, lo decía
    // en el retorno (correcto) y nadie lo miraba (no correcto). `cat` de un fichero
    // por SSH entregaba 1435 bytes de 3086 y salía con éxito. Se fija por los dos
    // lados: que el kernel transfiera hasta llenar el pipe y que `write_all`
    // complete lo que una escritura corta deje a medias.
    {
        const GRANDE: usize = 3000;
        let mut w_buf = alloc::vec![0u8; GRANDE];
        for (i, b) in w_buf.iter_mut().enumerate() {
            *b = (i % 251) as u8;
        }
        let (r, w) = match sys::pipe() {
            Ok(p) => p,
            Err(e) => {
                println!("init: FALLO  pipe() para la escritura grande, errno {e}");
                return 1;
            }
        };
        let n = sys::write(w, &w_buf);
        check!(
            n == GRANDE as i64,
            "una escritura de {GRANDE} B al pipe transfiere {n} (el tope de 256 era              del búfer del kernel, no del pipe)"
        );
        let mut r_buf = alloc::vec![0u8; GRANDE];
        let leidos = sys::read(r, &mut r_buf);
        check!(
            leidos == GRANDE as i64 && r_buf == w_buf,
            "y se relee igual ({leidos} B)"
        );
        // Y por encima de la capacidad del pipe (4 KiB) la escritura ES corta: eso
        // sí es legítimo, y `write_all` es quien tiene que completarla.
        let enorme = alloc::vec![0x5au8; 6000];
        let corta = sys::write(w, &enorme);
        check!(
            corta > 0 && corta < 6000,
            "por encima de la capacidad la escritura es corta ({corta} de 6000)"
        );
        let mut vaciar = alloc::vec![0u8; 8192];
        let _ = sys::read(r, &mut vaciar);
        sys::close(r);
        sys::close(w);
    }

    // Punteros que no son del proceso: EFAULT, no un fallo de página EN EL KERNEL.
    //
    // `sys_read` valida al entrar, pero tres caminos usaban `buf` antes de cualquier
    // comprobación —`write` a un pipe, `write` a un socket y `read_timeout` de un
    // socket—, así que un puntero basura hacía que el kernel copiase de una
    // dirección ajena: con suerte pánico, con mala suerte los datos de otro proceso.
    // Cualquier programa podía tumbar el sistema con una syscall.
    {
        const BASURA: u64 = 0x0000_7f00_dead_0000;
        let (r, w) = match sys::pipe() {
            Ok(p) => p,
            Err(e) => {
                println!("init: FALLO  pipe() para la prueba de punteros, errno {e}");
                return 1;
            }
        };
        let rc = sys::raw4(abi::SYS_WRITE, w, BASURA, 64, 0);
        check!(
            rc == -abi::EFAULT,
            "write a un pipe con puntero ajeno da EFAULT (rc={rc})"
        );
        // Y el camino bueno sigue funcionando después del rechazo.
        check!(sys::write(w, b"ok") == 2, "el pipe sigue usable tras el EFAULT");
        let mut b = [0u8; 2];
        check!(sys::read(r, &mut b) == 2 && &b == b"ok", "y se relee");
        sys::close(r);
        sys::close(w);
    }

    // Filesystem desde VARIOS HILOS a la vez.
    //
    // La auditoría de concurrencia de `fs` estaba pendiente y no tenía ninguna
    // prueba: todo lo que había era de un hilo. Esto no demuestra que sosofs sea
    // correcto bajo SMP, pero sí ejercita el camino que nadie ejercitaba —crear,
    // escribir, releer y borrar ficheros distintos desde hilos que corren en cores
    // distintos— y con SMP=4 un candado que falte se manifiesta como contenido
    // cruzado o un panic, no como un "parece que va".
    {
        use core::sync::atomic::{AtomicU32, Ordering};
        use libsoso::thread;

        static FS_LISTOS: AtomicU32 = AtomicU32::new(0);
        static FS_MAL: AtomicU32 = AtomicU32::new(0);
        const FS_HILOS: u32 = 4;
        const FS_VUELTAS: u32 = 12;

        extern "C" fn fs_entry(arg: u64) -> ! {
            let id = arg as u32;
            for vuelta in 0..FS_VUELTAS {
                // Nombre propio por hilo: lo que se comprueba es el aislamiento
                // entre ficheros distintos, no la escritura concurrente al mismo
                // (eso no lo promete nadie).
                let mut nombre = [0u8; 24];
                let base = b"/tmp/fs_";
                nombre[..base.len()].copy_from_slice(base);
                nombre[base.len()] = b'0' + (id % 10) as u8;
                nombre[base.len() + 1] = b'_';
                nombre[base.len() + 2] = b'0' + (vuelta % 10) as u8;
                let ruta = core::str::from_utf8(&nombre[..base.len() + 3]).unwrap_or("/tmp/x");

                // 4 KiB, no 64 B: cruza el bloque de sosofs, así que ejercita la
                // asignación de bloques y el CoW, no sólo el inodo. Y va en el heap
                // (el arena de libsoso), que es otro camino compartido entre hilos.
                let contenido = alloc::vec![b'a' + (id % 26) as u8; 4096];
                let fd = sys::open(ruta, abi::O_WRONLY);
                if fd < 0 {
                    FS_MAL.fetch_add(1, Ordering::Relaxed);
                    continue;
                }
                if sys::write(fd as u64, &contenido) != contenido.len() as i64 {
                    FS_MAL.fetch_add(1, Ordering::Relaxed);
                }
                sys::close(fd as u64);

                let mut leido = alloc::vec![0u8; contenido.len()];
                let fd = sys::open(ruta, abi::O_RDONLY);
                if fd < 0 {
                    FS_MAL.fetch_add(1, Ordering::Relaxed);
                    continue;
                }
                let n = sys::read(fd as u64, &mut leido);
                sys::close(fd as u64);
                if n != contenido.len() as i64 || leido != contenido {
                    FS_MAL.fetch_add(1, Ordering::Relaxed);
                }
                sys::unlink(ruta);
            }
            // Una línea por hilo al terminar. Parece ruido y no lo es: cuando la
            // batería se quedaba muda justo aquí, esta línea fue lo que DESCARTÓ
            // el futex — no aparecía ninguna de las cuatro, o sea que los hilos
            // no habían llegado al `fetch_add` y el principal dormido era el
            // comportamiento correcto. (La avería estaba en otro sitio: la pila
            // de red corriendo desde la IRQ dura, ver `net_irq_handler`.) Se
            // queda porque separa «falta un hilo» de «se perdió el despertar»,
            // que desde fuera se ven exactamente igual.
            println!("init: fs hilo {id} terminó sus {FS_VUELTAS} vueltas");
            FS_LISTOS.fetch_add(1, Ordering::Release);
            let _ = sys::futex_wake(&FS_LISTOS as *const AtomicU32 as *const u32, u64::MAX);
            sys::exit(0);
        }

        FS_LISTOS.store(0, Ordering::Relaxed);
        FS_MAL.store(0, Ordering::Relaxed);
        let mut handles = alloc::vec::Vec::new();
        let mut err: i64 = 0;
        for i in 0..FS_HILOS as usize {
            match thread::spawn(fs_entry, i as u64) {
                Ok(h) => handles.push(h),
                Err(e) => {
                    err = e;
                    break;
                }
            }
        }
        check!(err == 0, "thread_spawn para fs ×{FS_HILOS} (errno {err})");
        while FS_LISTOS.load(Ordering::Acquire) < FS_HILOS {
            let d = FS_LISTOS.load(Ordering::Acquire);
            let _ = sys::futex_wait(&FS_LISTOS as *const AtomicU32 as *const u32, d);
        }
        for h in handles {
            let _ = h.join();
        }
        check!(
            FS_MAL.load(Ordering::Relaxed) == 0,
            "fs concurrente: {} errores en {} hilos × {} ficheros",
            FS_MAL.load(Ordering::Relaxed),
            FS_HILOS,
            FS_VUELTAS
        );
    }

    // Camino de syscalls GPU sobre el dispositivo software del kernel.
    //
    // POR QUÉ ESTÁ AQUÍ: sin GPU en PCI, `SYS_GPU_ALLOC/MAP/SUBMIT/READ` no se
    // ejecutan ni una vez en QEMU, así que el primer sitio donde se probarían es
    // la tarjeta real — y allí un fallo de fontanería (un handle mal contado, un
    // búfer que se quedó corto) es indistinguible de un fallo de la GPU, con un
    // ciclo VFIO de coste por intento. El dispositivo software calcula en la CPU
    // del kernel: no prueba nada de la GPU, prueba TODO lo que la rodea.
    {
        let soft = sys::gpu_submit(b"SOFTG");
        if soft < 0 {
            // Con GPU real presente el kernel contesta EBUSY y no se toca nada:
            // esta subprueba no va a tapar un dispositivo de verdad. Pero entonces
            // hay algo MEJOR que hacer: una sonda de UN solo matvec sobre el
            // silicio. Acotada a propósito — un lanzamiento, un semáforo, un
            // timeout— porque en un ciclo de VFIO conviene saber si el camino de
            // GPU funciona antes de soltarle una inferencia de cientos de matvec.
            println!("init: OK  sin dispositivo software (rc={soft}, hay GPU real o no aplica)");
            let mut info = abi::GpuInfo::default();
            if sys::gpu_info(&mut info) == 0 && info.present == 1 && info.compute == 1 {
                const R: usize = 4;
                const C: usize = 8;
                let mut w = [0.0f32; R * C];
                let mut x = [0.0f32; C];
                let mut esperado = [0.0f32; R];
                for c in 0..C {
                    x[c] = (c as f32) + 1.0;
                }
                for r in 0..R {
                    let mut sum = 0.0f32;
                    for c in 0..C {
                        w[r * C + c] = ((r * C + c) as f32) * 0.5 - 3.0;
                        sum += w[r * C + c] * x[c];
                    }
                    esperado[r] = sum;
                }
                // DOS sondas con la misma matriz, y en este orden: primero los
                // pesos en sysmem (el camino escalonado, el que llevaba meses
                // funcionando) y después residentes en VRAM (el camino G6). Con
                // una sola no se puede distinguir "el compute está roto" de "la
                // ventana de VRAM está rota", que es justo lo que pasó el
                // 2026-07-30: el primer lanzamiento del arranque fue el
                // residente, colgó, y el log no decía cuál de los dos fallaba.
                // TRES pasadas, y la repetición de sysmem no es por gusto: el
                // volcado de registros que RM adjunta al GR_EXCEPTION dice que
                // la interrupción viene del CTXCTL (0x400100 bit 19 =
                // `gf100_gr_ctxctl_isr` en nouveau), o sea del cambio de
                // contexto, no del SM. Eso abre la posibilidad de que lo que
                // falle no sea «los pesos en VRAM» sino «el SEGUNDO lanzamiento
                // del canal», que es el primero que obliga a FECS a salvar y
                // restaurar contexto. Si la segunda pasada de sysmem también
                // falla, la variable no era la VRAM y llevábamos mirando al
                // sitio equivocado (2026-08-02).
                for (etiqueta, en_vram) in [
                    ("sysmem", false),
                    ("sysmem otra vez", false),
                    ("VRAM", true),
                ] {
                    let wh = if en_vram {
                        sys::gpu_alloc_vram((w.len() * 4) as u64)
                    } else {
                        sys::gpu_alloc((w.len() * 4) as u64)
                    };
                    // x e y son scratch: siempre sysmem, como en la inferencia.
                    let xh = sys::gpu_alloc((x.len() * 4) as u64);
                    let yh = sys::gpu_alloc((R * 4) as u64);
                    if wh < 0 || xh < 0 || yh < 0 {
                        continue;
                    }
                    let (wh, xh, yh) = (wh as u64, xh as u64, yh as u64);
                    let subido = sys::gpu_map(wh, w.as_ptr() as u64, (w.len() * 4) as u64) == 0
                        && sys::gpu_map(xh, x.as_ptr() as u64, (x.len() * 4) as u64) == 0;
                    let mut cmd = [0u8; 37];
                    cmd[0..5].copy_from_slice(b"MATVF");
                    cmd[5..13].copy_from_slice(&wh.to_le_bytes());
                    cmd[13..17].copy_from_slice(&(R as u32).to_le_bytes());
                    cmd[17..21].copy_from_slice(&(C as u32).to_le_bytes());
                    cmd[21..29].copy_from_slice(&xh.to_le_bytes());
                    cmd[29..37].copy_from_slice(&yh.to_le_bytes());
                    let bits = if subido { sys::gpu_submit(&cmd) } else { -1 };
                    let mut y = [0.0f32; R];
                    let leido = bits >= 0
                        && sys::gpu_read(yh, y.as_mut_ptr() as u64, (R * 4) as u64) == 0;
                    let bien = leido
                        && (0..R).all(|r| (y[r] - esperado[r]).abs() < 0.01);
                    let en_gpu = bits >= 0
                        && (bits as u64) & abi::GPU_SUBMIT_ON_GPU != 0;
                    // El resultado correcto es obligatorio; que lo haya hecho el
                    // silicio, no: sin canal el kernel lo calcula en CPU y lo dice.
                    check!(
                        bien,
                        "SONDA GPU ({etiqueta}): un matvec {R}x{C} da el resultado correcto                          (bits={bits:#x}, on_gpu={})",
                        en_gpu as u8
                    );
                    if en_gpu {
                        println!("init: >>> G5 EN SILICIO ({etiqueta}): on_gpu=1 <<<");
                    } else {
                        // La fase la da el propio kernel en `GpuInfo`: antes esto
                        // mandaba a abrir el log de serie, que son cientos de
                        // líneas para averiguar una palabra.
                        let fase = libsoso::str_hasta_nul(&info.phase);
                        println!(
                            "init: sonda GPU ({etiqueta}) calculada por la CPU del kernel                              (on_gpu=0) — el GSP se quedó en la fase «{fase}»"
                        );
                    }
                    let _ = sys::gpu_free(wh);
                    let _ = sys::gpu_free(xh);
                    let _ = sys::gpu_free(yh);
                }

                // Y una subida GRANDE a VRAM, que es lo que rompía en la placa:
                // los `ffn_up`/`ffn_down` de TinyLlama son 44 MiB en f32 y la
                // syscall los rechazaba con EFAULT por un techo de 16 MiB.
                //
                // OJO: esta sonda **no** vale como prueba del camino sin copias, y
                // su comentario decía que sí. El origen es un `mmap` anónimo, cuya
                // base está alineada a página por construcción, así que por diseño
                // no podía cazar el fallo que sí había: el payload de un shard
                // empieza en el byte 64 y el kernel exigía alineación de página, de
                // modo que los pesos de verdad se subían por rebote y nadie lo
                // sabía. Esa prueba es la de abajo, sobre un `.tensor` real.
                if info.vram_bufs == 1 {
                    const GRANDE: u64 = 20 * 1024 * 1024;
                    let src = sys::mmap(0, GRANDE, u64::MAX, 0);
                    let h = sys::gpu_alloc_vram(GRANDE);
                    if src > 0 && h >= 0 {
                        unsafe {
                            (src as *mut u32).write(0xa5a5_a5a5);
                            ((src as u64 + GRANDE - 4) as *mut u32).write(0x5a5a_5a5a);
                        }
                        let rc = sys::gpu_map(h as u64, src as u64, GRANDE);
                        check!(
                            rc == 0,
                            "SONDA GPU: subida de {} MiB a VRAM (rc={rc}) — el tamaño \
                             que la syscall rechazaba",
                            GRANDE / (1024 * 1024)
                        );
                        // LA SONDA QUE FALTABA: un origen que NO empieza en
                        // frontera de página. El payload de un shard `.som` está
                        // en el byte 64 del fichero, así que el puntero de todo
                        // tensor mapeado llega a `gpu_map` en +64; mientras el
                        // kernel exigió `%4096 == 0`, el camino sin copias no se
                        // ejecutó ni una vez con pesos de verdad y ninguna prueba
                        // podía verlo, porque todas subían desde un `mmap` cuya
                        // base está alineada por construcción. Se piden 64 bytes
                        // menos una página para no salirse del mapeo.
                        const DESALINEADO: u64 = GRANDE - 4096;
                        let h2 = sys::gpu_alloc_vram(DESALINEADO);
                        if h2 >= 0 {
                            let rc = sys::gpu_map(h2 as u64, src as u64 + 64, DESALINEADO);
                            check!(
                                rc == 0,
                                "SONDA GPU: subida de {} MiB desde un origen en +64 \
                                 (rc={rc}) — el caso del payload de un shard",
                                DESALINEADO / (1024 * 1024)
                            );
                            let _ = sys::gpu_free(h2 as u64);
                        }
                        // Y que hayan ido por DMA, no por rebote. Sin esto lo
                        // anterior pasa igual: el rebote da el mismo resultado, sólo
                        // cuesta una copia entera del tensor por la CPU.
                        let mut tras = abi::GpuInfo::default();
                        if sys::gpu_info(&mut tras) == 0 {
                            check!(
                                tras.uploads_dma > 0 && tras.uploads_bounce == 0,
                                "SONDA GPU: las subidas fueron por DMA del CE \
                                 (dma={} rebote={} {} MiB copiados)",
                                tras.uploads_dma,
                                tras.uploads_bounce,
                                tras.bounce_bytes >> 20
                            );
                        }
                        let _ = sys::gpu_free(h as u64);
                    } else {
                        println!(
                            "init: sin sitio para la sonda grande (mmap={src} handle={h})"
                        );
                    }
                    if src > 0 {
                        sys::munmap(src as u64, GRANDE);
                    }
                } else {
                    let fase = libsoso::str_hasta_nul(&info.phase);
                    println!(
                        "init: sin pool de VRAM (fase «{fase}») — la sonda de subida grande no aplica"
                    );
                }
            }
        } else {
            let mut info = abi::GpuInfo::default();
            check!(
                sys::gpu_info(&mut info) == 0 && info.present == 1 && info.compute == 1,
                "gpu_info: dispositivo software presente y con cómputo"
            );

            // Matriz 3×4 y vector, con valores que hacen visible cualquier
            // transposición: si se confundieran filas y columnas, los resultados
            // no coincidirían con la referencia de abajo.
            const ROWS: usize = 3;
            const COLS: usize = 4;
            let w: [f32; ROWS * COLS] = [
                1.0, 2.0, 3.0, 4.0,
                5.0, 6.0, 7.0, 8.0,
                -1.0, 0.5, 2.0, -3.0,
            ];
            let x: [f32; COLS] = [1.0, 10.0, 100.0, 1000.0];
            let mut esperado = [0.0f32; ROWS];
            for r in 0..ROWS {
                let mut sum = 0.0f32;
                for c in 0..COLS {
                    sum += w[r * COLS + c] * x[c];
                }
                esperado[r] = sum;
            }

            let w_h = sys::gpu_alloc((w.len() * 4) as u64);
            let x_h = sys::gpu_alloc((x.len() * 4) as u64);
            let y_h = sys::gpu_alloc((ROWS * 4) as u64);
            check!(
                w_h >= 0 && x_h >= 0 && y_h >= 0,
                "gpu_alloc ×3 (w={w_h} x={x_h} y={y_h})"
            );
            let (w_h, x_h, y_h) = (w_h as u64, x_h as u64, y_h as u64);

            // Subir más de lo que cabe tiene que ser EINVAL, no un recorte
            // silencioso: el recorte convertía un búfer que se quedó pequeño en
            // media matriz subida y un resultado creíble.
            let rc = sys::gpu_map(y_h, w.as_ptr() as u64, (w.len() * 4) as u64);
            check!(
                rc == -abi::EINVAL,
                "gpu_map de {} B en un búfer de {} B da EINVAL (rc={rc})",
                w.len() * 4,
                ROWS * 4
            );
            let rc = sys::gpu_read(y_h, esperado.as_mut_ptr() as u64, 4096);
            check!(rc == -abi::EINVAL, "gpu_read pasado de largo da EINVAL (rc={rc})");
            // `esperado` no se ha tocado (EINVAL antes de escribir), pero se
            // recalcula por si acaso: una comprobación que se apoya en un búfer
            // que acaba de fallar no demuestra nada.
            for r in 0..ROWS {
                let mut sum = 0.0f32;
                for c in 0..COLS {
                    sum += w[r * COLS + c] * x[c];
                }
                esperado[r] = sum;
            }

            // Los dos rc por separado: un `&&` de dos syscalls dice que algo falló
            // pero no cuál ni por qué, y con -22/-14/-38 en juego eso es la
            // diferencia entre un handle malo, un puntero que el proceso no puede
            // leer y un búfer que vive en el dispositivo.
            let rc_w = sys::gpu_map(w_h, w.as_ptr() as u64, (w.len() * 4) as u64);
            let rc_x = sys::gpu_map(x_h, x.as_ptr() as u64, (x.len() * 4) as u64);
            check!(
                rc_w == 0 && rc_x == 0,
                "gpu_map de w y x (rc_w={rc_w} rc_x={rc_x})"
            );

            let mut cmd = [0u8; 37];
            cmd[0..5].copy_from_slice(b"MATVF");
            cmd[5..13].copy_from_slice(&w_h.to_le_bytes());
            cmd[13..17].copy_from_slice(&(ROWS as u32).to_le_bytes());
            cmd[17..21].copy_from_slice(&(COLS as u32).to_le_bytes());
            cmd[21..29].copy_from_slice(&x_h.to_le_bytes());
            cmd[29..37].copy_from_slice(&y_h.to_le_bytes());
            let bits = sys::gpu_submit(&cmd);
            check!(bits >= 0, "gpu_submit MATVF (rc={bits})");
            let bits = bits as u64;
            check!(
                bits & abi::GPU_SUBMIT_COMPUTED != 0,
                "MATVF dice COMPUTED (el resultado está en el búfer)"
            );
            check!(
                bits & abi::GPU_SUBMIT_ON_GPU == 0,
                "MATVF NO dice ON_GPU: lo calculó la CPU del kernel y se admite"
            );

            let mut y = [0.0f32; ROWS];
            check!(
                sys::gpu_read(y_h, y.as_mut_ptr() as u64, (ROWS * 4) as u64) == 0,
                "gpu_read del resultado"
            );
            let mut iguales = true;
            for r in 0..ROWS {
                if (y[r] - esperado[r]).abs() > 0.001 {
                    iguales = false;
                }
            }
            check!(
                iguales,
                "MATVF calcula y=W·x ({} {} {} vs {} {} {})",
                y[0], y[1], y[2], esperado[0], esperado[1], esperado[2]
            );

            // MATVQ: la matriz va CUANTIZADA en el búfer y el dispositivo la
            // multiplica sin expandirla. Se prueba con Q8_0 —escala f32 + 32 int8,
            // sin escalas de 6 bits ni nibbles cruzados— porque `init` no enlaza
            // `soso-llm-core` y los bytes se construyen a mano aquí: si esto falla,
            // el problema es del comando y no de la decodificación.
            {
                const QROWS: usize = 3;
                const QCOLS: usize = 32; // un bloque exacto
                const QBLK: usize = 36; // 4 + 32
                let mut wq = [0u8; QROWS * QBLK];
                let mut esp = [0.0f32; QROWS];
                // `x` del MATVF son 4 columnas; aquí hacen falta 32 (un bloque).
                let mut xq = [0.0f32; QCOLS];
                for (c, v) in xq.iter_mut().enumerate() {
                    *v = (c as f32 - 16.0) * 0.5;
                }
                let escala = 0.25f32;
                for r in 0..QROWS {
                    wq[r * QBLK..r * QBLK + 4].copy_from_slice(&escala.to_le_bytes());
                    let mut sum = 0.0f32;
                    for c in 0..QCOLS {
                        let q = ((r * 7 + c) as i32 % 61 - 30) as i8;
                        wq[r * QBLK + 4 + c] = q as u8;
                        sum += q as f32 * escala * xq[c];
                    }
                    esp[r] = sum;
                }
                let wq_h = sys::gpu_alloc(wq.len() as u64);
                let xq_h = sys::gpu_alloc((QCOLS * 4) as u64);
                let yq_h = sys::gpu_alloc((QROWS * 4) as u64);
                check!(
                    wq_h >= 0 && xq_h >= 0 && yq_h >= 0,
                    "gpu_alloc para MATVQ (w={wq_h} x={xq_h} y={yq_h})"
                );
                let (wq_h, xq_h, yq_h) = (wq_h as u64, xq_h as u64, yq_h as u64);
                check!(
                    sys::gpu_map(wq_h, wq.as_ptr() as u64, wq.len() as u64) == 0
                        && sys::gpu_map(xq_h, xq.as_ptr() as u64, (QCOLS * 4) as u64) == 0,
                    "gpu_map de la matriz Q8_0 en crudo y del vector"
                );
                let mut q = [0u8; 38];
                q[0..5].copy_from_slice(b"MATVQ");
                q[5..13].copy_from_slice(&wq_h.to_le_bytes());
                q[13..17].copy_from_slice(&(QROWS as u32).to_le_bytes());
                q[17..21].copy_from_slice(&(QCOLS as u32).to_le_bytes());
                q[21..29].copy_from_slice(&xq_h.to_le_bytes());
                q[29..37].copy_from_slice(&yq_h.to_le_bytes());
                q[37] = 2; // DTYPE_Q8_0
                let bits = sys::gpu_submit(&q);
                check!(bits >= 0, "gpu_submit MATVQ (rc={bits})");
                let bits = bits as u64;
                check!(
                    bits & abi::GPU_SUBMIT_COMPUTED != 0,
                    "MATVQ dice COMPUTED"
                );
                let mut yq = [0.0f32; QROWS];
                check!(
                    sys::gpu_read(yq_h, yq.as_mut_ptr() as u64, (QROWS * 4) as u64) == 0,
                    "gpu_read del resultado de MATVQ"
                );
                let mut ok = true;
                for r in 0..QROWS {
                    if (yq[r] - esp[r]).abs() > 0.01 {
                        ok = false;
                    }
                }
                check!(
                    ok,
                    "MATVQ multiplica Q8_0 sin expandir ({} {} {} vs {} {} {})",
                    yq[0], yq[1], yq[2], esp[0], esp[1], esp[2]
                );

                // Los dos rechazos que impiden un resultado creíble y falso: un
                // `cols` que no es bloque entero se manda a CPU (sin COMPUTED), y un
                // búfer más pequeño que `rows*row_bytes` es EINVAL. Sin el segundo,
                // el kernel leería la fila del tensor de al lado.
                q[17..21].copy_from_slice(&33u32.to_le_bytes());
                let bits = sys::gpu_submit(&q);
                check!(
                    bits >= 0 && (bits as u64) & abi::GPU_SUBMIT_COMPUTED == 0,
                    "MATVQ con cols=33 (no es bloque Q8_0 entero) se va a CPU (rc={bits})"
                );
                q[17..21].copy_from_slice(&(QCOLS as u32).to_le_bytes());
                q[13..17].copy_from_slice(&(QROWS as u32 + 1).to_le_bytes());
                let bits = sys::gpu_submit(&q);
                check!(
                    bits == -abi::EINVAL,
                    "MATVQ con más filas de las que caben en el búfer da EINVAL (rc={bits})"
                );
                let _ = sys::gpu_free(wq_h);
                let _ = sys::gpu_free(xq_h);
                let _ = sys::gpu_free(yq_h);
            }

            // La ASIMETRÍA de permisos entre las dos syscalls, que es lo que
            // permite subir los pesos sin copiarlos: `gpu_map` LEE del proceso, así
            // que un mapeo de sólo lectura (como el del modelo) vale; `gpu_read`
            // ESCRIBE en él, así que el mismo mapeo tiene que ser rechazado. Si
            // alguien "endurece" el primero, los pesos dejarían de subirse y el
            // offload se iría a CPU sin decir nada — por eso está fijado aquí.
            {
                let mut st_ro = abi::Stat::default();
                if sys::stat("/etc/motd", &mut st_ro) == 0 && st_ro.size >= 8 {
                    let fd = sys::open("/etc/motd", abi::O_RDONLY);
                    if fd >= 0 {
                        let map = sys::mmap(0, st_ro.size, fd as u64, 0);
                        check!(map > 0, "mmap de sólo lectura para el dispositivo");
                        let h = sys::gpu_alloc(8);
                        check!(h >= 0, "gpu_alloc para la prueba de sólo lectura");
                        let h = h as u64;
                        check!(
                            sys::gpu_map(h, map as u64, 8) == 0,
                            "gpu_map LEE: acepta un mapeo de sólo lectura (los pesos \
                             del modelo están así)"
                        );
                        check!(
                            sys::gpu_read(h, map as u64, 8) == -abi::EFAULT,
                            "gpu_read ESCRIBE: rechaza el mismo mapeo con EFAULT"
                        );
                        let _ = sys::gpu_free(h);
                        sys::munmap(map as u64, st_ro.size.next_multiple_of(4096) as u64);
                        sys::close(fd as u64);
                    }
                }
            }

            // Una subida MAYOR DE 16 MiB, que es el agujero que dejó la placa en
            // rojo el 2026-08-17: `gpu_map` y `gpu_read` validaban el rango con el
            // techo de un búfer de syscall normal, así que cualquier tensor por
            // encima daba EFAULT — y ni un test subía más de unos KiB, así que el
            // primer sitio donde se vio fue TinyLlama en la placa, con sus
            // `ffn_up`/`ffn_down` de 44 MiB en f32, disfrazado de «subida de
            // pesos» del offload. 20 MiB es lo justo para pasarse del techo.
            //
            // Anónimo y no un `Vec`: hace falta que el rango exista sin haber sido
            // tocado, que es además el caso que materializa la propia syscall.
            {
                const GRANDE: u64 = 20 * 1024 * 1024;
                let src = sys::mmap(0, GRANDE, u64::MAX, 0);
                let dst = sys::mmap(0, GRANDE, u64::MAX, 0);
                check!(src > 0 && dst > 0, "mmap ×2 de {} MiB", GRANDE / (1024 * 1024));
                // GART y no VRAM: el ida y vuelta necesita que el búfer sea
                // legible desde la CPU, y un búfer del dispositivo no lo es
                // (`gpu_read` da ENOSYS sobre VRAM residente). Da igual para lo
                // que se comprueba: el techo estaba en la SYSCALL, no en el tipo
                // de búfer. La variante en VRAM va justo debajo.
                let h = sys::gpu_alloc(GRANDE);
                check!(h >= 0, "gpu_alloc de {} MiB (rc={h})", GRANDE / (1024 * 1024));
                if src > 0 && dst > 0 && h >= 0 {
                    let (h, src, dst) = (h as u64, src as u64, dst as u64);
                    // Centinelas en los dos extremos: el del final es el que caza un
                    // recorte silencioso, que es el fallo que un tope disfraza.
                    let primero = 0xa5a5_a5a5u32;
                    let ultimo = 0x5a5a_5a5au32;
                    unsafe {
                        (src as *mut u32).write(primero);
                        ((src + GRANDE - 4) as *mut u32).write(ultimo);
                    }
                    let rc = sys::gpu_map(h, src, GRANDE);
                    check!(rc == 0, "gpu_map de {} MiB (rc={rc})", GRANDE / (1024 * 1024));
                    let rc = sys::gpu_read(h, dst, GRANDE);
                    check!(rc == 0, "gpu_read de {} MiB (rc={rc})", GRANDE / (1024 * 1024));
                    let (a, b) = unsafe {
                        (
                            (dst as *const u32).read(),
                            ((dst + GRANDE - 4) as *const u32).read(),
                        )
                    };
                    check!(
                        a == primero && b == ultimo,
                        "la subida grande vuelve entera (0x{a:08x}/0x{b:08x} \
                         esperado 0x{primero:08x}/0x{ultimo:08x})"
                    );
                    // Y el tope del BÚFER sigue en pie: lo que se quitó es el techo
                    // de la syscall, no la comprobación de que el destino quepa.
                    let rc = sys::gpu_map(h, src, GRANDE + 4);
                    check!(rc == -abi::EINVAL, "pasarse del búfer sigue dando EINVAL (rc={rc})");
                    let _ = sys::gpu_free(h);
                    sys::munmap(src, GRANDE);
                    sys::munmap(dst, GRANDE);
                }
            }

            // free devuelve los bytes y el handle deja de valer.
            let freed = sys::gpu_free(w_h);
            check!(
                freed == (w.len() * 4) as i64,
                "gpu_free devuelve {} bytes (esperado {})",
                freed,
                w.len() * 4
            );
            check!(
                sys::gpu_free(w_h) == -abi::EINVAL,
                "gpu_free dos veces del mismo handle da EINVAL"
            );
            check!(
                sys::gpu_map(w_h, w.as_ptr() as u64, 4) == -abi::EINVAL,
                "un handle liberado ya no acepta datos"
            );
            let _ = sys::gpu_free(x_h);
            let _ = sys::gpu_free(y_h);

            // Y se apaga: dejarlo puesto haría que el siguiente proceso de este
            // arranque viera una GPU que no existe.
            check!(
                sys::gpu_submit(b"SOFTX") == 0,
                "dispositivo software apagado"
            );
            let mut info = abi::GpuInfo::default();
            check!(
                sys::gpu_info(&mut info) == 0 && info.present == 0,
                "gpu_info vuelve a decir que no hay dispositivo"
            );
        }
    }

    // TCP loopback 127.0.0.1: empareja listener userspace sin NIC loopback.
    {
        const PORT: u16 = 17420;
        let listener = sys::tcp_listen(PORT);
        check!(listener >= 0, "tcp_listen loopback (rc={listener})");
        let listener = listener as u64;
        let addr = sys::sock_addr(127, 0, 0, 1, PORT);
        let client = sys::tcp_connect(&addr, 5_000);
        check!(client >= 0, "tcp_connect 127.0.0.1 (rc={client})");
        let client = client as u64;
        let server = sys::tcp_accept(listener, 5_000);
        check!(server >= 0, "tcp_accept loopback (rc={server})");
        let server = server as u64;
        let msg = b"hola loopback";
        check!(sys::write_all(client, msg).is_ok(), "write cliente loopback");
        let mut buf = [0u8; 32];
        let n = sys::read(server, &mut buf);
        check!(n == msg.len() as i64, "read servidor loopback (n={n})");
        check!(&buf[..n as usize] == msg, "datos loopback coinciden");
        // Cerrar un extremo tiene que dar EOF (0) al otro, no EAGAIN eterno.
        // Sin esto sosh giraba en `ask` si el askd moría a media respuesta.
        check!(sys::write_all(server, b"eof").is_ok(), "write servidor antes de cerrar");
        let _ = sys::close(server);
        let n = sys::read_timeout(client, &mut buf, 2_000);
        check!(n == 3 && &buf[..3] == b"eof", "drenar resto tras close (n={n})");
        let n = sys::read_timeout(client, &mut buf, 2_000);
        check!(n == 0, "cierre del par da EOF (n={n})");
        let _ = sys::close(client);
        let client2 = sys::tcp_connect(&addr, 5_000);
        check!(client2 >= 0, "segundo tcp_connect loopback (rc={client2})");
        let server2 = sys::tcp_accept(listener, 5_000);
        check!(server2 >= 0, "segundo tcp_accept loopback (rc={server2})");
        let _ = sys::close(client2 as u64);
        let _ = sys::close(server2 as u64);
        let _ = sys::close(listener);
        let sin_listener = sys::tcp_connect(&addr, 500);
        check!(
            sin_listener == -(abi::ECONNREFUSED as i64),
            "connect sin listener da ECONNREFUSED (rc={sin_listener})"
        );
        let l1 = sys::tcp_listen(PORT);
        check!(l1 >= 0, "re-listen tras cerrar (rc={l1})");
        let l2 = sys::tcp_listen(PORT);
        check!(
            l2 == -(abi::EADDRINUSE as i64),
            "listen duplicado da EADDRINUSE (rc={l2})"
        );
        let _ = sys::close(l1 as u64);
    }

    // Self-hosting (ruta A): rename, reloj, aleatorio, dup2/fstat, getenv.
    {
        let mut path_buf = [0u8; 64];
        let n = sys::getenv("PATH", &mut path_buf);
        check!(n > 0, "SYS_GETENV PATH (n={n})");
        let path = core::str::from_utf8(&path_buf[..n as usize]).unwrap_or("");
        check!(path.contains("/bin"), "PATH contiene /bin ({path})");
        let _ = sys::mkdir("/tmp/sh-test");
        let fd = sys::open("/tmp/sh-test/a", abi::O_WRONLY | abi::O_CREAT | abi::O_TRUNC);
        check!(fd >= 0, "open escribir self-host (fd={fd})");
        let _ = sys::write_all(fd as u64, b"abc");
        let _ = sys::close(fd as u64);
        let rc = sys::rename("/tmp/sh-test/a", "/tmp/sh-test/b");
        check!(rc == 0, "SYS_RENAME (rc={rc})");
        let mut ts = abi::Timespec::default();
        check!(
            sys::clock_gettime(abi::CLOCK_REALTIME, &mut ts) == 0,
            "SYS_CLOCK_GETTIME"
        );
        let mut rnd = [0u8; 8];
        check!(sys::getrandom(&mut rnd) == 8, "SYS_GETRANDOM");
        let fd2 = sys::open("/tmp/sh-test/b", abi::O_RDONLY);
        check!(fd2 >= 0, "open tras rename (fd={fd2})");
        let dup = sys::dup2(fd2 as u64, 9);
        check!(dup == 9, "SYS_DUP2 (rc={dup})");
        let mut st = abi::Stat::default();
        check!(sys::fstat(9, &mut st) == 0, "SYS_FSTAT");
        check!(st.size == 3, "fstat size tras rename");
        let mtime = ts.tv_sec as u64;
        check!(sys::utime("/tmp/sh-test/b", mtime) == 0, "SYS_UTIME");
        let _ = sys::sched_yield();
        let fdw = sys::open("/tmp/sh-test/c", abi::O_WRONLY | abi::O_CREAT | abi::O_TRUNC);
        check!(fdw >= 0, "open pwrite test");
        check!(sys::pwrite(fdw as u64, b"xy", 0) == 2, "SYS_PWRITE");
        check!(sys::fsync(fdw as u64) == 0, "SYS_FSYNC");
        let _ = sys::close(fdw as u64);
        let _ = sys::close(9);
        let _ = sys::close(fd2 as u64);
        let _ = sys::unlink("/tmp/sh-test/b");
    }

    // Hito 2: humo soso-std en guest.
    {
        let pid = sys::spawn("/bin/hola-std", "demo");
        check!(pid > 0, "spawn hola-std (pid={pid})");
        check!(sys::wait().is_ok(), "wait hola-std");
        let pid = sys::spawn("/bin/soso-std-test", "");
        check!(pid > 0, "spawn soso-std-test (pid={pid})");
        let w = sys::wait();
        check!(w.is_ok(), "wait soso-std-test");
        if let Ok((_, code)) = w {
            check!(code == 0, "soso-std-test exit {code}");
        }
        let pid = sys::spawn("/bin/soso-test-sosofs", "");
        check!(pid > 0, "spawn soso-test-sosofs (pid={pid})");
        let w = sys::wait();
        check!(w.is_ok(), "wait soso-test-sosofs");
        if let Ok((_, code)) = w {
            check!(code == 0, "soso-test-sosofs exit {code}");
        }
    }

    // Hito 2/3: self-hosting — forja local y soso-rustc stub.
    {
        let pid = sys::spawn("/bin/soso-forja", "local");
        check!(pid > 0, "spawn soso-forja local (pid={pid})");
        let w = sys::wait();
        check!(w.is_ok(), "wait soso-forja local");
        if let Ok((_, code)) = w {
            check!(code == 0, "soso-forja local exit {code}");
        }
        let pid = sys::spawn("/bin/soso-rustc", "--version");
        check!(pid > 0, "spawn soso-rustc (pid={pid})");
        let w = sys::wait();
        check!(w.is_ok(), "wait soso-rustc");
        if let Ok((_, code)) = w {
            check!(code == 0, "soso-rustc --version exit {code}");
        }
    }

    println!("init: TODO OK — syscalls desde ring 3 (incl. pipe, spawn_io, hilos y GPU)");
    0
}

/// Quema CPU sin tocar el kernel (~decenas de ms en QEMU debug).
fn busy() {
    let mut x = 0u64;
    for i in 0..30_000_000u64 {
        x = x.wrapping_add(i ^ x.rotate_left(7));
    }
    core::hint::black_box(x);
}

/// u64 → &str sin depender de format! (evita otra pasada por el heap).
fn itoa(mut n: u64) -> &'static str {
    static mut BUF: [u8; 20] = [0; 20];
    unsafe {
        let buf = &mut *core::ptr::addr_of_mut!(BUF);
        let mut i = buf.len();
        loop {
            i -= 1;
            buf[i] = b'0' + (n % 10) as u8;
            n /= 10;
            if n == 0 {
                break;
            }
        }
        core::str::from_utf8_unchecked(&buf[i..])
    }
}
