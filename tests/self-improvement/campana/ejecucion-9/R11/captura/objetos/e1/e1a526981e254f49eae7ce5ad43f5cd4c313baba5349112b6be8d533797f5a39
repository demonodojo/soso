//! Prototipo de servidor SSH con sunset 0.5 (API event-driven no-async).
//!
//! Escucha en 127.0.0.1:2223 y atiende UNA sesión: autentica (sin
//! comprobar credenciales, como hará soso en monousuario), abre un canal
//! de shell y corre una mini-shell de eco por línea con un par de
//! comandos. El objetivo es ejercitar exactamente la secuencia de eventos
//! que necesitará el kernel:
//!
//!   Hostkeys → FirstAuth/PasswordAuth/PubkeyAuth → Authenticated →
//!   OpenSession → SessionPty? → SessionShell → (datos del canal) → Defunct
//!
//! Verificado con: `ssh -p 2223 test@localhost`.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};

use sunset::event::{Event, ServEvent};
use sunset::{ChanData, Runner, Server, SignKey};

fn main() {
    env_logger::init();
    let key = SignKey::generate(sunset::KeyType::Ed25519, None).expect("generar host key");

    let listener = TcpListener::bind("127.0.0.1:2223").expect("bind :2223");
    eprintln!("ssh-proto: escuchando en :2223 (ssh -p 2223 test@localhost)");
    for stream in listener.incoming() {
        match stream {
            Ok(s) => {
                eprintln!("--- conexión de {:?}", s.peer_addr());
                if let Err(e) = sesion(s, &key) {
                    eprintln!("--- sesión terminó: {e:?}");
                } else {
                    eprintln!("--- sesión cerrada limpiamente");
                }
            }
            Err(e) => println!("accept: {e}"),
        }
    }
}

/// Estado de la mini-shell de línea que corre sobre el canal.
struct Shell {
    linea: Vec<u8>,
    saludado: bool,
    salir: bool,
}

impl Shell {
    fn new() -> Self {
        Self { linea: Vec::new(), saludado: false, salir: false }
    }

    /// Procesa bytes recibidos del cliente, devolviendo lo que hay que
    /// escribir de vuelta al canal (eco + salida de comandos).
    fn feed(&mut self, datos: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        for &b in datos {
            match b {
                b'\r' | b'\n' => {
                    out.extend_from_slice(b"\r\n");
                    let cmd = String::from_utf8_lossy(&self.linea).trim().to_string();
                    self.linea.clear();
                    self.ejecutar(&cmd, &mut out);
                    if self.salir {
                        return out;
                    }
                    out.extend_from_slice(b"proto$ ");
                }
                0x7f | 0x08 => {
                    if self.linea.pop().is_some() {
                        out.extend_from_slice(b"\x08 \x08");
                    }
                }
                0x03 => {
                    // Ctrl-C: cancela la línea.
                    out.extend_from_slice(b"^C\r\n proto$ ");
                    self.linea.clear();
                }
                c => {
                    self.linea.push(c);
                    out.push(c); // eco
                }
            }
        }
        out
    }

    fn ejecutar(&mut self, cmd: &str, out: &mut Vec<u8>) {
        match cmd {
            "" => {}
            "exit" | "quit" => {
                out.extend_from_slice(b"adios\r\n");
                self.salir = true;
            }
            "help" => out.extend_from_slice(b"comandos: help, echo <x>, exit\r\n"),
            _ if cmd.starts_with("echo ") => {
                out.extend_from_slice(cmd[5..].as_bytes());
                out.extend_from_slice(b"\r\n");
            }
            _ => {
                out.extend_from_slice(b"no such command: ");
                out.extend_from_slice(cmd.as_bytes());
                out.extend_from_slice(b"\r\n");
            }
        }
    }

    fn saludo(&mut self) -> &'static [u8] {
        self.saludado = true;
        b"\r\n== sunset proto shell ==\r\nescribe 'help'\r\nproto$ "
    }
}

fn sesion(mut stream: TcpStream, key: &SignKey) -> Result<(), sunset::Error> {
    stream.set_nodelay(true).ok();
    // No bloqueante: intercalamos lectura de socket y progreso del runner.
    stream.set_nonblocking(true).ok();

    let mut runner = Runner::<Server>::new_server_owned();
    let mut shell = Shell::new();
    let mut chan: Option<sunset::ChanHandle> = None;
    let mut autenticado = false;

    // Buffer de entrada persistente: [pos..have) son bytes leídos del
    // socket aún no aceptados por el runner. `input()` puede aceptar solo
    // parte (hasta un límite de paquete) y devolver 0 hasta que
    // `progress()` lo consuma, así que NO se puede alimentar en un bucle
    // cerrado: se alimenta lo que acepte y el resto espera a la próxima
    // vuelta, tras drenar eventos.
    let mut netbuf = [0u8; 4096];
    let mut pos = 0usize;
    let mut have = 0usize;
    loop {
        let mut hubo_io = false;

        // 1a) Rellenar el buffer solo si está vacío y el runner quiere más.
        if pos == have && runner.is_input_ready() {
            pos = 0;
            have = 0;
            match stream.read(&mut netbuf) {
                Ok(0) => {
                    runner.close_input();
                }
                Ok(n) => {
                    hubo_io = true;
                    have = n;
                }
                Err(e)
                    if e.kind() == std::io::ErrorKind::WouldBlock
                        || e.kind() == std::io::ErrorKind::TimedOut => {}
                Err(e) => return Err(sunset_io_err(e)),
            }
        }

        // 1b) Alimentar lo que el runner acepte ahora (puede ser 0).
        if pos < have && runner.is_input_ready() {
            let c = runner.input(&netbuf[pos..have])?;
            pos += c;
            if c > 0 {
                hubo_io = true;
            }
        }

        // 2) Drenar TODOS los eventos pendientes: un solo payload puede
        // generar varios (p. ej. auth + apertura de canal). Cada evento
        // presta el runner, así que se resuelve en su propio bloque.
        let mut defunct = false;
        loop {
            let mut mas = true;
            {
                let ev = runner.progress()?;
                if !matches!(ev, Event::None | Event::Progressed) {
                    eprintln!("    progress -> {ev:?}");
                }
                match ev {
                    Event::Serv(ServEvent::Hostkeys(h)) => {
                        h.hostkeys(&[key])?;
                    }
                    Event::Serv(ServEvent::FirstAuth(a)) => {
                        // Monousuario: aceptamos al primer intento (como soso).
                        eprintln!("    auth: usuario {:?} aceptado sin credencial", a.username());
                        a.allow()?;
                    }
                    Event::Serv(ServEvent::PasswordAuth(a)) => {
                        a.allow()?;
                    }
                    Event::Serv(ServEvent::PubkeyAuth(a)) => {
                        a.allow()?;
                    }
                    Event::Serv(ServEvent::Authenticated) => {
                        autenticado = true;
                        eprintln!("    autenticado");
                    }
                    Event::Serv(ServEvent::OpenSession(o)) => {
                        chan = Some(o.accept()?);
                        eprintln!("    canal de sesión abierto");
                    }
                    Event::Serv(ServEvent::SessionPty(p)) => {
                        p.succeed()?;
                    }
                    Event::Serv(ServEvent::SessionEnv(e)) => {
                        // Aceptamos e ignoramos las variables de entorno.
                        eprintln!("    env {:?}={:?}", e.name(), e.value());
                        e.succeed()?;
                    }
                    Event::Serv(ServEvent::SessionShell(r)) => {
                        r.succeed()?;
                        eprintln!("    shell solicitada");
                    }
                    Event::Serv(ServEvent::SessionExec(r)) => {
                        // No soportamos exec en el proto: rechazar.
                        r.fail()?;
                    }
                    Event::Serv(ServEvent::Defunct) => {
                        eprintln!("    defunct");
                        defunct = true;
                        mas = false;
                    }
                    Event::Serv(otro) => {
                        eprintln!("    evento serv sin manejar: {otro:?}");
                    }
                    Event::Cli(_) => unreachable!("somos servidor"),
                    // None/Progressed/PollAgain: no hay más eventos ahora.
                    Event::None => mas = false,
                    Event::Progressed => {}
                }
            }
            if !mas {
                break;
            }
        }
        if defunct {
            break;
        }

        // 3) Mostrar el saludo en cuanto haya canal y shell.
        if autenticado
            && let Some(ch) = &chan
            && !shell.saludado
            && runner.write_channel_ready(ch, ChanData::Normal)?.unwrap_or(0) > 0
        {
            let saludo = shell.saludo().to_vec();
            escribir_canal(&mut runner, ch, &saludo)?;
        }

        // 4) Datos del canal: leer lo tecleado y devolver el eco/salida.
        if let Some(ch) = &chan {
            let mut cbuf = [0u8; 1024];
            let n = match runner.read_channel(ch, ChanData::Normal, &mut cbuf) {
                Ok(n) => n,
                Err(sunset::Error::ChannelEOF) => {
                    break;
                }
                Err(e) => return Err(e),
            };
            if n > 0 {
                let respuesta = shell.feed(&cbuf[..n]);
                escribir_canal(&mut runner, ch, &respuesta)?;
                if shell.salir {
                    let ch = chan.take().unwrap();
                    runner.channel_done(ch)?;
                }
            }
        }

        // 5) Vaciar la salida del runner al socket.
        let pend = runner.output_buf();
        if !pend.is_empty() {
            hubo_io = true;
            let escrito = stream.write(pend).map_err(sunset_io_err)?;
            runner.consume_output(escrito);
        }

        // Sin actividad: breve pausa para no quemar CPU (el kernel usará el
        // tick de timer / IRQ en su lugar).
        if !hubo_io {
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }

    // Vaciar lo que quede antes de cerrar.
    loop {
        let pend = runner.output_buf();
        if pend.is_empty() {
            break;
        }
        let escrito = stream.write(pend).map_err(sunset_io_err)?;
        runner.consume_output(escrito);
    }
    stream.flush().ok();
    Ok(())
}

/// Escribe todo `datos` al canal, insistiendo mientras el canal acepte.
fn escribir_canal(
    runner: &mut Runner<Server>,
    ch: &sunset::ChanHandle,
    datos: &[u8],
) -> Result<(), sunset::Error> {
    let mut ofs = 0;
    while ofs < datos.len() {
        match runner.write_channel(ch, ChanData::Normal, &datos[ofs..]) {
            Ok(0) => break, // sin sitio ahora; el resto se perderá (proto)
            Ok(n) => ofs += n,
            Err(sunset::Error::ChannelEOF) => break,
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

fn sunset_io_err(_e: std::io::Error) -> sunset::Error {
    sunset::Error::ChannelEOF
}
