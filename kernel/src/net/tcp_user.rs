//! Sockets TCP de userspace sobre smoltcp.

use crate::arch::pit;
use alloc::vec;
use alloc::vec::Vec;
use smoltcp::iface::{Interface, SocketHandle};
use smoltcp::socket::tcp;
use smoltcp::wire::{IpAddress, IpEndpoint, IpListenEndpoint, Ipv4Address};

pub const MAX_USER_TCP: usize = 8;
const TCP_BUF: usize = 65536;

/// Rango de puertos locales para conexiones salientes.
const PUERTO_EFIMERO_MIN: u16 = 49152;
const PUERTOS_EFIMEROS: u32 = 65536 - PUERTO_EFIMERO_MIN as u32;
static SIGUIENTE_EFIMERO: core::sync::atomic::AtomicU32 =
    core::sync::atomic::AtomicU32::new(0);

/// Puerto local para un `connect` saliente.
///
/// **No puede salir del índice de slot.** Al cerrar una conexión el slot se
/// libera y se reutiliza de inmediato, así que con `49152 + slot` la conexión
/// siguiente al mismo destino repetía la **cuádrupla entera**: el par todavía
/// la tiene en `TIME_WAIT` y descarta el SYN. El `connect` vencía a los 30 s
/// con `EAGAIN` y sin una sola pista de por qué.
///
/// No se ve con una conexión sola —por eso pasaba la fase `saliente`—: hace
/// falta una **segunda** conexión al mismo host, que es exactamente lo que
/// provoca cualquier redirección HTTP (2026-09-20).
fn puerto_efimero() -> u16 {
    let n = SIGUIENTE_EFIMERO.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    PUERTO_EFIMERO_MIN + (n % PUERTOS_EFIMEROS) as u16
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TcpRole {
    Listening,
    Connecting,
    Connected,
}

pub struct UserTcp {
    pub handle: SocketHandle,
    pub role: TcpRole,
    pub port: u16,
    pub remote: Option<IpEndpoint>,
    pub closed: bool,
    pub connect_started: bool,
    /// Último estado visto de la conexión, para contar sólo los cambios: sin
    /// esto la traza escupiría una línea por vuelta del planificador.
    pub ultimo_estado: tcp::State,
    /// Conexión 127.0.0.1 (sin smoltcp).
    pub loopback: bool,
    pub loop_pair: Option<usize>,
    pub loop_side: super::loopback::LoopSide,
    /// Listener loopback: no pasa a Connected al aceptar.
    pub loop_listener: bool,
}

pub struct TcpTable {
    pub entries: Vec<Option<UserTcp>>,
    /// Sockets cerrados que **todavía tienen datos por enviar**. Ver `free`.
    cerrando: Vec<(SocketHandle, u64)>,
}

/// Cuánto se espera a que un socket cerrado termine de vaciarse antes de
/// tirarlo igualmente. Un par que no asiente no puede retener memoria del
/// kernel para siempre.
const DRENAJE_MAX_MS: u64 = 3_000;

impl TcpTable {
    pub fn new() -> Self {
        Self {
            entries: (0..MAX_USER_TCP).map(|_| None).collect(),
            cerrando: Vec::new(),
        }
    }

    pub fn alloc(
        &mut self,
        sockets: &mut smoltcp::iface::SocketSet<'static>,
        role: TcpRole,
        port: u16,
        remote: Option<IpEndpoint>,
    ) -> Result<usize, ()> {
        let slot = self.entries.iter().position(|e| e.is_none()).ok_or(())?;
        let handle = sockets.add(tcp::Socket::new(
            tcp::SocketBuffer::new(vec![0; TCP_BUF]),
            tcp::SocketBuffer::new(vec![0; TCP_BUF]),
        ));
        self.entries[slot] = Some(UserTcp {
            handle,
            role,
            port,
            remote,
            closed: false,
            connect_started: false,
            ultimo_estado: tcp::State::Closed,
            loopback: false,
            loop_pair: None,
            loop_side: super::loopback::LoopSide::Client,
            loop_listener: false,
        });
        Ok(slot)
    }

    /// Slot loopback sin socket smoltcp (127.0.0.1).
    pub fn alloc_loopback(
        &mut self,
        sockets: &mut smoltcp::iface::SocketSet<'static>,
        role: TcpRole,
        pair_id: usize,
        side: super::loopback::LoopSide,
    ) -> Result<usize, ()> {
        let slot = self.entries.iter().position(|e| e.is_none()).ok_or(())?;
        let handle = sockets.add(tcp::Socket::new(
            tcp::SocketBuffer::new(vec![0; TCP_BUF]),
            tcp::SocketBuffer::new(vec![0; TCP_BUF]),
        ));
        self.entries[slot] = Some(UserTcp {
            handle,
            role,
            port: 0,
            remote: None,
            closed: false,
            connect_started: false,
            ultimo_estado: tcp::State::Closed,
            loopback: true,
            loop_pair: Some(pair_id),
            loop_side: side,
            loop_listener: false,
        });
        Ok(slot)
    }

    pub fn free(&mut self, sockets: &mut smoltcp::iface::SocketSet<'static>, slot: usize) {
        if let Some(entry) = self.entries[slot].take() {
            if entry.loopback {
                if let (Some(pair), side) = (entry.loop_pair, entry.loop_side) {
                    super::loopback::close_side(pair, side);
                }
            } else {
                let s = sockets.get_mut::<tcp::Socket>(entry.handle);
                if s.is_open() {
                    s.close();
                }
            }
            // `alloc_loopback` también mete un socket smoltcp (no se usa para
            // copiar, pero ocupa 128 KiB). Sin quitarlo, cada `ask` fugaba uno
            // y a la octava conexión `tcp_listen`/`connect` morían con EMFILE.
            //
            // Pero quitarlo **aquí** tira lo que el usuario acaba de escribir y
            // aún no ha salido por el cable: el servidor HTTP del guest
            // contestaba y el cliente veía «empty reply», porque `close()` sólo
            // pide el FIN y es smoltcp quien vacía el búfer en los siguientes
            // `poll`. El socket pasa a la cola de drenaje y se quita cuando ha
            // terminado —o a los `DRENAJE_MAX_MS`, para que un par mudo no
            // retenga memoria—.
            if entry.loopback {
                sockets.remove(entry.handle);
            } else {
                let plazo = pit::uptime_ms().saturating_add(DRENAJE_MAX_MS);
                self.cerrando.push((entry.handle, plazo));
            }
        }
    }

    /// Quita los sockets ya drenados. Se llama desde `net::poll`.
    pub fn purgar_cerrados(&mut self, sockets: &mut smoltcp::iface::SocketSet<'static>) {
        let ahora = pit::uptime_ms();
        let mut fuera: Vec<SocketHandle> = Vec::new();
        self.cerrando.retain(|&(handle, plazo)| {
            let s = sockets.get::<tcp::Socket>(handle);
            // `Closed`/`TimeWait` = el otro extremo ya asintió lo que había.
            let terminado = matches!(s.state(), tcp::State::Closed | tcp::State::TimeWait);
            if terminado || ahora >= plazo {
                fuera.push(handle);
                return false;
            }
            true
        });
        for handle in fuera {
            sockets.remove(handle);
        }
    }

    /// Slot en escucha sobre `port`, si existe.
    pub fn find_listener(&self, port: u16) -> Option<usize> {
        self.entries.iter().enumerate().find_map(|(i, e)| {
            let e = e.as_ref()?;
            if e.role == TcpRole::Listening && e.port == port && !e.closed {
                Some(i)
            } else {
                None
            }
        })
    }
}

/// Entrega al usuario una conexión **externa** ya establecida sobre el socket
/// del listener, y deja el listener escuchando otra vez.
///
/// Hasta 2026-09-23 esto no existía: `tcp_listen` marcaba todo listener como
/// `loop_listener` y `accept` sólo miraba pares de loopback, así que un
/// servidor de userspace era inalcanzable desde fuera del guest —el SYN
/// llegaba, smoltcp lo establecía y nadie lo recogía nunca—. El eco y SSH no
/// lo delataban porque son servidores del kernel, no de userspace.
///
/// El socket establecido **se cede** al slot nuevo y el listener recibe uno
/// recién creado: sin eso, la primera petición dejaría el puerto sin escucha y
/// la segunda moriría, que con HTTP es siempre.
pub fn ceder_establecida(
    table: &mut TcpTable,
    sockets: &mut smoltcp::iface::SocketSet<'static>,
    listener_slot: usize,
) -> Result<usize, i64> {
    let (handle, port) = {
        let e = table
            .entries
            .get(listener_slot)
            .and_then(|e| e.as_ref())
            .ok_or(-soso_abi::EBADF)?;
        (e.handle, e.port)
    };
    let nuevo = table
        .entries
        .iter()
        .position(|e| e.is_none())
        .ok_or(-soso_abi::EMFILE)?;
    let (remote, estado) = {
        let s = sockets.get::<tcp::Socket>(handle);
        (s.remote_endpoint(), s.state())
    };

    // El socket de relevo se crea antes de tocar el listener: si no hubiera
    // sitio, el listener se queda exactamente como estaba.
    let relevo = sockets.add(tcp::Socket::new(
        tcp::SocketBuffer::new(vec![0; TCP_BUF]),
        tcp::SocketBuffer::new(vec![0; TCP_BUF]),
    ));

    table.entries[nuevo] = Some(UserTcp {
        handle,
        role: TcpRole::Connected,
        port,
        remote,
        closed: false,
        connect_started: false,
        ultimo_estado: estado,
        loopback: false,
        loop_pair: None,
        loop_side: super::loopback::LoopSide::Client,
        loop_listener: false,
    });

    if let Some(l) = table.entries[listener_slot].as_mut() {
        l.handle = relevo;
        // `poll_entry` pasa un listener a `Connected` en cuanto su socket se
        // establece; al cederlo hay que devolverlo a `Listening` o dejaría de
        // rearmar la escucha.
        l.role = TcpRole::Listening;
        l.ultimo_estado = tcp::State::Closed;
        if listen_start(sockets, l).is_err() {
            crate::println!("tcp: no pude rearmar la escucha en {port}");
        }
    }
    Ok(nuevo)
}

pub fn listen_start(
    sockets: &mut smoltcp::iface::SocketSet<'static>,
    entry: &mut UserTcp,
) -> Result<(), ()> {
    let s = sockets.get_mut::<tcp::Socket>(entry.handle);
    if !s.is_open() {
        s.listen(entry.port).map_err(|_| ())?;
    }
    Ok(())
}

/// Reexporta los estados de smoltcp para que `net::mod` no tenga que importar
/// el crate entero sólo para nombrarlos.
pub use smoltcp::socket::tcp::State as EstadoSmoltcp;

pub fn is_established(sockets: &smoltcp::iface::SocketSet<'static>, handle: SocketHandle) -> bool {
    let s = sockets.get::<tcp::Socket>(handle);
    s.state() == tcp::State::Established
}

/// ¿Hay una conexión entrante que entregar al usuario en este listener?
///
/// **No basta con `Established`.** Un cliente que manda su petición y cierra
/// —un sondeo con plazo corto, `curl` con `--max-time`, cualquier cosa detrás
/// de slirp— deja el socket en `CloseWait` antes de que el `accept` del
/// proceso llegue a mirarlo, y con la condición estrecha la conexión se perdía
/// **y** el listener se quedaba clavado en `CloseWait`: la primera petición
/// mataba el puerto para siempre. La petición ya está en el búfer de
/// recepción; se puede leer y contestar igual.
pub fn tiene_conexion(sockets: &smoltcp::iface::SocketSet<'static>, handle: SocketHandle) -> bool {
    matches!(
        sockets.get::<tcp::Socket>(handle).state(),
        tcp::State::Established
            | tcp::State::CloseWait
            | tcp::State::FinWait1
            | tcp::State::FinWait2
            | tcp::State::Closing
            | tcp::State::LastAck
    )
}

pub fn try_read_loopback(
    entry: &UserTcp,
    buf: u64,
    len: u64,
) -> Result<u64, i64> {
    if entry.role != TcpRole::Connected {
        return Err(-soso_abi::ENOTCONN);
    }
    let pair = entry.loop_pair.ok_or(-soso_abi::ENOTCONN)?;
    super::loopback::try_read(pair, entry.loop_side, buf, len)
}

pub fn try_write_loopback(
    entry: &UserTcp,
    buf: u64,
    len: u64,
) -> Result<u64, i64> {
    if entry.role != TcpRole::Connected {
        return Err(-soso_abi::ENOTCONN);
    }
    let pair = entry.loop_pair.ok_or(-soso_abi::ENOTCONN)?;
    super::loopback::try_write(pair, entry.loop_side, buf, len)
}

pub fn try_read(
    sockets: &mut smoltcp::iface::SocketSet<'static>,
    handle: SocketHandle,
    role: TcpRole,
    buf: u64,
    len: u64,
) -> Result<u64, i64> {
    if role != TcpRole::Connected {
        return Err(-soso_abi::ENOTCONN);
    }
    let s = sockets.get_mut::<tcp::Socket>(handle);
    if !s.can_recv() {
        if matches!(s.state(), tcp::State::CloseWait | tcp::State::Closed) {
            return Ok(0);
        }
        return Ok(0);
    }
    let dst_len = len as usize;
    if dst_len == 0 {
        return Ok(0);
    }
    let mut tmp = [0u8; 4096];
    let chunk = dst_len.min(tmp.len());
    let n = s.recv_slice(&mut tmp[..chunk]).unwrap_or(0);
    if n == 0 {
        return Ok(0);
    }
    for i in 0..n {
        unsafe {
            *((buf + i as u64) as *mut u8) = tmp[i];
        }
    }
    Ok(n as u64)
}

pub fn try_write(
    sockets: &mut smoltcp::iface::SocketSet<'static>,
    handle: SocketHandle,
    role: TcpRole,
    buf: u64,
    len: u64,
) -> Result<u64, i64> {
    if role != TcpRole::Connected {
        return Err(-soso_abi::ENOTCONN);
    }
    let s = sockets.get_mut::<tcp::Socket>(handle);
    if !s.can_send() {
        return Ok(0);
    }
    let src_len = len as usize;
    if src_len == 0 {
        return Ok(0);
    }
    let mut tmp = [0u8; 4096];
    let chunk = src_len.min(tmp.len());
    for i in 0..chunk {
        tmp[i] = unsafe { *((buf + i as u64) as *const u8) };
    }
    let n = s.send_slice(&tmp[..chunk]).unwrap_or(0);
    Ok(n as u64)
}

pub fn poll_entry(
    iface: &mut Interface,
    sockets: &mut smoltcp::iface::SocketSet<'static>,
    slot: usize,
    entry: &mut UserTcp,
    configured: bool,
) {
    if entry.closed || entry.loopback {
        return;
    }
    if !configured {
        return;
    }
    match entry.role {
        TcpRole::Listening => {
            let s = sockets.get_mut::<tcp::Socket>(entry.handle);
            if !s.is_open() {
                let _ = s.listen(entry.port);
            } else if s.state() == tcp::State::Established {
                entry.role = TcpRole::Connected;
            }
            // Un listener cuyo socket muere sin que nadie lo aceptara tiene que
            // volver a escuchar, o el puerto queda mudo para siempre.
            let estado = sockets.get::<tcp::Socket>(entry.handle).state();
            if matches!(estado, tcp::State::Closed | tcp::State::TimeWait) {
                let s = sockets.get_mut::<tcp::Socket>(entry.handle);
                if !s.is_open() {
                    let _ = s.listen(entry.port);
                }
            }
            entry.ultimo_estado = estado;
        }
        TcpRole::Connecting => {
            if !entry.connect_started {
                if let Some(remote) = entry.remote {
                    let local_port = puerto_efimero();
                    let local_ip = iface
                        .ip_addrs()
                        .iter()
                        .find_map(|cidr| match cidr.address() {
                            IpAddress::Ipv4(v4) => Some(v4),
                        })
                        .unwrap_or(Ipv4Address::UNSPECIFIED);
                    let local = IpListenEndpoint::from((
                        IpAddress::Ipv4(local_ip),
                        local_port,
                    ));
                    let s = sockets.get_mut::<tcp::Socket>(entry.handle);
                    let cx = iface.context();
                    // Un `connect` que el propio smoltcp rechaza sí se dice: no
                    // hay otra forma de enterarse desde fuera.
                    match s.connect(cx, remote, local) {
                        Ok(()) => entry.connect_started = true,
                        Err(e) => crate::println!("tcp: connect rechazado ({e:?}) slot {slot}"),
                    }
                }
            } else {
                let s = sockets.get::<tcp::Socket>(entry.handle);
                let estado = s.state();
                entry.ultimo_estado = estado;
                if estado == tcp::State::Established {
                    entry.role = TcpRole::Connected;
                } else if matches!(estado, tcp::State::Closed | tcp::State::TimeWait) {
                    entry.closed = true;
                }
            }
        }
        TcpRole::Connected => {
            let s = sockets.get_mut::<tcp::Socket>(entry.handle);
            // **No** se cierra por estar en `CloseWait`. El medio cierre del
            // cliente (`shutdown(Write)` tras mandar la petición) es corriente
            // en HTTP y sólo dice «no mando más», no «no quiero respuesta».
            // Cerrando aquí, la aplicación escribía su respuesta sobre un
            // socket ya cerrado y el cliente recibía cero bytes: era el
            // «empty reply from server» del servidor del guest. El socket lo
            // cierra su dueño, y `free` lo drena.
            if matches!(s.state(), tcp::State::Closed | tcp::State::TimeWait) {
                entry.closed = true;
            }
        }
    }
}

pub fn endpoint_from_abi(addr: &soso_abi::SockAddr) -> IpEndpoint {
    IpEndpoint::new(
        IpAddress::Ipv4(Ipv4Address::new(
            addr.addr[0],
            addr.addr[1],
            addr.addr[2],
            addr.addr[3],
        )),
        addr.port,
    )
}

pub fn fallback_addr_from_mac(mac: [u8; 6]) -> Ipv4Address {
    let last = mac[5];
    if last == 0x15 {
        Ipv4Address::new(10, 0, 2, 15)
    } else {
        Ipv4Address::new(10, 0, 2, last)
    }
}

#[allow(dead_code)]
pub fn uptime_ms() -> u64 {
    pit::uptime_ms()
}
