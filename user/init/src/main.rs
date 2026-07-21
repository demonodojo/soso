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

/// Bucle de PID 1: sosh en marcha siempre. Si la shell sale limpia
/// (exit 0), init termina y el kernel vuelve a su shell de emergencia.
fn lanzar_shell() -> u8 {
    loop {
        let pid = sys::spawn("/bin/sosh", "");
        if pid < 0 {
            println!("init: no puedo lanzar /bin/sosh (errno {pid})");
            return 1;
        }
        match sys::wait() {
            Ok((_, 0)) => {
                println!("init: shell cerrada; adiós");
                return 0;
            }
            Ok((_, code)) => {
                println!("init: sosh murió con código {code}; relanzando");
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

    // Hilos + futex: N workers incrementan un contador compartido.
    {
        use core::sync::atomic::{AtomicU32, Ordering};
        use libsoso::thread;

        static COUNTER: AtomicU32 = AtomicU32::new(0);
        static DONE: AtomicU32 = AtomicU32::new(0);
        const N: u32 = 4;
        const PER: u32 = 1000;

        extern "C" fn thr_entry(_arg: u64) -> ! {
            for _ in 0..PER {
                COUNTER.fetch_add(1, Ordering::Relaxed);
            }
            DONE.fetch_add(1, Ordering::Release);
            let _ = sys::futex_wake(&DONE as *const AtomicU32 as *const u32, u64::MAX);
            sys::exit(0);
        }

        COUNTER.store(0, Ordering::Relaxed);
        DONE.store(0, Ordering::Relaxed);
        let mut tids = [0u64; N as usize];
        let mut spawn_err: i64 = 0;
        for i in 0..N as usize {
            match thread::spawn(thr_entry, i as u64) {
                Ok(t) => tids[i] = t,
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
        // Reclamar zombis de los hilos.
        for _ in 0..N {
            let _ = sys::wait();
        }
        check!(
            COUNTER.load(Ordering::Relaxed) == N * PER,
            "hilos: contador={} (esperado {})",
            COUNTER.load(Ordering::Relaxed),
            N * PER
        );
        check!(sys::ncpu() >= 1, "ncpu={}", sys::ncpu());
        let _ = tids;
    }

    println!("init: TODO OK — syscalls desde ring 3 (incl. pipe, spawn_io e hilos)");
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
