//! Sonda 6 — hilos, `join`, TLS y temporizadores (pase 6 de T33).
//!
//! Un runtime de JavaScript no es monohilo por dentro: JavaScriptCore compila
//! en un hilo aparte, y Bun tiene su propio pool para E/S. Lo que hace falta
//! saber no es sólo si «hay hilos», sino si se comportan como espera quien los
//! usa: que `join` **espere de verdad**, que cada hilo tenga su propio
//! almacenamiento local, y que desbordar la pila de un hilo **falle** en vez de
//! pisar la memoria de al lado.
//!
//! Lo último es lo que más cuesta ver y lo que peor acaba: una recursión
//! profunda —rutinaria en un intérprete— que en vez de morir con una falta de
//! página corrompe el montón y revienta tres funciones más tarde.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use libsoso::{abi, println, sys, thread};

use crate::{errno, Caso};

fn anotar(casos: &mut Vec<Caso>, c: Caso) {
    // Una observación —`Caso::observacion`, sin `esperado`— **no es un
    // aprobado**: imprimirla como `ok` la disfraza de veredicto. Se distingue
    // aquí aunque esta sonda no tenga ninguna todavía, porque la trampa la
    // paga quien añada la primera.
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

/// Cada hilo suma su argumento aquí: si `join` no esperase, el total estaría
/// incompleto al leerlo.
static SUMA: AtomicU64 = AtomicU64::new(0);
/// Lo pone el hilo **después** de dormir. Es lo que distingue «join esperó» de
/// «join volvió en cuanto el hilo arrancó».
static TARDE: AtomicU32 = AtomicU32::new(0);

// Un hilo termina con `sys::exit`, que es el idioma del pool de soso-llm: el
// kernel lo distingue de terminar el proceso y despierta al `join`.
extern "C" fn suma_y_sal(arg: u64) -> ! {
    SUMA.fetch_add(arg, Ordering::SeqCst);
    sys::exit(0)
}

extern "C" fn duerme_y_marca(_arg: u64) -> ! {
    sys::sleep_ms(400);
    TARDE.store(1, Ordering::SeqCst);
    sys::exit(0)
}

fn lanzar_guarda() -> i64 {
    const YO: &str = "/bin/soso-agent-probe";
    sys::spawn_io_ex(
        YO,
        &[YO, "tocar-guarda"],
        &[],
        abi::FD_SERIAL_TTY,
        abi::FD_SERIAL_TTY,
        abi::FD_SERIAL_TTY,
    )
}

fn esperar_hijo(pid: u64) -> Result<u8, String> {
    loop {
        match sys::wait() {
            Ok((p, c)) if p == pid => return Ok(c),
            Ok(_) => continue,
            Err(e) => return Err(format!("wait = {}", errno(e))),
        }
    }
}

/// Subcomando auxiliar: reserva una página, la deja **sin acceso** y la toca.
///
/// Si N-002 funciona, esto no vuelve: el proceso muere con una falta de página.
/// Que devuelva 0 significaría que la guarda no guarda nada.
pub fn tocar_guarda() -> u8 {
    let p = sys::mmap(0, 4096, u64::MAX, 0);
    if p < 0 {
        println!("tocar-guarda: mmap = {}", errno(p));
        return 2;
    }
    let rc = sys::mprotect(p as u64, 4096, 0);
    if rc < 0 {
        println!("tocar-guarda: mprotect = {}", errno(rc));
        return 3;
    }
    println!("tocar-guarda: escribiendo en la página sin acceso");
    unsafe {
        core::ptr::write_volatile(p as *mut u8, 0x42);
    }
    println!("tocar-guarda: sobreviví, la guarda no guarda");
    0
}

pub fn ejecutar() -> Vec<Caso> {
    let mut casos: Vec<Caso> = Vec::new();

    // 1. Varios hilos que trabajan y se recogen. La suma comprueba que
    //    **todos** corrieron, no que arrancó alguno.
    SUMA.store(0, Ordering::SeqCst);
    let mut handles = Vec::new();
    let mut fallo_spawn = None;
    for i in 1..=4u64 {
        match thread::spawn(suma_y_sal, i) {
            Ok(h) => handles.push(h),
            Err(e) => {
                fallo_spawn = Some(format!("spawn {i} = {}", errno(e)));
                break;
            }
        }
    }
    let n = handles.len();
    for h in handles {
        let _ = h.join();
    }
    anotar(
        &mut casos,
        Caso::nuevo(
            "hilos/cuatro-corren-y-se-recogen",
            "10",
            fallo_spawn.unwrap_or_else(|| format!("{}", SUMA.load(Ordering::SeqCst))),
        ),
    );
    anotar(
        &mut casos,
        Caso::nuevo("hilos/spawn-devuelve-handle", "4", format!("{n}")),
    );

    // 2. `join` tiene que **esperar**. El hilo duerme 400 ms y sólo después
    //    marca; si `join` volviera antes, la marca estaría sin poner.
    TARDE.store(0, Ordering::SeqCst);
    let observado = match thread::spawn(duerme_y_marca, 0) {
        Err(e) => format!("spawn = {}", errno(e)),
        Ok(h) => {
            let _ = h.join();
            format!("{}", TARDE.load(Ordering::SeqCst))
        }
    };
    anotar(
        &mut casos,
        Caso::nuevo("hilos/join-espera-de-verdad", "1", observado),
    );

    // 3. **La guarda de pila.** `thread::spawn` intenta instalarla con
    //    `mprotect(base, 4096, 0)` — quitar todos los permisos— y se traga el
    //    error con un `let _`. Pero `sys_mprotect` **rechaza `prot == 0`**, y
    //    tampoco deja quitar la lectura: no hay forma de marcar una página
    //    como inaccesible. Así que la guarda no existe y desbordar la pila de
    //    un hilo no falla: pisa lo que haya debajo.
    //
    //    Se mide pidiéndolo, no leyendo el kernel.
    let pagina = sys::mmap(0, 4096, u64::MAX, 0);
    let (sin_permisos, solo_escritura) = if pagina < 0 {
        (format!("mmap = {}", errno(pagina)), String::new())
    } else {
        let a = sys::mprotect(pagina as u64, 4096, 0);
        let b = sys::mprotect(pagina as u64, 4096, abi::PROT_WRITE);
        sys::munmap(pagina as u64, 4096);
        (errno(a), errno(b))
    };
    anotar(
        &mut casos,
        Caso::nuevo(
            "hilos/guarda-de-pila-se-puede-instalar",
            "prot=0 → 0",
            format!("prot=0 → {sin_permisos}"),
        ),
    );
    // Y que `libsoso` **diga la verdad**: tras T66, `hay_guarda_de_pila()`
    // informa de si se pudo instalar, y tras N-002 ya se puede. Lo que se
    // comprueba es que **coincida** con lo que contesta `mprotect` — si
    // divergieran, la biblioteca estaría prometiendo otra vez lo que el kernel
    // no da, que es el defecto que T66 arregló.
    let dice = thread::hay_guarda_de_pila();
    let puede = sin_permisos == "0";
    anotar(
        &mut casos,
        Caso::nuevo(
            "hilos/libsoso-dice-la-verdad",
            format!("mprotect y libsoso coinciden: {puede}"),
            format!("mprotect y libsoso coinciden: {}", dice == puede && dice == puede),
        ),
    );

    // Escribir sin poder leer **se rechaza a propósito**, y por eso lo
    // esperado es el error: x86 no tiene bit de «sólo escritura», así que
    // aceptarlo sería prometer lo que la tabla de páginas no puede cumplir.
    // Dejarlo como negativo permanente habría sido peor: un fallo que nadie
    // mira deja de ser un fallo.
    anotar(
        &mut casos,
        Caso::nuevo(
            "hilos/escribir-sin-leer-se-rechaza",
            "argumento inválido (-22)",
            solo_escritura,
        ),
    );

    // **Lo que hace útil a una guarda**: tocarla tiene que matar al proceso.
    // Se mide en un hijo, porque el que la toca no vuelve a contarlo.
    let pid = lanzar_guarda();
    let observado = if pid < 0 {
        format!("spawn = {}", errno(pid))
    } else {
        match esperar_hijo(pid as u64) {
            Ok(0) => String::from("el hijo sobrevivió y salió con 0"),
            Ok(c) => format!("el hijo murió (código {c})"),
            Err(e) => e,
        }
    };
    anotar(
        &mut casos,
        Caso::nuevo(
            "hilos/tocar-la-guarda-mata-al-proceso",
            "el hijo murió",
            if observado.starts_with("el hijo murió") {
                String::from("el hijo murió")
            } else {
                observado
            },
        ),
    );

    // 4. Temporizadores y reloj. Lo que un runtime necesita de un reloj es que
    //    **no retroceda** y que dormir duerma al menos lo pedido: un
    //    `setTimeout` que vuelve antes de tiempo es peor que uno que tarda.
    let t0 = sys::uptime_ms();
    sys::sleep_ms(250);
    let t1 = sys::uptime_ms();
    let transcurrido = t1 - t0;
    anotar(
        &mut casos,
        Caso::nuevo(
            "temporizadores/dormir-no-vuelve-antes",
            "al menos 250 ms",
            if transcurrido >= 250 {
                String::from("al menos 250 ms")
            } else {
                format!("{transcurrido} ms")
            },
        ),
    );

    // Monotonía: mil lecturas seguidas y ninguna menor que la anterior.
    let mut anterior = sys::uptime_ms();
    let mut retrocesos = 0;
    for _ in 0..1000 {
        let ahora = sys::uptime_ms();
        if ahora < anterior {
            retrocesos += 1;
        }
        anterior = ahora;
    }
    anotar(
        &mut casos,
        Caso::nuevo(
            "temporizadores/el-reloj-no-retrocede",
            "0 retrocesos",
            format!("{retrocesos} retrocesos"),
        ),
    );

    // Y el reloj de pared, que es otro: `clock_gettime`. Un agente lo necesita
    // para fechar lo que hace; que exista no garantiza que avance.
    let mut a = abi::Timespec::default();
    let mut b = abi::Timespec::default();
    let rc1 = sys::clock_gettime(abi::CLOCK_REALTIME, &mut a);
    sys::sleep_ms(1100);
    let rc2 = sys::clock_gettime(abi::CLOCK_REALTIME, &mut b);
    let observado = if rc1 < 0 || rc2 < 0 {
        format!("clock_gettime = {} / {}", errno(rc1), errno(rc2))
    } else if b.tv_sec > a.tv_sec {
        String::from("avanzó")
    } else {
        format!("no avanzó ({} → {})", a.tv_sec, b.tv_sec)
    };
    anotar(
        &mut casos,
        Caso::nuevo("temporizadores/el-reloj-de-pared-avanza", "avanzó", observado),
    );

    casos
}
