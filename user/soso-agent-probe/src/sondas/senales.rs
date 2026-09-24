//! Sonda 4 — señales, medidas por lo que un runtime necesita de ellas
//! (pase 4 de T33).
//!
//! [T32](../../../../docs/self-improvement/T32-opencode-inventario.md) anotó
//! que soso tiene `SYS_KILL` y números de señal pero **ninguna forma de
//! instalar un manejador**. Medir «¿se puede manejar una señal?» daría un no
//! que ya sabemos leyendo el ABI, y no diría nada útil.
//!
//! Lo que sí hay que medir es lo que un agente usa de verdad:
//!
//! - **Matar un subproceso desbocado**, que es el fondo de cualquier timeout
//!   de herramienta. OpenCode lanza `bash`, `git` o `rg` con plazo; cuando
//!   vence, alguien tiene que poder cortarlos.
//! - **Distinguir «murió por señal» de «salió con código»**. Si las dos cosas
//!   llegaran iguales al padre, un timeout sería indistinguible de un fallo
//!   del programa, y el agente aprendería la lección equivocada.
//! - **Escribir en una tubería cerrada**. En POSIX eso es `SIGPIPE` y mata al
//!   escritor salvo que se ignore. Sin manejadores, la pregunta es qué hace
//!   soso: devolver un error es lo bueno; morir en silencio, lo malo.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use libsoso::{abi, println, sys};

use crate::{errno, Caso};

const YO: &str = "/bin/soso-agent-probe";

fn anotar(casos: &mut Vec<Caso>, c: Caso) {
    if c.paso {
        println!("probe: {} ok ({})", c.id, c.observado);
    } else {
        println!(
            "probe: {} FALLO esperado={:?} observado={:?}",
            c.id, c.esperado, c.observado
        );
    }
    casos.push(c);
}

/// Lanza este mismo binario con un subcomando, heredando la consola.
fn lanzar(argv: &[&str]) -> i64 {
    sys::spawn_io_ex(
        YO,
        argv,
        &[],
        abi::FD_SERIAL_TTY,
        abi::FD_SERIAL_TTY,
        abi::FD_SERIAL_TTY,
    )
}

/// Espera a un pid concreto; los hijos ajenos que salgan antes se recogen y se
/// descartan, para no confundir el resultado de otro con el que se busca.
///
/// **No lleva plazo, y no es un descuido:** `wait()` bloquea y no hay forma de
/// sondearlo, así que un tope aquí sería un adorno que no vence nunca. Si un
/// hijo no muere, esto se queda esperando y quien corta es el plazo del paso
/// de la suite — que entonces informa «la sonda no llegó al informe final»,
/// que es exactamente lo que habría pasado. Fingir un timeout propio sólo
/// cambiaría el mensaje, no el desenlace.
fn esperar_a(pid: u64) -> Result<u8, String> {
    loop {
        match sys::wait() {
            Ok((p, code)) if p == pid => return Ok(code),
            Ok(_) => continue,
            Err(e) if e == -(abi::ECHILD as i64) => {
                return Err(String::from("ECHILD: no quedaban hijos"))
            }
            Err(e) => return Err(format!("wait = {}", errno(e))),
        }
    }
}

pub fn ejecutar() -> Vec<Caso> {
    let mut casos: Vec<Caso> = Vec::new();

    // 1. Salida normal: el código del hijo llega al padre tal cual. Es la
    //    referencia contra la que se compara la muerte por señal; sin esto no
    //    se puede afirmar que 143 signifique algo.
    let pid = lanzar(&[YO, "salir", "7"]);
    let observado = if pid < 0 {
        format!("spawn = {}", errno(pid))
    } else {
        match esperar_a(pid as u64) {
            Ok(c) => format!("{c}"),
            Err(e) => e,
        }
    };
    anotar(
        &mut casos,
        Caso::nuevo("senales/salida-normal", "7", observado),
    );

    // 2. **El timeout de herramienta.** Un hijo que duerme mucho más de lo que
    //    se le va a dar, y un `kill` que tiene que cortarlo.
    let pid = lanzar(&[YO, "dormir", "60000"]);
    let observado = if pid < 0 {
        format!("spawn = {}", errno(pid))
    } else {
        let pid = pid as u64;
        // Un respiro para que el hijo llegue a dormirse: matar a un proceso
        // que aún no ha arrancado mediría otra cosa.
        sys::sleep_ms(300);
        let rc = sys::kill(pid as i64, abi::SIGTERM);
        if rc < 0 {
            format!("kill = {}", errno(rc))
        } else {
            match esperar_a(pid) {
                Ok(c) => format!("{c}"),
                Err(e) => e,
            }
        }
    };
    anotar(
        &mut casos,
        Caso::nuevo(
            "senales/sigterm-mata-y-se-distingue",
            format!("{}", abi::exit_by_signal(abi::SIGTERM as u8)),
            observado,
        ),
    );

    // 3. Igual con SIGKILL: un runtime escala cuando SIGTERM no basta, y el
    //    código tiene que decir cuál de las dos fue.
    let pid = lanzar(&[YO, "dormir", "60000"]);
    let observado = if pid < 0 {
        format!("spawn = {}", errno(pid))
    } else {
        let pid = pid as u64;
        sys::sleep_ms(300);
        let rc = sys::kill(pid as i64, abi::SIGKILL);
        if rc < 0 {
            format!("kill = {}", errno(rc))
        } else {
            match esperar_a(pid) {
                Ok(c) => format!("{c}"),
                Err(e) => e,
            }
        }
    };
    anotar(
        &mut casos,
        Caso::nuevo(
            "senales/sigkill-se-distingue-de-sigterm",
            format!("{}", abi::exit_by_signal(abi::SIGKILL as u8)),
            observado,
        ),
    );

    // 4. `kill(pid, 0)` — el sondeo. Un supervisor lo usa para saber si un hijo
    //    sigue vivo **sin** tocarlo. Vivo: 0. Inexistente: ESRCH.
    let pid = lanzar(&[YO, "dormir", "3000"]);
    let observado = if pid < 0 {
        format!("spawn = {}", errno(pid))
    } else {
        let pid = pid as u64;
        sys::sleep_ms(200);
        let vivo = sys::kill(pid as i64, abi::SIGPROBE);
        let _ = sys::kill(pid as i64, abi::SIGKILL);
        let _ = esperar_a(pid);
        // Un pid que no existe: uno muy alto que nadie va a ocupar.
        let muerto = sys::kill(999_999, abi::SIGPROBE);
        // Lo que importa es que **se distingan**, no el valor exacto: soso
        // devuelve el número de procesos señalados y POSIX devuelve 0. Un
        // supervisor sólo necesita «>0 = vivo» frente a «error = no está».
        format!(
            "vivo={} inexistente={}",
            if vivo > 0 { String::from("sí") } else { errno(vivo) },
            if muerto < 0 { "no" } else { "¡sí!" }
        )
    };
    anotar(
        &mut casos,
        Caso::nuevo(
            "senales/sondeo-distingue-vivo-de-inexistente",
            "vivo=sí inexistente=no",
            observado,
        ),
    );

    // 4b. Y el valor exacto que devuelve `kill`, aparte: soso da el **número
    //     de procesos señalados** y POSIX da 0. No es un defecto —la
    //     capacidad está—, pero cualquier capa de compatibilidad tiene que
    //     traducirlo, y eso sólo se sabe si queda medido.
    let pid = lanzar(&[YO, "dormir", "2000"]);
    let observado = if pid < 0 {
        format!("spawn = {}", errno(pid))
    } else {
        let pid = pid as u64;
        sys::sleep_ms(200);
        let n = sys::kill(pid as i64, abi::SIGPROBE);
        let _ = sys::kill(pid as i64, abi::SIGKILL);
        let _ = esperar_a(pid);
        format!("{n}")
    };
    anotar(
        &mut casos,
        Caso::nuevo(
            "senales/kill-devuelve-cuantos-no-cero",
            "1",
            observado,
        ),
    );

    // 5. Escribir en una tubería cuyo lector ya cerró. En POSIX esto levanta
    //    `SIGPIPE` y mata al escritor si no lo ignora; sin manejadores, lo
    //    único aceptable es un error. Morir aquí dejaría al agente sin saber
    //    por qué desapareció su herramienta.
    let observado = match sys::pipe() {
        Err(e) => format!("pipe = {}", errno(e)),
        Ok((r, w)) => {
            sys::close(r);
            println!("probe: senales/tuberia-sin-lector escribiendo (si muere aquí, es SIGPIPE)");
            let rc = sys::write(w, b"nadie escucha");
            sys::close(w);
            if rc < 0 {
                format!("error {}", errno(rc))
            } else {
                format!("aceptó {rc} bytes")
            }
        }
    };
    anotar(
        &mut casos,
        Caso::nuevo(
            "senales/tuberia-sin-lector",
            "error la tubería no tiene lector (-32)",
            observado,
        ),
    );

    casos
}

/// Subcomando auxiliar: dormir y salir con 0.
pub fn dormir(ms: &str) -> u8 {
    let ms: u64 = ms.bytes().fold(0u64, |a, b| {
        if b.is_ascii_digit() {
            a.saturating_mul(10).saturating_add((b - b'0') as u64)
        } else {
            a
        }
    });
    sys::sleep_ms(ms);
    println!("probe: el hijo terminó de dormir {ms} ms sin que lo mataran");
    0
}

/// Subcomando auxiliar: salir con el código pedido.
pub fn salir(codigo: &str) -> u8 {
    codigo.bytes().fold(0u8, |a, b| {
        if b.is_ascii_digit() {
            a.saturating_mul(10).saturating_add(b - b'0')
        } else {
            a
        }
    })
}
