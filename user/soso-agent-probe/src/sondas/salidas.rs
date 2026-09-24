//! Sonda 5 — lo que necesita un lanzador de herramientas (pase 5 de T33).
//!
//! Un agente no «ejecuta programas»: ejecuta **herramientas** y lee lo que
//! devuelven. Para eso hacen falta cuatro cosas que se suelen dar por
//! supuestas, y que aquí se miden por separado porque fallan por separado:
//!
//! - **stdout y stderr separados.** Si se mezclan, el diagnóstico de una
//!   herramienta contamina su salida y el agente parsea basura. `git` escribe
//!   avisos por stderr constantemente; `rg` también.
//! - **El código de salida**, que es cómo se sabe si funcionó.
//! - **El entorno**, que es como se le pasa configuración sin meterla en la
//!   línea de órdenes.
//! - **El directorio de trabajo**, porque una herramienta se lanza *en* un
//!   sitio.
//!
//! [T47](../../../../docs/self-improvement/T47-procesos-nativos.md) dejó el
//! adaptador y [T61](../../../../docs/self-improvement/T61-pipe-no-bloqueante.md)
//! la lectura con plazo; lo que falta es medir estas cuatro como **capacidad**,
//! desde el lado del padre.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use libsoso::{abi, println, sys};

use crate::{errno, Caso};

const YO: &str = "/bin/soso-agent-probe";
const MARCA_OUT: &str = "SALIDA-POR-STDOUT";
const MARCA_ERR: &str = "DIAGNOSTICO-POR-STDERR";

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

/// Drena una tubería hasta EOF, con plazo (T61).
fn drenar(fd: u64) -> String {
    let mut out = Vec::new();
    let mut buf = [0u8; 256];
    let mut vacios = 0;
    loop {
        let n = sys::read_timeout(fd, &mut buf, 200);
        if n > 0 {
            out.extend_from_slice(&buf[..n as usize]);
            vacios = 0;
        } else if n == 0 {
            break; // EOF: el escritor cerró
        } else {
            // -EAGAIN: todavía nada. Unas cuantas vueltas y se deja.
            vacios += 1;
            if vacios > 25 {
                break;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub fn ejecutar() -> Vec<Caso> {
    let mut casos: Vec<Caso> = Vec::new();

    // 1. **stdout y stderr por canales distintos.** El hijo escribe una marca
    //    en cada uno; el padre los lee por tuberías separadas. Lo que se
    //    comprueba no es sólo que cada marca llegue, sino que **no** llegue la
    //    otra: mezclarlos es el fallo que corrompe la salida de una
    //    herramienta sin que nadie se dé cuenta.
    let (r_out, w_out) = match sys::pipe() {
        Ok(p) => p,
        Err(e) => {
            anotar(
                &mut casos,
                Caso::nuevo("salidas/pipe", "dos tuberías", format!("pipe = {}", errno(e))),
            );
            return casos;
        }
    };
    let (r_err, w_err) = match sys::pipe() {
        Ok(p) => p,
        Err(e) => {
            anotar(
                &mut casos,
                Caso::nuevo("salidas/pipe", "dos tuberías", format!("pipe = {}", errno(e))),
            );
            return casos;
        }
    };
    let pid = sys::spawn_io_full(YO, &[YO, "gritar"], &[], [abi::FD_SERIAL_TTY, w_out, w_err, abi::FD_KERNEL_LOG]);
    // El padre cierra **sus** copias de los extremos de escritura: si no, la
    // tubería nunca da EOF y el drenaje se queda esperando a nadie.
    sys::close(w_out);
    sys::close(w_err);

    let (texto_out, texto_err) = if pid < 0 {
        (format!("spawn = {}", errno(pid)), String::new())
    } else {
        let o = drenar(r_out);
        let e = drenar(r_err);
        (o, e)
    };
    sys::close(r_out);
    sys::close(r_err);

    anotar(
        &mut casos,
        Caso::nuevo(
            "salidas/stdout-lleva-solo-lo-suyo",
            format!("contiene {MARCA_OUT}, no {MARCA_ERR}"),
            if texto_out.contains(MARCA_OUT) && !texto_out.contains(MARCA_ERR) {
                format!("contiene {MARCA_OUT}, no {MARCA_ERR}")
            } else {
                format!("{texto_out:?}")
            },
        ),
    );
    anotar(
        &mut casos,
        Caso::nuevo(
            "salidas/stderr-lleva-solo-lo-suyo",
            format!("contiene {MARCA_ERR}, no {MARCA_OUT}"),
            if texto_err.contains(MARCA_ERR) && !texto_err.contains(MARCA_OUT) {
                format!("contiene {MARCA_ERR}, no {MARCA_OUT}")
            } else {
                format!("{texto_err:?}")
            },
        ),
    );

    // 2. El código de salida del mismo hijo. Se espera después de drenar: al
    //    revés, un hijo que llene la tubería se bloquearía y el padre estaría
    //    esperando a un proceso que espera al padre.
    let observado = if pid < 0 {
        String::from("no se lanzó")
    } else {
        match sys::wait() {
            Ok((p, c)) if p == pid as u64 => format!("{c}"),
            Ok((p, c)) => format!("otro hijo: pid {p} código {c}"),
            Err(e) => format!("wait = {}", errno(e)),
        }
    };
    anotar(
        &mut casos,
        Caso::nuevo("salidas/codigo-de-salida", "3", observado),
    );

    // 3. **El entorno.** Es como se le pasa configuración a una herramienta
    //    sin meterla en la línea de órdenes — donde, además, quedaría a la
    //    vista de cualquiera que liste procesos.
    let (r, w) = sys::pipe().unwrap_or((0, 0));
    let pid = sys::spawn_io_full(
        YO,
        &[YO, "eco-entorno"],
        &["SOSO_PROBE_VAR=valor con espacios"],
        [abi::FD_SERIAL_TTY, w, abi::FD_SERIAL_TTY, abi::FD_KERNEL_LOG],
    );
    sys::close(w);
    let visto = if pid < 0 {
        format!("spawn = {}", errno(pid))
    } else {
        let t = drenar(r);
        let _ = sys::wait();
        t.trim().into()
    };
    sys::close(r);
    anotar(
        &mut casos,
        Caso::nuevo(
            "entorno/variable-heredada",
            "valor con espacios",
            visto,
        ),
    );

    // 4. **El directorio de trabajo.** No hay parámetro de cwd en el spawn: el
    //    hijo hereda el del padre, así que lanzar una herramienta «en» un sitio
    //    obliga a hacer `chdir` antes — y el `chdir` es del **proceso**, no de
    //    la llamada. Se mide la herencia, y la consecuencia queda dicha.
    let antes = {
        let mut b = [0u8; 256];
        let rc = sys::getcwd(&mut b);
        if rc < 0 {
            String::from("?")
        } else {
            // `getcwd` devuelve el puntero al búfer, no una longitud: la
            // cadena acaba en NUL (lo aprendió T47 por las malas).
            let fin = b.iter().position(|c| *c == 0).unwrap_or(b.len());
            String::from_utf8_lossy(&b[..fin]).into_owned()
        }
    };
    sys::mkdir("/tmp/t33-cwd");
    sys::chdir("/tmp/t33-cwd");
    let (r, w) = sys::pipe().unwrap_or((0, 0));
    let pid = sys::spawn_io_full(
        YO,
        &[YO, "eco-cwd"],
        &[],
        [abi::FD_SERIAL_TTY, w, abi::FD_SERIAL_TTY, abi::FD_KERNEL_LOG],
    );
    sys::close(w);
    let visto = if pid < 0 {
        format!("spawn = {}", errno(pid))
    } else {
        let t = drenar(r);
        let _ = sys::wait();
        t.trim().into()
    };
    sys::close(r);
    sys::chdir(&antes);
    anotar(
        &mut casos,
        Caso::nuevo("cwd/el-hijo-hereda-el-del-padre", "/tmp/t33-cwd", visto),
    );

    casos
}

/// Subcomando auxiliar: una marca por stdout y otra por stderr, y salir con 3.
pub fn gritar() -> u8 {
    // `println!` va a stdout; para stderr hace falta escribir al fd 2 a mano,
    // porque libsoso no tiene un `eprintln!`.
    println!("{MARCA_OUT}");
    let msg = format!("{MARCA_ERR}\n");
    let mut n = 0;
    while n < msg.len() {
        let w = sys::write(2, &msg.as_bytes()[n..]);
        if w <= 0 {
            break;
        }
        n += w as usize;
    }
    3
}

/// Subcomando auxiliar: imprime una variable de entorno.
pub fn eco_entorno() -> u8 {
    let mut buf = [0u8; 256];
    let rc = sys::getenv("SOSO_PROBE_VAR", &mut buf);
    if rc < 0 {
        println!("getenv = {}", errno(rc));
        return 1;
    }
    let fin = buf.iter().position(|c| *c == 0).unwrap_or(rc.max(0) as usize);
    println!("{}", String::from_utf8_lossy(&buf[..fin.min(rc.max(0) as usize)]));
    0
}

/// Subcomando auxiliar: imprime el directorio de trabajo.
pub fn eco_cwd() -> u8 {
    let mut b = [0u8; 256];
    let rc = sys::getcwd(&mut b);
    if rc < 0 {
        println!("getcwd = {}", errno(rc));
        return 1;
    }
    let fin = b.iter().position(|c| *c == 0).unwrap_or(b.len());
    println!("{}", String::from_utf8_lossy(&b[..fin]));
    0
}
