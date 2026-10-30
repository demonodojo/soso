//! Sondas de viabilidad para portar **moli** (V8, reservas de VA, W^X, hilos,
//! espera multi-socket). Inventario en `docs/EVALUACION-MOLI.md`.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, Ordering};

use libsoso::{abi, println, sys, thread};

use crate::{errno, Caso};

const PAGINA: u64 = 4096;
const GIB: u64 = 1024 * 1024 * 1024;
const RESERVA_4G: u64 = 4 * GIB;
const RESERVA_1T: u64 = 1024 * GIB;

/// `mov eax, edi; add eax, 1; ret`
const CODIGO_INC: &[u8] = &[0x89, 0xf8, 0x83, 0xc0, 0x01, 0xc3];
/// `mov eax, edi; add eax, 10; ret`
const CODIGO_INC10: &[u8] = &[0x89, 0xf8, 0x83, 0xc0, 0x0a, 0xc3];

static CONTEO_HILOS: AtomicU32 = AtomicU32::new(0);

fn anotar(casos: &mut Vec<Caso>, c: Caso) {
    if c.paso && c.esperado.is_empty() {
        println!("probe: {} medido: {}", c.id, c.observado);
    } else if c.paso {
        println!("probe: {} ok ({})", c.id, c.observado);
    } else {
        println!(
            "probe: {} FALLO esperado={:?} observado={:?}",
            c.id, c.esperado, c.observado
        );
    }
    casos.push(c);
}

unsafe fn llamar(addr: u64, arg: u32) -> u32 {
    let f: extern "C" fn(u32) -> u32 = unsafe { core::mem::transmute(addr as usize) };
    f(arg)
}

extern "C" fn hilo_cuenta(_arg: u64) -> ! {
    CONTEO_HILOS.fetch_add(1, Ordering::SeqCst);
    sys::exit(0)
}

pub fn ejecutar() -> Vec<Caso> {
    let mut casos: Vec<Caso> = Vec::new();

    // 1. Reserva de 4 GiB (jaula de punteros comprimidos de V8).
    let r4 = sys::mmap(0, RESERVA_4G, u64::MAX, 0);
    if r4 < 0 {
        anotar(
            &mut casos,
            Caso::nuevo(
                "moli/reserva-4g-sin-acceso",
                "dirección virtual",
                format!("mmap 4G = {}", errno(r4)),
            ),
        );
    } else {
        let base = r4 as u64;
        // Marcar la primera página sin acceso (equivalente a PROT_NONE sobre un
        // subrango tras reservar).
        let rc0 = sys::mprotect(base, PAGINA, 0);
        let rc_rw = sys::mprotect(base, PAGINA, abi::PROT_READ | abi::PROT_WRITE);
        let mut tocable = false;
        if rc_rw >= 0 {
            unsafe {
                core::ptr::write_volatile(base as *mut u8, 0x5a);
            }
            tocable = unsafe { core::ptr::read_volatile(base as *const u8) } == 0x5a;
        }
        anotar(
            &mut casos,
            Caso::nuevo("moli/reserva-4g-mmap", "ok", String::from("ok")),
        );
        anotar(
            &mut casos,
            Caso::nuevo("moli/reserva-4g-prot-none", "0", errno(rc0)),
        );
        anotar(
            &mut casos,
            Caso::nuevo(
                "moli/reserva-4g-subrango-rw",
                if tocable { "byte ok" } else { "byte ok" },
                if tocable {
                    String::from("byte ok")
                } else {
                    format!("RW→{} sin byte", errno(rc_rw))
                },
            ),
        );
        sys::munmap(base, RESERVA_4G);
    }

    // 2. Reserva de 1 TiB (sandbox de V8 cuando `v8_enable_sandbox=true`).
    let r1t = sys::mmap(0, RESERVA_1T, u64::MAX, 0);
    anotar(
        &mut casos,
        Caso::observacion(
            "moli/reserva-1t",
            if r1t < 0 {
                format!(
                    "mmap 1T falló ({}) — ventana mmap ~416 GiB; compilar V8 con sandbox=false",
                    errno(r1t)
                )
            } else {
                let base = r1t as u64;
                sys::munmap(base, RESERVA_1T);
                format!("mmap 1T ok va=0x{base:x} (inesperado en ventana ~416 GiB)")
            },
        ),
    );

    // 3. JIT alternando RW ↔ RX (W^X estricto).
    let pagina = sys::mmap(0, PAGINA, u64::MAX, 0);
    if pagina < 0 {
        anotar(
            &mut casos,
            Caso::nuevo("moli/jit-alterna-rw-rx", "42 y 51", format!("mmap = {}", errno(pagina))),
        );
    } else {
        let addr = pagina as u64;
        let rc = sys::mprotect(addr, PAGINA, abi::PROT_READ | abi::PROT_WRITE);
        if rc < 0 {
            anotar(
                &mut casos,
                Caso::nuevo(
                    "moli/jit-alterna-rw-rx",
                    "42 y 51",
                    format!("mprotect RW = {}", errno(rc)),
                ),
            );
        } else {
            unsafe {
                core::ptr::copy_nonoverlapping(CODIGO_INC.as_ptr(), addr as *mut u8, CODIGO_INC.len());
            }
            let rc_rx = sys::mprotect(addr, PAGINA, abi::PROT_READ | abi::PROT_EXEC);
            println!("probe: moli/jit-alterna-rw-rx llamando en RX (si muere, falló W^X)");
            let (r1, r2, obs) = if rc_rx < 0 {
                (
                    0u32,
                    0u32,
                    format!("mprotect RX = {}", errno(rc_rx)),
                )
            } else {
                let a = unsafe { llamar(addr, 41) };
                let rc_rw2 = sys::mprotect(addr, PAGINA, abi::PROT_READ | abi::PROT_WRITE);
                if rc_rw2 < 0 {
                    (a, 0u32, format!("vuelta RW = {}", errno(rc_rw2)))
                } else {
                    unsafe {
                        core::ptr::copy_nonoverlapping(
                            CODIGO_INC10.as_ptr(),
                            addr as *mut u8,
                            CODIGO_INC10.len(),
                        );
                    }
                    let rc_rx2 = sys::mprotect(addr, PAGINA, abi::PROT_READ | abi::PROT_EXEC);
                    if rc_rx2 < 0 {
                        (a, 0u32, format!("segundo RX = {}", errno(rc_rx2)))
                    } else {
                        let b = unsafe { llamar(addr, 41) };
                        (a, b, format!("{a} y {b}"))
                    }
                }
            };
            anotar(
                &mut casos,
                Caso::nuevo("moli/jit-alterna-rw-rx", "42 y 51", obs),
            );
            let _ = (r1, r2);
        }
        sys::munmap(addr, PAGINA);
    }

    // 4. Pool de hilos (~24 en moli en Linux).
    CONTEO_HILOS.store(0, Ordering::SeqCst);
    let objetivo = 32u32;
    let mut handles = Vec::new();
    let mut fallo = None;
    for _ in 0..objetivo {
        match thread::spawn(hilo_cuenta, 0) {
            Ok(h) => handles.push(h),
            Err(e) => {
                fallo = Some(format!("spawn = {}", errno(e)));
                break;
            }
        }
    }
    for h in handles {
        let _ = h.join();
    }
    let n = CONTEO_HILOS.load(Ordering::SeqCst);
    anotar(
        &mut casos,
        Caso::nuevo(
            "moli/32-hilos-futex",
            "32",
            fallo.unwrap_or_else(|| format!("{n}")),
        ),
    );

    // 5. Espera sobre varios sockets (libcurl multi usa poll/epoll en Linux).
    const PUERTO_A: u16 = 9472;
    const PUERTO_B: u16 = 9473;
    let escucha = |puerto: u16| -> Result<i64, String> {
        let fd = sys::tcp_listen(puerto);
        if fd < 0 {
            return Err(format!("listen {puerto} = {}", errno(fd)));
        }
        Ok(fd)
    };
    let loopback = abi::SockAddr {
        addr: [127, 0, 0, 1],
        port: 0,
        _pad: 0,
    };
    match (escucha(PUERTO_A), escucha(PUERTO_B)) {
        (Ok(la), Ok(lb)) => {
            let mut addr_a = loopback;
            addr_a.port = PUERTO_A;
            let mut addr_b = loopback;
            addr_b.port = PUERTO_B;
            let ca = sys::tcp_connect(&addr_a, 5_000);
            let cb = sys::tcp_connect(&addr_b, 5_000);
            if ca < 0 || cb < 0 {
                anotar(
                    &mut casos,
                    Caso::nuevo(
                        "moli/espera-varios-sockets",
                        "dos conectados",
                        format!("connect A={} B={}", errno(ca), errno(cb)),
                    ),
                );
            } else {
                let _ = sys::tcp_accept(la as u64, 5_000);
                let _ = sys::tcp_accept(lb as u64, 5_000);
                let mut buf = [0u8; 1];
                let mut vistos = 0u32;
                for fd in [ca as u64, cb as u64] {
                    let n = sys::read_timeout(fd, &mut buf, 50);
                    if n == 0 || n > 0 {
                        vistos += 1;
                    }
                }
                sys::close(ca as u64);
                sys::close(cb as u64);
                anotar(
                    &mut casos,
                    Caso::observacion(
                        "moli/espera-varios-sockets",
                        format!(
                            "{vistos}/2 sockets sondeados con read_timeout; no hay poll/epoll/select en el ABI — curl multi necesitaría parche o syscall nueva"
                        ),
                    ),
                );
            }
            sys::close(la as u64);
            sys::close(lb as u64);
        }
        (a, b) => {
            anotar(
                &mut casos,
                Caso::nuevo(
                    "moli/espera-varios-sockets",
                    "listen ok",
                    format!("A={a:?} B={b:?}"),
                ),
            );
        }
    }

    casos
}
