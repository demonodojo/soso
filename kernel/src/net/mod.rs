//! Pila de red: smoltcp sobre e1000e o virtio-net. DHCP al arrancar; si no
//! hay lease en unos segundos, fallback a IP estática 10.0.2.15/24 (QEMU
//! slirp).
//!
//! `poll()` se llama desde el bucle del scheduler, el tick del timer (sólo si
//! interrumpió ring 3) y el `irq_exit` de una IRQ que venía de ring 3 — **nunca
//! desde una IRQ dura**: ahí abajo se toman PROCS, las colas de ssh, el VFS y el
//! heap, y las syscalls sostienen esos mismos candados en ring 0 con IF=1. Lo
//! comprueba el aserto de `poll()`. Usa try_lock: si la pila está ocupada, la
//! próxima pasada lo recoge.

mod device;
pub mod dns;
pub mod ssh;
mod tcp_user;

use crate::arch::pit;
use crate::drivers::{e1000e, virtio_net};
use crate::println;
use alloc::vec;
use alloc::vec::Vec;
use device::{E1000Dev, NicDev, SmolDev};
use smoltcp::iface::{Config, Interface, SocketHandle, SocketSet};
use smoltcp::socket::{dhcpv4, tcp};
use smoltcp::time::{Duration, Instant};
use smoltcp::wire::{EthernetAddress, IpCidr, Ipv4Address, Ipv4Cidr};
use spin::{Mutex, Once};

pub const ECHO_PORT: u16 = 7;

/// Sockets de eco escuchando a la vez: mientras uno atiende o se cierra,
/// otro sigue en LISTEN (si no, una segunda conexión rápida recibe RST).
const ECHO_SOCKETS: usize = 4;

/// Tiempo máximo de espera de DHCP antes del fallback estático.
const DHCP_TIMEOUT: Duration = Duration::from_secs(8);

/// IP y pasarela del fallback QEMU slirp (último octeto derivado de MAC).
const FALLBACK_GW: Ipv4Address = Ipv4Address::new(10, 0, 2, 2);

struct NetStack {
    iface: Interface,
    sockets: SocketSet<'static>,
    echo: Vec<SocketHandle>,
    ssh: SocketHandle,
    dhcp: SocketHandle,
    configured: bool,
    dhcp_started: Instant,
    dev: NicDev,
    mac: [u8; 6],
    user_tcp: tcp_user::TcpTable,
}

static NET: Once<Mutex<NetStack>> = Once::new();

fn now() -> Instant {
    Instant::from_millis(pit::uptime_ms() as i64)
}

fn net_backend() -> Option<([u8; 6], NicDev)> {
    #[cfg(feature = "lxdde")]
    if crate::lxdde::e1000e_present() {
        let mac = crate::lxdde::e1000e_mac()?;
        println!("net: backend lx-e1000e");
        return Some((mac, NicDev::LxE1000e(device::LxE1000Dev)));
    }
    if e1000e::present() {
        let mac = e1000e::mac()?;
        println!("net: backend e1000e");
        Some((mac, NicDev::E1000e(E1000Dev)))
    } else if let Some(mac) = virtio_net::init() {
        println!("net: backend virtio-net");
        Some((mac, NicDev::Virtio(SmolDev)))
    } else {
        None
    }
}

fn random_seed() -> u64 {
    let mut buf = [0u8; 8];
    if getrandom::getrandom(&mut buf).is_ok() {
        u64::from_le_bytes(buf)
    } else {
        pit::uptime_ms() as u64
    }
}

fn set_ipv4_addr(iface: &mut Interface, cidr: Ipv4Cidr) {
    iface.update_ip_addrs(|addrs| {
        addrs.clear();
        addrs.push(IpCidr::Ipv4(cidr)).unwrap();
    });
}

fn apply_static_fallback(iface: &mut Interface, mac: [u8; 6]) {
    let addr = tcp_user::fallback_addr_from_mac(mac);
    set_ipv4_addr(iface, Ipv4Cidr::new(addr, 24));
    iface.routes_mut().remove_default_ipv4_route();
    iface
        .routes_mut()
        .add_default_ipv4_route(FALLBACK_GW)
        .unwrap();
}

fn clear_ipv4_config(iface: &mut Interface) {
    iface.update_ip_addrs(|addrs| addrs.clear());
    iface.routes_mut().remove_default_ipv4_route();
}

fn close_tcp_services(sockets: &mut SocketSet<'static>, echo: &[SocketHandle], ssh: SocketHandle) {
    for &h in echo {
        let s = sockets.get_mut::<tcp::Socket>(h);
        if s.is_open() {
            s.close();
        }
    }
    let s = sockets.get_mut::<tcp::Socket>(ssh);
    if s.is_open() {
        s.close();
    }
}

pub fn init() {
    // Preferir lx-e1000e (driver Linux vía lxdde), luego e1000e nativo, luego virtio.
    let Some((mac, mut dev)) = net_backend() else {
        println!("net: sin NIC");
        return;
    };

    let mut config = Config::new(EthernetAddress(mac).into());
    config.random_seed = random_seed();
    let iface = Interface::new(config, &mut dev, now());

    let mut sockets = SocketSet::new(Vec::new());
    let dhcp = sockets.add(dhcpv4::Socket::new());
    let echo = (0..ECHO_SOCKETS)
        .map(|_| {
            sockets.add(tcp::Socket::new(
                tcp::SocketBuffer::new(vec![0; 8192]),
                tcp::SocketBuffer::new(vec![0; 8192]),
            ))
        })
        .collect();
    let ssh = sockets.add(tcp::Socket::new(
        tcp::SocketBuffer::new(vec![0; 16384]),
        tcp::SocketBuffer::new(vec![0; 16384]),
    ));

    println!(
        "net: dhcp… (mac {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x})",
        mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
    );
    ssh::init();
    let dhcp_started = now();
    NET.call_once(|| {
        Mutex::new(NetStack {
            iface,
            sockets,
            echo,
            ssh,
            dhcp,
            configured: false,
            dhcp_started,
            dev,
            mac,
            user_tcp: tcp_user::TcpTable::new(),
        })
    });
}

fn poll_dhcp(
    iface: &mut Interface,
    sockets: &mut SocketSet<'static>,
    echo: &[SocketHandle],
    ssh: SocketHandle,
    dhcp: SocketHandle,
    configured: &mut bool,
) {
    let event = sockets.get_mut::<dhcpv4::Socket>(dhcp).poll();
    match event {
        None => {}
        Some(dhcpv4::Event::Configured(config)) => {
            set_ipv4_addr(iface, config.address);
            iface.routes_mut().remove_default_ipv4_route();
            if let Some(router) = config.router {
                iface.routes_mut().add_default_ipv4_route(router).unwrap();
                println!(
                    "net: dhcp {}/{} gw {}",
                    config.address.address(),
                    config.address.prefix_len(),
                    router
                );
            } else {
                println!(
                    "net: dhcp {}/{}",
                    config.address.address(),
                    config.address.prefix_len()
                );
            }
            *configured = true;
        }
        Some(dhcpv4::Event::Deconfigured) => {
            // smoltcp emite Deconfigured al arrancar (estado Discovering con
            // config_changed=true); ignorar si aún no hubo lease.
            if !*configured {
                return;
            }
            println!("net: dhcp perdido");
            clear_ipv4_config(iface);
            close_tcp_services(sockets, echo, ssh);
            *configured = false;
        }
    }
}

fn try_static_fallback(
    iface: &mut Interface,
    mac: [u8; 6],
    dhcp_started: Instant,
    configured: &mut bool,
) {
    if *configured || now() < dhcp_started + DHCP_TIMEOUT {
        return;
    }
    apply_static_fallback(iface, mac);
    *configured = true;
    let addr = tcp_user::fallback_addr_from_mac(mac);
    println!("net: sin dhcp, ip estática {addr}/24");
}

fn poll_tcp_services(
    sockets: &mut SocketSet<'static>,
    echo: &[SocketHandle],
    ssh: SocketHandle,
    configured: bool,
) {
    if !configured {
        return;
    }

    // Servidor SSH (puerto 22).
    ssh::poll(sockets.get_mut::<tcp::Socket>(ssh));

    for &h in echo {
        let s = sockets.get_mut::<tcp::Socket>(h);
        if !s.is_open() {
            s.listen(ECHO_PORT).ok();
            continue;
        }
        // Eco: reenviar lo recibido mientras quepa en el buffer de salida.
        let mut buf = [0u8; 1024];
        while s.can_recv() && s.can_send() {
            let leidos = s.recv_slice(&mut buf).unwrap_or(0);
            if leidos == 0 {
                break;
            }
            s.send_slice(&buf[..leidos]).ok();
        }
        // El cliente cerró su lado: cerrar el nuestro al vaciarse todo.
        if s.state() == tcp::State::CloseWait && !s.can_recv() {
            s.close();
        }
    }
}

/// Trabajo de red agendado por una IRQ dura y aún sin procesar. Es el
/// `__napi_schedule` de Linux: el handler marca y sale, la pila corre fuera del
/// contexto de interrupción (ver la avería documentada en
/// `drivers::virtio_net::net_irq_handler`).
static TRABAJO_PENDIENTE: core::sync::atomic::AtomicBool =
    core::sync::atomic::AtomicBool::new(false);

/// Llamable desde IRQ dura: sólo marca, no toca ni un candado.
pub fn marcar_trabajo_pendiente() {
    TRABAJO_PENDIENTE.store(true, core::sync::atomic::Ordering::Release);
}

/// ¿Hay trabajo agendado? Lo consume: lo va a procesar quien pregunte.
pub fn trabajo_pendiente() -> bool {
    TRABAJO_PENDIENTE.swap(false, core::sync::atomic::Ordering::AcqRel)
}

/// Procesa la red: DHCP, entrada/salida pendiente y el servidor de eco.
/// Reentrante-seguro vía try_lock (puede llamarse desde el tick de timer
/// que interrumpió a un proceso de usuario: ahí el kernel no tiene locks).
pub fn poll() {
    // NUNCA desde una IRQ dura: aquí abajo se toman PROCS (`task::exists`,
    // `spawn_console`), las colas RX/TX de ssh, el VFS y el heap del kernel, y
    // las syscalls sostienen esos mismos candados en ring 0 con IF=1 (el `sti`
    // de `syscall_entry`). Reentrar desde el handler MSI-X era un interbloqueo
    // en el propio core y así se colgaba la sesión SSH justo tras el prompt
    // (2026-08-01). El aserto es una carga atómica: sale gratis y convierte una
    // regresión silenciosa en un panic con traza, en vez de en una tarde de
    // bisección con QEMU.
    debug_assert!(
        !crate::arch::irq::en_irq_dura(),
        "net::poll() desde IRQ dura: reentraría en PROCS/RX/TX/heap"
    );
    let Some(net) = NET.get() else { return };
    let Some(mut n) = net.try_lock() else { return };
    // El flag se limpia DESPUÉS del try_lock: si la pila estaba ocupada, el
    // aviso tiene que sobrevivir para la siguiente pasada.
    TRABAJO_PENDIENTE.store(false, core::sync::atomic::Ordering::Relaxed);
    let NetStack {
        iface,
        sockets,
        echo,
        ssh,
        dhcp,
        configured,
        dhcp_started,
        dev,
        mac,
        user_tcp,
    } = &mut *n;

    iface.poll(now(), dev, sockets);
    poll_dhcp(iface, sockets, echo, *ssh, *dhcp, configured);
    try_static_fallback(iface, *mac, *dhcp_started, configured);
    poll_tcp_services(sockets, echo, *ssh, *configured);
    poll_user_tcp(iface, sockets, user_tcp, *configured);
    iface.poll(now(), dev, sockets);
}

fn poll_user_tcp(
    iface: &mut Interface,
    sockets: &mut SocketSet<'static>,
    user_tcp: &mut tcp_user::TcpTable,
    configured: bool,
) {
    for slot in 0..user_tcp.entries.len() {
        if let Some(entry) = user_tcp.entries[slot].as_mut() {
            tcp_user::poll_entry(iface, sockets, slot, entry, configured);
        }
    }
}

/// Reserva un socket TCP en la pila de red (llamar con NET tomado).
pub fn tcp_listen(port: u16) -> Result<usize, i64> {
    let net = NET.get().ok_or(-soso_abi::EIO)?;
    let mut n = net.lock();
    let NetStack {
        sockets,
        user_tcp,
        ..
    } = &mut *n;
    let slot = user_tcp
        .alloc(sockets, tcp_user::TcpRole::Listening, port, None)
        .map_err(|_| -soso_abi::EMFILE)?;
    if let Some(entry) = user_tcp.entries[slot].as_mut() {
        tcp_user::listen_start(sockets, entry).map_err(|_| -soso_abi::EIO)?;
    }
    Ok(slot)
}

pub fn tcp_connect(remote: soso_abi::SockAddr) -> Result<usize, i64> {
    let net = NET.get().ok_or(-soso_abi::EIO)?;
    let mut n = net.lock();
    let NetStack {
        sockets,
        user_tcp,
        ..
    } = &mut *n;
    let ep = tcp_user::endpoint_from_abi(&remote);
    let slot = user_tcp
        .alloc(
            sockets,
            tcp_user::TcpRole::Connecting,
            0,
            Some(ep),
        )
        .map_err(|_| -soso_abi::EMFILE)?;
    Ok(slot)
}

pub fn tcp_accept(listener_slot: usize) -> Result<usize, i64> {
    let net = NET.get().ok_or(-soso_abi::EIO)?;
    let n = net.lock();
    let entry = n
        .user_tcp
        .entries
        .get(listener_slot)
        .and_then(|e| e.as_ref())
        .ok_or(-soso_abi::EBADF)?;
    if entry.role != tcp_user::TcpRole::Listening && entry.role != tcp_user::TcpRole::Connected {
        return Err(-soso_abi::EINVAL);
    }
    if tcp_user::is_established(&n.sockets, entry.handle) {
        drop(n);
        let net = NET.get().unwrap();
        let mut n = net.lock();
        if let Some(e) = n
            .user_tcp
            .entries
            .get_mut(listener_slot)
            .and_then(|x| x.as_mut())
        {
            e.role = tcp_user::TcpRole::Connected;
        }
        return Ok(listener_slot);
    }
    Err(-soso_abi::EAGAIN)
}

pub fn tcp_try_read(slot: usize, buf: u64, len: u64) -> Result<u64, i64> {
    let net = NET.get().ok_or(-soso_abi::EIO)?;
    let mut n = net.lock();
    let (handle, role, closed) = {
        let entry = n
            .user_tcp
            .entries
            .get(slot)
            .and_then(|e| e.as_ref())
            .ok_or(-soso_abi::EBADF)?;
        (entry.handle, entry.role, entry.closed)
    };
    if closed {
        return Ok(0);
    }
    tcp_user::try_read(&mut n.sockets, handle, role, buf, len)
}

pub fn tcp_try_write(slot: usize, buf: u64, len: u64) -> Result<u64, i64> {
    let net = NET.get().ok_or(-soso_abi::EIO)?;
    let mut n = net.lock();
    let (handle, role, closed) = {
        let entry = n
            .user_tcp
            .entries
            .get(slot)
            .and_then(|e| e.as_ref())
            .ok_or(-soso_abi::EBADF)?;
        (entry.handle, entry.role, entry.closed)
    };
    if closed {
        return Err(-soso_abi::EPIPE);
    }
    tcp_user::try_write(&mut n.sockets, handle, role, buf, len)
}

pub fn tcp_close(slot: usize) {
    if let Some(net) = NET.get() {
        let mut n = net.lock();
        let NetStack {
            sockets,
            user_tcp,
            ..
        } = &mut *n;
        user_tcp.free(sockets, slot);
    }
}

pub fn tcp_is_connected(slot: usize) -> bool {
    let Some(net) = NET.get() else { return false };
    let n = net.lock();
    let Some(entry) = n.user_tcp.entries.get(slot).and_then(|e| e.as_ref()) else {
        return false;
    };
    entry.role == tcp_user::TcpRole::Connected && !entry.closed
}

#[allow(dead_code)]
pub fn tcp_is_connecting(slot: usize) -> bool {
    let Some(net) = NET.get() else { return false };
    let n = net.lock();
    let Some(entry) = n.user_tcp.entries.get(slot).and_then(|e| e.as_ref()) else {
        return false;
    };
    entry.role == tcp_user::TcpRole::Connecting && !entry.closed
}

pub fn tcp_connect_failed(slot: usize) -> bool {
    let Some(net) = NET.get() else { return false };
    let n = net.lock();
    let Some(entry) = n.user_tcp.entries.get(slot).and_then(|e| e.as_ref()) else {
        return false;
    };
    entry.role == tcp_user::TcpRole::Connecting && entry.closed
}

pub fn tcp_listener_ready(slot: usize) -> bool {
    let Some(net) = NET.get() else { return false };
    let n = net.lock();
    let Some(entry) = n.user_tcp.entries.get(slot).and_then(|e| e.as_ref()) else {
        return false;
    };
    tcp_user::is_established(&n.sockets, entry.handle)
}
