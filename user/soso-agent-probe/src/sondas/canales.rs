//! Sonda 7 — tuberías con EOF y TCP con cierre y reconexión (pase 7 de T33).
//!
//! Las dos últimas capacidades, y las dos son de lo mismo: **saber cuándo se
//! acabó**. Un agente lee la salida de una herramienta por una tubería y habla
//! con el modelo por un socket; en los dos casos, confundir «todavía no hay
//! datos» con «ya no va a haber más» es la diferencia entre esperar de más y
//! truncar la respuesta.
//!
//! [T61](../../../../docs/self-improvement/T61-pipe-no-bloqueante.md) separó
//! los tres desenlaces en la syscall y
//! [T48](../../../../docs/self-improvement/T48-reloj-red.md) en el transporte.
//! Aquí se mide como capacidad del agente, con los dos casos que más se dan y
//! peor se detectan: una salida **grande** que no cabe de una vez, y un
//! escritor que **muere** sin cerrar.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use libsoso::{abi, println, sys};

use crate::{errno, Caso};

const YO: &str = "/bin/soso-agent-probe";
/// 64 KiB: dieciséis veces el búfer de la tubería, así que el escritor se
/// bloquea varias veces y el lector tiene que volver muchas.
const GRANDE: usize = 64 * 1024;
const PUERTO: u16 = 9471;

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

/// Byte esperado en la posición `i` del chorro grande. Un patrón que cambia
/// con la posición: si algo se reordena o se pierde un trozo, el contenido lo
/// dice y no sólo la longitud.
fn patron(i: usize) -> u8 {
    (i % 251) as u8
}

pub fn ejecutar() -> Vec<Caso> {
    let mut casos: Vec<Caso> = Vec::new();

    // 1. Una salida **grande** por la tubería. No cabe de una vez, así que el
    //    hijo se bloquea escribiendo y el lector tiene que volver muchas
    //    veces. Se comprueba longitud **y** contenido: una tubería que pierde
    //    un trozo del medio da la longitud mal, pero una que reordena la da
    //    bien.
    let (r, w) = match sys::pipe() {
        Ok(p) => p,
        Err(e) => {
            anotar(
                &mut casos,
                Caso::nuevo("canales/pipe", "creada", format!("pipe = {}", errno(e))),
            );
            return casos;
        }
    };
    let pid = sys::spawn_io_full(
        YO,
        &[YO, "chorro"],
        &[],
        [abi::FD_SERIAL_TTY, w, abi::FD_SERIAL_TTY, abi::FD_KERNEL_LOG],
    );
    sys::close(w);

    let mut recibido: Vec<u8> = Vec::with_capacity(GRANDE);
    let mut eof = false;
    let mut esperas = 0;
    if pid >= 0 {
        let mut buf = [0u8; 1024];
        loop {
            let n = sys::read_timeout(r, &mut buf, 500);
            if n > 0 {
                recibido.extend_from_slice(&buf[..n as usize]);
            } else if n == 0 {
                eof = true;
                break;
            } else {
                esperas += 1;
                if esperas > 60 {
                    break;
                }
            }
        }
        let _ = sys::wait();
    }
    sys::close(r);

    let primer_fallo = recibido
        .iter()
        .enumerate()
        .find(|(i, b)| **b != patron(*i))
        .map(|(i, b)| format!("en {i} llegó {b} y tocaba {}", patron(i)));
    anotar(
        &mut casos,
        Caso::nuevo(
            "canales/salida-grande-intacta",
            format!("{GRANDE} bytes en orden"),
            match primer_fallo {
                Some(d) => d,
                None if recibido.len() == GRANDE => format!("{GRANDE} bytes en orden"),
                None => format!("{} bytes en orden, faltan {}", recibido.len(), GRANDE - recibido.len()),
            },
        ),
    );

    // 2. **EOF cuando el escritor muere sin cerrar.** Es lo que pasa cuando una
    //    herramienta revienta: nadie llama a `close`, el proceso simplemente
    //    deja de existir. Si el lector no viera EOF ahí, se quedaría esperando
    //    para siempre a un programa que ya no está.
    anotar(
        &mut casos,
        Caso::nuevo(
            "canales/eof-cuando-el-escritor-muere",
            "EOF",
            if eof {
                String::from("EOF")
            } else {
                format!("sin EOF tras {esperas} esperas")
            },
        ),
    );

    // 3. TCP contra sí mismo: escuchar, conectar, hablar y cerrar. El servidor
    //    va en un hijo porque `accept` bloquea.
    let servidor = sys::spawn_io_full(
        YO,
        &[YO, "eco-tcp"],
        &[],
        [abi::FD_SERIAL_TTY, abi::FD_SERIAL_TTY, abi::FD_SERIAL_TTY, abi::FD_KERNEL_LOG],
    );
    // Un respiro para que llegue a `tcp_listen`: conectar antes daría
    // ECONNREFUSED y mediría la carrera, no el socket.
    sys::sleep_ms(400);

    let addr = abi::SockAddr {
        addr: [127, 0, 0, 1],
        port: PUERTO,
        _pad: 0,
    };
    let mut vueltas: Vec<String> = Vec::new();
    // **Dos conexiones seguidas al mismo destino.** Con una sola no se ve el
    // fallo que el puerto local por índice de ranura producía: la segunda
    // moría en TIME_WAIT.
    for i in 0..2 {
        let fd = sys::tcp_connect(&addr, 5_000);
        if fd < 0 {
            vueltas.push(format!("conexión {i}: {}", errno(fd)));
            continue;
        }
        let fd = fd as u64;
        let msg = format!("hola-{i}");
        let w = sys::write(fd, msg.as_bytes());
        let mut buf = [0u8; 64];
        let n = sys::read_timeout(fd, &mut buf, 5_000);
        sys::close(fd);
        if w < 0 || n <= 0 {
            vueltas.push(format!("conexión {i}: write={} read={}", errno(w), errno(n)));
        } else {
            vueltas.push(String::from_utf8_lossy(&buf[..n as usize]).into_owned());
        }
    }
    if servidor >= 0 {
        let _ = sys::kill(servidor, abi::SIGKILL);
        let _ = sys::wait();
    }
    anotar(
        &mut casos,
        Caso::nuevo(
            "canales/tcp-dos-conexiones-al-mismo-destino",
            "hola-0 y hola-1",
            format!("{} y {}", vueltas.first().map(|s| s.as_str()).unwrap_or("?"),
                    vueltas.get(1).map(|s| s.as_str()).unwrap_or("?")),
        ),
    );

    casos
}

/// Subcomando auxiliar: escupe [`GRANDE`] bytes por stdout y **sale sin
/// cerrar**, para que el lector tenga que ver el EOF por la muerte del hijo.
pub fn chorro() -> u8 {
    let datos: Vec<u8> = (0..GRANDE).map(patron).collect();
    let mut n = 0;
    while n < datos.len() {
        let w = sys::write(1, &datos[n..]);
        if w <= 0 {
            break;
        }
        n += w as usize;
    }
    0
}

/// Subcomando auxiliar: un eco TCP que atiende dos conexiones y se va.
pub fn eco_tcp() -> u8 {
    let escucha = sys::tcp_listen(PUERTO);
    if escucha < 0 {
        println!("eco-tcp: listen = {}", errno(escucha));
        return 1;
    }
    for _ in 0..2 {
        let c = sys::tcp_accept(escucha as u64, 15_000);
        if c < 0 {
            println!("eco-tcp: accept = {}", errno(c));
            break;
        }
        let c = c as u64;
        let mut buf = [0u8; 256];
        let n = sys::read_timeout(c, &mut buf, 5_000);
        if n > 0 {
            let mut e = 0;
            while e < n as usize {
                let w = sys::write(c, &buf[e..n as usize]);
                if w <= 0 {
                    break;
                }
                e += w as usize;
            }
        }
        sys::close(c);
    }
    sys::close(escucha as u64);
    0
}
