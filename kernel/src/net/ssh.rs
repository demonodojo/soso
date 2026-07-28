//! Servidor SSH-2 con sunset sobre un socket smoltcp (puerto 22).
//!
//! Una sola sesión a la vez (monousuario). El flujo replica el prototipo
//! host `tools/ssh-proto`, ya validado con un cliente OpenSSH real:
//!   Hostkeys → FirstAuth → Authenticated → OpenSession → Env/Pty →
//!   SessionShell → (datos del canal) → Defunct.
//!
//! Al recibir la petición de shell se lanza `/bin/sosh` con su consola
//! atada a este canal (Console::Ssh): lo que la shell escribe en stdout va
//! a la cola TX (que este módulo drena hacia el canal) y lo que llega por
//! el canal va a la cola RX (que la shell lee por su fd 0).
//!
//! Reglas de concurrencia: `poll()` solo corre desde `net::poll` (bajo el
//! try_lock de NetStack) y nunca reentra; las colas RX/TX las tocan además
//! las syscalls del proceso, pero jamás a la vez (una syscall corre en
//! ring 0 y el tick de timer no llama a `net::poll` desde ring 0).

use crate::task::{self, Console};
use alloc::collections::VecDeque;
use smoltcp::socket::tcp;
use spin::{Mutex, Once};
use sunset::event::{Event, ServEvent};
use sunset::{ChanData, ChanHandle, Runner, Server, SignKey};

pub const SSH_PORT: u16 = 22;

/// Host key ed25519: persistente desde /etc/ssh_host_key (semilla de 32
/// bytes) o generada al arranque si el fichero no existe.
static HOST_KEY: Once<SignKey> = Once::new();
/// Clave pública ed25519 autorizada (32 bytes) de /etc/authorized_key.
/// None => no hay fichero: se rechaza cualquier login.
static AUTHORIZED: Once<Option<[u8; 32]>> = Once::new();

/// Datos del canal SSH hacia el stdin de la shell.
static RX: Mutex<VecDeque<u8>> = Mutex::new(VecDeque::new());
/// stdout de la shell hacia el canal SSH.
static TX: Mutex<VecDeque<u8>> = Mutex::new(VecDeque::new());

pub fn rx_has_data() -> bool {
    !RX.lock().is_empty()
}
pub fn rx_pop() -> Option<u8> {
    RX.lock().pop_front()
}
/// Empujar stdout de la shell hacia el canal. La tty SSH es cruda: sin
/// `\r` antes de `\n` el cursor no vuelve al inicio de línea.
pub fn tx_push(data: &[u8]) {
    let mut tx = TX.lock();
    let mut prev = tx.back().copied();
    for &b in data {
        if b == b'\n' && prev != Some(b'\r') {
            tx.push_back(b'\r');
        }
        tx.push_back(b);
        prev = Some(b);
    }
}

/// Lee un fichero pequeño del FS montado (claves). None si no existe.
fn leer_fichero(path: &str) -> Option<alloc::vec::Vec<u8>> {
    let ino = crate::vfs::resolve(path).ok()?;
    crate::vfs::read_file(ino).ok()
}

pub fn init() {
    // Host key: semilla de 32 bytes de /etc/ssh_host_key; si no, generar.
    let key = match leer_fichero("/etc/ssh_host_key") {
        Some(seed) if seed.len() >= 32 => {
            let mut s = [0u8; 32];
            s.copy_from_slice(&seed[..32]);
            SignKey::Ed25519(ed25519_dalek::SigningKey::from_bytes(&s))
        }
        _ => {
            crate::println!("ssh: /etc/ssh_host_key ausente, generando host key efímera");
            SignKey::generate(sunset::KeyType::Ed25519, None)
                .expect("no se pudo generar la host key ed25519")
        }
    };
    if let sunset::PubKey::Ed25519(pk) = key.pubkey() {
        let b = pk.key.0;
        crate::println!(
            "ssh: host key ed25519 {:02x}{:02x}{:02x}{:02x}…{:02x}{:02x}",
            b[0], b[1], b[2], b[3], b[30], b[31]
        );
    }
    HOST_KEY.call_once(|| key);

    // Clave autorizada: 32 bytes de /etc/authorized_key.
    let auth = match leer_fichero("/etc/authorized_key") {
        Some(k) if k.len() >= 32 => {
            let mut a = [0u8; 32];
            a.copy_from_slice(&k[..32]);
            crate::println!("ssh: auth por clave pública ed25519 (/etc/authorized_key)");
            Some(a)
        }
        _ => {
            crate::println!("ssh: sin /etc/authorized_key: se rechazarán todos los logins");
            None
        }
    };
    AUTHORIZED.call_once(|| auth);
}

/// ¿Coincide `pk` con la clave autorizada?
fn pubkey_autorizada(pk: &sunset::PubKey) -> bool {
    let Some(Some(esperada)) = AUTHORIZED.get() else { return false };
    matches!(pk, sunset::PubKey::Ed25519(k) if &k.key.0 == esperada)
}

/// Estado de la sesión SSH en curso (solo una).
pub struct SshSession {
    runner: Runner<'static, Server>,
    chan: Option<ChanHandle>,
    shell_pid: Option<u64>,
    /// La shell llegó a lanzarse (para cerrar el socket cuando muera).
    tuvo_shell: bool,
    /// Buffer de entrada persistente (ver lección del prototipo: `input`
    /// puede aceptar parcialmente y devolver 0).
    netbuf: [u8; 4096],
    pos: usize,
    have: usize,
}

impl SshSession {
    fn new() -> Self {
        Self {
            runner: Runner::new_server_owned(),
            chan: None,
            shell_pid: None,
            tuvo_shell: false,
            netbuf: [0; 4096],
            pos: 0,
            have: 0,
        }
    }
}

static SESSION: Mutex<Option<SshSession>> = Mutex::new(None);

/// Mata la shell, limpia colas y deja el socket escuchando de nuevo.
fn reset_socket(socket: &mut tcp::Socket, guard: &mut Option<SshSession>) {
    teardown(guard);
    socket.abort();
    let _ = socket.listen(SSH_PORT);
}

/// Avanza la sesión SSH usando `socket` como transporte. Llamada desde
/// `net::poll` con el socket TCP del puerto 22 ya poll-eado por la iface.
pub fn poll(socket: &mut tcp::Socket) {
    use tcp::State;
    let mut guard = SESSION.lock();

    match socket.state() {
        // Sin conexión: derribar sesión previa y volver a escuchar.
        State::Closed => {
            if guard.is_some() {
                teardown(&mut guard);
            }
            socket.listen(SSH_PORT).ok();
            return;
        }
        // Esperando o negociando conexión: nada que hacer todavía; una
        // sesión colgada aquí es basura de una conexión anterior.
        State::Listen | State::SynSent | State::SynReceived => {
            if guard.is_some() {
                teardown(&mut guard);
            }
            return;
        }
        // Cliente desconectado (p. ej. Ctrl-C) o socket en TIME-WAIT: no
        // puede aceptar otra conexión hasta abortar y volver a LISTEN.
        State::CloseWait | State::TimeWait => {
            reset_socket(socket, &mut guard);
            return;
        }
        _ => {}
    }

    // Hay conexión (establecida o cerrándose con datos pendientes).
    if guard.is_none() {
        RX.lock().clear();
        TX.lock().clear();
        *guard = Some(SshSession::new());
    }
    let drive_err = {
        let sess = guard.as_mut().unwrap();
        drive(sess, socket).err()
    };
    if let Some(e) = drive_err {
        crate::println!("ssh: sesión terminada ({e:?})");
        reset_socket(socket, &mut guard);
        return;
    }

    // Cierre iniciado por nosotros (shell terminada): cuando no quede
    // salida pendiente, abortar y volver a escuchar otra conexión.
    let should_reset = {
        let s = guard.as_mut().unwrap();
        matches!(
            socket.state(),
            State::FinWait1 | State::FinWait2 | State::Closing | State::LastAck
        ) && s.shell_pid.is_none()
            && s.runner.output_buf().is_empty()
            && TX.lock().is_empty()
    };
    if should_reset {
        reset_socket(socket, &mut guard);
    }
}

/// Cierra la shell y limpia el estado de la sesión.
fn teardown(guard: &mut Option<SshSession>) {
    if let Some(sess) = guard
        && let Some(pid) = sess.shell_pid.take()
    {
        task::kill_pid(pid);
    }
    RX.lock().clear();
    TX.lock().clear();
    *guard = None;
}

fn drive(sess: &mut SshSession, socket: &mut tcp::Socket) -> Result<(), sunset::Error> {
    let key = HOST_KEY.get().expect("ssh sin host key");

    // 1) Rellenar netbuf desde el socket si está vacío y el runner acepta.
    if sess.pos == sess.have && sess.runner.is_input_ready() && socket.can_recv() {
        sess.pos = 0;
        sess.have = 0;
        let buf = &mut sess.netbuf;
        sess.have = socket.recv(|datos| {
            let n = datos.len().min(buf.len());
            buf[..n].copy_from_slice(&datos[..n]);
            (n, n)
        }).unwrap_or(0);
    }

    // 2) Alimentar lo que el runner acepte ahora.
    if sess.pos < sess.have && sess.runner.is_input_ready() {
        let c = sess.runner.input(&sess.netbuf[sess.pos..sess.have])?;
        sess.pos += c;
    }

    // 3) Drenar TODOS los eventos pendientes de este payload.
    loop {
        let mut mas = true;
        let mut abrir_shell = false;
        {
            let ev = sess.runner.progress()?;
            match ev {
                Event::Serv(ServEvent::Hostkeys(h)) => {
                    h.hostkeys(&[key])?;
                }
                Event::Serv(ServEvent::FirstAuth(mut a)) => {
                    // Solo clave pública: nada de aceptar el primer intento
                    // ni contraseña. Se ofrece 'publickey' y se rechaza.
                    a.set_auth_methods(false, true)?;
                    a.reject()?;
                }
                Event::Serv(ServEvent::PasswordAuth(a)) => {
                    a.reject()?;
                }
                Event::Serv(ServEvent::PubkeyAuth(a)) => {
                    // Aceptar solo si la clave presentada es la autorizada.
                    let ok = a.pubkey().map(|pk| pubkey_autorizada(&pk)).unwrap_or(false);
                    if ok {
                        a.allow()?;
                    } else {
                        a.reject()?;
                    }
                }
                Event::Serv(ServEvent::Authenticated) => {}
                Event::Serv(ServEvent::OpenSession(o)) => {
                    sess.chan = Some(o.accept()?);
                }
                Event::Serv(ServEvent::SessionPty(p)) => {
                    p.succeed()?;
                }
                Event::Serv(ServEvent::SessionEnv(e)) => {
                    // Aceptar e ignorar las variables de entorno.
                    e.succeed()?;
                }
                Event::Serv(ServEvent::SessionShell(r)) => {
                    r.succeed()?;
                    abrir_shell = true;
                }
                Event::Serv(ServEvent::SessionExec(r)) => {
                    // Sin exec: solo shell interactiva.
                    r.fail()?;
                }
                Event::Serv(ServEvent::Defunct) => {
                    return Err(sunset::Error::ChannelEOF);
                }
                Event::Serv(_) => {}
                Event::Cli(_) => {}
                Event::None => mas = false,
                Event::Progressed => {}
            }
        }
        // Fuera del préstamo del evento: ya se puede tocar sess.
        if abrir_shell {
            lanzar_shell(sess);
        }
        if !mas {
            break;
        }
    }

    // 4) ¿Terminó la shell? Cerrar el canal.
    //
    // LIMITACIÓN CONOCIDA (medida, no especulada): lo que quedara en TX al
    // morir la shell se pierde aquí — 8 bytes en una sesión trivial ("$ " más
    // el eco de "exit"). Es cosmético: la salida sustantiva ya salió en
    // vueltas anteriores del paso 5 (comprobado: llegan "init: TODO OK" y la
    // línea "soso-llm: generado … tok/s", que es el criterio GO del ciclo de
    // GPU). Se intentó vaciar TX antes de `channel_done` y NO vale: los bytes
    // recién metidos en el buffer de envío de smoltcp hacen que `close()` no
    // pase a FinWait, `poll` no llega a resetear, y el cliente se queda
    // esperando un FIN que no llega — la sesión no termina nunca. Cambiar
    // 8 bytes cosméticos por un cuelgue es peor; si algún día se arregla,
    // hay que tocar también el criterio de cierre de `poll`.
    if let Some(pid) = sess.shell_pid
        && !task::exists(pid)
    {
        sess.shell_pid = None;
        if let Some(ch) = sess.chan.take() {
            sess.runner.channel_done(ch)?;
        }
    }

    // 5) E/S del canal: recibido → RX (stdin de la shell); TX → canal.
    if let Some(ch) = &sess.chan {
        // Entrada: del canal a la cola RX.
        let mut cbuf = [0u8; 1024];
        match sess.runner.read_channel(ch, ChanData::Normal, &mut cbuf) {
            Ok(n) if n > 0 => {
                let mut rx = RX.lock();
                for &b in &cbuf[..n] {
                    rx.push_back(b);
                }
            }
            Ok(_) => {}
            // El EOF del cliente NO pierde entrada: medido con sonda, lo que
            // manda llega siempre antes (`read_channel Ok(15)` con el payload
            // completo). Lo que sí rompe es la reacción de sunset a ese EOF:
            // `channel.rs::handle_eof` ESPEJA un ChannelEof de vuelta
            // (`if !self.sent_eof { s.send(ChannelEof) }`, con su propio
            // "//TODO: check existing state?"), o sea que anuncia "no enviaré
            // más datos" solo porque el cliente dejó de enviar. El EOF es por
            // dirección, así que eso es incorrecto: OpenSSH lo recibe, cierra
            // su salida ("output drain -> closed") y TIRA todo lo que la shell
            // imprima después. Efecto práctico: `printf 'cmd\n' | ssh` ejecuta
            // el comando (verificado escribiendo un fichero y leyéndolo en otra
            // sesión) pero no devuelve NADA de su salida.
            // Por eso los arneses (xtask/src/test.rs, scripts/l6-g1-vfio-test.sh)
            // mantienen stdin abierto con un FIFO y nunca cierran la entrada.
            // Arreglarlo de verdad exige parchear sunset.
            Err(sunset::Error::ChannelEOF) => {}
            Err(e) => return Err(e),
        }
        // Salida: de la cola TX al canal, mientras quepa.
        loop {
            let listo = sess.runner.write_channel_ready(ch, ChanData::Normal)?.unwrap_or(0);
            if listo == 0 {
                break;
            }
            let mut chunk = [0u8; 1024];
            let n = {
                let mut tx = TX.lock();
                let n = tx.len().min(chunk.len()).min(listo);
                for c in chunk.iter_mut().take(n) {
                    *c = tx.pop_front().unwrap();
                }
                n
            };
            if n == 0 {
                break;
            }
            let escrito = sess.runner.write_channel(ch, ChanData::Normal, &chunk[..n])?;
            // Si el canal aceptó menos de lo sacado, devolver el resto a TX.
            if escrito < n {
                let mut tx = TX.lock();
                for &b in chunk[escrito..n].iter().rev() {
                    tx.push_front(b);
                }
                break;
            }
        }
    }

    // 6) Volcar la salida del runner al socket.
    while socket.can_send() {
        let out = sess.runner.output_buf();
        if out.is_empty() {
            break;
        }
        let escrito = socket.send_slice(out).unwrap_or(0);
        if escrito == 0 {
            break;
        }
        sess.runner.consume_output(escrito);
    }

    // 7) La shell terminó y ya no queda nada por enviar: cerrar el socket
    // para que el cliente reciba el FIN y salga limpiamente (sunset 0.5 no
    // manda exit-status; el cierre del canal + TCP es la señal de fin).
    if sess.tuvo_shell
        && sess.shell_pid.is_none()
        && sess.chan.is_none()
        && sess.runner.output_buf().is_empty()
    {
        socket.close();
    }
    Ok(())
}

/// Lanza /bin/sosh con su consola atada a este canal SSH. Antes empuja el
/// motd al canal (aparece antes del banner de la shell).
fn lanzar_shell(sess: &mut SshSession) {
    if sess.shell_pid.is_some() {
        return; // ya hay shell
    }
    if let Some(motd) = leer_fichero("/etc/motd") {
        tx_push(&motd);
    }
    match task::spawn_console("/bin/sosh", "", 0, Console::Ssh) {
        Ok(pid) => {
            crate::println!("ssh: sesión abierta, /bin/sosh pid {pid}");
            sess.shell_pid = Some(pid);
            sess.tuvo_shell = true;
        }
        Err(e) => crate::println!("ssh: no pude lanzar sosh (errno {e})"),
    }
}
