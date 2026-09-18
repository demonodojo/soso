//! Servidor SSH-2 con sunset sobre sockets smoltcp (puerto 22).
//!
//! Hasta `SSH_SESSIONS` sesiones concurrentes (monousuario, misma clave).
//! El flujo replica el prototipo host `tools/ssh-proto`, ya validado con un
//! cliente OpenSSH real:
//!   Hostkeys → FirstAuth → Authenticated → OpenSession → Env/Pty →
//!   SessionShell → (datos del canal) → Defunct.
//!
//! Al recibir la petición de shell se lanza `/bin/sosh` con su consola
//! atada a este canal (`Console::Ssh(slot)`): lo que la shell escribe en
//! stdout va a la cola TX de la ranura (que este módulo drena hacia el
//! canal) y lo que llega por el canal va a la cola RX (que la shell lee
//! por su fd 0).
//!
//! Reglas de concurrencia: `poll()` solo corre desde `net::poll` (bajo el
//! try_lock de NetStack) y nunca reentra. Las colas RX/TX las tocan además las
//! syscalls del proceso, en ring 0 y con las interrupciones ABIERTAS (el `sti`
//! de `syscall_entry`), así que se toman con `without_interrupts`, igual que el
//! serie con su IRQ.
//!
//! Hasta 2026-08-01 aquí se afirmaba «jamás a la vez» porque el tick del timer
//! no llama a `net::poll` desde ring 0. Cierto — pero el handler MSI-X de
//! virtio-net sí lo hacía, y el interbloqueo monocore resultante mataba la
//! máquina justo después del prompt de una sesión nueva. La regla ahora la
//! comprueba el aserto de `net::poll` en vez de un comentario.

use crate::arch::pit;
use crate::task::{self, Console};
use alloc::collections::VecDeque;
use smoltcp::socket::tcp;
use spin::{Mutex, Once};
use x86_64::instructions::interrupts::without_interrupts;
use sunset::event::{Event, ServEvent};
use sunset::{ChanData, ChanHandle, Runner, Server, SignKey};

pub const SSH_PORT: u16 = 22;
pub const SSH_SESSIONS: usize = 4;

/// Host key ed25519: persistente desde /etc/ssh_host_key (semilla de 32
/// bytes) o generada al arranque si el fichero no existe.
static HOST_KEY: Once<SignKey> = Once::new();
/// Clave pública ed25519 autorizada (32 bytes) de /etc/authorized_key.
/// None => no hay fichero: se rechaza cualquier login.
static AUTHORIZED: Once<Option<[u8; 32]>> = Once::new();

struct Colas {
    rx: Mutex<VecDeque<u8>>,
    tx: Mutex<VecDeque<u8>>,
}

static COLAS: [Colas; SSH_SESSIONS] = [
    Colas {
        rx: Mutex::new(VecDeque::new()),
        tx: Mutex::new(VecDeque::new()),
    },
    Colas {
        rx: Mutex::new(VecDeque::new()),
        tx: Mutex::new(VecDeque::new()),
    },
    Colas {
        rx: Mutex::new(VecDeque::new()),
        tx: Mutex::new(VecDeque::new()),
    },
    Colas {
        rx: Mutex::new(VecDeque::new()),
        tx: Mutex::new(VecDeque::new()),
    },
];

pub fn rx_has_data(slot: usize) -> bool {
    without_interrupts(|| COLAS.get(slot).is_some_and(|c| !c.rx.lock().is_empty()))
}
pub fn rx_pop(slot: usize) -> Option<u8> {
    without_interrupts(|| COLAS.get(slot)?.rx.lock().pop_front())
}
/// Empujar stdout de la shell hacia el canal. La tty SSH es cruda: sin
/// `\r` antes de `\n` el cursor no vuelve al inicio de línea.
pub fn tx_push(slot: usize, data: &[u8]) {
    let Some(colas) = COLAS.get(slot) else {
        return;
    };
    without_interrupts(|| {
        let mut tx = colas.tx.lock();
        let mut prev = tx.back().copied();
        for &b in data {
            if b == b'\n' && prev != Some(b'\r') {
                tx.push_back(b'\r');
            }
            tx.push_back(b);
            prev = Some(b);
        }
    });
}

fn colas_vaciar(slot: usize) {
    if let Some(colas) = COLAS.get(slot) {
        colas.rx.lock().clear();
        colas.tx.lock().clear();
    }
}

fn tx_vacia(slot: usize) -> bool {
    COLAS
        .get(slot)
        .is_none_or(|c| without_interrupts(|| c.tx.lock().is_empty()))
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

/// Estado de una sesión SSH en una ranura.
pub struct SshSession {
    slot: u8,
    runner: Runner<'static, Server>,
    chan: Option<ChanHandle>,
    shell_pid: Option<u64>,
    /// La shell llegó a lanzarse (para cerrar el socket cuando muera).
    tuvo_shell: bool,
    /// `uptime_ms` de la muerte de la shell: falta vaciar TX antes de cerrar
    /// el canal, pero no indefinidamente (ver `GRACIA_TX_MS`).
    muerte_ms: Option<u64>,
    /// `uptime_ms` del `socket.close()` que iniciamos nosotros. Marca el
    /// comienzo del plazo de gracia del cierre ordenado.
    cierre_ms: Option<u64>,
    /// Buffer de entrada persistente (ver lección del prototipo: `input`
    /// puede aceptar parcialmente y devolver 0).
    netbuf: [u8; 4096],
    pos: usize,
    have: usize,
}

impl SshSession {
    fn new(slot: u8) -> Self {
        Self {
            slot,
            runner: Runner::new_server_owned(),
            chan: None,
            shell_pid: None,
            tuvo_shell: false,
            muerte_ms: None,
            cierre_ms: None,
            netbuf: [0; 4096],
            pos: 0,
            have: 0,
        }
    }
}

/// Plazo máximo para vaciar TX hacia el canal tras morir la shell. Si el
/// cliente deja de abrir ventana, se cierra igual y se pierde lo que quede:
/// peor que perder unos bytes es no cerrar la sesión nunca.
const GRACIA_TX_MS: u64 = 2_000;

static SESIONES: [Mutex<Option<SshSession>>; SSH_SESSIONS] = [
    Mutex::new(None),
    Mutex::new(None),
    Mutex::new(None),
    Mutex::new(None),
];

/// Vuelve a LISTEN **sin** RST. `abort()` pone CLOSED y smoltcp manda un RST;
/// en el AX200 del ROG eso tumba la radio (`No route to host`) y el Enter
/// siguiente en consola acaba en panic. `listen` admite Closed y TimeWait
/// (`is_open` es falso); CloseWait se cierra con FIN (`close` → LastAck).
fn reciclar_listen(
    slot: usize,
    socket: &mut tcp::Socket,
    guard: &mut Option<SshSession>,
    motivo: &str,
) {
    if guard.is_some() {
        teardown(slot, guard);
        crate::println!("ssh[{slot}]: sesión cerrada ({motivo})");
    }
    match socket.state() {
        tcp::State::Closed | tcp::State::TimeWait => {
            let _ = socket.listen(SSH_PORT);
        }
        tcp::State::CloseWait | tcp::State::Established | tcp::State::SynReceived => {
            socket.close();
        }
        _ => {}
    }
}

/// Avanza la sesión SSH usando `socket` como transporte. Llamada desde
/// `net::poll` con el socket TCP del puerto 22 ya poll-eado por la iface.
pub fn poll(slot: usize, socket: &mut tcp::Socket) {
    use tcp::State;
    let Some(mutex) = SESIONES.get(slot) else {
        return;
    };
    let mut guard = mutex.lock();

    match socket.state() {
        State::Closed | State::TimeWait => {
            reciclar_listen(slot, socket, &mut guard, "fin");
            return;
        }
        State::Listen | State::SynSent | State::SynReceived => {
            if guard.is_some() {
                teardown(slot, &mut guard);
            }
            return;
        }
        // El cliente cerró (Ctrl-C, hangup): FIN nuestro, no RST.
        State::CloseWait => {
            reciclar_listen(slot, socket, &mut guard, "cliente");
            return;
        }
        // Cierre ordenado en curso. Sin sesión no inventar otra.
        State::FinWait1 | State::FinWait2 | State::Closing | State::LastAck => {
            if guard.is_none() {
                return;
            }
        }
        State::Established => {}
    }

    if guard.is_none() {
        if socket.state() != State::Established {
            return;
        }
        colas_vaciar(slot);
        *guard = Some(SshSession::new(slot as u8));
    }
    let drive_err = {
        let sess = guard.as_mut().unwrap();
        drive(sess, socket).err()
    };
    if let Some(e) = drive_err {
        crate::println!("ssh[{slot}]: sesión terminada ({e:?})");
        reciclar_listen(slot, socket, &mut guard, "error");
    }
}

/// Cierra la shell y limpia el estado de la sesión.
fn teardown(slot: usize, guard: &mut Option<SshSession>) {
    task::kill_console(Console::Ssh(slot as u8));
    colas_vaciar(slot);
    *guard = None;
}

fn drive(sess: &mut SshSession, socket: &mut tcp::Socket) -> Result<(), sunset::Error> {
    let slot = sess.slot as usize;
    let key = HOST_KEY.get().expect("ssh sin host key");
    let colas = COLAS.get(slot).expect("ranura ssh inválida");

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

    // 4) ¿Terminó la shell? Anotarlo, pero NO cerrar el canal todavía: lo que
    // quede en TX (el "$ " y el eco de `exit`, y a veces la última línea de
    // salida) aún tiene que pasar al canal en el paso 5. El canal se cierra
    // en el paso 5b, cuando TX ya está vacía.
    if let Some(pid) = sess.shell_pid
        && !task::exists(pid)
    {
        sess.shell_pid = None;
        sess.muerte_ms = Some(pit::uptime_ms());
    }

    // 5) E/S del canal: recibido → RX (stdin de la shell); TX → canal.
    if let Some(ch) = &sess.chan {
        let console = Console::Ssh(sess.slot);
        // Entrada: del canal a la cola RX.
        let mut cbuf = [0u8; 1024];
        match sess.runner.read_channel(ch, ChanData::Normal, &mut cbuf) {
            Ok(n) if n > 0 => {
                for &b in &cbuf[..n] {
                    if b == 0x03 {
                        task::signal_console(console, soso_abi::SIGINT as u8);
                    } else {
                        colas.rx.lock().push_back(b);
                    }
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
                let mut tx = colas.tx.lock();
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
                let mut tx = colas.tx.lock();
                for &b in chunk[escrito..n].iter().rev() {
                    tx.push_front(b);
                }
                break;
            }
        }
    }

    // 5b) La shell murió y TX ya está vacía (o se agotó el plazo): cerrar el
    // canal. El plazo importa porque si el cliente deja de abrir ventana, TX
    // no se vacía nunca y sin él la sesión no terminaría jamás.
    if let Some(t) = sess.muerte_ms
        && (tx_vacia(slot) || pit::uptime_ms().saturating_sub(t) >= GRACIA_TX_MS)
        && let Some(ch) = sess.chan.take()
    {
        sess.runner.channel_done(ch)?;
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
        if sess.cierre_ms.is_none() {
            sess.cierre_ms = Some(pit::uptime_ms());
        }
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
    let slot = sess.slot as usize;
    let console = Console::Ssh(sess.slot);
    if let Some(motd) = leer_fichero("/etc/motd") {
        tx_push(slot, &motd);
    }
    match task::spawn_console("/bin/sosh", "", 0, console) {
        Ok(pid) => {
            task::session_leader(pid, console);
            crate::println!("ssh[{slot}]: sesión abierta, /bin/sosh pid {pid}");
            sess.shell_pid = Some(pid);
            sess.tuvo_shell = true;
        }
        Err(e) => crate::println!("ssh[{slot}]: no pude lanzar sosh (errno {e})"),
    }
}
